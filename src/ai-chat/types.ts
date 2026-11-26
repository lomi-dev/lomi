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
  /**
   * System note; `title` is a stable key: "context_reset" | "checkpoint_unavailable" |
   * "context_compacted" | "steer" (`detail` is then the message sent mid-turn).
   */
  | "notice"
  /** A question for the user; see `Activity.question`. */
  | "question"
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
export type QuestionState = "pending" | "answered" | "declined" | "expired";
export interface QuestionOption {
  label: string;
  description: string;
}
export interface QuestionItem {
  /** Key of the answer (Claude: the question text). */
  id: string;
  header: string;
  question: string;
  options: QuestionOption[];
  multiSelect: boolean;
  /** Free text is accepted besides the options. */
  allowOther: boolean;
  /** The answer is a secret; it is never stored. */
  secret?: boolean;
}
export interface Question {
  requestId: string;
  items: QuestionItem[];
  state: QuestionState;
  /** Chosen labels or free text by item id; secret answers are never stored. */
  answers?: Record<string, string[]>;
}
/** A slash command of the thread's CLI, sent as plain message text. */
export interface ThreadCommand {
  name: string;
  description: string;
  argumentHint: string;
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
  /** Present on `question` activities. */
  question?: Question;
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
  /** The turn's own edit activities touched this path. */
  agent: boolean;
  /** Restored by a selective undo. */
  undone: boolean;
}
export interface TurnChanges {
  files: FileChange[];
  undone: boolean;
}
/** Result of `ai_thread_rewind`; `context` tells how the agent regains the earlier conversation. */
export interface RewindReport {
  thread: ThreadSummary;
  restored: string[];
  skipped: string[];
  context: "native" | "fresh" | "replay" | "api";
}
export interface TurnUndoResult {
  restored: string[];
  skipped: string[];
}
export type ErrorKind =
  | "auth"
  | "rate_limit"
  | "network"
  | "missing_cli"
  | "context_length"
  | "other";
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
  /** Class of `error` on a failed turn. */
  errorKind?: ErrorKind | null;
  /** Epoch ms at which a rate limit resets, when reported. */
  retryAtMs?: number | null;
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
  /** Older messages exist beyond a `limit`ed snapshot. */
  hasMore?: boolean;
}
export interface MessagePage {
  /** Oldest first. */
  messages: ThreadMessage[];
  hasMore: boolean;
}
/** In-memory state of the streaming reply; drop deltas with `seq <= live.seq`. */
export interface ThreadLive {
  messageId: string;
  seq: number;
  content: string;
  activity: Activity[];
}
export type RunState = "idle" | "running" | "waiting";
export interface ThreadStatus {
  threadId: string;
  state: RunState;
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
  /** Epoch ms the message was stored. */
  createdMs: number;
}
