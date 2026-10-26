import { useEffect, useRef, useState } from "react";
import { errorMessage, windows } from "./api";
import { isWindowsPath } from "./file-path";
import { normalizePath } from "./explorer-model";
import type { FileOperation } from "./explorer-model";

type Position = { x: number; y: number };
export interface ExplorerFileDragSource {
  root: string;
  relative: string;
  isDirectory: boolean;
}

export type ExplorerFileDrag =
  | {
      type: "enter" | "over";
      position: Position;
      source?: ExplorerFileDragSource;
    }
  | {
      type: "drop";
      position: Position;
      paths?: string[];
      source?: ExplorerFileDragSource;
    }
  | { type: "leave" };

function validMoveDestination(
  source: ExplorerFileDragSource,
  root: string,
  destination: string,
) {
  const windowsPath = isWindowsPath(source.root);
  const rootPath = normalizePath(source.root, source.root);
  const destinationRoot = normalizePath(root, root);
  const compare = (path: string) => (windowsPath ? path.toLowerCase() : path);
  if (compare(rootPath) !== compare(destinationRoot)) return false;

  const sourcePath = normalizePath(source.relative, source.root);
  const destinationPath = normalizePath(destination, root);
  if (!sourcePath) return false;

  const parent = normalizePath(
    sourcePath.split("/").slice(0, -1).join("/"),
    source.root,
  );
  if (compare(parent) === compare(destinationPath)) return false;

  if (source.isDirectory) {
    const sourceKey = compare(sourcePath);
    const destinationKey = compare(destinationPath);
    if (
      destinationKey === sourceKey ||
      destinationKey.startsWith(`${sourceKey}/`)
    )
      return false;
  }
  return true;
}

interface Options {
  root: string;
  hidden: boolean;
  busy: boolean;
  onOperation: (relative: string, operation: FileOperation) => Promise<boolean>;
  onMove: (source: ExplorerFileDragSource, destination: string) => void;
  onExpand: (relative: string) => void;
  onRefresh: () => void;
  onError: (message: string) => void;
}

