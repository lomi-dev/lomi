import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "../api";
import { t } from "../i18n";
import {
  liveText,
  mergeActivity,
  mergedHasMore,
  mergeMessages,
  seedLive,
  upsertActivity,
} from "./activity";
import { enqueue, remove, type QueuedMessage } from "./send-queue";
import type {
  ApprovalDecision,
  Activity,
  Attachment,
  ChatItem,
  ErrorKind,
  Permissions,
  ThreadMessage,
  ThreadSendAccepted,
  ThreadSnapshot,
  ThreadStreamEvent,
  ThreadSummary,
  TurnUndoResult,
} from "./types";

/** Streamed text of one message: appended deltas and the last seq seen. */
export interface LiveChunk {
  seq: number;
  text: string;
  /** Seq and text offset of each delta, so stored content can be skipped exactly. */
  marks: readonly (readonly [seq: number, start: number])[];
  /** Stored content when a seeded attach began; `text` continues it. */
  base?: string;
}
/** A rendered message plus the typed failure class (see ThreadMessage). */
export type ChatItemView = ChatItem & {
  errorKind?: ErrorKind | null;
  retryAtMs?: number | null;
  /** Files of the user message that produced a failed reply. */
  resendAttachments?: Attachment[];
};
/** ChatItem.error placeholder; ThreadMessages translates it at render time. */
export const INTERRUPTED_ERROR = "\0interrupted";
export interface ThreadViewState {
  snapshot?: ThreadSnapshot;
  loaded: boolean;
  missing: boolean;
  error: string;
  busy: boolean;
  /** An older page is being fetched. */
  loadingEarlier: boolean;
  live: ReadonlyMap<string, LiveChunk>;
  /** Activity received over the stream, cleared when the message is final. */
  liveActivity: ReadonlyMap<string, readonly Activity[]>;
  /**
   * Follow-ups typed while a reply runs. Kept in memory with the view, so it
   * survives remounts and tab switches but not an app restart.
   */
  queued: readonly QueuedMessage[];
  /** The queue waits for an explicit "Send next" (stopped reply or failed send). */
  paused: boolean;
}
interface ChangedPayload {
  scope: string;
  threadId: string | null;
}

const changed = new Set<(payload: ChangedPayload) => void>();
let unlisten: Promise<() => void> | undefined;
/** Shares one ai-chat-changed subscription between all views and lists. */
function onChanged(listener: (payload: ChangedPayload) => void) {
  changed.add(listener);
  unlisten ??= listen<ChangedPayload>("ai-chat-changed", (event) => {
    for (const item of changed) item(event.payload);
  });
  return () => {
    changed.delete(listener);
    if (!changed.size && unlisten) {
      void unlisten.then((fn) => fn()).catch(() => {});
      unlisten = undefined;
    }
  };
}

const active = (message: ThreadMessage) =>
  message.state === "dispatching" || message.state === "streaming";

