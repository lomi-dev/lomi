import { McpClientsSettings } from "./McpClientsSettings";
import { AgentBrowserUploadApproval } from "./AgentBrowserUploadApproval";
import { AgentAndroidApproval } from "./AgentAndroidApproval";
import Pairing from "./AgentControlPairing";
import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import { useT } from "./i18n";
import { Modal } from "./ui";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";
import type { ControlState } from "./agent-control";
import type { ControlStartupState } from "./agent-control-startup";
import { formatShortcut } from "./keybindings";
import "./agent-control-settings.css";

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
  const { t } = useT();
  const name = (id: unknown) => (id === null ? "Lomi" : String(id));
  const appearance = (value: unknown) =>
    value === "system"
      ? t("Follow system")
      : value === "light"
        ? t("Light")
        : t("Dark");
  return (
    <dl>
      <dt>{t("Color theme")}</dt>
      <dd>
        {name(before.active)} → {name(after.active)}
      </dd>
      <dt>{t("Appearance preference")}</dt>
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
  const { t } = useT();
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
          ? t("Restore default shortcut")
          : next.shortcut === null
            ? t("Disable shortcut")
            : t("Assign shortcut")}
      </dt>
      <dd>
        {action.label} (<code>{action.id}</code>)
      </dd>
      <dt>{t("Shortcut")}</dt>
      <dd>
        <code>
          {action.shortcut === null
            ? t("Disabled")
            : formatShortcut(action.shortcut)}
        </code>{" "}
        →{" "}
        <code>
          {next.shortcut === null
            ? t("Disabled")
            : formatShortcut(next.shortcut)}
        </code>
      </dd>
    </dl>
  ) : (
    <dl>
      <dt>{t("Focus follows pointer")}</dt>
      <dd>
        {before.focusFollowsPointer ? t("On") : t("Off")} →{" "}
        {after.focusFollowsPointer ? t("On") : t("Off")}
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
  const { t } = useT();
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
      ? t("Theme default")
      : typeof value === "boolean"
        ? value
          ? t("On")
          : t("Off")
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
          {t(
            "Reducing scrollback removes older lines beyond the new limit from terminal displays.",
          )}
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
                {change.path.includes(".colors.") ? t("Color: ") : ""}
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
    <p>{t("The requested value is already selected.")}</p>
  );
}

