import { Channel, invoke } from "@tauri-apps/api/core";
import { mark } from "../../remote-web/src/timing";
import {
  NativeCoreClient,
  type NativeCoreDeps,
  type NativeCoreEvent,
} from "./remote-core-native";

declare const __LOMI_REMOTE_CORE__: boolean | undefined;

/** Build-time switch (`LOMI_REMOTE_CORE=1`, see vite.config.ts); off by default. */
export const remoteCoreEnabled = (): boolean =>
  typeof __LOMI_REMOTE_CORE__ !== "undefined" && __LOMI_REMOTE_CORE__ === true;

const nativeCoreDeps: NativeCoreDeps = {
  async start(onEvent, expectedAccount) {
    const events = new Channel<NativeCoreEvent>();
    events.onmessage = onEvent;
    return invoke<boolean>("remote_v2_core_start", { expectedAccount, events });
  },
  names: (hosts) => invoke("remote_v2_core_names", { hosts }),
  renew: () => invoke("remote_v2_core_renew"),
  wipe: () => invoke("remote_v2_core_wipe"),
  mark,
};

let client: NativeCoreClient | null | undefined;

/**
 * Stands in for Remote Web's `core/instance` module in the desktop bundle (vite.config.ts),
 * so the shared Host list reads the Rust core instead of starting a second, WASM one.
 */
export function getCoreClient(): NativeCoreClient | null {
  if (client !== undefined) return client;
  return (client = remoteCoreEnabled()
    ? new NativeCoreClient(nativeCoreDeps)
    : null);
}

export async function wipeCoreDirectory(): Promise<void> {
  await getCoreClient()?.wipe();
}

/** Unused on the desktop (the link runs in Rust); kept for the module's shape. */
export function linkUrlFor(relayUrl: string): string {
  return relayUrl;
}
