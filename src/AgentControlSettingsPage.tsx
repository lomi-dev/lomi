import { AgentAndroidApproval } from "./AgentAndroidApproval";
import { AgentChatPermission } from "./AgentChatPermission";
import { Fragment, useCallback, useEffect, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import type { ControlState, ControlWorkspace } from "./agent-control";
import { formatShortcut } from "./keybindings";

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
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [visible, setVisible] = useState(true);
  const enabled = Boolean(state?.broker);
  useEffect(() => {
    if (!native) return;
    const stop = listen<boolean>("settings-visibility", ({ payload }) =>
      setVisible(payload),
    );
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);
  const refresh = useCallback(async () => {
    setState(await api<ControlState>("agent_control_state"));
  }, []);
  useEffect(() => {
    if (!native || !visible) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const next = await api<ControlState>("agent_control_state");
        if (alive) setState(next);
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
  }, [enabled, visible]);
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
  return (
    <main className="keybindings-page agent-control-page">
      <header className="settings-page-heading">
        <div>
          <h1>Agent control</h1>
          <p>Connect a local MCP client to selected Lomi workspaces.</p>
        </div>
      </header>
      <p className="settings-help">
        Access lasts for this connection. Restarting Lomi or its workspace view
        ends every grant. Enable control, configure your client, then approve
        its pending session here.
      </p>
      {error && (
        <div className="keybindings-error" role="alert">
          {error}
        </div>
      )}
      <div className="keybindings-status" role="status">
        {status ||
          (!state
            ? "Loading agent control…"
            : broker
              ? broker.uiReady
                ? "Ready for pairing"
                : "Waiting for the workspace window"
              : "Agent control is off")}
      </div>
      <div className="agent-control-actions">
        <button
          className="button"
          disabled={!native || !state?.supported || busy}
          onClick={() =>
            void run(() => api("agent_control_enable", { enabled: !broker }))
          }
        >
          {broker ? "Disable agent control" : "Enable for this Lomi session"}
        </button>
        {broker && (
          <button
            className="button"
            disabled={busy}
            onClick={() =>
              void run(
                () => api("agent_control_revoke"),
                "All sessions stopped and pending requests rejected.",
              )
            }
          >
            Stop agent control
          </button>
        )}
      </div>
      {state && !state.supported && (
        <p className="settings-help">
          This host has not been qualified for local agent control.
        </p>
      )}
      {state?.supported && (
        <section
          className="keybindings-group"
          aria-labelledby="agent-recovery-heading"
        >
          <h2 id="agent-recovery-heading">File recovery</h2>
          <p className="settings-help">
            Interrupted Trash operations keep recoverable files in an
            operation’s “entry” folder. Its “plan.json” records the original
            project and path. Restore to an unused name to preserve newer work.
            A “completed.json” record means the item reached the system Trash.
          </p>
          <button
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
        </section>
      )}
      {broker && (
        <>
          {!!broker.pendingControls.length && (
            <section
              className="keybindings-group"
              aria-labelledby="control-terminals-heading"
            >
              <h2 id="control-terminals-heading">Terminal input requests</h2>
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
                    . Request expires in {request.secondsRemaining} seconds.
                    Typing in the terminal takes control back.
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
            </section>
          )}
          {!!broker.pendingSettingsUpdates?.length && (
            <section
              className="keybindings-group"
              aria-labelledby="control-settings-updates-heading"
            >
              <h2 id="control-settings-updates-heading">
                Preference change requests
              </h2>
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
                    Operation {request.operationId}; request{" "}
                    {request.requestKey}. Expires in {request.secondsRemaining}{" "}
                    s. A concurrent preference change cancels this write.
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
            </section>
          )}
          {!!broker.pendingProjectOpens?.length && (
            <section
              className="keybindings-group"
              aria-labelledby="control-project-opens-heading"
            >
              <h2 id="control-project-opens-heading">
                Project folder requests
              </h2>
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
                    This project will share the connection’s approved
                    permissions:
                  </p>
                  <p>{request.scopes.join(", ")}</p>
                  <p className="settings-help">
                    A blank editor opens first. Any terminal execution
                    permission runs with your user account permissions; the
                    folder is not a sandbox.
                  </p>
                  <p className="settings-help">
                    Operation {request.operationId}; request{" "}
                    {request.requestKey}. Expires in {request.secondsRemaining}
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
            </section>
          )}
          {!!broker.pendingAndroidManagement?.length && (
            <section
              className="keybindings-group"
              aria-label="Android setup and device requests"
            >
              <h2>Android setup and device requests</h2>
              {broker.pendingAndroidManagement.map((request) => (
                <AgentAndroidApproval
                  key={`${request.operationId}:${request.plan.revision}`}
                  request={request}
                  busy={busy}
                  run={run}
                />
              ))}
            </section>
          )}
          {!!broker.pendingInstalls?.length && (
            <section
              className="keybindings-group"
              aria-labelledby="control-installs-heading"
            >
              <h2 id="control-installs-heading">APK installation requests</h2>
              {broker.pendingInstalls.map((request) => (
                <article
                  className="agent-control-request"
                  key={request.operationId}
                >
                  <h3>
                    {request.clientLabel} — Install APK on {request.title}
                  </h3>
                  <p>
                    {request.relativePath} (
                    {request.byteLength.toLocaleString()} bytes)
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
                    Install this completed private copy. It may update an
                    existing app and retain its data. Lomi will not uninstall an
                    app to resolve an installation failure.
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
            </section>
          )}
          <section
            className="keybindings-group"
            aria-labelledby="control-config-heading"
          >
            <h2 id="control-config-heading">Client configuration</h2>
            {config ? (
              <>
                <label htmlFor="control-config">
                  MCP JSON configuration for this running instance
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
                  className="button"
                  onClick={() =>
                    void run(() => writeText(config), "Configuration copied.")
                  }
                >
                  Copy configuration
                </button>
              </>
            ) : (
              <p className="settings-help">
                The lomi-mcp executable is missing beside the application. Build
                or install the matching helper before configuring your client.
              </p>
            )}
            <p className="settings-help">
              This configuration includes the public identity of this instance.
              It grants no access by itself. Use a fresh configuration after
              restarting Lomi or disabling control.
            </p>
          </section>
          <section
            className="keybindings-group"
            aria-labelledby="control-pending-heading"
          >
            <h2 id="control-pending-heading">Pending sessions</h2>
            {!broker.pending.length && (
              <p className="settings-help">
                No client is waiting for approval.
              </p>
            )}
            {broker.pending.map((request) => (
              <Pairing
                key={request.id}
                request={request}
                workspaces={broker.workspaces}
                busy={busy}
                run={run}
              />
            ))}
          </section>
          <section
            className="keybindings-group"
            aria-labelledby="control-sessions-heading"
          >
            <h2 id="control-sessions-heading">Active sessions</h2>
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
                        broker.workspaces.find((w) => w.id === id)?.name ??
                        "Unavailable workspace",
                    )
                    .join(", ")}
                </p>
                <p className="settings-help">
                  Granted: {session.scopes.join(", ")}
                  {session.chatConversations?.length > 0 && (
                    <>
                      {" "}
                      · Chat conversations: {session.chatConversations.length}
                    </>
                  )}
                  {session.androidPackages?.length > 0 && (
                    <> · Android apps: {session.androidPackages.join(", ")}</>
                  )}
                  {session.browserOrigins.length > 0 && (
                    <>
                      <br />
                      Browser origins: {session.browserOrigins.join(", ")}
                    </>
                  )}
                </p>
              </article>
            ))}
          </section>
          <p className="settings-help">
            The client name is a label, not proof of identity. All processes
            using its stdio channel share this authorization. You can also stop
            access from the native Lomi menu if the workspace view stops
            responding.
          </p>
        </>
      )}
    </main>
  );
}

