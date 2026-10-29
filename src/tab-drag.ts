import { useEffect, useRef } from "react";
import type { PointerEvent as ReactPointerEvent, RefObject } from "react";
import { canMergeTabs, splitGeometry, tabTitle } from "./model";
import type { Tab, TabDropSide } from "./model";

interface Props {
  tabs: Tab[];
  activeTabId: string;
  strip: RefObject<HTMLDivElement | null>;
  mergeContainer: RefObject<HTMLDivElement | null>;
  onMove: (id: string, beforeId: string | null) => void;
  onMerge: (id: string, targetId: string, side: TabDropSide) => void;
  canMove?: (id: string, beforeId: string | null) => boolean | string;
  canMerge?: (
    id: string,
    targetId: string,
    side: TabDropSide,
  ) => boolean | string;
}

type Destination =
  | { type: "move"; beforeId: string | null }
  | { type: "merge"; targetId: string; side: TabDropSide };

// Pointer events work in native webviews that intercept HTML drag-and-drop.
export function useTabDrag(props: Props) {
  const latest = useRef(props);
  latest.current = props;
  const cleanup = useRef<() => void>(() => {});
  const suppressClick = useRef(false);
  useEffect(() => () => cleanup.current(), []);

  const beginDrag = (event: ReactPointerEvent<HTMLButtonElement>, tab: Tab) => {
    if (event.button !== 0 || !event.isPrimary) return;
    cleanup.current();
    suppressClick.current = false;
    const button = event.currentTarget;
    const pointerId = event.pointerId;
    const startX = event.clientX;
    const startY = event.clientY;
    const activeTabId = latest.current.activeTabId;
    let x = startX;
    let y = startY;
    let ghost: HTMLDivElement | undefined;
    let indicator: HTMLDivElement | undefined;
    let destination: Destination | undefined;
    let frame = 0;
    let lastTime = 0;
    button.setPointerCapture(pointerId);

    const preview = () => {
      if (!ghost || !indicator) return;
      const { tabs, strip, mergeContainer } = latest.current;
      if (
        latest.current.activeTabId !== activeTabId ||
        !button.isConnected ||
        !tabs.some((candidate) => candidate.id === tab.id)
      ) {
        clean();
        return;
      }
      ghost.textContent = tabTitle(tab);
      ghost.style.left = `${Math.max(0, Math.min(x + 12, innerWidth - ghost.offsetWidth))}px`;
      ghost.style.top = `${Math.max(0, Math.min(y + 12, innerHeight - ghost.offsetHeight))}px`;
      const bounds = strip.current?.getBoundingClientRect();
      if (
        bounds &&
        x >= bounds.left &&
        x <= bounds.right &&
        y >= bounds.top &&
        y <= bounds.bottom
      ) {
        const candidates = [
          ...strip.current!.querySelectorAll<HTMLElement>("[data-tab-id]"),
        ].filter(
          (element) =>
            element.dataset.tabId !== tab.id &&
            tabs.some((candidate) => candidate.id === element.dataset.tabId),
        );
        const before = candidates.find((element) => {
          const rect = element.getBoundingClientRect();
          return x < rect.left + rect.width / 2;
        });
        const edge =
          before?.getBoundingClientRect().left ??
          candidates.at(-1)?.getBoundingClientRect().right ??
          bounds.left;
        const beforeId = before?.dataset.tabId ?? null;
        const permission = latest.current.canMove?.(tab.id, beforeId) ?? true;
        const allowed = permission === true;
        destination = allowed ? { type: "move", beforeId } : undefined;
        ghost.textContent = allowed
          ? tabTitle(tab)
          : typeof permission === "string"
            ? permission
            : "This tab cannot be moved here.";
        indicator.className = allowed
          ? "tab-drop-marker"
          : "tab-merge-preview is-blocked";
        indicator.textContent = "";
        Object.assign(indicator.style, {
          left: `${Math.max(bounds.left, Math.min(bounds.right - 2, edge))}px`,
          top: `${bounds.top + 6}px`,
          width: "2px",
          height: `${bounds.height - 12}px`,
        });
        indicator.hidden = false;
        return;
      }
      const container = mergeContainer.current;
      const target = tabs.find((candidate) => candidate.id === activeTabId);
      const source = tabs.find((candidate) => candidate.id === tab.id);
      const area = container?.getBoundingClientRect();
      if (
        !area ||
        !source ||
        !target ||
        source.type === "commit" ||
        source.type === "diff" ||
        target.type !== "terminal" ||
        source.id === target.id ||
        x < area.left ||
        x > area.right ||
        y < area.top ||
        y > area.bottom
      ) {
        destination = undefined;
        indicator.hidden = true;
        return;
      }
      const horizontal = (x - area.left) / area.width;
      const vertical = (y - area.top) / area.height;
      let side: TabDropSide =
        Math.min(horizontal, 1 - horizontal) <= Math.min(vertical, 1 - vertical)
          ? horizontal < 0.5
            ? "left"
            : "right"
          : vertical < 0.5
            ? "top"
            : "bottom";
      const distances = {
        left: horizontal,
        right: 1 - horizontal,
        top: vertical,
        bottom: 1 - vertical,
      };
      // Match pane dragging's tolerance for small movements along zone boundaries.
      if (
        destination?.type === "merge" &&
        destination.targetId === target.id &&
        distances[destination.side] - distances[side] <
          12 / Math.min(area.width, area.height)
      )
        side = destination.side;
      const fits = canMergeTabs(source, target, side, area);
      const permission = fits
        ? (latest.current.canMerge?.(tab.id, target.id, side) ?? true)
        : false;
      const allowed = fits && permission === true;
      const sourceLayout = source.type !== "terminal" ? source : source.layout;
      destination = allowed
        ? { type: "merge", targetId: target.id, side }
        : undefined;
      indicator.className = `tab-merge-preview${allowed ? "" : " is-blocked"}`;
      indicator.dataset.side = side;
      indicator.textContent = allowed
        ? `Move ${source.title} here`
        : typeof permission === "string"
          ? permission
          : "Not enough room for these panels";
      const columns = side === "left" || side === "right";
      const before = side === "left" || side === "top";
      const geometry = splitGeometry(
        {
          type: "split",
          id: "preview",
          axis: columns ? "horizontal" : "vertical",
          ratio: 0.5,
          first: before ? sourceLayout : target.layout,
          second: before ? target.layout : sourceLayout,
        },
        { width: area.width, height: area.height },
      );
      const previewSize = allowed
        ? before
          ? geometry.first
          : geometry.second
        : {
            width: columns ? area.width / 2 : area.width,
            height: columns ? area.height : area.height / 2,
          };
      Object.assign(indicator.style, {
        left: `${side === "right" ? area.right - previewSize.width : area.left}px`,
        top: `${side === "bottom" ? area.bottom - previewSize.height : area.top}px`,
        width: `${previewSize.width}px`,
        height: `${previewSize.height}px`,
      });
      indicator.hidden = false;
    };
    const tick = (time: number) => {
      const strip = latest.current.strip.current;
      const bounds = strip?.getBoundingClientRect();
      if (
        strip &&
        bounds &&
        y >= bounds.top &&
        y <= bounds.bottom &&
        x >= bounds.left &&
        x <= bounds.right
      ) {
        const direction =
          x < bounds.left + 32 ? -1 : x > bounds.right - 32 ? 1 : 0;
        strip.scrollLeft +=
          direction * Math.min(32, time - (lastTime || time)) * 0.7;
      }
      lastTime = time;
      preview();
      if (ghost) frame = requestAnimationFrame(tick);
    };
    const move = (event: PointerEvent) => {
      if (event.pointerId !== pointerId) return;
      x = event.clientX;
      y = event.clientY;
      if (!ghost && Math.hypot(x - startX, y - startY) < 6) return;
      if (!ghost) {
        suppressClick.current = true;
        ghost = document.createElement("div");
        ghost.className = "tab-drag-ghost";
        indicator = document.createElement("div");
        indicator.hidden = true;
        document.body.append(indicator, ghost);
        document.body.classList.add("dragging-tab");
        button.closest(".tab")?.classList.add("is-dragging");
        frame = requestAnimationFrame(tick);
      }
      event.preventDefault();
      preview();
    };
    const clean = () => {
      cancelAnimationFrame(frame);
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", up);
      document.removeEventListener("pointercancel", cancel);
      document.removeEventListener("keydown", key, true);
      window.removeEventListener("blur", clean);
      button.removeEventListener("lostpointercapture", clean);
      if (button.hasPointerCapture(pointerId))
        button.releasePointerCapture(pointerId);
      ghost?.remove();
      ghost = undefined;
      indicator?.remove();
      document.body.classList.remove("dragging-tab");
      button.closest(".tab")?.classList.remove("is-dragging");
      cleanup.current = () => {};
    };
    const up = (event: PointerEvent) => {
      if (event.pointerId !== pointerId) return;
      x = event.clientX;
      y = event.clientY;
      preview();
      const drop = ghost ? destination : undefined;
      clean();
      if (drop?.type === "move") latest.current.onMove(tab.id, drop.beforeId);
      if (drop?.type === "merge")
        latest.current.onMerge(tab.id, drop.targetId, drop.side);
    };
    const cancel = (event: PointerEvent) => {
      if (event.pointerId === pointerId) clean();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        clean();
      }
    };
    cleanup.current = clean;
    document.addEventListener("pointermove", move, { passive: false });
    document.addEventListener("pointerup", up);
    document.addEventListener("pointercancel", cancel);
    document.addEventListener("keydown", key, true);
    window.addEventListener("blur", clean);
    button.addEventListener("lostpointercapture", clean);
  };
  return { beginDrag, suppressClick };
}
