// The chosen speech model, kept in webview storage like other per-device UI
// preferences. Native code falls back to the first downloaded model.
const KEY = "lomi.stt.model";
const listeners = new Set<() => void>();

export function chosenModel(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

export function chooseModel(id: string | null) {
  try {
    if (id) localStorage.setItem(KEY, id);
    else localStorage.removeItem(KEY);
  } catch {
    // Storage can be unavailable; the native fallback still picks a model.
  }
  listeners.forEach((listener) => listener());
}

export function subscribeChosenModel(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
