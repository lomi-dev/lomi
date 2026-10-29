import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import type { AndroidTab } from "../model";
import { api, errorMessage } from "../api";
import {
  ArrowLeft,
  Camera,
  Circle,
  Import,
  Power,
  Square,
  Ellipsis,
  X,
  RotateCw,
  Smartphone,
  Moon,
  CircleAlert,
} from "../icons";
import { IconButton, Modal } from "../ui";
import ContextMenu from "../ContextMenu";
import PhonePicker from "./PhonePicker";
import PhoneStartup from "./PhoneStartup";
import { previewSize } from "./frame";
import {
  androidRuntime,
  openAndroidSettings,
  reportAndroidError,
} from "./service";
import { useAndroid } from "./state";
import {
  enqueueInput,
  focusInput,
  pasteInput,
  releaseInput,
  type InputEvent,
} from "./view-input";

type Runtime = Awaited<ReturnType<typeof androidRuntime>>;
const noopSubscribe = () => () => {};
const empty = () => "";

function PhoneState({
  title,
  icon = <Smartphone size={22} />,
  children,
  actions,
}: {
  title: string;
  icon?: ReactNode;
  children: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="android-panel-state">
      <header className="android-state-heading">
        <span className="android-state-icon" aria-hidden="true">
          {icon}
        </span>
        <div className="android-state-copy">
          <h2>{title}</h2>
          {children}
        </div>
      </header>
      {actions && <div className="android-state-actions">{actions}</div>}
    </div>
  );
}

