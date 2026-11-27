import { useEffect, useState, useSyncExternalStore } from "react";
import { api } from "../api";
import { useT } from "../i18n";
import { LoaderCircle, Mic, MicOff } from "../icons";
import { IconButton } from "../ui";
import {
  dictationState,
  startDictation,
  stopDictation,
  subscribeDictation,
  type DictationTarget,
} from "./stt-runtime";

function elapsed(since: number) {
  const seconds = Math.max(0, Math.floor((Date.now() - since) / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export default function MicStatusButton({
  target,
}: {
  /** Resolved when recording starts, so the text lands where speaking began. */
  target: () => DictationTarget | null;
}) {
  const { t } = useT();
  const state = useSyncExternalStore(subscribeDictation, dictationState);
  const [hint, setHint] = useState<string>();
  const [, tick] = useState(0);
  const [available, setAvailable] = useState(false);
  // Builds without the `stt` feature have no such command: the button stays hidden.
  useEffect(() => {
    api<{ available: boolean }>("stt_status").then(
      (status) => setAvailable(status.available),
      () => setAvailable(false),
    );
  }, []);
  const recording = state.phase === "recording";
  useEffect(() => {
    if (!recording) return;
    const timer = window.setInterval(() => tick((n) => n + 1), 500);
    return () => window.clearInterval(timer);
  }, [recording]);
  if (!available) return null;

  const busy = state.phase === "starting" || state.phase === "transcribing";
  const title =
    hint ??
    (state.phase === "error"
      ? (state.message ?? t("Speech recognition failed"))
      : recording
        ? t("Stop dictation (Esc cancels)")
        : state.phase === "transcribing"
          ? t("Recognizing speech…")
          : state.phase === "starting"
            ? t("Starting the microphone…")
            : t("Dictate"));

  function click() {
    if (recording) return stopDictation();
    if (busy) return;
    const next = target();
    if (!next) {
      setHint(t("Click into an AI chat or a terminal to dictate"));
      return;
    }
    setHint(undefined);
    void startDictation(next);
  }

  return (
    <>
      <IconButton
        title={title}
        className="icon-button stt-mic"
        data-phase={state.phase}
        aria-pressed={recording}
        aria-busy={busy || undefined}
        onClick={click}
        onBlur={() => setHint(undefined)}
      >
        {state.phase === "transcribing" ? (
          <LoaderCircle size={14} className="stt-spin" />
        ) : state.phase === "error" ? (
          <MicOff size={14} />
        ) : (
          <Mic size={14} />
        )}
        {recording && (
          <>
            <span
              className="stt-level"
              style={{ transform: `scaleY(${0.15 + state.level * 0.85})` }}
            />
            <span className="stt-time">{elapsed(state.startedAt)}</span>
          </>
        )}
      </IconButton>
      <span className="stt-live" aria-live="polite">
        {recording
          ? t("Dictation started")
          : state.phase === "transcribing"
            ? t("Recognizing speech…")
            : ""}
      </span>
    </>
  );
}
