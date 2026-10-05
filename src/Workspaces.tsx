import type { RemoteState } from "./remote-workspaces";
import ResourceIcon from "./ResourceIcon";
import {
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  FileDiff,
  ChevronRight,
  Ellipsis,
  Folder,
  GitCommitHorizontal,
  Globe,
  MessageSquare,
  Monitor,
  Plus,
  Puzzle,
  Terminal,
} from "./icons";
import type { Project, Workspace } from "./model";
import { tabTitle } from "./model";
import type { GitRepositoryScan } from "./api";
import type { WorkspaceAppearance } from "./workspace-appearance";
import ContextMenu from "./ContextMenu";
import { IconButton, Modal } from "./ui";
import WorkspaceAvatar from "./WorkspaceAvatar";
import WorkspaceAppearanceDialog from "./WorkspaceAppearanceDialog";
import { getTerminalAgents, subscribeTerminalAgents } from "./terminal-runtime";
import { summarizeWorkspaceAgents } from "./workspace-summary";
import { CliAgentIcon } from "./CliAgentIcon";
import { cliNames } from "./cli-agents";
import useWorkspaceRepositories from "./useWorkspaceRepositories";

export default function Workspaces({
  projects,
  activeWorkspaceId,
  activeRoot,
  git,
  onSelect,
  onNew,
  onRename,
  onAppearanceChange,
  onDelete,
  remote,
}: {
  projects: Project[];
  remote?: {
    available: boolean;
    state?: RemoteState;
    error: string;
    busy?: string;
    share: (id: string, shared: boolean) => Promise<void>;
  };
  activeWorkspaceId?: string;
  activeRoot: string;
  git: GitRepositoryScan & { loading: boolean };
  onSelect: (path: string, workspaceId: string, tabId?: string) => void;
  onNew: () => void;
  onRename: (workspace: Workspace) => void;
  onAppearanceChange: (
    workspace: Workspace,
    appearance?: WorkspaceAppearance,
  ) => void;
  onDelete: (workspace: Workspace) => void;
}) {
  const [menu, setMenu] = useState<{ id: string; x: number; y: number }>();
  const [customizing, setCustomizing] = useState<Workspace>();
  const [sharingId, setSharingId] = useState<string>();
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const trigger = useRef<HTMLButtonElement>(null);
  const agents = useSyncExternalStore(
    subscribeTerminalAgents,
    getTerminalAgents,
  );
  const repositories = useWorkspaceRepositories(projects, activeRoot, git);
  const count = projects.reduce(
    (total, project) => total + project.workspaces.length,
    0,
  );
  const workspace =
    menu &&
    projects
      .flatMap((project) => project.workspaces)
      .find((workspace) => workspace.id === menu.id);
  const sharingWorkspace = projects
    .flatMap((project) => project.workspaces)
    .find((workspace) => workspace.id === sharingId);
  const menuShared = remote?.state?.workspaces.find(
    (candidate) => candidate.id === workspace?.id,
  )?.shared;
  const sharingAvailable = !!remote?.available && !!remote.state?.qualified;
  const sharingActive = remote?.state?.workspaces.find(
    (workspace) => workspace.id === sharingId,
  )?.shared;
  useEffect(() => {
    if (sharingId && (!sharingWorkspace || sharingActive))
      setSharingId(undefined);
  }, [sharingId, sharingWorkspace, sharingActive]);
  const showMenu = (
    id: string,
    element: HTMLButtonElement,
    x?: number,
    y?: number,
  ) => {
    trigger.current = element;
    const bounds = element.getBoundingClientRect();
    setMenu({ id, x: x ?? bounds.left, y: y ?? bounds.bottom });
  };
  return (
    <div className="sidebar-panel workspace-panel">
      <div className="sidebar-heading">
        <span>WORKSPACES</span>
        <IconButton title="New workspace" onClick={onNew}>
          <Plus size={15} />
        </IconButton>
      </div>
      {remote?.error && (
        <p className="sidebar-empty" role="alert">
          {remote.error}
        </p>
      )}
      {remote?.state?.qualified === false && (
        <p className="sidebar-empty">
          Remote hosting currently requires Lomi on a Mac with Apple silicon.
        </p>
      )}
      <nav className="workspace-list" aria-label="Workspace list">
        {projects.flatMap((project) =>
          project.workspaces.map((workspace) => {
            const summary = summarizeWorkspaceAgents(workspace, agents);
            const repository = repositories[project.path];
            const shared = remote?.state?.workspaces.find(
              (candidate) => candidate.id === workspace.id,
            )?.shared;
            const extraAgents = summary.groups.slice(2);
            const extraAgentsLabel = extraAgents
              .map(
                ({ cli, count }) =>
                  `${cliNames[cli]} · ${count} ${count === 1 ? "agent" : "agents"}`,
              )
              .join(", ");
            const metadata =
              repository?.branch ||
              (repository?.count
                ? `${repository.count} ${repository.count === 1 ? "repository" : "repositories"}`
                : !repository || repository.loading
                  ? "Loading repositories…"
                  : repository.error
                    ? "Repositories unavailable"
                    : repository.limited
                      ? "Repository scan incomplete"
                      : `${workspace.tabs.length} ${workspace.tabs.length === 1 ? "tab" : "tabs"}`);
            const metadataTitle = [
              repository?.loading ? "Refreshing repositories…" : undefined,
              repository?.error,
              repository?.limited
                ? "Repository discovery reached its limit. Showing discovered repositories."
                : undefined,
              metadata,
            ]
              .filter(Boolean)
              .join("\n");
            return (
              <div
                className={`workspace-list-entry${workspace.id === activeWorkspaceId ? " is-active" : ""}`}
                key={workspace.id}
                data-workspace-id={workspace.id}
              >
                <div
                  className={`workspace-list-heading${expanded.has(workspace.id) ? " is-expanded" : ""}${menu?.id === workspace.id ? " is-menu-open" : ""}`}
                >
                  <button
                    type="button"
                    className="workspace-list-item"
                    aria-current={
                      workspace.id === activeWorkspaceId ? "true" : undefined
                    }
                    title={`${workspace.name}\n${project.path}`}
                    aria-label={`${workspace.name} ${project.path} ${metadata}${shared ? " Shared remotely" : ""}`}
                    aria-description={metadataTitle}
                    onClick={() => onSelect(project.path, workspace.id)}
                    aria-haspopup="menu"
                    aria-expanded={menu?.id === workspace.id}
                    onContextMenu={(event) => {
                      event.preventDefault();
                      showMenu(
                        workspace.id,
                        event.currentTarget,
                        event.clientX,
                        event.clientY,
                      );
                    }}
                    onKeyDown={(event) => {
                      if (
                        event.key === "ContextMenu" ||
                        (event.shiftKey && event.key === "F10")
                      ) {
                        event.preventDefault();
                        showMenu(workspace.id, event.currentTarget);
                      }
                    }}
                  >
                    <span className="workspace-folder-row">
                      {workspace.appearance ? (
                        <WorkspaceAvatar appearance={workspace.appearance} />
                      ) : (
                        <Folder size={14} aria-hidden="true" />
                      )}
                      <span className="workspace-folder" title={project.path}>
                        <bdi dir="ltr">{project.path}</bdi>
                      </span>
                      {shared && (
                        <span
                          className="workspace-remote-status"
                          title="Shared remotely"
                        >
                          <Globe size={12} aria-hidden="true" />
                          <span className="workspace-visually-hidden">
                            Shared remotely
                          </span>
                        </span>
                      )}
                    </span>
                    <span className="workspace-name">{workspace.name}</span>
                    <span
                      className={`workspace-metadata${summary.total ? " has-agents" : ""}${extraAgents.length ? " has-agent-overflow" : ""}`}
                      title={metadataTitle}
                    >
                      {metadata}
                    </span>
                  </button>
                  <IconButton
                    className="icon-button workspace-menu-trigger"
                    title={`Actions for ${workspace.name}`}
                    aria-haspopup="menu"
                    aria-expanded={menu?.id === workspace.id}
                    onClick={(event) =>
                      showMenu(workspace.id, event.currentTarget)
                    }
                  >
                    <Ellipsis size={16} />
                  </IconButton>
                  <div className="workspace-footer-controls">
                    {summary.total > 0 && (
                      <div
                        className="workspace-agent-list"
                        aria-label={`Agents in ${workspace.name}`}
                      >
                        {summary.groups.slice(0, 2).map(({ cli, count }) => (
                          <span
                            className="workspace-agent-chip"
                            key={cli}
                            role="img"
                            aria-label={`${cliNames[cli]} · ${count} ${count === 1 ? "agent" : "agents"}`}
                            title={`${cliNames[cli]} · ${count} ${count === 1 ? "agent" : "agents"}`}
                          >
                            <CliAgentIcon cli={cli} />
                          </span>
                        ))}
                        {extraAgents.length > 0 && (
                          <span
                            className="workspace-agent-chip workspace-agent-overflow"
                            role="img"
                            aria-label={extraAgentsLabel}
                            title={extraAgentsLabel}
                          >
                            +{extraAgents.length}
                          </span>
                        )}
                      </div>
                    )}
                    <IconButton
                      className="icon-button workspace-tabs-toggle"
                      title={`${expanded.has(workspace.id) ? "Collapse" : "Expand"} tabs in ${workspace.name}`}
                      aria-expanded={expanded.has(workspace.id)}
                      aria-controls={`workspace-tabs-${workspace.id}`}
                      onClick={() =>
                        setExpanded((current) => {
                          const next = new Set(current);
                          if (next.has(workspace.id)) next.delete(workspace.id);
                          else next.add(workspace.id);
                          return next;
                        })
                      }
                    >
                      <ChevronRight size={12} aria-hidden="true" />
                    </IconButton>
                  </div>
                </div>
                <ul
                  id={`workspace-tabs-${workspace.id}`}
                  className="workspace-tab-list"
                  aria-label={`Tabs in ${workspace.name}`}
                  hidden={!expanded.has(workspace.id)}
                >
                  {workspace.tabs.map((tab) => {
                    const tabAgents = summary.byTab[tab.id] ?? [];
                    return (
                      <li key={tab.id}>
                        <button
                          type="button"
                          className="workspace-tab-item"
                          aria-current={
                            workspace.id === activeWorkspaceId &&
                            tab.id === workspace.activeTabId
                              ? "true"
                              : undefined
                          }
                          title={
                            tab.type === "file" ? tab.relative : tabTitle(tab)
                          }
                          onClick={() =>
                            onSelect(project.path, workspace.id, tab.id)
                          }
                        >
                          {tab.type === "commit" ? (
                            <GitCommitHorizontal size={14} aria-hidden="true" />
                          ) : tab.type === "diff" ? (
                            <ResourceIcon
                              path={`${tab.root}/${tab.relative}`}
                              size={14}
                              fallback={FileDiff}
                            />
                          ) : tab.type === "file" ? (
                            <ResourceIcon
                              path={`${tab.root}/${tab.relative}`}
                              size={14}
                            />
                          ) : tab.type === "browser" ? (
                            <Globe size={14} aria-hidden="true" />
                          ) : tab.type === "android" ? (
                            <Monitor size={14} aria-hidden="true" />
                          ) : tab.type === "chat" ? (
                            <MessageSquare size={14} aria-hidden="true" />
                          ) : tab.type === "plugin" ? (
                            <Puzzle size={14} aria-hidden="true" />
                          ) : tabAgents.length === 1 ? (
                            <CliAgentIcon cli={tabAgents[0].cli} />
                          ) : (
                            <Terminal size={14} aria-hidden="true" />
                          )}
                          <span>{tabTitle(tab)}</span>
                          {tabAgents.length > 0 && (
                            <span
                              className="workspace-tab-agents"
                              title={tabAgents
                                .map(
                                  ({ cli, count }) =>
                                    `${cliNames[cli]} ×${count}`,
                                )
                                .join(", ")}
                              aria-label={tabAgents
                                .map(
                                  ({ cli, count }) =>
                                    `${count} ${cliNames[cli]}`,
                                )
                                .join(", ")}
                            >
                              {tabAgents
                                .map(({ count }) => count)
                                .reduce((sum, count) => sum + count, 0)}
                            </span>
                          )}
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </div>
            );
          }),
        )}
        {count === 0 && (
          <div className="workspace-empty">
            <WorkspaceAvatar />
            <p>Your workspaces live here.</p>
            <small>Choose a folder to get started.</small>
            <button type="button" className="button" onClick={onNew}>
              <Plus size={14} />
              Add workspace
            </button>
          </div>
        )}
      </nav>
      {menu && workspace && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          label="Workspace actions"
          actions={[
            { label: "Rename workspace", run: () => onRename(workspace) },
            remote
              ? {
                  label: menuShared
                    ? "Stop sharing remotely"
                    : "Share remotely",
                  disabled:
                    !(menuShared ? remote.available : sharingAvailable) ||
                    !!remote.busy,
                  run: () => {
                    if (!(menuShared ? remote.available : sharingAvailable))
                      return;
                    if (menuShared) void remote.share(workspace.id, false);
                    else setSharingId(workspace.id);
                  },
                }
              : null,
            {
              label: "Customize workspace…",
              run: () => setCustomizing(workspace),
            },
            {
              label: "Delete workspace…",
              danger: true,
              run: () => onDelete(workspace),
            },
          ]}
          onClose={() => {
            setMenu(undefined);
            trigger.current?.focus({ preventScroll: true });
          }}
        />
      )}
      {customizing && (
        <WorkspaceAppearanceDialog
          key={customizing.id}
          workspace={customizing}
          onSave={(appearance) => onAppearanceChange(customizing, appearance)}
          onClose={() => setCustomizing(undefined)}
        />
      )}
      {sharingWorkspace && remote && !sharingActive && (
        <StartRemoteSharingDialog
          key={sharingWorkspace.id}
          workspace={sharingWorkspace}
          available={sharingAvailable}
          qualified={!!remote.state?.qualified}
          signedIn={remote.available}
          busy={!!remote.busy}
          error={remote.error}
          onConfirm={() => remote.share(sharingWorkspace.id, true)}
          onClose={() => setSharingId(undefined)}
        />
      )}
    </div>
  );
}

function StartRemoteSharingDialog({
  workspace,
  available,
  qualified,
  signedIn,
  busy,
  error,
  onConfirm,
  onClose,
}: {
  workspace: Workspace;
  available: boolean;
  qualified: boolean;
  signedIn: boolean;
  busy: boolean;
  error: string;
  onConfirm: () => Promise<void>;
  onClose: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  const descriptionId = useId();
  const [attempted, setAttempted] = useState(false);
  return (
    <Modal
      className="remote-sharing-dialog"
      title="Share workspace remotely?"
      role="alertdialog"
      tone="warning"
      protectTheme
      descriptionId={descriptionId}
      initialFocus={cancel}
      closeDisabled={busy}
      onClose={onClose}
    >
      <div className="dialog-form" aria-busy={busy}>
        <p id={descriptionId}>
          Share “{workspace.name}” remotely? Browsers signed in to your account
          will be able to view and control all terminals in this workspace,
          including inactive tabs and new splits.
          {!signedIn && " Sign in to use remote sharing."}
          {!qualified &&
            " Remote hosting currently requires Lomi on a Mac with Apple silicon."}
        </p>
        {attempted && error && <p role="alert">{error}</p>}
        <div className="dialog-actions">
          <button
            ref={cancel}
            type="button"
            className="button"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            className="button button-primary"
            disabled={!available || busy}
            onClick={() => {
              if (!available) return;
              setAttempted(true);
              void onConfirm();
            }}
          >
            {busy ? "Sharing…" : "Share remotely"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
