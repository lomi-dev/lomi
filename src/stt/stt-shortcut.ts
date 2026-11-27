// Dictation shortcut: a tap toggles, holding past HOLD_MS is push-to-talk.
import {
  dictationState,
  startDictation,
  stopDictation,
  type DictationTarget,
} from "./stt-runtime";

const HOLD_MS = 300;

let held: { code: string; since: number } | undefined;

// macOS drops the letter's keyup while Cmd is down, so a modifier release
// also counts as releasing the shortcut.
const RELEASE_KEYS = new Set(["Control", "Meta", "Shift", "Alt"]);

function release(stop: boolean) {
  window.removeEventListener("keyup", onKeyUp, true);
  window.removeEventListener("blur", onBlur);
  held = undefined;
  if (stop) stopDictation();
}

function onKeyUp(event: KeyboardEvent) {
  if (!held || (event.code !== held.code && !RELEASE_KEYS.has(event.key)))
    return;
  release(Date.now() - held.since >= HOLD_MS);
}

// A window that loses focus never sees the keyup; end the hold instead of
// recording until the time limit.
const onBlur = () => release(true);

export function dictationShortcut(
  code: string,
  target: () => DictationTarget | null,
) {
  const { phase } = dictationState();
  if (phase === "recording" || phase === "starting") return stopDictation();
  if (phase === "transcribing" || held) return;
  const next = target();
  if (!next) return;
  held = { code, since: Date.now() };
  window.addEventListener("keyup", onKeyUp, true);
  window.addEventListener("blur", onBlur);
  void startDictation(next);
}
