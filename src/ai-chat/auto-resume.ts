import { useSyncExternalStore } from "react";

/** A failed reply armed to be sent again once its rate limit resets. */
export interface AutoResume {
  messageId: string;
  retryAtMs: number;
  text: string;
}

/** Thread ids whose armed entry has reached its reset time. */
export const due = (
  entries: ReadonlyMap<string, AutoResume>,
  now: number,
): string[] =>
  [...entries].filter(([, e]) => e.retryAtMs <= now).map(([id]) => id);

/** Copy of the entries without `threadId`. */
export function without(
  entries: ReadonlyMap<string, AutoResume>,
  threadId: string,
): ReadonlyMap<string, AutoResume> {
  const next = new Map(entries);
  next.delete(threadId);
  return next;
}

let entries: ReadonlyMap<string, AutoResume> = new Map();
let timer: ReturnType<typeof setInterval> | undefined;
const listeners = new Set<() => void>();

function set(next: ReadonlyMap<string, AutoResume>) {
  entries = next;
  if (!entries.size && timer) {
    clearInterval(timer);
    timer = undefined;
  } else if (entries.size && !timer) timer = setInterval(tick, 1000);
  listeners.forEach((listener) => listener());
}

async function tick() {
  // Lazy import keeps the pure helpers above loadable without Tauri.
  const { getThreadView } = await import("./thread-runtime");
  for (const threadId of due(entries, Date.now())) {
    const entry = entries.get(threadId);
    const view = getThreadView(threadId);
    const snapshot = view.state.snapshot;
    const last = snapshot?.messages.at(-1);
    if (!entry || !snapshot) continue;
    // The failed reply is no longer the end of the thread: nothing to resume.
    if (last?.messageId !== entry.messageId || last.state !== "failed") {
      set(without(entries, threadId));
      continue;
    }
    if (
      view.state.busy ||
      snapshot.messages.some(
        (m) => m.state === "streaming" || m.state === "dispatching",
      )
    )
      continue; // Busy: stay armed, retry next tick.
    set(without(entries, threadId));
    // The send path needs only the view, so it works without a mounted pane.
    view
      .send({
        operationId: crypto.randomUUID(),
        expectedRevision: snapshot.thread.revision,
        instanceId: snapshot.thread.instanceId,
        model: snapshot.thread.model,
        text: entry.text,
        attachments: [],
      })
      .catch(() => {});
  }
}

export const arm = (threadId: string, entry: AutoResume) =>
  set(new Map(entries).set(threadId, entry));
export const disarm = (threadId: string) => set(without(entries, threadId));

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => void listeners.delete(listener);
};

/** The armed entry of a thread, or undefined. */
export const useAutoResume = (threadId: string) =>
  useSyncExternalStore(subscribe, () => entries.get(threadId));
