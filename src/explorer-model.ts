import { basename, layoutPanes, newTab } from "./model.ts";
import type { FileTab, Layout, Project, Session, Tab } from "./model.ts";
import type { GitStatus } from "./api.ts";
import { joinFilePath, normalizePath, parentPath } from "./file-path.ts";
export { normalizePath, parentPath } from "./file-path.ts";

export interface FileChange {
  oldPath: string | null;
  newPath: string | null;
}
export interface FileRootAlias {
  root: string;
  canonical: string;
}

// Native mutations report canonical paths. Retained sessions may instead use
// restored directory aliases, so update each known logical namespace as well.
export function fileChangeAliases(
  change: FileChange,
  aliases: readonly FileRootAlias[],
  destinationRoot: string,
): FileChange[] {
  if (!change.oldPath) return [change];
  const oldPath = normalizePath(change.oldPath);
  const newPath =
    change.newPath === null ? null : normalizePath(change.newPath);
  const roots = aliases.map(({ root, canonical }) => ({
    root: normalizePath(root),
    canonical: normalizePath(canonical),
  }));
  const destination =
    newPath === null
      ? undefined
      : roots
          .filter((alias) => containsPath(alias.canonical, newPath))
          .sort(
            (a, b) =>
              b.canonical.length - a.canonical.length ||
              Number(b.root === normalizePath(destinationRoot)) -
                Number(a.root === normalizePath(destinationRoot)),
          )[0];
  const changes: FileChange[] = [{ oldPath, newPath }];
  for (const alias of roots.sort(
    (a, b) => a.canonical.length - b.canonical.length,
  )) {
    if (containsPath(alias.canonical, oldPath)) {
      let target = newPath;
      if (newPath !== null) {
        if (containsPath(alias.canonical, newPath)) {
          target = alias.root + newPath.slice(alias.canonical.length);
        } else if (destination) {
          target =
            destination.root + newPath.slice(destination.canonical.length);
        }
      }
      changes.push({
        oldPath: alias.root + oldPath.slice(alias.canonical.length),
        newPath: target,
      });
    } else if (containsPath(oldPath, alias.canonical)) {
      // An independently restored alias of a descendant also loses its target.
      changes.push({
        oldPath: alias.root,
        newPath:
          newPath === null
            ? null
            : newPath + alias.canonical.slice(oldPath.length),
      });
    }
  }
  return changes.filter(
    (item, index) =>
      changes.findIndex(
        (other) =>
          other.oldPath === item.oldPath && other.newPath === item.newPath,
      ) === index,
  );
}

export type FileOperation =
  | { kind: "newFile" | "newFolder" | "rename"; name: string }
  | { kind: "copy" | "move"; sourceRoot: string; source: string }
  | { kind: "duplicate" | "trash" | "delete" };
export const absoluteFilePath = (file: Pick<FileTab, "root" | "relative">) =>
  normalizePath(joinFilePath(file.root, file.relative));
export const containsPath = (parent: string, path: string) => {
  const base = normalizePath(parent);
  const candidate = normalizePath(path);
  return candidate === base || candidate.startsWith(`${base}/`);
};
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
  change: FileChange | readonly FileChange[],
  profileId: string,
): Session {
  const changes = (Array.isArray(change) ? change : [change])
    .flatMap((item) =>
      item.oldPath
        ? [
            {
              oldPath: normalizePath(item.oldPath),
              newPath:
                item.newPath === null ? null : normalizePath(item.newPath),
            },
          ]
        : [],
    )
    .sort((a, b) => b.oldPath.length - a.oldPath.length);
  if (!changes.length) return session;
  // Match every path in the original namespace. A canonical parent move must
  // not consume a more specific restored alias or relocate a destination twice.
  const matchingChange = (path: string) =>
    changes.find((item) => containsPath(item.oldPath, path));
  const removed = (path: string) => matchingChange(path)?.newPath === null;
  const relocate = (path: string) => {
    const item = matchingChange(path);
    return item?.newPath != null
      ? item.newPath + normalizePath(path).slice(item.oldPath.length)
      : path;
  };
  const projects = session.projects
    .filter((project) => !removed(project.path))
    .map((project) => ({ ...project, path: relocate(project.path) }));
  const file = (file: FileTab): FileTab | null => {
    if (file.untitled) return file;
    const path = absoluteFilePath(file);
    if (!matchingChange(path)) return file;
    if (removed(path)) return null;
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
      item.type === "android" ||
      item.type === "agent-task"
    )
      return item;
    if (item.type === "terminal")
      return {
        ...item,
        cwd: removed(item.cwd) ? project.path : relocate(item.cwd),
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
          tab.type === "android" ||
          tab.type === "agent-task"
        )
          return [tab];
        if (tab.type === "diff") {
          if (removed(tab.root)) return [];
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
              title: `${basename(relative, root)} · ${tab.staged ? "Staged changes" : "Changes"}`,
            },
          ];
        }
        if (tab.type === "commit")
          return removed(tab.root)
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
