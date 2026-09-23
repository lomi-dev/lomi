import {
  active,
  layoutPanes,
  mergeTabs,
  movePane,
  moveTab,
  transferTab,
  type Session,
  type Workspace,
} from "./model.ts";

export type PanelMove =
  | { type: "reorder_tab"; tabId: string; beforeTabId: string | null }
  | {
      type: "transfer_tab";
      tabId: string;
      targetWorkspaceId: string;
      beforeTabId: string | null;
    }
  | {
      type: "dock_tab";
      tabId: string;
      targetTabId: string;
      side: "left" | "right" | "top" | "bottom";
    }
  | {
      type: "move_pane";
      panelId: string;
      targetPanelId: string;
      side: "left" | "right" | "top" | "bottom";
    };
export interface PanelMoveIdentity {
  panelId: string;
  tabId: string;
  kind: string;
  terminalSessionId: string | null;
  browserGeneration: string | null;
  androidDeviceId: string | null;
}
export interface PanelMoveDestination {
  workspaceId: string;
  tabOrder: string[];
  panels: PanelMoveIdentity[];
}
export function panelMoveIdentities(
  workspace: Workspace,
  terminal: (id: string) => string | null,
): PanelMoveIdentity[] {
  return workspace.tabs
    .flatMap((t) =>
      (t.type === "terminal" ? layoutPanes(t.layout) : [t]).map((p) => ({
        panelId: p.id,
        tabId: t.id,
        kind: p.type,
        terminalSessionId: p.type === "terminal" ? terminal(p.id) : null,
        browserGeneration:
          p.type === "browser" ? (p.automation?.generation ?? null) : null,
        androidDeviceId: p.type === "android" ? p.deviceId : null,
      })),
    )
    .sort((a, b) =>
      a.panelId < b.panelId ? -1 : a.panelId > b.panelId ? 1 : 0,
    );
}
export function moveAgentPanel(
  session: Session,
  projectId: string,
  workspaceId: string,
  movement: PanelMove,
  size: { width: number; height: number } | null,
): Session {
  const project = session.projects.find((p) => p.id === projectId);
  const workspace = project?.workspaces.find((w) => w.id === workspaceId);
  if (!workspace) throw new Error("TARGET_NOT_FOUND");
  if (movement.type === "transfer_tab") {
    const moved = transferTab(
      session,
      projectId,
      workspaceId,
      movement.targetWorkspaceId,
      movement.tabId,
      movement.beforeTabId,
    );
    if (moved === session) throw new Error("TARGET_NOT_FOUND");
    return moved;
  }
  let updated = workspace;
  if (movement.type === "reorder_tab") {
    if (
      !workspace.tabs.some((t) => t.id === movement.tabId) ||
      (movement.beforeTabId !== null &&
        !workspace.tabs.some((t) => t.id === movement.beforeTabId)) ||
      movement.beforeTabId === movement.tabId
    )
      throw new Error("TARGET_NOT_FOUND");
    updated = moveTab(workspace, movement.tabId, movement.beforeTabId);
  } else {
    const selected = active(session);
    if (
      !size ||
      size.width <= 0 ||
      size.height <= 0 ||
      selected?.workspace.id !== workspaceId ||
      selected.tab.type !== "terminal"
    )
      throw new Error("PANEL_NOT_RENDERABLE");
    if (movement.type === "dock_tab") {
      if (selected.tab.id !== movement.targetTabId)
        throw new Error("PANEL_NOT_RENDERABLE");
      updated = mergeTabs(
        workspace,
        movement.tabId,
        movement.targetTabId,
        movement.side,
        size,
      );
    } else {
      const layout = movePane(
        selected.tab.layout,
        movement.panelId,
        movement.targetPanelId,
        movement.side,
        size,
      );
      if (layout !== selected.tab.layout)
        updated = {
          ...workspace,
          tabs: workspace.tabs.map((t) =>
            t.id === selected.tab.id
              ? { ...selected.tab, layout, activePaneId: movement.panelId }
              : t,
          ),
        };
    }
    if (updated === workspace) throw new Error("PANEL_NOT_RENDERABLE");
  }
  return updated === workspace
    ? session
    : {
        ...session,
        projects: session.projects.map((p) =>
          p.id === projectId
            ? {
                ...p,
                workspaces: p.workspaces.map((w) =>
                  w.id === workspaceId ? updated : w,
                ),
              }
            : p,
        ),
      };
}