export default function AgentControlSettingsPage({
  active = true,
}: {
  active?: boolean;
}) {
  const { t, tp } = useT();
  const [state, setState] = useState<ControlState>();
  const [startupState, setStartupState] = useState<ControlStartupState>();
  const [error, setError] = useState("");
  const [startupError, setStartupError] = useState("");
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [startupBusy, setStartupBusy] = useState(false);
  const [yoloConfirmationOpen, setYoloConfirmationOpen] = useState(false);
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
    if (!native || !active) return;
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
  }, [acceptControlState, enabled, active]);
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
        <SettingsSection title={t("Terminal input requests")} grouped={false}>
          {broker.pendingControls.map((request) => (
            <article
              className="agent-control-request"
              key={request.operationId}
              data-control-operation={request.operationId}
            >
              <h3>{request.title}</h3>
              <p>
                {t(
                  "{client} requests permission to type in this running terminal. Existing text and processes will remain in place.",
                  { client: request.clientLabel },
                )}
              </p>
              <p className="settings-help">
                {t(
                  "Workspace: {workspace}. Request expires in {seconds} seconds. Typing in the terminal takes control back.",
                  {
                    workspace:
                      broker.workspaces.find(
                        (w) => w.id === request.workspaceId,
                      )?.name ?? t("Unavailable"),
                    seconds: request.secondsRemaining,
                  },
                )}
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
                      t("Terminal input request denied."),
                    )
                  }
                >
                  {t("Deny input")}
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
                      t("Terminal input decision recorded."),
                    )
                  }
                >
                  {t("Allow input")}
                </button>
              </div>
            </article>
          ))}
        </SettingsSection>
      )}
      {!!broker.pendingSettingsUpdates?.length && (
        <SettingsSection
          title={t("Preference change requests")}
          grouped={false}
        >
          {broker.pendingSettingsUpdates.map((request) => (
            <article
              className="agent-control-request"
              key={request.operationId}
              data-settings-operation={request.operationId}
            >
              <h3>
                {request.clientLabel} —{" "}
                {request.section === "terminal"
                  ? t("Terminal preferences")
                  : request.section === "keybinds"
                    ? t("Keyboard shortcuts")
                    : request.section === "themes"
                      ? t("Theme preferences")
                      : t("Editor defaults")}
              </h3>
              <p>
                {t("This changes application defaults across all projects.")}{" "}
                {request.section === "terminal"
                  ? t("Running terminals keep their processes.")
                  : request.section === "keybinds"
                    ? t("Shortcut changes apply to every window.")
                    : request.section === "themes"
                      ? t("Running terminals and editor text are retained.")
                      : t("Existing buffer overrides remain in place.")}
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
                  <dt>{t("Tab size")}</dt>
                  <dd>
                    {request.before.tabSize} → {request.after.tabSize}
                  </dd>
                  <dt>{t("Indentation")}</dt>
                  <dd>
                    {request.before.insertSpaces ? t("Spaces") : t("Tabs")} →{" "}
                    {request.after.insertSpaces ? t("Spaces") : t("Tabs")}
                  </dd>
                </dl>
              )}
              <p className="settings-help">
                {t(
                  "Operation {operation}; request {request}. Expires in {seconds} s. A concurrent preference change cancels this write.",
                  {
                    operation: request.operationId,
                    request: request.requestKey,
                    seconds: request.secondsRemaining,
                  },
                )}
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
                      t("Preference change rejected."),
                    )
                  }
                >
                  {t("Reject change")}
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
                      t("Preference change applied."),
                    )
                  }
                >
                  {t("Apply change")}
                </button>
              </div>
            </article>
          ))}
        </SettingsSection>
      )}
      {!!broker.pendingProjectOpens?.length && (
        <SettingsSection title={t("Project folder requests")} grouped={false}>
          {broker.pendingProjectOpens.map((request) => (
            <article
              className="agent-control-request"
              key={request.operationId}
            >
              <h3>
                {t("{client} — Open project folder", {
                  client: request.clientLabel,
                })}
              </h3>
              <p>
                {t("Folder:")} <code>{request.projectPath}</code>
              </p>
              <p>
                {t("Initial workspace:")}{" "}
                <strong>{request.workspaceName}</strong>
              </p>
              <p>
                {t(
                  "This project will share the connection’s approved permissions:",
                )}
              </p>
              <p>{request.scopes.join(", ")}</p>
              <p className="settings-help">
                {t(
                  "A blank editor opens first. Any terminal execution permission runs with your user account permissions; the folder is not a sandbox.",
                )}
              </p>
              <p className="settings-help">
                {t(
                  "Operation {operation}; request {request}. Expires in {seconds} s.",
                  {
                    operation: request.operationId,
                    request: request.requestKey,
                    seconds: request.secondsRemaining,
                  },
                )}
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
                      t("Project request rejected."),
                    )
                  }
                >
                  {t("Reject folder")}
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
                      t("Project folder approved for this request."),
                    )
                  }
                >
                  {t("Approve folder")}
                </button>
              </div>
            </article>
          ))}
        </SettingsSection>
      )}
      {!!broker.pendingAndroidManagement?.length && (
        <SettingsSection
          title={t("Android setup and device requests")}
          grouped={false}
        >
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
        <SettingsSection title={t("APK installation requests")} grouped={false}>
          {broker.pendingInstalls.map((request) => (
            <article
              className="agent-control-request"
              key={request.operationId}
            >
              <h3>
                {t("{client} — Install APK on {device}", {
                  client: request.clientLabel,
                  device: request.title,
                })}
              </h3>
              <p>
                {request.relativePath} (
                {t("{bytes} bytes", {
                  bytes: request.byteLength.toLocaleString(),
                })}
                )
              </p>
              <p className="settings-help">
                {t("Workspace:")}{" "}
                {broker.workspaces.find((w) => w.id === request.workspaceId)
                  ?.name ?? t("Unavailable workspace")}
                <br />
                {t("Device:")} <code>{request.deviceId}</code>
                <br />
                {t("Running instance:")} <code>{request.generation}</code>
                <br />
                SHA-256: <code>{request.sha256}</code>
                <br />
                {t("Expires in {seconds} seconds", {
                  seconds: request.secondsRemaining,
                })}
              </p>
              <p className="settings-help">
                {t(
                  "Install this completed private copy. It may update an existing app and retain its data. Lomi will not uninstall an app to resolve an installation failure.",
                )}
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
                      t("APK installation denied."),
                    )
                  }
                >
                  {t("Deny installation")}
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
                      t("APK installation approved."),
                    )
                  }
                >
                  {t("Install this APK")}
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
    ? t("Checking…")
    : broker
      ? broker.uiReady
        ? t("Ready")
        : t("Waiting for workspace")
      : t("Off");
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
      title={t("Agent control")}
      className="agent-control-page"
      status={status}
    >
      {startupState?.yoloMode && (
        <SettingsNotice tone="warning" role="note">
          <strong>{t("YOLO mode is on.")}</strong>{" "}
          {t(
            "Local clients pair automatically and Lomi approves their requests in every workspace.",
          )}
        </SettingsNotice>
      )}
      {startupError && !yoloConfirmationOpen && (
        <SettingsNotice tone="error">{startupError}</SettingsNotice>
      )}
      {startupState?.error && (
        <SettingsNotice tone="error">
          {startupState.autoStart === true
            ? t(
                "The server couldn’t start automatically: {error} Automatic start stays on.",
                { error: startupState.error },
              )
            : startupState.error}
        </SettingsNotice>
      )}
      {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
      {state && !state.supported && (
        <SettingsNotice tone="warning">
          {t("Agent control isn’t available on this computer yet.")}
        </SettingsNotice>
      )}

      <div
        className="agent-control-tabs"
        role="tablist"
        aria-label={t("Agent control")}
      >
        <button
          type="button"
          className="agent-control-tab"
          {...tabButtons("connect")}
        >
          {t("Connect an agent")}
        </button>
        <button
          type="button"
          className="agent-control-tab"
          {...tabButtons("sessions")}
        >
          {t("Sessions")}
          {requestCount > 0 && (
            <span
              className="agent-control-tab-count"
              aria-label={tp(
                requestCount,
                "{count} pending request",
                "{count} pending requests",
              )}
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
          {t("Preferences")}
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
              {t("Review requests")}
            </button>
          }
        >
          {tp(
            requestCount,
            "{count} request needs review.",
            "{count} requests need review.",
          )}
        </SettingsNotice>
      )}

      {activeTab === "sessions" && (
        <SettingsSection
          title={t("Local connection")}
          className="agent-control-server-section"
        >
          <SettingRow label={t("Connection server")}>
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
                void run(() =>
                  api("agent_control_enable", { enabled: !broker }),
                )
              }
            >
              {broker ? t("Turn off server") : t("Start server")}
            </button>
          </SettingRow>
        </SettingsSection>
      )}

      {yoloConfirmationOpen && (
        <Modal
          protectTheme
          role="alertdialog"
          tone="warning"
          title={t("Enable YOLO mode?")}
          descriptionId="agent-control-yolo-confirmation-description"
          initialFocus={yoloCancelButton}
          closeDisabled={startupBusy}
          onClose={() => {
            if (!startupBusy) setYoloConfirmationOpen(false);
          }}
        >
          <div className="dialog-form dialog-layout" aria-busy={startupBusy}>
            <div className="dialog-content">
              <p id="agent-control-yolo-confirmation-description">
                {t(
                  "Local MCP clients will pair automatically, and their requests run in every workspace without asking: terminal commands, file changes, Git, browser, and Android. Active sessions disconnect and must reconnect.",
                )}
              </p>
              {startupError && (
                <div className="keybindings-error" role="alert">
                  {startupError}
                </div>
              )}
            </div>
            <div className="dialog-actions">
              <button
                ref={yoloCancelButton}
                type="button"
                className="button"
                disabled={startupBusy}
                onClick={() => setYoloConfirmationOpen(false)}
              >
                {t("Cancel")}
              </button>
              <button
                type="button"
                className="button button-primary"
                disabled={startupBusy}
                onClick={() => void confirmYoloMode()}
              >
                {startupBusy ? t("Saving…") : t("Enable YOLO mode")}
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
            serverEnabled={!!broker}
            serverStatus={serverStatus}
            serverBusy={busy}
            canStartServer={native && state.supported}
            onStartServer={async () => {
              if (!native || !state.supported || busy || broker) return;
              await run(() => api("agent_control_enable", { enabled: true }));
            }}
            onInstalled={() =>
              refresh().catch((cause) => setError(errorMessage(cause)))
            }
          />
        ) : state ? null : (
          <p className="settings-help" role="status">
            {t("Loading…")}
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
            <h2>{t("No active sessions")}</h2>
            <p>
              {t(
                "Start the server above, then connect an agent to review its access request here.",
              )}
            </p>
            <button
              type="button"
              className="button"
              onClick={() => selectTab("connect")}
            >
              {t("Connect an agent")}
            </button>
          </div>
        ) : (
          <>
            <SettingsSection title={t("New session requests")} grouped={false}>
              {!broker.pending.length && (
                <p className="settings-help">
                  {t("No client is waiting to connect.")}
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

            <SettingsSection title={t("Active sessions")} grouped={false}>
              {!broker.sessions.length && (
                <p className="settings-help">{t("No active sessions.")}</p>
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
                          )?.name ?? t("Unavailable workspace"),
                      )
                      .join(", ")}
                  </p>
                  <div className="agent-control-details-content">
                    <dl>
                      <dt>{t("Granted access")}</dt>
                      <dd>
                        {session.scopes.join(", ") || t("No additional scopes")}
                      </dd>
                      {session.scopes.includes("terminal.execute") &&
                        session.terminalProfile && (
                          <>
                            <dt>{t("Shell")}</dt>
                            <dd>{session.terminalProfile.id}</dd>
                          </>
                        )}
                      {session.androidPackages?.length > 0 && (
                        <>
                          <dt>{t("Android apps")}</dt>
                          <dd>{session.androidPackages.join(", ")}</dd>
                        </>
                      )}
                      {session.browserOrigins.length > 0 && (
                        <>
                          <dt>{t("Browser origins")}</dt>
                          <dd>{session.browserOrigins.join(", ")}</dd>
                        </>
                      )}
                    </dl>
                  </div>
                </article>
              ))}
            </SettingsSection>
            {(broker.sessions.length > 0 || requestCount > 0) && (
              <SettingRow
                label={t("Stop all access")}
                description={t(
                  "Ends every session and rejects pending requests. The server stays on.",
                )}
              >
                <button
                  type="button"
                  className="button"
                  disabled={busy}
                  onClick={() =>
                    void run(
                      () => api("agent_control_revoke"),
                      t("All sessions stopped and pending requests rejected."),
                    )
                  }
                >
                  {t("Stop all sessions")}
                </button>
              </SettingRow>
            )}
            <p className="settings-footnote">
              {t("Client names are labels, not verified identities.")}
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
        <SettingsSection title={t("Server preferences")}>
          <SettingRow
            label={t("Start MCP server when Lomi opens")}
            htmlFor="agent-control-auto-start"
          >
            <input
              id="agent-control-auto-start"
              className="settings-switch"
              type="checkbox"
              role="switch"
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
        </SettingsSection>
        <SettingsSection
          title={t("Automatic approvals (YOLO)")}
          className="agent-control-automatic-approvals"
        >
          <SettingRow
            label={t("Automatically approve requests (YOLO)")}
            htmlFor="agent-control-yolo-mode"
            description={t(
              "Approves requests from all local clients without asking. Active sessions reconnect.",
            )}
            descriptionId="agent-control-yolo-mode-help"
          >
            <input
              id="agent-control-yolo-mode"
              ref={yoloSwitch}
              className="settings-switch"
              type="checkbox"
              role="switch"
              aria-label={t("YOLO mode")}
              aria-describedby="agent-control-yolo-mode-help"
              checked={startupState?.yoloMode === true}
              disabled={
                !native ||
                !startupState ||
                !startupState.supported ||
                startupBusy
              }
              onChange={(event) => {
                if (event.target.checked) {
                  event.currentTarget.focus();
                  setYoloConfirmationOpen(true);
                } else void setYoloMode(false);
              }}
            />
          </SettingRow>
        </SettingsSection>

        {state?.supported && (
          <SettingsSection title={t("Manual configuration")}>
            <div className="agent-control-details-content">
              {config ? (
                <>
                  <label htmlFor="control-config">
                    {t("MCP configuration for this server")}
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
                        t("Configuration copied."),
                      )
                    }
                  >
                    {t("Copy configuration")}
                  </button>
                </>
              ) : broker ? (
                <p className="settings-help">
                  {t(
                    "Manual setup needs the optional lomi-mcp executable next to the app. Automatic setup is in Connect an agent.",
                  )}
                </p>
              ) : (
                <p className="settings-help">
                  {t(
                    "Start the server to generate a configuration. Automatic setup is in Connect an agent.",
                  )}
                </p>
              )}
            </div>
          </SettingsSection>
        )}
      </section>
    </SettingsPage>
  );
}
