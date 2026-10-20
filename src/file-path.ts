export const isWindowsPath = (root: string) =>
  /^(?:[a-z]:[\\/]|\\\\|\/\/)/i.test(root);

// Relative paths inherit their root's separator rules. Backslashes are valid
// filename characters on Unix, including in names that resemble drive paths.
export const normalizePath = (path: string, root = path) =>
  (isWindowsPath(root)
    ? path
        .replaceAll("\\", "/")
        .replace(/^\/\/\?\/UNC\//i, "//")
        .replace(/^\/\/\?\/(?=[a-z]:\/)/i, "")
    : path
  ).replace(/\/$/, "");

export const parentPath = (path: string, root = path) =>
  normalizePath(path, root).split("/").slice(0, -1).join("/");

export const pathBasename = (path: string, root = path) =>
  (isWindowsPath(root)
    ? path.replace(/[\\/]+$/, "").split(/[\\/]/)
    : path.replace(/\/+$/, "").split("/")
  ).pop() || path;

export function joinFilePath(root: string, relative: string) {
  if (!isWindowsPath(root)) return `${root.replace(/\/$/, "")}/${relative}`;
  const separator = root.includes("\\") ? "\\" : "/";
  return (
    root.replace(/[\\/]$/, "") +
    separator +
    relative.replace(/[\\/]/g, separator)
  );
}
