import { useEffect, useState } from "react";
import { api, errorMessage } from "./api";

interface Conversation {
  conversationId: string;
  title: string;
}

export function AgentChatPermission({
  projectId,
  selected,
  onChange,
  disabled,
}: {
  projectId: string;
  selected: string[];
  onChange: (ids: string[]) => void;
  disabled: boolean;
}) {
  const [afterId, setAfterId] = useState<string | null>(null);
  const [page, setPage] = useState<{
    items: Conversation[];
    next: string | null;
  } | null>(null);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let alive = true;
    setPage(null);
    setError("");
    void api<{ items: Conversation[]; next: string | null }>(
      "agent_control_chat_catalog",
      { projectId, afterId },
    )
      .then((value) => {
        if (alive) setPage(value);
      })
      .catch((reason) => {
        if (alive) setError(errorMessage(reason));
      });
    return () => {
      alive = false;
    };
  }, [projectId, afterId, retry]);
  return (
    <fieldset className="agent-chat-permission" disabled={disabled}>
      <legend>Conversations to share</legend>
      <p className="settings-help">
        Share saved messages and drafts from only the conversations you select.
        Unsaved text, system prompts, attachments, reasoning and provider
        credentials are excluded. This does not allow sending messages or
        stopping responses.
      </p>
      <p role="status">{selected.length} of 64 conversations selected</p>
      {error ? (
        <>
          <p role="alert">{error}</p>
          <button
            type="button"
            className="button"
            onClick={() => setRetry((n) => n + 1)}
          >
            Retry loading conversations
          </button>
        </>
      ) : page ? (
        <>
          {page.items.length === 0 && <p>No conversations in this project.</p>}
          {page.items.map((item) => (
            <label key={item.conversationId}>
              <input
                type="checkbox"
                checked={selected.includes(item.conversationId)}
                disabled={
                  selected.length >= 64 &&
                  !selected.includes(item.conversationId)
                }
                onChange={(event) =>
                  onChange(
                    event.target.checked
                      ? [...selected, item.conversationId]
                      : selected.filter((id) => id !== item.conversationId),
                  )
                }
              />
              <span>
                {item.title || "Untitled conversation"}{" "}
                <code>{item.conversationId}</code>
              </span>
            </label>
          ))}
          <div className="agent-control-actions">
            {afterId !== null && (
              <button
                type="button"
                className="button"
                onClick={() => setAfterId(null)}
              >
                First conversations
              </button>
            )}
            {page.next !== null && (
              <button
                type="button"
                className="button"
                onClick={() => setAfterId(page.next)}
              >
                Next conversations
              </button>
            )}
          </div>
        </>
      ) : (
        <p role="status">Loading conversations…</p>
      )}
      {selected.length > 0 && (
        <button type="button" className="button" onClick={() => onChange([])}>
          Clear conversation selection
        </button>
      )}
    </fieldset>
  );
}
