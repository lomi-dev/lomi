import { panes, tabTitle } from "./model.ts";
import type { Layout, Pane, Session, ShellProfile } from "./model.ts";

export interface RemoteWorkspaceStatus {
  id: string;
  name: string;
  shared: boolean;
  message: string | null;
}
export interface RemoteState {
  qualified: boolean;
  enabled: boolean;
  message: string | null;
  workspaces: RemoteWorkspaceStatus[];
}
export type RemoteLayout =
  | { type: "terminal"; paneId: string }
  | {
      type: "split";
      axis: "horizontal" | "vertical";
      ratio: number;
      first: RemoteLayout;
      second: RemoteLayout;
    };
export interface RemoteTab {
  id: string;
  title: string;
  layout: RemoteLayout;
}
export interface WorkspaceProjection {
  id: string;
  name: string;
  tabs?: RemoteTab[];
  terminals: { paneId: string; sessionId: string | null; title: string }[];
}
export interface LocalWorkspaceTerminalContext {
  workspaceId: string;
  workspaceName: string;
  rootPath: string;
  desktopTabId: string;
  tabName: string;
}
function terminalLayout(layout: Layout | undefined): RemoteLayout | undefined {
  if (!layout) return undefined;
  if (layout.type === "terminal")
    return { type: "terminal", paneId: layout.id };
  if (layout.type !== "split") return undefined;
  const first = terminalLayout(layout.first);
  const second = terminalLayout(layout.second);
  if (!first) return second;
  if (!second) return first;
  return {
    type: "split",
    axis: layout.axis,
    ratio:
      Number.isFinite(layout.ratio) && layout.ratio > 0 && layout.ratio < 1
        ? layout.ratio
        : 0.5,
    first,
    second,
  };
}
export function isAgentTerminalState(state: {
  staged?: boolean;
  agentControlled?: boolean;
  titleCli?: boolean;
}) {
  return Boolean(state.staged || state.agentControlled || state.titleCli);
}
export function workspaceTerminals(
  session: Session | undefined,
  isAgentTerminal: (paneId: string) => boolean = () => false,
) {
  return (
    session?.projects.flatMap((project) =>
      project.workspaces.map((workspace) => ({
        id: workspace.id,
        name: workspace.name,
        rootPath: project.path,
        tabs: workspace.tabs.flatMap((tab) => {
          if (tab.type !== "terminal") return [];
          const layout = terminalLayout(
            filterAgentTerminals(tab.layout, isAgentTerminal),
          );
          return layout ? [{ id: tab.id, title: tabTitle(tab), layout }] : [];
        }),
        terminals: workspace.tabs.flatMap((tab) =>
          tab.type === "terminal"
            ? panes(tab.layout)
                .filter((pane) => !isAgentTerminal(pane.id))
                .map((pane) => ({
                  pane,
                  profileId: pane.profileId ?? tab.profileId,
                  title: tabTitle(tab),
                  desktopTabId: tab.id,
                  tabName: tabTitle(tab),
                }))
            : [],
        ),
      })),
    ) ?? []
  );
}
function filterAgentTerminals(
  layout: Layout,
  isAgentTerminal: (paneId: string) => boolean,
): Layout | undefined {
  if (layout.type === "terminal")
    return isAgentTerminal(layout.id) ? undefined : layout;
  if (layout.type !== "split") return undefined;
  const first = filterAgentTerminals(layout.first, isAgentTerminal);
  const second = filterAgentTerminals(layout.second, isAgentTerminal);
  if (!first) return second;
  if (!second) return first;
  return { ...layout, first, second };
}

/** One publisher owns the native epoch and never publishes a captured domain after awaits. */
export class WorkspacePublisher {
  private epoch?: string;
  private revision = 0;
  private dirty = false;
  private stopped = false;
  private pending?: Promise<void>;
  private last = "";
  readonly desired = new Set<string>();
  private readonly ports: {
    session: () => Session | undefined;
    profiles: () => ShellProfile[];
    sessionId: (paneId: string) => string | null;
    isAgentTerminal: (paneId: string) => boolean;
    start: (
      pane: Pane,
      profile: ShellProfile,
      workspace: LocalWorkspaceTerminalContext,
    ) => Promise<void>;
    begin: () => Promise<{ epoch: string }>;
    sync: (
      epoch: string,
      revision: number,
      workspaces: WorkspaceProjection[],
    ) => Promise<void>;
  };
  constructor(ports: WorkspacePublisher["ports"]) {
    this.ports = ports;
  }
  stop() {
    this.stopped = true;
  }
  notify() {
    this.dirty = true;
    if (!this.pending) {
      this.pending = this.drain().finally(() => {
        this.pending = undefined;
      });
    }
    return this.pending;
  }
  private async drain() {
    if (!this.epoch) this.epoch = (await this.ports.begin()).epoch;
    while (this.dirty && !this.stopped) {
      this.dirty = false;
      const projection = workspaceTerminals(
        this.ports.session(),
        this.ports.isAgentTerminal,
      ).map((workspace) => ({
        id: workspace.id,
        name: workspace.name,
        tabs: workspace.tabs,
        terminals: workspace.terminals.map(({ pane, title }) => ({
          paneId: pane.id,
          sessionId: this.ports.sessionId(pane.id),
          title,
        })),
      }));
      const key = JSON.stringify(projection);
      if (key !== this.last) {
        await this.ports.sync(this.epoch, ++this.revision, projection);
        this.last = key;
      }
      if (this.stopped) return;
      // Read current domain before each start: deletion, moves, and unshare can happen during a start.
      const candidates = workspaceTerminals(
        this.ports.session(),
        this.ports.isAgentTerminal,
      )
        .filter((w) => this.desired.has(w.id))
        .flatMap((w) =>
          w.terminals.map((t) => ({
            ...t,
            workspaceId: w.id,
            workspaceName: w.name,
            rootPath: w.rootPath,
          })),
        );
      for (const candidate of candidates) {
        if (this.stopped) return;
        const latest = workspaceTerminals(
          this.ports.session(),
          this.ports.isAgentTerminal,
        ).find((w) => w.id === candidate.workspaceId);
        const terminal = latest?.terminals.find(
          (t) => t.pane.id === candidate.pane.id,
        );
        if (!this.desired.has(candidate.workspaceId) || !terminal) continue;
        const profile = this.ports
          .profiles()
          .find((p) => p.id === terminal.profileId);
        if (!profile)
          throw new Error("The terminal shell profile is unavailable.");
        const before = this.ports.sessionId(terminal.pane.id);
        await this.ports.start(terminal.pane, profile, {
          workspaceId: candidate.workspaceId,
          workspaceName: candidate.workspaceName,
          rootPath: candidate.rootPath,
          desktopTabId: terminal.desktopTabId,
          tabName: terminal.tabName,
        });
        if (this.ports.sessionId(terminal.pane.id) !== before)
          this.dirty = true;
      }
    }
  }
}