class ThreadView {
  state: ThreadViewState = {
    loaded: false,
    missing: false,
    error: "",
    busy: false,
    loadingEarlier: false,
    live: new Map(),
    liveActivity: new Map(),
    queued: [],
    paused: false,
  };
  /** Whether a reply was running at the last queue check. */
  wasRunning = false;
  private listeners = new Set<() => void>();
  private release?: () => void;
  private poll?: ReturnType<typeof setInterval>;
  /** Messages whose stream this webview owns through a channel. */
  private channels = new Set<string>();
  private sending = false;
  private frame = 0;
  private queue: ThreadStreamEvent[] = [];
  constructor(readonly threadId: string) {}
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    if (this.listeners.size === 1) {
      this.release = onChanged((payload) => {
        if (
          payload.scope === "threads" &&
          (!payload.threadId || payload.threadId === this.threadId)
        )
          void this.refresh();
      });
      void this.refresh();
      this.syncPoll();
    }
    return () => {
      this.listeners.delete(listener);
      if (!this.listeners.size) {
        this.release?.();
        clearInterval(this.poll);
        this.poll = undefined;
        this.tried.clear();
        this.retried.clear();
      }
    };
  };
  getSnapshot = () => this.state;
  private update(change: Partial<ThreadViewState>) {
    this.state = { ...this.state, ...change };
    this.syncPoll();
    for (const listener of this.listeners) listener();
  }
  /** Message ids already tried with `threadAttach` since the last poll tick. */
  private tried = new Set<string>();
  /** Messages whose first empty attach already got its quick second try. */
  private retried = new Set<string>();
  /**
   * Attaches to a turn that streams without a channel of this webview (e.g.
   * after reopening); a slow poll remains only while attaching is impossible.
   */
  private syncPoll() {
    const orphan = this.state.snapshot?.messages.find(
      (message) => active(message) && !this.channels.has(message.messageId),
    );
    if (orphan && this.listeners.size) {
      if (!this.tried.has(orphan.messageId)) this.attach(orphan);
      this.poll ??= setInterval(() => {
        this.tried.clear();
        void this.refresh();
      }, 5000);
    } else if (!orphan && this.poll) {
      clearInterval(this.poll);
      this.poll = undefined;
    }
  }
  private attach(message: ThreadMessage) {
    this.tried.add(message.messageId);
    const onEvent = new Channel<ThreadStreamEvent>();
    // Hold events until the returned state is seeded, so seq filtering applies.
    let early: ThreadStreamEvent[] | null = [];
    onEvent.onmessage = (event) => {
      if (early) {
        early.push(event);
        return;
      }
      this.queue.push(event);
      this.frame ||= requestAnimationFrame(this.flush);
    };
    threadAttach(this.threadId, onEvent).then(
      (live) => {
        if (!live) {
          // The turn may not be attachable yet; try once soon, then the slow poll.
          if (this.retried.has(message.messageId)) return;
          this.retried.add(message.messageId);
          setTimeout(() => {
            if (!this.listeners.size) return;
            this.tried.delete(message.messageId);
            void this.refresh().then(() => this.syncPoll());
          }, 300);
          return;
        }
        this.channels.add(live.messageId);
        const stored = this.state.snapshot?.messages.find(
          (item) => item.messageId === live.messageId,
        );
        const chunk = stored && seedLive(stored, live);
        const liveMap = new Map(this.state.live);
        if (chunk) liveMap.set(live.messageId, chunk);
        const activityMap = new Map(this.state.liveActivity);
        let activity = activityMap.get(live.messageId) ?? [];
        for (const item of live.activity)
          activity = upsertActivity(activity, item);
        activityMap.set(live.messageId, activity);
        this.update({ live: liveMap, liveActivity: activityMap });
        this.queue.push(...early!);
        early = null;
        if (this.queue.length) this.flush();
      },
      () => {},
    );
  }
  private accept(snapshot: ThreadSnapshot) {
    const current = this.state.snapshot;
    if (current && snapshot.thread.revision < current.thread.revision) return;
    // Fresh windows cover only the newest messages; keep loaded older pages.
    const merged: ThreadSnapshot = {
      ...snapshot,
      messages: mergeMessages(current?.messages ?? [], snapshot.messages),
      hasMore: mergedHasMore(current, snapshot),
    };
    this.update({ snapshot: merged, loaded: true, missing: false, error: "" });
  }
  /** Prepends the next page of older messages. */
  async loadEarlier() {
    const current = this.state.snapshot;
    const first = current?.messages[0];
    if (!current || !first || !current.hasMore || this.state.loadingEarlier)
      return;
    this.update({ loadingEarlier: true });
    try {
      const page = await threadMessagesBefore(this.threadId, first.seq, 50);
      const latest = this.state.snapshot ?? current;
      const oldest = latest.messages[0]?.seq ?? Infinity;
      this.update({
        snapshot: {
          ...latest,
          messages: [
            ...page.messages.filter((message) => message.seq < oldest),
            ...latest.messages,
          ],
          hasMore: page.hasMore,
        },
      });
    } catch (cause) {
      this.update({ error: errorMessage(cause) });
    } finally {
      this.update({ loadingEarlier: false });
    }
  }
  seed(thread: ThreadSummary) {
    if (!this.state.snapshot)
      this.update({ snapshot: { thread, messages: [] }, loaded: true });
  }
  async refresh() {
    try {
      this.accept(await threadSnapshotLimited(this.threadId, 100));
    } catch (cause) {
      const message = errorMessage(cause);
      if (/not found|no such|deleted/i.test(message))
        this.update({ missing: true, loaded: true, error: "" });
      else this.update({ error: message, loaded: true });
    }
  }
  private flush = () => {
    this.frame = 0;
    const events = this.queue.splice(0);
    let live = this.state.live;
    let liveActivity = this.state.liveActivity;
    let snapshot = this.state.snapshot;
    for (const event of events) {
      if (event.type === "delta") {
        const known = live.get(event.messageId);
        // Replayed or out-of-order deltas are dropped by seq.
        if (known && event.seq <= known.seq) continue;
        const next = new Map(live);
        const text = known?.text ?? "";
        next.set(event.messageId, {
          seq: event.seq,
          text: text + event.text,
          marks: [...(known?.marks ?? []), [event.seq, text.length]],
          base: known?.base,
        });
        live = next;
      } else if (event.type === "activity") {
        const next = new Map(liveActivity);
        next.set(
          event.messageId,
          upsertActivity(next.get(event.messageId) ?? [], event.activity),
        );
        liveActivity = next;
      } else {
        const next = new Map(live);
        next.delete(event.message.messageId);
        live = next;
        const nextActivity = new Map(liveActivity);
        nextActivity.delete(event.message.messageId);
        liveActivity = nextActivity;
        this.channels.delete(event.message.messageId);
        if (snapshot)
          snapshot = {
            ...snapshot,
            thread: event.thread,
            messages: snapshot.messages.map((message) =>
              message.messageId === event.message.messageId
                ? event.message
                : message,
            ),
          };
      }
    }
    this.update({ live, liveActivity, snapshot });
  };
  async send(request: {
    operationId: string;
    expectedRevision: number;
    instanceId: string;
    model: string;
    text: string;
    attachments: string[];
  }) {
    if (this.sending) throw new Error(t("A message is already being sent."));
    this.sending = true;
    this.update({ busy: true, error: "" });
    const onEvent = new Channel<ThreadStreamEvent>();
    // Events can arrive before the accepted reply; hold them until the
    // accepted messages are in the snapshot so Final has a row to replace.
    let early: ThreadStreamEvent[] | null = [];
    onEvent.onmessage = (event) => {
      if (early) {
        early.push(event);
        return;
      }
      this.queue.push(event);
      this.frame ||= requestAnimationFrame(this.flush);
    };
    try {
      const accepted = await api<ThreadSendAccepted>("ai_thread_send", {
        request: { ...request, threadId: this.threadId },
        onEvent,
      });
      this.channels.add(accepted.assistant.messageId);
      const kept = (this.state.snapshot?.messages ?? []).filter(
        (message) =>
          message.messageId !== accepted.user.messageId &&
          message.messageId !== accepted.assistant.messageId,
      );
      this.accept({
        thread: accepted.thread,
        messages: [...kept, accepted.user, accepted.assistant],
      });
      this.queue.push(...early);
      early = null;
      if (this.queue.length) this.flush();
    } catch (cause) {
      this.update({ error: errorMessage(cause) });
      throw cause;
    } finally {
      this.sending = false;
      this.update({ busy: false });
    }
  }
  enqueue(item: QueuedMessage) {
    this.update({ queued: enqueue(this.state.queued, item) });
  }
  dequeue(id: string) {
    this.update({ queued: remove(this.state.queued, id) });
  }
  setPaused(paused: boolean) {
    if (paused !== this.state.paused) this.update({ paused });
  }
  async stop() {
    try {
      await api("ai_thread_stop", { threadId: this.threadId });
    } catch (cause) {
      this.update({ error: errorMessage(cause) });
    }
  }
}

