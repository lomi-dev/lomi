// Dictation session state outside React; the native side owns the audio.
import { Channel } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api } from "../api";
import { t } from "../i18n";
import { runningTerminal } from "../terminal-runtime";
import { getDraft, setDraft } from "../ai-chat/thread-runtime";
import { sanitizeDictation, spaceForInsert } from "./stt-insert";
import {
  chosenModel,
  microphoneGain,
  MAX_VOCABULARY,
  microphoneOrder,
  vocabularyLines,
} from "./stt-preferences";

export type DictationTarget =
  { kind: "thread"; id: string } | { kind: "terminal"; id: string };

export type DictationPhase =
  "idle" | "starting" | "recording" | "transcribing" | "error";

export type DictationState = {
  phase: DictationPhase;
  level: number;
  startedAt: number;
  message?: string;
  /** Text whose target closed while recording; copied only on request. */
  unsent?: string;
  /** The engine failed; its audio is kept briefly for `retryDictation`. */
  retryTarget?: DictationTarget;
  /** Where the current or last session delivers its text. */
  target?: DictationTarget;
};

type NativeEvent =
  | { type: "recording" }
  | { type: "level"; value: number }
  | { type: "transcribing" }
  | { type: "result"; text: string; deviceLost: boolean }
  | { type: "error"; code: string; message: string }
  | { type: "cancelled" };

let state: DictationState = { phase: "idle", level: 0, startedAt: 0 };
// `id` arrives after stt_start resolves; earlier Stop/Cancel requests wait in `pending`.
let session:
  { id?: number; target: DictationTarget; pending?: "stop" } | undefined;
const listeners = new Set<() => void>();

export const dictationState = () => state;
export function subscribeDictation(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function set(next: Partial<DictationState>) {
  state = { ...state, ...next };
  listeners.forEach((listener) => listener());
}

function errorMessage(code: string, fallback: string) {
  switch (code) {
    case "no-model":
      return t("Download a speech model to dictate");
    case "busy":
      return t("Dictation is already running");
    case "permission-denied":
      return t(
        "Lomi has no access to the microphone. Allow it in system settings.",
      );
    case "no-device":
      return t("No microphone was found");
    case "silent-input":
      return t(
        "The microphone delivers no sound. Check microphone access in system settings.",
      );
    case "no-speech":
      return t("Nothing was heard");
    case "engine-failed":
      return t("Speech recognition failed");
    case "engine-retry":
      return t(
        "Speech recognition failed. Click to try again without speaking.",
      );
    case "expired":
      return t("The recording is no longer available. Dictate again.");
    default:
      return fallback;
  }
}

// Escape cancels only while a session runs, before the terminal or the chat
// composer (where Escape stops the agent) can see it.
function onEscape(event: KeyboardEvent) {
  if (event.key !== "Escape" || event.isComposing) return;
  event.preventDefault();
  event.stopPropagation();
  cancelDictation();
}

function end(next: Partial<DictationState>) {
  session = undefined;
  window.removeEventListener("keydown", onEscape, true);
  set({ level: 0, ...next });
}

export const startDictation = (target: DictationTarget) =>
  run(target, "stt_start");

/** Recognizes the audio kept from the last failed attempt again. */
export function retryDictation() {
  if (state.retryTarget) void run(state.retryTarget, "stt_retry");
}

async function run(
  target: DictationTarget,
  command: "stt_start" | "stt_retry",
) {
  if (session) return;
  session = { target };
  window.addEventListener("keydown", onEscape, true);
  set({
    phase: command === "stt_retry" ? "transcribing" : "starting",
    target,
    level: 0,
    message: undefined,
    unsent: undefined,
    retryTarget: undefined,
  });
  if (target.kind === "thread")
    threadElement(target.id)?.dispatchEvent(
      new CustomEvent("thread-dictation-start"),
    );
  const events = new Channel<NativeEvent>();
  const current = session;
  events.onmessage = (event) => {
    if (session !== current) return;
    switch (event.type) {
      case "recording":
        set({ phase: "recording", startedAt: Date.now() });
        break;
      case "level":
        set({ level: event.value });
        break;
      case "transcribing":
        set({ phase: "transcribing", level: 0 });
        break;
      case "result": {
        const text = sanitizeDictation(event.text);
        if (!text || deliver(current.target, text)) end({ phase: "idle" });
        else
          end({
            phase: "error",
            message: t(
              "The dictation target was closed. Click to copy the text.",
            ),
            unsent: text,
          });
        break;
      }
      case "error":
        end({
          phase: "error",
          message: errorMessage(event.code, event.message),
          retryTarget:
            event.code === "engine-retry" ? current.target : undefined,
        });
        break;
      case "cancelled":
        end({ phase: "idle" });
        break;
    }
  };
  try {
    const id = await api<number>(command, {
      events,
      ...(command === "stt_start" && {
        modelId: chosenModel(),
        preferences: {
          microphones: microphoneOrder().map((entry) => entry.id),
          gainDb: microphoneGain(),
          vocabulary: vocabularyLines().slice(0, MAX_VOCABULARY),
        },
      }),
    });
    current.id = id;
    // Cancelled before the native session existed: stop it now.
    if (session !== current) void api("stt_cancel", { id }).catch(() => {});
    else if (current.pending) stopDictation();
  } catch (error) {
    if (session === current)
      end({
        phase: "error",
        message: errorMessage(String(error), String(error)),
      });
  }
}

export function stopDictation() {
  if (!session) return;
  if (session.id === undefined) session.pending = "stop";
  else void api("stt_stop", { id: session.id }).catch(() => {});
}

export function cancelDictation() {
  if (!session) return;
  if (session.id !== undefined)
    void api("stt_cancel", { id: session.id }).catch(() => {});
  else end({ phase: "idle" });
}

const threadElement = (id: string) =>
  document.querySelector(`[data-thread-tab-id="${CSS.escape(id)}"]`);

/** Copies text whose target was closed, then clears the error. */
export async function copyUnsent() {
  if (!state.unsent) return;
  await writeText(state.unsent);
  set({ phase: "idle", message: undefined, unsent: undefined });
}

/** Inserts at the target captured when the session started. */
function deliver(target: DictationTarget, text: string): boolean {
  if (target.kind === "terminal") {
    const terminal = runningTerminal(target.id);
    // Pasted, never executed: Enter is not appended.
    terminal?.insertText(text);
    return !!terminal;
  }
  const pane = threadElement(target.id);
  if (pane)
    pane.dispatchEvent(new CustomEvent("thread-dictation", { detail: text }));
  // The thread was switched away from: keep the words in its draft.
  else {
    const draft = getDraft(target.id);
    setDraft(target.id, draft + spaceForInsert(draft, text, ""));
  }
  return true;
}
