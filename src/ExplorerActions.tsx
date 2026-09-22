import { useId, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent, MouseEvent } from "react";
import ResourceIcon from "./ResourceIcon";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api, errorMessage } from "./api";
import type { FileEntry, GitCommitSummary, GitStatus } from "./api";
import { parentPath, repositoryForPath, gitFilePath } from "./explorer-model";
import type { FileOperation } from "./explorer-model";
import ContextMenu from "./ContextMenu";
import { Modal } from "./ui";
import GitHistory from "./GitHistory";

let clipboard: { root: string; relative: string; cut: boolean } | undefined;

interface Props {
  root: string;
  repositories: GitStatus[];
  onTerminal: (path: string) => void;
  onSearch: (relative: string) => void;
  onOpenFile: (relative: string) => void;
  onOpenCommit: (commit: GitCommitSummary, root: string) => void;
  onOperation: (relative: string, operation: FileOperation) => Promise<boolean>;
  onRefresh: () => void;
  onExpand: (relative: string) => void;
  onError: (message: string) => void;
}

export function useExplorerActions(props: Props) {
  const [context, setContext] = useState<{
    entry: FileEntry;
    x: number;
    y: number;
    background: boolean;
  }>();
  const [prompt, setPrompt] = useState<{
    entry: FileEntry;
    kind: "rename" | "newFile" | "newFolder" | "trash" | "delete";
  }>();
  const [value, setValue] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [history, setHistory] = useState<FileEntry>();
  const errorId = useId();
  const trigger = useRef<HTMLElement>(null);
  const parent = (entry: FileEntry) =>
    entry.isDirectory ? entry.relativePath : parentPath(entry.relativePath);
  const dismiss = () => {
    setContext(undefined);
    if (trigger.current?.isConnected)
      (
        trigger.current.querySelector<HTMLElement>(".tree-entry") ??
        trigger.current
      ).focus({ preventScroll: true });
  };
  const open = (
    entry: FileEntry,
    element: HTMLElement,
    x?: number,
    y?: number,
    background = false,
  ) => {
    trigger.current = element;
    const bounds = element.getBoundingClientRect();
    setContext({
      entry,
      x: x || bounds.left,
      y: y || bounds.bottom,
      background,
    });
  };
  const onContext = (
    event: MouseEvent<HTMLElement>,
    entry: FileEntry,
    background = false,
  ) => {
    event.preventDefault();
    event.stopPropagation();
    if (!busy)
      open(
        entry,
        event.currentTarget,
        event.clientX,
        event.clientY,
        background,
      );
  };
  const ask = (entry: FileEntry, kind: NonNullable<typeof prompt>["kind"]) => {
    setError("");
    setValue(kind === "rename" ? entry.name : "");
    if (kind === "newFile" || kind === "newFolder")
      props.onExpand(parent(entry));
    setPrompt({ entry, kind });
  };
  const run = (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    void action()
      .catch((error) => props.onError(errorMessage(error)))
      .finally(() => setBusy(false));
  };
  const operate = async (relative: string, operation: FileOperation) => {
    if (await props.onOperation(relative, operation)) {
      props.onRefresh();
      return true;
    }
    return false;
  };
  const paste = (entry: FileEntry) => {
    if (!clipboard) return;
    const source = clipboard;
    run(async () => {
      if (
        (await operate(parent(entry), {
          kind: source.cut ? "move" : "copy",
          sourceRoot: source.root,
          source: source.relative,
        })) &&
        source.cut &&
        clipboard === source
      )
        clipboard = undefined;
    });
  };
  const copy = (entry: FileEntry, cut: boolean) => {
    if (entry.relativePath)
      clipboard = { root: props.root, relative: entry.relativePath, cut };
  };
  const reveal = (entry: FileEntry, reveal: boolean) =>
    run(() =>
      api("open_project_item", {
        root: props.root,
        relative: entry.relativePath,
        reveal,
      }),
    );
  const ignore = (entry: FileEntry, local: boolean) =>
    run(async () => {
      const repository = repositoryForPath(props.repositories, entry.path);
      if (!repository)
        throw new Error("This file is no longer in a Git repository.");
      await api("ignore_project_item", {
        root: repository.root,
        relative: gitFilePath(entry.path).slice(
          gitFilePath(repository.root).length + 1,
        ),
        local,
      });
      props.onRefresh();
    });
  const onKey = (event: KeyboardEvent<HTMLElement>, entry: FileEntry) => {
    if (busy || event.target instanceof HTMLInputElement) return;
    const mod = event.ctrlKey || event.metaKey;
    if (
      event.key === "ContextMenu" ||
      (event.shiftKey && event.key === "F10")
    ) {
      event.preventDefault();
      event.stopPropagation();
      open(entry, event.currentTarget);
      return;
    }
    let action: (() => void) | undefined;
    if (event.key === "F2" && !mod) action = () => ask(entry, "rename");
    if (event.key === "Delete")
      action = () => ask(entry, mod ? "delete" : "trash");
    if (mod && !event.altKey && !event.shiftKey) {
      if (event.key.toLowerCase() === "x") action = () => copy(entry, true);
      if (event.key.toLowerCase() === "c") action = () => copy(entry, false);
      if (event.key.toLowerCase() === "v") action = () => paste(entry);
    }
    if (action) {
      event.preventDefault();
      event.stopPropagation();
      trigger.current = event.currentTarget;
      action();
    }
  };
  const entry = context?.entry;
  const entryRepository =
    entry && repositoryForPath(props.repositories, entry.path);
  const canIgnore =
    entry &&
    entryRepository &&
    gitFilePath(entryRepository.root) !== gitFilePath(entry.path);
  const menu = context && entry && (
    <ContextMenu
      {...context}
      label={`${entry.name} actions`}
      onClose={dismiss}
      actions={[
        { label: "New File", run: () => ask(entry, "newFile") },
        { label: "New Folder", run: () => ask(entry, "newFolder") },
        null,
        ...(entry.isDirectory
          ? [
              {
                label: "Search in Folder…",
                run: () => props.onSearch(entry.relativePath),
              },
            ]
          : []),
        { label: "Reveal in File Manager", run: () => reveal(entry, true) },
        { label: "Open in Default App", run: () => reveal(entry, false) },
        {
          label: "Open in Terminal",
          run: () =>
            props.onTerminal(
              entry.isDirectory ? entry.path : parentPath(entry.path),
            ),
        },
        null,
        {
          label: "Cut",
          shortcut: "Ctrl+X",
          disabled: !entry.relativePath,
          run: () => copy(entry, true),
        },
        {
          label: "Copy",
          shortcut: "Ctrl+C",
          disabled: !entry.relativePath,
          run: () => copy(entry, false),
        },
        {
          label: "Duplicate",
          disabled: !entry.relativePath,
          run: () =>
            run(() => operate(entry.relativePath, { kind: "duplicate" })),
        },
        {
          label: "Paste",
          shortcut: "Ctrl+V",
          disabled: !clipboard,
          run: () => paste(entry),
        },
        null,
        { label: "Copy Path", run: () => run(() => writeText(entry.path)) },
        {
          label: "Copy Relative Path",
          run: () => run(() => writeText(entry.relativePath || ".")),
        },
        null,
        {
          label: "Add to .gitignore",
          disabled: !canIgnore,
          run: () => ignore(entry, false),
        },
        {
          label: "Add to .git/info/exclude",
          disabled: !canIgnore,
          run: () => ignore(entry, true),
        },
        {
          label: "View History",
          disabled: !entryRepository,
          run: () => setHistory(entry),
        },
        ...(context.background
          ? []
          : [
              null,
              {
                label: "Rename…",
                shortcut: "F2",
                run: () => ask(entry, "rename"),
              },
              {
                label: "Move to Trash…",
                shortcut: "Delete",
                run: () => ask(entry, "trash"),
              },
              {
                label: "Delete Permanently…",
                shortcut: "Ctrl+Delete",
                danger: true,
                run: () => ask(entry, "delete"),
              },
            ]),
      ]}
    />
  );
  const creating = prompt?.kind === "newFile" || prompt?.kind === "newFolder";
  const deleting = prompt?.kind === "delete" || prompt?.kind === "trash";
  const title =
    prompt?.kind === "rename"
      ? "Rename"
      : prompt?.kind === "newFile"
        ? "New File"
        : prompt?.kind === "newFolder"
          ? "New Folder"
          : prompt?.kind === "trash"
            ? "Move to Trash"
            : "Delete Permanently";
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (busy || !prompt) return;
    if (
      !deleting &&
      (!value.trim() ||
        (prompt.kind === "rename" && value === prompt.entry.name))
    ) {
      setPrompt(undefined);
      requestAnimationFrame(dismiss);
      return;
    }
    setBusy(true);
    setError("");
    const { entry, kind } = prompt;
    const relative = creating ? parent(entry) : entry.relativePath;
    void operate(
      relative,
      kind === "trash" || kind === "delete" ? { kind } : { kind, name: value },
    )
      .then((completed) => {
        if (completed) {
          setPrompt(undefined);
          if (kind === "newFile")
            props.onOpenFile([relative, value].filter(Boolean).join("/"));
        }
      })
      .catch((error) => setError(errorMessage(error)))
      .finally(() => setBusy(false));
  };
  const naming =
    prompt && !deleting
      ? {
          relative: creating ? parent(prompt.entry) : prompt.entry.relativePath,
          node: (
            <form
              className="tree-create"
              onSubmit={submit}
              onContextMenu={(event) => event.stopPropagation()}
            >
              <div className="tree-row">
                <div className="tree-entry">
                  <span className="tree-indent" />
                  <ResourceIcon
                    path={
                      creating
                        ? `${prompt.entry.path}/${value}`
                        : prompt.entry.path
                    }
                    folder={
                      creating
                        ? prompt.kind === "newFolder"
                        : prompt.entry.isDirectory
                    }
                    size={14}
                  />
                  <input
                    aria-label={`${title} name`}
                    aria-invalid={!!error}
                    aria-describedby={error ? errorId : undefined}
                    title={`Enter to ${creating ? "create" : "rename"}, Escape to cancel`}
                    autoFocus
                    autoComplete="off"
                    spellCheck={false}
                    value={value}
                    readOnly={busy}
                    onChange={(event) => setValue(event.target.value)}
                    onFocus={(event) => event.target.select()}
                    onBlur={() => {
                      if (!busy) setPrompt(undefined);
                    }}
                    onKeyDown={(event) => {
                      if (
                        event.nativeEvent.isComposing ||
                        event.nativeEvent.keyCode === 229
                      ) {
                        if (event.key === "Enter") event.preventDefault();
                        return;
                      }
                      if (event.key === "Escape") {
                        event.preventDefault();
                        event.stopPropagation();
                        if (!busy) {
                          setPrompt(undefined);
                          requestAnimationFrame(dismiss);
                        }
                      }
                    }}
                  />
                </div>
              </div>
              {error && (
                <div
                  id={errorId}
                  className="tree-message text-error"
                  role="alert"
                >
                  {error}
                </div>
              )}
            </form>
          ),
        }
      : undefined;
  const creation = creating ? naming : undefined;
  const rename = prompt?.kind === "rename" ? naming : undefined;
  const dialog = prompt && deleting && (
    <Modal
      tone="danger"
      title={title}
      onClose={() => {
        if (!busy) setPrompt(undefined);
      }}
    >
      <form className="dialog-form" onSubmit={submit}>
        <div className="dialog-body">
          <p>
            {prompt.kind === "trash" ? "Move" : "Permanently delete"}{" "}
            <strong>{prompt.entry.name}</strong>
            {prompt.entry.isDirectory ? " and all its contents" : ""}?{" "}
            {prompt.kind === "delete" && "This cannot be undone."}
            {!prompt.entry.relativePath &&
              " This also closes the project and its terminals."}
          </p>
          {error && (
            <p className="text-error" role="alert">
              {error}
            </p>
          )}
        </div>
        <div className="dialog-actions">
          <button
            type="button"
            className="button"
            disabled={busy}
            onClick={() => setPrompt(undefined)}
          >
            Cancel
          </button>
          <button type="submit" className="button text-error" disabled={busy}>
            {busy ? "Working…" : title}
          </button>
        </div>
      </form>
    </Modal>
  );
  const historyRoot =
    history && repositoryForPath(props.repositories, history.path)?.root;
  const historyDialog = history && historyRoot && (
    <Modal
      title={`Git History · ${history.name}`}
      wide
      className="file-history-dialog"
      onClose={() => setHistory(undefined)}
    >
      <GitHistory
        key={history.path}
        root={historyRoot}
        path={gitFilePath(history.path)
          .slice(gitFilePath(historyRoot).length)
          .replace(/^\//, "")}
        onOpenCommit={(commit) => {
          setHistory(undefined);
          props.onOpenCommit(commit, historyRoot);
        }}
      />
    </Modal>
  );
  return {
    onContext,
    onKey,
    menu,
    dialog,
    historyDialog,
    busy,
    creation,
    rename,
  };
}
