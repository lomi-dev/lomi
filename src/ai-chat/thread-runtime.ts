import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "../api";
import { t } from "../i18n";
import type {
  ChatItem,
  ThreadMessage,
  ThreadSendAccepted,
  ThreadSnapshot,
  ThreadStreamEvent,
  ThreadSummary,
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
  };
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
    let snapshot = this.state.snapshot;
    for (const event of events) {
      if (event.type === "delta") {
        const next = new Map(live);
        next.set(event.messageId, [
          ...(next.get(event.messageId) ?? []),
          { seq: event.seq, text: event.text },
        ]);
        live = next;
      } else {
        const next = new Map(live);
        next.delete(event.message.messageId);
        live = next;
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
    this.update({ live, snapshot });
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

/** Displayed API text: stored content plus live chunks newer than it. */
export function apiMessages(
  messages: readonly ThreadMessage[],
  live: ReadonlyMap<string, readonly LiveChunk[]>,
): ChatItem[] {
  const items: ChatItem[] = [];
  let userText = "";
  for (const message of messages) {
    if (message.role === "user") {
      userText = message.content;
      items.push({
        id: message.messageId,
        role: "user",
        text: message.content,
        streaming: false,
        error: null,
        stopped: false,
        tools: [],
        attachments: message.attachments,
        resend: null,
      });
      continue;
    }
    const chunks = (live.get(message.messageId) ?? [])
      .filter((chunk) => chunk.seq > message.streamSeq)
      .sort((a, b) => a.seq - b.seq)
      .filter((chunk, index, all) => chunk.seq !== all[index - 1]?.seq);
    const failed =
      message.state === "failed" || message.state === "interrupted";
    items.push({
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
      tools: [],
      attachments: [],
      resend: failed ? userText : null,
    });
  }
  return items;
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

/** Drafts survive tab remounts but not app restarts. */
const drafts = new Map<string, string>();
export const getDraft = (tabId: string) => drafts.get(tabId) ?? "";
export function setDraft(tabId: string, value: string) {
  if (value) drafts.set(tabId, value);
  else drafts.delete(tabId);
}

export const threadTitle = (text: string) =>
  [...(text.trim().split(/\r?\n/)[0] || "New chat")].slice(0, 60).join("");
