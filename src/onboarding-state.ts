import { api } from "./api.ts";

// Webview storage keeps the values for synchronous reads; a native file keeps
// the durable copy, because webview storage can be lost (WebView2 on Windows),
// which used to show onboarding again on every launch.
const STORAGE_KEY = "lomi.onboarding.v1";
const MODE_KEY = "lomi.work-mode.v1";

export type WorkMode = "cli" | "chat";
interface Saved {
  done: boolean;
  workMode?: WorkMode;
}

const read = (key: string) => {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
};
const write = (key: string, value: string) => {
  try {
    localStorage.setItem(key, value);
  } catch {
    // The native copy still records it.
  }
};
const localState = (): Saved => {
  const mode = read(MODE_KEY);
  return {
    done: read(STORAGE_KEY) === "done",
    ...(mode === "cli" || mode === "chat" ? { workMode: mode } : {}),
  };
};
const persist = () =>
  void api("save_onboarding_state", { state: localState() }).catch(() => {});

/** Merges the native copy into webview storage (and back), before the first read. */
export async function loadOnboardingState() {
  let saved: Saved;
  try {
    saved = await api<Saved>("load_onboarding_state");
  } catch {
    return;
  }
  const local = localState();
  if (saved.done && !local.done) write(STORAGE_KEY, "done");
  if (saved.workMode && !local.workMode) write(MODE_KEY, saved.workMode);
  // Installs from before the native copy existed have only webview storage.
  const merged = localState();
  if (merged.done !== saved.done || merged.workMode !== saved.workMode)
    persist();
}

export function onboardingDone() {
  try {
    return localStorage.getItem(STORAGE_KEY) === "done";
  } catch {
    return true;
  }
}

export function completeOnboarding() {
  write(STORAGE_KEY, "done");
  persist();
}

export function workMode(): WorkMode {
  return read(MODE_KEY) === "chat" ? "chat" : "cli";
}

export function setWorkMode(mode: WorkMode) {
  write(MODE_KEY, mode);
  persist();
}
