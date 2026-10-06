import { useEffect, useId, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "../api";
import { Modal } from "../ui";
import { permissionControls } from "./permission-model";
import { replyPermission } from "./permission-runtime";
import type { PendingPermission } from "./types";
const claims = new Set<string>();
export default function PermissionReview({ taskId }: { taskId?: string }) {
  const textFieldId = useId();
  const [pending, setPending] = useState<PendingPermission>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  const [text, setText] = useState("");
  const controls = pending ? permissionControls(pending) : undefined;
  useEffect(
    () => setText(controls?.textInput?.initial ?? ""),
    [pending?.approvalToken],
  );
  useEffect(() => {
    let live = true;
    let claimed: string | undefined;
    let snapshotSequence = 0;
    const refresh = async () => {
      const sequence = ++snapshotSequence;
      try {
        const permissions = await api<PendingPermission[]>(
          "agent_permission_snapshot",
          { taskId },
        );
        if (!live || sequence !== snapshotSequence) return;
        if (
          claimed &&
          !permissions.some(
            (permission) => permission.approvalToken === claimed,
          )
        ) {
          claims.delete(claimed);
          claimed = undefined;
        }
        const next = permissions.find(
          (permission) =>
            permission.approvalToken === claimed ||
            !claims.has(permission.approvalToken),
        );
        if (next) {
          claims.add(next.approvalToken);
          claimed = next.approvalToken;
        }
        setPending(next);
      } catch (cause) {
        if (live && sequence === snapshotSequence)
          setError(errorMessage(cause));
      }
    };
    const subscription = listen("agent-runtime-changed", () => void refresh())
      .then((stop) => {
        if (!live) stop();
        else void refresh();
        return stop;
      })
      .catch((cause) => {
        if (live) setError(errorMessage(cause));
      });
    const timer = setInterval(() => void refresh(), 500);
    return () => {
      live = false;
      clearInterval(timer);
      if (claimed) claims.delete(claimed);
      void subscription.then((stop) => stop?.());
    };
  }, [taskId]);
  async function decide(allow: boolean, choice?: unknown) {
    if (!pending || busy) return;
    setBusy(true);
    setError("");
    try {
      await replyPermission(pending, allow, choice);
      claims.delete(pending.approvalToken);
      setPending(undefined);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }
  if (!pending) return error ? <p role="alert">{error}</p> : null;
  return (
    <Modal
      title="Review native CLI permission"
      wide
      initialFocus={cancel}
      closeDisabled={busy || !controls?.cancel}
      onClose={() => {
        if (controls?.cancel) void decide(false, controls.cancel.value);
      }}
    >
      <div className="dialog-form" aria-busy={busy}>
        <p>
          This operation belongs to the active native attempt in{" "}
          {pending.projectRoot}. Save or close unsaved project editors before
          allowing it. Editors stay frozen until the attempt settles.
        </p>
        <pre>{JSON.stringify(pending.permission.raw, null, 2)}</pre>
        {error && <p role="alert">{error}</p>}
        {controls?.textInput && (
          <div>
            <label htmlFor={textFieldId}>{controls.textInput.label}</label>
            {controls.textInput.multiline ? (
              <textarea
                id={textFieldId}
                value={text}
                disabled={busy}
                onChange={(event) => setText(event.target.value)}
              />
            ) : (
              <input
                id={textFieldId}
                value={text}
                disabled={busy}
                onChange={(event) => setText(event.target.value)}
              />
            )}
          </div>
        )}
        <div className="dialog-actions">
          {controls?.choices.map((choice, index) => (
            <button
              key={index}
              ref={!choice.allow ? cancel : undefined}
              className={`button${choice.allow ? " button-primary" : ""}`}
              disabled={busy}
              onClick={() => void decide(choice.allow, choice.value)}
            >
              {choice.label}
            </button>
          ))}
          {controls?.textInput && (
            <button
              className="button button-primary"
              disabled={
                busy || new TextEncoder().encode(text).length > 64 * 1024
              }
              onClick={() => void decide(true, text)}
            >
              Submit response
            </button>
          )}
          {!controls?.choices.length && !controls?.textInput && (
            <p role="alert">
              The native client did not provide an enforceable permission
              choice. Stop the task to settle it.
            </p>
          )}
        </div>
      </div>
    </Modal>
  );
}
