import {
  active,
  removeWorkspace,
  type Layout,
  type LayoutPane,
  type Tab,
  type Session,
  type Workspace,
} from "./model.ts";

let conflictObserver: ((left: unknown, right: unknown) => void) | undefined;
// Native development fixtures install a bounded observer; production builds
// cannot register it and never disclose model snapshots through this hook.
export function observeAgentDomainConflicts(
  observer: (left: unknown, right: unknown) => void,
) {
  if (import.meta.env?.DEV) conflictObserver = observer;
}

export function sameAgentSession(
  left: Session | undefined,
  right: Session | undefined,
): boolean {
  const equal =
    left === right ||
    agentSessionSignature(left) === agentSessionSignature(right);
  if (!equal) conflictObserver?.(left, right);
  return equal;
}

function panelIdentity(panel: Tab | LayoutPane, closing: boolean): unknown {
  if (panel.type === "file") {
    const { position: _position, ...identity } = panel;
    if (!closing) return identity;
    const { title: _title, ...resource } = identity;
    return resource;
  }
  if (panel.type === "terminal" && "layout" in panel) {
    const identity = {
      ...panel,
      layout: layoutIdentity(panel.layout, closing),
    };
    if (!closing) return identity;
    const { title: _title, ...resource } = identity;
    return resource;
  }
  if (
    closing &&
    (panel.type === "browser" ||
      panel.type === "diff" ||
      panel.type === "commit")
  ) {
    const { title: _title, ...resource } = panel;
    return resource;
  }
  return panel;
}
function layoutIdentity(layout: Layout, closing: boolean): unknown {
  return layout.type === "split"
    ? {
        ...layout,
        first: layoutIdentity(layout.first, closing),
        second: layoutIdentity(layout.second, closing),
      }
    : panelIdentity(layout, closing);
}
function workspaceIdentity(workspace: Workspace, closing: boolean) {
  return {
    ...workspace,
    tabs: workspace.tabs.map((t) => panelIdentity(t, closing)),
  };
}
export function agentSessionSignature(
  session: Session | undefined,
): string | undefined {
  // Cursor/scroll persistence on editor detach does not invalidate a domain
  // operation. File identities, layout, selection and plugin state still do.
  return JSON.stringify(
    session && {
      ...session,
      projects: session.projects.map((p) => ({
        ...p,
        workspaces: p.workspaces.map((w) => workspaceIdentity(w, false)),
      })),
    },
  );
}
function closureIdentity(workspace: Workspace): string {
  // Only runtime titles and editor viewport positions may change after commit;
  // all paths, generations, approved names and layout children must match.
  return JSON.stringify(workspaceIdentity(workspace, true));
}

export function closeAgentWorkspace(
  before: Session,
  current: Session | undefined,
  projectId: string,
  workspaceId: string,
): Session {
  const oldProject = before.projects.find((p) => p.id === projectId);
  const project = current?.projects.find((p) => p.id === projectId);
  const previous = oldProject?.workspaces.find((w) => w.id === workspaceId);
  const workspace = project?.workspaces.find((w) => w.id === workspaceId);
  if (
    !current ||
    !previous ||
    !workspace ||
    project!.path !== oldProject!.path ||
    closureIdentity(previous) !== closureIdentity(workspace)
  )
    throw new Error("REVISION_CONFLICT");
  const removed = removeWorkspace(current, workspaceId);
  return active(current)?.workspace.id === workspaceId
    ? { ...removed, activeProjectId: null }
    : removed;
}

export function closeAgentProject(
  before: Session,
  current: Session | undefined,
  projectId: string,
): Session {
  const previous = before.projects.find((p) => p.id === projectId);
  const project = current?.projects.find((p) => p.id === projectId);
  if (
    !current ||
    !previous ||
    !project ||
    previous.path !== project.path ||
    previous.workspaces.length !== project.workspaces.length ||
    previous.workspaces.some(
      (w, i) => closureIdentity(w) !== closureIdentity(project.workspaces[i]),
    )
  )
    throw new Error("REVISION_CONFLICT");
  return {
    ...current,
    projects: current.projects.filter((p) => p.id !== projectId),
    activeProjectId:
      current.activeProjectId === projectId ? null : current.activeProjectId,
  };
}
