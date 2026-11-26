import { isCliAgent, type AgentUsageTarget } from "./agent-usage.ts";

// Chat panes using a CLI provider, keyed by tab id. Their usage is read as if
// that CLI ran in a terminal; pid 0 tells native code there is no process.
// Entries outlive the pane's mount (only the active tab is mounted) and are
// removed when the chat switches to an API provider or leaves the session.
const chats = new Map<string, string>();
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((listener) => listener());

export function registerChatUsage(tabId: string, cli: string | null) {
  const next = cli && isCliAgent(cli) ? cli : undefined;
  if (chats.get(tabId) === next) return;
  if (next) chats.set(tabId, next);
  else chats.delete(tabId);
  notify();
}

/** Drops chats whose tab or pane id no longer exists. */
export function pruneChatUsage(live: ReadonlySet<string>) {
  let changed = false;
  for (const id of chats.keys())
    if (!live.has(id)) changed = chats.delete(id) || changed;
  if (changed) notify();
}

export function subscribeChatUsage(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** Chat targets for CLIs without a terminal target; a terminal one is preferred. */
export function withChatTargets(
  terminal: AgentUsageTarget[],
  registered: ReadonlyMap<string, string> = chats,
): AgentUsageTarget[] {
  const seen = new Set<string>(terminal.map(({ process }) => process.cli));
  const extra: AgentUsageTarget[] = [];
  for (const [tabId, cli] of [...registered].sort(([a], [b]) =>
    a.localeCompare(b),
  )) {
    if (seen.has(cli) || !isCliAgent(cli)) continue;
    seen.add(cli);
    extra.push({ id: `chat:${tabId}`, process: { cli, pid: 0 } });
  }
  return [...terminal, ...extra].sort((l, r) => l.id.localeCompare(r.id));
}
