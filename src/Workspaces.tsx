import type { RemoteState } from "./remote-workspaces";
import type { SharedHostDesktopUncertaintyWarning } from "./shared-host";
import ResourceIcon from "./ResourceIcon";
import {
  useEffect,
  useId,
  useMemo,
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
import type { LayoutPane, Project, Tab, Workspace } from "./model";
import { layoutPanes, tabTitle } from "./model";
import type { GitRepositoryScan } from "./api";
import type { WorkspaceAppearance } from "./workspace-appearance";
import ContextMenu from "./ContextMenu";
import { IconButton, Modal } from "./ui";
import WorkspaceAvatar from "./WorkspaceAvatar";
import WorkspaceAppearanceDialog from "./WorkspaceAppearanceDialog";
import {
  getTerminalAgents,
  getTerminalAgentStates,
  runningTerminal,
  subscribeTerminalAgents,
  subscribeTerminalAgentStates,
  subscribeTerminalLifecycle,
} from "./terminal-runtime";
import { summarizeWorkspaceAgents } from "./workspace-summary";
import { CliAgentIcon } from "./CliAgentIcon";
import { cliNames } from "./cli-agents";
import type { CliAgent } from "./cli-agents";
import { cliTerminalDisplayTitle } from "./cli-terminal-title";
import type { AgentUsageEntry } from "./agent-usage";
import {
  aggregateCliActivity,
  usageCliActivity,
  type CliActivity,
} from "./cli-activity";
import useWorkspaceRepositories from "./useWorkspaceRepositories";
import {
  getNotificationSnapshot,
  subscribeNotificationSnapshot,
  unreadTerminalPaneIds,
} from "./notifications";

const tabItems = (tab: Tab): Array<Tab | LayoutPane> =>
  tab.type === "terminal" ? layoutPanes(tab.layout) : [tab];

const cliActivityLabels: Record<CliActivity, string> = {
  working: "Working",
  attention: "Needs input",
  error: "CLI error or usage limit reached",
};

function toggleExpanded(current: Set<string>, id: string) {
  const next = new Set(current);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

export default function Workspaces({
  projects,
  activeWorkspaceId,
  activeRoot,
  git,
  usage,
  onSelect,
  onReveal,
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
    uncertaintyWarnings: SharedHostDesktopUncertaintyWarning[];
    acknowledgeUncertaintyWarning: (
      warning: SharedHostDesktopUncertaintyWarning,
    ) => Promise<void>;
    share: (id: string, shared: boolean) => Promise<void>;
  };
  activeWorkspaceId?: string;
  activeRoot: string;
  git: GitRepositoryScan & { loading: boolean };
  usage?: {
    entries: AgentUsageEntry[];
    now: number;
    failed: boolean;
  };
  onSelect: (
    path: string,
    workspaceId: string,
    tabId?: string,
    paneId?: string,
  ) => void;
  onReveal: (
    path: string,
    workspaceId: string,
    tabId: string,
    paneId: string,
  ) => void;
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
  const agentStates = useSyncExternalStore(
    subscribeTerminalAgentStates,
    getTerminalAgentStates,
  );
  const activities = useMemo(() => {
    const result: Record<string, CliActivity | null> = {};
    for (const [paneId, state] of Object.entries(agentStates)) {
      if (state.cli === "codex") {
        // Account quota windows do not describe the current Codex turn.
        // Its reported lifecycle must remain authoritative for the icon.
        result[paneId] = state.activity;
        continue;
      }
      const entry = usage?.failed
        ? undefined
        : usage?.entries.find(
            (entry) =>
              entry.id === state.sessionId &&
              entry.process.cli === state.cli &&
              entry.process.pid === state.process?.pid,
          );
      result[paneId] = aggregateCliActivity([
        state.activity,
        usageCliActivity(entry ?? null, usage?.now),
      ]);
    }
    return result;
  }, [agentStates, usage?.entries, usage?.now, usage?.failed]);
  const activityFor = (items: Array<Tab | LayoutPane>, cli?: CliAgent) =>
    aggregateCliActivity(
      items
        .filter(
          (item) =>
            item.type === "terminal" && (!cli || agents[item.id] === cli),
        )
        .map((item) => activities[item.id]),
    );
  const notifications = useSyncExternalStore(
    subscribeNotificationSnapshot,
    getNotificationSnapshot,
  );
  const unreadPanes = useMemo(
    () => unreadTerminalPaneIds(notifications),
    [notifications],
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
  const sharingAvailable = !!remote?.available && !!remote.state;
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
        <span>Workspaces</span>
        <IconButton title="New workspace" onClick={onNew}>
          <Plus size={15} />
        </IconButton>
      </div>
      {remote?.error && (
        <p className="sidebar-empty" role="alert">
          {remote.error}
        </p>
      )}
      {remote?.uncertaintyWarnings.map((warning) => (
        <div
          className="sidebar-empty"
          role="alert"
          key={`${warning.operationId}:${warning.requestHash}`}
        >
          <p>{warning.message}</p>
          <button
            type="button"
            className="button"
            onClick={() => void remote.acknowledgeUncertaintyWarning(warning)}
          >
            Acknowledge
          </button>
        </div>
      ))}
      <nav className="workspace-list" aria-label="Workspace list">
        {projects.flatMap((project) =>
          project.workspaces.map((workspace) => {
            const summary = summarizeWorkspaceAgents(workspace, agents);
            const workspaceItems = workspace.tabs.flatMap(tabItems);
            let terminalIndex = 0;
            const openItems = workspace.tabs.flatMap((tab) =>
              tabItems(tab).map((item) => ({
                item,
                ownerTab: tab,
                terminalIndex:
                  item.type === "terminal" ? terminalIndex++ : undefined,
              })),
            );
            const hasUnread = workspaceItems.some(
              (item) => item.type === "terminal" && unreadPanes.has(item.id),
            );
            const repository = repositories[project.path];
            const shared = remote?.state?.workspaces.find(
              (candidate) => candidate.id === workspace.id,
            )?.shared;
            const extraAgents = summary.groups.slice(2);
            const extraActivity = aggregateCliActivity(
              extraAgents.map(({ cli }) => activityFor(workspaceItems, cli)),
            );
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
                      : "");
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
                    aria-label={`${workspace.name} ${project.path} ${metadata}${shared ? " Shared remotely" : ""}${hasUnread ? " Unread terminal notifications" : ""}`}
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
                    <span className="workspace-name-row">
                      {workspace.appearance ? (
                        <WorkspaceAvatar appearance={workspace.appearance} />
                      ) : (
                        <Folder size={14} aria-hidden="true" />
                      )}
                      <span className="workspace-name">{workspace.name}</span>
                      {hasUnread && <WorkspaceNotificationDot />}
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
                    <span className="workspace-folder-row">
                      <span className="workspace-folder" title={project.path}>
                        <bdi dir="ltr">{project.path}</bdi>
                      </span>
                      {metadata && (
                        <span
                          className="workspace-metadata"
                          title={metadataTitle}
                        >
                          {metadata}
                        </span>
                      )}
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
                        {summary.groups.slice(0, 2).map(({ cli, count }) => {
                          const activity = activityFor(workspaceItems, cli);
                          const label = `${cliNames[cli]} · ${count} ${count === 1 ? "agent" : "agents"}`;
                          const status = activity
                            ? cliActivityLabels[activity]
                            : undefined;
                          return (
                            <span
                              className="workspace-agent-chip"
                              key={cli}
                              role="img"
                              data-cli={cli}
                              data-activity={activity ?? undefined}
                              aria-label={label}
                              aria-description={status}
                              title={status ? `${label} · ${status}` : label}
                            >
                              <WorkspaceCliIcon cli={cli} activity={activity} />
                            </span>
                          );
                        })}
                        {extraAgents.length > 0 && (
                          <span
                            className="workspace-agent-chip workspace-agent-overflow"
                            role="img"
                            data-activity={extraActivity ?? undefined}
                            aria-label={extraAgentsLabel}
                            aria-description={
                              extraActivity
                                ? cliActivityLabels[extraActivity]
                                : undefined
                            }
                            title={`${extraAgentsLabel}${extraActivity ? ` · ${cliActivityLabels[extraActivity]}` : ""}`}
                          >
                            +{extraAgents.length}
                            {extraActivity && (
                              <WorkspaceCliActivityDot
                                activity={extraActivity}
                              />
                            )}
                          </span>
                        )}
                      </div>
                    )}
                    <IconButton
                      className="icon-button workspace-tabs-toggle"
                      title={`${expanded.has(workspace.id) ? "Collapse" : "Expand"} items in ${workspace.name}`}
                      aria-expanded={expanded.has(workspace.id)}
                      aria-controls={`workspace-items-${workspace.id}`}
                      onClick={() =>
                        setExpanded((current) =>
                          toggleExpanded(current, workspace.id),
                        )
                      }
                    >
                      <ChevronRight size={12} aria-hidden="true" />
                    </IconButton>
                  </div>
                </div>
                <ul
                  id={`workspace-items-${workspace.id}`}
                  className="workspace-item-list"
                  aria-label={`Open items in ${workspace.name}`}
                  hidden={!expanded.has(workspace.id)}
                >
                  {expanded.has(workspace.id) &&
                    openItems.map(
                      ({
                        item,
                        ownerTab,
                        terminalIndex: itemTerminalIndex,
                      }) => {
                        const activity =
                          item.type === "terminal"
                            ? activities[item.id]
                            : undefined;
                        const unread =
                          item.type === "terminal" && unreadPanes.has(item.id);
                        const active =
                          workspace.id === activeWorkspaceId &&
                          ownerTab.id === workspace.activeTabId &&
                          (ownerTab.type !== "terminal" ||
                            ownerTab.activePaneId === item.id);
                        return (
                          <WorkspaceOpenItem
                            key={`${ownerTab.id}:${item.id}`}
                            item={item}
                            ownerTab={ownerTab}
                            terminalIndex={itemTerminalIndex}
                            agent={agents[item.id]}
                            activity={activity}
                            unread={unread}
                            active={active}
                            onSelect={() =>
                              onSelect(
                                project.path,
                                workspace.id,
                                ownerTab.id,
                                item.id,
                              )
                            }
                            onReveal={() =>
                              onReveal(
                                project.path,
                                workspace.id,
                                ownerTab.id,
                                item.id,
                              )
                            }
                          />
                        );
                      },
                    )}
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

function WorkspaceNotificationDot() {
  return (
    <span
      className="workspace-notification-dot"
      role="img"
      aria-label="Unread terminal notifications"
      title="Unread terminal notifications"
    />
  );
}

function WorkspaceCliActivityDot({ activity }: { activity: CliActivity }) {
  return (
    <span
      className="workspace-cli-activity-dot"
      data-activity={activity}
      aria-hidden="true"
    />
  );
}

function WorkspaceCliIcon({
  cli,
  activity,
}: {
  cli: CliAgent;
  activity?: CliActivity | null;
}) {
  return (
    <span
      className="workspace-cli-icon"
      data-activity={activity ?? undefined}
      title={`${cliNames[cli]}${activity ? ` · ${cliActivityLabels[activity]}` : ""}`}
      aria-hidden="true"
    >
      <CliAgentIcon cli={cli} />
      {activity && <WorkspaceCliActivityDot activity={activity} />}
    </span>
  );
}

function WorkspaceItemIcon({
  item,
  agent,
  activity,
}: {
  item: Tab | LayoutPane;
  agent?: CliAgent;
  activity?: CliActivity | null;
}) {
  if (item.type === "file" || item.type === "diff")
    return (
      <ResourceIcon
        path={`${item.root}/${item.relative}`}
        size={14}
        fallback={item.type === "diff" ? FileDiff : undefined}
      />
    );
  const Icon =
    item.type === "commit"
      ? GitCommitHorizontal
      : item.type === "browser"
        ? Globe
        : item.type === "android"
          ? Monitor
          : item.type === "chat"
            ? MessageSquare
            : item.type === "plugin"
              ? Puzzle
              : Terminal;
  return item.type === "terminal" && agent ? (
    <WorkspaceCliIcon cli={agent} activity={activity} />
  ) : (
    <Icon size={14} aria-hidden="true" />
  );
}

const emptySubscribe = () => () => {};
const emptySnapshot = () => null;

function WorkspaceOpenItem({
  item,
  ownerTab,
  terminalIndex,
  agent,
  activity,
  unread,
  active,
  onSelect,
  onReveal,
}: {
  item: Tab | LayoutPane;
  ownerTab: Tab;
  terminalIndex?: number;
  agent?: CliAgent;
  activity?: CliActivity | null;
  unread: boolean;
  active: boolean;
  onSelect: () => void;
  onReveal: () => void;
}) {
  const runtime = useSyncExternalStore(subscribeTerminalLifecycle, () =>
    item.type === "terminal" ? runningTerminal(item.id) : undefined,
  );
  const snapshot = useSyncExternalStore(
    runtime?.subscribe ?? emptySubscribe,
    runtime?.getSnapshot ?? emptySnapshot,
  );
  const title =
    item.type === "terminal"
      ? (cliTerminalDisplayTitle(
          snapshot?.title,
          snapshot?.foregroundProgram,
          agent,
        ) ?? `Terminal ${(terminalIndex ?? 0) + 1}`)
      : tabTitle(item);
  const ownerTabTitle = tabTitle(ownerTab);
  const detail =
    item.type === "terminal" && "cwd" in item
      ? item.cwd
      : item.type === "file" || item.type === "diff"
        ? item.relative
        : item.type === "browser"
          ? item.url
          : title;
  const description = [
    `In tab ${ownerTabTitle}`,
    activity ? cliActivityLabels[activity] : undefined,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <li className="workspace-open-item-entry">
      <button
        type="button"
        className="workspace-open-item"
        aria-current={active ? "true" : undefined}
        aria-description={description}
        data-workspace-pane-id={item.id}
        data-owner-tab-id={ownerTab.id}
        title={`${title}${title === detail ? "" : `\n${detail}`}\nTab: ${ownerTabTitle}`}
        onClick={onSelect}
        onDoubleClick={onReveal}
      >
        <WorkspaceItemIcon item={item} agent={agent} activity={activity} />
        <span className="workspace-item-label">{title}</span>
        {unread && <WorkspaceNotificationDot />}
      </button>
    </li>
  );
}

function StartRemoteSharingDialog({
  workspace,
  available,
  signedIn,
  busy,
  error,
  onConfirm,
  onClose,
}: {
  workspace: Workspace;
  available: boolean;
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
      <div className="dialog-form dialog-layout" aria-busy={busy}>
        <div className="dialog-content">
          <p id={descriptionId}>
            Share “{workspace.name}” with approved desktop, browser, and mobile
            windows signed in to this account? They can request access to this
            workspace’s shared terminal tree and project files. After you
            approve a request, they can view files, create files and folders,
            rename or remove entries, inspect Git status,
            diffs and history, stage changes, commit, fetch, fast-forward pull,
            and push. They can also create, control, resize, and close shared
            terminals.
            {" "}Editor buffers, chat, browser panes, plugins, and AI-controlled
            terminals stay private. Sharing does not start a shell; choose
            Create terminal to start one.
            {!signedIn && " Sign in to use remote sharing."}
          </p>
          {attempted && error && <p role="alert">{error}</p>}
        </div>
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
