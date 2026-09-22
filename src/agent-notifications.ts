import { basename, panes, tabTitle } from "./model.ts";
import type { Session } from "./model.ts";

export type AgentSignal = "working" | "attention" | "finished";

export function parseAgentSignal(value: string): AgentSignal | null {
  const prefix = "notify;Lomi;claude;";
  if (!value.startsWith(prefix)) return null;
  const signal = value.slice(prefix.length);
  return signal === "working" || signal === "attention" || signal === "finished"
    ? signal
    : null;
}

export function notificationContext(
  session: Session | undefined,
  paneId: string,
): string | null {
  for (const project of session?.projects ?? [])
    for (const workspace of project.workspaces)
      for (const tab of workspace.tabs)
        if (
          tab.type === "terminal" &&
          panes(tab.layout).some((pane) => pane.id === paneId)
        )
          return [basename(project.path), workspace.name, tabTitle(tab)]
            .join(" · ")
            .replace(/[\x00-\x1f\x7f-\x9f]/g, "")
            .slice(0, 300);
  return null;
}

export function createAgentNotificationGate() {
  const recent = new Map<AgentSignal, number>();
  return (signal: AgentSignal, now = Date.now()): boolean => {
    if (signal === "working") return false;
    const previous = recent.get(signal);
    if (previous !== undefined && now >= previous && now - previous < 2_000)
      return false;
    recent.set(signal, now);
    return true;
  };
}

export interface AgentNotification {
  paneId: string;
  sessionId: string;
  kind: "attention" | "finished";
}
const listeners = new Set<(event: AgentNotification) => void>();
export function subscribeAgentNotifications(
  listener: (event: AgentNotification) => void,
) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function emitAgentNotification(event: AgentNotification) {
  for (const listener of listeners) listener(event);
}
