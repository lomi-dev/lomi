import { useRef, useState } from "react";
import type { KeyboardEvent, MouseEvent } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api, errorMessage } from "./api";
import type { GitChange, GitCommitSummary } from "./api";
import ContextMenu from "./ContextMenu";
import GitHistory from "./GitHistory";
import { Modal } from "./ui";
import { useT } from "./i18n";

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
  const { t } = useT();
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
      const action = async () => {
        const result = await api<{ warning?: string | null }>(
          "ignore_project_item",
          {
            root: props.root,
            relative: context!.file.path,
            local,
          },
        );
        if (result.warning) props.onError(result.warning);
      };
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
      label={t("Source control actions for {path}", { path: file.path })}
      onClose={dismiss}
      actions={[
        ...(change && props.working
          ? [
              {
                label: file.staged ? t("Unstage file") : t("Stage file"),
                disabled,
                run: () => props.working!.onStage(change, !file.staged),
              },
              ...(!file.staged
                ? [
                    {
                      label: t("Discard changes…"),
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
                label: t("Unstaged changes"),
                disabled: [" ", "!"].includes(change.worktree),
                run: () => props.onDiff(file.path, false),
              },
              {
                label: t("Staged changes"),
                disabled: [" ", "?", "!"].includes(change.index),
                run: () => props.onDiff(file.path, true),
              },
              null,
            ]
          : []),
        {
          label: t("Copy path"),
          run: () =>
            run(() =>
              writeText(`${props.root.replace(/[\\/]$/, "")}/${file.path}`),
            ),
        },
        {
          label: t("Copy relative path"),
          run: () => run(() => writeText(file.path)),
        },
        ...(change
          ? [
              null,
              {
                label: t("Add to .gitignore"),
                disabled: disabled || change.index !== "?",
                run: () => ignore(false),
              },
              {
                label: t("Add to .git/info/exclude"),
                disabled: disabled || change.index !== "?",
                run: () => ignore(true),
              },
            ]
          : []),
        null,
        {
          label: t("Open diff"),
          run: () => props.onDiff(file.path, !!file.staged),
        },
        {
          label: change ? t("View file") : t("View working file"),
          disabled: file.deleted || file.path.endsWith("/"),
          run: () => props.onOpenFile(file.path),
        },
        null,
        { label: t("View file history"), run: () => setHistory(file.path) },
      ]}
    />
  );
  const dialogs = (
    <>
      {history !== undefined && (
        <Modal
          title={t("Git History · {path}", { path: history })}
          wide
          className="file-history-dialog"
          onClose={() => setHistory(undefined)}
        >
          <div className="dialog-content dialog-history-content">
            <GitHistory
              root={props.root}
              path={history}
              onOpenCommit={(commit) => {
                setHistory(undefined);
                props.onOpenCommit(commit);
              }}
            />
          </div>
        </Modal>
      )}
      {discard && (
        <Modal
          protectTheme
          title={t("Discard changes")}
          tone="danger"
          closeDisabled={busy}
          onClose={() => {
            if (!busy) setDiscard(undefined);
          }}
        >
          <form
            className="dialog-form dialog-layout"
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
            <div className="dialog-content">
              <p>
                {discard.index === "?"
                  ? t("Move {path} to the trash?", { path: discard.path })
                  : t(
                      "Discard unstaged changes in {path}? The file will be restored to its staged version. This cannot be undone.",
                      { path: discard.path },
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
                {t("Cancel")}
              </button>
              <button
                type="submit"
                className="button text-error"
                disabled={disabled}
              >
                {busy
                  ? t("Working…")
                  : discard.index === "?"
                    ? t("Move to trash")
                    : t("Discard changes")}
              </button>
            </div>
          </form>
        </Modal>
      )}
    </>
  );
  return { onContext: open, onKey, menu, dialogs };
}
