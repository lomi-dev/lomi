import "./settings-sections.css";
import { useEffect, useState } from "react";
import { api, errorMessage } from "./api";
import { msg, useT } from "./i18n";
import type { ControlState, ControlWorkspace } from "./agent-control";
import { DisclosureSummary } from "./ui";

export default function AgentControlPairing({
  request,
  workspaces,
  terminalProfiles,
  busy,
  run,
}: {
  request: NonNullable<ControlState["broker"]>["pending"][number];
  workspaces: ControlWorkspace[];
  terminalProfiles: { id: string; revision: string }[];
  busy: boolean;
  run: (action: () => Promise<unknown>, message?: string) => Promise<void>;
}) {
  const { t } = useT();
  const [workspaceId, setWorkspaceId] = useState("");
  const [extraWorkspaceIds, setExtraWorkspaceIds] = useState<string[]>([]);
  const [readSettings, setReadSettings] = useState(false);
  const [writeSettings, setWriteSettings] = useState(false);
  const [openSettings, setOpenSettings] = useState(false);
  const [writeWorkspace, setWriteWorkspace] = useState(false);
  const [executeTerminal, setExecuteTerminal] = useState(false);
  const [terminalProfileId, setTerminalProfileId] = useState(
    terminalProfiles[0]?.id ?? "",
  );
  const [closeWorkspaces, setCloseWorkspaces] = useState(false);
  const [closeProjects, setCloseProjects] = useState(false);
  const [openProjects, setOpenProjects] = useState(false);
  const [movePanels, setMovePanels] = useState(false);
  const [managePanels, setManagePanels] = useState(false);
  const [navigateBrowser, setNavigateBrowser] = useState(false);
  const [captureBrowser, setCaptureBrowser] = useState(false);
  const [interactBrowser, setInteractBrowser] = useState(false);
  const [readBrowser, setReadBrowser] = useState(false);
  const [downloadBrowser, setDownloadBrowser] = useState(false);
  const [uploadBrowser, setUploadBrowser] = useState(false);
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
  const [importFiles, setImportFiles] = useState(false);
  const [exportArtifacts, setExportArtifacts] = useState(false);
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
  const terminalAccessAdded = executeTerminal;
  const filesAccessAdded = readFiles;
  const gitAccessAdded = readFiles && readGit;
  const browserAccessAdded = navigateBrowser;
  const androidAccessAdded = readAndroid || (readFiles && importApk);
  const workspacesAccessAdded = writeWorkspace || managePanels;
  const preferencesAccessAdded = readSettings || openSettings;
  const activePermissionGroups = [
    ...(terminalAccessAdded ? [msg("Terminal")] : []),
    ...(filesAccessAdded ? [msg("Files & editor")] : []),
    ...(gitAccessAdded ? [msg("Git")] : []),
    ...(browserAccessAdded ? [msg("Browser")] : []),
    ...(androidAccessAdded ? [msg("Android")] : []),
    ...(workspacesAccessAdded ? [msg("Workspaces & panels")] : []),
    ...(preferencesAccessAdded ? [msg("Application preferences")] : []),
  ];
  const grantedScopes = [
    ...new Set([
      "workspace.read",
      ...(openSettings ? ["settings.open"] : []),
      ...(readSettings ? ["settings.read"] : []),
      ...(readSettings && writeSettings ? ["settings.write"] : []),
      ...(readFiles ? ["files.read"] : []),
      ...(readFiles && readGit ? ["git.read"] : []),
      ...(readFiles && readGit && writeGit ? ["git.write", "git.execute"] : []),
      ...(readFiles && readGit && writeGit && discardGit
        ? ["git.discard"]
        : []),
      ...(readFiles && readGit && writeGit && networkGit && pullGit
        ? ["git.pull"]
        : []),
      ...(readFiles && readGit && writeGit && networkGit
        ? ["git.network"]
        : []),
      ...(readFiles && readGit && writeGit && networkGit && pushGit
        ? ["git.push"]
        : []),
      ...(readFiles && readBuffers ? ["editor.read"] : []),
      ...(readFiles && readBuffers && writeBuffers ? ["editor.write"] : []),
      ...(readFiles && readBuffers && writeBuffers && saveFiles
        ? ["files.mutate"]
        : []),
      ...(readFiles && createFiles ? ["files.mutate", "files.create"] : []),
      ...(readFiles && renameFiles ? ["files.mutate", "files.rename"] : []),
      ...(readFiles && trashFiles ? ["files.mutate", "files.trash"] : []),
      ...(readFiles && importApk ? ["artifact.import"] : []),
      ...(readFiles && importFiles ? ["artifact.import_file"] : []),
      ...(readFiles && createFiles && exportArtifacts
        ? ["artifact.export"]
        : []),
      ...(readFiles && importApk && readAndroid && controlAndroid && installApk
        ? ["android.install"]
        : []),
      ...(readAndroid ? ["android.read"] : []),
      ...(readAndroid && setupAndroid ? ["android.setup"] : []),
      ...(readAndroid && manageAndroid ? ["android.manage"] : []),
      ...(readAndroid && controlAndroid ? ["android.control"] : []),
      ...(readAndroid && controlAndroid && interactAndroid
        ? ["android.interact"]
        : []),
      ...(readAndroid && controlAndroid && launchAndroid
        ? ["android.launch"]
        : []),
      ...(readAndroid && controlAndroid && logsAndroid ? ["android.logs"] : []),
      ...(readAndroid && controlAndroid && observeAndroid
        ? ["android.observe"]
        : []),
      ...(readAndroid && controlAndroid && captureAndroid
        ? ["android.capture"]
        : []),
      ...(openProjects && writeWorkspace ? ["project.open"] : []),
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
      ...(managePanels ? ["panel.focus", "panel.close", "panel.create"] : []),
      ...(writeWorkspace ? ["workspace.write", "panel.create"] : []),
      ...(navigateBrowser
        ? [
            "panel.create",
            "browser.navigate",
            ...(captureBrowser ? ["browser.capture_composite"] : []),
            ...(readBrowser
              ? [
                  "browser.read",
                  ...(downloadBrowser ? ["browser.download"] : []),
                  ...(interactBrowser
                    ? [
                        "browser.interact",
                        ...(uploadBrowser ? ["browser.upload"] : []),
                      ]
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
  ];
  return (
    <article className="agent-control-request agent-pairing">
      <header className="agent-pairing-header">
        <div>
          <span className="agent-pairing-kicker">{t("Wants to connect")}</span>
          <h3>{request.clientLabel}</h3>
        </div>
        <span
          className="agent-control-badge"
          data-state={request.secondsRemaining <= 30 ? "waiting" : undefined}
        >
          {t("Expires in {seconds} seconds", {
            seconds: request.secondsRemaining,
          })}
        </span>
      </header>
      <div className="agent-pairing-identity">
        <span className="agent-pairing-step">1</span>
        <div>
          <h4>{t("Verify the request")}</h4>
          <p className="settings-help">
            {t(
              "Match this request ID with the pairingRequestId returned by lomi_status in your client before approving.",
            )}
          </p>
          <code className="agent-pairing-request-id">{request.id}</code>
          <details className="agent-pairing-details">
            <DisclosureSummary>{t("Client certificate:")}</DisclosureSummary>
            <code>{request.certificateSha256}</code>
          </details>
        </div>
      </div>
      <div className="agent-pairing-workspace">
        <span className="agent-pairing-step">2</span>
        <div>
          <label htmlFor={`control-workspace-${request.id}`}>
            <h4>{t("Choose a workspace")}</h4>
          </label>
          <select
            id={`control-workspace-${request.id}`}
            value={workspaceId}
            onChange={(event) => {
              setWorkspaceId(event.target.value);
              setExtraWorkspaceIds([]);
              setCloseProjects(false);
            }}
            disabled={busy}
          >
            <option value="">{t("Choose a workspace")}</option>
            {workspaces.map((w) => (
              <option key={w.id} value={w.id}>
                {w.projectName} / {w.name}
              </option>
            ))}
          </select>
          {valid && (
            <p className="settings-help">
              {t("Project folder:")}{" "}
              <code>
                {workspaces.find((w) => w.id === workspaceId)?.projectPath}
              </code>
            </p>
          )}
          {additionalWorkspaces.length > 0 && (
            <fieldset>
              <legend>{t("Other workspaces in this project")}</legend>
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
                {t(
                  "Only checked workspaces share this connection’s permissions.",
                )}
              </p>
            </fieldset>
          )}
        </div>
      </div>
      <div className="agent-permissions-heading">
        <span className="agent-pairing-step">3</span>
        <div>
          <h4>{t("Choose access")}</h4>
          <p className="settings-help">
            {t(
              "Workspace names and layout are shared by default. Add only the access this agent needs.",
            )}
          </p>
        </div>
      </div>
      <div className="agent-permission-list">
        <details className="agent-permission-group">
          <DisclosureSummary>
            <span className="agent-permission-title">{t("Terminal")}</span>
            <span className="agent-permission-description">
              {t("Run commands in one approved shell.")}
            </span>
            {terminalAccessAdded && (
              <span className="agent-permission-selected">
                {t("Access added")}
              </span>
            )}
          </DisclosureSummary>
          <div className="agent-permission-content">
            <label>
              <input
                type="checkbox"
                checked={executeTerminal}
                disabled={busy || !terminalProfiles.length}
                onChange={(event) => setExecuteTerminal(event.target.checked)}
              />{" "}
              {t("Allow terminal creation, command execution and output reads")}
            </label>
            {executeTerminal && (
              <label>
                {t("Approved terminal shell")}
                <select
                  value={terminalProfileId}
                  disabled={busy}
                  onChange={(event) => setTerminalProfileId(event.target.value)}
                >
                  {terminalProfiles.map((profile) => (
                    <option key={profile.id} value={profile.id}>
                      {profile.id === "local:zsh"
                        ? "Zsh (/bin/zsh)"
                        : profile.id === "local:bash"
                          ? "Bash (/bin/bash)"
                          : profile.id}
                    </option>
                  ))}
                </select>
              </label>
            )}
            <p className="settings-help">
              {t(
                "Shell commands run with your user account permissions. The project folder is a starting directory, not a sandbox. Only the selected shell profile is approved. Its normal user configuration still applies.",
              )}
            </p>
          </div>
        </details>
        <details className="agent-permission-group">
          <DisclosureSummary>
            <span className="agent-permission-title">
              {t("Files & editor")}
            </span>
            <span className="agent-permission-description">
              {t("Access project files and editor buffers.")}
            </span>
            {filesAccessAdded && (
              <span className="agent-permission-selected">
                {t("Access added")}
              </span>
            )}
          </DisclosureSummary>
          <div className="agent-permission-content">
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
                    setImportFiles(false);
                    setExportArtifacts(false);
                    setInstallApk(false);
                  }
                }}
              />
              {t("Allow reading project files")}
            </label>
            <p className="settings-help">
              {t(
                "Share text files on disk in this project. Known secret paths and links are excluded. This permission does not read unsaved editor buffers or allow changes.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={createFiles}
                disabled={busy || !readFiles}
                onChange={(event) => {
                  setCreateFiles(event.target.checked);
                  if (!event.target.checked) setExportArtifacts(false);
                }}
              />
              {t("Allow creating project files and folders")}
            </label>
            <p className="settings-help">
              {t(
                "Create empty files and folders inside this project. Existing names are preserved. Editing and saving file contents require separate permissions.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={renameFiles}
                disabled={busy || !readFiles}
                onChange={(event) => setRenameFiles(event.target.checked)}
              />
              {t("Allow renaming and moving project files and folders")}
            </label>
            <p className="settings-help">
              {t(
                "Move files and folders within this project after checking their disk versions. Existing destinations are preserved. Open editors keep unsaved text and undo history.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={trashFiles}
                disabled={busy || !readFiles}
                onChange={(event) => setTrashFiles(event.target.checked)}
              />
              {t("Allow moving project files and folders to Trash")}
            </label>
            <p className="settings-help">
              {t(
                "Move approved project entries to the system Trash. Unsaved files require your choice to save, discard or cancel. Interrupted moves retain recovery data.",
              )}
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
              {t("Allow reading unsaved editor buffers")}
            </label>
            <p className="settings-help">
              {t(
                "Share text currently loaded in this workspace's editors, including unsaved changes. Secret paths and links remain excluded. This does not allow editing or saving.",
              )}
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
              {t("Allow editing loaded buffers")}
            </label>
            <p className="settings-help">
              {t(
                "Allow changes to the shared editor text, with revision checks and undo. Saving to disk requires a separate permission.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={saveFiles}
                disabled={busy || !readFiles || !readBuffers || !writeBuffers}
                onChange={(event) => setSaveFiles(event.target.checked)}
              />
              {t("Allow saving editor files to disk")}
            </label>
            <p className="settings-help">
              {t(
                "Replace project files with the approved editor buffer after checking buffer and disk revisions.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={importFiles}
                disabled={busy || !readFiles}
                onChange={(event) => setImportFiles(event.target.checked)}
              />
              {t("Allow importing project files as artifacts")}
            </label>
            <p className="settings-help">
              {t(
                "Make private copies of files up to 4 MiB, identified by their content hash. Links and known secret paths are excluded. Upload and export require separate permissions.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={exportArtifacts}
                disabled={busy || !readFiles || !createFiles}
                onChange={(event) => setExportArtifacts(event.target.checked)}
              />
              {t("Allow exporting artifacts to new project files")}
            </label>
            <p className="settings-help">
              {t(
                "Save an accessible artifact of up to 4 MiB at an explicit project path. Existing files are preserved. The original artifact permissions still apply.",
              )}
            </p>
          </div>
        </details>
        <details className="agent-permission-group">
          <DisclosureSummary>
            <span className="agent-permission-title">Git</span>
            <span className="agent-permission-description">
              {t(
                "Read repository data or request approved changes. Requires project file access in Files & editor.",
              )}
            </span>
            {gitAccessAdded && (
              <span className="agent-permission-selected">
                {t("Access added")}
              </span>
            )}
          </DisclosureSummary>
          <div className="agent-permission-content">
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
              {t("Allow reading Git information in this project")}
            </label>
            <p className="settings-help">
              {t(
                "Share supported repository observations. This permission does not allow Git changes, network access, hooks or external helpers.",
              )}
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
              {t(
                "Allow requesting Git changes and executing configured Git code",
              )}
            </label>
            <p className="settings-help">
              {t(
                "Each Git change needs your approval in the main window for the exact operation and repository state. Git hooks, helpers and filters can run with your account’s access. Only grant this for repositories you trust.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={discardGit}
                disabled={busy || !readFiles || !readGit || !writeGit}
                onChange={(event) => setDiscardGit(event.target.checked)}
              />
              {t("Allow requesting discard of working Git changes")}
            </label>
            <p className="settings-help">
              {t(
                "Each discard shows the exact working changes before approval. It restores staged versions without saving the discarded bytes in Git or Trash.",
              )}
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
              {t("Allow requesting contact with configured Git remotes")}
            </label>
            <p className="settings-help">
              {t(
                "Fetch and pull requests show the remote and branch in the main window before connecting. Configured transports and credential helpers may run after approval. This permission does not allow pushing commits.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={pullGit}
                disabled={
                  busy || !readFiles || !readGit || !writeGit || !networkGit
                }
                onChange={(event) => setPullGit(event.target.checked)}
              />
              {t("Allow requesting Git pulls")}
            </label>
            <p className="settings-help">
              {t(
                "Each pull needs your approval for the exact incoming commit and mode. Pull changes working files; rebase can rewrite local commit history. Conflicts stay in the repository for you to resolve.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={pushGit}
                disabled={
                  busy || !readFiles || !readGit || !writeGit || !networkGit
                }
                onChange={(event) => setPushGit(event.target.checked)}
              />
              {t("Allow requesting Git pushes")}
            </label>
            <p className="settings-help">
              {t(
                "Each push needs your approval for the exact remote branch and commits. Push can publish repository content. Only creating a branch or moving it forward is allowed; rewriting remote history is excluded.",
              )}
            </p>
          </div>
        </details>
        <details className="agent-permission-group">
          <DisclosureSummary>
            <span className="agent-permission-title">{t("Browser")}</span>
            <span className="agent-permission-description">
              {t("Navigate isolated browser panels and choose page access.")}
            </span>
            {browserAccessAdded && (
              <span className="agent-permission-selected">
                {t("Access added")}
              </span>
            )}
          </DisclosureSummary>
          <div className="agent-permission-content">
            <label>
              <input
                type="checkbox"
                checked={navigateBrowser}
                disabled={busy}
                onChange={(event) => {
                  setNavigateBrowser(event.target.checked);
                  if (!event.target.checked) {
                    setDownloadBrowser(false);
                    setUploadBrowser(false);
                  }
                }}
              />{" "}
              {t("Allow opening and navigating isolated browser panels")}
            </label>
            <label>
              <input
                type="checkbox"
                checked={readBrowser}
                disabled={busy || !navigateBrowser}
                onChange={(event) => {
                  setReadBrowser(event.target.checked);
                  if (!event.target.checked) {
                    setDownloadBrowser(false);
                    setUploadBrowser(false);
                  }
                }}
              />{" "}
              {t("Allow reading page text, form structure and browser logs")}
            </label>
            <label>
              <input
                type="checkbox"
                checked={interactBrowser}
                disabled={busy || !navigateBrowser || !readBrowser}
                onChange={(event) => {
                  setInteractBrowser(event.target.checked);
                  if (!event.target.checked) setUploadBrowser(false);
                }}
              />{" "}
              {t("Allow clicking, typing and scrolling in pages")}
            </label>
            <label>
              <input
                type="checkbox"
                checked={captureBrowser}
                disabled={busy || !navigateBrowser}
                onChange={(event) => setCaptureBrowser(event.target.checked)}
              />{" "}
              {t(
                "Allow screenshots of pages, including embedded third-party frames",
              )}
            </label>
            <label>
              <input
                type="checkbox"
                checked={downloadBrowser}
                disabled={busy || !navigateBrowser || !readBrowser}
                onChange={(event) => setDownloadBrowser(event.target.checked)}
              />{" "}
              {t("Allow downloading page files as private artifacts")}
            </label>
            <p className="settings-help">
              {t(
                "Fetch an explicit URL from the current page's origin, using its browser session. Transfers are limited to 4 MiB and block redirects. Export to a project file requires separate permission.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={uploadBrowser}
                disabled={
                  busy || !navigateBrowser || !readBrowser || !interactBrowser
                }
                onChange={(event) => setUploadBrowser(event.target.checked)}
              />{" "}
              {t("Allow requesting file uploads to pages")}
            </label>
            <p className="settings-help">
              {t(
                "Each upload needs your approval of the destination, filename, size and SHA-256 in Settings. Attaching the private copy lets the page read and send its bytes immediately. Files are limited to 4 MiB.",
              )}
            </p>
            {navigateBrowser && (
              <>
                <label htmlFor={`control-origins-${request.id}`}>
                  {t("Allowed browser origins")}
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
                  {t(
                    "One exact origin per line, including its port. These panels use separate browser data. Origin restrictions apply to page navigation; pages can still request other network resources.",
                  )}
                </p>
              </>
            )}
          </div>
        </details>
        <details className="agent-permission-group">
          <DisclosureSummary>
            <span className="agent-permission-title">Android</span>
            <span className="agent-permission-description">
              {t(
                "Choose device access. APK imports require project file access in Files & editor.",
              )}
            </span>
            {androidAccessAdded && (
              <span className="agent-permission-selected">
                {t("Access added")}
              </span>
            )}
          </DisclosureSummary>
          <div className="agent-permission-content">
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
              {t("Allow importing APK files from this project")}
            </label>
            <p className="settings-help">
              {t(
                "Make private copies of APK files up to 512 MiB, identified by their content hash. Links, paths outside the project and known secret paths are denied. Installing a copy requires separate permission and approval.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={installApk}
                disabled={busy || !importApk || !readAndroid || !controlAndroid}
                onChange={(event) => setInstallApk(event.target.checked)}
              />
              {t("Allow requesting APK installation on this Android device")}
            </label>
            <p className="settings-help">
              {t(
                "Each installation requires a separate decision here for the exact file hash and running phone instance.",
              )}
            </p>
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
              {t("Allow reading selected Android device status")}
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
                  {t(
                    "Allow Android SDK setup, recovery and cache cleanup requests",
                  )}
                </label>
                <label>
                  <input
                    type="checkbox"
                    checked={manageAndroid}
                    disabled={busy}
                    onChange={(event) => setManageAndroid(event.target.checked)}
                  />{" "}
                  {t("Allow creating devices and changing the selected device")}
                </label>
                <p className="settings-help">
                  {t(
                    "Setup and device changes each require a separate approval here. Provider licenses and erasing data always require your decision. New devices become visible only to the client that created them.",
                  )}
                </p>
                <label htmlFor={`control-android-${request.id}`}>
                  {t("Managed Android device")}
                </label>
                <select
                  id={`control-android-${request.id}`}
                  value={androidDevice}
                  disabled={busy}
                  onChange={(event) => setAndroidDevice(event.target.value)}
                >
                  <option value="">{t("Choose a managed device")}</option>
                  {androidDevices.map((device) => (
                    <option key={device.id} value={device.id}>
                      {device.name}
                    </option>
                  ))}
                </select>
                <p className="settings-help">
                  {t(
                    "Only this device's name and runtime status will be shared. This permission does not start it or allow input.",
                  )}
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
                  {t("Allow starting and stopping this Android device")}
                </label>
                <p className="settings-help">
                  {t(
                    "Start can boot a stopped phone. An already running phone remains under its current control. Human actions take control back.",
                  )}
                </p>
                <label>
                  <input
                    type="checkbox"
                    checked={interactAndroid}
                    disabled={busy || !controlAndroid}
                    onChange={(event) =>
                      setInteractAndroid(event.target.checked)
                    }
                  />
                  {t("Allow touch, keys and text in this Android device")}
                </label>
                <p className="settings-help">
                  {t(
                    "Input requires the selected phone panel in a focused Lomi window. Take control returns input to you.",
                  )}
                </p>
                <label>
                  <input
                    type="checkbox"
                    checked={observeAndroid}
                    disabled={busy || !controlAndroid}
                    onChange={(event) =>
                      setObserveAndroid(event.target.checked)
                    }
                  />
                  {t("Allow reading screen content in this Android device")}
                </label>
                <p className="settings-help">
                  {t(
                    "Share a limited UI hierarchy from the phone started by this connection. Password and editable field values are omitted.",
                  )}
                </p>
                <label>
                  <input
                    type="checkbox"
                    checked={launchAndroid}
                    disabled={busy || !controlAndroid}
                    onChange={(event) => setLaunchAndroid(event.target.checked)}
                  />
                  {t("Allow launching approved Android apps")}
                </label>
                <label>
                  <input
                    type="checkbox"
                    checked={logsAndroid}
                    disabled={busy || !controlAndroid}
                    onChange={(event) => setLogsAndroid(event.target.checked)}
                  />
                  {t("Allow reading logs from approved Android apps")}
                </label>
                {(launchAndroid || logsAndroid) && (
                  <>
                    <label htmlFor={`control-packages-${request.id}`}>
                      {t("Allowed Android packages")}
                    </label>
                    <textarea
                      id={`control-packages-${request.id}`}
                      value={androidPackages}
                      disabled={busy}
                      placeholder={t("org.example.app")}
                      onChange={(event) =>
                        setAndroidPackages(event.target.value)
                      }
                    />
                    <p className="settings-help">
                      {t(
                        "Enter up to 16 exact package names, separated by spaces or newlines. Launch may open any exported activity in these apps. Logs can contain app data and personal information.",
                      )}
                    </p>
                  </>
                )}
                {androidError && <p role="alert">{androidError}</p>}
                <label>
                  <input
                    type="checkbox"
                    checked={captureAndroid}
                    disabled={busy || !controlAndroid}
                    onChange={(event) =>
                      setCaptureAndroid(event.target.checked)
                    }
                  />
                  {t("Allow screenshots of this Android device")}
                </label>
                <p className="settings-help">
                  {t(
                    "Share the full phone screen, including visible text and fields, as individually requested images. This permission does not grant input.",
                  )}
                </p>
              </>
            )}
          </div>
        </details>
        <details className="agent-permission-group">
          <DisclosureSummary>
            <span className="agent-permission-title">
              {t("Workspaces & panels")}
            </span>
            <span className="agent-permission-description">
              {t("Manage workspaces, projects and their panels.")}
            </span>
            {workspacesAccessAdded && (
              <span className="agent-permission-selected">
                {t("Access added")}
              </span>
            )}
          </DisclosureSummary>
          <div className="agent-permission-content">
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
              {t("Allow workspace creation, renaming and selection")}
            </label>
            <label>
              <input
                type="checkbox"
                checked={openProjects && writeWorkspace}
                disabled={busy || !writeWorkspace}
                onChange={(event) => setOpenProjects(event.target.checked)}
              />{" "}
              {t("Allow requesting access to new project folders")}
            </label>
            <p className="settings-help">
              {t(
                "Each folder requires a separate approval here for its initial workspace and this connection’s permissions. Opening starts with a blank editor.",
              )}
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
              {t("Allow selecting and closing panels")}
            </label>
            <label>
              <input
                type="checkbox"
                checked={movePanels}
                disabled={busy || !writeWorkspace}
                onChange={(event) => setMovePanels(event.target.checked)}
              />{" "}
              {t("Allow rearranging existing panels")}
            </label>
            <p className="settings-help">
              {t(
                "Requires workspace changes. Docking also requires selecting panels. Existing terminal processes and editor buffers are preserved.",
              )}
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
              {t("Allow requesting workspace closure")}
            </label>
            <p className="settings-help">
              {t(
                "Closes approved views without deleting the project folder. Unsaved files keep their save/discard/cancel dialog; protected, busy or human-controlled terminal sessions are preserved.",
              )}
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
              {t("Allow requesting project closure")}
            </label>
            <p className="settings-help">
              {t(
                "Requires access to every workspace in this project. A new unapproved workspace blocks closure. Closing preserves the project folder and uses the same unsaved-file and protected-process checks.",
              )}
            </p>
          </div>
        </details>
        <details className="agent-permission-group">
          <DisclosureSummary>
            <span className="agent-permission-title">
              {t("Application preferences")}
            </span>
            <span className="agent-permission-description">
              {t("Read or request changes to nonsecret preferences.")}
            </span>
            {preferencesAccessAdded && (
              <span className="agent-permission-selected">
                {t("Access added")}
              </span>
            )}
          </DisclosureSummary>
          <div className="agent-permission-content">
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
              {t("Allow reading nonsecret application preferences")}
            </label>
            <p className="settings-help">
              {t(
                "Shares global editor defaults, terminal preferences, shortcut bindings and theme selections. Conversation history, credentials and plugin source are excluded.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={writeSettings}
                disabled={busy || !readSettings}
                onChange={(e) => setWriteSettings(e.target.checked)}
              />{" "}
              {t("Allow requesting application preference changes")}
            </label>
            <p className="settings-help">
              {t(
                "Requires preference read access. Each change shows its exact before and after values in Settings for your approval. Supports editor defaults, terminal preferences, keyboard shortcuts and built-in color themes.",
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={openSettings}
                disabled={busy}
                onChange={(event) => setOpenSettings(event.target.checked)}
              />{" "}
              {t("Allow opening Settings sections")}
            </label>
            <p className="settings-help">
              {t(
                "This may focus the Settings tab. Reading and changing preferences require separate permissions.",
              )}
            </p>
          </div>
        </details>
      </div>
      <footer className="agent-pairing-footer">
        <p className="settings-help" aria-live="polite">
          {!valid
            ? t("Choose a workspace to approve this session.")
            : activePermissionGroups.length > 0
              ? t("Additional access: {groups}.", {
                  groups: activePermissionGroups.map((g) => t(g)).join(", "),
                })
              : `${t("Included by default: workspace metadata.")} ${t("No additional access selected.")}`}
        </p>
        <div className="agent-control-actions">
          <button
            className="button"
            disabled={busy}
            onClick={() =>
              void run(() =>
                api("agent_control_reject", { requestId: request.id }),
              )
            }
          >
            {t("Reject")}
          </button>
          <button
            className="button button-primary"
            disabled={
              busy ||
              !valid ||
              (executeTerminal &&
                !terminalProfiles.some((p) => p.id === terminalProfileId)) ||
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
                    scopes: grantedScopes,
                    ...(executeTerminal ? { terminalProfileId } : {}),
                    androidDevices:
                      readAndroid && androidDevice ? [androidDevice] : [],
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
                t("Session approved."),
              )
            }
          >
            {t("Approve session")}
          </button>
        </div>
      </footer>
    </article>
  );
}
