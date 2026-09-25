import { McpClientsSettings } from "./McpClientsSettings";
import { AgentBrowserUploadApproval } from "./AgentBrowserUploadApproval";
import { AgentAndroidApproval } from "./AgentAndroidApproval";
import Pairing from "./AgentControlPairing";
import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import { DisclosureSummary, Modal } from "./ui";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";
import type { ControlState } from "./agent-control";
import type { ControlStartupState } from "./agent-control-startup";
import { formatShortcut } from "./keybindings";

type AgentControlTab = "connect" | "sessions" | "preferences";

function pendingRequestCount(
  broker: ControlState["broker"] | null | undefined,
) {
  if (!broker) return 0;
  return (
    broker.pending.length +
    broker.pendingControls.length +
    (broker.pendingSettingsUpdates?.length ?? 0) +
    (broker.pendingProjectOpens?.length ?? 0) +
    (broker.pendingAndroidManagement?.length ?? 0) +
    (broker.pendingBrowserUploads?.length ?? 0) +
    (broker.pendingInstalls?.length ?? 0)
  );
}

function ThemePreferenceChanges({
  before,
  after,
}: {
  before: Record<string, unknown>;
  after: Record<string, unknown>;
}) {
  const name = (id: unknown) =>
    id === null ? "Lomi" : id === "@builtin-deepmono" ? "DeepMono" : String(id);
  const appearance = (value: unknown) =>
    value === "system" ? "Follow system" : value === "light" ? "Light" : "Dark";
  return (
    <dl>
      <dt>Color theme</dt>
      <dd>
        {name(before.active)} → {name(after.active)}
      </dd>
      <dt>Appearance preference</dt>
      <dd>
        {appearance(before.appearance)} → {appearance(after.appearance)}
      </dd>
    </dl>
  );
}

function KeybindingChanges({
  before,
  after,
  reset,
}: {
  before: Record<string, unknown>;
  after: Record<string, unknown>;
  reset: boolean;
}) {
  const action = before.action as {
    id: string;
    label: string;
    shortcut: string | null;
  } | null;
  const next = after.action as { shortcut: string | null } | null;
  return action && next ? (
    <dl>
      <dt>
        {reset
          ? "Restore default shortcut"
          : next.shortcut === null
            ? "Disable shortcut"
            : "Assign shortcut"}
      </dt>
      <dd>
        {action.label} (<code>{action.id}</code>)
      </dd>
      <dt>Shortcut</dt>
      <dd>
        <code>
          {action.shortcut === null
            ? "Disabled"
            : formatShortcut(action.shortcut)}
        </code>{" "}
        →{" "}
        <code>
          {next.shortcut === null ? "Disabled" : formatShortcut(next.shortcut)}
        </code>
      </dd>
    </dl>
  ) : (
    <dl>
      <dt>Focus follows pointer</dt>
      <dd>
        {before.focusFollowsPointer ? "On" : "Off"} →{" "}
        {after.focusFollowsPointer ? "On" : "Off"}
      </dd>
    </dl>
  );
}

function TerminalPreferenceChanges({
  before,
  after,
}: {
  before: Record<string, unknown>;
  after: Record<string, unknown>;
}) {
  const changes: { path: string; before: unknown; after: unknown }[] = [];
  const visit = (left: unknown, right: unknown, path: string) => {
    if (JSON.stringify(left) === JSON.stringify(right)) return;
    if (
      (left && typeof left === "object") ||
      (right && typeof right === "object")
    ) {
      const a = (left ?? {}) as Record<string, unknown>;
      const b = (right ?? {}) as Record<string, unknown>;
      for (const key of new Set([...Object.keys(a), ...Object.keys(b)]))
        visit(a[key], b[key], path ? `${path}.${key}` : key);
    } else changes.push({ path, before: left, after: right });
  };
  visit(before, after, "");
  const display = (value: unknown) =>
    value === undefined
      ? "Theme default"
      : typeof value === "boolean"
        ? value
          ? "On"
          : "Off"
        : String(value);
  return changes.length ? (
    <>
      {changes.some(
        (c) =>
          c.path === "behavior.scrollback" &&
          typeof c.before === "number" &&
          typeof c.after === "number" &&
          c.after < c.before,
      ) && (
        <p className="settings-help">
          Reducing scrollback removes older lines beyond the new limit from
          terminal displays.
        </p>
      )}
      <dl>
        {changes.map((change) => {
          const name = change.path
            .split(".")
            .at(-1)!
            .replace(/[A-Z]/g, (c) => ` ${c.toLowerCase()}`);
          return (
            <Fragment key={change.path}>
              <dt>
                {change.path.includes(".colors.") ? "Color: " : ""}
                {name[0].toUpperCase() + name.slice(1)}
              </dt>
              <dd>
                {display(change.before)} → {display(change.after)}
              </dd>
            </Fragment>
          );
        })}
      </dl>
    </>
  ) : (
    <p>The requested value is already selected.</p>
  );
}

