import type { Attachment } from "./types.ts";

export interface QueuedMessage {
  /** Also the send operation id, so a retried send stays idempotent. */
  id: string;
  text: string;
  attachments: Attachment[];
}

export type QueueAction = "send" | "pause" | "none";

export const enqueue = (
  queue: readonly QueuedMessage[],
  item: QueuedMessage,
) => [...queue, item];

export const remove = (queue: readonly QueuedMessage[], id: string) =>
  queue.filter((item) => item.id !== id);

/**
 * Entry to move back into the composer. Files cannot return to the composer,
 * and a non-empty composer would be overwritten; both refuse.
 */
export function editable(
  queue: readonly QueuedMessage[],
  id: string,
  composerEmpty: boolean,
) {
  const item = queue.find((entry) => entry.id === id);
  return item && composerEmpty && !item.attachments.length ? item : undefined;
}

/**
 * What to do with the queue head after a state change. A reply the user
 * stopped pauses the queue; any other idle state (completed, failed,
 * interrupted) drains it. A paused queue only moves via an explicit action.
 */
export function decide(input: {
  wasRunning: boolean;
  running: boolean;
  /** State of the last assistant message. */
  lastState: string | undefined;
  queued: number;
  paused: boolean;
}): QueueAction {
  if (input.running || !input.queued || input.paused) return "none";
  return input.wasRunning && input.lastState === "stopped" ? "pause" : "send";
}
