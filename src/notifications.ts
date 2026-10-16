import { cliNames } from "./cli-agents.ts";
import type { CliAgent } from "./cli-agents.ts";

export interface InboxNotification {
  id: string;
  kind: "attention" | "finished" | "info";
  title: string;
  body: string;
  createdAt: number;
  read: boolean;
  agent?: CliAgent | null;
  source?: ApplicationNotificationSource | null;
  action?: ApplicationNotificationAction | null;
}

export type ApplicationNotificationSource = "app" | "android" | "mcp";
export type ApplicationNotificationAction =
  | "recover-session"
  | "retry-mcp-startup"
  | "open-agent-control-settings"
  | "open-android-settings";

export function notificationAgent(item: InboxNotification): CliAgent | null {
  if (item.source) return null;
  if (item.agent != null)
    return Object.hasOwn(cliNames, item.agent) ? item.agent : null;
  if (
    item.title === "Claude Code needs your input" ||
    item.title === "Claude Code finished responding"
  )
    return "claude";
  return null;
}

export function notificationSource(item: InboxNotification) {
  if (item.source)
    return `Notification from ${{ app: "Lomi", android: "Android", mcp: "MCP" }[item.source]}`;
  const agent = notificationAgent(item);
  return `Notification from ${agent ? cliNames[agent] : "terminal"}`;
}

export function notificationKindLabel(item: InboxNotification) {
  return item.kind === "attention"
    ? "Needs attention"
    : item.kind === "info"
      ? "Information"
      : "Finished";
}

export function notificationRelativeTime(createdAt: number, now = Date.now()) {
  if (!Number.isFinite(createdAt) || !Number.isFinite(now))
    return "Unknown time";
  const seconds = Math.max(0, Math.floor((now - createdAt) / 1000));
  if (seconds < 60) return "Just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

export interface NotificationSnapshot {
  revision: number;
  items: InboxNotification[];
}

// Equal revisions describe the same canonical state. Keeping the existing value
// also prevents a delayed response from replacing a snapshot received by event.
export function newestNotificationSnapshot(
  current: NotificationSnapshot | null,
  incoming: NotificationSnapshot,
): NotificationSnapshot {
  return !current || incoming.revision > current.revision ? incoming : current;
}

export function unreadNotificationCount(snapshot: NotificationSnapshot | null) {
  return snapshot?.items.filter((item) => !item.read).length ?? 0;
}

export function notificationBadge(count: number) {
  return count > 9 ? "9+" : String(count);
}

let lastError: string | null = null;
const errorListeners = new Set<(message: string) => void>();
export function reportNotificationError(message: string) {
  lastError = message;
  for (const listener of errorListeners) listener(message);
}
export function subscribeNotificationErrors(
  listener: (message: string) => void,
) {
  errorListeners.add(listener);
  if (lastError) listener(lastError);
  return () => {
    errorListeners.delete(listener);
  };
}
export function clearNotificationError() {
  lastError = null;
}
