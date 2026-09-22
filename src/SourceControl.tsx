import type { ReactNode } from "react";
import type { SourceControlState } from "./source-control-state";
import { useId, useRef, useState, useSyncExternalStore } from "react";
import {
  Check,
  CircleAlert,
  Info,
  ChevronDown,
  ChevronRight,
  FileDiff,
  GitBranch,
  RefreshCw,
  SquareArrowRight,
  SquareDot,
  SquareMinus,
  SquarePlus,
} from "./icons";
import { api, errorMessage } from "./api";
import type { GitChange, GitCommitSummary, GitStatus } from "./api";
import GitHistory from "./GitHistory";
import ContextMenu from "./ContextMenu";
import { useGitFileActions } from "./GitFileActions";
import { IconButton, Modal } from "./ui";

export default function SourceControl({
  projectRoot,
  repositories,
  state,
  errors,
  limited,
  loading = false,
  onRefresh,
  onPull,
  onDiff,
  onOpenCommit,
  onOpenFile,
  onDiscard,
  onError,
}: {
  projectRoot: string;
  repositories: GitStatus[];
  state: SourceControlState;
  errors: { root: string; message: string }[];
  limited: boolean;
  loading?: boolean;
  onRefresh: () => void;
  onPull: (root: string, rebase: boolean) => Promise<void>;
  onDiff: (root: string, path: string, staged: boolean) => void;
  onOpenCommit: (root: string, commit: GitCommitSummary) => void;
  onOpenFile: (root: string, path: string) => void;
  onDiscard: (root: string, change: GitChange) => Promise<void>;
  onError: (message: string) => void;
}) {
  useSyncExternalStore(state.subscribe, state.snapshot);
  const [scanDetails, setScanDetails] = useState(false);
  const changesId = useId();
  const changedRepositories = repositories.filter(
    (repository) => repository.changes.length > 0,
  );
  const scanAction = (errors.length > 0 || limited) && (
    <IconButton
      title={
        errors.length ? "Repository scan errors" : "Repository scan details"
      }
      onClick={() => setScanDetails(true)}
    >
      {errors.length ? <CircleAlert size={14} /> : <Info size={14} />}
    </IconButton>
  );
  const selectedRoot = state.selection(projectRoot);
  const setSelectedRoot = (root: string | null) =>
    state.select(projectRoot, root);
  const selected = repositories.find(
    (repository) => repository.root === selectedRoot,
  );
  const active = repositories.length === 1 ? repositories[0] : selected;
  const label = (root: string) => {
    const project = projectRoot.replace(/\\/g, "/").replace(/\/$/, "");
    const path = root.replace(/\\/g, "/");
    const relative = path.startsWith(`${project}/`)
      ? path.slice(project.length + 1)
      : path === project
        ? "."
        : root;
    return relative === "."
      ? projectRoot.split(/[\\/]/).at(-1) || root
      : relative;
  };
  return (
    <div className="sidebar-panel source-panel">
      {(errors.length > 0 || limited) && (
        <>
          {scanDetails && (
            <Modal
              title="Repository scan"
              onClose={() => setScanDetails(false)}
            >
              <div className="dialog-body source-scan-details">
                {errors.map((error, index) => (
                  <p key={`${error.root}:${index}`}>
                    <strong>{label(error.root)}</strong>: {error.message}
                  </p>
                ))}
                {errors.length > 0 && (
                  <p>
                    Some repositories could not be refreshed. Previously loaded
                    statuses are kept until the next successful refresh.
                  </p>
                )}
                {limited && (
                  <p>
                    Only part of this folder was scanned. Open a more specific
                    project folder to see other repositories.
                  </p>
                )}
              </div>
              <div className="dialog-actions">
                <button
                  type="button"
                  className="button"
                  onClick={() => setScanDetails(false)}
                >
                  Close
                </button>
                <button
                  type="button"
                  className="button"
                  onClick={() => {
                    setScanDetails(false);
                    onRefresh();
                  }}
                >
                  Retry repository scan
                </button>
              </div>
            </Modal>
          )}
        </>
      )}
      {repositories.length > 1 && (
        <nav className="source-repositories" aria-label="Repositories">
          <div className="source-repositories-heading">REPOSITORIES</div>
          <button
            type="button"
            className="source-repository"
            aria-current={!active ? "page" : undefined}
            onClick={() => setSelectedRoot(null)}
          >
            <span>All repositories</span>
            <span className="git-count">{repositories.length}</span>
          </button>
          {repositories.map((repository) => (
            <button
              key={repository.root}
              type="button"
              className="source-repository"
              aria-current={
                active?.root === repository.root ? "page" : undefined
              }
              title={repository.root}
              onClick={() => setSelectedRoot(repository.root)}
            >
              <GitBranch size={13} />
              <span className="source-repository-name">
                {label(repository.root)}
              </span>
              <span className="git-count">{repository.changes.length}</span>
            </button>
          ))}
        </nav>
      )}
      {!active && repositories.length > 1 ? (
        <div className="source-all">
          <header className="source-heading">
            <span>All changes</span>
            <div className="source-heading-actions">
              {scanAction}
              <IconButton title="Refresh source control" onClick={onRefresh}>
                <RefreshCw size={14} />
              </IconButton>
            </div>
          </header>
          <div className="source-all-list">
            {changedRepositories.length === 0 && (
              <div className="git-clean">
                <Check size={20} />
                <p>No changes.</p>
              </div>
            )}
            {changedRepositories.map((repository, index) => {
              const collapsed = state.repository(
                repository.root,
              ).changesCollapsed;
              const listId = `${changesId}-${index}`;
              return (
                <section
                  className="source-all-repository"
                  key={repository.root}
                  aria-label={label(repository.root)}
                >
                  <button
                    type="button"
                    className="source-all-repository-heading"
                    aria-expanded={!collapsed}
                    aria-controls={listId}
                    onClick={() =>
                      state.update(repository.root, {
                        changesCollapsed: !collapsed,
                      })
                    }
                    title={`${collapsed ? "Expand" : "Collapse"} ${label(repository.root)}`}
                  >
                    {collapsed ? (
                      <ChevronRight size={13} aria-hidden="true" />
                    ) : (
                      <ChevronDown size={13} aria-hidden="true" />
                    )}
                    <span className="source-all-repository-name">
                      {label(repository.root)}
                    </span>
                    <span className="git-count source-all-branch">
                      {repository.branch}
                    </span>
                    <span className="git-count">
                      {repository.changes.length}
                    </span>
                  </button>
                  <div id={listId} hidden={collapsed}>
                    {repository.changes.flatMap((change) => {
                      const entries: { staged: boolean; code: string }[] = [];
                      if (![" ", "?", "!"].includes(change.index))
                        entries.push({ staged: true, code: change.index });
                      if (![" ", "!"].includes(change.worktree))
                        entries.push({ staged: false, code: change.worktree });
                      return entries.map(({ staged, code }) => (
                        <button
                          key={`${change.path}:${staged}`}
                          type="button"
                          className="source-all-file"
                          title={`${staged ? "Staged" : "Working"}: ${change.path}`}
                          onClick={() =>
                            onDiff(repository.root, change.path, staged)
                          }
                        >
                          <span className="git-file-icon" data-status={code}>
                            {code}
                          </span>
                          <span>{change.path}</span>
                          {staged && <span className="git-count">staged</span>}
                        </button>
                      ));
                    })}
                  </div>
                </section>
              );
            })}
          </div>
        </div>
      ) : (
        <RepositoryControl
          key={active?.root ?? "empty"}
          status={active ?? null}
          loading={loading}
          scanAction={scanAction}
          state={state}
          onRefresh={onRefresh}
          onPull={(rebase) => onPull(active!.root, rebase)}
          onDiff={(path, staged) => onDiff(active!.root, path, staged)}
          onOpenCommit={(commit) => onOpenCommit(active!.root, commit)}
          onOpenFile={(path) => onOpenFile(active!.root, path)}
          onDiscard={(change) => onDiscard(active!.root, change)}
          onError={onError}
        />
      )}
    </div>
  );
}