const views = new Map<string, ThreadView>();
export function getThreadView(threadId: string) {
  let view = views.get(threadId);
  if (!view) {
    view = new ThreadView(threadId);
    views.set(threadId, view);
  }
  return view;
}
const emptyState: ThreadViewState = {
  loaded: true,
  missing: false,
  error: "",
  busy: false,
  loadingEarlier: false,
  live: new Map(),
  liveActivity: new Map(),
  queued: [],
  paused: false,
};
const emptyView = {
  subscribe: () => () => {},
  getSnapshot: () => emptyState,
};
/** Thread view state; an unsaved new chat (null id) is empty. */
export function useThreadView(threadId: string | null) {
  const view = threadId ? getThreadView(threadId) : undefined;
  const state = useSyncExternalStore(
    (view ?? emptyView).subscribe,
    (view ?? emptyView).getSnapshot,
  );
  return { view, state };
}

interface CachedItem {
  chunks: LiveChunk | undefined;
  extra: readonly Activity[] | undefined;
  resend: string;
  files: Attachment[];
  item: ChatItemView;
}
/** Keeps ChatItem identity while a message and its live inputs are unchanged. */
const itemCache = new WeakMap<ThreadMessage, CachedItem>();

/** Displayed API text: stored content plus live chunks newer than it. */
export function apiMessages(
  messages: readonly ThreadMessage[],
  live: ReadonlyMap<string, LiveChunk>,
  liveActivity: ReadonlyMap<string, readonly Activity[]> = new Map(),
): ChatItemView[] {
  const items: ChatItemView[] = [];
  let userText = "";
  let userFiles: Attachment[] = [];
  for (const message of messages) {
    const user = message.role === "user";
    if (user) {
      userText = message.content;
      userFiles = message.attachments;
    }
    const liveChunks = user ? undefined : live.get(message.messageId);
    const extra = user ? undefined : liveActivity.get(message.messageId);
    const hit = itemCache.get(message);
    if (
      hit &&
      hit.chunks === liveChunks &&
      hit.extra === extra &&
      hit.resend === userText &&
      hit.files === userFiles
    ) {
      items.push(hit.item);
      continue;
    }
    const item = user
      ? userItem(message)
      : assistantItem(message, liveChunks, extra ?? [], userText, userFiles);
    itemCache.set(message, {
      chunks: liveChunks,
      extra,
      resend: userText,
      files: userFiles,
      item,
    });
    items.push(item);
  }
  return items;
}

