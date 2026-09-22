import { useId, useRef, useState } from "react";
import {
  Check,
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
  status,
  loading = false,
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
  onRefresh: () => void;
  onPull: (rebase: boolean) => Promise<void>;
  onDiff: (path: string, staged: boolean) => void;
  onOpenCommit: (commit: GitCommitSummary) => void;
  onOpenFile: (path: string) => void;
  onDiscard: (change: GitChange) => Promise<void>;
  onError: (message: string) => void;
}) {
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [remoteStatus, setRemoteStatus] = useState("");
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
  const [historyRevision, setHistoryRevision] = useState(0);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const groupId = useId();
  const run = async (action: () => Promise<void>, progress = "") => {
    if (busy) return;
    setBusy(true);
    setRemoteStatus(progress);
    try {
      await action();
    } catch (error) {
      setRemoteStatus("");
      onError(errorMessage(error));
    } finally {
      setBusy(false);
      onRefresh();
      setHistoryRevision((value) => value + 1);
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
      onRefresh,
    },
  });
  if (!status)
    return (
      <div className="sidebar-panel">
        <header className="sidebar-heading">SOURCE CONTROL</header>
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
    <div className="sidebar-panel source-panel">
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
        <IconButton
          title="Refresh source control"
          disabled={busy}
          onClick={() => {
            onRefresh();
            setHistoryRevision((value) => value + 1);
          }}
        >
          <RefreshCw size={14} />
        </IconButton>
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
                setMessage("");
              });
            }}
          >
            <textarea
              aria-label="Commit message"
              placeholder="Enter commit message"
              value={message}
              onChange={(event) => setMessage(event.target.value)}
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
