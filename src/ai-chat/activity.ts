import type { Activity } from "./types.ts";

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

/** Completed activities beyond `limit` collapse; running, failed and pending stay. */
export function collapseGroup(
  group: readonly Activity[],
  limit = 3,
): { collapsed: Activity[]; visible: Activity[] } {
  const done = group.filter(
    (item) => item.status === "completed" && !isPending(item),
  );
  if (done.length <= limit) return { collapsed: [], visible: [...group] };
  return {
    collapsed: done,
    visible: group.filter(
      (item) => item.status !== "completed" || isPending(item),
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
