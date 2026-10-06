import type { CliAgent } from "../cli-agents";
export interface AccountRecovery {
  state: "ownership_unknown" | "effects_review_required";
  reason: string;
  operationIds: string[];
  recoverable: boolean;
  requiresVerifiedBootChange: boolean;
}
export interface AccountInstance {
  accountId: string;
  cli: CliAgent;
  label: string;
  enabled: boolean;
  revision: number;
  authRevision: number;
  authState: string;
  availabilityReason: string | null;
  acceptedVersion: string | null;
  recovery?: AccountRecovery | null;
}
export interface Capability {
  cli: CliAgent;
  accountTerminal: boolean;
  managedExecution: boolean;
  versions: string[];
  crossAccountNativeResume: boolean;
  reviewedTransfer: boolean;
  stopAndContinueQualified: boolean;
  stopAndContinueReason: string | null;
  reason: string;
}
export interface AccountsSnapshot {
  schema: 1;
  revision: number;
  accounts: AccountInstance[];
  capabilities: Capability[];
}
export type TaskState =
  | "idle"
  | "starting"
  | "running"
  | "stopping"
  | "stopped"
  | "completed"
  | "prepared"
  | "recovery_required"
  | "delivery_uncertain"
  | "archived";
export interface Attempt {
  attemptId: string;
  operationId: string;
  accountId: string;
  authRevision: number;
  generation: number;
  input: string;
  continuationMethod: string;
  state: string;
  output: string;
  nativeRef: string | null;
  version: string | null;
  effectsState: string;
}
export interface HistoryRecord {
  sequence: number;
  attemptId: string;
  accountId: string;
  authRevision: number;
  kind: string;
  state: string;
  content: unknown;
}
export interface HistoryGrant {
  accountId: string;
  authRevision: number;
  revision: number;
}
export interface SwitchOperation {
  operationId: string;
  sourceAttemptId: string | null;
  accountId: string;
  authRevision: number;
  historyRevision: number;
  mode: "next_turn" | "stop_and_continue" | "reviewed_transfer";
  phase: string;
  continuationMethod: string;
  reason: string | null;
  coverage: number;
  budgetBytes: number;
  contextDigest: string | null;
  stopSupervisionQualified: boolean;
}
export interface TransferReview {
  digest: string;
  bytes: number;
  taskId: string;
  accountId: string;
  authRevision: number;
  coverage: number;
  sourceAttemptId: string;
}
export interface Task {
  taskId: string;
  cwd: string;
  title: string;
  cli: CliAgent | null;
  availabilityReason?: string | null;
  model: string;
  reasoningEffort: string | null;
  revision: number;
  historyRevision: number;
  generation: number;
  state: TaskState;
  nextAccountId: string;
  activeAccountId: string | null;
  activeAttemptId: string | null;
  statusMessage: string;
  attempts: Attempt[];
  history: HistoryRecord[];
  grants: HistoryGrant[];
  switches: SwitchOperation[];
}
export interface TasksSnapshot {
  schema: 1;
  revision: number;
  tasks: Task[];
}
export interface PendingPermission {
  taskId: string;
  attemptId: string;
  generation: number;
  approvalToken: string;
  projectRoot: string;
  permission: {
    requestId: unknown;
    sessionId: string;
    turnId: string;
    toolId: string | null;
    choices: unknown[];
    raw: unknown;
  };
}
