import { basename, layoutPanes, newTab } from "./model.ts";
import type { FileTab, Layout, Project, Session, Tab } from "./model.ts";
import type { GitStatus } from "./api.ts";

export interface FileChange {
  oldPath: string | null;
  newPath: string | null;
}
export type FileOperation =
  | { kind: "newFile" | "newFolder" | "rename"; name: string }
  | { kind: "copy" | "move"; sourceRoot: string; source: string }
  | { kind: "duplicate" | "trash" | "delete" };
export const normalizePath = (path: string) =>
  path.replaceAll("\\", "/").replace(/\/$/, "");
export const absoluteFilePath = (file: Pick<FileTab, "root" | "relative">) =>
  `${normalizePath(file.root)}/${normalizePath(file.relative)}`;
export const containsPath = (parent: string, path: string) => {
  const base = normalizePath(parent);
  const candidate = normalizePath(path);
  return candidate === base || candidate.startsWith(`${base}/`);
};
export const parentPath = (path: string) =>
  normalizePath(path).split("/").slice(0, -1).join("/");

export const gitFilePath = (path: string) =>
  normalizePath(path)
    .replace(/^\/\/\?\/UNC\//, "//")
    .replace(/^\/\/\?\//, "");

export function repositoryForPath(repositories: GitStatus[], path: string) {
  return repositories
    .filter((repo) => containsPath(gitFilePath(repo.root), gitFilePath(path)))
    .sort((a, b) => b.root.length - a.root.length)[0];
}

export function explorerGitStatuses(
  statuses: GitStatus | GitStatus[] | null,
  projectRoot?: string,
) {
  const files = new Map<string, string>();
  const repositories = Array.isArray(statuses)
    ? statuses
    : statuses
      ? [statuses]
      : [];
  const folders = new Map<string, string>();
  const priority: Record<string, number> = { A: 1, M: 2, U: 3 };
  for (const status of [...repositories].sort(
    (a, b) => a.root.length - b.root.length,
  )) {
    const root = gitFilePath(status.root);
    const boundary =
      projectRoot && containsPath(gitFilePath(projectRoot), root)
        ? gitFilePath(projectRoot)
        : root;
    for (const change of status.changes) {
      const absolute = gitFilePath(`${root}/${change.path}`);
      const owner = repositoryForPath(repositories, absolute);
      if (
        owner &&
        owner.root !== status.root &&
        absolute !== gitFilePath(owner.root)
      )
        continue;
      const { index, worktree } = change;
      let code = worktree === " " ? index : worktree;
      if (index === "A" && worktree !== "D") code = "A";
      if (
        index === "U" ||
        worktree === "U" ||
        ["AA", "DD"].includes(index + worktree)
      )
        code = "U";
      if (!["?", "A", "M", "D", "R", "C", "T", "U"].includes(code)) continue;
      files.set(gitFilePath(`${root}/${change.path}`), code);
      const folderCode =
        code === "U" ? "U" : ["?", "A"].includes(code) ? "A" : "M";
      const paths = [change.path];
      if (change.originalPath && (index === "R" || worktree === "R"))
        paths.push(change.originalPath);
      for (const relative of paths) {
        for (
          let parent = parentPath(gitFilePath(`${root}/${relative}`));
          containsPath(boundary, parent);
          parent = parentPath(parent)
        ) {
          if (priority[folderCode] > (priority[folders.get(parent) ?? ""] ?? 0))
            folders.set(parent, folderCode);
          if (parent === boundary) break;
        }
      }
    }
  }
  return new Map([...files, ...folders]);
}

export function applyFileChange(
  session: Session,
  change: FileChange,
  profileId: string,
): Session {
  if (!change.oldPath) return session;
  const oldPath = normalizePath(change.oldPath);
  const newPath =
    change.newPath === null ? null : normalizePath(change.newPath);
  const relocate = (path: string) =>
    containsPath(oldPath, path) && newPath !== null
      ? newPath + normalizePath(path).slice(oldPath.length)
      : path;
  const projects = session.projects
    .filter(
      (project) => newPath !== null || !containsPath(oldPath, project.path),
    )
    .map((project) => ({ ...project, path: relocate(project.path) }));
  const file = (file: FileTab): FileTab | null => {
    if (file.untitled) return file;
    const path = absoluteFilePath(file);
    if (!containsPath(oldPath, path)) return file;
    if (newPath === null) return null;
    const destination = relocate(path);
    const previousRoot = relocate(file.root);
    const root = containsPath(previousRoot, destination)
      ? previousRoot
      : (projects
          .filter((project) => containsPath(project.path, destination))
          .sort((a, b) => b.path.length - a.path.length)[0]?.path ??
        parentPath(destination));
    return {
      ...file,
      root,
      relative: normalizePath(destination).slice(
        normalizePath(root).length + 1,
      ),
      title: basename(destination),
    };
  };
  const layout = (item: Layout, project: Project): Layout | null => {
    if (item.type === "file") return file(item);
    if (
      item.type === "browser" ||
      item.type === "plugin" ||
      item.type === "chat" ||
      item.type === "android"
    )
      return item;
    if (item.type === "terminal")
      return {
        ...item,
        cwd:
          newPath === null && containsPath(oldPath, item.cwd)
            ? project.path
            : relocate(item.cwd),
      };
    const first = layout(item.first, project);
    const second = layout(item.second, project);
    return first && second ? { ...item, first, second } : (first ?? second);
  };
  const updated = projects.map((project) => ({
    ...project,
    workspaces: project.workspaces.map((workspace) => {
      const tabs = workspace.tabs.flatMap((tab): Tab[] => {
        if (tab.type === "file") {
          const updated = file(tab);
          return updated ? [updated] : [];
        }
        if (
          tab.type === "browser" ||
          tab.type === "plugin" ||
          tab.type === "chat" ||
          tab.type === "android"
        )
          return [tab];
        if (tab.type === "diff") {
          if (newPath === null && containsPath(oldPath, tab.root)) return [];
          const root = relocate(tab.root);
          const destination = relocate(absoluteFilePath(tab));
          const relative = containsPath(root, destination)
            ? normalizePath(destination).slice(normalizePath(root).length + 1)
            : tab.relative;
          return [
            {
              ...tab,
              root,
              relative,
              title: `${basename(relative)} · ${tab.staged ? "Staged changes" : "Changes"}`,
            },
          ];
        }
        if (tab.type === "commit")
          return newPath === null && containsPath(oldPath, tab.root)
            ? []
            : [{ ...tab, root: relocate(tab.root) }];
        const updated = layout(tab.layout, project);
        if (!updated) return [];
        const panels = layoutPanes(updated);
        return [
          {
            ...tab,
            layout: updated,
            activePaneId: panels.some((pane) => pane.id === tab.activePaneId)
              ? tab.activePaneId
              : panels[0].id,
          },
        ];
      });
      if (!tabs.length) tabs.push(newTab(project.path, profileId));
      return {
        ...workspace,
        tabs,
        activeTabId: tabs.some((tab) => tab.id === workspace.activeTabId)
          ? workspace.activeTabId
          : tabs[0].id,
      };
    }),
  }));
  return {
    ...session,
    projects: updated,
    activeProjectId: updated.some(
      (project) => project.id === session.activeProjectId,
    )
      ? session.activeProjectId
      : (updated[0]?.id ?? null),
  };
}
