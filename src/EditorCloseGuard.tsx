import ResourceIcon from "./ResourceIcon";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { CircleAlert, Save } from "./icons";
import { closingEditorDocuments } from "./editor-service";
import { pluginHost } from "./plugins/runtime";
interface CloseDocument {
  path: string;
  readonly dirty: boolean;
  save: () => Promise<boolean>;
  discard?: () => void;
}
import { errorMessage } from "./api";
import { basename } from "./model";
import { Modal } from "./ui";

interface Request {
  documents: CloseDocument[];
  finish: (close: boolean) => void;
  decision?: EditorCloseDecision;
  cancelled?: boolean;
}
export interface EditorCloseDecision {
  description: string;
  onDecision: (choice: "save" | "discard") => Promise<void>;
  onSaveStart?: () => Promise<void>;
  isActive: () => Promise<boolean>;
}

export function useEditorCloseGuard() {
  const descriptionId = useId();
  const [request, setRequest] = useState<Request>();
  const current = useRef<Request>(undefined);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const saveButton = useRef<HTMLButtonElement>(null);
  useEffect(
    () => () => {
      current.current?.finish(false);
      current.current = undefined;
    },
    [],
  );
  useEffect(() => {
    if (!request?.decision) return;
    let stopped = false;
    let pending = false;
    const check = async () => {
      if (pending) return;
      pending = true;
      const active = await request.decision!.isActive().catch(() => false);
      pending = false;
      if (stopped || current.current !== request) return;
      if (!active) request.cancelled = true;
      if (request.cancelled && !busy) {
        request.finish(false);
        current.current = undefined;
        setRequest(undefined);
      }
    };
    const timer = window.setInterval(() => void check(), 500);
    void check();
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, [request, busy]);
  const confirm = useCallback(
    (
      ids?: ReadonlySet<string>,
      decision?: EditorCloseDecision,
    ): Promise<boolean> => {
      if (current.current) return Promise.resolve(false);
      const documents: CloseDocument[] = [
        ...closingEditorDocuments(ids),
        ...[...pluginHost.dirtyViews]
          .filter(([id]) => !ids || ids.has(id))
          .map(([id, { view }]): CloseDocument => ({
            path: view.title || id,
            get dirty() {
              return pluginHost.isDirty(id);
            },
            save: async () => {
              await view.save();
              return !view.isDirty();
            },
            discard: () => view.discard(),
          }))
          .filter((view) => view.dirty),
      ];
      if (!documents.length) return Promise.resolve(true);
      return new Promise((finish) => {
        const request = { documents, finish, decision };
        current.current = request;
        setRequest(request);
        setError("");
      });
    },
    [],
  );
  const finish = (close: boolean) => {
    if (busy) return;
    current.current?.finish(close);
    current.current = undefined;
    setRequest(undefined);
  };
  const save = async () => {
    if (!request || busy) return;
    setBusy(true);
    setError("");
    try {
      if (request.cancelled) return;
      await request.decision?.onSaveStart?.();
      for (const document of request.documents)
        if (document.dirty && !(await document.save())) return;
      if (request.documents.some((document) => document.dirty))
        throw new Error("Some files still have unsaved changes.");
      if (request.cancelled) return;
      await request.decision?.onDecision("save");
      request.finish(true);
      current.current = undefined;
      setRequest(undefined);
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };
  return {
    confirm,
    dialog: request && (
      <Modal
        protectTheme
        title="Save changes before closing?"
        tone="warning"
        className="editor-close-dialog"
        descriptionId={descriptionId}
        initialFocus={saveButton}
        onClose={() => finish(false)}
      >
        <div className="editor-close-content">
          <p id={descriptionId} className="editor-close-description">
            {request.decision && <>{request.decision.description} </>}
            {request.documents.length === 1
              ? "This file has unsaved changes."
              : `${request.documents.length} files have unsaved changes.`}{" "}
            Your edits will be lost if you discard them.
          </p>
          <ul
            className="editor-unsaved-files"
            aria-label="Files with unsaved changes"
          >
            {request.documents.map((document) => (
              <li key={document.path} title={document.path}>
                <ResourceIcon path={document.path} size={18} />
                <span className="editor-unsaved-file">
                  <span className="editor-unsaved-name">
                    {basename(document.path)}
                  </span>
                  <span className="editor-unsaved-path">{document.path}</span>
                </span>
              </li>
            ))}
          </ul>
          {error && (
            <p className="editor-close-error" role="alert">
              <CircleAlert size={16} aria-hidden="true" />
              <span>{error}</span>
            </p>
          )}
        </div>
        <div className="editor-close-actions">
          <button
            type="button"
            className="button editor-close-cancel"
            disabled={busy}
            onClick={() => finish(false)}
          >
            Cancel
          </button>
          <button
            type="button"
            className="button button-danger"
            disabled={busy}
            onClick={async () => {
              if (busy) return;
              setBusy(true);
              setError("");
              try {
                if (request.cancelled) return;
                await request.decision?.onDecision("discard");
                for (const document of request.documents) document.discard?.();
                request.finish(true);
                current.current = undefined;
                setRequest(undefined);
              } catch (error) {
                setError(errorMessage(error));
              } finally {
                setBusy(false);
              }
            }}
          >
            Discard changes
          </button>
          <button
            ref={saveButton}
            type="button"
            className="button button-primary"
            disabled={busy}
            onClick={() => void save()}
          >
            <Save size={14} aria-hidden="true" />
            {busy ? "Saving…" : "Save changes"}
          </button>
        </div>
      </Modal>
    ),
  };
}