const userItem = (message: ThreadMessage): ChatItem => ({
  id: message.messageId,
  role: "user",
  text: message.content,
  streaming: false,
  error: null,
  stopped: false,
  activity: [],
  usage: null,
  changes: null,
  attachments: message.attachments,
  resend: null,
});

function assistantItem(
  message: ThreadMessage,
  liveChunks: LiveChunk | undefined,
  extra: readonly Activity[],
  userText: string,
  userFiles: Attachment[],
): ChatItemView {
  const failed = message.state === "failed" || message.state === "interrupted";
  return {
    id: message.messageId,
    role: "assistant",
    text: liveText(message, liveChunks),
    streaming: active(message),
    error: failed
      ? (message.error ?? INTERRUPTED_ERROR)
      : message.state === "completed"
        ? message.error
        : null,
    errorKind: failed ? message.errorKind : null,
    retryAtMs: failed ? message.retryAtMs : null,
    stopped: message.state === "stopped",
    activity: mergeActivity(message.activity ?? [], extra),
    usage: message.usage ?? null,
    changes: message.changes ?? null,
    attachments: [],
    resend: failed ? userText : null,
    resendAttachments: failed ? userFiles : undefined,
  };
}

export const listThreads = (project: string, workspaceId: string) =>
  api<ThreadSummary[]>("ai_threads_list", { project, workspaceId });

export function useThreads(project: string, workspaceId: string) {
  const [threads, setThreads] = useState<ThreadSummary[]>([]);
  const [error, setError] = useState("");
  const reload = useCallback(() => {
    listThreads(project, workspaceId).then(
      (next) => {
        setThreads(next);
        setError("");
      },
      (cause) => setError(errorMessage(cause)),
    );
  }, [project, workspaceId]);
  useEffect(() => {
    reload();
    return onChanged((payload) => {
      if (payload.scope === "threads") reload();
    });
  }, [reload]);
  return { threads, error, reload };
}

export const createThread = (request: {
  operationId: string;
  project: string;
  /** Workspace whose history lists the thread. */
  workspaceId: string;
  instanceId: string;
  model: string;
  title: string;
  /** Reasoning effort of a CLI thread. */
  effort?: string | null;
  /** Working directory for CLI threads. */
  cwd?: string;
}) => api<ThreadSummary>("ai_thread_create", { request });

export const updateThread = (
  thread: Pick<ThreadSummary, "threadId" | "revision">,
  change: {
    title?: string;
    instanceId?: string;
    model?: string;
    /** Omitted keeps it; null clears it. */
    effort?: string | null;
    permissions?: Permissions;
  },
) =>
  api<ThreadSummary>("ai_thread_update", {
    request: {
      threadId: thread.threadId,
      expectedRevision: thread.revision,
      title: change.title ?? null,
      instanceId: change.instanceId ?? null,
      model: change.model ?? null,
      ...(change.effort !== undefined && { effort: change.effort }),
      ...(change.permissions && { permissions: change.permissions }),
    },
  });

export const renameThread = (
  thread: Pick<ThreadSummary, "threadId" | "revision">,
  title: string,
) => updateThread(thread, { title });

export const removeThread = (
  thread: Pick<ThreadSummary, "threadId" | "revision">,
) =>
  api<void>("ai_thread_delete", {
    request: { threadId: thread.threadId, expectedRevision: thread.revision },
  });

export const respondToApproval = (
  threadId: string,
  requestId: string,
  decision: ApprovalDecision,
) => api<void>("ai_thread_respond", { threadId, requestId, decision });

export const turnDiff = (threadId: string, messageId: string, path: string) =>
  api<string>("ai_turn_diff", { threadId, messageId, path });

export const undoTurn = (threadId: string, messageId: string) =>
  api<TurnUndoResult>("ai_turn_undo", { threadId, messageId });

