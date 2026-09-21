import type { PanelDescriptor } from "@lomi-dev/plugin-sdk";
import { jsonState, pluginId } from "./plugins/manifest.ts";
export type PluginPanel = PanelDescriptor;
import { restoreBrowserUrl } from "./browser-url.ts";

export interface ShellProfile {
  id: string;
  name: string;
  kind: string;
  program: string;
  distro: string | null;
  home: string;
}

export interface AppInfo {
  directory: string;
  home: string;
  platform: string;
  profiles: ShellProfile[];
}

export interface Pane {
  type: "terminal";
  id: string;
  cwd: string;
  profileId?: string;
}
export interface Split {
  type: "split";
  id: string;
  axis: "horizontal" | "vertical";
  ratio: number;
  first: Layout;
  second: Layout;
}
export type LayoutPane =
  Pane | FileTab | BrowserTab | AndroidTab | ChatTab | PluginPanel;
export type Layout = LayoutPane | Split;
export interface LayoutSize {
  width: number;
  height: number;
}
export interface LayoutBounds extends LayoutSize {
  left: number;
  top: number;
}
export const MIN_PANE_WIDTH = 240;
export const MIN_PANE_HEIGHT = 120;
export const SPLIT_DIVIDER_SIZE = 3;
export interface TerminalTab {
  type: "terminal";
  id: string;
  title: string;
  customTitle?: string;
  profileId: string;
  activePaneId: string;
  layout: Layout;
}
export interface CommitTab {
  type: "commit";
  id: string;
  title: string;
  customTitle?: string;
  root: string;
  commit: string;
}
export interface DiffTab {
  type: "diff";
  id: string;
  title: string;
  customTitle?: string;
  root: string;
  relative: string;
  staged: boolean;
}
export interface EditorPosition {
  anchor: number;
  head: number;
  scrollTop: number;
  scrollLeft: number;
}
export interface FileTab {
  type: "file";
  id: string;
  title: string;
  customTitle?: string;
  root: string;
  relative: string;
  untitled?: true;
  position?: EditorPosition;
  previewView?: FilePreviewView;
}
export type FilePreviewView = "editor" | "split" | "preview";
export interface BrowserTab {
  type: "browser";
  id: string;
  title: string;
  customTitle?: string;
  url: string;
}
export interface AndroidTab {
  type: "android";
  id: string;
  title: string;
  customTitle?: string;
  deviceId: string | null;
}
export const newAndroidTab = (
  deviceId: string | null = null,
  title = "Android",
): AndroidTab => ({ type: "android", id: newId(), title, deviceId });
export function androidTabs(session?: Session): AndroidTab[] {
  return (
    session?.projects.flatMap((project) =>
      project.workspaces.flatMap((workspace) =>
        workspace.tabs.flatMap((tab) =>
          tab.type === "terminal"
            ? layoutPanes(tab.layout).filter(
                (pane): pane is AndroidTab => pane.type === "android",
              )
            : tab.type === "android"
              ? [tab]
              : [],
        ),
      ),
    ) ?? []
  );
}
export function updateAndroid(
  session: Session,
  id: string,
  change: Partial<Pick<AndroidTab, "deviceId" | "title">>,
): Session {
  const apply = (layout: Layout): Layout =>
    layout.type === "split"
      ? { ...layout, first: apply(layout.first), second: apply(layout.second) }
      : layout.type === "android" && layout.id === id
        ? { ...layout, ...change }
        : layout;
  return {
    ...session,
    projects: session.projects.map((project) => ({
      ...project,
      workspaces: project.workspaces.map((workspace) => ({
        ...workspace,
        tabs: workspace.tabs.map((tab) =>
          tab.type === "terminal"
            ? { ...tab, layout: apply(tab.layout) }
            : tab.type === "android" && tab.id === id
              ? { ...tab, ...change }
              : tab,
        ),
      })),
    })),
  };
}
export interface ChatTab {
  type: "chat";
  id: string;
  title: string;
  customTitle?: string;
  conversationId: string;
}
export const newChatTab = (
  conversationId: string,
  title = "Chat AI",
): ChatTab => ({
  type: "chat",
  id: newId(),
  title,
  conversationId,
});
export function chatTabs(session?: Session): ChatTab[] {
  return (
    session?.projects.flatMap((p) =>
      p.workspaces.flatMap((w) =>
        w.tabs.flatMap((t) =>
          t.type === "terminal"
            ? layoutPanes(t.layout).filter(
                (p): p is ChatTab => p.type === "chat",
              )
            : t.type === "chat"
              ? [t]
              : [],
        ),
      ),
    ) ?? []
  );
}
export function updateChat(
  session: Session,
  id: string,
  change: Partial<Pick<ChatTab, "conversationId" | "title">>,
): Session {
  return {
    ...session,
    projects: session.projects.map((p) => ({
      ...p,
      workspaces: p.workspaces.map((w) => ({
        ...w,
        tabs: w.tabs.map((t) => {
          const apply = (p: Layout): Layout =>
            p.type === "split"
              ? { ...p, first: apply(p.first), second: apply(p.second) }
              : p.type === "chat" && p.id === id
                ? { ...p, ...change }
                : p;
          return t.type === "chat" && t.id === id
            ? { ...t, ...change }
            : t.type === "terminal"
              ? { ...t, layout: apply(t.layout) }
              : t;
        }),
      })),
    })),
  };
}
export const newBrowserTab = (url = "about:blank"): BrowserTab => ({
  type: "browser",
  id: newId(),
  title: "Browser",
  url: restoreBrowserUrl(url),
});
export type Tab =
  | TerminalTab
  | CommitTab
  | DiffTab
  | FileTab
  | BrowserTab
  | AndroidTab
  | ChatTab
  | PluginPanel;
