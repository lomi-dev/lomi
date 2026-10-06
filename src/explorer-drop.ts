import { useEffect, useRef, useState } from "react";
import { errorMessage, windows } from "./api";
import { normalizePath } from "./explorer-model";
import type { FileOperation } from "./explorer-model";

type Position = { x: number; y: number };
export type ExplorerFileDrag =
  | { type: "enter" | "over"; position: Position }
  | { type: "drop"; position: Position; paths: string[] }
  | { type: "leave" };

interface Options {
  root: string;
  hidden: boolean;
  busy: boolean;
  onOperation: (relative: string, operation: FileOperation) => Promise<boolean>;
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
    let hover: { folder: string; x: number; y: number } | undefined;
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
      if (detail.type === "drop") {
        clear();
        locked.current = true;
        setImporting(true);
        const handlers = latest.current;
        void (async () => {
          let copied = false;
          try {
            for (const path of detail.paths) {
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
      if (hover && hover.folder === folder && hover.x === x && hover.y === y)
        return;
      clear();
      target.setAttribute("data-explorer-drop-target", "true");
      node
        .querySelectorAll<HTMLElement>(
          "[data-explorer-folder], .project-tree-heading",
        )
        .forEach((element) => {
          if ((element.dataset.explorerFolder ?? "") === destination)
            element.setAttribute("data-explorer-drop-destination", "true");
        });
      if (folder === undefined) return;
      hover = { folder, x, y };
      if (target.querySelector('[aria-expanded="false"]'))
        timer = setTimeout(() => {
          timer = undefined;
          if (
            current &&
            !latest.current.busy &&
            !locked.current &&
            target.isConnected &&
            targetAt({ x, y }) === target
          )
            latest.current.onExpand(folder);
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
  // An in-flight copy may finish after navigation; its lock still prevents overlap.
  useEffect(() => setImporting(locked.current), [options.root, options.hidden]);
  return { panel, importing };
}
