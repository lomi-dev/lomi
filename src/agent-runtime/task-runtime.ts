import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "../api";
import {
  newId,
  agentTaskTabs,
  type AgentTaskTab,
  type Session,
} from "../model";
import { acceptTask, finalTaskIds, isRunning } from "./model";
import type {
  AccountInstance,
  AccountsSnapshot,
  Task,
  TasksSnapshot,
  TransferReview,
} from "./types";
let publicationPaused = false;
interface TaskViewState {
  task?: Task;
  accounts?: AccountsSnapshot;
  loaded: boolean;
  missing: boolean;
  error: string;
  busy: boolean;
  draft: string;
  targetAccountId: string;
}
class TaskView {
  state: TaskViewState = {
    loaded: false,
    missing: false,
    error: "",
    busy: false,
    draft: "",
    targetAccountId: "",
  };
  closing?: symbol;
  private commandBusy = false;
  private continuationEpoch = 0;
  cancelContinuations() {
    ++this.continuationEpoch;
  }
  private submission?: { attemptId: string; draft: string };
  private listeners = new Set<() => void>();
  constructor(readonly taskId: string) {}
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  getSnapshot = () => this.state;
  update(change: Partial<TaskViewState>) {
    this.state = {
      ...this.state,
      ...change,
      busy: this.commandBusy || !!this.closing || publicationPaused,
    };
    for (const listener of this.listeners) listener();
  }
  setDraft = (draft: string) => this.update({ draft });
  setTarget = (targetAccountId: string) => this.update({ targetAccountId });
  trackSubmission(task: Task, draft: string) {
    const attempt = task.attempts.at(-1);
    if (attempt) {
      this.submission = { attemptId: attempt.attemptId, draft };
      this.accept(task);
    }
  }
  accept(task: Task) {
    const accepted = acceptTask(this.state.task, task);
    if (this.submission) {
      const attempt = accepted.attempts.find(
        (attempt) => attempt.attemptId === this.submission!.attemptId,
      );
      if (attempt?.state === "rejected") {
        if (!this.state.draft) this.setDraft(this.submission.draft);
        this.submission = undefined;
      } else if (
        attempt &&
        ["running", "dispatched", "completed", "stopped"].includes(
          attempt.state,
        )
      ) {
        if (this.state.draft === this.submission.draft) this.setDraft("");
        if (["completed", "stopped"].includes(attempt.state))
          this.submission = undefined;
      }
    }
    this.update({
      task: accepted,
      loaded: true,
      missing: false,
      ...(!this.state.targetAccountId
        ? { targetAccountId: accepted.nextAccountId }
        : {}),
    });
  }
  async completeSwitch(operationId: string, account: AccountInstance) {
    if (this.state.busy) throw new Error("This task is busy or closing.");
    this.commandBusy = true;
    const continuationEpoch = this.continuationEpoch;
    this.update({ error: "" });
    try {
      const deadline = Date.now() + 30000;
      for (;;) {
        if (
          continuationEpoch !== this.continuationEpoch ||
          views.get(this.taskId) !== this ||
          !retained.some((tab) => tab.taskId === this.taskId)
        )
          throw new Error(
            "The switch was cancelled while its final view prepared to close. Choose Continue explicitly to resume.",
          );
        const task = await api<Task>("agent_task_snapshot", {
          taskId: this.taskId,
        });
        this.accept(task);
        const operation = task.switches.find(
          (item) => item.operationId === operationId,
        );
        if (
          !operation ||
          operation.accountId !== account.accountId ||
          operation.authRevision !== account.authRevision
        )
          throw new Error("The reviewed switch target changed.");
        if (
          operation.reason ||
          operation.phase === "recovery_required" ||
          operation.phase === "delivery_uncertain"
        )
          throw new Error(operation.reason ?? task.statusMessage);
        if (
          this.closing ||
          continuationEpoch !== this.continuationEpoch ||
          views.get(this.taskId) !== this
        )
          throw new Error("This task is preparing to close.");
        if (operation.phase === "prepared") {
          const next = await api<Task>("agent_task_commit_switch", {
            request: {
              operationId: newId(),
              taskId: task.taskId,
              switchOperationId: operationId,
              expectedRevision: task.revision,
              accountId: account.accountId,
              authRevision: account.authRevision,
            },
          });
          this.accept(next);
          return next;
        }
        if (Date.now() >= deadline)
          throw new Error(
            "The source attempt has not settled. The selected account is retained; no new work was started.",
          );
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
    } catch (cause) {
      this.update({ error: errorMessage(cause) });
      throw cause;
    } finally {
      this.commandBusy = false;
      this.update({});
    }
  }
  async previewTransfer(request: Record<string, unknown>) {
    if (this.state.busy) throw new Error("This task is busy or closing.");
    this.commandBusy = true;
    this.update({ error: "" });
    try {
      const preview = await api<TransferReview>("agent_task_transfer_preview", {
        request,
      });
      if (
        preview.taskId !== this.taskId ||
        preview.accountId !== request.accountId ||
        preview.authRevision !== request.authRevision ||
        preview.coverage !== request.expectedHistoryRevision
      )
        throw new Error(
          "The native transfer review does not match the selected task and account.",
        );
      return preview;
    } catch (cause) {
      this.update({ error: errorMessage(cause) });
      throw cause;
    } finally {
      this.commandBusy = false;
      this.update({});
    }
  }
  async command(name: string, request: Record<string, unknown>) {
    if (this.state.busy) throw new Error("This task is busy or closing.");
    this.commandBusy = true;
    this.update({ error: "" });
    try {
      const task = await api<Task>(name, { request });
      this.accept(task);
      return task;
    } catch (cause) {
      await refresh();
      this.update({ error: errorMessage(cause) });
      throw cause;
    } finally {
      this.commandBusy = false;
      this.update({});
    }
  }
}
let retained: AgentTaskTab[] = [];
const views = new Map<string, TaskView>();
let epoch = 0;
let subscribed = false;
let stop: (() => void) | undefined;
let poll: ReturnType<typeof setInterval> | undefined;
let refreshPromise: Promise<void> | undefined;
let refreshAgain = false;
export function refresh() {
  if (refreshPromise) {
    refreshAgain = true;
    return refreshPromise;
  }
  const generation = epoch;
  refreshPromise = (async () => {
    do {
      refreshAgain = false;
      try {
        const [tasks, accounts] = await Promise.all([
          api<TasksSnapshot>("agent_tasks_snapshot"),
          api<AccountsSnapshot>("agent_accounts_snapshot"),
        ]);
        if (generation !== epoch) return;
        for (const view of views.values()) {
          const task = tasks.tasks.find((task) => task.taskId === view.taskId);
          if (task) view.accept(task);
          view.update({ accounts, loaded: true, missing: !task });
        }
        updatePoll();
      } catch (cause) {
        if (generation === epoch)
          for (const view of views.values())
            view.update({ error: errorMessage(cause) });
      }
    } while (refreshAgain && generation === epoch);
  })().finally(() => {
    refreshPromise = undefined;
    if (refreshAgain && retained.length && stop) void refresh();
  });
  return refreshPromise;
}
function updatePoll() {
  const active = retained.some((tab) => {
    const task = views.get(tab.taskId)?.state.task;
    return task && isRunning(task.state);
  });
  if (active && !poll) poll = setInterval(() => void refresh(), 1000);
  if (!active && poll) {
    clearInterval(poll);
    poll = undefined;
  }
}
function connect() {
  if (subscribed || !retained.length) return;
  subscribed = true;
  const generation = ++epoch;
  void listen<{ revision: number; taskId?: string }>(
    "agent-runtime-changed",
    () => {
      if (generation === epoch) void refresh();
    },
  )
    .then((unlisten) => {
      if (generation !== epoch) {
        unlisten();
        return;
      }
      stop = unlisten;
      void refresh();
    })
    .catch((cause) => {
      if (generation === epoch) {
        subscribed = false;
        for (const view of views.values())
          view.update({ error: errorMessage(cause) });
      }
    });
}
export function getTaskView(taskId: string) {
  let view = views.get(taskId);
  if (!view) {
    view = new TaskView(taskId);
    views.set(taskId, view);
  }
  const connected = !!stop;
  connect();
  if (connected && !view.state.loaded) void refresh();
  return view;
}
export function retainAgentTasks(session?: Session) {
  retained = agentTaskTabs(session);
  const ids = new Set(retained.map((tab) => tab.taskId));
  for (const id of ids) getTaskView(id);
  const released: string[] = [];
  for (const [id, view] of views)
    if (!ids.has(id)) {
      if (view.closing) released.push(id);
      views.delete(id);
    }
  if (released.length)
    void api("agent_task_close_release", { taskIds: released }).catch(() => {});
  if (!ids.size) {
    ++epoch;
    subscribed = false;
    stop?.();
    stop = undefined;
  }
  updatePoll();
}
export function hasActiveAgentTasks(panelIds?: ReadonlySet<string>) {
  return finalTaskIds(retained, panelIds).some((id) => {
    const task = views.get(id)?.state.task;
    return task && isRunning(task.state);
  });
}
export interface TaskCloseLease {
  release: () => Promise<void>;
}
export async function closeAgentTaskViews(
  panelIds: ReadonlySet<string>,
): Promise<TaskCloseLease> {
  const ids = finalTaskIds(retained, panelIds);
  if (!ids.length) return { release: async () => {} };
  const entries = ids.map(getTaskView);
  if (entries.some((entry) => entry.closing))
    throw new Error("These task views are already closing.");
  const owner = Symbol("Task view close");
  for (const entry of entries) {
    entry.cancelContinuations();
    entry.closing = owner;
    entry.update({});
  }
  const lease = {
    release: async () => {
      const pending = entries.filter(
        (entry) => views.get(entry.taskId) === entry && entry.closing === owner,
      );
      if (!pending.length) return;
      await api("agent_task_close_release", {
        taskIds: pending.map((entry) => entry.taskId),
      });
      for (const entry of pending) {
        entry.closing = undefined;
        entry.update({});
      }
    },
  };
  try {
    await api("agent_task_prepare_close", { taskIds: ids });
    await refresh();
    return lease;
  } catch (cause) {
    await lease.release();
    throw cause;
  }
}

export function setTaskPublicationPaused(paused: boolean) {
  publicationPaused = paused;
  for (const view of views.values()) view.update({});
}