export function useExplorerDrop(options: Options) {
  const panel = useRef<HTMLDivElement>(null);
  const latest = useRef(options);
  latest.current = options;
  const locked = useRef(false);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const [importing, setImporting] = useState(false);
  const cancelHover = useRef(() => {});

  useEffect(() => {
    const node = panel.current;
    if (!node || options.hidden) return;
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let hover:
      | {
          target: HTMLElement;
          folder?: string;
          destination: string;
          position: Position;
          kind: "copy" | "move";
        }
      | undefined;
    const clear = () => {
      clearTimeout(timer);
      timer = undefined;
      hover = undefined;
      node
        .querySelectorAll(
          "[data-explorer-drop-target], [data-explorer-drop-destination]",
        )
        .forEach((element) => {
          element.removeAttribute("data-explorer-drop-target");
          element.removeAttribute("data-explorer-drop-destination");
          element.removeAttribute("data-explorer-drop-kind");
        });
    };
    cancelHover.current = clear;
    const targetAt = ({ x, y }: Position) => {
      const hit = document.elementFromPoint(x, y);
      if (!hit || !node.contains(hit)) return null;
      if (
        hit.closest(
          'input, textarea, [role="dialog"], [role="menu"], .tree-action, .icon-button',
        )
      )
        return null;
      const target = hit.closest<HTMLElement>("[data-explorer-directory]");
      return target && node.contains(target) ? target : null;
    };
    const receive = (event: Event) => {
      const detail = (event as CustomEvent<ExplorerFileDrag>).detail;
      if (detail.type === "leave" || latest.current.busy || locked.current) {
        clear();
        return;
      }
      const target = targetAt(detail.position);
      if (!target) {
        clear();
        return;
      }
      const destination = target.dataset.explorerDirectory!;
      const source = detail.source;
      if (
        source &&
        !validMoveDestination(source, latest.current.root, destination)
      ) {
        clear();
        return;
      }
      if (detail.type === "drop") {
        clear();
        if (source) {
          if (!validMoveDestination(source, latest.current.root, destination))
            return;
          locked.current = true;
          setImporting(true);
          const handlers = latest.current;
          void (async () => {
            let moved = false;
            try {
              try {
                moved = await handlers.onOperation(destination, {
                  kind: "move",
                  sourceRoot: source.root,
                  source: source.relative,
                });
              } catch (error) {
                handlers.onError(errorMessage(error));
              }
            } finally {
              if (current && moved) {
                handlers.onMove(source, destination);
                handlers.onExpand(destination);
                handlers.onRefresh();
              }
              locked.current = false;
              if (mounted.current) setImporting(false);
            }
          })();
          return;
        }
        const paths = detail.paths ?? [];
        if (!paths.length) return;
        locked.current = true;
        setImporting(true);
        const handlers = latest.current;
        void (async () => {
          let copied = false;
          try {
            for (const path of paths) {
              const normalized = windows
                ? normalizePath(path)
                : path.replace(/\/$/, "");
              const separator = normalized.lastIndexOf("/");
              if (separator < 0 || separator === normalized.length - 1) {
                handlers.onError(`Cannot import path: ${path}`);
                continue;
              }
              try {
                copied =
                  (await handlers.onOperation(destination, {
                    kind: "copy",
                    // Keep the separator so / and C:/ remain absolute roots.
                    sourceRoot: normalized.slice(0, separator + 1),
                    source: normalized.slice(separator + 1),
                  })) || copied;
              } catch (error) {
                handlers.onError(errorMessage(error));
              }
            }
          } finally {
            if (current && copied) {
              handlers.onExpand(destination);
              handlers.onRefresh();
            }
            locked.current = false;
            if (mounted.current) setImporting(false);
          }
        })();
        return;
      }
      const folder = target.dataset.explorerFolder;
      const { x, y } = detail.position;
      const kind = source ? "move" : "copy";
      if (
        hover &&
        hover.target === target &&
        hover.destination === destination &&
        hover.kind === kind
      ) {
        if (kind === "move") {
          hover.position = { x, y };
          return;
        }
        if (hover.position.x === x && hover.position.y === y) return;
      }
      clear();
      target.setAttribute("data-explorer-drop-target", "true");
      if (source) target.setAttribute("data-explorer-drop-kind", "move");
      node
        .querySelectorAll<HTMLElement>(
          "[data-explorer-folder], .project-tree-heading",
        )
        .forEach((element) => {
          if ((element.dataset.explorerFolder ?? "") === destination)
            element.setAttribute("data-explorer-drop-destination", "true");
        });
      if (folder === undefined) return;
      hover = { target, folder, destination, position: { x, y }, kind };
      if (target.querySelector('[aria-expanded="false"]'))
        timer = setTimeout(() => {
          timer = undefined;
          const currentHover = hover;
          if (
            current &&
            !latest.current.busy &&
            !locked.current &&
            currentHover &&
            currentHover.target.isConnected &&
            targetAt(currentHover.position) === currentHover.target
          )
            latest.current.onExpand(currentHover.folder!);
          else clear();
        }, 1000);
    };
    node.addEventListener("explorer-file-drag", receive);
    node.addEventListener("scroll", clear, true);
    window.addEventListener("resize", clear);
    return () => {
      current = false;
      clear();
      node.removeEventListener("explorer-file-drag", receive);
      node.removeEventListener("scroll", clear, true);
      window.removeEventListener("resize", clear);
    };
  }, [options.root, options.hidden]);

  useEffect(() => {
    if (options.busy) cancelHover.current();
  }, [options.busy]);
  // An in-flight file operation may finish after navigation; its lock persists.
  useEffect(() => setImporting(locked.current), [options.root, options.hidden]);
  return { panel, importing };
}
