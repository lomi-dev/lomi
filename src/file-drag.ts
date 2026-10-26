import type { PointerEvent as ReactPointerEvent } from "react";
import { runningTerminal } from "./terminal-runtime";
import { errorMessage } from "./api";
import { windowZoom } from "./useWindowZoom";
import type { ExplorerFileDragSource } from "./explorer-drop";

export function nativePositionToClient(
  position: { x: number; y: number },
  platform: string,
) {
  // Wry 0.55 reports AppKit/GTK logical coordinates but Windows physical pixels.
  // WebKit page zoom is separate from devicePixelRatio; WebView2 includes it.
  const scale = platform === "windows" ? window.devicePixelRatio : windowZoom();
  return { x: position.x / scale, y: position.y / scale };
}

export function terminalAt(x: number, y: number): HTMLElement | null {
  return (
    document
      .elementFromPoint(x, y)
      ?.closest<HTMLElement>(
        "[data-pane-id]:not(.is-overview), [data-chat-pane-id]",
      ) ?? null
  );
}

export async function dropPaths(
  target: HTMLElement | null,
  paths: string[],
  onError: (message: string) => void,
) {
  if (target?.dataset.chatPaneId) {
    target.dispatchEvent(new CustomEvent("chat-files", { detail: paths }));
    return;
  }
  const id = target?.dataset.paneId;
  if (!id) return;
  try {
    await runningTerminal(id)?.pastePaths(paths);
  } catch (error) {
    onError(errorMessage(error));
  }
}

// Pointer dragging also works where the native webview intercepts HTML5 drops.
export function beginFileDrag(
  event: ReactPointerEvent,
  path: string,
  onError: (message: string) => void,
  source?: ExplorerFileDragSource,
) {
  if (
    event.button !== 0 ||
    (event.target as HTMLElement).closest(".tree-action")
  )
    return;
  const startX = event.clientX;
  const startY = event.clientY;
  const pointerId = event.pointerId;
  const sourcePanel = source
    ? (event.currentTarget as HTMLElement).closest<HTMLElement>(
        "[data-explorer-root]",
      )
    : null;
  let ghost: HTMLDivElement | undefined;
  let target: HTMLElement | null = null;
  let explorer: HTMLElement | null = null;
  let cleaned = false;
  let sourceObserver: MutationObserver | undefined;
  let suppressClickTimer: ReturnType<typeof setTimeout> | undefined;
  const dispatchExplorer = (
    panel: HTMLElement,
    type: "enter" | "over" | "drop" | "leave",
    position?: { x: number; y: number },
  ) => {
    panel.dispatchEvent(
      new CustomEvent("explorer-file-drag", {
        detail: {
          type,
          ...(position ? { position } : {}),
          ...(source && type !== "leave" ? { source } : {}),
        },
      }),
    );
  };
  const leaveExplorer = () => {
    if (explorer) dispatchExplorer(explorer, "leave");
    explorer = null;
  };
  const explorerAt = (x: number, y: number) =>
    document
      .elementFromPoint(x, y)
      ?.closest<HTMLElement>("[data-explorer-root]") ?? null;
  const sourceStillMounted = () =>
    !source ||
    (sourcePanel?.isConnected === true &&
      sourcePanel.dataset.explorerRoot === source.root);
  const suppressClick = () => {
    if (suppressClickTimer !== undefined) return;
    const swallowClick = (event: MouseEvent) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      clearTimeout(suppressClickTimer);
      document.removeEventListener("click", swallowClick, true);
      suppressClickTimer = undefined;
    };
    document.addEventListener("click", swallowClick, {
      capture: true,
      once: true,
    });
    suppressClickTimer = setTimeout(() => {
      document.removeEventListener("click", swallowClick, true);
      suppressClickTimer = undefined;
    }, 100);
  };
  const move = (event: PointerEvent) => {
    if (event.pointerId !== pointerId || cleaned) return;
    if (!sourceStillMounted()) {
      cancel();
      return;
    }
    if (
      !ghost &&
      Math.hypot(event.clientX - startX, event.clientY - startY) < 6
    )
      return;
    if (!ghost) {
      ghost = document.createElement("div");
      ghost.className = "file-drag-ghost";
      ghost.textContent = path.split(/[\\/]/).pop() ?? path;
      document.body.append(ghost);
    }
    ghost.style.transform = `translate(${event.clientX + 12}px, ${event.clientY + 12}px)`;
    target?.classList.remove("drop-target");
    target = null;
    const nextExplorer = source
      ? explorerAt(event.clientX, event.clientY)
      : null;
    const enteringExplorer = explorer !== nextExplorer;
    if (enteringExplorer) leaveExplorer();
    if (nextExplorer) {
      target = null;
      explorer = nextExplorer;
      dispatchExplorer(explorer, enteringExplorer ? "enter" : "over", {
        x: event.clientX,
        y: event.clientY,
      });
    } else {
      leaveExplorer();
      target = terminalAt(event.clientX, event.clientY);
      target?.classList.add("drop-target");
    }
    event.preventDefault();
  };
  const clean = () => {
    if (cleaned) return;
    cleaned = true;
    document.removeEventListener("pointermove", move);
    document.removeEventListener("pointerup", up);
    document.removeEventListener("pointercancel", cancel);
    document.removeEventListener("keydown", key);
    window.removeEventListener("blur", cancel);
    sourceObserver?.disconnect();
    leaveExplorer();
    ghost?.remove();
    target?.classList.remove("drop-target");
  };
  const cancel = () => {
    if (ghost) suppressClick();
    clean();
  };
  const key = (event: KeyboardEvent) => {
    if (event.key === "Escape") cancel();
  };
  const up = (event: PointerEvent) => {
    if (event.pointerId !== pointerId || cleaned) return;
    if (ghost) {
      suppressClick();
      if (source && !sourceStillMounted()) {
        clean();
        return;
      }
      const nextExplorer = source
        ? explorerAt(event.clientX, event.clientY)
        : null;
      if (nextExplorer) {
        if (explorer !== nextExplorer) leaveExplorer();
        explorer = nextExplorer;
        target?.classList.remove("drop-target");
        target = null;
        dispatchExplorer(explorer, "drop", {
          x: event.clientX,
          y: event.clientY,
        });
      } else {
        leaveExplorer();
        target?.classList.remove("drop-target");
        target = terminalAt(event.clientX, event.clientY);
        void dropPaths(target, [path], onError);
      }
    }
    clean();
  };
  document.addEventListener("pointermove", move, { passive: false });
  document.addEventListener("pointerup", up);
  document.addEventListener("pointercancel", cancel);
  document.addEventListener("keydown", key);
  window.addEventListener("blur", cancel);
  if (sourcePanel && document.documentElement) {
    sourceObserver = new MutationObserver(() => {
      if (!sourceStillMounted()) cancel();
    });
    sourceObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-explorer-root"],
      childList: true,
      subtree: true,
    });
  }
}
