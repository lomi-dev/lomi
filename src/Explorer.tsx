import ResourceIcon from "./ResourceIcon";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Scissors } from "lucide-react";
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
import { api, errorMessage, windows } from "./api";
import type { FileEntry, GitCommitSummary, GitStatus } from "./api";
import { basename } from "./model";
import { beginFileDrag } from "./file-drag";
import { IconButton } from "./ui";

import ProjectSearch from "./ProjectSearch";
import type { SearchMatch } from "./ProjectSearch";
import type { FileOperation } from "./explorer-model";
import { useExplorerDrop } from "./explorer-drop";
import { explorerGitStatuses, gitFilePath, parentPath } from "./explorer-model";
import { useExplorerActions, type ExplorerClipboard } from "./ExplorerActions";

const isCutItem = (
  clipboard: ExplorerClipboard | undefined,
  root: string,
  relative: string,
) =>
  !!clipboard?.cut &&
  clipboard.root === root &&
  clipboard.relative === relative;

interface Props {
  root: string;
  explorerOpenOnSingleClick: boolean;
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
  const [selection, setSelection] = useState<
    { root: string; relativePath: string } | undefined
  >();
  const selectedPath =
    selection?.root === props.root ? selection.relativePath : undefined;
  const select = (relativePath: string | undefined) =>
    setSelection(
      relativePath === undefined
        ? undefined
        : { root: props.root, relativePath },
    );
  useEffect(() => {
    setSelection(undefined);
  }, [props.root]);
  const pendingActivation = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const pendingPointerDown = useRef<(() => void) | undefined>(undefined);
  const pointerFocusPaths = useRef(new Set<string>());
  const activationContext = useRef({
    root: props.root,
    enabled: props.explorerOpenOnSingleClick,
  });
  activationContext.current = {
    root: props.root,
    enabled: props.explorerOpenOnSingleClick,
  };
  const removePendingPointerDown = useCallback(() => {
    if (pendingPointerDown.current) {
      document.removeEventListener(
        "pointerdown",
        pendingPointerDown.current,
        true,
      );
      pendingPointerDown.current = undefined;
    }
  }, []);
  const cancelPendingActivation = useCallback(() => {
    if (pendingActivation.current !== undefined)
      clearTimeout(pendingActivation.current);
    pendingActivation.current = undefined;
    removePendingPointerDown();
  }, [removePendingPointerDown]);
  const scheduleActivation = useCallback(
    (activate: () => void) => {
      cancelPendingActivation();
      const root = props.root;
      const cancelOnPointerDown = () => cancelPendingActivation();
      pendingPointerDown.current = cancelOnPointerDown;
      document.addEventListener("pointerdown", cancelOnPointerDown, true);
      pendingActivation.current = setTimeout(() => {
        pendingActivation.current = undefined;
        removePendingPointerDown();
        if (
          activationContext.current.root === root &&
          activationContext.current.enabled
        )
          activate();
      }, 500);
    },
    [cancelPendingActivation, props.root, removePendingPointerDown],
  );
  useEffect(() => {
    cancelPendingActivation();
    pointerFocusPaths.current.clear();
    return () => {
      cancelPendingActivation();
      pointerFocusPaths.current.clear();
    };
  }, [
    props.root,
    props.explorerOpenOnSingleClick,
    searchOpen,
    cancelPendingActivation,
  ]);
  useEffect(() => {
    const clearPointerFocus = () => pointerFocusPaths.current.clear();
    document.addEventListener("pointerup", clearPointerFocus, true);
    document.addEventListener("pointercancel", clearPointerFocus, true);
    return () => {
      document.removeEventListener("pointerup", clearPointerFocus, true);
      document.removeEventListener("pointercancel", clearPointerFocus, true);
    };
  }, []);
  const actions = useExplorerActions({
    ...props,
    onSearch: search,
    onRefresh: refresh,
    onExpand: (relative) =>
      setExpanded((previous) => new Set(previous).add(relative)),
  });
  const onContext: typeof actions.onContext = (event, entry, background) => {
    cancelPendingActivation();
    actions.onContext(event, entry, background);
  };
  const onClipboard: typeof actions.onClipboard = (event, entry) => {
    cancelPendingActivation();
    actions.onClipboard(event, entry);
  };
  const rootEntry: FileEntry = {
    name: basename(props.root),
    relativePath: "",
    path: props.root,
    isDirectory: true,
    isSymlink: false,
  };
  const [showHidden, setShowHidden] = useState(true);
  const [expanded, setExpanded] = useState(new Set<string>());
  const drop = useExplorerDrop({
    ...props,
    hidden: searchOpen,
    busy: actions.busy,
    onExpand: (relative) =>
      setExpanded((previous) => new Set(previous).add(relative)),
    onRefresh: refresh,
  });
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
        <div
          className="sidebar-panel explorer-panel"
          ref={drop.panel}
          data-explorer-root={props.root}
          aria-busy={actions.busy || drop.importing}
          onPointerDown={cancelPendingActivation}
          onKeyDownCapture={cancelPendingActivation}
        >
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
              data-explorer-directory=""
              data-git-status={gitStatuses.get(gitFilePath(props.root))}
              data-selected={selectedPath === "" ? "true" : undefined}
              onContextMenu={(event) => {
                select("");
                onContext(event, rootEntry);
              }}
              onKeyDown={(event) => {
                cancelPendingActivation();
                actions.onKey(event, rootEntry);
              }}
            >
              <button
                type="button"
                className="tree-entry project-tree-entry"
                aria-label={`Project folder ${rootEntry.name}`}
                aria-pressed={selectedPath === ""}
                onFocus={() => select("")}
                onClick={(event) => {
                  if (event.detail !== 0)
                    event.currentTarget.focus({ preventScroll: true });
                  select("");
                }}
                onCopy={(event) => onClipboard(event, rootEntry)}
                onCut={(event) => onClipboard(event, rootEntry)}
                onPaste={(event) => onClipboard(event, rootEntry)}
              >
                <ResourceIcon
                  path={props.root}
                  folder
                  root
                  expanded
                  size={14}
                />
                <span title={props.root}>{basename(props.root)}</span>
              </button>
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
            data-explorer-directory=""
            aria-label="Project files"
            onContextMenu={(event) => onContext(event, rootEntry, true)}
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
              selectedPath={selectedPath}
              clipboard={actions.clipboard}
              onSelect={select}
              pointerFocusPaths={pointerFocusPaths}
              explorerOpenOnSingleClick={props.explorerOpenOnSingleClick}
              scheduleActivation={scheduleActivation}
              cancelPendingActivation={cancelPendingActivation}
              onContext={onContext}
              onKey={actions.onKey}
              onClipboard={onClipboard}
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
  onClipboard: ReturnType<typeof useExplorerActions>["onClipboard"];
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
  selectedPath: string | undefined;
  clipboard: ExplorerClipboard | undefined;
  onSelect: (relativePath: string | undefined) => void;
  pointerFocusPaths: { current: Set<string> };
  explorerOpenOnSingleClick: boolean;
  scheduleActivation: (activate: () => void) => void;
  cancelPendingActivation: () => void;
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
    selectedPath,
    onSelect,
    pointerFocusPaths,
    explorerOpenOnSingleClick,
    scheduleActivation,
    cancelPendingActivation,
    onTerminal,
    onOpenFile,
    onError,
  } = props;
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const suppressedFocusPath = useRef<string | undefined>(undefined);
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
          data-explorer-directory={relative}
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
                data-explorer-directory={
                  entry.isDirectory
                    ? entry.relativePath
                    : windows
                      ? parentPath(entry.relativePath)
                      : entry.relativePath.split("/").slice(0, -1).join("/")
                }
                data-explorer-folder={
                  entry.isDirectory ? entry.relativePath : undefined
                }
                data-cut={
                  isCutItem(props.clipboard, root, entry.relativePath)
                    ? "true"
                    : undefined
                }
                data-selected={
                  selectedPath === entry.relativePath ? "true" : undefined
                }
                onContextMenu={(event) => {
                  onSelect(entry.relativePath);
                  props.onContext(event, entry);
                }}
                onKeyDown={(event) => {
                  cancelPendingActivation();
                  props.onKey(event, entry);
                }}
                onCopy={(event) => props.onClipboard(event, entry)}
                onCut={(event) => props.onClipboard(event, entry)}
                onPaste={(event) => props.onClipboard(event, entry)}
                style={{
                  paddingLeft: `calc(${depth} * var(--tree-indent) + var(--space-10))`,
                }}
                onPointerDown={(event) => {
                  cancelPendingActivation();
                  if (!(event.target as HTMLElement).closest(".tree-toggle"))
                    beginFileDrag(event, entry.path, onError);
                }}
              >
                {entry.isDirectory ? (
                  <button
                    type="button"
                    className="tree-toggle"
                    title={`${open ? "Collapse" : "Expand"} ${entry.name}`}
                    aria-label={`${open ? "Collapse" : "Expand"} ${entry.name}`}
                    aria-expanded={open}
                    onFocus={() => onSelect(entry.relativePath)}
                    onClick={(event) => {
                      if (event.detail !== 0)
                        event.currentTarget.focus({ preventScroll: true });
                      onSelect(entry.relativePath);
                      toggle(entry.relativePath);
                    }}
                  >
                    {open ? (
                      <ChevronDown size={12} />
                    ) : (
                      <ChevronRight size={12} />
                    )}
                  </button>
                ) : (
                  <span className="tree-toggle-space" aria-hidden="true" />
                )}
                <button
                  type="button"
                  className="tree-entry"
                  data-git-status={props.gitStatuses.get(
                    gitFilePath(entry.path),
                  )}
                  title={entry.path}
                  aria-expanded={entry.isDirectory ? open : undefined}
                  aria-pressed={selectedPath === entry.relativePath}
                  aria-description={
                    isCutItem(props.clipboard, root, entry.relativePath)
                      ? "Cut item"
                      : undefined
                  }
                  onClick={(event) => {
                    const activate = () =>
                      entry.isDirectory
                        ? toggle(entry.relativePath)
                        : onOpenFile(entry.relativePath);
                    if (!explorerOpenOnSingleClick) {
                      if (event.detail !== 0)
                        event.currentTarget.focus({ preventScroll: true });
                      onSelect(entry.relativePath);
                      if (event.detail === 0) activate();
                      return;
                    }

                    pointerFocusPaths.current.delete(entry.relativePath);
                    const button = event.currentTarget;
                    suppressedFocusPath.current = entry.relativePath;
                    button.focus({ preventScroll: true });
                    suppressedFocusPath.current = undefined;
                    if (event.detail === 0) {
                      cancelPendingActivation();
                      onSelect(entry.relativePath);
                      activate();
                    } else if (event.detail > 1) {
                      cancelPendingActivation();
                      onSelect(entry.relativePath);
                    } else {
                      onSelect(undefined);
                      scheduleActivation(() => {
                        if (button.isConnected) activate();
                      });
                    }
                  }}
                  onFocus={() => {
                    if (
                      explorerOpenOnSingleClick &&
                      (pointerFocusPaths.current.has(entry.relativePath) ||
                        suppressedFocusPath.current === entry.relativePath)
                    )
                      return;
                    onSelect(entry.relativePath);
                  }}
                  onPointerDown={(event) => {
                    if (event.button === 0)
                      pointerFocusPaths.current.add(entry.relativePath);
                  }}
                  onDoubleClick={(event) => {
                    cancelPendingActivation();
                    if (explorerOpenOnSingleClick) {
                      suppressedFocusPath.current = entry.relativePath;
                      event.currentTarget.focus({ preventScroll: true });
                      suppressedFocusPath.current = undefined;
                      onSelect(entry.relativePath);
                    } else {
                      entry.isDirectory
                        ? toggle(entry.relativePath)
                        : onOpenFile(entry.relativePath);
                    }
                  }}
                >
                  {isCutItem(props.clipboard, root, entry.relativePath) && (
                    <Scissors
                      className="tree-cut-mark"
                      size={12}
                      aria-hidden="true"
                    />
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
                  type="button"
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
                    type="button"
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
