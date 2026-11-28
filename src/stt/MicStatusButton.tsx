import { useEffect, useState, useSyncExternalStore } from "react";
import { api } from "../api";
import ContextMenu, { type ContextAction } from "../ContextMenu";
import { useT } from "../i18n";
import { Check, LoaderCircle, Mic, MicOff, Settings } from "../icons";
import { IconButton } from "../ui";
import {
  chooseModel,
  chosenModel,
  DEFAULT_MICROPHONE,
  microphoneOrder,
  setMicrophoneOrder,
  type MicrophoneEntry,
} from "./stt-preferences";
import {
  copyUnsent,
  dictationState,
  retryDictation,
  startDictation,
  stopDictation,
  subscribeDictation,
  type DictationState,
  type DictationTarget,
} from "./stt-runtime";

function elapsed(since: number) {
  const seconds = Math.max(0, Math.floor((Date.now() - since) / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

const IDLE: DictationState = { phase: "idle", level: 0, startedAt: 0 };

type Menu = {
  x: number;
  y: number;
  models: { id: string; name: string; state: string }[];
  microphones: MicrophoneEntry[];
};

export default function MicStatusButton({
  target,
  side,
  scope,
}: {
  /** Resolved when recording starts, so the text lands where speaking began. */
  target: () => DictationTarget | null;
  /** Status-bar side; the button leads Source Control's group. */
  side?: "left" | "right";
  /** A chat composer's thread: only that thread's session is shown here. */
  scope?: string;
}) {
  const { t } = useT();
  const shared = useSyncExternalStore(subscribeDictation, dictationState);
  const [hint, setHint] = useState<string>();
  const [, tick] = useState(0);
  const [available, setAvailable] = useState(false);
  const [menu, setMenu] = useState<Menu>();
  const mine =
    !scope || (shared.target?.kind === "thread" && shared.target.id === scope);
  const state = mine ? shared : IDLE;
  // Another panel's session owns the microphone.
  const elsewhere =
    !mine && ["starting", "recording", "transcribing"].includes(shared.phase);
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
    (elsewhere
      ? t("Dictation is already running")
      : state.phase === "error"
        ? (state.message ?? t("Speech recognition failed"))
        : recording
          ? t("Stop dictation (Esc cancels)")
          : state.phase === "transcribing"
            ? t("Recognizing speech…")
            : state.phase === "starting"
              ? t("Starting the microphone…")
              : t("Dictate"));

  function click() {
    // A starting microphone can take ~1.5 s to deliver sound; allow stopping.
    if (recording || state.phase === "starting") return stopDictation();
    if (state.unsent) return void copyUnsent().catch(() => {});
    if (state.retryTarget) return retryDictation();
    if (busy || elsewhere) return;
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

  async function openMenu(x: number, y: number) {
    const [models, devices] = await Promise.all([
      api<Menu["models"]>("stt_list_models").catch(() => []),
      api<MicrophoneEntry[]>("stt_list_microphones").catch(() => []),
    ]);
    const connected = new Set(devices.map((device) => device.id));
    setMenu({
      x,
      y,
      models: models.filter((model) => model.state === "ready"),
      microphones: microphoneOrder().filter(
        (entry) => entry.id === DEFAULT_MICROPHONE || connected.has(entry.id),
      ),
    });
  }

  const actions = (open: Menu): (ContextAction | null)[] => {
    const chosen =
      open.models.find((model) => model.id === chosenModel()) ?? open.models[0];
    const inUse = open.microphones[0]?.id;
    const mark = (on: boolean) =>
      on ? <Check size={14} /> : <span className="stt-menu-spacer" />;
    const groups: ContextAction[][] = [
      open.models.map((model) => ({
        label: model.name,
        icon: mark(model.id === chosen?.id),
        run: () => chooseModel(model.id),
      })),
      open.microphones.map((entry) => ({
        label:
          entry.id === DEFAULT_MICROPHONE ? t("System default") : entry.name,
        icon: mark(entry.id === inUse),
        // Picking a microphone moves it to the top of the priority list.
        run: () =>
          setMicrophoneOrder([
            entry,
            ...microphoneOrder().filter((other) => other.id !== entry.id),
          ]),
      })),
      [
        {
          label: t("Speech settings"),
          icon: <Settings size={14} />,
          run: () => void api("open_settings", { page: "speech" }),
        },
      ],
    ];
    return groups
      .filter((group) => group.length)
      .flatMap((group, index) => (index ? [null, ...group] : group));
  };

  const button = (
    <IconButton
      title={title}
      className="icon-button stt-mic"
      data-phase={state.phase}
      aria-pressed={recording}
      aria-busy={busy || undefined}
      aria-haspopup="menu"
      disabled={elsewhere}
      onClick={click}
      onBlur={() => setHint(undefined)}
      onContextMenu={(event) => {
        event.preventDefault();
        const bounds = event.currentTarget.getBoundingClientRect();
        void openMenu(
          event.clientX || bounds.left,
          event.clientY || bounds.top,
        );
      }}
      onKeyDown={(event) => {
        if (
          event.key === "ContextMenu" ||
          (event.shiftKey && event.key === "F10")
        ) {
          event.preventDefault();
          const bounds = event.currentTarget.getBoundingClientRect();
          void openMenu(bounds.left, bounds.top);
        }
      }}
    >
      {state.phase === "transcribing" ? (
        <LoaderCircle size={side ? 14 : 16} className="stt-spin" />
      ) : state.phase === "error" ? (
        <MicOff size={side ? 14 : 16} />
      ) : (
        <Mic size={side ? 14 : 16} />
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
  );
  const extras = (
    <>
      <span className="stt-live" aria-live="polite">
        {recording
          ? t("Dictation started")
          : state.phase === "transcribing"
            ? t("Recognizing speech…")
            : ""}
      </span>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          label={t("Dictation")}
          actions={actions(menu)}
          onClose={() => setMenu(undefined)}
        />
      )}
    </>
  );
  return side ? (
    <div className="status-panel-control" data-side={side}>
      {button}
      {extras}
    </div>
  ) : (
    <>
      {button}
      {extras}
    </>
  );
}
