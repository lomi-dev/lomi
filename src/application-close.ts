import { api, native } from "./api";

export type ReleaseClosePreparation = () => Promise<void>;

let pendingPreparation: string | undefined;
let preparing = false;
let agentControlPrepared = false;

async function releasePreparation() {
  if (pendingPreparation) {
    await api("android_exit", {
      action: { type: "resume", preparation: pendingPreparation },
    });
    pendingPreparation = undefined;
  }
  if (agentControlPrepared) {
    await api("agent_control_closing", { closing: false });
    agentControlPrepared = false;
  }
}

/** Shared by window closure, updater installation and plugin restart. */
export async function prepareApplicationClose(
  confirm: () => Promise<boolean>,
  saveCurrentSession: () => Promise<void>,
  protectWhileStopping?: () => {
    cancelled: () => boolean;
    release: () => void;
  },
): Promise<ReleaseClosePreparation | null> {
  if (preparing)
    throw new Error("Application shutdown is already in progress.");
  preparing = true;
  let protection:
    ReturnType<NonNullable<typeof protectWhileStopping>> | undefined;
  try {
    if (native) {
      // A failed release stays available for an explicit retry; it cannot silently
      // strand the native start/mutation gate while the application remains open.
      await releasePreparation();
      await api("agent_control_closing", { closing: true });
      agentControlPrepared = true;
      const token = await api<string>("android_exit", {
        action: { type: "begin" },
      });
      if (typeof token !== "string" || !token) {
        throw new Error("Could not prepare Android shutdown. Retry closing.");
      }
      pendingPreparation = token;
    }
    if (!(await confirm())) {
      await releasePreparation();
      preparing = false;
      return null;
    }
    protection = protectWhileStopping?.();
    // Read the current session after asynchronous guards; never restore an old
    // session snapshot when a device stop fails or the user cancels closing.
    await saveCurrentSession();
    if (native && !protection?.cancelled()) {
      await api("android_exit", {
        action: {
          type: "finish",
          preparation: pendingPreparation,
          force: false,
        },
      });
    }
    if (protection?.cancelled()) {
      await releasePreparation();
      protection.release();
      preparing = false;
      return null;
    }
    const token = pendingPreparation;
    return async () => {
      if (pendingPreparation !== token) return;
      try {
        await releasePreparation();
      } finally {
        preparing = false;
        protection?.release();
      }
    };
  } catch (error) {
    preparing = false;
    protection?.release();
    try {
      await releasePreparation();
    } catch (releaseError) {
      throw new AggregateError(
        [error, releaseError],
        `${String(error)}; closing could not be cancelled: ${String(releaseError)}`,
      );
    }
    throw error;
  }
}