export default function AgentControlSettingsPage() {
  const [state, setState] = useState<ControlState>();
  const [startupState, setStartupState] = useState<ControlStartupState>();
  const [error, setError] = useState("");
  const [startupError, setStartupError] = useState("");
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [startupBusy, setStartupBusy] = useState(false);
  const [yoloConfirmationOpen, setYoloConfirmationOpen] = useState(false);
  const [visible, setVisible] = useState(true);
  const [activeTab, setActiveTab] = useState<AgentControlTab>("connect");
  const yoloCancelButton = useRef<HTMLButtonElement>(null);
  const yoloSwitch = useRef<HTMLInputElement>(null);
  const restoreYoloFocus = useRef(false);
  const initialControlSnapshotSeen = useRef(false);
  const tabSelectedByUser = useRef(false);
  const startupStateRef = useRef<ControlStartupState | undefined>(undefined);
  const startupRevision = useRef(0);
  const startupMutationRevision = useRef(0);
  const startupSaveLock = useRef(false);
  const enabled = Boolean(state?.broker);
  useEffect(() => {
    if (!yoloConfirmationOpen && !startupBusy && restoreYoloFocus.current) {
      restoreYoloFocus.current = false;
      yoloSwitch.current?.focus();
    }
  }, [startupBusy, yoloConfirmationOpen]);
  useEffect(() => {
    if (!native) return;
    const stop = listen<boolean>("settings-visibility", ({ payload }) =>
      setVisible(payload),
    );
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);
  const acceptControlState = useCallback((next: ControlState) => {
    setState(next);
    if (!initialControlSnapshotSeen.current) {
      initialControlSnapshotSeen.current = true;
      if (pendingRequestCount(next.broker) && !tabSelectedByUser.current)
        setActiveTab("sessions");
    }
  }, []);
  const refresh = useCallback(async () => {
    acceptControlState(await api<ControlState>("agent_control_state"));
  }, [acceptControlState]);
  const applyStartupState = useCallback((next: ControlStartupState) => {
    if (
      (next.autoStart !== null &&
        next.autoStart !== startupStateRef.current?.autoStart) ||
      next.yoloMode !== startupStateRef.current?.yoloMode
    ) {
      ++startupMutationRevision.current;
      setStartupError("");
    }
    startupStateRef.current = next;
    setStartupState(next);
  }, []);
  const refreshStartup = useCallback(async () => {
    const request = ++startupRevision.current;
    try {
      const next = await api<ControlStartupState>(
        "agent_control_startup_state",
      );
      if (request !== startupRevision.current) return;
      applyStartupState(next);
    } catch (e) {
      if (request === startupRevision.current) setStartupError(errorMessage(e));
    }
  }, [applyStartupState]);
  useEffect(() => {
    if (!native) return;
    void refreshStartup();
    const stop = listen<ControlStartupState>(
      "agent-control-startup-changed",
      ({ payload }) => {
        ++startupRevision.current;
        applyStartupState(payload);
        void refresh().catch((e) => setError(errorMessage(e)));
      },
    );
    const focused = () => {
      void refreshStartup();
      void refresh().catch((e) => setError(errorMessage(e)));
    };
    window.addEventListener("focus", focused);
    return () => {
      ++startupRevision.current;
      ++startupMutationRevision.current;
      window.removeEventListener("focus", focused);
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [applyStartupState, refresh, refreshStartup]);
  useEffect(() => {
    if (!native || !visible) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const next = await api<ControlState>("agent_control_state");
        if (alive) acceptControlState(next);
        if (alive && next.broker) timer = setTimeout(() => void poll(), 1000);
      } catch (e) {
        if (alive) setError(errorMessage(e));
      }
    };
    void poll();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [acceptControlState, enabled, visible]);
  const run = async (
    action: () => Promise<unknown>,
    message: string | ((result: unknown) => string) = "",
  ) => {
    if (busy) return;
    setBusy(true);
    setError("");
    setStatus("");
    try {
      const result = await action();
      await refresh();
      setStatus(typeof message === "function" ? message(result) : message);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const setAutomaticStart = async (enabled: boolean) => {
    if (startupSaveLock.current) return;
    startupSaveLock.current = true;
    const request = ++startupMutationRevision.current;
    ++startupRevision.current;
    setStartupBusy(true);
    setStartupError("");
    try {
      const next = await api<ControlStartupState>(
        "agent_control_set_auto_start",
        { enabled },
      );
      if (request === startupMutationRevision.current) {
        ++startupRevision.current;
        applyStartupState(next);
        setStartupError("");
      }
    } catch (e) {
      if (request === startupMutationRevision.current)
        setStartupError(errorMessage(e));
    } finally {
      startupSaveLock.current = false;
      setStartupBusy(false);
    }
  };
  const setYoloMode = async (enabled: boolean) => {
    if (startupSaveLock.current) return false;
    startupSaveLock.current = true;
    const request = ++startupMutationRevision.current;
    ++startupRevision.current;
    setStartupBusy(true);
    setStartupError("");
    try {
      const next = await api<ControlStartupState>(
        "agent_control_set_yolo_mode",
        { enabled },
      );
      if (request === startupMutationRevision.current) {
        ++startupRevision.current;
        applyStartupState(next);
        setStartupError("");
      }
      return true;
    } catch (e) {
      if (request === startupMutationRevision.current) {
        setStartupError(errorMessage(e));
        await refreshStartup();
      }
      return false;
    } finally {
      startupSaveLock.current = false;
      setStartupBusy(false);
    }
  };
  const confirmYoloMode = async () => {
    if (startupBusy || startupSaveLock.current) return;
    if (await setYoloMode(true)) {
      restoreYoloFocus.current = true;
      setYoloConfirmationOpen(false);
    }
  };
  const broker = state?.broker;
  const config =
    broker && state?.helperPath
      ? JSON.stringify(
          {
            mcpServers: {
              lomi: {
                command: state.helperPath,
                args: [
                  "--endpoint",
                  broker.endpoint.endpoint,
                  "--instance",
                  broker.endpoint.instanceId,
                  "--broker-sha256",
                  broker.endpoint.brokerSha256,
                ],
              },
            },
          },
          null,
          2,
        )
      : "";

  const pendingOperationRequests = broker ? (
    <>
      {!!broker.pendingControls.length && (
        <SettingsSection title="Terminal input requests">
          {broker.pendingControls.map((request) => (
            <article
              className="agent-control-request"
              key={request.operationId}
              data-control-operation={request.operationId}
            >
              <h3>{request.title}</h3>
              <p>
                {request.clientLabel} requests permission to type in this
                running terminal. Existing text and processes will remain in
                place.
              </p>
              <p className="settings-help">
                Workspace:{" "}
                {broker.workspaces.find((w) => w.id === request.workspaceId)
                  ?.name ?? "Unavailable"}
                . Request expires in {request.secondsRemaining} seconds. Typing
                in the terminal takes control back.
              </p>
              <div className="agent-control-actions">
                <button
                  type="button"
                  className="button"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () =>
                        api("agent_control_decide_terminal", {
                          operationId: request.operationId,
                          approve: false,
                        }),
                      "Terminal input request denied.",
                    )
                  }
                >
                  Deny input
                </button>
                <button
                  type="button"
                  className="button button-primary"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () =>
                        api("agent_control_decide_terminal", {
                          operationId: request.operationId,
                          approve: true,
                        }),
                      "Terminal input decision recorded.",
                    )
                  }
                >
                  Allow input
                </button>
              </div>
            </article>
          ))}
        </SettingsSection>
      )}
      {!!broker.pendingSettingsUpdates?.length && (
        <SettingsSection title="Preference change requests">
          {broker.pendingSettingsUpdates.map((request) => (
            <article
              className="agent-control-request"
              key={request.operationId}
              data-settings-operation={request.operationId}
            >
              <h3>
                {request.clientLabel} —{" "}
                {request.section === "terminal"
                  ? "Terminal preferences"
                  : request.section === "keybinds"
                    ? "Keyboard shortcuts"
                    : request.section === "themes"
                      ? "Theme preferences"
                      : "Editor defaults"}
              </h3>
              <p>
                This changes application defaults across all projects.{" "}
                {request.section === "terminal"
                  ? "Running terminals keep their processes."
                  : request.section === "keybinds"
                    ? "Shortcut changes apply to every window."
                    : request.section === "themes"
                      ? "Running terminals and editor text are retained."
                      : "Existing buffer overrides remain in place."}
              </p>
              {request.section === "terminal" ? (
                <TerminalPreferenceChanges
                  before={request.before}
                  after={request.after}
                />
              ) : request.section === "keybinds" ? (
                <KeybindingChanges
                  before={request.before}
                  after={request.after}
                  reset={request.patch?.type === "keybinding_reset"}
                />
              ) : request.section === "themes" ? (
                <ThemePreferenceChanges
                  before={request.before}
                  after={request.after}
                />
              ) : (
                <dl>
                  <dt>Tab size</dt>
                  <dd>
                    {request.before.tabSize} → {request.after.tabSize}
                  </dd>
                  <dt>Indentation</dt>
                  <dd>
                    {request.before.insertSpaces ? "Spaces" : "Tabs"} →{" "}
                    {request.after.insertSpaces ? "Spaces" : "Tabs"}
                  </dd>
                </dl>
              )}
              <p className="settings-help">
                Operation {request.operationId}; request {request.requestKey}.
                Expires in {request.secondsRemaining} s. A concurrent preference
                change cancels this write.
              </p>
              <div className="agent-control-actions">
                <button
                  type="button"
                  className="button"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () =>
                        api("agent_control_settings_decide", {
                          operationId: request.operationId,
                          approve: false,
                        }),
                      "Preference change rejected.",
                    )
                  }
                >
                  Reject change
                </button>
                <button
                  type="button"
                  className="button button-primary"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () =>
                        api("agent_control_settings_decide", {
                          operationId: request.operationId,
                          approve: true,
                        }),
                      "Preference change applied.",
                    )
                  }
                >
                  Apply change
                </button>
              </div>
            </article>
          ))}
        </SettingsSection>
      )}
      {!!broker.pendingProjectOpens?.length && (
        <SettingsSection title="Project folder requests">
          {broker.pendingProjectOpens.map((request) => (
            <article
              className="agent-control-request"
              key={request.operationId}
            >
              <h3>{request.clientLabel} — Open project folder</h3>
              <p>
                Folder: <code>{request.projectPath}</code>
              </p>
              <p>
                Initial workspace: <strong>{request.workspaceName}</strong>
              </p>
              <p>
                This project will share the connection’s approved permissions:
              </p>
              <p>{request.scopes.join(", ")}</p>
              <p className="settings-help">
                A blank editor opens first. Any terminal execution permission
                runs with your user account permissions; the folder is not a
                sandbox.
              </p>
              <p className="settings-help">
                Operation {request.operationId}; request {request.requestKey}.
                Expires in {request.secondsRemaining}
                s.
              </p>
              <div className="agent-control-actions">
                <button
                  className="button"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () =>
                        api("agent_control_project_open_decide", {
                          operationId: request.operationId,
                          approved: false,
                        }),
                      "Project request rejected.",
                    )
                  }
                >
                  Reject folder
                </button>
                <button
                  className="button button-primary"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () =>
                        api("agent_control_project_open_decide", {
                          operationId: request.operationId,
                          approved: true,
                        }),
                      "Project folder approved for this request.",
                    )
                  }
                >
                  Approve folder
                </button>
              </div>
            </article>
          ))}
        </SettingsSection>
      )}
      {!!broker.pendingAndroidManagement?.length && (
        <SettingsSection title="Android setup and device requests">
          {broker.pendingAndroidManagement.map((request) => (
            <AgentAndroidApproval
              key={`${request.operationId}:${request.plan.revision}`}
              request={request}
              busy={busy}
              run={run}
            />
          ))}
        </SettingsSection>
      )}
      <AgentBrowserUploadApproval
        requests={broker.pendingBrowserUploads ?? []}
        busy={busy}
        run={run}
      />
      {!!broker.pendingInstalls?.length && (
        <SettingsSection title="APK installation requests">
          {broker.pendingInstalls.map((request) => (
            <article
              className="agent-control-request"
              key={request.operationId}
            >
              <h3>
                {request.clientLabel} — Install APK on {request.title}
              </h3>
              <p>
                {request.relativePath} ({request.byteLength.toLocaleString()}{" "}
                bytes)
              </p>
              <p className="settings-help">
                Workspace:{" "}
                {broker.workspaces.find((w) => w.id === request.workspaceId)
                  ?.name ?? "Unavailable workspace"}
                <br />
                Device: <code>{request.deviceId}</code>
                <br />
                Running instance: <code>{request.generation}</code>
                <br />
                SHA-256: <code>{request.sha256}</code>
                <br />
                Expires in {request.secondsRemaining} seconds
              </p>
              <p className="settings-help">
                Install this completed private copy. It may update an existing
                app and retain its data. Lomi will not uninstall an app to
                resolve an installation failure.
              </p>
              <div className="agent-control-actions">
                <button
                  className="button"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () =>
                        api("agent_control_decide_install", {
                          operationId: request.operationId,
                          approve: false,
                        }),
                      "APK installation denied.",
                    )
                  }
                >
                  Deny installation
                </button>
                <button
                  className="button button-primary"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () =>
                        api("agent_control_decide_install", {
                          operationId: request.operationId,
                          approve: true,
                        }),
                      "APK installation approved.",
                    )
                  }
                >
                  Install this APK
                </button>
              </div>
            </article>
          ))}
        </SettingsSection>
      )}
    </>
  ) : null;
  const requestCount = pendingRequestCount(broker);
  const serverStatus = !state
    ? "Checking…"
    : broker
      ? broker.uiReady
        ? "Ready"
        : "Waiting for workspace"
      : "Off";
  const serverState = !state
    ? "loading"
    : broker
      ? broker.uiReady
        ? "ready"
        : "waiting"
      : "off";
  const selectTab = (tab: AgentControlTab) => {
    tabSelectedByUser.current = true;
    setActiveTab(tab);
  };
  const tabOrder: AgentControlTab[] = ["connect", "sessions", "preferences"];
  const tabButtons = (tab: AgentControlTab) => ({
    id: `agent-control-tab-${tab}`,
    role: "tab" as const,
    "aria-selected": activeTab === tab,
    "aria-controls": `agent-control-panel-${tab}`,
    tabIndex: activeTab === tab ? 0 : -1,
    onClick: () => selectTab(tab),
    onKeyDown: (event: React.KeyboardEvent<HTMLButtonElement>) => {
      const currentIndex = tabOrder.indexOf(tab);
      let next: AgentControlTab | undefined;
      if (event.key === "ArrowRight")
        next = tabOrder[(currentIndex + 1) % tabOrder.length];
      else if (event.key === "ArrowLeft")
        next = tabOrder[(currentIndex - 1 + tabOrder.length) % tabOrder.length];
      else if (event.key === "Home") next = tabOrder[0];
      else if (event.key === "End") next = tabOrder.at(-1);
      if (!next) return;
      event.preventDefault();
      selectTab(next);
      document.getElementById(`agent-control-tab-${next}`)?.focus();
    },
  });
  return (
    <SettingsPage
      title="Agent control"
      description="Let coding agents work with Lomi over MCP."
      className="agent-control-page"
      status={status}
      actions={
        <>
          <span
            className="agent-control-badge"
            data-state={serverState}
            aria-live="polite"
          >
            {serverStatus}
          </span>
          <button
            type="button"
            className="button"
            disabled={!native || !state?.supported || busy}
            onClick={() =>
              void run(() => api("agent_control_enable", { enabled: !broker }))
            }
          >
            {broker ? "Turn off server" : "Start server"}
          </button>
        </>
      }
    >
      {startupState?.yoloMode && (
        <SettingsNotice tone="warning" role="note">
          <strong>YOLO mode is on.</strong> Local clients pair automatically and
          Lomi approves their requests in every workspace.
        </SettingsNotice>
      )}
      {startupError && !yoloConfirmationOpen && (
        <SettingsNotice tone="error">{startupError}</SettingsNotice>
      )}
      {startupState?.error && (
        <SettingsNotice tone="error">
          {startupState.autoStart === true
            ? `The server couldn’t start automatically: ${startupState.error} Automatic start stays on.`
            : startupState.error}
        </SettingsNotice>
      )}
      {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
      {state && !state.supported && (
        <SettingsNotice tone="warning">
          Agent control isn’t available on this computer yet.
        </SettingsNotice>
      )}

      <div
        className="agent-control-tabs"
        role="tablist"
        aria-label="Agent control"
      >
        <button
          type="button"
          className="agent-control-tab"
          {...tabButtons("connect")}
        >
          Connect an agent
        </button>
        <button
          type="button"
          className="agent-control-tab"
          {...tabButtons("sessions")}
        >
          Sessions
          {requestCount > 0 && (
            <span
              className="agent-control-tab-count"
              aria-label={`${requestCount} pending requests`}
            >
              {requestCount}
            </span>
          )}
        </button>
        <button
          type="button"
          className="agent-control-tab"
          {...tabButtons("preferences")}
        >
          Preferences
        </button>
      </div>

      {requestCount > 0 && activeTab !== "sessions" && (
        <SettingsNotice
          className="agent-control-notice"
          role="status"
          action={
            <button
              type="button"
              className="button"
              onClick={() => selectTab("sessions")}
            >
              Review requests
            </button>
          }
        >
          {requestCount} request{requestCount === 1 ? " needs" : "s need"}{" "}
          review.
        </SettingsNotice>
      )}

      {yoloConfirmationOpen && (
        <Modal
          protectTheme
          role="alertdialog"
          tone="warning"
          title="Enable YOLO mode?"
          descriptionId="agent-control-yolo-confirmation-description"
          initialFocus={yoloCancelButton}
          closeDisabled={startupBusy}
          onClose={() => {
            if (!startupBusy) setYoloConfirmationOpen(false);
          }}
        >
          <div className="dialog-form" aria-busy={startupBusy}>
            <p id="agent-control-yolo-confirmation-description">
              Local MCP clients will pair automatically, and their requests run
              in every workspace without asking: terminal commands, file
              changes, Git, browser, Android and chat messages. Active sessions
              disconnect and must reconnect.
            </p>
            {startupError && (
              <div className="keybindings-error" role="alert">
                {startupError}
              </div>
            )}
            <div className="dialog-actions">
              <button
                ref={yoloCancelButton}
                type="button"
                className="button"
                disabled={startupBusy}
                onClick={() => setYoloConfirmationOpen(false)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="button button-primary"
                disabled={startupBusy}
                onClick={() => void confirmYoloMode()}
              >
                {startupBusy ? "Saving…" : "Enable YOLO mode"}
              </button>
            </div>
          </div>
        </Modal>
      )}

      <section
        className="agent-control-panel"
        id="agent-control-panel-connect"
        role="tabpanel"
        aria-labelledby="agent-control-tab-connect"
        tabIndex={0}
        hidden={activeTab !== "connect"}
      >
        {state?.supported ? (
          <McpClientsSettings
            yoloMode={startupState?.yoloMode === true}
            onInstalled={() =>
              refresh().catch((cause) => setError(errorMessage(cause)))
            }
          />
        ) : state ? null : (
          <p className="settings-help" role="status">
            Loading…
          </p>
        )}
      </section>

      <section
        className="agent-control-panel"
        id="agent-control-panel-sessions"
        role="tabpanel"
        aria-labelledby="agent-control-tab-sessions"
        tabIndex={0}
        hidden={activeTab !== "sessions"}
      >
        {!broker ? (
          <div className="settings-empty">
            <h2>No active sessions</h2>
            <p>
              Start the server and connect a client to see its request here.
            </p>
            <button
              type="button"
              className="button"
              onClick={() => selectTab("connect")}
            >
              Connect an agent
            </button>
          </div>
        ) : (
          <>
            <SettingsSection title="New session requests">
              {!broker.pending.length && (
                <p className="settings-help">
                  No client is waiting to connect.
                </p>
              )}
              {broker.pending.map((request) => (
                <Pairing
                  key={request.id}
                  request={request}
                  workspaces={broker.workspaces}
                  terminalProfiles={
                    broker.terminalProfiles ??
                    (broker.terminalProfile ? [broker.terminalProfile] : [])
                  }
                  busy={busy}
                  run={run}
                />
              ))}
            </SettingsSection>

            {pendingOperationRequests}

            <SettingsSection title="Active sessions">
              {!broker.sessions.length && (
                <p className="settings-help">No active sessions.</p>
              )}
              {broker.sessions.map((session) => (
                <article className="agent-control-request" key={session.id}>
                  <h3>{session.clientLabel}</h3>
                  <p>
                    {session.workspaceIds
                      .map(
                        (id) =>
                          broker.workspaces.find(
                            (workspace) => workspace.id === id,
                          )?.name ?? "Unavailable workspace",
                      )
                      .join(", ")}
                  </p>
                  <details className="settings-disclosure agent-control-details">
                    <DisclosureSummary>Access details</DisclosureSummary>
                    <div className="agent-control-details-content">
                      <dl>
                        <dt>Granted access</dt>
                        <dd>
                          {session.scopes.join(", ") || "No additional scopes"}
                        </dd>
                        {session.scopes.includes("terminal.execute") &&
                          session.terminalProfile && (
                            <>
                              <dt>Shell</dt>
                              <dd>{session.terminalProfile.id}</dd>
                            </>
                          )}
                        {session.chatConversations?.length > 0 && (
                          <>
                            <dt>Chat conversations</dt>
                            <dd>{session.chatConversations.length}</dd>
                          </>
                        )}
                        {session.androidPackages?.length > 0 && (
                          <>
                            <dt>Android apps</dt>
                            <dd>{session.androidPackages.join(", ")}</dd>
                          </>
                        )}
                        {session.browserOrigins.length > 0 && (
                          <>
                            <dt>Browser origins</dt>
                            <dd>{session.browserOrigins.join(", ")}</dd>
                          </>
                        )}
                      </dl>
                    </div>
                  </details>
                </article>
              ))}
            </SettingsSection>
            {(broker.sessions.length > 0 || requestCount > 0) && (
              <SettingRow
                label="Stop all access"
                description="Ends every session and rejects pending requests. The server stays on."
              >
                <button
                  type="button"
                  className="button"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () => api("agent_control_revoke"),
                      "All sessions stopped and pending requests rejected.",
                    )
                  }
                >
                  Stop all sessions
                </button>
              </SettingRow>
            )}
            <p className="settings-footnote">
              Access ends when the connection closes or Lomi restarts. If this
              view stops responding, stop access from the Lomi menu. Client
              names are labels, not verified identities.
            </p>
          </>
        )}
      </section>

      <section
        className="agent-control-panel"
        id="agent-control-panel-preferences"
        role="tabpanel"
        aria-labelledby="agent-control-tab-preferences"
        tabIndex={0}
        hidden={activeTab !== "preferences"}
      >
        <SettingsSection title="Server preferences">
          <SettingRow
            label="Start MCP server when Lomi opens"
            htmlFor="agent-control-auto-start"
            description="Takes effect the next time Lomi opens."
            descriptionId="agent-control-auto-start-help"
          >
            <input
              id="agent-control-auto-start"
              className="settings-switch"
              type="checkbox"
              role="switch"
              aria-describedby="agent-control-auto-start-help"
              checked={startupState?.autoStart === true}
              disabled={
                !native ||
                !startupState ||
                !startupState.supported ||
                startupBusy
              }
              onChange={(event) => void setAutomaticStart(event.target.checked)}
            />
          </SettingRow>
          <SettingRow
            label="Automatically approve requests (YOLO)"
            htmlFor="agent-control-yolo-mode"
            description="Approves requests from all local clients without asking. Active sessions reconnect."
            descriptionId="agent-control-yolo-mode-help"
          >
            <input
              id="agent-control-yolo-mode"
              ref={yoloSwitch}
              className="settings-switch"
              type="checkbox"
              role="switch"
              aria-label="YOLO mode"
              aria-describedby="agent-control-yolo-mode-help"
              checked={startupState?.yoloMode === true}
              disabled={
                !native ||
                !startupState ||
                !startupState.supported ||
                startupBusy
              }
              onChange={(event) => {
                if (event.target.checked) setYoloConfirmationOpen(true);
                else void setYoloMode(false);
              }}
            />
          </SettingRow>
        </SettingsSection>

        {state?.supported && (
          <>
            <details className="settings-disclosure agent-control-details">
              <DisclosureSummary>Manual configuration</DisclosureSummary>
              <div className="settings-disclosure-body agent-control-details-content">
                {config ? (
                  <>
                    <label htmlFor="control-config">
                      MCP configuration for this server
                    </label>
                    <textarea
                      id="control-config"
                      className="agent-control-config"
                      readOnly
                      value={config}
                      rows={13}
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="button"
                      onClick={() =>
                        void run(
                          () => writeText(config),
                          "Configuration copied.",
                        )
                      }
                    >
                      Copy configuration
                    </button>
                  </>
                ) : broker ? (
                  <p className="settings-help">
                    Manual setup needs the optional lomi-mcp executable next to
                    the app. Automatic setup is in Connect an agent.
                  </p>
                ) : (
                  <p className="settings-help">
                    Start the server to generate a configuration. Automatic
                    setup is in Connect an agent.
                  </p>
                )}
                <p className="settings-help">
                  It grants no access by itself. Copy it again after restarting
                  the server.
                </p>
              </div>
            </details>

            <details className="settings-disclosure agent-control-details">
              <DisclosureSummary>File recovery</DisclosureSummary>
              <div className="settings-disclosure-body agent-control-details-content">
                <p className="settings-help">
                  Files from an interrupted move to Trash stay in the
                  operation’s “entry” folder, and “plan.json” records their
                  original path. Restore to an unused name to keep newer work.
                  “completed.json” means the item reached the Trash.
                </p>
                <button
                  type="button"
                  className="button"
                  disabled={!native || busy}
                  onClick={() =>
                    void run(
                      () => api("agent_control_open_recovery"),
                      (opened) =>
                        opened
                          ? "Recovery folder opened."
                          : "No file recovery data has been created.",
                    )
                  }
                >
                  Show recovery folder
                </button>
              </div>
            </details>
          </>
        )}
      </section>
    </SettingsPage>
  );
}
