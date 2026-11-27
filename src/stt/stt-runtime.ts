// Dictation session state outside React; the native side owns the audio.
import { Channel } from "@tauri-apps/api/core";
import { api } from "../api";
import { t } from "../i18n";
import { runningTerminal } from "../terminal-runtime";
import { sanitizeDictation } from "./stt-insert";

export type DictationTarget =
  { kind: "thread"; id: string } | { kind: "terminal"; id: string };

export type DictationPhase =
  "idle" | "starting" | "recording" | "transcribing" | "error";

export type DictationState = {
  phase: DictationPhase;
  level: number;
  startedAt: number;
  message?: string;
};

type NativeEvent =
  | { type: "recording" }
  | { type: "level"; value: number }
  | { type: "transcribing" }
  | { type: "result"; text: string; deviceLost: boolean }
  | { type: "error"; code: string; message: string }
  | { type: "cancelled" };

let state: DictationState = { phase: "idle", level: 0, startedAt: 0 };
let session: { id?: number; target: DictationTarget } | undefined;
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

export async function startDictation(target: DictationTarget) {
  if (session) return;
  session = { target };
  window.addEventListener("keydown", onEscape, true);
  set({ phase: "starting", level: 0, message: undefined });
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
      case "result":
        deliver(current.target, event.text);
        end({ phase: "idle" });
        break;
      case "error":
        end({
          phase: "error",
          message: errorMessage(event.code, event.message),
        });
        break;
      case "cancelled":
        end({ phase: "idle" });
        break;
    }
  };
  try {
    current.id = await api<number>("stt_start", { events });
  } catch (error) {
    if (session === current)
      end({
        phase: "error",
        message: errorMessage(String(error), String(error)),
      });
  }
}

export function stopDictation() {
  if (session?.id !== undefined)
    void api("stt_stop", { id: session.id }).catch(() => {});
}

export function cancelDictation() {
  if (!session) return;
  if (session.id !== undefined)
    void api("stt_cancel", { id: session.id }).catch(() => {});
  else end({ phase: "idle" });
}

/** Inserts at the target captured when the session started. */
function deliver(target: DictationTarget, raw: string) {
  const text = sanitizeDictation(raw);
  if (!text) return;
  if (target.kind === "terminal") {
    // Pasted, never executed: Enter is not appended.
    runningTerminal(target.id)?.insertText(text);
    return;
  }
  document
    .querySelector(`[data-thread-tab-id="${CSS.escape(target.id)}"]`)
    ?.dispatchEvent(new CustomEvent("thread-dictation", { detail: text }));
}