export const tabTitle = (tab: Tab) => tab.customTitle ?? tab.title;
export type TabDropSide = "left" | "right" | "top" | "bottom";
export type TabCloseAction =
  "close" | "others" | "left" | "right" | "clean" | "all";
export interface Workspace {
  id: string;
  name: string;
  activeTabId: string;
  tabs: Tab[];
  pluginSidebars?: PluginPanel[];
}
export interface Project {
  id: string;
  path: string;
  activeWorkspaceId: string;
  workspaces: Workspace[];
}
export type SidebarPanel =
  "files" | "git" | "workspaces" | `${string}.${string}`;
export type SidebarSide = "left" | "right";
export interface Session {
  version: 3;
  activeProjectId: string | null;
  projects: Project[];
  sidebar: SidebarPanel | null;
  sidebarWidth: number;
  rightSidebar: SidebarPanel | null;
  rightSidebarWidth: number;
  sidebarSides: Record<SidebarPanel, SidebarSide>;
  terminalOverviewSide: SidebarSide;
}

export const newId = () => crypto.randomUUID();
export const basename = (path: string) =>
  path
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/)
    .pop() || path;
export const newPane = (cwd: string): Pane => ({
  type: "terminal",
  id: newId(),
  cwd,
});
export function newTab(
  cwd: string,
  profileId: string,
  title = "Terminal",
): TerminalTab {
  const layout = newPane(cwd);
  return {
    type: "terminal",
    id: newId(),
    title,
    profileId,
    layout,
    activePaneId: layout.id,
  };
}
export function newWorkspace(
  cwd: string,
  profileId: string,
  name = "Default",
): Workspace {
  const tab = newTab(cwd, profileId);
  return { id: newId(), name, tabs: [tab], activeTabId: tab.id };
}
export function newProject(path: string, profileId: string): Project {
  const workspace = newWorkspace(path, profileId);
  return {
    id: newId(),
    path,
    workspaces: [workspace],
    activeWorkspaceId: workspace.id,
  };
}
export function addWorkspace(
  session: Session,
  path: string,
  profileId: string,
  name: string,
): Session {
  const workspace = newWorkspace(path, profileId, name);
  const existing = session.projects.find((project) => project.path === path);
  const project: Project = {
    id: existing?.id ?? newId(),
    path,
    workspaces: [...(existing?.workspaces ?? []), workspace],
    activeWorkspaceId: workspace.id,
  };
  return {
    ...session,
    activeProjectId: project.id,
    projects: existing
      ? session.projects.map((item) =>
          item.id === project.id ? project : item,
        )
      : [...session.projects, project],
  };
}
export function removeWorkspace(session: Session, id: string): Session {
  const project = session.projects.find((project) =>
    project.workspaces.some((workspace) => workspace.id === id),
  );
  if (!project) return session;
  const workspaces = project.workspaces.filter(
    (workspace) => workspace.id !== id,
  );
  const projects = workspaces.length
    ? session.projects.map((candidate) =>
        candidate.id === project.id
          ? {
              ...project,
              workspaces,
              activeWorkspaceId:
                project.activeWorkspaceId === id
                  ? workspaces[0].id
                  : project.activeWorkspaceId,
            }
          : candidate,
      )
    : session.projects.filter((candidate) => candidate.id !== project.id);
  return {
    ...session,
    projects,
    activeProjectId:
      !workspaces.length && session.activeProjectId === project.id
        ? (projects[0]?.id ?? null)
        : session.activeProjectId,
  };
}
export function newSession(): Session {
  return {
    version: 3,
    projects: [],
    activeProjectId: null,
    sidebar: "files",
    sidebarWidth: 250,
    rightSidebar: null,
    rightSidebarWidth: 250,
    sidebarSides: { files: "left", git: "left", workspaces: "left" },
    terminalOverviewSide: "left",
  };
}
export function showSidebar(session: Session, panel: SidebarPanel): Session {
  const slot =
    session.sidebarSides[panel] === "left" ? "sidebar" : "rightSidebar";
  return { ...session, [slot]: panel };
}
export function toggleSidebar(session: Session, panel: SidebarPanel): Session {
  const slot =
    session.sidebarSides[panel] === "left" ? "sidebar" : "rightSidebar";
  return { ...session, [slot]: session[slot] === panel ? null : panel };
}
export function moveSidebar(
  session: Session,
  panel: SidebarPanel,
  side: SidebarSide,
): Session {
  return showSidebar(
    {
      ...session,
      sidebar: session.sidebar === panel ? null : session.sidebar,
      rightSidebar:
        session.rightSidebar === panel ? null : session.rightSidebar,
      sidebarSides: { ...session.sidebarSides, [panel]: side },
    },
    panel,
  );
}
export function layoutPanes(layout: Layout): LayoutPane[] {
  return layout.type === "split"
    ? [...layoutPanes(layout.first), ...layoutPanes(layout.second)]
    : [layout];
}
export function panes(layout: Layout): Pane[] {
  return layoutPanes(layout).filter(
    (pane): pane is Pane => pane.type === "terminal",
  );
}
export function filesInTab(tab: Tab): FileTab[] {
  return tab.type === "file"
    ? [tab]
    : tab.type === "terminal"
      ? layoutPanes(tab.layout).filter(
          (pane): pane is FileTab => pane.type === "file",
        )
      : [];
}
export function activePanel(
  tab: Tab,
): LayoutPane | CommitTab | DiffTab | undefined {
  return tab.type === "terminal"
    ? layoutPanes(tab.layout).find((pane) => pane.id === tab.activePaneId)
    : tab;
}
export function minimumLayoutSize(layout: Layout): LayoutSize {
  if (layout.type !== "split")
    return { width: MIN_PANE_WIDTH, height: MIN_PANE_HEIGHT };
  const first = minimumLayoutSize(layout.first);
  const second = minimumLayoutSize(layout.second);
  return layout.axis === "horizontal"
    ? {
        width: first.width + SPLIT_DIVIDER_SIZE + second.width,
        height: Math.max(first.height, second.height),
      }
    : {
        width: Math.max(first.width, second.width),
        height: first.height + SPLIT_DIVIDER_SIZE + second.height,
      };
}
export function layoutFits(layout: Layout, size: LayoutSize): boolean {
  const minimum = minimumLayoutSize(layout);
  return size.width >= minimum.width && size.height >= minimum.height;
}
export function splitGeometry(layout: Split, size: LayoutSize) {
  const dimension = layout.axis === "horizontal" ? "width" : "height";
  const available = Math.max(0, size[dimension] - SPLIT_DIVIDER_SIZE);
  const firstMinimum = minimumLayoutSize(layout.first)[dimension];
  const secondMinimum = minimumLayoutSize(layout.second)[dimension];
  const minRatio = available > 0 ? firstMinimum / available : 0.5;
  const maxRatio = available > 0 ? 1 - secondMinimum / available : 0.5;
  const ratio = Math.max(minRatio, Math.min(maxRatio, layout.ratio));
  return {
    ratio,
    minRatio,
    maxRatio,
    first: { ...size, [dimension]: available * ratio },
    second: { ...size, [dimension]: available * (1 - ratio) },
  };
}
export function layoutPositions(layout: Layout, size: LayoutSize) {
  const positions: { layout: Layout; bounds: LayoutBounds }[] = [];
  const visit = (layout: Layout, bounds: LayoutBounds) => {
    positions.push({ layout, bounds });
    if (layout.type !== "split") return;
    const geometry = splitGeometry(layout, bounds);
    visit(layout.first, { ...bounds, ...geometry.first });
    visit(layout.second, {
      ...bounds,
      ...geometry.second,
      ...(layout.axis === "horizontal"
        ? { left: bounds.left + geometry.first.width + SPLIT_DIVIDER_SIZE }
        : { top: bounds.top + geometry.first.height + SPLIT_DIVIDER_SIZE }),
    });
  };
  visit(layout, { width: size.width, height: size.height, left: 0, top: 0 });
  return positions;
}
function paneSize(
  layout: Layout,
  paneId: string,
  size: LayoutSize,
): LayoutSize | undefined {
  if (layout.type !== "split") return layout.id === paneId ? size : undefined;
  const geometry = splitGeometry(layout, size);
  return (
    paneSize(layout.first, paneId, geometry.first) ??
    paneSize(layout.second, paneId, geometry.second)
  );
}
export function canSplitPane(
  layout: Layout,
  paneId: string,
  axis: Split["axis"],
  size: LayoutSize,
): boolean {
  if (!layoutFits(layout, size)) return false;
  const bounds = paneSize(layout, paneId, size);
  if (!bounds) return false;
  return axis === "horizontal"
    ? bounds.width >= MIN_PANE_WIDTH * 2 + SPLIT_DIVIDER_SIZE
    : bounds.height >= MIN_PANE_HEIGHT * 2 + SPLIT_DIVIDER_SIZE;
}
export function mapLayout(
  layout: Layout,
  transform: (pane: Pane) => Pane,
): Layout {
  return layout.type !== "split"
    ? layout.type === "terminal"
      ? transform(layout)
      : layout
    : {
        ...layout,
        first: mapLayout(layout.first, transform),
        second: mapLayout(layout.second, transform),
      };
}
export function splitPane(
  layout: Layout,
  paneId: string,
  axis: Split["axis"],
  added: LayoutPane,
  before = false,
): Layout {
  if (layout.type !== "split")
    return layout.id === paneId
      ? {
          type: "split",
          id: newId(),
          axis,
          ratio: 0.5,
          first: before ? added : layout,
          second: before ? layout : added,
        }
      : layout;
  return {
    ...layout,
    first: splitPane(layout.first, paneId, axis, added, before),
    second: splitPane(layout.second, paneId, axis, added, before),
  };
}
export function removePane(layout: Layout, paneId: string): Layout | null {
  if (layout.type !== "split") return layout.id === paneId ? null : layout;
  const first = removePane(layout.first, paneId);
  const second = removePane(layout.second, paneId);
  return first && second ? { ...layout, first, second } : (first ?? second);
}
export function movePane(
  layout: Layout,
  id: string,
  targetId: string,
  side: TabDropSide,
  size: LayoutSize,
): Layout {
  const panels = layoutPanes(layout);
  const source = panels.find((pane) => pane.id === id);
  if (
    !source ||
    id === targetId ||
    !panels.some((pane) => pane.id === targetId)
  )
    return layout;
  const remaining = removePane(layout, id);
  if (!remaining) return layout;
  const moved = splitPane(
    remaining,
    targetId,
    side === "left" || side === "right" ? "horizontal" : "vertical",
    source,
    side === "left" || side === "top",
  );
  return layoutFits(moved, size) ? moved : layout;
}
export function resizeSplit(layout: Layout, id: string, ratio: number): Layout {
  if (!Number.isFinite(ratio)) return layout;
  if (layout.type !== "split") return layout;
  if (layout.id === id)
    return { ...layout, ratio: Math.max(0, Math.min(1, ratio)) };
  return {
    ...layout,
    first: resizeSplit(layout.first, id, ratio),
    second: resizeSplit(layout.second, id, ratio),
  };
}
export function active(session: Session) {
  if (session.activeProjectId === null) return;
  const project =
    session.projects.find(
      (project) => project.id === session.activeProjectId,
    ) ?? session.projects[0];
  if (!project) return;
  const workspace =
    project.workspaces.find(
      (workspace) => workspace.id === project.activeWorkspaceId,
    ) ?? project.workspaces[0];
  const tab =
    workspace.tabs.find((tab) => tab.id === workspace.activeTabId) ??
    workspace.tabs[0];
  return { project, workspace, tab };
}
export function updateWorkspace(
  session: Session,
  id: string,
  transform: (workspace: Workspace) => Workspace,
): Session {
  return {
    ...session,
    projects: session.projects.map((project) => ({
      ...project,
      workspaces: project.workspaces.map((workspace) =>
        workspace.id === id ? transform(workspace) : workspace,
      ),
    })),
  };
}
export function tabsToClose(
  tabs: Tab[],
  id: string,
  action: TabCloseAction,
  modified: ReadonlySet<string>,
): Tab[] {
  const index = tabs.findIndex((tab) => tab.id === id);
  if (index < 0) return [];
  switch (action) {
    case "close":
      return [tabs[index]];
    case "others":
      return tabs.filter((tab) => tab.id !== id);
    case "left":
      return tabs.slice(0, index);
    case "right":
      return tabs.slice(index + 1);
    case "clean":
      return tabs.filter((tab) => !modified.has(tab.id));
    case "all":
      return tabs;
  }
}