/** Drafts live in memory and are mirrored to localStorage, debounced. */
const drafts = new Map<string, string>();
const draftPrefix = "lomi.chat.draft.";
const maxStoredDraft = 64 * 1024;
const draftTimers = new Map<string, ReturnType<typeof setTimeout>>();
export function getDraft(tabId: string) {
  const known = drafts.get(tabId);
  if (known !== undefined) return known;
  try {
    const stored = localStorage.getItem(draftPrefix + tabId) ?? "";
    if (stored) drafts.set(tabId, stored);
    return stored;
  } catch {
    return "";
  }
}
/** Forgets the draft of a closed chat tab, including its stored copy. */
export function deleteDraft(tabId: string) {
  drafts.delete(tabId);
  clearTimeout(draftTimers.get(tabId));
  draftTimers.delete(tabId);
  try {
    localStorage.removeItem(draftPrefix + tabId);
  } catch {
    // Storage may be unavailable.
  }
}
export function setDraft(tabId: string, value: string) {
  if (value) drafts.set(tabId, value);
  else drafts.delete(tabId);
  clearTimeout(draftTimers.get(tabId));
  draftTimers.set(
    tabId,
    setTimeout(() => writeDraft(tabId), 300),
  );
}
function writeDraft(tabId: string) {
  draftTimers.delete(tabId);
  const value = drafts.get(tabId);
  try {
    if (value && value.length <= maxStoredDraft)
      localStorage.setItem(draftPrefix + tabId, value);
    else localStorage.removeItem(draftPrefix + tabId);
  } catch {
    // Storage may be unavailable or full; the in-memory draft still works.
  }
}
// Writes pending debounced drafts before the page goes away.
if (typeof window !== "undefined")
  window.addEventListener("pagehide", () => {
    for (const [tabId, timer] of draftTimers) {
      clearTimeout(timer);
      writeDraft(tabId);
    }
  });

export const threadTitle = (text: string) =>
  [...(text.trim().split(/\r?\n/)[0] || "New chat")].slice(0, 60).join("");

import type {
  MessagePage,
  ThreadLive,
  ThreadSnapshot as LimitedSnapshot,
  ThreadStatus,
  TurnUndoResult as FilesUndoResult,
} from "./types";

/** The newest `limit` messages of a thread (`hasMore` tells if older ones exist). */
export const threadSnapshotLimited = (threadId: string, limit: number) =>
  api<LimitedSnapshot>("ai_thread_snapshot", { threadId, limit });

/** Up to `limit` messages older than `beforeSeq`, oldest first. */
export const threadMessagesBefore = (
  threadId: string,
  beforeSeq: number,
  limit: number,
) =>
  api<MessagePage>("ai_thread_messages_before", { threadId, beforeSeq, limit });

/** Subscribe to the stream first, then call this and drop deltas with `seq <= live.seq`. */
export const threadLive = (threadId: string) =>
  api<ThreadLive | null>("ai_thread_live", { threadId });

export const threadsStatus = (project: string, workspaceId: string) =>
  api<ThreadStatus[]>("ai_threads_status", { project, workspaceId });

/** Fires on every run-state change of any thread (idle, running, waiting). */
export const listenThreadStatus = (cb: (status: ThreadStatus) => void) =>
  listen<ThreadStatus>("ai-thread-status", (e) => cb(e.payload));

/** Undoes only the given files (each must be recorded for the turn). */
export const undoTurnFiles = (
  threadId: string,
  messageId: string,
  paths: string[],
) => api<FilesUndoResult>("ai_turn_undo", { threadId, messageId, paths });

import type { ThreadCommand, ThreadStreamEvent as StreamEvent } from "./types";

/**
 * Subscribes `onEvent` to the running turn of a thread and returns its state so
 * far (drop deltas with `seq <= live.seq`), or null when no turn runs.
 */
export const threadAttach = (threadId: string, onEvent: Channel<StreamEvent>) =>
  api<ThreadLive | null>("ai_thread_attach", { threadId, onEvent });

/** Stable error of `steerThread` when the message must be queued instead. */
export const steerUnsupported = "steer_unsupported";

/** Stable error of `steerThread` when delivery could not be confirmed. */
export const steerUnknown = "steer_unknown";

/** Adds a message to the running turn; rejects with `steerUnsupported` to queue it. */
export const steerThread = (threadId: string, text: string) =>
  api<void>("ai_thread_steer", { threadId, text });

/** Answers a pending question by item id; null declines it. */
export const answerQuestion = (
  threadId: string,
  requestId: string,
  answers: Record<string, string[]> | null,
) => api<void>("ai_thread_answer", { threadId, requestId, answers });

/** Slash commands of the thread's CLI; empty before its session started. */
export const threadCommands = (threadId: string) =>
  api<ThreadCommand[]>("ai_thread_commands", { threadId });
