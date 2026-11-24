import type { CliAgent } from "../cli-agents.ts";

export type ApiKind = "openai" | "anthropic" | "google" | "openai-compatible";
export interface Header {
  name: string;
  value: string;
}
export interface ModelEntry {
  id: string;
  source: "fetched" | "manual";
  enabled: boolean;
  favorite: boolean;
}
export interface ApiProvider {
  instanceId: string;
  kind: ApiKind;
  label: string;
  accentColor: string | null;
  enabled: boolean;
  baseUrl: string;
  headers: Header[];
  models: ModelEntry[];
  apiKeySet: boolean;
}
export interface CliProvider {
  instanceId: string;
  cli: CliAgent;
  label: string;
  enabled: boolean;
  models: ModelEntry[];
}
export interface ChatDefaults {
  instanceId: string | null;
  model: string | null;
  effort: string | null;
  fallback: { instanceId: string; model: string; effort: string | null } | null;
  permissions: Permissions;
  workspace: Workspace;
}
export type Permissions = "read-only" | "edit" | "full";
export type Workspace = "project" | "private";
export interface ChatBehavior {
  sendKey: "enter" | "mod-enter";
  systemInstructions: string;
  maxOutputTokens: number | null;
}
export interface ChatSettings {
  version: number;
  revision: number;
  apiProviders: ApiProvider[];
  cliProviders: CliProvider[];
  defaults: ChatDefaults;
  behavior: ChatBehavior;
  /** CLI ids already auto-added or dismissed; never re-added. */
  autoDetected: string[];
}
/** Result of `ai_cli_status`; signedIn null means unknown. */
export interface CliStatus {
  cli: CliAgent;
  binaryPath: string | null;
  version: string | null;
  signedIn: boolean | null;
  /** Subscription plan name, e.g. "Max 5x". */
  plan: string | null;
  error: string | null;
}
export interface CliModel {
  id: string;
  label: string;
  efforts: string[];
}
export type MessageState =
  | "committed"
  | "dispatching"
  | "streaming"
  | "completed"
  | "stopped"
  | "failed"
  | "interrupted";
export interface ThreadSummary {
  threadId: string;
  project: string;
  kind: "api" | "cli";
  instanceId: string;
  model: string;
  title: string;
  /** Working directory of a CLI thread; null for API threads. */
  cwd?: string | null;
  revision: number;
  createdMs: number;
  updatedMs: number;
  streaming: boolean;
  effort: string | null;
  permissions: Permissions;
  workspace: Workspace;
}
export interface Attachment {
  id: string;
  name: string;
  mime: string;
  kind: "image" | "text" | "pdf";
  size: number;
}
export type ActivityKind =
  | "reasoning"
  | "read"
  | "edit"
  | "search"
  | "execute"
  | "fetch"
  | "agent"
  | "other";
export type ActivityStatus = "running" | "completed" | "failed";
export interface ActivityDiff {
  path: string;
  before: string | null;
  after: string;
}
export type ApprovalState = "pending" | "allowed" | "denied" | "expired";
export type ApprovalDecision = "allow" | "allow_session" | "deny";
export interface Approval {
  requestId: string;
  state: ApprovalState;
  canRemember: boolean;
}
export interface Activity {
  id: string;
  kind: ActivityKind;
  title: string;
  status: ActivityStatus;
  /** UTF-16 offset into the message content where it happened. */
  at: number;
  detail: string | null;
  diff: ActivityDiff | null;
  /** Absent when no permission prompt happened. */
  approval?: Approval;
}
export interface Usage {
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  costUsd: number | null;
}
export type ChangeStatus = "added" | "modified" | "deleted" | "renamed";
export interface FileChange {
  path: string;
  status: ChangeStatus;
  additions: number;
  deletions: number;
  binary: boolean;
}
export interface TurnChanges {
  files: FileChange[];
  undone: boolean;
}
export interface TurnUndoResult {
  restored: string[];
  skipped: string[];
}
export interface ThreadMessage {
  messageId: string;
  seq: number;
  role: "user" | "assistant";
  content: string;
  state: MessageState;
  instanceId: string;
  model: string;
  streamSeq: number;
  stopReason: string | null;
  error: string | null;
  createdMs: number;
  attachments: Attachment[];
  /** Missing on data stored before activity existed. */
  activity?: Activity[];
  usage?: Usage | null;
  /** Files the agent changed in this turn; absent when none were captured. */
  changes?: TurnChanges | null;
}
export interface ThreadSnapshot {
  thread: ThreadSummary;
  messages: ThreadMessage[];
}
export interface ThreadSendAccepted {
  thread: ThreadSummary;
  user: ThreadMessage;
  assistant: ThreadMessage;
}
export type ThreadStreamEvent =
  | { type: "delta"; messageId: string; seq: number; text: string }
  | { type: "activity"; messageId: string; activity: Activity }
  | { type: "final"; thread: ThreadSummary; message: ThreadMessage };
export type ProviderKind = ApiKind | "cli";
export interface ProviderEntry {
  instanceId: string;
  kind: ProviderKind;
  cli: CliAgent | null;
  label: string;
  enabled: boolean;
  available: boolean;
  unavailableReason: string | null;
  statusLine: string;
  versionLabel: string | null;
  /** Pickable model ids: enabled, favorites first. Empty for CLIs (loaded at runtime). */
  models: string[];
  /** Stored model preferences in list order. */
  modelPrefs: ModelEntry[];
  cliStatus?: CliStatus;
  api?: ApiProvider;
}
/** One rendered message. */
export interface ChatItem {
  id: string;
  role: "user" | "assistant";
  text: string;
  streaming: boolean;
  /** Failure notice shown under the message. */
  error: string | null;
  /** The reply was cut short by the user. */
  stopped: boolean;
  activity: Activity[];
  usage: Usage | null;
  changes: TurnChanges | null;
  attachments: Attachment[];
  /** Text of the user message that produced a failed reply. */
  resend: string | null;
}