export function moveTab(
  workspace: Workspace,
  id: string,
  beforeId: string | null,
): Workspace {
  const moved = workspace.tabs.find((tab) => tab.id === id);
  if (
    !moved ||
    id === beforeId ||
    (beforeId !== null && !workspace.tabs.some((tab) => tab.id === beforeId))
  )
    return workspace;
  const tabs = workspace.tabs.filter((tab) => tab.id !== id);
  const index =
    beforeId === null
      ? tabs.length
      : tabs.findIndex((tab) => tab.id === beforeId);
  tabs.splice(index, 0, moved);
  return tabs.every((tab, index) => tab === workspace.tabs[index])
    ? workspace
    : { ...workspace, tabs };
}

export function canMergeTabs(
  source: Tab,
  target: Tab,
  side: TabDropSide,
  size: LayoutSize,
): boolean {
  if (
    source.id === target.id ||
    source.type === "commit" ||
    source.type === "diff" ||
    target.type !== "terminal"
  )
    return false;
  const first = minimumLayoutSize(target.layout);
  const second = minimumLayoutSize(
    source.type !== "terminal" ? source : source.layout,
  );
  return side === "left" || side === "right"
    ? size.width >= first.width + SPLIT_DIVIDER_SIZE + second.width &&
        size.height >= Math.max(first.height, second.height)
    : size.width >= Math.max(first.width, second.width) &&
        size.height >= first.height + SPLIT_DIVIDER_SIZE + second.height;
}

