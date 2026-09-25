import ResourceIcon from "./ResourceIcon";
import {
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { Redo2, RotateCcw, Save, Search, Undo2, X } from "./icons";
import {
  SPLIT_DIVIDER_SIZE,
  type EditorPosition,
  type FileTab,
  type FilePreviewView,
} from "./model";
import { errorMessage } from "./api";
import { loadedEditor, openEditorDocument } from "./editor-service";
import type { EditorDocument } from "./editor-runtime";
import { useKeybindings } from "./KeybindingsProvider";
import { useEditorPreferences } from "./EditorPreferencesProvider";
import { shortcutTitle } from "./keybindings";
import { IconButton, Modal } from "./ui";
import { isMarkdownFile } from "./markdown";
import FilePreviewToggle from "./FilePreviewToggle";
import ImagePreview from "./ImagePreview";
import { imagePreviewType, isSvgFile } from "./image-preview";
import SvgPreview from "./SvgPreview";
import { useAgentPreview } from "./agent-preview";

const MarkdownPreview = lazy(() => import("./MarkdownPreview"));

interface Props {
  tab: FileTab;
  active?: boolean;
  onClose?: () => void;
  onPosition: (position: EditorPosition) => void;
  onPreviewView: (view: FilePreviewView) => void;
  onPreviewResize: (ratio: number) => void;
  onOpenFile: (root: string, relative: string) => void;
}

export default function FileEditor(props: Props) {
  if (
    !props.tab.untitled &&
    imagePreviewType(props.tab.relative) &&
    !isSvgFile(props.tab.relative) &&
    !loadedEditor(props.tab)
  )
    return (
      <ImagePreview
        key={`${props.tab.root}/${props.tab.relative}`}
        {...props}
      />
    );
  return <TextFileEditor {...props} />;
}

function TextFileEditor(props: Props) {
  const { ready } = useEditorPreferences();
  const [document, setDocument] = useState(() => loadedEditor(props.tab));
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!ready) return;
    let current = true;
    setError("");
    void openEditorDocument(props.tab)
      .then((document) => {
        if (current) setDocument(document);
      })
      .catch((error) => {
        if (current) setError(errorMessage(error));
      });
    return () => {
      current = false;
    };
  }, [props.tab.id, props.tab.root, props.tab.relative, attempt, ready]);
  if (!document)
    return (
      <div className="empty-message editor-loading" role="status">
        <ResourceIcon
          path={`${props.tab.root}/${props.tab.relative}`}
          size={25}
        />
        <strong>{props.tab.title}</strong>
        <p>{error || "Opening file…"}</p>
        {error && (
          <button
            className="button"
            onClick={() => setAttempt((value) => value + 1)}
          >
            Try again
          </button>
        )}
      </div>
    );
  return <DocumentEditor {...props} document={document} />;
}