function Pairing({
  request,
  workspaces,
  busy,
  run,
}: {
  request: NonNullable<ControlState["broker"]>["pending"][number];
  workspaces: ControlWorkspace[];
  busy: boolean;
  run: (action: () => Promise<unknown>, message?: string) => Promise<void>;
}) {
  const [workspaceId, setWorkspaceId] = useState("");
  const [extraWorkspaceIds, setExtraWorkspaceIds] = useState<string[]>([]);
  const [readChat, setReadChat] = useState(false);
  const [sendChat, setSendChat] = useState(false);
  const [stopChat, setStopChat] = useState(false);
  const [exportChat, setExportChat] = useState(false);
  const [draftChat, setDraftChat] = useState(false);
  const [openChat, setOpenChat] = useState(false);
  const [createChat, setCreateChat] = useState(false);
  const [chatConversations, setChatConversations] = useState<string[]>([]);
  const [readSettings, setReadSettings] = useState(false);
  const [writeSettings, setWriteSettings] = useState(false);
  const [openSettings, setOpenSettings] = useState(false);
  const [writeWorkspace, setWriteWorkspace] = useState(false);
  const [executeTerminal, setExecuteTerminal] = useState(false);
  const [closeWorkspaces, setCloseWorkspaces] = useState(false);
  const [closeProjects, setCloseProjects] = useState(false);
  const [openProjects, setOpenProjects] = useState(false);
  const [movePanels, setMovePanels] = useState(false);
  const [managePanels, setManagePanels] = useState(false);
  const [navigateBrowser, setNavigateBrowser] = useState(false);
  const [captureBrowser, setCaptureBrowser] = useState(false);
  const [interactBrowser, setInteractBrowser] = useState(false);
  const [readBrowser, setReadBrowser] = useState(false);
  const [readAndroid, setReadAndroid] = useState(false);
  const [setupAndroid, setSetupAndroid] = useState(false);
  const [manageAndroid, setManageAndroid] = useState(false);
  const [controlAndroid, setControlAndroid] = useState(false);
  const [interactAndroid, setInteractAndroid] = useState(false);
  const [observeAndroid, setObserveAndroid] = useState(false);
  const [launchAndroid, setLaunchAndroid] = useState(false);
  const [logsAndroid, setLogsAndroid] = useState(false);
  const [androidPackages, setAndroidPackages] = useState("");
  const [installApk, setInstallApk] = useState(false);
  const [importApk, setImportApk] = useState(false);
  const [readFiles, setReadFiles] = useState(false);
  const [readGit, setReadGit] = useState(false);
  const [writeGit, setWriteGit] = useState(false);
  const [networkGit, setNetworkGit] = useState(false);
  const [pushGit, setPushGit] = useState(false);
  const [discardGit, setDiscardGit] = useState(false);
  const [pullGit, setPullGit] = useState(false);
  const [readBuffers, setReadBuffers] = useState(false);
  const [writeBuffers, setWriteBuffers] = useState(false);
  const [saveFiles, setSaveFiles] = useState(false);
  const [createFiles, setCreateFiles] = useState(false);
  const [renameFiles, setRenameFiles] = useState(false);
  const [trashFiles, setTrashFiles] = useState(false);
  const [captureAndroid, setCaptureAndroid] = useState(false);
  const [androidDevices, setAndroidDevices] = useState<
    { id: string; name: string }[]
  >([]);
  const [androidDevice, setAndroidDevice] = useState("");
  const [androidError, setAndroidError] = useState("");
  useEffect(() => {
    if (!readAndroid) return;
    let alive = true;
    setAndroidError("");
    void api<{ devices: { devices: { id: string; name: string }[] } | null }>(
      "android_state",
    )
      .then((state) => {
        if (alive) setAndroidDevices(state.devices?.devices ?? []);
      })
      .catch((error) => {
        if (alive) setAndroidError(errorMessage(error));
      });
    return () => {
      alive = false;
    };
  }, [readAndroid]);
  const [browserOrigins, setBrowserOrigins] = useState("");
  const valid = workspaces.some((w) => w.id === workspaceId);
  const selectedProject = workspaces.find(
    (w) => w.id === workspaceId,
  )?.projectId;
  const additionalWorkspaces = workspaces.filter(
    (w) => w.projectId === selectedProject && w.id !== workspaceId,
  );
  const allProjectWorkspaces =
    valid &&
    additionalWorkspaces.every((w) => extraWorkspaceIds.includes(w.id));
  return (
    <article className="agent-control-request">
      <h3>{request.clientLabel}</h3>
      <p className="settings-help">
        Expires in {request.secondsRemaining} seconds
      </p>
      <p className="settings-help">
        Match this request ID with the pairingRequestId returned by lomi_status
        in your client before approving.
      </p>
      <p className="settings-help">
        Request: <code>{request.id}</code>
        <br />
        Client certificate: <code>{request.certificateSha256}</code>
      </p>
      <label htmlFor={`control-workspace-${request.id}`}>Workspace</label>
      <select
        id={`control-workspace-${request.id}`}
        value={workspaceId}
        onChange={(event) => {
          setWorkspaceId(event.target.value);
          setReadChat(false);
          setOpenChat(false);
          setDraftChat(false);
          setSendChat(false);
          setStopChat(false);
          setExportChat(false);
          setCreateChat(false);
          setChatConversations([]);
          setExtraWorkspaceIds([]);
          setCloseProjects(false);
        }}
        disabled={busy}
      >
        <option value="">Choose a workspace</option>
        {workspaces.map((w) => (
          <option key={w.id} value={w.id}>
            {w.projectName} / {w.name}
          </option>
        ))}
      </select>
      {valid && (
        <p className="settings-help">
          Project folder:{" "}
          <code>
            {workspaces.find((w) => w.id === workspaceId)?.projectPath}
          </code>
        </p>
      )}
      {additionalWorkspaces.length > 0 && (
        <fieldset>
          <legend>Other workspaces in this project</legend>
          {additionalWorkspaces.map((workspace) => (
            <label key={workspace.id}>
              <input
                type="checkbox"
                disabled={busy}
                checked={extraWorkspaceIds.includes(workspace.id)}
                onChange={(event) => {
                  const checked = event.target.checked;
                  setExtraWorkspaceIds((ids) =>
                    checked
                      ? [...ids, workspace.id]
                      : ids.filter((id) => id !== workspace.id),
                  );
                  if (!checked) setCloseProjects(false);
                }}
              />{" "}
              {workspace.name}
            </label>
          ))}
          <p className="settings-help">
            Only checked workspaces share this connection’s permissions.
          </p>
        </fieldset>
      )}
      <p className="settings-help">
        Allow workspace metadata reads for this connection.
      </p>
      <label>
        <input
          type="checkbox"
          checked={readChat}
          disabled={busy || !selectedProject}
          onChange={(event) => {
            setReadChat(event.target.checked);
            setOpenChat(false);
            setDraftChat(false);
            setSendChat(false);
            setStopChat(false);
            setExportChat(false);
            setCreateChat(false);
            setChatConversations([]);
          }}
        />
        Allow reading selected Chat AI conversations
      </label>
      {readChat && selectedProject && (
        <AgentChatPermission
          key={selectedProject}
          projectId={selectedProject}
          selected={chatConversations}
          onChange={setChatConversations}
          disabled={busy}
        />
      )}

      <label>
        <input
          type="checkbox"
          checked={openChat}
          disabled={busy || !readChat}
          onChange={(event) => {
            setOpenChat(event.target.checked);
            if (!event.target.checked) setCreateChat(false);
          }}
        />
        Allow opening selected Chat AI conversations
      </label>
      <label>
        <input
          type="checkbox"
          checked={createChat}
          disabled={busy || !readChat || !openChat}
          onChange={(event) => setCreateChat(event.target.checked)}
        />
        Allow creating Chat AI conversations
      </label>
      <label>
        <input
          type="checkbox"
          checked={draftChat}
          disabled={busy || !readChat}
          onChange={(event) => setDraftChat(event.target.checked)}
        />
        Allow editing selected Chat AI drafts
      </label>
      <label>
        <input
          type="checkbox"
          checked={exportChat}
          disabled={busy || !readChat}
          onChange={(e) => setExportChat(e.target.checked)}
        />
        Allow exporting selected Chat AI text
      </label>
      <label>
        <input
          type="checkbox"
          checked={stopChat}
          disabled={busy || !readChat}
          onChange={(e) => setStopChat(e.target.checked)}
        />
        Allow stopping selected Chat AI responses
      </label>
      <label>
        <input
          type="checkbox"
          checked={sendChat}
          disabled={busy || !readChat}
          onChange={(event) => setSendChat(event.target.checked)}
        />
        Allow sending selected Chat AI messages
      </label>
      <p className="settings-help">
        Each send requires your approval of the connection, model, message,
        context and possible provider charges.
      </p>
      <p className="settings-help">
        Draft editing saves text locally in an open conversation. Unsaved human
        text is protected, and messages are not sent.
      </p>
      <p className="settings-help">
        New conversations become readable by this connection. Opening and
        creating do not send messages to a provider.
      </p>
      <label>
        <input
          type="checkbox"
          checked={writeWorkspace}
          disabled={busy}
          onChange={(event) => {
            setWriteWorkspace(event.target.checked);
            if (!event.target.checked) {
              setOpenProjects(false);
              setMovePanels(false);
              setCloseWorkspaces(false);
              setCloseProjects(false);
            }
          }}
        />{" "}
        Allow workspace creation, renaming and selection
      </label>
      <label>
        <input
          type="checkbox"
          checked={openProjects && writeWorkspace}
          disabled={busy || !writeWorkspace}
          onChange={(event) => setOpenProjects(event.target.checked)}
        />{" "}
        Allow requesting access to new project folders
      </label>
      <p className="settings-help">
        Each folder requires a separate approval here for its initial workspace
        and this connection’s permissions. Opening starts with a blank editor.
      </p>
      <label>
        <input
          type="checkbox"
          checked={executeTerminal}
          disabled={busy}
          onChange={(event) => setExecuteTerminal(event.target.checked)}
        />{" "}
        Allow Zsh terminal creation, command execution and output reads
      </label>
      <p className="settings-help">
        Shell commands run with your user account permissions. The project
        folder is a starting directory, not a sandbox.
      </p>
      <label>
        <input
          type="checkbox"
          checked={managePanels}
          disabled={busy}
          onChange={(event) => {
            setManagePanels(event.target.checked);
            if (!event.target.checked) {
              setCloseWorkspaces(false);
              setCloseProjects(false);
            }
          }}
        />{" "}
        Allow selecting and closing panels
      </label>
      <label>
        <input
          type="checkbox"
          checked={movePanels}
          disabled={busy || !writeWorkspace}
          onChange={(event) => setMovePanels(event.target.checked)}
        />{" "}
        Allow rearranging existing panels
      </label>
      <p className="settings-help">
        Requires workspace changes. Docking also requires selecting panels.
        Existing terminal processes and editor buffers are preserved.
      </p>
      <label>
        <input
          type="checkbox"
          checked={closeWorkspaces && writeWorkspace && managePanels}
          disabled={busy || !writeWorkspace || !managePanels}
          onChange={(event) => {
            setCloseWorkspaces(event.target.checked);
            if (!event.target.checked) setCloseProjects(false);
          }}
        />{" "}
        Allow requesting workspace closure
      </label>
      <p className="settings-help">
        Closes approved views without deleting the project folder. Unsaved files
        keep their save/discard/cancel dialog; protected, busy or
        human-controlled terminal sessions are preserved.
      </p>
      <label>
        <input
          type="checkbox"
          checked={
            closeProjects &&
            closeWorkspaces &&
            writeWorkspace &&
            managePanels &&
            allProjectWorkspaces
          }
          disabled={
            busy ||
            !closeWorkspaces ||
            !writeWorkspace ||
            !managePanels ||
            !allProjectWorkspaces
          }
          onChange={(event) => setCloseProjects(event.target.checked)}
        />{" "}
        Allow requesting project closure
      </label>
      <p className="settings-help">
        Requires access to every workspace in this project. A new unapproved
        workspace blocks closure. Closing preserves the project folder and uses
        the same unsaved-file and protected-process checks.
      </p>
      <label>
        <input
          type="checkbox"
          checked={readSettings}
          disabled={busy}
          onChange={(e) => {
            setReadSettings(e.target.checked);
            if (!e.target.checked) setWriteSettings(false);
          }}
        />{" "}
        Allow reading nonsecret application preferences
      </label>
      <p className="settings-help">
        Shares global editor defaults, terminal preferences, shortcut bindings
        and theme selections. Conversation history, credentials and plugin
        source are excluded.
      </p>
      <label>
        <input
          type="checkbox"
          checked={writeSettings}
          disabled={busy || !readSettings}
          onChange={(e) => setWriteSettings(e.target.checked)}
        />{" "}
        Allow requesting application preference changes
      </label>
      <p className="settings-help">
        Requires preference read access. Each change shows its exact before and
        after values in Settings for your approval. Supports editor defaults,
        terminal preferences, keyboard shortcuts and built-in color themes.
      </p>
      <label>
        <input
          type="checkbox"
          checked={openSettings}
          disabled={busy}
          onChange={(event) => setOpenSettings(event.target.checked)}
        />{" "}
        Allow opening Settings sections
      </label>
      <p className="settings-help">
        This may focus the Settings window. Reading and changing preferences
        require separate permissions.
      </p>
      <label>
        <input
          type="checkbox"
          checked={navigateBrowser}
          disabled={busy}
          onChange={(event) => setNavigateBrowser(event.target.checked)}
        />{" "}
        Allow opening and navigating isolated browser panels
      </label>
      <label>
        <input
          type="checkbox"
          checked={readBrowser}
          disabled={busy || !navigateBrowser}
          onChange={(event) => setReadBrowser(event.target.checked)}
        />{" "}
        Allow reading page text, form structure and JavaScript errors
      </label>
      <label>
        <input
          type="checkbox"
          checked={interactBrowser}
          disabled={busy || !navigateBrowser || !readBrowser}
          onChange={(event) => setInteractBrowser(event.target.checked)}
        />{" "}
        Allow clicking, typing and scrolling in pages
      </label>
      <label>
        <input
          type="checkbox"
          checked={captureBrowser}
          disabled={busy || !navigateBrowser}
          onChange={(event) => setCaptureBrowser(event.target.checked)}
        />{" "}
        Allow screenshots of pages, including embedded third-party frames
      </label>
      {navigateBrowser && (
        <>
          <label htmlFor={`control-origins-${request.id}`}>
            Allowed browser origins
          </label>
          <textarea
            id={`control-origins-${request.id}`}
            value={browserOrigins}
            disabled={busy}
            onChange={(event) => setBrowserOrigins(event.target.value)}
            rows={3}
            placeholder="http://localhost:3000"
            spellCheck={false}
          />
          <p className="settings-help">
            One exact origin per line, including its port. These panels use
            separate browser data. Origin restrictions apply to page navigation;
            pages can still request other network resources.
          </p>
        </>
      )}
      <label>
        <input
          type="checkbox"
          checked={readAndroid}
          disabled={busy}
          onChange={(event) => {
            setReadAndroid(event.target.checked);
            if (!event.target.checked) {
              setControlAndroid(false);
              setSetupAndroid(false);
              setManageAndroid(false);
              setLaunchAndroid(false);
              setLogsAndroid(false);
              setInteractAndroid(false);
              setObserveAndroid(false);
              setCaptureAndroid(false);
              setInstallApk(false);
            }
          }}
        />{" "}
        Allow reading selected Android device status
      </label>
      {readAndroid && (
        <>
          <label>
            <input
              type="checkbox"
              checked={setupAndroid}
              disabled={busy}
              onChange={(event) => setSetupAndroid(event.target.checked)}
            />{" "}
            Allow Android SDK setup, recovery and cache cleanup requests
          </label>
          <label>
            <input
              type="checkbox"
              checked={manageAndroid}
              disabled={busy}
              onChange={(event) => setManageAndroid(event.target.checked)}
            />{" "}
            Allow creating devices and changing the selected device
          </label>
          <p className="settings-help">
            Setup and device changes each require a separate approval here.
            Provider licenses and erasing data always require your decision. New
            devices become visible only to the client that created them.
          </p>
          <label htmlFor={`control-android-${request.id}`}>
            Managed Android device
          </label>
          <select
            id={`control-android-${request.id}`}
            value={androidDevice}
            disabled={busy}
            onChange={(event) => setAndroidDevice(event.target.value)}
          >
            <option value="">Choose a managed device</option>
            {androidDevices.map((device) => (
              <option key={device.id} value={device.id}>
                {device.name}
              </option>
            ))}
          </select>
          <p className="settings-help">
            Only this device's name and runtime status will be shared. This
            permission does not start it or allow input.
          </p>
          <label>
            <input
              type="checkbox"
              checked={controlAndroid}
              disabled={busy}
              onChange={(event) => {
                setControlAndroid(event.target.checked);
                if (!event.target.checked) {
                  setLaunchAndroid(false);
                  setLogsAndroid(false);
                  setInteractAndroid(false);
                  setObserveAndroid(false);
                  setCaptureAndroid(false);
                  setInstallApk(false);
                }
              }}
            />
            Allow starting and stopping this Android device
          </label>
          <p className="settings-help">
            Start can boot a stopped phone. An already running phone remains
            under its current control. Human actions take control back.
          </p>
          <label>
            <input
              type="checkbox"
              checked={interactAndroid}
              disabled={busy || !controlAndroid}
              onChange={(event) => setInteractAndroid(event.target.checked)}
            />
            Allow touch, keys and text in this Android device
          </label>
          <p className="settings-help">
            Input requires the selected phone panel in a focused Lomi window.
            Take control returns input to you.
          </p>
          <label>
            <input
              type="checkbox"
              checked={observeAndroid}
              disabled={busy || !controlAndroid}
              onChange={(event) => setObserveAndroid(event.target.checked)}
            />
            Allow reading screen content in this Android device
          </label>
          <p className="settings-help">
            Share a limited UI hierarchy from the phone started by this
            connection. Password and editable field values are omitted.
          </p>
          <label>
            <input
              type="checkbox"
              checked={launchAndroid}
              disabled={busy || !controlAndroid}
              onChange={(event) => setLaunchAndroid(event.target.checked)}
            />
            Allow launching approved Android apps
          </label>
          <label>
            <input
              type="checkbox"
              checked={logsAndroid}
              disabled={busy || !controlAndroid}
              onChange={(event) => setLogsAndroid(event.target.checked)}
            />
            Allow reading logs from approved Android apps
          </label>
          {(launchAndroid || logsAndroid) && (
            <>
              <label htmlFor={`control-packages-${request.id}`}>
                Allowed Android packages
              </label>
              <textarea
                id={`control-packages-${request.id}`}
                value={androidPackages}
                disabled={busy}
                placeholder="org.example.app"
                onChange={(event) => setAndroidPackages(event.target.value)}
              />
              <p className="settings-help">
                Enter up to 16 exact package names, separated by spaces or
                newlines. Launch may open any exported activity in these apps.
                Logs can contain app data and personal information.
              </p>
            </>
          )}
          {androidError && <p role="alert">{androidError}</p>}
          <label>
            <input
              type="checkbox"
              checked={captureAndroid}
              disabled={busy || !controlAndroid}
              onChange={(event) => setCaptureAndroid(event.target.checked)}
            />
            Allow screenshots of this Android device
          </label>
          <p className="settings-help">
            Share the full phone screen, including visible text and fields, as
            individually requested images. This permission does not grant input.
          </p>
        </>
      )}
      <label>
        <input
          type="checkbox"
          checked={readFiles}
          disabled={busy}
          onChange={(event) => {
            setReadFiles(event.target.checked);
            if (!event.target.checked) {
              setReadGit(false);
              setWriteGit(false);
              setDiscardGit(false);
              setNetworkGit(false);
              setPushGit(false);
              setPullGit(false);
              setCreateFiles(false);
              setRenameFiles(false);
              setTrashFiles(false);
              setReadBuffers(false);
              setWriteBuffers(false);
              setSaveFiles(false);
              setImportApk(false);
              setInstallApk(false);
            }
          }}
        />
        Allow reading project files
      </label>
      <p className="settings-help">
        Share text files on disk in this project. Known secret paths and links
        are excluded. This permission does not read unsaved editor buffers or
        allow changes.
      </p>
      <label>
        <input
          type="checkbox"
          checked={readGit}
          disabled={busy || !readFiles}
          onChange={(event) => {
            setReadGit(event.target.checked);
            if (!event.target.checked) {
              setWriteGit(false);
              setDiscardGit(false);
              setNetworkGit(false);
              setPushGit(false);
              setPullGit(false);
            }
          }}
        />
        Allow reading Git information in this project
      </label>
      <p className="settings-help">
        Share supported repository observations. This permission does not allow
        Git changes, network access, hooks or external helpers.
      </p>
      <label>
        <input
          type="checkbox"
          checked={writeGit}
          disabled={busy || !readFiles || !readGit}
          onChange={(event) => {
            setWriteGit(event.target.checked);
            if (!event.target.checked) setDiscardGit(false);
            if (!event.target.checked) {
              setNetworkGit(false);
              setPushGit(false);
              setPullGit(false);
            }
          }}
        />
        Allow requesting Git changes and executing configured Git code
      </label>
      <p className="settings-help">
        Each Git change needs your approval in the main window for the exact
        operation and repository state. Git hooks, helpers and filters can run
        with your account’s access. Only grant this for repositories you trust.
      </p>
      <label>
        <input
          type="checkbox"
          checked={discardGit}
          disabled={busy || !readFiles || !readGit || !writeGit}
          onChange={(event) => setDiscardGit(event.target.checked)}
        />
        Allow requesting discard of working Git changes
      </label>
      <p className="settings-help">
        Each discard shows the exact working changes before approval. It
        restores staged versions without saving the discarded bytes in Git or
        Trash.
      </p>
      <label>
        <input
          type="checkbox"
          checked={networkGit}
          disabled={busy || !readFiles || !readGit || !writeGit}
          onChange={(event) => {
            setNetworkGit(event.target.checked);
            if (!event.target.checked) {
              setPushGit(false);
              setPullGit(false);
            }
          }}
        />
        Allow requesting contact with configured Git remotes
      </label>
      <p className="settings-help">
        Fetch and pull requests show the remote and branch in the main window
        before connecting. Configured transports and credential helpers may run
        after approval. This permission does not allow pushing commits.
      </p>
      <label>
        <input
          type="checkbox"
          checked={pullGit}
          disabled={busy || !readFiles || !readGit || !writeGit || !networkGit}
          onChange={(event) => setPullGit(event.target.checked)}
        />
        Allow requesting Git pulls
      </label>
      <p className="settings-help">
        Each pull needs your approval for the exact incoming commit and mode.
        Pull changes working files; rebase can rewrite local commit history.
        Conflicts stay in the repository for you to resolve.
      </p>
      <label>
        <input
          type="checkbox"
          checked={pushGit}
          disabled={busy || !readFiles || !readGit || !writeGit || !networkGit}
          onChange={(event) => setPushGit(event.target.checked)}
        />
        Allow requesting Git pushes
      </label>
      <p className="settings-help">
        Each push needs your approval for the exact remote branch and commits.
        Push can publish repository content. Only creating a branch or moving it
        forward is allowed; rewriting remote history is excluded.
      </p>
      <label>
        <input
          type="checkbox"
          checked={createFiles}
          disabled={busy || !readFiles}
          onChange={(event) => setCreateFiles(event.target.checked)}
        />
        Allow creating project files and folders
      </label>
      <p className="settings-help">
        Create empty files and folders inside this project. Existing names are
        preserved. Editing and saving file contents require separate
        permissions.
      </p>
      <label>
        <input
          type="checkbox"
          checked={renameFiles}
          disabled={busy || !readFiles}
          onChange={(event) => setRenameFiles(event.target.checked)}
        />
        Allow renaming and moving project files and folders
      </label>
      <p className="settings-help">
        Move files and folders within this project after checking their disk
        versions. Existing destinations are preserved. Open editors keep unsaved
        text and undo history.
      </p>
      <label>
        <input
          type="checkbox"
          checked={trashFiles}
          disabled={busy || !readFiles}
          onChange={(event) => setTrashFiles(event.target.checked)}
        />
        Allow moving project files and folders to Trash
      </label>
      <p className="settings-help">
        Move approved project entries to the system Trash. Unsaved files require
        your choice to save, discard or cancel. Interrupted moves retain
        recovery data.
      </p>
      <label>
        <input
          type="checkbox"
          checked={readBuffers}
          disabled={busy || !readFiles}
          onChange={(event) => {
            setReadBuffers(event.target.checked);
            if (!event.target.checked) {
              setWriteBuffers(false);
              setSaveFiles(false);
            }
          }}
        />
        Allow reading unsaved editor buffers
      </label>
      <p className="settings-help">
        Share text currently loaded in this workspace's editors, including
        unsaved changes. Secret paths and links remain excluded. This does not
        allow editing or saving.
      </p>
      <label>
        <input
          type="checkbox"
          checked={writeBuffers}
          disabled={busy || !readFiles || !readBuffers}
          onChange={(event) => {
            setWriteBuffers(event.target.checked);
            if (!event.target.checked) setSaveFiles(false);
          }}
        />
        Allow editing loaded buffers
      </label>
      <p className="settings-help">
        Allow changes to the shared editor text, with revision checks and undo.
        Saving to disk requires a separate permission.
      </p>
      <label>
        <input
          type="checkbox"
          checked={saveFiles}
          disabled={busy || !readFiles || !readBuffers || !writeBuffers}
          onChange={(event) => setSaveFiles(event.target.checked)}
        />
        Allow saving editor files to disk
      </label>
      <p className="settings-help">
        Replace project files with the approved editor buffer after checking
        buffer and disk revisions.
      </p>
      <label>
        <input
          type="checkbox"
          checked={importApk}
          disabled={busy || !readFiles}
          onChange={(event) => {
            setImportApk(event.target.checked);
            if (!event.target.checked) setInstallApk(false);
          }}
        />
        Allow importing APK files from this project
      </label>
      <p className="settings-help">
        Make private copies of APK files up to 512 MiB, identified by their
        content hash. Links, paths outside the project and known secret paths
        are denied. Installing a copy requires separate permission and approval.
      </p>
      <label>
        <input
          type="checkbox"
          checked={installApk}
          disabled={busy || !importApk || !readAndroid || !controlAndroid}
          onChange={(event) => setInstallApk(event.target.checked)}
        />
        Allow requesting APK installation on this Android device
      </label>
      <p className="settings-help">
        Each installation requires a separate decision here for the exact file
        hash and running phone instance.
      </p>
      <div className="agent-control-actions">
        <button
          className="button"
          disabled={
            busy ||
            !valid ||
            (readChat && chatConversations.length === 0 && !createChat) ||
            (readAndroid &&
              !setupAndroid &&
              !manageAndroid &&
              !androidDevices.some((d) => d.id === androidDevice))
          }
          onClick={() =>
            void run(
              () =>
                api("agent_control_approve", {
                  requestId: request.id,
                  workspaceIds: [
                    workspaceId,
                    ...extraWorkspaceIds.filter((id) =>
                      additionalWorkspaces.some((w) => w.id === id),
                    ),
                  ],
                  scopes: [
                    ...new Set([
                      "workspace.read",
                      ...(readChat ? ["chat.read"] : []),
                      ...(readChat && draftChat ? ["chat.draft"] : []),
                      ...(readChat && sendChat ? ["chat.send"] : []),
                      ...(readChat && stopChat ? ["chat.stop"] : []),
                      ...(readChat && exportChat ? ["chat.export"] : []),
                      ...(readChat && openChat
                        ? ["chat.open", "panel.create", "panel.focus"]
                        : []),
                      ...(readChat && openChat && createChat
                        ? ["chat.create"]
                        : []),
                      ...(openSettings ? ["settings.open"] : []),
                      ...(readSettings ? ["settings.read"] : []),
                      ...(readSettings && writeSettings
                        ? ["settings.write"]
                        : []),
                      ...(readFiles ? ["files.read"] : []),
                      ...(readFiles && readGit ? ["git.read"] : []),
                      ...(readFiles && readGit && writeGit
                        ? ["git.write", "git.execute"]
                        : []),
                      ...(readFiles && readGit && writeGit && discardGit
                        ? ["git.discard"]
                        : []),
                      ...(readFiles &&
                      readGit &&
                      writeGit &&
                      networkGit &&
                      pullGit
                        ? ["git.pull"]
                        : []),
                      ...(readFiles && readGit && writeGit && networkGit
                        ? ["git.network"]
                        : []),
                      ...(readFiles &&
                      readGit &&
                      writeGit &&
                      networkGit &&
                      pushGit
                        ? ["git.push"]
                        : []),
                      ...(readFiles && readBuffers ? ["editor.read"] : []),
                      ...(readFiles && readBuffers && writeBuffers
                        ? ["editor.write"]
                        : []),
                      ...(readFiles && readBuffers && writeBuffers && saveFiles
                        ? ["files.mutate"]
                        : []),
                      ...(readFiles && createFiles
                        ? ["files.mutate", "files.create"]
                        : []),
                      ...(readFiles && renameFiles
                        ? ["files.mutate", "files.rename"]
                        : []),
                      ...(readFiles && trashFiles
                        ? ["files.mutate", "files.trash"]
                        : []),
                      ...(readFiles && importApk ? ["artifact.import"] : []),
                      ...(readFiles &&
                      importApk &&
                      readAndroid &&
                      controlAndroid &&
                      installApk
                        ? ["android.install"]
                        : []),
                      ...(readAndroid ? ["android.read"] : []),
                      ...(readAndroid && setupAndroid ? ["android.setup"] : []),
                      ...(readAndroid && manageAndroid
                        ? ["android.manage"]
                        : []),
                      ...(readAndroid && controlAndroid
                        ? ["android.control"]
                        : []),
                      ...(readAndroid && controlAndroid && interactAndroid
                        ? ["android.interact"]
                        : []),
                      ...(readAndroid && controlAndroid && launchAndroid
                        ? ["android.launch"]
                        : []),
                      ...(readAndroid && controlAndroid && logsAndroid
                        ? ["android.logs"]
                        : []),
                      ...(readAndroid && controlAndroid && observeAndroid
                        ? ["android.observe"]
                        : []),
                      ...(readAndroid && controlAndroid && captureAndroid
                        ? ["android.capture"]
                        : []),
                      ...(openProjects && writeWorkspace
                        ? ["project.open"]
                        : []),
                      ...(closeWorkspaces && writeWorkspace && managePanels
                        ? ["workspace.close"]
                        : []),
                      ...(closeProjects &&
                      closeWorkspaces &&
                      writeWorkspace &&
                      managePanels &&
                      allProjectWorkspaces
                        ? ["project.close"]
                        : []),
                      ...(movePanels && writeWorkspace ? ["panel.move"] : []),
                      ...(managePanels
                        ? ["panel.focus", "panel.close", "panel.create"]
                        : []),
                      ...(writeWorkspace
                        ? ["workspace.write", "panel.create"]
                        : []),
                      ...(navigateBrowser
                        ? [
                            "panel.create",
                            "browser.navigate",
                            ...(captureBrowser
                              ? ["browser.capture_composite"]
                              : []),
                            ...(readBrowser
                              ? [
                                  "browser.read",
                                  ...(interactBrowser
                                    ? ["browser.interact"]
                                    : []),
                                ]
                              : []),
                          ]
                        : []),
                      ...(executeTerminal
                        ? [
                            ...(writeWorkspace ? [] : ["panel.create"]),
                            "terminal.execute",
                            "terminal.read",
                          ]
                        : []),
                    ]),
                  ],
                  androidDevices:
                    readAndroid && androidDevice ? [androidDevice] : [],
                  chatConversations: readChat ? chatConversations : [],
                  androidPackages:
                    readAndroid &&
                    controlAndroid &&
                    (launchAndroid || logsAndroid)
                      ? androidPackages.split(/\s+/).filter(Boolean)
                      : [],
                  browserOrigins: navigateBrowser
                    ? browserOrigins.split(/\s+/).filter(Boolean)
                    : [],
                }),
              "Session approved.",
            )
          }
        >
          Approve session
        </button>
        <button
          className="button"
          disabled={busy}
          onClick={() =>
            void run(() =>
              api("agent_control_reject", { requestId: request.id }),
            )
          }
        >
          Reject
        </button>
      </div>
    </article>
  );
}
