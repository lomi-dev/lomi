import { useRef, useState } from "react";
import type { KeyboardEvent, MouseEvent } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api, errorMessage } from "./api";
import type { GitChange, GitCommitSummary } from "./api";
import ContextMenu from "./ContextMenu";
import GitHistory from "./GitHistory";
import { Modal } from "./ui";

interface GitFile {
  path: string;
  change?: GitChange;
  staged?: boolean;
  deleted?: boolean;
}

export function useGitFileActions(props: {
  root: string;
  onDiff: (path: string, staged: boolean) => void;
  onOpenFile: (path: string) => void;
  onOpenCommit: (commit: GitCommitSummary) => void;
  onError: (message: string) => void;
  working?: {
    busy: boolean;
    onStage: (change: GitChange, staged: boolean) => void;
    onDiscard: (change: GitChange) => Promise<void>;
    onRun: (action: () => Promise<void>) => Promise<void>;
    onRefresh: () => void;
  };
}) {
  const [context, setContext] = useState<{
    file: GitFile;
    x: number;
    y: number;
  }>();
  const [history, setHistory] = useState<string>();
  const [discard, setDiscard] = useState<GitChange>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const trigger = useRef<HTMLElement>(null);
  const disabled = busy || props.working?.busy;
  const dismiss = () => {
    setContext(undefined);
    if (trigger.current?.isConnected)
      trigger.current.focus({ preventScroll: true });
  };
  const open = (
    event: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>,
    file: GitFile,
  ) => {
    event.preventDefault();
    event.stopPropagation();
    if (disabled) return;
    trigger.current =
      event.currentTarget.querySelector<HTMLElement>(".git-file-open") ??
      event.currentTarget;
    const bounds = trigger.current.getBoundingClientRect();
    setContext({
      file,
      x: "clientX" in event && event.clientX ? event.clientX : bounds.left,
      y: "clientY" in event && event.clientY ? event.clientY : bounds.bottom,
    });
  };
  const onKey = (event: KeyboardEvent<HTMLElement>, file: GitFile) => {
    if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))
      open(event, file);
  };
  const run = (action: () => Promise<unknown>) => {
    if (disabled) return;
    setBusy(true);
    void action()
      .catch((error) => props.onError(errorMessage(error)))
      .finally(() => setBusy(false));
  };
  const ignore = (local: boolean) =>
    run(async () => {
      const action = () =>
        api<void>("ignore_project_item", {
          root: props.root,
          relative: context!.file.path,
          local,
        });
      if (props.working) await props.working.onRun(action);
      else await action();
      props.working?.onRefresh();
    });
  const file = context?.file;
  const change = file?.change;
  const menu = context && file && (
    <ContextMenu
      x={context.x}
      y={context.y}
      label={`Source control actions for ${file.path}`}
      onClose={dismiss}
      actions={[
        ...(change && props.working
          ? [
              {
                label: file.staged ? "Unstage File" : "Stage File",
                disabled,
                run: () => props.working!.onStage(change, !file.staged),
              },
              ...(!file.staged
                ? [
                    {
                      label: "Discard Changes…",
                      danger: true,
                      disabled,
                      run: () => {
                        setError("");
                        setDiscard(change);
                      },
                    },
                  ]
                : []),
              null,
              {
                label: "Unstaged Changes",
                disabled: [" ", "!"].includes(change.worktree),
                run: () => props.onDiff(file.path, false),
              },
              {
                label: "Staged Changes",
                disabled: [" ", "?", "!"].includes(change.index),
                run: () => props.onDiff(file.path, true),
              },
              null,
            ]
          : []),
        {
          label: "Copy Path",
          run: () =>
            run(() =>
              writeText(`${props.root.replace(/[\\/]$/, "")}/${file.path}`),
            ),
        },
        {
          label: "Copy Relative Path",
          run: () => run(() => writeText(file.path)),
        },
        ...(change
          ? [
              null,
              {
                label: "Add to .gitignore",
                disabled: disabled || change.index !== "?",
                run: () => ignore(false),
              },
              {
                label: "Add to .git/info/exclude",
                disabled: disabled || change.index !== "?",
                run: () => ignore(true),
              },
            ]
          : []),
        null,
        {
          label: "Open Diff",
          run: () => props.onDiff(file.path, !!file.staged),
        },
        {
          label: change ? "View File" : "View Working File",
          disabled: file.deleted || file.path.endsWith("/"),
          run: () => props.onOpenFile(file.path),
        },
        null,
        { label: "View File History", run: () => setHistory(file.path) },
      ]}
    />
  );
  const dialogs = (
    <>
      {history !== undefined && (
        <Modal
          title={`Git History · ${history}`}
          wide
          className="file-history-dialog"
          onClose={() => setHistory(undefined)}
        >
          <GitHistory
            root={props.root}
            path={history}
            onOpenCommit={(commit) => {
              setHistory(undefined);
              props.onOpenCommit(commit);
            }}
          />
        </Modal>
      )}
      {discard && (
        <Modal
          protectTheme
          title="Discard Changes"
          tone="danger"
          onClose={() => {
            if (!busy) setDiscard(undefined);
          }}
        >
          <form
            className="dialog-form"
            onSubmit={(event) => {
              event.preventDefault();
              if (disabled || !props.working) return;
              setBusy(true);
              setError("");
              void props.working
                .onRun(() => props.working!.onDiscard(discard))
                .then(() => {
                  setDiscard(undefined);
                  props.working?.onRefresh();
                })
                .catch((error) => setError(errorMessage(error)))
                .finally(() => setBusy(false));
            }}
          >
            <div className="dialog-body">
              <p>
                {discard.index === "?" ? (
                  <>
                    Move <strong>{discard.path}</strong> to the trash?
                  </>
                ) : (
                  <>
                    Discard unstaged changes in <strong>{discard.path}</strong>?
                    The file will be restored to its staged version. This cannot
                    be undone.
                  </>
                )}
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
                onClick={() => setDiscard(undefined)}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="button text-error"
                disabled={disabled}
              >
                {busy
                  ? "Working…"
                  : discard.index === "?"
                    ? "Move to Trash"
                    : "Discard Changes"}
              </button>
            </div>
          </form>
        </Modal>
      )}
    </>
  );
  return { onContext: open, onKey, menu, dialogs };
}
