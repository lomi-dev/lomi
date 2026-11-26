import { isCliAgent, type AgentUsageTarget } from "./agent-usage.ts";

// Chat panes showing a CLI provider, keyed by tab id. Their usage is read as if
// that CLI ran in a terminal; pid 0 tells native code there is no process.
const chats = new Map<string, string>();
const listeners = new Set<() => void>();

export function registerChatUsage(tabId: string, cli: string | null) {
  if (!cli || !isCliAgent(cli)) return () => {};
  chats.set(tabId, cli);
  listeners.forEach((listener) => listener());
  return () => {
    chats.delete(tabId);
    listeners.forEach((listener) => listener());
  };
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