export function mergeTabs(
  workspace: Workspace,
  sourceId: string,
  targetId: string,
  side: TabDropSide,
  size: LayoutSize,
): Workspace {
  const source = workspace.tabs.find((tab) => tab.id === sourceId);
  const target = workspace.tabs.find((tab) => tab.id === targetId);
  if (
    !source ||
    source.type === "commit" ||
    source.type === "diff" ||
    target?.type !== "terminal" ||
    !canMergeTabs(source, target, side, size)
  )
    return workspace;
  // Moved panes retain their shells even after restoring or restarting them.
  const moved =
    source.type !== "terminal"
      ? source
      : mapLayout(source.layout, (pane) =>
          pane.profileId !== undefined || source.profileId === target.profileId
            ? pane
            : { ...pane, profileId: source.profileId },
        );
  const before = side === "left" || side === "top";
  const layout: Split = {
    type: "split",
    id: newId(),
    axis: side === "left" || side === "right" ? "horizontal" : "vertical",
    ratio: 0.5,
    first: before ? moved : target.layout,
    second: before ? target.layout : moved,
  };
  return {
    ...workspace,
    activeTabId: target.id,
    tabs: workspace.tabs
      .filter((tab) => tab.id !== source.id)
      .map((tab) =>
        tab.id === target.id
          ? {
              ...target,
              layout,
              activePaneId:
                source.type !== "terminal" ? source.id : source.activePaneId,
            }
          : tab,
      ),
  };
}

