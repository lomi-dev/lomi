import { useRef, useState } from "react";
import { errorMessage } from "../api";
import { Pencil, Trash2 } from "../icons";
import { IconButton, Modal } from "../ui";
import { removeThread, renameThread, useThreads } from "./thread-runtime";
import type { ThreadSummary } from "./types";

type Action = { kind: "rename" | "delete"; thread: ThreadSummary };

export default function ThreadHistory({
  project,
  workspaceId,
  activeId,
  onOpen,
  onRenamed,
  onDeleted,
}: {
  project: string;
  workspaceId: string;
  activeId: string | null;
  onOpen: (thread: ThreadSummary) => void;
  onRenamed: (thread: ThreadSummary) => void;
  onDeleted: (threadId: string) => void;
}) {
  const { threads, error, reload } = useThreads(project, workspaceId);
  const [action, setAction] = useState<Action>();
  const [title, setTitle] = useState("");
  const [failure, setFailure] = useState("");
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const close = () => {
    setAction(undefined);
    setFailure("");
  };
  async function confirm() {
    if (!action) return;
    setBusy(true);
    setFailure("");
    try {
      if (action.kind === "rename") {
        const next = await renameThread(action.thread, title.trim());
        onRenamed(next);
      } else {
        await removeThread(action.thread);
        onDeleted(action.thread.threadId);
      }
      close();
      reload();
    } catch (cause) {
      setFailure(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <aside className="thread-history" aria-label="Chat history">
      <h2>History</h2>
      {error && <p className="thread-note">{error}</p>}
      {!threads.length && !error && (
        <p className="thread-note">No chats in this workspace yet.</p>
      )}
      <ul>
        {threads.map((thread) => (
          <li
            key={thread.threadId}
            data-active={thread.threadId === activeId || undefined}
          >
            <button
              type="button"
              className="thread-history-open"
              onClick={() => onOpen(thread)}
            >
              <span className="thread-history-title">{thread.title}</span>
              <small>
                {thread.kind === "cli" ? "CLI · " : ""}
                {thread.model} ·{" "}
                {new Date(thread.updatedMs).toLocaleDateString()}
              </small>
            </button>
            <IconButton
              title="Rename chat"
              onClick={() => {
                setTitle(thread.title);
                setAction({ kind: "rename", thread });
              }}
            >
              <Pencil size={13} />
            </IconButton>
            <IconButton
              title="Delete chat"
              onClick={() => setAction({ kind: "delete", thread })}
            >
              <Trash2 size={13} />
            </IconButton>
          </li>
        ))}
      </ul>
      {action && (
        <Modal
          title={action.kind === "rename" ? "Rename chat" : "Delete chat?"}
          tone={action.kind === "delete" ? "danger" : undefined}
          initialFocus={action.kind === "rename" ? input : cancel}
          closeDisabled={busy}
          onClose={close}
        >
          <form
            className="dialog-form dialog-layout"
            onSubmit={(event) => {
              event.preventDefault();
              if (action.kind === "delete" || title.trim()) void confirm();
            }}
          >
            <div className="dialog-content">
              {action.kind === "rename" ? (
                <input
                  ref={input}
                  value={title}
                  maxLength={200}
                  aria-label="Chat title"
                  onChange={(event) => setTitle(event.target.value)}
                />
              ) : (
                <p>
                  Delete <strong>{action.thread.title}</strong>? This cannot be
                  undone.
                </p>
              )}
              {failure && <p role="alert">{failure}</p>}
            </div>
            <div className="dialog-actions">
              <button
                type="button"
                ref={cancel}
                className="button"
                disabled={busy}
                onClick={close}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="button button-primary"
                disabled={busy || (action.kind === "rename" && !title.trim())}
              >
                {action.kind === "rename" ? "Rename" : "Delete"}
              </button>
            </div>
          </form>
        </Modal>
      )}
    </aside>
  );
}
