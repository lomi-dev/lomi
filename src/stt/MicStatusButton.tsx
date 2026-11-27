import { useEffect, useState, useSyncExternalStore } from "react";
import { api } from "../api";
import { useT } from "../i18n";
import { LoaderCircle, Mic, MicOff } from "../icons";
import { IconButton } from "../ui";
import {
  copyUnsent,
  dictationState,
  retryDictation,
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
  side,
}: {
  /** Resolved when recording starts, so the text lands where speaking began. */
  target: () => DictationTarget | null;
  /** Status-bar side; the button leads Source Control's group. */
  side: "left" | "right";
}) {
  const { t } = useT();
  const state = useSyncExternalStore(subscribeDictation, dictationState);
  const [hint, setHint] = useState<string>();
  const [, tick] = useState(0);
  const [available, setAvailable] = useState(false);
  // Builds without the `stt` feature have no such command: the button stays hidden.
  useEffect(() => {
    api<{ available: boolean; modelReady: boolean }>("stt_status").then(
      (status) => {
        setAvailable(status.available);
        // Warm the engine so the first dictation does not wait for GPU setup.
        if (status.available && status.modelReady)
          void api("stt_prepare").catch(() => {});
      },
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
    if (state.unsent) return void copyUnsent().catch(() => {});
    if (state.retryTarget) return retryDictation();
    if (busy) return;
    const next = target();
    if (!next) {
      setHint(t("Click into an AI chat or a terminal to dictate"));
      return;
    }
    setHint(undefined);
    // Asked on every click: a model may have been downloaded or deleted meanwhile.
    void api<{ modelReady: boolean }>("stt_status").then(
      ({ modelReady }) =>
        modelReady
          ? startDictation(next)
          : api("open_settings", { page: "speech" }),
      () => {},
    );
  }

  return (
    <div className="status-panel-control" data-side={side}>
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
    </div>
  );
}
