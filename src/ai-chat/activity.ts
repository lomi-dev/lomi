import type { Activity, ThreadLive, ThreadMessage } from "./types.ts";
import type { LiveChunk } from "./thread-runtime.ts";

export type Part =
  { type: "text"; text: string } | { type: "group"; activities: Activity[] };

/** Stored activities with live ones: live wins by id, first appearance orders. */
export function mergeActivity(
  stored: readonly Activity[],
  live: readonly Activity[],
): Activity[] {
  if (!live.length) return [...stored];
  const byId = new Map<string, Activity>();
  for (const item of stored) byId.set(item.id, item);
  for (const item of live) byId.set(item.id, item);
  return [...byId.values()];
}

/** Inserts or replaces by id, keeping the first position. */
export function upsertActivity(
  list: readonly Activity[],
  next: Activity,
): Activity[] {
  const index = list.findIndex((item) => item.id === next.id);
  if (index < 0) return [...list, next];
  const copy = [...list];
  copy[index] = next;
  return copy;
}

/**
 * Splits text at activity offsets (UTF-16 indexes, clamped) into text parts
 * and groups of consecutive same-offset activities. Always ends with a text
 * part, possibly empty, so a streaming caret has a place to render.
 */
export function interleave(
  text: string,
  activities: readonly Activity[],
): Part[] {
  const place = (at: number) => {
    let index = Math.min(Math.max(Math.trunc(at) || 0, 0), text.length);
    // Never cut a surrogate pair in half.
    if (
      index > 0 &&
      index < text.length &&
      /[\uD800-\uDBFF]/.test(text[index - 1]) &&
      /[\uDC00-\uDFFF]/.test(text[index])
    )
      index += 1;
    return index;
  };
  const placed = activities
    .map((activity, order) => ({ activity, order, at: place(activity.at) }))
    .sort((a, b) => a.at - b.at || a.order - b.order);
  const parts: Part[] = [];
  let cursor = 0;
  for (let i = 0; i < placed.length;) {
    const at = placed[i].at;
    if (at > cursor) parts.push({ type: "text", text: text.slice(cursor, at) });
    cursor = at;
    const group: Activity[] = [];
    while (i < placed.length && placed[i].at === at)
      group.push(placed[i++].activity);
    parts.push({ type: "group", activities: group });
  }
  parts.push({ type: "text", text: text.slice(cursor) });
  return parts;
}

export const isPending = (item: Activity) => item.approval?.state === "pending";

/** True while any activity waits for the user's permission decision. */
export const hasPendingApproval = (activities: readonly Activity[]) =>
  activities.some(isPending);

export const isPendingQuestion = (item: Activity) =>
  item.question?.state === "pending";

/**
 * Live text of an attached turn. `live.content` is the full accumulated text
 * while the stored message may hold only a prefix, so only the remainder is
 * kept and one mark at `live.seq` lets later stored snapshots skip it exactly.
 * Null when the stored content is not a prefix (the next snapshot decides).
 */
export function seedLive(
  message: Pick<ThreadMessage, "content" | "streamSeq">,
  live: Pick<ThreadLive, "seq" | "content">,
): LiveChunk | null {
  if (!live.content.startsWith(message.content)) return null;
  const text = live.content.slice(message.content.length);
  return {
    seq: live.seq,
    text,
    marks: live.seq > message.streamSeq && text ? [[live.seq, 0]] : [],
    base: message.content,
  };
}

/** Displayed text: stored content plus the live text it does not contain yet. */
export function liveText(
  message: Pick<ThreadMessage, "content" | "streamSeq">,
  chunk: LiveChunk | undefined,
): string {
  if (!chunk) return message.content;
  // A seeded chunk is a continuation of `base`; a newer snapshot may already hold part of it.
  if (chunk.base !== undefined) {
    const full = chunk.base + chunk.text;
    if (full.startsWith(message.content)) return full;
  }
  // Only deltas newer than the stored content; earlier ones are already in it.
  const start = chunk.marks.find(([seq]) => seq > message.streamSeq)?.[1];
  return message.content + (start === undefined ? "" : chunk.text.slice(start));
}

/** Completed activities beyond `limit` collapse; running, failed and pending stay. */
export function collapseGroup(
  group: readonly Activity[],
  limit = 3,
): { collapsed: Activity[]; visible: Activity[] } {
  const done = group.filter(
    (item) =>
      item.status === "completed" &&
      !isPending(item) &&
      !isPendingQuestion(item) &&
      item.kind !== "notice",
  );
  if (done.length <= limit) return { collapsed: [], visible: [...group] };
  return {
    collapsed: done,
    visible: group.filter(
      (item) =>
        item.status !== "completed" ||
        isPending(item) ||
        isPendingQuestion(item) ||
        item.kind === "notice",
    ),
  };
}

export interface DiffLine {
  sign: "+" | "-";
  text: string;
}

const lines = (text: string) => (text === "" ? [] : text.split("\n"));

/** Changed middle between common leading and trailing lines. */
export function lineDiff(before: string | null, after: string): DiffLine[] {
  const a = before === null ? [] : lines(before);
  const b = lines(after);
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start])
    start += 1;
  let end = 0;
  while (
    end < a.length - start &&
    end < b.length - start &&
    a[a.length - 1 - end] === b[b.length - 1 - end]
  )
    end += 1;
  return [
    ...a
      .slice(start, a.length - end)
      .map((text) => ({ sign: "-" as const, text })),
    ...b
      .slice(start, b.length - end)
      .map((text) => ({ sign: "+" as const, text })),
  ];
}

/** Shows a path under the thread cwd as relative. */
export function relativeTitle(title: string, cwd: string): string {
  const base = cwd.replace(/\/+$/, "");
  return base && title.startsWith(base + "/")
    ? title.slice(base.length + 1)
    : title;
}

/** Compact count such as 12.3K in the given locale. */
export function compactNumber(value: number, locale?: string): string {
  return new Intl.NumberFormat(locale, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}

/**
 * Replaces the newest window with a fresh one while keeping older pages that
 * were loaded earlier (anything before the fresh window's first seq).
 */
export function mergeMessages(
  current: readonly ThreadMessage[],
  next: readonly ThreadMessage[],
): ThreadMessage[] {
  const first = next[0]?.seq;
  if (first === undefined) return [...next];
  return [...current.filter((message) => message.seq < first), ...next];
}

/** Whether older messages exist after merging a fresh window into `current`. */
export function mergedHasMore(
  current:
    { messages: readonly ThreadMessage[]; hasMore?: boolean } | undefined,
  next: { messages: readonly ThreadMessage[]; hasMore?: boolean },
): boolean | undefined {
  const reaches =
    !current?.messages.length ||
    !next.messages.length ||
    next.messages[0].seq <= current.messages[0].seq;
  return reaches && next.hasMore !== undefined
    ? next.hasMore
    : current?.hasMore;
}

/** Remaining time as m:ss or h:mm:ss, rounded up to whole seconds. */
export function formatCountdown(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}