export function removeTabs(
  workspace: Workspace,
  ids: ReadonlySet<string>,
  cwd: string,
  profileId: string,
): Workspace {
  const tabs = workspace.tabs.filter((tab) => !ids.has(tab.id));
  if (tabs.length === workspace.tabs.length) return workspace;
  if (!tabs.length) tabs.push(newTab(cwd, profileId));
  const activeIndex = workspace.tabs.findIndex(
    (tab) => tab.id === workspace.activeTabId,
  );
  const next = ids.has(workspace.activeTabId)
    ? (workspace.tabs.slice(activeIndex + 1).find((tab) => !ids.has(tab.id)) ??
      tabs[tabs.length - 1])
    : undefined;
  return {
    ...workspace,
    tabs,
    activeTabId: next?.id ?? workspace.activeTabId,
  };
}

export function updateTab(
  session: Session,
  id: string,
  transform: (tab: Tab) => Tab,
): Session {
  return {
    ...session,
    projects: session.projects.map((project) => ({
      ...project,
      workspaces: project.workspaces.map((workspace) => ({
        ...workspace,
        tabs: workspace.tabs.map((tab) =>
          tab.id === id ? transform(tab) : tab,
        ),
      })),
    })),
  };
}
export function openCommitTab(
  session: Session,
  workspaceId: string,
  root: string,
  commit: string,
  title: string,
): Session {
  return updateWorkspace(session, workspaceId, (workspace) => {
    const existing = workspace.tabs.find(
      (tab) =>
        tab.type === "commit" && tab.root === root && tab.commit === commit,
    );
    if (existing) return { ...workspace, activeTabId: existing.id };
    const tab: CommitTab = { type: "commit", id: newId(), root, commit, title };
    return {
      ...workspace,
      tabs: [...workspace.tabs, tab],
      activeTabId: tab.id,
    };
  });
}
export function openDiffTab(
  session: Session,
  workspaceId: string,
  root: string,
  relative: string,
  staged: boolean,
): Session {
  return updateWorkspace(session, workspaceId, (workspace) => {
    const existing = workspace.tabs.find(
      (tab) =>
        tab.type === "diff" &&
        tab.root === root &&
        tab.relative === relative &&
        tab.staged === staged,
    );
    if (existing) return { ...workspace, activeTabId: existing.id };
    const tab: DiffTab = {
      type: "diff",
      id: newId(),
      root,
      relative,
      staged,
      title: `${basename(relative)} · ${staged ? "Staged changes" : "Changes"}`,
    };
    return {
      ...workspace,
      tabs: [...workspace.tabs, tab],
      activeTabId: tab.id,
    };
  });
}
export function updateDirectories(
  session: Session,
  directories: Record<string, string>,
): Session {
  let changed = false;
  const projects = session.projects.map((project) => ({
    ...project,
    workspaces: project.workspaces.map((workspace) => ({
      ...workspace,
      tabs: workspace.tabs.map((tab) =>
        tab.type !== "terminal"
          ? tab
          : {
              ...tab,
              layout: mapLayout(tab.layout, (pane) => {
                const cwd = directories[pane.id];
                if (!cwd || cwd === pane.cwd) return pane;
                changed = true;
                return { ...pane, cwd };
              }),
            },
      ),
    })),
  }));
  return changed ? { ...session, projects } : session;
}

export function openFileTab(
  session: Session,
  workspaceId: string,
  root: string,
  relative: string,
): Session {
  return updateWorkspace(session, workspaceId, (workspace) => {
    for (const tab of workspace.tabs) {
      const file = filesInTab(tab).find(
        (file) => file.root === root && file.relative === relative,
      );
      if (file)
        return {
          ...workspace,
          activeTabId: tab.id,
          tabs: workspace.tabs.map((candidate) =>
            candidate.id === tab.id && candidate.type === "terminal"
              ? { ...candidate, activePaneId: file.id }
              : candidate,
          ),
        };
    }
    const tab: FileTab = {
      type: "file",
      id: newId(),
      title: basename(relative),
      root,
      relative,
    };
    return {
      ...workspace,
      tabs: [...workspace.tabs, tab],
      activeTabId: tab.id,
    };
  });
}

export function fileTabs(session: Session): FileTab[] {
  return session.projects.flatMap((project) =>
    project.workspaces.flatMap((workspace) =>
      workspace.tabs.flatMap(filesInTab),
    ),
  );
}

export function newFileTab(session: Session): FileTab {
  const titles = new Set(fileTabs(session).map((file) => file.title));
  let number = 1;
  while (titles.has(`Untitled-${number}`)) ++number;
  return {
    type: "file",
    id: newId(),
    title: `Untitled-${number}`,
    root: "",
    relative: "",
    untitled: true,
  };
}

export function updateFilePosition(
  session: Session,
  id: string,
  position: EditorPosition,
): Session {
  return updateFile(session, id, (file) => ({ ...file, position }));
}

