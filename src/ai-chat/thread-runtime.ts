import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "../api";
import { t } from "../i18n";
import { mergeActivity, upsertActivity } from "./activity";
import { enqueue, remove, type QueuedMessage } from "./send-queue";
import type {
  ApprovalDecision,
  Activity,
  ChatItem,
  Permissions,
  ThreadMessage,
  ThreadSendAccepted,
  ThreadSnapshot,
  ThreadStreamEvent,
  ThreadSummary,
  TurnUndoResult,
} from "./types";

export interface LiveChunk {
  seq: number;
  text: string;
}
export interface ThreadViewState {
  snapshot?: ThreadSnapshot;
  loaded: boolean;
  missing: boolean;
  error: string;
  busy: boolean;
  live: ReadonlyMap<string, readonly LiveChunk[]>;
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
      }
    };
  };
  getSnapshot = () => this.state;
  private update(change: Partial<ThreadViewState>) {
    this.state = { ...this.state, ...change };
    this.syncPoll();
    for (const listener of this.listeners) listener();
  }
  /** Polls while a message streams without a channel (e.g. after reopening). */
  private syncPoll() {
    const orphan = this.state.snapshot?.messages.some(
      (message) => active(message) && !this.channels.has(message.messageId),
    );
    if (orphan && this.listeners.size && !this.poll)
      this.poll = setInterval(() => void this.refresh(), 1000);
    else if (!orphan && this.poll) {
      clearInterval(this.poll);
      this.poll = undefined;
    }
  }
  private accept(snapshot: ThreadSnapshot) {
    const current = this.state.snapshot;
    if (current && snapshot.thread.revision < current.thread.revision) return;
    this.update({ snapshot, loaded: true, missing: false, error: "" });
  }
  seed(thread: ThreadSummary) {
    if (!this.state.snapshot)
      this.update({ snapshot: { thread, messages: [] }, loaded: true });
  }
  async refresh() {
    try {
      this.accept(
        await api<ThreadSnapshot>("ai_thread_snapshot", {
          threadId: this.threadId,
        }),
      );
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
        const next = new Map(live);
        next.set(event.messageId, [
          ...(next.get(event.messageId) ?? []),
          { seq: event.seq, text: event.text },
        ]);
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
  chunks: readonly LiveChunk[] | undefined;
  extra: readonly Activity[] | undefined;
  resend: string;
  item: ChatItem;
}
/** Keeps ChatItem identity while a message and its live inputs are unchanged. */
const itemCache = new WeakMap<ThreadMessage, CachedItem>();

/** Displayed API text: stored content plus live chunks newer than it. */
export function apiMessages(
  messages: readonly ThreadMessage[],
  live: ReadonlyMap<string, readonly LiveChunk[]>,
  liveActivity: ReadonlyMap<string, readonly Activity[]> = new Map(),
): ChatItem[] {
  const items: ChatItem[] = [];
  let userText = "";
  for (const message of messages) {
    const user = message.role === "user";
    if (user) userText = message.content;
    const liveChunks = user ? undefined : live.get(message.messageId);
    const extra = user ? undefined : liveActivity.get(message.messageId);
    const hit = itemCache.get(message);
    if (
      hit &&
      hit.chunks === liveChunks &&
      hit.extra === extra &&
      hit.resend === userText
    ) {
      items.push(hit.item);
      continue;
    }
    const item = user
      ? userItem(message)
      : assistantItem(message, liveChunks ?? [], extra ?? [], userText);
    itemCache.set(message, {
      chunks: liveChunks,
      extra,
      resend: userText,
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
  liveChunks: readonly LiveChunk[],
  extra: readonly Activity[],
  userText: string,
): ChatItem {
  const chunks = liveChunks
    .filter((chunk) => chunk.seq > message.streamSeq)
    .sort((a, b) => a.seq - b.seq)
    .filter((chunk, index, all) => chunk.seq !== all[index - 1]?.seq);
  const failed = message.state === "failed" || message.state === "interrupted";
  return {
    id: message.messageId,
    role: "assistant",
    text: message.content + chunks.map((chunk) => chunk.text).join(""),
    streaming: active(message),
    error: failed
      ? (message.error ??
        t("The response was interrupted; the partial reply was kept."))
      : message.state === "completed"
        ? message.error
        : null,
    stopped: message.state === "stopped",
    activity: mergeActivity(message.activity ?? [], extra),
    usage: message.usage ?? null,
    changes: message.changes ?? null,
    attachments: [],
    resend: failed ? userText : null,
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
