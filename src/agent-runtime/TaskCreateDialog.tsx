import { useEffect, useId, useRef, useState } from "react";
import LegacyImport from "./LegacyImport";
import NativeQualificationNotice from "./NativeQualificationNotice";
import { Modal } from "../ui";
import { api, errorMessage } from "../api";
import { newId } from "../model";
import { cliNames } from "../cli-agents";
import { useAccountsSnapshot } from "./useAccountsSnapshot";
import type { Task, TasksSnapshot } from "./types";
export default function TaskCreateDialog({
  cwd,
  shellProfileId,
  onClose,
  onOpenTask,
}: {
  cwd: string;
  shellProfileId: string;
  onClose: () => void;
  onOpenTask: (task: Task) => void;
}) {
  const accountFieldId = useId();
  const { snapshot, error: accountsError } = useAccountsSnapshot();
  const [accountId, setAccountId] = useState("");
  const [model, setModel] = useState("");
  const [title, setTitle] = useState("");
  const [reasoningEffort, setReasoningEffort] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [tasks, setTasks] = useState<Task[]>([]);
  const initial = useRef<HTMLSelectElement>(null);
  const accounts =
    snapshot?.accounts.filter((account) =>
      snapshot.capabilities.some(
        (capability) =>
          capability.cli === account.cli && capability.managedExecution,
      ),
    ) ?? [];
  const selected = accounts.find((account) => account.accountId === accountId);
  useEffect(() => {
    let live = true;
    void api<TasksSnapshot>("agent_tasks_snapshot")
      .then((snapshot) => {
        if (live) setTasks(snapshot.tasks.filter((task) => task.cwd === cwd));
      })
      .catch((cause) => {
        if (live) setError(errorMessage(cause));
      });
    return () => {
      live = false;
    };
  }, [cwd]);
  async function create() {
    if (!selected || busy) return;
    setBusy(true);
    setError("");
    try {
      const task = await api<Task>("agent_task_create", {
        request: {
          operationId: newId(),
          accountId: selected.accountId,
          authRevision: selected.authRevision,
          cwd,
          title: title.trim() || "CLI task",
          model: model.trim(),
          reasoningEffort: reasoningEffort.trim() || null,
          shellProfileId,
        },
      });
      onOpenTask(task);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      title="New CLI task"
      initialFocus={initial}
      closeDisabled={busy}
      onClose={onClose}
    >
      <div className="dialog-form" aria-busy={busy}>
        <p>Work in {cwd} using a native CLI account.</p>
        <div className="dialog-form">
          <label htmlFor={accountFieldId}>Account</label>
          <select
            id={accountFieldId}
            ref={initial}
            value={accountId}
            disabled={busy}
            onChange={(event) => setAccountId(event.target.value)}
          >
            <option value="">Choose an account</option>
            {accounts.map((account) => (
              <option key={account.accountId} value={account.accountId}>
                {account.label} · {cliNames[account.cli]}
              </option>
            ))}
          </select>
        </div>
        {selected && (
          <p role="status">
            {selected.authState}
            {selected.availabilityReason
              ? ` · ${selected.availabilityReason}`
              : ""}
          </p>
        )}
        <label>
          Task title
          <input
            value={title}
            disabled={busy}
            onChange={(event) => setTitle(event.target.value)}
          />
        </label>
        <label>
          Model
          <input
            value={model}
            disabled={busy}
            onChange={(event) => setModel(event.target.value)}
            placeholder="Exact model identifier"
          />
        </label>
        <label>
          Reasoning effort (optional)
          <input
            value={reasoningEffort}
            disabled={busy}
            onChange={(event) => setReasoningEffort(event.target.value)}
          />
        </label>
        {(error || accountsError) && (
          <p role="alert">{error || accountsError}</p>
        )}
        <NativeQualificationNotice
          cli={selected?.cli}
          qualified={
            snapshot?.capabilities.find(
              (capability) => capability.cli === selected?.cli,
            )?.stopAndContinueQualified
          }
        />
        <div className="dialog-actions">
          <button className="button" disabled={busy} onClick={onClose}>
            Cancel
          </button>
          <button
            className="button button-primary"
            disabled={busy || !selected || !selected.enabled || !model.trim()}
            onClick={() => void create()}
          >
            Create task
          </button>
        </div>
        <LegacyImport />
        {tasks.length > 0 && (
          <section aria-label="Saved tasks">
            <h3>Saved tasks</h3>
            {tasks.map((task) => (
              <button
                className="button"
                key={task.taskId}
                disabled={busy}
                onClick={() => onOpenTask(task)}
              >
                {task.title} · {task.state}
              </button>
            ))}
          </section>
        )}
      </div>
    </Modal>
  );
}
