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

export const subscribeMicrophones = subscribeChosenModel;

export type MicrophoneEntry = { id: string; name: string };

/** The system default input; always listed and never removable. */
export const DEFAULT_MICROPHONE = "default";
const MICROPHONES_KEY = "lomi.stt.microphones";
let microphones: MicrophoneEntry[] | undefined;

function readMicrophones(): MicrophoneEntry[] {
  try {
    const parsed: unknown = JSON.parse(
      localStorage.getItem(MICROPHONES_KEY) ?? "[]",
    );
    if (Array.isArray(parsed))
      return parsed.filter(
        (entry): entry is MicrophoneEntry =>
          typeof entry?.id === "string" && typeof entry?.name === "string",
      );
  } catch {
    // Unreadable storage falls back to the system default only.
  }
  return [];
}

/** Priority order; dictation records from the first connected entry. */
export function microphoneOrder(): MicrophoneEntry[] {
  if (!microphones) {
    const stored = readMicrophones();
    microphones = stored.some((entry) => entry.id === DEFAULT_MICROPHONE)
      ? stored
      : [{ id: DEFAULT_MICROPHONE, name: "" }, ...stored];
  }
  return microphones;
}

export function setMicrophoneOrder(next: MicrophoneEntry[]) {
  microphones = next;
  try {
    localStorage.setItem(MICROPHONES_KEY, JSON.stringify(next));
  } catch {
    // The order still applies for this run.
  }
  listeners.forEach((listener) => listener());
}

/** Input volume in dB (-10..+10), applied natively before recognition. */
const GAIN_KEY = "lomi.stt.gain";
export const MAX_GAIN_DB = 10;
let gain: number | undefined;

export function microphoneGain(): number {
  if (gain === undefined) {
    try {
      gain = Number(localStorage.getItem(GAIN_KEY) ?? 0);
    } catch {
      gain = 0;
    }
    if (!Number.isFinite(gain) || Math.abs(gain) > MAX_GAIN_DB) gain = 0;
  }
  return gain;
}

export function setMicrophoneGain(db: number) {
  gain = db;
  try {
    localStorage.setItem(GAIN_KEY, String(db));
  } catch {
    // The value still applies for this run.
  }
  listeners.forEach((listener) => listener());
}

/** Vocabulary lines: `term` or `term = misheard, misheard`. */
const VOCABULARY_KEY = "lomi.stt.vocabulary";
export const MAX_VOCABULARY = 100;
export const MAX_VOCABULARY_LINE = 200;
let vocabularyText: string | undefined;

export function vocabulary(): string {
  if (vocabularyText === undefined) {
    try {
      vocabularyText = localStorage.getItem(VOCABULARY_KEY) ?? "";
    } catch {
      vocabularyText = "";
    }
  }
  return vocabularyText;
}

/** Non-empty lines, as sent to the native side. */
export const vocabularyLines = () =>
  vocabulary()
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && line.length <= MAX_VOCABULARY_LINE);

export function setVocabulary(text: string) {
  vocabularyText = text;
  try {
    localStorage.setItem(VOCABULARY_KEY, text);
  } catch {
    // The list still applies for this run.
  }
  listeners.forEach((listener) => listener());
}