function DocumentEditor({
  tab,
  active = true,
  onClose,
  document,
  onPosition,
  onPreviewView,
  onPreviewResize,
  onOpenFile,
}: Props & { document: EditorDocument }) {
  const agentPreview = useAgentPreview(tab.id);
  const status = useSyncExternalStore(document.subscribe, document.getSnapshot);
  const { bindings } = useKeybindings();
  const host = useRef<HTMLDivElement>(null);
  const positionCallback = useRef(onPosition);
  positionCallback.current = onPosition;
  const [confirmation, setConfirmation] = useState<
    "reload" | "overwrite" | null
  >(null);
  const [busy, setBusy] = useState(false);
  const markdown = isMarkdownFile(tab.relative);
  const svg = isSvgFile(tab.relative);
  const view = markdown || svg ? (tab.previewView ?? "editor") : "editor";
  const previewRatio =
    typeof tab.previewRatio === "number" && Number.isFinite(tab.previewRatio)
      ? Math.max(0.1, Math.min(0.9, tab.previewRatio))
      : 0.5;
  const sourceVisible = view !== "preview";
  const wasSourceVisible = useRef(sourceVisible);
  useLayoutEffect(() => {
    if (!sourceVisible) return;
    document.attach(host.current!, tab);
    return () => {
      const position = document.position();
      document.detach();
      positionCallback.current(position);
    };
  }, [document, tab.id, sourceVisible]);
  useEffect(() => {
    const restoringSource = sourceVisible && !wasSourceVisible.current;
    wasSourceVisible.current = sourceVisible;
    if (
      active &&
      sourceVisible &&
      (restoringSource ||
        !host.current
          ?.closest(".file-editor")
          ?.contains(window.document.activeElement))
    )
      document.focus();
  }, [active, document, sourceVisible, view]);
  const save = () =>
    void document
      .save()
      .catch((error) => document.reportError(errorMessage(error)));
  const confirm = async () => {
    setBusy(true);
    try {
      if (confirmation === "reload") await document.reload();
      else await document.save(true);
      setConfirmation(null);
    } catch (error) {
      document.reportError(errorMessage(error));
      setConfirmation(null);
    } finally {
      setBusy(false);
    }
  };
  const previewToggle = (markdown || svg) && (
    <FilePreviewToggle
      kind={svg ? "SVG" : "Markdown"}
      view={view}
      onChange={onPreviewView}
    />
  );
  const resizePreview = (ratio: number) =>
    onPreviewResize(Math.max(0.1, Math.min(0.9, ratio)));
  return (
    <section className="file-editor" aria-label={`Editor for ${tab.title}`}>
      <header className="editor-heading" data-pane-drag-handle>
        <ResourceIcon path={document.path} size={15} />
        <span className="editor-path" title={document.path}>
          {tab.untitled ? tab.title : tab.relative}
          {status.dirty ? " •" : ""}
        </span>
        <div className="editor-actions">
          <IconButton
            title="Undo"
            disabled={status.readOnly || !sourceVisible}
            onClick={() => document.command("undo")}
          >
            <Undo2 size={15} />
          </IconButton>
          <IconButton
            title="Redo"
            disabled={status.readOnly || !sourceVisible}
            onClick={() => document.command("redo")}
          >
            <Redo2 size={15} />
          </IconButton>
          <IconButton
            title={shortcutTitle("Find in file", bindings.findFile)}
            disabled={!sourceVisible}
            onClick={() => document.command("findFile")}
          >
            <Search size={15} />
          </IconButton>
          <IconButton
            title="Reload from disk"
            disabled={status.saving || !!tab.untitled}
            onClick={() => setConfirmation("reload")}
          >
            <RotateCcw size={15} />
          </IconButton>
          <button
            className="button editor-save"
            title={shortcutTitle("Save file", bindings.saveFile)}
            disabled={
              status.readOnly ||
              status.saving ||
              (!status.dirty && !tab.untitled) ||
              status.conflict
            }
            onClick={save}
          >
            <Save size={14} />
            {status.saving ? "Saving…" : "Save"}
          </button>
          {onClose && (
            <IconButton title={`Close ${tab.title} panel`} onClick={onClose}>
              <X size={15} />
            </IconButton>
          )}
        </div>
      </header>
      {status.conflict && (
        <div className="editor-notice" role="alert">
          <span>This file changed on disk. Your edits are preserved.</span>
          <button
            className="text-button"
            onClick={() => setConfirmation("reload")}
          >
            Reload from disk…
          </button>
          <button
            className="text-button"
            disabled={status.saving || status.readOnly}
            onClick={() => setConfirmation("overwrite")}
          >
            Overwrite disk version…
          </button>
        </div>
      )}
      {status.error && (
        <div className="editor-notice text-error" role="alert">
          {status.error}
        </div>
      )}
      <div
        className={`editor-content${view === "split" ? " is-split" : ""}${markdown || svg ? " has-preview" : ""}`}
        style={
          view === "split"
            ? {
                gridTemplateColumns: `minmax(0, ${previewRatio}fr) ${SPLIT_DIVIDER_SIZE}px minmax(0, ${1 - previewRatio}fr)`,
              }
            : undefined
        }
      >
        {sourceVisible && <div className="editor-host" ref={host} />}
        {view === "split" && (
          <div
            className="split-divider file-preview-divider"
            role="separator"
            aria-label="Resize preview"
            aria-orientation="vertical"
            aria-valuemin={10}
            aria-valuemax={90}
            aria-valuenow={Math.round(previewRatio * 100)}
            tabIndex={0}
            onDoubleClick={() => resizePreview(0.5)}
            onKeyDown={(event) => {
              if (event.key === "Home" || event.key === "End") {
                event.preventDefault();
                resizePreview(event.key === "Home" ? 0.1 : 0.9);
              } else if (
                ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(
                  event.key,
                )
              ) {
                event.preventDefault();
                resizePreview(
                  previewRatio +
                    (["ArrowLeft", "ArrowUp"].includes(event.key)
                      ? -0.05
                      : 0.05),
                );
              }
            }}
            onPointerDown={(event) => {
              if (!event.isPrimary || event.button !== 0) return;
              event.preventDefault();
              event.currentTarget.setPointerCapture(event.pointerId);
            }}
            onPointerMove={(event) => {
              if (!event.currentTarget.hasPointerCapture(event.pointerId))
                return;
              const bounds =
                event.currentTarget.parentElement?.getBoundingClientRect();
              if (!bounds) return;
              const availableWidth = bounds.width - SPLIT_DIVIDER_SIZE;
              if (availableWidth <= 0) return;
              resizePreview(
                (event.clientX - bounds.left - SPLIT_DIVIDER_SIZE / 2) /
                  availableWidth,
              );
            }}
            onPointerUp={(event) => {
              if (event.currentTarget.hasPointerCapture(event.pointerId))
                event.currentTarget.releasePointerCapture(event.pointerId);
            }}
            onPointerCancel={(event) => {
              if (event.currentTarget.hasPointerCapture(event.pointerId))
                event.currentTarget.releasePointerCapture(event.pointerId);
            }}
          />
        )}
        {view !== "editor" && svg && (
          <SvgPreview
            tab={tab}
            document={document}
            active={active && !sourceVisible}
          >
            {previewToggle}
          </SvgPreview>
        )}
        {view !== "editor" && markdown && (
          <Suspense
            fallback={
              <div
                className="empty-message markdown-preview-loading"
                role="status"
              >
                Loading preview…
              </div>
            }
          >
            <MarkdownPreview
              document={document}
              onOpenFile={onOpenFile}
              assetPermit={
                tab.agentPreview
                  ? (agentPreview?.assetPermit ?? null)
                  : undefined
              }
            />
          </Suspense>
        )}
        {(!svg || view === "editor") && previewToggle}
      </div>
      {confirmation && (
        <Modal
          protectTheme
          tone="warning"
          title={
            confirmation === "reload"
              ? "Reload file from disk?"
              : "Overwrite disk version?"
          }
          onClose={() => {
            if (!busy) setConfirmation(null);
          }}
        >
          <p>
            {confirmation === "reload"
              ? "Reloading replaces the editor contents and clears its undo history. Any unsaved edits to this file will be discarded."
              : "Saving replaces the version currently on disk with your editor contents. Changes made by another program will be replaced."}
          </p>
          <p className="editor-confirm-path">{document.path}</p>
          <div className="dialog-actions">
            <button
              className="button"
              autoFocus
              disabled={busy}
              onClick={() => setConfirmation(null)}
            >
              Cancel
            </button>
            <button
              className="button button-primary button-danger"
              disabled={busy}
              onClick={() => void confirm()}
            >
              {busy
                ? "Working…"
                : confirmation === "reload"
                  ? "Reload file"
                  : "Overwrite file"}
            </button>
          </div>
        </Modal>
      )}
    </section>
  );
}
