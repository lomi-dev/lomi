import type { PointerEvent as ReactPointerEvent } from "react";
import { runningTerminal } from "./terminal-runtime";
import { errorMessage } from "./api";
import { windowZoom } from "./useWindowZoom";

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
) {
  if (
    event.button !== 0 ||
    (event.target as HTMLElement).closest(".tree-action")
  )
    return;
  const startX = event.clientX;
  const startY = event.clientY;
  let ghost: HTMLDivElement | undefined;
  let target: HTMLElement | null = null;
  const move = (event: PointerEvent) => {
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
    target = terminalAt(event.clientX, event.clientY);
    target?.classList.add("drop-target");
    event.preventDefault();
  };
  const clean = () => {
    document.removeEventListener("pointermove", move);
    document.removeEventListener("pointerup", up);
    document.removeEventListener("pointercancel", cancel);
    document.removeEventListener("keydown", key);
    ghost?.remove();
    target?.classList.remove("drop-target");
  };
  const cancel = () => clean();
  const key = (event: KeyboardEvent) => {
    if (event.key === "Escape") clean();
  };
  const up = () => {
    if (ghost) {
      const swallowClick = (event: MouseEvent) => {
        event.preventDefault();
        event.stopImmediatePropagation();
      };
      document.addEventListener("click", swallowClick, {
        capture: true,
        once: true,
      });
      setTimeout(
        () => document.removeEventListener("click", swallowClick, true),
        100,
      );
      void dropPaths(target, [path], onError);
    }
    clean();
  };
  document.addEventListener("pointermove", move, { passive: false });
  document.addEventListener("pointerup", up);
  document.addEventListener("pointercancel", cancel);
  document.addEventListener("keydown", key);
}