export function updateFilePreviewView(
  session: Session,
  id: string,
  view: FilePreviewView,
): Session {
  return updateFile(session, id, (file) => {
    const { previewView: _previous, ...rest } = file;
    return view === "editor" ? rest : { ...rest, previewView: view };
  });
}

export function updateFile(
  session: Session,
  id: string,
  change: (file: FileTab) => FileTab,
): Session {
  const update = (layout: Layout): Layout =>
    layout.type === "split"
      ? {
          ...layout,
          first: update(layout.first),
          second: update(layout.second),
        }
      : layout.type === "file" && layout.id === id
        ? change(layout)
        : layout;
  return {
    ...session,
    projects: session.projects.map((project) => ({
      ...project,
      workspaces: project.workspaces.map((workspace) => ({
        ...workspace,
        tabs: workspace.tabs.map((tab) =>
          tab.type === "terminal"
            ? { ...tab, layout: update(tab.layout) }
            : tab.type === "file" && tab.id === id
              ? change(tab)
              : tab,
        ),
      })),
    })),
  };
}

export function browsersInTab(tab: Tab): BrowserTab[] {
  return tab.type === "browser"
    ? [tab]
    : tab.type === "terminal"
      ? layoutPanes(tab.layout).filter(
          (pane): pane is BrowserTab => pane.type === "browser",
        )
      : [];
}

export function browserTabs(session: Session): BrowserTab[] {
  return session.projects.flatMap((project) =>
    project.workspaces.flatMap((workspace) =>
      workspace.tabs.flatMap(browsersInTab),
    ),
  );
}

export function updateBrowser(
  session: Session,
  id: string,
  change: Partial<Pick<BrowserTab, "url" | "title">>,
): Session {
  const update = (layout: Layout): Layout =>
    layout.type === "split"
      ? {
          ...layout,
          first: update(layout.first),
          second: update(layout.second),
        }
      : layout.type === "browser" && layout.id === id
        ? { ...layout, ...change }
        : layout;
  return {
    ...session,
    projects: session.projects.map((project) => ({
      ...project,
      workspaces: project.workspaces.map((workspace) => ({
        ...workspace,
        tabs: workspace.tabs.map((tab) =>
          tab.type === "terminal"
            ? { ...tab, layout: update(tab.layout) }
            : tab.type === "browser" && tab.id === id
              ? { ...tab, ...change }
              : tab,
        ),
      })),
    })),
  };
}

const record = (value: unknown): Record<string, unknown> =>
  typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : {};
const string = (value: unknown, fallback: string) =>
  typeof value === "string" && value.length > 0 ? value : fallback;

