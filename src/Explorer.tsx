import ResourceIcon from "./ResourceIcon";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Channel, Resource } from "@tauri-apps/api/core";
import {
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  Copy,
  Eye,
  EyeOff,
  RefreshCw,
  Search,
  Terminal,
} from "./icons";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api, errorMessage } from "./api";
import type { FileEntry, GitCommitSummary, GitStatus } from "./api";
import { basename } from "./model";
import { beginFileDrag } from "./file-drag";
import { IconButton } from "./ui";

import ProjectSearch from "./ProjectSearch";
import type { SearchMatch } from "./ProjectSearch";
import type { FileOperation } from "./explorer-model";
import { explorerGitStatuses, gitFilePath } from "./explorer-model";
import { useExplorerActions } from "./ExplorerActions";

interface Props {
  root: string;
  onTerminal: (path: string) => void;
  onOpenFile: (relative: string, match?: SearchMatch) => void;
  repositories: GitStatus[];
  onRefreshGit: () => void;
  onOpenCommit: (commit: GitCommitSummary, root: string) => void;
  onOperation: (relative: string, operation: FileOperation) => Promise<boolean>;
  onError: (message: string) => void;
}

export default function Explorer(props: Props) {
  const gitStatuses = useMemo(
    () => explorerGitStatuses(props.repositories, props.root),
    [props.repositories, props.root],
  );
  const gitRevision = JSON.stringify(props.repositories);
  const [revision, setRevision] = useState(0);
  const [watched, setWatched] = useState(new Set<string>());
  const [directoryRevisions, setDirectoryRevisions] = useState<
    Record<string, number>
  >({});
  const watchDirectory = useCallback((relative: string, watch: boolean) => {
    setWatched((previous) => {
      const next = new Set(previous);
      if (watch) next.add(relative);
      else next.delete(relative);
      return next;
    });
  }, []);
  const watchPaths = JSON.stringify([...watched].sort());
  useEffect(() => {
    const relatives: string[] = JSON.parse(watchPaths);
    if (!relatives.length) return;
    let current = true;
    const changed = (directories: string[]) => {
      const dirty = new Set(directories);
      if (current)
        setDirectoryRevisions((previous) =>
          Object.fromEntries(
            relatives.map((relative) => [
              relative,
              (previous[relative] ?? 0) + (dirty.has(relative) ? 1 : 0),
            ]),
          ),
        );
    };
    const watcher = api<number>("watch_explorer_directories", {
      root: props.root,
      relatives,
      onChange: new Channel<string[]>(changed),
    }).then((rid) => new Resource(rid));
    void watcher
      // Catch changes between listing directories and registering the watch.
      .then(() => changed(relatives))
      .catch((error) => {
        if (current)
          props.onError(`Explorer auto-refresh failed: ${errorMessage(error)}`);
      });
    return () => {
      current = false;
      void watcher.then((watcher) => watcher.close()).catch(() => {});
    };
  }, [props.root, props.onError, watchPaths, revision]);
  const [searchScope, setSearchScope] = useState<string>();
  const [searchOpen, setSearchOpen] = useState(false);
  const search = (relative: string) => {
    setSearchScope(relative);
    setSearchOpen(true);
  };
  const refresh = () => {
    setRevision((revision) => revision + 1);
    props.onRefreshGit();
  };
  const actions = useExplorerActions({
    ...props,
    onSearch: search,
    onRefresh: refresh,
    onExpand: (relative) =>
      setExpanded((previous) => new Set(previous).add(relative)),
  });
  const rootEntry: FileEntry = {
    name: basename(props.root),
    relativePath: "",
    path: props.root,
    isDirectory: true,
    isSymlink: false,
  };
  const [showHidden, setShowHidden] = useState(true);
  const [expanded, setExpanded] = useState(new Set<string>());
  const toggle = (path: string) =>
    setExpanded((previous) => {
      const next = new Set(previous);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  return (
    <>
      {searchScope !== undefined && (
        <ProjectSearch
          root={props.root}
          relative={searchScope}
          hidden={!searchOpen}
          onClose={() => setSearchOpen(false)}
          onScope={setSearchScope}
          onOpenFile={props.onOpenFile}
        />
      )}
      {!searchOpen && (
        <div className="sidebar-panel explorer-panel" aria-busy={actions.busy}>
          <header className="sidebar-heading">
            <span>EXPLORER</span>
            <div>
              <IconButton title="Search in project" onClick={() => search("")}>
                <Search size={14} />
              </IconButton>
              <IconButton
                title={showHidden ? "Hide dotfiles" : "Show dotfiles"}
                onClick={() => setShowHidden(!showHidden)}
              >
                {showHidden ? <Eye size={14} /> : <EyeOff size={14} />}
              </IconButton>
              <IconButton
                title="Collapse folders"
                onClick={() => setExpanded(new Set())}
              >
                <ChevronsDownUp size={14} />
              </IconButton>
              <IconButton title="Refresh explorer" onClick={refresh}>
                <RefreshCw size={14} />
              </IconButton>
            </div>
          </header>
          {actions.rename?.relative === "" ? (
            <div style={{ padding: "var(--tree-heading-padding)" }}>
              {actions.rename.node}
            </div>
          ) : (
            <div
              className="project-tree-heading"
              data-git-status={gitStatuses.get(gitFilePath(props.root))}
              tabIndex={0}
              role="button"
              aria-label={`Project folder ${rootEntry.name}`}
              onContextMenu={(event) => actions.onContext(event, rootEntry)}
              onKeyDown={(event) => actions.onKey(event, rootEntry)}
            >
              <ResourceIcon path={props.root} folder root expanded size={14} />
              <span title={props.root}>{basename(props.root)}</span>
              <IconButton
                title="Copy project folder path"
                onClick={() =>
                  void writeText(props.root).catch((error) =>
                    props.onError(errorMessage(error)),
                  )
                }
              >
                <Copy size={14} />
              </IconButton>
              <IconButton
                title="Open terminal in project folder"
                onClick={() => props.onTerminal(props.root)}
              >
                <Terminal size={14} />
              </IconButton>
            </div>
          )}
          <div
            className="file-tree"
            aria-label="Project files"
            onContextMenu={(event) => actions.onContext(event, rootEntry, true)}
          >
            <Directory
              {...props}
              relative=""
              depth={0}
              revision={revision}
              directoryRevisions={directoryRevisions}
              watchDirectory={watchDirectory}
              gitStatuses={gitStatuses}
              gitRevision={gitRevision}
              showHidden={showHidden}
              expanded={expanded}
              toggle={toggle}
              onContext={actions.onContext}
              onKey={actions.onKey}
              creation={actions.creation}
              rename={actions.rename}
            />
          </div>
          {actions.menu}
          {actions.dialog}
          {actions.historyDialog}
        </div>
      )}
    </>
  );
}

interface DirectoryProps extends Props {
  gitStatuses: Map<string, string>;
  gitRevision: string | undefined;
  onContext: ReturnType<typeof useExplorerActions>["onContext"];
  onKey: ReturnType<typeof useExplorerActions>["onKey"];
  creation: ReturnType<typeof useExplorerActions>["creation"];
  rename: ReturnType<typeof useExplorerActions>["rename"];
  relative: string;
  depth: number;
  revision: number;
  directoryRevisions: Record<string, number>;
  watchDirectory: (relative: string, watch: boolean) => void;
  showHidden: boolean;
  expanded: Set<string>;
  toggle: (path: string) => void;
}
function Directory(props: DirectoryProps) {
  const {
    root,
    relative,
    depth,
    revision,
    gitRevision,
    watchDirectory,
    showHidden,
    expanded,
    toggle,
    onTerminal,
    onOpenFile,
    onError,
  } = props;
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const reload = useRef(() => {});
  useEffect(() => {
    let current = true;
    let busy = false;
    let dirty = false;
    const update = async () => {
      busy = true;
      do {
        dirty = false;
        setLoading(true);
        setError("");
        try {
          const entries = await api<FileEntry[]>("list_directory", {
            root,
            relative,
          });
          if (current) setEntries(entries);
        } catch (error) {
          if (current) setError(errorMessage(error));
        } finally {
          if (current) setLoading(false);
        }
      } while (current && dirty);
      busy = false;
    };
    reload.current = () => {
      dirty = true;
      if (!busy) void update();
    };
    watchDirectory(relative, true);
    return () => {
      current = false;
      watchDirectory(relative, false);
    };
  }, [root, relative, watchDirectory]);
  const directoryRevision = props.directoryRevisions[relative];
  useEffect(() => {
    reload.current();
  }, [root, relative, revision, gitRevision, directoryRevision]);
  const creation =
    props.creation?.relative === relative ? props.creation : undefined;
  const visible = error
    ? []
    : entries.filter((entry) => showHidden || !entry.name.startsWith("."));
  const message =
    error ||
    (loading && !entries.length
      ? "Loading…"
      : !visible.length && !creation
        ? "Empty folder"
        : "");
  return (
    <>
      {creation && (
        <div
          style={{
            paddingLeft: `calc(${depth} * var(--tree-indent) + var(--space-10))`,
          }}
        >
          {creation.node}
        </div>
      )}
      {message && (
        <div
          className={`tree-message${error ? " text-error" : ""}`}
          title={error || undefined}
          style={{
            paddingLeft: `calc(${depth} * var(--tree-indent) + var(--space-16))`,
          }}
        >
          {message}
        </div>
      )}
      {visible.map((entry) => {
        const open = expanded.has(entry.relativePath);
        return (
          <div key={entry.relativePath}>
            {props.rename?.relative === entry.relativePath ? (
              <div
                style={{
                  paddingLeft: `calc(${depth} * var(--tree-indent) + var(--space-10))`,
                }}
              >
                {props.rename.node}
              </div>
            ) : (
              <div
                className="tree-row"
                onContextMenu={(event) => props.onContext(event, entry)}
                onKeyDown={(event) => props.onKey(event, entry)}
                style={{
                  paddingLeft: `calc(${depth} * var(--tree-indent) + var(--space-10))`,
                }}
                onPointerDown={(event) =>
                  beginFileDrag(event, entry.path, onError)
                }
              >
                <button
                  className="tree-entry"
                  data-git-status={props.gitStatuses.get(
                    gitFilePath(entry.path),
                  )}
                  title={entry.path}
                  aria-expanded={entry.isDirectory ? open : undefined}
                  onClick={() =>
                    entry.isDirectory
                      ? toggle(entry.relativePath)
                      : onOpenFile(entry.relativePath)
                  }
                >
                  {entry.isDirectory ? (
                    open ? (
                      <ChevronDown size={12} />
                    ) : (
                      <ChevronRight size={12} />
                    )
                  ) : (
                    <span className="tree-indent" />
                  )}
                  <ResourceIcon
                    path={entry.path}
                    folder={entry.isDirectory}
                    expanded={open}
                    size={14}
                  />
                  <span className={entry.name.startsWith(".") ? "dotfile" : ""}>
                    {entry.name}
                  </span>
                  {entry.isSymlink && <span className="symlink-mark">↗</span>}
                </button>
                <button
                  className="tree-action"
                  title={
                    entry.isDirectory ? "Copy folder path" : "Copy file path"
                  }
                  aria-label={`Copy path of ${entry.name}`}
                  onClick={() =>
                    void writeText(entry.path).catch((error) =>
                      onError(errorMessage(error)),
                    )
                  }
                >
                  <Copy size={12} />
                </button>
                {entry.isDirectory && (
                  <button
                    className="tree-action"
                    title="Open terminal here"
                    aria-label={`Open terminal in ${entry.name}`}
                    onClick={() => onTerminal(entry.path)}
                  >
                    <Terminal size={13} />
                  </button>
                )}
              </div>
            )}
            {entry.isDirectory && open && (
              <Directory
                {...props}
                relative={entry.relativePath}
                depth={depth + 1}
              />
            )}
          </div>
        );
      })}
    </>
  );
}
