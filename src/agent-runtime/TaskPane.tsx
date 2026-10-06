import { useId, useRef, useState, useSyncExternalStore } from "react";
import type { AgentTaskTab } from "../model";
import { newId } from "../model";
import { errorMessage } from "../api";
import { cliNames } from "../cli-agents";
import { Modal } from "../ui";
import { getTaskView } from "./task-runtime";
import {
  continuationLabel,
  executionRequest,
  hasHistoryGrant,
  isRunning,
} from "./model";
import type { AccountInstance, Task, TransferReview } from "./types";
import LegacyImport from "./LegacyImport";
import NativeQualificationNotice from "./NativeQualificationNotice";
import "./task.css";
type Action =
  "send" | "continue" | "next_turn" | "stop_and_continue" | "transfer";
export default function TaskPane({
  tab,
  onFocus,
  onClose,
}: {
  tab: AgentTaskTab;
  onFocus: () => void;
  onClose: () => void;
}) {
  const messageId = useId();
  const owner = getTaskView(tab.taskId);
  const state = useSyncExternalStore(owner.subscribe, owner.getSnapshot);
  const [review, setReview] = useState<{
    task: Task;
    account: AccountInstance;
    action: Action;
    draft: string;
  }>();
  const [error, setError] = useState("");
  const [transferReview, setTransferReview] = useState<{
    task: Task;
    account: AccountInstance;
    preview: TransferReview;
  }>();
  const [transferStatus, setTransferStatus] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  const task = state.task;
  const target = state.accounts?.accounts.find(
    (account) => account.accountId === state.targetAccountId,
  );
  const active = state.accounts?.accounts.find(
    (account) => account.accountId === task?.activeAccountId,
  );
  const available =
    state.accounts?.accounts.filter((account) => account.cli === task?.cli) ??
    [];
  async function execute(
    action: Action,
    reviewedTask: Task,
    account: AccountInstance,
    draft: string,
  ) {
    const currentAccount = owner.state.accounts?.accounts.find(
      (candidate) => candidate.accountId === account.accountId,
    );
    if (currentAccount?.recovery)
      throw new Error(currentAccount.recovery.reason);
    if (action === "transfer") {
      const preview = await owner.previewTransfer({
        operationId: newId(),
        taskId: reviewedTask.taskId,
        expectedRevision: reviewedTask.revision,
        accountId: account.accountId,
        authRevision: account.authRevision,
        expectedHistoryRevision: reviewedTask.historyRevision,
      });
      setTransferReview({ task: reviewedTask, account, preview });
      return;
    }
    if (action === "next_turn" || action === "stop_and_continue") {
      const prepared = await owner.command("agent_task_prepare_switch", {
        operationId: newId(),
        taskId: reviewedTask.taskId,
        expectedRevision: reviewedTask.revision,
        accountId: account.accountId,
        authRevision: account.authRevision,
        expectedHistoryRevision: reviewedTask.historyRevision,
        sourceAttemptId: reviewedTask.activeAttemptId,
        mode: action,
      });
      if (action === "stop_and_continue") {
        const operation = prepared.switches.at(-1);
        if (!operation)
          throw new Error(
            "The switch preparation did not return a retained operation.",
          );
        await owner.completeSwitch(operation.operationId, account);
      }
      return;
    }
    const sent = await owner.command(
      "agent_task_send",
      executionRequest(
        reviewedTask,
        account,
        newId(),
        draft,
        action === "continue",
      ),
    );
    owner.trackSubmission(sent, draft);
  }
  async function act(action: Action) {
    if (!task || !target || state.busy) return;
    setError("");
    if (task.history.length && !hasHistoryGrant(task.grants, target)) {
      setReview({ task, account: { ...target }, action, draft: state.draft });
      return;
    }
    try {
      await execute(action, task, { ...target }, state.draft);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }
  async function grant() {
    if (!review) return;
    try {
      const granted = await owner.command("agent_task_update_history_grant", {
        operationId: newId(),
        taskId: review.task.taskId,
        expectedRevision: review.task.revision,
        accountId: review.account.accountId,
        authRevision: review.account.authRevision,
        expectedHistoryRevision: review.task.historyRevision,
        allow: true,
      });
      const selected = review;
      setReview(undefined);
      await execute(selected.action, granted, selected.account, selected.draft);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }
  const running = task && isRunning(task.state);
  const needsRecovery =
    task?.state === "recovery_required" || task?.state === "delivery_uncertain";
  const blocked = !target || !target.enabled || !!target.recovery || state.busy;
  const capability = state.accounts?.capabilities.find(
    (candidate) => candidate.cli === task?.cli,
  );
  const supportsTransfer =
    task?.cli === "pi" &&
    state.accounts?.capabilities.some(
      (capability) => capability.cli === "pi" && capability.reviewedTransfer,
    );
  async function applyTransfer() {
    if (!transferReview) return;
    setError("");
    try {
      const { task: reviewedTask, account, preview } = transferReview;
      const currentAccount = owner.state.accounts?.accounts.find(
        (candidate) => candidate.accountId === account.accountId,
      );
      if (
        owner.state.targetAccountId !== account.accountId ||
        currentAccount?.authRevision !== account.authRevision
      )
        throw new Error(
          "The selected account or its login changed. Review the native transfer again.",
        );
      await owner.command("agent_task_transfer_apply", {
        operationId: newId(),
        taskId: reviewedTask.taskId,
        expectedRevision: reviewedTask.revision,
        accountId: account.accountId,
        authRevision: account.authRevision,
        expectedHistoryRevision: reviewedTask.historyRevision,
        expectedDigest: preview.digest,
        expectedBytes: preview.bytes,
      });
      setTransferReview(undefined);
      setTransferStatus(
        `History prepared for ${account.label}. Choose Send or Continue explicitly to start work.`,
      );
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }
  return (
    <section
      className="agent-task-pane"
      onPointerDown={onFocus}
      aria-label={tab.title}
      data-task-id={tab.taskId}
    >
      <header className="agent-task-header">
        <strong>{task?.title ?? tab.title}</strong>
        <button className="button" onClick={onClose}>
          Close task
        </button>
      </header>
      <div className="agent-task-content">
        {!state.loaded && <p role="status">Loading task…</p>}
        {state.loaded && state.missing && (
          <p role="status">
            {tab.taskId.startsWith("legacy:")
              ? "This saved CLI task is preserved as legacy history. Import its archive before continuing."
              : "This task is unavailable. Its saved view remains here; no work was started."}
          </p>
        )}
        {state.loaded && state.missing && tab.taskId.startsWith("legacy:") && (
          <LegacyImport />
        )}
        {(error || state.error) && <p role="alert">{error || state.error}</p>}
        {task && (
          <>
            {task.availabilityReason && (
              <p role="status">{task.availabilityReason}</p>
            )}
            <p aria-live="polite">{task.statusMessage || task.state}</p>
            <dl className="agent-task-meta">
              <div>
                <dt>Active account</dt>
                <dd>{active?.label ?? task.activeAccountId ?? "None"}</dd>
              </div>
              <div>
                <dt>Next execution</dt>
                <dd>
                  {target?.label ??
                    state.targetAccountId ??
                    "Choose an account"}
                </dd>
              </div>
              <div>
                <dt>CLI</dt>
                <dd>{task.cli ? cliNames[task.cli] : "Unknown CLI"}</dd>
              </div>
              <div>
                <dt>Model</dt>
                <dd>{task.model}</dd>
              </div>
            </dl>
            <label>
              Next account
              <select
                value={state.targetAccountId}
                disabled={state.busy}
                onChange={(event) => owner.setTarget(event.target.value)}
              >
                {!available.some(
                  (account) => account.accountId === state.targetAccountId,
                ) && (
                  <option value={state.targetAccountId}>
                    {state.targetAccountId || "Choose an account"} (unavailable)
                  </option>
                )}
                {available.map((account) => (
                  <option key={account.accountId} value={account.accountId}>
                    {account.label} · {cliNames[account.cli]}
                    {!account.enabled ? " (disabled)" : ""}
                  </option>
                ))}
              </select>
            </label>
            {target?.availabilityReason && <p>{target.availabilityReason}</p>}
            <p className="settings-help">
              Quota unavailable. A qualified account can still be selected
              manually.
            </p>
            <div className="dialog-actions">
              {supportsTransfer && (
                <button
                  className="button"
                  disabled={
                    blocked ||
                    !!running ||
                    needsRecovery ||
                    task.state === "archived"
                  }
                  onClick={() => void act("transfer")}
                >
                  Review native Pi history transfer
                </button>
              )}
              <button
                className="button"
                disabled={blocked}
                onClick={() => void act("next_turn")}
              >
                Use after this turn
              </button>
              {running && (
                <button
                  className="button"
                  disabled={blocked || !capability?.stopAndContinueQualified}
                  onClick={() => void act("stop_and_continue")}
                >
                  Stop and continue on selected account
                </button>
              )}
            </div>
            {running && !capability?.stopAndContinueQualified && (
              <p className="settings-help" role="status">
                {capability?.stopAndContinueReason ??
                  "This CLI’s background work cannot yet be verified as stopped."}
              </p>
            )}
            {transferStatus && <p role="status">{transferStatus}</p>}
            {task.switches.slice(-1).map((operation) => (
              <p key={operation.operationId} className="settings-help">
                {continuationLabel(operation.continuationMethod)} ·{" "}
                {operation.coverage} history records included
                {operation.reason ? ` · ${operation.reason}` : ""}
              </p>
            ))}
            <div className="agent-task-history" aria-label="Task history">
              {task.attempts.map((attempt) => (
                <article key={attempt.attemptId}>
                  <h3>
                    {state.accounts?.accounts.find(
                      (account) => account.accountId === attempt.accountId,
                    )?.label ?? attempt.accountId}{" "}
                    · {attempt.state}
                  </h3>
                  <p className="settings-help">
                    {continuationLabel(attempt.continuationMethod)} ·{" "}
                    {attempt.effectsState}
                  </p>
                  <pre>{attempt.input}</pre>
                  <pre>{attempt.output}</pre>
                  {task.history
                    .filter(
                      (record) =>
                        record.attemptId === attempt.attemptId &&
                        record.kind !== "user" &&
                        record.kind !== "assistant",
                    )
                    .map((record) => (
                      <details key={record.sequence}>
                        <summary>
                          {record.kind} · {record.state}
                        </summary>
                        <pre>{JSON.stringify(record.content, null, 2)}</pre>
                      </details>
                    ))}
                </article>
              ))}
              {task.history
                .filter(
                  (record) =>
                    !task.attempts.some(
                      (attempt) => attempt.attemptId === record.attemptId,
                    ),
                )
                .map((record) => (
                  <article key={`history-${record.sequence}`}>
                    <h3>
                      {record.kind === "legacy_record"
                        ? "Archived history"
                        : "Retained history"}
                    </h3>
                    <p className="settings-help">
                      Source account:{" "}
                      {state.accounts?.accounts.find(
                        (account) => account.accountId === record.accountId,
                      )?.label ?? record.accountId}
                      {" · "}Login revision {record.authRevision}
                      {" · "}
                      {record.state}
                    </p>
                    <pre>
                      {typeof record.content === "string"
                        ? record.content
                        : JSON.stringify(record.content, null, 2)}
                    </pre>
                  </article>
                ))}
            </div>
            <div className="agent-task-field">
              <label htmlFor={messageId}>Message</label>
              <textarea
                id={messageId}
                value={state.draft}
                disabled={state.busy || task.state === "archived"}
                onChange={(event) => owner.setDraft(event.target.value)}
                rows={4}
              />
            </div>
            <NativeQualificationNotice
              cli={task.cli}
              qualified={capability?.stopAndContinueQualified}
            />
            {target?.recovery && (
              <p role="alert">
                {target.recovery.reason} Open Settings → Agent control → CLI
                Accounts to review this account.
              </p>
            )}
            <div className="dialog-actions">
              <button
                className="button button-primary"
                disabled={
                  blocked ||
                  !!running ||
                  needsRecovery ||
                  !state.draft.trim() ||
                  task.state === "archived"
                }
                onClick={() => void act("send")}
              >
                Send
              </button>
              <button
                className="button"
                disabled={
                  blocked ||
                  !!running ||
                  needsRecovery ||
                  !task.attempts.length ||
                  task.state === "archived"
                }
                onClick={() => void act("continue")}
              >
                Continue on selected account
              </button>
              {(running || needsRecovery) && (
                <button
                  className="button"
                  disabled={state.busy}
                  onClick={() =>
                    void owner
                      .command("agent_task_stop", {
                        operationId: newId(),
                        taskId: task.taskId,
                        expectedRevision: task.revision,
                      })
                      .catch((cause) => setError(errorMessage(cause)))
                  }
                >
                  {needsRecovery ? "Stop previous process" : "Stop"}
                </button>
              )}
            </div>
            {(task.state === "recovery_required" ||
              task.state === "delivery_uncertain" ||
              task.state === "archived") && (
              <div>
                <p>
                  Review the retained history and project effects. Recovery
                  settles this attempt without repeating its input.
                </p>
                <button
                  className="button"
                  disabled={state.busy}
                  onClick={() =>
                    void owner
                      .command("agent_task_recover", {
                        operationId: newId(),
                        taskId: task.taskId,
                        expectedRevision: task.revision,
                        expectedHistoryRevision: task.historyRevision,
                        acknowledgeEffects: true,
                      })
                      .catch((cause) => setError(errorMessage(cause)))
                  }
                >
                  Acknowledge reviewed effects
                </button>
              </div>
            )}
          </>
        )}
      </div>
      {review && (
        <Modal
          title="Allow account access to task history?"
          wide
          initialFocus={cancel}
          closeDisabled={state.busy}
          onClose={() => setReview(undefined)}
        >
          <div className="dialog-form">
            <p>
              Allow <strong>{review.account.label}</strong>, login revision{" "}
              {review.account.authRevision}, to read this task’s messages and
              observed tool results? This permission remains valid for this
              login.
            </p>
            <p>
              {review.task.history.length} records in {review.task.cwd}. Model:{" "}
              {review.task.model}.
            </p>
            <details>
              <summary>Review task history</summary>
              <pre>{JSON.stringify(review.task.history, null, 2)}</pre>
            </details>
            {error && <p role="alert">{error}</p>}
            <div className="dialog-actions">
              <button
                ref={cancel}
                className="button"
                disabled={state.busy}
                onClick={() => setReview(undefined)}
              >
                Cancel
              </button>
              <button
                className="button button-primary"
                disabled={state.busy}
                onClick={() => void grant()}
              >
                {review.action === "transfer"
                  ? "Allow history access"
                  : "Allow and continue"}
              </button>
            </div>
          </div>
        </Modal>
      )}
      {transferReview && (
        <Modal
          title="Review native Pi history transfer"
          initialFocus={cancel}
          closeDisabled={state.busy}
          onClose={() => setTransferReview(undefined)}
        >
          <div className="dialog-form">
            <p>
              Transfer the settled native Pi session to{" "}
              {transferReview.account.label} (login revision{" "}
              {transferReview.account.authRevision}).
            </p>
            <p>
              Complete observed history through record{" "}
              {transferReview.preview.coverage}
              {" · "}
              {transferReview.preview.bytes.toLocaleString()} bytes
            </p>
            <p className="settings-help">
              Source attempt: {transferReview.preview.sourceAttemptId}
            </p>
            <details>
              <summary>Reviewed history fingerprint</summary>
              <pre>{transferReview.preview.digest}</pre>
            </details>
            <p>
              Applying this review prepares the selected account. Choose Send or
              Continue separately to start new work. Historical tools are not
              replayed.
            </p>
            {error && <p role="alert">{error}</p>}
            <div className="dialog-actions">
              <button
                ref={cancel}
                className="button"
                disabled={state.busy}
                onClick={() => setTransferReview(undefined)}
              >
                Cancel
              </button>
              <button
                className="button button-primary"
                disabled={state.busy}
                onClick={() => void applyTransfer()}
              >
                Apply reviewed transfer
              </button>
            </div>
          </div>
        </Modal>
      )}
    </section>
  );
}