export function restoreSession(value: unknown, info: AppInfo): Session {
  const data = record(value);
  if (
    ![1, 2, 3].includes(data.version as number) ||
    !Array.isArray(data.projects)
  )
    if (value == null) return newSession();
    else
      throw new Error(
        "The saved session is corrupt or unsupported. The original file was preserved.",
      );
  const ids = new Set<string>();
  const id = (value: unknown) => {
    let candidate = string(value, newId());
    if (ids.has(candidate)) candidate = newId();
    ids.add(candidate);
    return candidate;
  };
  const file = (node: Record<string, unknown>, cwd: string): FileTab => {
    const position = record(node.position);
    const previewView = node.previewView ?? node.markdownView;
    const offset = (value: unknown) =>
      typeof value === "number" && Number.isFinite(value)
        ? Math.max(0, Math.floor(value))
        : 0;
    return {
      type: "file",
      id: id(node.id),
      title: string(node.title, basename(string(node.relative, "File"))),
      ...(string(node.customTitle, "")
        ? { customTitle: node.customTitle as string }
        : {}),
      root: node.untitled === true ? "" : string(node.root, cwd),
      relative: node.untitled === true ? "" : string(node.relative, ""),
      ...(node.untitled === true ? { untitled: true as const } : {}),
      ...(previewView === "editor" ||
      previewView === "split" ||
      previewView === "preview"
        ? { previewView }
        : {}),
      ...(node.position
        ? {
            position: {
              anchor: offset(position.anchor),
              head: offset(position.head),
              scrollTop: offset(position.scrollTop),
              scrollLeft: offset(position.scrollLeft),
            },
          }
        : {}),
    };
  };
  const browser = (node: Record<string, unknown>): BrowserTab => ({
    type: "browser",
    id: id(node.id),
    title: string(node.title, "Browser"),
    ...(string(node.customTitle, "")
      ? { customTitle: node.customTitle as string }
      : {}),
    url: restoreBrowserUrl(node.url),
  });
  const chat = (node: Record<string, unknown>): ChatTab => {
    if (
      typeof node.conversationId !== "string" ||
      !/^[A-Za-z0-9_-]{1,100}$/.test(node.conversationId)
    )
      throw new Error(
        "Invalid saved conversation descriptor. The session was preserved.",
      );
    return {
      type: "chat",
      id: id(node.id),
      title: string(node.title, "Chat AI"),
      conversationId: node.conversationId,
      ...(typeof node.customTitle === "string"
        ? { customTitle: node.customTitle }
        : {}),
    };
  };
  const android = (node: Record<string, unknown>): AndroidTab => {
    if (
      node.deviceId !== null &&
      (typeof node.deviceId !== "string" ||
        !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(
          node.deviceId,
        ))
    )
      throw new Error(
        "Invalid saved Android device reference. The session was preserved.",
      );
    return {
      type: "android",
      id: id(node.id),
      title: string(node.title, "Android"),
      deviceId: node.deviceId,
      ...(typeof node.customTitle === "string"
        ? { customTitle: node.customTitle }
        : {}),
    };
  };
  const plugin = (node: Record<string, unknown>): PluginPanel => {
    if (
      typeof node.owner !== "string" ||
      !pluginId.test(node.owner) ||
      typeof node.viewType !== "string" ||
      !pluginId.test(node.viewType) ||
      !Number.isInteger(node.stateVersion) ||
      (node.stateVersion as number) < 1
    )
      throw new Error(
        "The saved plugin panel has an unsupported descriptor. The session was preserved.",
      );
    jsonState(node.state);
    return {
      type: "plugin",
      id: id(node.id),
      title: string(node.title, "Unavailable plugin view"),
      ...(typeof node.customTitle === "string"
        ? { customTitle: node.customTitle }
        : {}),
      owner: node.owner,
      viewType: node.viewType,
      stateVersion: node.stateVersion as number,
      state: node.state,
    };
  };
  const layout = (value: unknown, cwd: string, depth = 0): Layout => {
    const node = record(value);
    if (node.type === "plugin") return plugin(node);
    if (node.type === "file") return file(node, cwd);
    if (node.type === "browser") return browser(node);
    if (node.type === "chat") return chat(node);
    if (node.type === "android") return android(node);
    // Preserve every layout accepted by the native JSON parser's nesting limit.
    if (node.type === "split" && depth < 128)
      return {
        type: "split",
        id: id(node.id),
        axis: node.axis === "vertical" ? "vertical" : "horizontal",
        ratio:
          typeof node.ratio === "number" && Number.isFinite(node.ratio)
            ? Math.max(0, Math.min(1, node.ratio))
            : 0.5,
        first: layout(node.first, cwd, depth + 1),
        second: layout(node.second, cwd, depth + 1),
      };
    if (node.type !== undefined && node.type !== "terminal")
      throw new Error("Unknown saved panel type. The session was preserved.");
    return {
      type: "terminal",
      id: id(node.id),
      cwd: string(node.cwd, cwd),
      ...(typeof node.profileId === "string"
        ? { profileId: node.profileId }
        : {}),
    };
  };
  const projects = data.projects
    .map((value): Project | null => {
      const project = record(value);
      if (typeof project.path !== "string" || !project.path) return null;
      const path = project.path;
      const workspaces = (
        Array.isArray(project.workspaces) ? project.workspaces : []
      ).map((value): Workspace => {
        const workspace = record(value);
        const tabs = (Array.isArray(workspace.tabs) ? workspace.tabs : []).map(
          (value): Tab => {
            const tab = record(value);
            if (tab.type === "plugin") return plugin(tab);
            if (tab.type === "browser") return browser(tab);
            if (tab.type === "chat") return chat(tab);
            if (tab.type === "android") return android(tab);
            if (tab.type === "file") {
              return file(tab, path);
            }
            if (tab.type === "commit") {
              return {
                type: "commit",
                id: id(tab.id),
                title: string(tab.title, "Commit"),
                ...(string(tab.customTitle, "")
                  ? { customTitle: tab.customTitle as string }
                  : {}),
                root: string(tab.root, path),
                commit: string(tab.commit, ""),
              };
            }
            if (tab.type === "diff") {
              return {
                type: "diff",
                id: id(tab.id),
                title: string(tab.title, "File changes"),
                ...(string(tab.customTitle, "")
                  ? { customTitle: tab.customTitle as string }
                  : {}),
                root: string(tab.root, path),
                relative: string(tab.relative, ""),
                staged: tab.staged === true,
              };
            }
            if (tab.type !== undefined && tab.type !== "terminal")
              throw new Error(
                "Unknown saved tab type. The session was preserved.",
              );
            const tree = layout(tab.layout, path);
            const leaves = layoutPanes(tree);
            return {
              type: "terminal",
              id: id(tab.id),
              title: string(tab.title, "Terminal"),
              ...(string(tab.customTitle, "")
                ? { customTitle: tab.customTitle as string }
                : {}),
              profileId: string(tab.profileId, info.profiles[0]?.id ?? ""),
              activePaneId: leaves.some((pane) => pane.id === tab.activePaneId)
                ? (tab.activePaneId as string)
                : leaves[0].id,
              layout: tree,
            };
          },
        );
        if (!tabs.length) tabs.push(newTab(path, info.profiles[0]?.id ?? ""));
        return {
          id: id(workspace.id),
          name: string(workspace.name, "Default"),
          tabs,
          ...(Array.isArray(workspace.pluginSidebars)
            ? {
                pluginSidebars: workspace.pluginSidebars.map((value) =>
                  plugin(record(value)),
                ),
              }
            : {}),
          activeTabId: tabs.some((tab) => tab.id === workspace.activeTabId)
            ? (workspace.activeTabId as string)
            : tabs[0].id,
        };
      });
      if (!workspaces.length)
        workspaces.push(newWorkspace(path, info.profiles[0]?.id ?? ""));
      return {
        id: id(project.id),
        path,
        workspaces,
        activeWorkspaceId: workspaces.some(
          (workspace) => workspace.id === project.activeWorkspaceId,
        )
          ? (project.activeWorkspaceId as string)
          : workspaces[0].id,
      };
    })
    .filter((project): project is Project => project !== null);
  const savedSides = record(data.sidebarSides);
  const sidebarSides: Session["sidebarSides"] = {
    files: savedSides.files === "right" ? "right" : "left",
    git: savedSides.git === "right" ? "right" : "left",
    workspaces: savedSides.workspaces === "right" ? "right" : "left",
  };
  for (const [key, side] of Object.entries(savedSides))
    if (pluginId.test(key) && (side === "left" || side === "right"))
      sidebarSides[key as SidebarPanel] = side;
  const left =
    (typeof data.sidebar === "string" && pluginId.test(data.sidebar)) ||
    data.sidebar === "git" ||
    data.sidebar === "workspaces"
      ? data.sidebar
      : data.sidebar === null
        ? null
        : "files";
  const right =
    (typeof data.rightSidebar === "string" &&
      pluginId.test(data.rightSidebar)) ||
    data.rightSidebar === "git" ||
    data.rightSidebar === "files" ||
    data.rightSidebar === "workspaces"
      ? data.rightSidebar
      : null;
  const sidebarWidth = (value: unknown) =>
    typeof value === "number" && Number.isFinite(value)
      ? value > 0
        ? value
        : 180
      : 250;
  return {
    version: 3,
    projects,
    activeProjectId:
      data.activeProjectId === null
        ? null
        : projects.some((project) => project.id === data.activeProjectId)
          ? (data.activeProjectId as string)
          : (projects[0]?.id ?? null),
    sidebar:
      left && sidebarSides[left as SidebarPanel] === "left"
        ? (left as SidebarPanel)
        : null,
    rightSidebar:
      right && sidebarSides[right as SidebarPanel] === "right"
        ? (right as SidebarPanel)
        : null,
    sidebarSides,
    terminalOverviewSide:
      data.terminalOverviewSide === "right" ? "right" : "left",
    sidebarWidth: sidebarWidth(data.sidebarWidth),
    rightSidebarWidth: sidebarWidth(data.rightSidebarWidth),
  };
}

