import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { Channel } from "@tauri-apps/api/core";
import { api, errorMessage } from "../api";
import { useT } from "../i18n";
import { ArrowDown, ArrowUp, GripVertical, RefreshCw, X } from "../icons";
import { SettingRow } from "../settings-ui";
import { IconButton } from "../ui";
import {
  DEFAULT_MICROPHONE,
  MAX_GAIN_DB,
  microphoneGain,
  microphoneOrder,
  setMicrophoneGain,
  setMicrophoneOrder,
  subscribeMicrophones,
  type MicrophoneEntry,
} from "./stt-preferences";

const METER_TICKS = 48;

/** Rows of the Speech → Input card: priority list, volume and a level test. */
export default function MicrophoneList() {
  const { t } = useT();
  const order = useSyncExternalStore(subscribeMicrophones, microphoneOrder);
  const gain = useSyncExternalStore(subscribeMicrophones, microphoneGain);
  const gainId = useId();
  const [connected, setConnected] = useState<Set<string>>();
  const [dragged, setDragged] = useState<number>();

  const refresh = useCallback(async () => {
    const devices = await api<MicrophoneEntry[]>("stt_list_microphones").catch(
      () => [],
    );
    setConnected(new Set(devices.map((device) => device.id)));
    // New devices join the end of the list; known ones keep their place and
    // stay listed while disconnected.
    const current = microphoneOrder();
    const known = new Set(current.map((entry) => entry.id));
    const renamed = current.map(
      (entry) => devices.find((device) => device.id === entry.id) ?? entry,
    );
    const added = devices.filter((device) => !known.has(device.id));
    if (
      added.length ||
      renamed.some((entry, index) => entry.name !== current[index].name)
    )
      setMicrophoneOrder([...renamed, ...added]);
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh]);

  const isConnected = (id: string) =>
    id === DEFAULT_MICROPHONE || !!connected?.has(id);
  const inUse = order.find((entry) => isConnected(entry.id))?.id;
  const label = (entry: MicrophoneEntry) =>
    entry.id === DEFAULT_MICROPHONE ? t("System default") : entry.name;

  const move = (from: number, to: number) => {
    if (to < 0 || to >= order.length || from === to) return;
    const next = [...order];
    next.splice(to, 0, ...next.splice(from, 1));
    setMicrophoneOrder(next);
  };
  const remove = (index: number) =>
    setMicrophoneOrder(order.filter((_, i) => i !== index));

  return (
    <>
      <SettingRow
        label={t("Microphone")}
        description={t(
          "Voice input uses the first connected microphone in this list.",
        )}
      >
        <IconButton
          title={t("Refresh microphones")}
          onClick={() => void refresh()}
        >
          <RefreshCw size={16} />
        </IconButton>
      </SettingRow>
      <ol className="speech-microphone-list">
        {order.map((entry, index) => (
          <li
            key={entry.id}
            className="speech-microphone"
            draggable
            data-dragging={dragged === index || undefined}
            onDragStart={(event) => {
              event.dataTransfer.effectAllowed = "move";
              setDragged(index);
            }}
            onDragOver={(event) => {
              if (dragged === undefined) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
            }}
            onDrop={(event) => {
              event.preventDefault();
              if (dragged !== undefined) move(dragged, index);
              setDragged(undefined);
            }}
            onDragEnd={() => setDragged(undefined)}
          >
            <span className="speech-microphone-index">{index + 1}</span>
            <span className="speech-microphone-name">
              {label(entry)}
              {entry.id === inUse && (
                <span className="speech-badge">{t("In use")}</span>
              )}
              {connected && !isConnected(entry.id) && (
                <span className="speech-meta">{t("Not connected")}</span>
              )}
            </span>
            <GripVertical
              size={16}
              className="speech-microphone-grip"
              aria-hidden="true"
            />
            <IconButton
              title={t("Move {name} up", { name: label(entry) })}
              disabled={index === 0}
              onClick={() => move(index, index - 1)}
            >
              <ArrowUp size={16} />
            </IconButton>
            <IconButton
              title={t("Move {name} down", { name: label(entry) })}
              disabled={index === order.length - 1}
              onClick={() => move(index, index + 1)}
            >
              <ArrowDown size={16} />
            </IconButton>
            {entry.id === DEFAULT_MICROPHONE ? (
              <span />
            ) : (
              <IconButton
                title={t("Remove {name}", { name: label(entry) })}
                onClick={() => remove(index)}
              >
                <X size={16} />
              </IconButton>
            )}
          </li>
        ))}
      </ol>
      <SettingRow
        label={t("Input volume")}
        htmlFor={gainId}
        description={t("Raise or lower the microphone level for dictation.")}
      >
        <input
          id={gainId}
          type="range"
          className="speech-gain"
          min={-MAX_GAIN_DB}
          max={MAX_GAIN_DB}
          step={1}
          value={gain}
          onChange={(event) => setMicrophoneGain(Number(event.target.value))}
        />
        <output htmlFor={gainId} className="setting-value speech-gain-value">
          {t("{value} dB", { value: gain > 0 ? `+${gain}` : String(gain) })}
        </output>
      </SettingRow>
      <MicrophoneTest gain={gain} />
    </>
  );
}

/** Live level of the microphone dictation would use; nothing is recorded. */
function MicrophoneTest({ gain }: { gain: number }) {
  const { t } = useT();
  const [testing, setTesting] = useState(false);
  const [level, setLevel] = useState(0);
  const [error, setError] = useState<string>();
  // Bumped on every stop and unmount, so a start that resolves late is undone.
  const run = useRef(0);

  useEffect(() => {
    if (!testing) return;
    return () => {
      run.current += 1;
      void api("stt_mic_test_stop").catch(() => {});
    };
  }, [testing]);
  useEffect(() => {
    if (testing)
      void api("stt_mic_test_gain", { gainDb: gain }).catch(() => {});
  }, [testing, gain]);

  const start = async () => {
    const token = ++run.current;
    setError(undefined);
    setLevel(0);
    setTesting(true);
    const levels = new Channel<number>();
    levels.onmessage = (value) => {
      if (run.current === token) setLevel(value);
    };
    try {
      await api("stt_mic_test_start", {
        microphones: microphoneOrder().map((entry) => entry.id),
        gainDb: microphoneGain(),
        levels,
      });
      if (run.current !== token) void api("stt_mic_test_stop").catch(() => {});
    } catch (reason) {
      if (run.current !== token) return;
      setError(errorMessage(reason));
      setTesting(false);
    }
  };
  const stop = () => {
    setTesting(false);
    setLevel(0);
  };

  const lit = testing ? Math.round(level * METER_TICKS) : 0;
  return (
    <SettingRow
      stacked
      className="speech-mic-test-row"
      label={t("Microphone test")}
      description={t("Speak to check the level. Nothing is recorded.")}
    >
      <div className="speech-mic-test">
        <button
          type="button"
          className="button button-primary"
          onClick={testing ? stop : () => void start()}
        >
          {testing ? t("Stop testing") : t("Test microphone")}
        </button>
        <div
          className="speech-meter"
          role="meter"
          aria-label={t("Microphone level")}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(level * 100)}
        >
          {Array.from({ length: METER_TICKS }, (_, index) => (
            <span key={index} data-lit={index < lit || undefined} />
          ))}
        </div>
      </div>
      {error && (
        <p className="speech-error" role="alert">
          {error}
        </p>
      )}
    </SettingRow>
  );
}
