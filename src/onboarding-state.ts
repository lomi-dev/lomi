// shortcut: completion lives in webview storage, so clearing site data shows
// onboarding again; move it into native preferences if that becomes a problem.
const STORAGE_KEY = "lomi.onboarding.v1";

export function onboardingDone() {
  try {
    return localStorage.getItem(STORAGE_KEY) === "done";
  } catch {
    return true;
  }
}

export function completeOnboarding() {
  try {
    localStorage.setItem(STORAGE_KEY, "done");
  } catch {
    // Storage can be unavailable; closing still returns to the workspace.
  }
}

export type WorkMode = "cli" | "chat";
// shortcut: stored next to onboarding completion in webview storage; move to
// native preferences together with it.
const MODE_KEY = "lomi.work-mode.v1";

export function workMode(): WorkMode {
  try {
    return localStorage.getItem(MODE_KEY) === "chat" ? "chat" : "cli";
  } catch {
    return "cli";
  }
}

export function setWorkMode(mode: WorkMode) {
  try {
    localStorage.setItem(MODE_KEY, mode);
  } catch {
    // Without storage the default terminal-first behavior stays in effect.
  }
}