export function pluginPanels(session: Session): PluginPanel[] {
  return session.projects.flatMap((p) =>
    p.workspaces.flatMap((w) => [
      ...(w.pluginSidebars ?? []),
      ...w.tabs.flatMap((tab) =>
        tab.type === "plugin"
          ? [tab]
          : tab.type === "terminal"
            ? layoutPanes(tab.layout).filter(
                (pane): pane is PluginPanel => pane.type === "plugin",
              )
            : [],
      ),
    ]),
  );
}
export function updatePluginPanel(
  session: Session,
  id: string,
  state: PluginPanel["state"],
): Session {
  jsonState(state);
  const update = (layout: Layout): Layout =>
    layout.type === "split"
      ? {
          ...layout,
          first: update(layout.first),
          second: update(layout.second),
        }
      : layout.type === "plugin" && layout.id === id
        ? { ...layout, state }
        : layout;
  return {
    ...session,
    projects: session.projects.map((p) => ({
      ...p,
      workspaces: p.workspaces.map((w) => ({
        ...w,
        ...(w.pluginSidebars
          ? {
              pluginSidebars: w.pluginSidebars.map((p) =>
                p.id === id ? { ...p, state } : p,
              ),
            }
          : {}),
        tabs: w.tabs.map((tab) =>
          tab.type === "terminal"
            ? { ...tab, layout: update(tab.layout) }
            : tab.type === "plugin" && tab.id === id
              ? { ...tab, state }
              : tab,
        ),
      })),
    })),
  };
}

export function removeChatConversation(
  session: Session,
  conversationId: string,
  profileId: string,
): Session {
  const prune = (layout: Layout): Layout | null => {
    if (layout.type === "chat" && layout.conversationId === conversationId)
      return null;
    if (layout.type !== "split") return layout;
    const first = prune(layout.first),
      second = prune(layout.second);
    return first && second ? { ...layout, first, second } : (first ?? second);
  };
  return {
    ...session,
    projects: session.projects.map((project) => ({
      ...project,
      workspaces: project.workspaces.map((workspace) => {
        const tabs = workspace.tabs.flatMap((tab): Tab[] => {
          if (tab.type === "chat" && tab.conversationId === conversationId)
            return [];
          if (tab.type !== "terminal") return [tab];
          const layout = prune(tab.layout);
          if (!layout) return [];
          const panels = layoutPanes(layout);
          return [
            {
              ...tab,
              layout,
              activePaneId: panels.some((p) => p.id === tab.activePaneId)
                ? tab.activePaneId
                : panels[0].id,
            },
          ];
        });
        if (!tabs.length) tabs.push(newTab(project.path, profileId));
        return {
          ...workspace,
          tabs,
          activeTabId: tabs.some((t) => t.id === workspace.activeTabId)
            ? workspace.activeTabId
            : tabs[0].id,
        };
      }),
    })),
  };
}