function RepositoryControl({
  status,
  loading = false,
  scanAction,
  state,
  onRefresh,
  onPull,
  onDiff,
  onOpenCommit,
  onOpenFile,
  onDiscard,
  onError,
}: {
  status: GitStatus | null;
  loading?: boolean;
  scanAction: ReactNode;
  state: SourceControlState;
  onRefresh: () => void;
  onPull: (rebase: boolean) => Promise<void>;
  onDiff: (path: string, staged: boolean) => void;
  onOpenCommit: (commit: GitCommitSummary) => void;
  onOpenFile: (path: string) => void;
  onDiscard: (change: GitChange) => Promise<void>;
  onError: (message: string) => void;
}) {
  const root = status?.root ?? "";
  const { busy, message, remoteStatus, historyRevision } =
    state.repository(root);
  const onMessageChange = (message: string) => state.update(root, { message });
  const setRemoteStatus = (remoteStatus: string) =>
    state.update(root, { remoteStatus });
  const [remoteMenu, setRemoteMenu] = useState<{
    x: number;
    y: number;
    choice?: { action: "fetch" | "push"; remotes: string[] };
  }>();
  const [forcePush, setForcePush] = useState(false);
  const remoteTrigger = useRef<HTMLButtonElement>(null);
  const openRemoteMenu = () => {
    if (busy) return;
    const bounds = remoteTrigger.current!.getBoundingClientRect();
    setRemoteMenu({ x: bounds.left, y: bounds.bottom + 4 });
  };
  const closeRemoteMenu = () => {
    setRemoteMenu(undefined);
    remoteTrigger.current?.focus({ preventScroll: true });
  };
  const [page, setPage] = useState<"changes" | "history">("changes");
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const groupId = useId();
  const run = async (action: () => Promise<void>, progress = "") => {
    try {
      await state.run(root, action, progress);
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      onRefresh();
    }
  };
  const fetch = (remote?: string) =>
    run(async () => {
      await api("git_fetch", { root: status!.root, remote });
      setRemoteStatus("Fetch complete.");
    }, "Fetching…");
  const pull = (rebase = false) =>
    run(
      async () => {
        await onPull(rebase);
        setRemoteStatus(
          rebase ? "Pull with rebase complete." : "Pull complete.",
        );
      },
      rebase ? "Pulling with rebase…" : "Pulling…",
    );
  const push = (remote?: string, force = false) =>
    run(async () => {
      await api("git_push", { root: status!.root, remote, force });
      setRemoteStatus("Push complete.");
    }, "Pushing…");
  const chooseRemote = (action: "fetch" | "push") =>
    run(async () => {
      const remotes = await api<string[]>("git_remotes", {
        root: status!.root,
      });
      if (!remotes.length)
        throw new Error(
          "No Git remote is configured. Add a remote in a terminal, then try again.",
        );
      setRemoteMenu({ ...remoteMenu!, choice: { action, remotes } });
      setRemoteStatus("");
    }, "Loading remotes…");
  const stage = (changes: GitChange[], stage: boolean) =>
    run(() =>
      api("git_stage", {
        root: status!.root,
        paths: [
          ...new Set(
            changes.flatMap((change) =>
              !stage && change.originalPath
                ? [change.path, change.originalPath]
                : [change.path],
            ),
          ),
        ],
        stage,
      }),
    );
  const actions = useGitFileActions({
    root: status?.root ?? "",
    onDiff,
    onOpenFile,
    onOpenCommit,
    onError,
    working: {
      busy,
      onStage: (change, staged) => void stage([change], staged),
      onDiscard,
      onRun: (action) => state.run(root, action),
      onRefresh,
    },
  });
  if (!status)
    return (
      <div className="sidebar-panel">
        <header className="sidebar-heading">
          <span>SOURCE CONTROL</span>
          {scanAction}
        </header>
        <p className="sidebar-empty" role={loading ? "status" : undefined}>
          {loading
            ? "Checking for a Git repository…"
            : "No Git repository was found in this project."}
        </p>
      </div>
    );
  const staged = status.changes.filter(
    (change) => ![" ", "?", "!"].includes(change.index),
  );
  const unstaged = status.changes.filter(
    (change) => ![" ", "!"].includes(change.worktree),
  );
  const tracked = unstaged.filter((change) => change.worktree !== "?");
  const untracked = unstaged.filter((change) => change.worktree === "?");
  const group = (name: string, changes: GitChange[], isStaged: boolean) => (
    <section className="git-group" aria-label={`${name} changes`}>
      <header>
        <button
          type="button"
          className="git-group-toggle"
          aria-expanded={!collapsed[name]}
          aria-controls={`${groupId}-${name}`}
          onClick={() =>
            setCollapsed((current) => ({ ...current, [name]: !current[name] }))
          }
        >
          {collapsed[name] ? (
            <ChevronRight size={12} />
          ) : (
            <ChevronDown size={12} />
          )}
          <span>{name}</span>
          <span className="git-count">{changes.length}</span>
        </button>
        <label className="git-stage-toggle">
          <input
            type="checkbox"
            aria-label={
              isStaged
                ? "Unstage staged changes"
                : `Stage ${name.toLowerCase()} changes`
            }
            title={
              isStaged
                ? "Unstage staged changes"
                : `Stage ${name.toLowerCase()} changes`
            }
            checked={isStaged}
            disabled={busy}
            onChange={() => void stage(changes, !isStaged)}
          />
        </label>
      </header>
      <ul
        className="git-file-list"
        id={`${groupId}-${name}`}
        hidden={collapsed[name]}
      >
        {changes.map((change) => {
          const separator = change.path.lastIndexOf("/");
          const filename = change.path.slice(separator + 1);
          const directory =
            separator < 0 ? "" : change.path.slice(0, separator);
          const code = isStaged ? change.index : change.worktree;
          const StatusIcon = ["A", "?"].includes(code)
            ? SquarePlus
            : code === "D"
              ? SquareMinus
              : ["R", "C"].includes(code)
                ? SquareArrowRight
                : SquareDot;
          const action = `${isStaged ? "Unstage" : "Stage"} ${change.path}`;
          const file = {
            path: change.path,
            change,
            staged: isStaged,
            deleted:
              change.worktree === "D" ||
              (change.index === "D" && change.worktree === " "),
          };
          return (
            <li
              className="git-file"
              key={change.path}
              onContextMenu={(event) => actions.onContext(event, file)}
              onKeyDown={(event) => actions.onKey(event, file)}
            >
              <button
                type="button"
                className="git-file-open"
                aria-label={`View ${isStaged ? "staged " : ""}diff for ${change.path}`}
                title={
                  change.originalPath
                    ? `${change.originalPath} → ${change.path}`
                    : change.path
                }
                onClick={() => onDiff(change.path, isStaged)}
              >
                <StatusIcon
                  size={13}
                  className="git-file-icon"
                  data-status={code}
                />
                <span className="git-file-label">
                  <span className="git-file-name">{filename}</span>
                  {directory && (
                    <span className="git-file-directory">{directory}</span>
                  )}
                </span>
              </button>
              <label className="git-stage-toggle" title={action}>
                <input
                  type="checkbox"
                  aria-label={action}
                  checked={isStaged}
                  disabled={busy}
                  onChange={() => void stage([change], !isStaged)}
                />
              </label>
            </li>
          );
        })}
      </ul>
    </section>
  );
  return (
    <div className="source-repository-control">
      <header className="source-heading">
        <div
          className="source-pages"
          role="tablist"
          aria-label="Source control pages"
        >
          {(["changes", "history"] as const).map((name, index) => (
            <button
              key={name}
              type="button"
              role="tab"
              id={`${groupId}-page-${name}`}
              aria-controls={`${groupId}-panel-${name}`}
              aria-selected={page === name}
              tabIndex={page === name ? 0 : -1}
              onClick={() => setPage(name)}
              onKeyDown={(event) => {
                if (
                  !["ArrowLeft", "ArrowRight", "Home", "End"].includes(
                    event.key,
                  )
                )
                  return;
                event.preventDefault();
                const next =
                  event.key === "Home"
                    ? "changes"
                    : event.key === "End"
                      ? "history"
                      : index
                        ? "changes"
                        : "history";
                setPage(next);
                document.getElementById(`${groupId}-page-${next}`)?.focus();
              }}
            >
              {name === "changes" ? (
                <>
                  Changes{" "}
                  <span className="git-count">({status.changes.length})</span>
                </>
              ) : (
                "History"
              )}
            </button>
          ))}
        </div>
        <div className="source-heading-actions">
          {scanAction}
          <IconButton
            title="Refresh source control"
            disabled={busy}
            onClick={() => {
              onRefresh();
              state.update(root, {
                historyRevision: state.repository(root).historyRevision + 1,
              });
            }}
          >
            <RefreshCw size={14} />
          </IconButton>
        </div>
      </header>
      <div className="git-remote-actions">
        <button
          ref={remoteTrigger}
          type="button"
          className="button git-remote-trigger"
          title="Git remote actions"
          aria-haspopup="menu"
          aria-expanded={!!remoteMenu}
          disabled={busy}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={() => (remoteMenu ? closeRemoteMenu() : openRemoteMenu())}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              openRemoteMenu();
            }
          }}
        >
          <span>
            <RefreshCw size={12} /> Fetch
          </span>
          <span className="git-remote-chevron">
            <ChevronDown size={12} />
          </span>
        </button>
      </div>
      {remoteMenu && (
        <ContextMenu
          key={remoteMenu.choice?.action ?? "actions"}
          {...remoteMenu}
          label={
            remoteMenu.choice
              ? remoteMenu.choice.action === "fetch"
                ? "Fetch From"
                : "Push To"
              : "Git remote actions"
          }
          onClose={closeRemoteMenu}
          actions={
            remoteMenu.choice
              ? remoteMenu.choice.remotes.map((remote) => ({
                  label: remote,
                  run: () =>
                    void (remoteMenu.choice!.action === "fetch"
                      ? fetch(remote)
                      : push(remote)),
                }))
              : [
                  {
                    label: "Fetch",
                    run: () => void fetch(),
                  },
                  {
                    label: "Fetch From",
                    run: () => void chooseRemote("fetch"),
                  },
                  {
                    label: "Pull",
                    run: () => void pull(),
                  },
                  {
                    label: "Pull (Rebase)",
                    run: () => void pull(true),
                  },
                  null,
                  {
                    label: "Push",
                    run: () => void push(),
                  },
                  {
                    label: "Push To",
                    run: () => void chooseRemote("push"),
                  },
                  {
                    label: "Force Push",
                    danger: true,
                    run: () => setForcePush(true),
                  },
                ]
          }
        />
      )}
      {forcePush && (
        <Modal
          title="Force Push"
          tone="warning"
          onClose={() => setForcePush(false)}
        >
          <div className="dialog-body">
            <p>
              Replace the remote branch history with your local commits? The
              push is rejected if the remote branch changed since your last
              fetch.
            </p>
          </div>
          <div className="dialog-actions">
            <button
              type="button"
              className="button"
              onClick={() => setForcePush(false)}
            >
              Cancel
            </button>
            <button
              type="button"
              className="button text-error"
              disabled={busy}
              onClick={() => {
                setForcePush(false);
                void push(undefined, true);
              }}
            >
              Force Push
            </button>
          </div>
        </Modal>
      )}
      {remoteStatus && (
        <div className="git-remote-status" role="status">
          {remoteStatus}
        </div>
      )}
      {page === "history" ? (
        <div
          className="source-page"
          role="tabpanel"
          id={`${groupId}-panel-history`}
          aria-labelledby={`${groupId}-page-history`}
        >
          <GitHistory
            key={`${status.root}:${historyRevision}`}
            root={status.root}
            onOpenCommit={onOpenCommit}
          />
        </div>
      ) : (
        <div
          className="source-page"
          role="tabpanel"
          id={`${groupId}-panel-changes`}
          aria-labelledby={`${groupId}-page-changes`}
        >
          <div className="git-toolbar">
            <span>
              <FileDiff size={13} /> Working tree
            </span>
            <button
              type="button"
              className="button git-stage-all"
              aria-label={
                unstaged.length || !staged.length
                  ? "Stage all changes"
                  : "Unstage all changes"
              }
              disabled={busy || !status.changes.length}
              onClick={() =>
                void stage(
                  unstaged.length ? unstaged : staged,
                  !!unstaged.length,
                )
              }
            >
              {unstaged.length || !staged.length ? "Stage All" : "Unstage All"}
            </button>
          </div>
          <div className="git-groups">
            {!!staged.length && group("Staged", staged, true)}
            {!!tracked.length && group("Tracked", tracked, false)}
            {!!untracked.length && group("Untracked", untracked, false)}
            {!status.changes.length && (
              <div className="git-clean">
                <Check size={20} />
                <p>Working tree clean.</p>
                <span>No changes to commit.</span>
              </div>
            )}
          </div>
          <div className="git-repository-bar">
            <div className="git-branch" title={status.branch}>
              <GitBranch size={13} />
              <span>{status.branch}</span>
            </div>
            <span className="git-staged-count">{staged.length} staged</span>
          </div>
          <form
            className="commit-form"
            onSubmit={(event) => {
              event.preventDefault();
              if (busy || !message.trim() || !staged.length) return;
              void run(async () => {
                await api("git_commit", { root: status.root, message });
                state.clearSubmittedMessage(root, message);
              });
            }}
          >
            <textarea
              aria-label="Commit message"
              placeholder="Enter commit message"
              value={message}
              onChange={(event) => onMessageChange(event.target.value)}
              readOnly={busy}
              rows={5}
            />
            <div className="commit-actions">
              <button
                type="submit"
                className="button commit-button"
                aria-label="Commit staged changes"
                disabled={busy || !message.trim() || !staged.length}
              >
                <Check size={13} />
                {busy ? "Working…" : "Commit Staged"}
              </button>
            </div>
          </form>
        </div>
      )}
      {actions.menu}
      {actions.dialogs}
    </div>
  );
}
