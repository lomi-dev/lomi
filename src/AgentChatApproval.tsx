import { useCallback, useEffect, useId, useRef, useState } from "react";
import { api, errorMessage } from "./api";
import { Modal } from "./ui";

export interface ChatSendPlan {
  planHash: string;
  conversationTitle: string;
  connectionId: string;
  connectionName: string;
  provider: string;
  model: string;
  maxOutputTokens: number;
  temperature: number | null;
  draftText: string;
  system: string;
  messageCount: number;
  contextBytes: number;
  attachments: { name: string; mime: string; byteLength: number }[];
}
export interface ChatApprovalRequest {
  operationId: string;
  nonce: string;
  plan: ChatSendPlan;
  isActive: () => Promise<boolean>;
}

export function useAgentChatApproval() {
  const [request, setRequest] = useState<ChatApprovalRequest>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const resolve = useRef<((approved: boolean) => void) | undefined>(undefined);
  const deciding = useRef(false);
  const cancel = useRef<HTMLButtonElement>(null);
  const description = useId();
  const finish = useCallback((approved: boolean) => {
    resolve.current?.(approved);
    resolve.current = undefined;
    setRequest(undefined);
  }, []);
  useEffect(
    () => () => {
      resolve.current?.(false);
      resolve.current = undefined;
    },
    [],
  );
  const confirm = useCallback((next: ChatApprovalRequest) => {
    if (resolve.current) return Promise.resolve(false);
    setError("");
    setBusy(false);
    deciding.current = false;
    return new Promise<boolean>((done) => {
      resolve.current = done;
      setRequest(next);
    });
  }, []);
  useEffect(() => {
    if (!request) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const check = async () => {
      if (!deciding.current) {
        const active = await request.isActive().catch(() => false);
        if (stopped) return;
        if (!active && !deciding.current) {
          finish(false);
          return;
        }
      }
      if (!stopped) timer = setTimeout(() => void check(), 500);
    };
    void check();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [request, finish]);
  const decide = async (approved: boolean) => {
    if (!request || deciding.current) return;
    deciding.current = true;
    setBusy(true);
    try {
      if (approved && !(await request.isActive()))
        throw new Error(
          "The Chat AI request is no longer current. Cancel and request a fresh preview.",
        );
      await api("agent_control_chat_send_decide", {
        operationId: request.operationId,
        nonce: request.nonce,
        planHash: request.plan.planHash,
        approved,
      });
      finish(approved);
    } catch (failure) {
      if (!approved) finish(false);
      else setError(errorMessage(failure));
    } finally {
      deciding.current = false;
      setBusy(false);
    }
  };
  const plan = request?.plan;
  return {
    confirm,
    dialog: request && plan && (
      <Modal
        protectTheme
        className="agent-chat-approval"
        tone="warning"
        title="Send this message for the agent?"
        descriptionId={description}
        initialFocus={cancel}
        onClose={() => {
          if (!busy) void decide(false);
        }}
      >
        <div className="dialog-form" aria-busy={busy}>
          <p id={description}>
            Send the message, prior conversation context, system instructions
            and listed attachments to {plan.provider} through{" "}
            {plan.connectionName}. This may incur charges from your provider.
          </p>
          <dl className="agent-chat-send-details">
            <dt>Conversation</dt>
            <dd>{plan.conversationTitle}</dd>
            <dt>Connection</dt>
            <dd>
              {plan.connectionName} ({plan.connectionId})
            </dd>
            <dt>Model</dt>
            <dd>{plan.model}</dd>
            <dt>Response limit</dt>
            <dd>{plan.maxOutputTokens.toLocaleString()} tokens</dd>
            <dt>Context</dt>
            <dd>
              {plan.messageCount.toLocaleString()} messages, including this
              draft · {plan.contextBytes.toLocaleString()} bytes
            </dd>
          </dl>
          <label>
            Message to send
            <textarea readOnly value={plan.draftText} rows={5} />
          </label>
          {plan.system ? (
            <details>
              <summary>System instructions included</summary>
              <label>
                System instructions
                <textarea readOnly value={plan.system} rows={4} />
              </label>
            </details>
          ) : (
            <p>No system instructions.</p>
          )}
          {plan.attachments.length ? (
            <details open>
              <summary>{plan.attachments.length} attachments included</summary>
              <ul>
                {plan.attachments.map((file, index) => (
                  <li key={index}>
                    {file.name} · {file.mime} ·{" "}
                    {file.byteLength.toLocaleString()} bytes
                  </li>
                ))}
              </ul>
            </details>
          ) : (
            <p>No attachments.</p>
          )}
          <p>
            The price depends on your provider and token usage. Approval applies
            only to this saved draft and this conversation’s current context.
          </p>
          {error && (
            <p className="error-text" role="alert">
              {error}
            </p>
          )}
          <div className="dialog-actions">
            <button
              ref={cancel}
              type="button"
              disabled={busy}
              onClick={() => void decide(false)}
            >
              Cancel
            </button>
            <button
              type="button"
              className="primary"
              disabled={busy}
              onClick={() => void decide(true)}
            >
              {busy ? "Confirming…" : "Send message"}
            </button>
          </div>
        </div>
      </Modal>
    ),
  };
}
