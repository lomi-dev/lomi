import {
  basename,
  filesInTab,
  newId,
  openFileTab,
  type FileTab,
  type Project,
  type Session,
  type Workspace,
} from "./model.ts";
import { joinFilePath, normalizePath } from "./file-path.ts";

export interface ResolvedExternalFile {
  root: string;
  relative: string;
}

function fileTab(root: string, relative: string): FileTab {
  return {
    type: "file",
    id: newId(),
    title: basename(relative, root),
    root,
    relative,
  };
}

function fileOnlyWorkspace(root: string, relative: string): Workspace {
  const file = fileTab(root, relative);
  return {
    id: newId(),
    name: "Default",
    tabs: [file],
    activeTabId: file.id,
  };
}

function fileOnlyProject(root: string, relative: string): Project {
  const workspace = fileOnlyWorkspace(root, relative);
  return {
    id: newId(),
    path: root,
    workspaces: [workspace],
    activeWorkspaceId: workspace.id,
  };
}

function focusFile(
  session: Session,
  projectId: string,
  workspaceId: string,
  tabId: string,
  fileId: string,
): Session {
  return {
    ...session,
    activeProjectId: projectId,
    projects: session.projects.map((project) =>
      project.id !== projectId
        ? project
        : {
            ...project,
            activeWorkspaceId: workspaceId,
            workspaces: project.workspaces.map((workspace) =>
              workspace.id !== workspaceId
                ? workspace
                : {
                    ...workspace,
                    activeTabId: tabId,
                    tabs: workspace.tabs.map((tab) => {
                      if (tab.id !== tabId || tab.type !== "terminal")
                        return tab;
                      return filesInTab(tab).some((file) => file.id === fileId)
                        ? { ...tab, activePaneId: fileId }
                        : tab;
                    }),
                  },
            ),
          },
    ),
  };
}

function fileIdentity(root: string, relative: string): string {
  return normalizePath(joinFilePath(root, relative), root);
}

function sameFile(file: FileTab, location: ResolvedExternalFile): boolean {
  return (
    fileIdentity(file.root, file.relative) ===
    fileIdentity(location.root, location.relative)
  );
}

/** Routes an OS-opened file to its project, reusing any existing editor tab. */
export function openExternalFile(
  session: Session,
  location: ResolvedExternalFile,
): Session {
  const { root, relative } = location;
  const project = session.projects.find((candidate) => candidate.path === root);

  if (!project) {
    const added = fileOnlyProject(root, relative);
    return {
      ...session,
      activeProjectId: added.id,
      projects: [added, ...session.projects],
    };
  }

  const workspaces = [
    ...project.workspaces.filter(
      (workspace) => workspace.id === project.activeWorkspaceId,
    ),
    ...project.workspaces.filter(
      (workspace) => workspace.id !== project.activeWorkspaceId,
    ),
  ];
  for (const workspace of workspaces) {
    for (const tab of workspace.tabs) {
      const file = filesInTab(tab).find((candidate) =>
        sameFile(candidate, location),
      );
      if (file)
        return focusFile(session, project.id, workspace.id, tab.id, file.id);
    }
  }

  const workspace = workspaces[0] ?? fileOnlyWorkspace(root, relative);
  if (!project.workspaces.length) {
    const projects = session.projects.map((candidate) =>
      candidate.id === project.id
        ? {
            ...project,
            activeWorkspaceId: workspace.id,
            workspaces: [workspace],
          }
        : candidate,
    );
    return { ...session, activeProjectId: project.id, projects };
  }

  const opened = openFileTab(session, workspace.id, root, relative);
  const selectedWorkspace = opened.projects
    .find((candidate) => candidate.id === project.id)
    ?.workspaces.find((candidate) => candidate.id === workspace.id);
  const selectedTab = selectedWorkspace?.tabs.find(
    (tab) => tab.id === selectedWorkspace.activeTabId,
  );
  const file = selectedTab
    ? filesInTab(selectedTab).find(
        (candidate) =>
          candidate.root === root && candidate.relative === relative,
      )
    : undefined;
  return file
    ? focusFile(opened, project.id, workspace.id, selectedTab!.id, file.id)
    : opened;
}
