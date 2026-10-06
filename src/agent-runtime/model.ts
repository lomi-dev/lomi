import type { AgentTaskTab } from "../model.ts";
import type {
  AccountInstance,
  HistoryGrant,
  Task,
  TaskState,
} from "./types.ts";
export function isRunning(state: TaskState) {
  return state === "starting" || state === "running" || state === "stopping";
}
export function finalTaskIds(
  retained: readonly AgentTaskTab[],
  panelIds?: ReadonlySet<string>,
) {
  const ids = new Set(
    retained
      .filter((tab) => !panelIds || panelIds.has(tab.id))
      .map((tab) => tab.taskId),
  );
  for (const tab of retained)
    if (panelIds && !panelIds.has(tab.id)) ids.delete(tab.taskId);
  return [...ids];
}
export function hasHistoryGrant(
  grants: readonly HistoryGrant[],
  account: AccountInstance,
) {
  return grants.some(
    (grant) =>
      grant.accountId === account.accountId &&
      grant.authRevision === account.authRevision,
  );
}
export function executionRequest(
  task: Task,
  account: AccountInstance,
  operationId: string,
  text: string,
  continuing = false,
) {
  return {
    operationId,
    taskId: task.taskId,
    expectedRevision: task.revision,
    accountId: account.accountId,
    authRevision: account.authRevision,
    text,
    continue: continuing,
  };
}
export function acceptTask(current: Task | undefined, next: Task) {
  return !current || next.revision >= current.revision ? next : current;
}

export function continuationLabel(method: string) {
  return (
    (
      {
        fresh: "New native session",
        native_resume: "Native session resume",
        reviewed_transfer: "Continuation with reviewed history",
        handoff: "New session with task context",
        blocked: "Continuation unavailable",
      } as Record<string, string>
    )[method] ?? "Continuation awaiting qualification"
  );
}