export default function AndroidPane({
  tab,
  onFocus = () => {},
  onClose,
  overview = false,
}: {
  tab: AndroidTab;
  onFocus?: () => void;
  onClose?: () => void;
  overview?: boolean;
}) {
  const state = useAndroid();
  const snapshot = state.snapshot;
  const device = snapshot?.devices?.devices.find(
    (device) => device.id === tab.deviceId,
  );
  const status = snapshot?.statuses.find(
    (status) => status.deviceId === tab.deviceId,
  );
  const stream = snapshot?.streams.find(
    (stream) =>
      stream.deviceId === tab.deviceId &&
      stream.generation === status?.generation,
  );
  const host = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const focus = useRef(onFocus);
  focus.current = onFocus;
  const [runtime, setRuntime] = useState<Runtime>();
  const [busy, setBusy] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [details, setDetails] = useState(false);
  const [force, setForce] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const transportError = useSyncExternalStore(
    runtime?.subscribeRuntime ?? noopSubscribe,
    runtime ? () => runtime.runtimeError(tab.deviceId ?? "") : empty,
  );
  const zoom = useSyncExternalStore(
    runtime?.subscribeRuntime ?? noopSubscribe,
    () => runtime?.viewZoom(tab.id) ?? "fit",
  );
  const launchPending = useSyncExternalStore(
    runtime?.subscribeRuntime ?? noopSubscribe,
    () =>
      !!tab.deviceId &&
      (!runtime || runtime.launchPending(tab.id, tab.deviceId)),
  );
  const stopPending = useSyncExternalStore(
    runtime?.subscribeRuntime ?? noopSubscribe,
    () => runtime?.stopPending(tab.deviceId ?? "") ?? false,
  );
  const fullPreview = status?.display ? previewSize(...status.display) : null;
  const scaledPreview =
    fullPreview &&
    status?.display &&
    (fullPreview.width !== status.display[0] ||
      fullPreview.height !== status.display[1]);
  const stopping = stopPending || status?.phase === "stopping";
  const starting =
    !stopping &&
    (status?.phase === "starting" ||
      status?.phase === "booting" ||
      (launchPending && !status?.processAlive && !transportError));
  const running = status?.phase === "running" && !stopping;
  const connecting =
    running && !transportError && (!stream || stream.phase === "connecting");
  useEffect(() => {
    let current = true;
    void androidRuntime().then((value) => {
      if (current) setRuntime(value);
    });
    return () => {
      current = false;
    };
  }, []);
  useLayoutEffect(() => {
    if (!runtime || !device || !host.current || !input.current) return;
    return runtime.mount(
      tab,
      host.current,
      input.current,
      () => focus.current(),
      overview,
    );
  }, [runtime, tab.id, tab.deviceId, device?.id, overview]);
  useEffect(() => {
    if (
      tab.deviceId !== null ||
      !snapshot?.preferences?.defaultDeviceId ||
      !runtime
    )
      return;
    const id = snapshot.preferences.defaultDeviceId;
    if (snapshot.devices?.devices.some((device) => device.id === id))
      void runtime
        .choose(tab.id, id)
        .catch((error) => reportAndroidError(errorMessage(error)));
  }, [tab.deviceId, tab.id, snapshot?.preferences?.defaultDeviceId, runtime]);
  const act = async (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    try {
      await action();
    } catch (error) {
      reportAndroidError(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };
  const focusPhone = () => {
    if (!running || !status.generation || !tab.deviceId || !input.current)
      return false;
    onFocus();
    input.current.focus({ preventScroll: true });
    focusInput(tab.id, tab.deviceId, status.generation);
    return true;
  };
  const control = (event: InputEvent) => {
    if (focusPhone()) enqueueInput(tab.id, event);
  };
  const rotatePhone = () => {
    if (!running || overview || !device) return;
    control({
      type: "rotate",
      quarterTurns: ((runtime?.rotation(device.id) ?? 0) + 1) % 4,
    });
  };
  const fileAction = (
    command: "android_install_apk" | "android_save_screenshot",
  ) => {
    if (!running || overview || !device || !status?.generation) return;
    void act(() =>
      api(command, { deviceId: device.id, generation: status.generation }),
    );
  };
  const setup = () =>
    void act(async () => {
      await releaseInput();
      await openAndroidSettings(tab.id);
    });
  const title = device?.name ?? "Android";
  const stopPhone = () => {
    if (!device || stopping) return;
    void androidRuntime()
      .then((runtime) => runtime.stop(device.id))
      .catch((reason) => reportAndroidError(errorMessage(reason)));
  };
  const ready =
    snapshot?.toolchainReady &&
    snapshot.requiredTools.every((id) => snapshot.packages?.packages[id]);
  return (
    <section
      className="android-pane"
      data-android-pane-id={tab.id}
      aria-label={title}
      onPointerDown={onFocus}
    >
      <aside
        className="android-toolbar"
        aria-label="Phone controls"
        data-pane-drag-handle
      >
        {onClose && (
          <IconButton
            className="icon-button android-toolbar-close"
            title="Close Android panel"
            onClick={onClose}
          >
            <X size={14} />
          </IconButton>
        )}
        <div className="android-toolbar-controls">
          <div
            className="android-navigation android-toolbar-group"
            role="group"
            aria-label="Android navigation"
          >
            <IconButton
              title="Android Back"
              disabled={!running || overview}
              onClick={() => control({ type: "navigation", key: "GoBack" })}
            >
              <ArrowLeft size={14} />
            </IconButton>
            <IconButton
              title="Android Home"
              disabled={!running || overview}
              onClick={() => control({ type: "navigation", key: "GoHome" })}
            >
              <Circle size={13} />
            </IconButton>
            <IconButton
              title="Android Recent apps"
              disabled={!running || overview}
              onClick={() => control({ type: "navigation", key: "AppSwitch" })}
            >
              <Square size={12} />
            </IconButton>
          </div>
          <div
            className="android-toolbar-group"
            role="group"
            aria-label="Phone screen"
          >
            <IconButton
              title="Toggle phone screen"
              disabled={!running || overview}
              onClick={() => control({ type: "navigation", key: "Power" })}
            >
              <Power size={16} />
            </IconButton>
            <IconButton
              title="Rotate phone"
              disabled={!running || overview}
              onClick={rotatePhone}
            >
              <RotateCw size={16} />
            </IconButton>
          </div>
          <div
            className="android-toolbar-group"
            role="group"
            aria-label="Phone files"
          >
            <IconButton
              title="Install APK…"
              disabled={busy || !running || overview}
              onClick={() => fileAction("android_install_apk")}
            >
              <Import size={16} />
            </IconButton>
            <IconButton
              title="Save screenshot…"
              disabled={busy || !running || overview}
              onClick={() => fileAction("android_save_screenshot")}
            >
              <Camera size={16} />
            </IconButton>
          </div>
        </div>
        <button
          ref={button}
          type="button"
          className="icon-button android-toolbar-more"
          title="Android actions"
          aria-label="Android actions"
          aria-haspopup="menu"
          aria-expanded={!!menu}
          disabled={overview}
          onClick={() => {
            const rect = button.current!.getBoundingClientRect();
            setMenu({ x: rect.right, y: rect.bottom });
          }}
        >
          <Ellipsis size={16} />
        </button>
      </aside>
      <div
        className={`android-viewport${zoom === "fit" ? " is-fit" : " is-actual"}`}
        ref={host}
      >
        {(!running ||
          !!transportError ||
          overview ||
          !stream ||
          stream.phase === "connecting" ||
          stream?.phase === "sleeping" ||
          stream?.phase === "disconnected") && (
          <div className="android-placeholder">
            {overview ? (
              <PhoneState title="Android is running">
                <p>
                  Android continues running. Select this panel to resume its
                  screen.
                </p>
              </PhoneState>
            ) : state.loading && !snapshot ? (
              <PhoneState title="Checking Android…">
                <p>Looking for your devices and Android tools.</p>
              </PhoneState>
            ) : snapshot && !snapshot.qualified && !status?.processAlive ? (
              <PhoneState
                title="Android is unavailable on this computer"
                icon={<CircleAlert size={22} />}
                actions={
                  <button className="button" onClick={setup}>
                    Android settings
                  </button>
                }
              >
                <p>
                  Android setup and Start are unavailable on {snapshot.host} in
                  this build. Use a Lomi build qualified for this operating
                  system and CPU.
                </p>
              </PhoneState>
            ) : !ready ? (
              <PhoneState
                title="Bring Android into your workspace"
                actions={
                  <button className="button button-primary" onClick={setup}>
                    Set up Android
                  </button>
                }
              >
                <p>
                  Run a virtual phone alongside your code. Set up Android to
                  choose a device and get started.
                </p>
              </PhoneState>
            ) : !device ? (
              snapshot && (
                <PhonePicker
                  snapshot={snapshot}
                  missing={!!tab.deviceId}
                  disabled={busy}
                  onChoose={(id) =>
                    void act(async () =>
                      (await androidRuntime()).choose(tab.id, id),
                    )
                  }
                  onManage={setup}
                />
              )
            ) : starting || stopping || connecting ? (
              <PhoneStartup
                step={running ? 2 : status?.phase === "booting" ? 1 : 0}
                stopping={stopping}
                onCancel={stopPhone}
              />
            ) : running && stream?.phase === "sleeping" ? (
              <PhoneState
                title="Your phone screen is asleep"
                icon={<Moon size={22} />}
                actions={
                  <button
                    className="button button-primary"
                    onClick={() =>
                      control({ type: "navigation", key: "Power" })
                    }
                  >
                    Wake phone
                  </button>
                }
              >
                <p>The phone screen is asleep.</p>
              </PhoneState>
            ) : running ? (
              <PhoneState
                title="Reconnect to your phone"
                icon={<RotateCw size={22} />}
                actions={
                  <button
                    className="button button-primary"
                    onClick={() => runtime?.reconnect(device.id)}
                  >
                    Reconnect
                  </button>
                }
              >
                <p>
                  The screen is disconnected. Your Android apps are still
                  running.
                </p>
              </PhoneState>
            ) : (
              <PhoneState
                title={
                  status?.processAlive
                    ? "Stop the previous phone session"
                    : status?.phase === "failed"
                      ? "Your phone couldn’t start"
                      : "Start your phone"
                }
                icon={
                  status?.processAlive || status?.phase === "failed" ? (
                    <CircleAlert size={22} />
                  ) : (
                    <Smartphone size={22} />
                  )
                }
                actions={
                  <>
                    <button
                      className="button button-primary"
                      disabled={
                        busy || (!status?.processAlive && !snapshot?.qualified)
                      }
                      onClick={() =>
                        void act(async () => {
                          const runtime = await androidRuntime();
                          await (status?.processAlive
                            ? runtime.stop(device.id)
                            : runtime.start(device.id));
                        })
                      }
                    >
                      {status?.processAlive
                        ? "Retry Stop"
                        : status?.phase === "failed"
                          ? "Retry start"
                          : "Start"}
                    </button>
                    {status?.processAlive && (
                      <button className="button" onClick={() => setForce(true)}>
                        Force stop…
                      </button>
                    )}
                  </>
                }
              >
                <p>
                  {status?.processAlive
                    ? "The previous phone process is still running. Stop it before retrying."
                    : "Android is stopped. Its apps and data are kept."}
                </p>
              </PhoneState>
            )}
          </div>
        )}
      </div>
      {running &&
        tab.deviceId &&
        state.agentInput[tab.deviceId] === status?.generation &&
        status?.generation && (
          <button
            className="button android-agent-control"
            disabled={busy}
            onClick={() =>
              void act(async () => {
                await api("android_take_control", { deviceId: tab.deviceId });
                input.current?.focus({ preventScroll: true });
              })
            }
          >
            Agent input · Take control
          </button>
        )}
      <textarea
        ref={input}
        className="android-input"
        data-android-input
        aria-label="Android phone input"
        aria-description="Type into the focused phone. Tab leaves the screen. Alt-drag performs a mirrored two-finger gesture."
        autoCapitalize="off"
        autoComplete="off"
        spellCheck={false}
        tabIndex={running && !overview ? 0 : -1}
      />
      {menu && (
        <ContextMenu
          {...menu}
          label="Android actions"
          trigger={button}
          onClose={() => setMenu(null)}
          actions={[
            {
              label: "Start",
              disabled:
                busy ||
                starting ||
                stopping ||
                !device ||
                !snapshot?.qualified ||
                !!status?.processAlive,
              run: () =>
                void act(async () =>
                  (await androidRuntime()).start(device!.id),
                ),
            },
            {
              label: "Stop",
              disabled: stopping || (!starting && !status?.processAlive),
              run: stopPhone,
            },
            {
              label: "Restart (cold boot)",
              disabled:
                busy || starting || stopping || !device || !snapshot?.qualified,
              run: () =>
                void act(async () =>
                  (await androidRuntime()).restart(device!.id),
                ),
            },
            {
              label: "Rotate phone",
              disabled: !running,
              run: rotatePhone,
            },
            {
              label: "Power",
              disabled: !running,
              run: () => control({ type: "navigation", key: "Power" }),
            },
            {
              label: "Zoom in",
              disabled: !running || !runtime?.canZoom(tab.id, 1),
              run: () => runtime?.stepZoom(tab.id, 1),
            },
            {
              label: "Zoom out",
              disabled: !running || !runtime?.canZoom(tab.id, -1),
              run: () => runtime?.stepZoom(tab.id, -1),
            },
            {
              label:
                zoom === "fit"
                  ? scaledPreview
                    ? "Preview size (100%)"
                    : "Actual size (1:1)"
                  : "Fit to panel",
              disabled: !running,
              run: () => runtime?.setZoom(tab.id, zoom === "fit" ? 100 : "fit"),
            },
            {
              label: "Paste",
              disabled: !running,
              run: () =>
                void act(async () => {
                  if (focusPhone()) await pasteInput(tab.id, readText);
                }),
            },
            {
              label: "Install APK…",
              disabled: busy || !running,
              run: () => fileAction("android_install_apk"),
            },
            {
              label: "Save screenshot…",
              disabled: busy || !running,
              run: () => fileAction("android_save_screenshot"),
            },
            {
              label: "Phone settings",
              disabled: !running,
              run: () => control({ type: "settings" }),
            },
            {
              label: "Device details",
              disabled: !device,
              run: () => setDetails(true),
            },
            { label: "Manage Android", run: setup },
          ]}
        />
      )}
      {details && (
        <Modal
          className="android-dialog"
          title="Android device details"
          onClose={() => setDetails(false)}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content android-device-form">
              <p className="android-path">
                ADB serial:{" "}
                <code>{status?.serial ?? "Available after Start"}</code>
              </p>
              <p className="android-path">
                ADB:{" "}
                <code>
                  {snapshot?.adbPath ?? "Install platform tools in Settings"}
                </code>
              </p>
              <p className="settings-help">
                Click and drag to touch. Alt-drag adds a mirrored second finger.
                Scroll over the screen to swipe. Tab moves focus out of the
                phone. Paste is sent only when you request it.
              </p>
              <p className="settings-help">
                Fit shows the entire phone. Pinch or Ctrl/Cmd-scroll to zoom;
                middle-drag or Shift-scroll to move around the enlarged preview.
                Ordinary scrolling still swipes inside Android. Zoom is separate
                for each view and does not change the phone’s resolution.
              </p>
              <p className="settings-help">
                100% maps one preview pixel to one display pixel. Large phones
                use a scaled preview to keep streaming responsive; zoom does not
                add detail. Save screenshot exports the full phone resolution.
                Hidden panels pause image transfer; use Stop to release
                Android’s memory.
              </p>
              {status?.display && fullPreview && (
                <p className="settings-help">
                  Phone: {status.display[0]} × {status.display[1]} · Preview: up
                  to {fullPreview.width} × {fullPreview.height}
                </p>
              )}
            </div>
          </div>
        </Modal>
      )}
      {force && device && (
        <Modal
          protectTheme
          className="android-dialog"
          title={`Force stop ${device.name}?`}
          tone="warning"
          onClose={() => setForce(false)}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content android-device-form">
              <p>
                Unsaved Android data can be lost. The device and its stored apps
                will be kept.
              </p>
            </div>
            <div className="dialog-actions">
              <button className="button" onClick={() => setForce(false)}>
                Cancel
              </button>
              <button
                className="button danger"
                disabled={busy}
                onClick={() =>
                  void act(async () => {
                    await (await androidRuntime()).stop(device.id, true);
                    setForce(false);
                  })
                }
              >
                Force stop
              </button>
            </div>
          </div>
        </Modal>
      )}
    </section>
  );
}
