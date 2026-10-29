import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, RefObject } from "react";
import { layoutPositions, movePane, tabTitle } from "./model";
import type { Layout, PaneTabDestination, Tab, TabDropSide } from "./model";

export interface PaneTabDropOptions {
  strip: RefObject<HTMLDivElement | null>;
  tabs: Tab[];
  sourceTabId: string;
  canDrop: (
    id: string,
    destination: PaneTabDestination,
  ) => boolean | string;
  onDrop: (id: string, destination: PaneTabDestination) => void;
  canMove?: (
    id: string,
    targetId: string,
    side: TabDropSide,
  ) => boolean | string;
}

interface Props {
  layout: Layout;
  root: RefObject<HTMLDivElement | null>;
  enabled: boolean;
  onMove: (id: string, targetId: string, side: TabDropSide) => void;
  tabDrop?: PaneTabDropOptions;
}

// Use pointer capture, as native webviews can intercept HTML drag-and-drop.
export function usePaneDrag(props: Props) {
  const { layout, root, enabled } = props;
  const latest = useRef(props);
  latest.current = props;
  const [moveModifierHeld, setMoveModifierHeld] = useState(false);
  const cleanup = useRef<(() => void) | null>(null);
  const suppressClick = useRef(false);
  useLayoutEffect(() => () => cleanup.current?.(), [layout, enabled]);
  useEffect(() => {
    if (!enabled) return;
    const key = (event: KeyboardEvent) => {
      setMoveModifierHeld(event.altKey || event.ctrlKey);
      if (event.key === "Escape" && cleanup.current) {
        event.preventDefault();
        event.stopImmediatePropagation();
        cleanup.current();
      }
      if (!event.altKey && !event.ctrlKey) cleanup.current?.();
    };
    const blur = () => {
      setMoveModifierHeld(false);
      cleanup.current?.();
    };
    window.addEventListener("keydown", key, true);
    window.addEventListener("keyup", key, true);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("keyup", key, true);
      window.removeEventListener("blur", blur);
      setMoveModifierHeld(false);
    };
  }, [enabled]);

  const beginDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (
      !enabled ||
      (!event.altKey && !event.ctrlKey) ||
      event.button !== 0 ||
      !event.isPrimary
    )
      return;
    const element = event.target as Element;
    const handle = element.closest<HTMLElement>(
      ".terminal-title-box, [data-pane-drag-handle]",
    );
    const source = handle?.closest<HTMLElement>(
      "[data-pane-id], [data-plugin-pane-id], [data-file-pane-id], [data-browser-pane-id], [data-chat-pane-id], [data-android-pane-id], [data-task-pane-id]",
    );
    const id =
      source?.dataset.paneId ??
      source?.dataset.pluginPaneId ??
      source?.dataset.filePaneId ??
      source?.dataset.browserPaneId ??
      source?.dataset.chatPaneId ??
      source?.dataset.androidPaneId ??
      source?.dataset.taskPaneId;
    const container = root.current;
    if (
      !handle ||
      !id ||
      !container ||
      element.closest("button, input, textarea, select")
    )
      return;
    cleanup.current?.();
    event.preventDefault();
    const pointerId = event.pointerId;
    const startX = event.clientX;
    const startY = event.clientY;
    let ghost: HTMLDivElement | undefined;
    let indicator: HTMLDivElement | undefined;
    let destination: { targetId: string; side: TabDropSide } | undefined;
    let tabDestination: PaneTabDestination | undefined;
    let x = startX;
    let y = startY;
    let frame = 0;
    let lastTime = 0;
    const sourceTabId = latest.current.tabDrop?.sourceTabId;
    handle.setPointerCapture(pointerId);

    const preview = () => {
      if (!ghost || !indicator) return;
      const options = latest.current.tabDrop;
      if (
        !handle.isConnected ||
        latest.current.layout !== layout ||
        !latest.current.enabled ||
        options?.sourceTabId !== sourceTabId ||
        (options && !options.tabs.some((tab) => tab.id === sourceTabId))
      ) {
        clean();
        return;
      }
      ghost.textContent = handle.textContent;
      ghost.style.left = `${Math.max(0, Math.min(x + 12, innerWidth - ghost.offsetWidth))}px`;
      ghost.style.top = `${Math.max(0, Math.min(y + 12, innerHeight - ghost.offsetHeight))}px`;
      tabDestination = undefined;
      const strip = options?.strip.current;
      const stripBounds = strip?.getBoundingClientRect();
      if (
        options &&
        strip &&
        stripBounds &&
        x >= stripBounds.left &&
        x <= stripBounds.right &&
        y >= stripBounds.top &&
        y <= stripBounds.bottom
      ) {
        destination = undefined;
        const elements = [
          ...strip.querySelectorAll<HTMLElement>("[data-tab-id]"),
        ];
        const hovered = elements.find((element) => {
          const rect = element.getBoundingClientRect();
          return x >= rect.left && x <= rect.right;
        });
        const hoveredTab = options.tabs.find(
          (tab) => tab.id === hovered?.dataset.tabId,
        );
        const rect = hovered?.getBoundingClientRect();
        const edgeWidth = rect ? Math.min(24, rect.width / 4) : 0;
        const middle =
          rect && x > rect.left + edgeWidth && x < rect.right - edgeWidth;
        const candidates = elements.filter((element) =>
          options.tabs.some((tab) => tab.id === element.dataset.tabId),
        );
        const before = candidates.find((element) => {
          const bounds = element.getBoundingClientRect();
          return x < bounds.left + bounds.width / 2;
        });
        const proposed: PaneTabDestination =
          middle && hoveredTab
            ? { type: "tab", tabId: hoveredTab.id }
            : { type: "new-tab", beforeId: before?.dataset.tabId ?? null };
        const supported =
          !hovered ||
          (hoveredTab &&
            hoveredTab.type !== "diff" &&
            hoveredTab.type !== "commit");
        const permission =
          supported &&
          !(proposed.type === "tab" && proposed.tabId === sourceTabId)
            ? options.canDrop(id, proposed)
            : false;
        const allowed = permission === true;
        if (allowed) tabDestination = proposed;
        ghost.textContent = allowed
          ? proposed.type === "new-tab"
            ? "Move panel to a new tab"
            : `Move panel to ${tabTitle(hoveredTab!)}`
          : typeof permission === "string"
            ? permission
            : "Cannot move panel here";
        indicator.removeAttribute("style");
        indicator.textContent = "";
        delete indicator.dataset.side;
        if (allowed && proposed.type === "new-tab") {
          const edge =
            before?.getBoundingClientRect().left ??
            candidates.at(-1)?.getBoundingClientRect().right ??
            stripBounds.left;
          indicator.className = "tab-drop-marker";
          Object.assign(indicator.style, {
            left: `${Math.max(stripBounds.left, Math.min(stripBounds.right - 2, edge))}px`,
            top: `${stripBounds.top + 6}px`,
            width: "2px",
            height: `${Math.max(0, stripBounds.height - 12)}px`,
          });
        } else {
          indicator.className = `pane-tab-drop-preview${allowed ? "" : " is-blocked"}`;
          const left = Math.max(
            stripBounds.left,
            rect?.left ?? stripBounds.left,
          );
          const right = Math.min(
            stripBounds.right,
            rect?.right ?? stripBounds.right,
          );
          Object.assign(indicator.style, {
            left: `${left}px`,
            top: `${stripBounds.top}px`,
            width: `${Math.max(0, right - left)}px`,
            height: `${stripBounds.height}px`,
          });
        }
        indicator.hidden = false;
        return;
      }
      const area = container.getBoundingClientRect();
      const positions = layoutPositions(layout, area);
      const target = positions.find(
        ({ layout, bounds }) =>
          layout.type !== "split" &&
          layout.id !== id &&
          x >= area.left + bounds.left &&
          x <= area.left + bounds.left + bounds.width &&
          y >= area.top + bounds.top &&
          y <= area.top + bounds.top + bounds.height,
      );
      if (!target) {
        destination = undefined;
        indicator.hidden = true;
        return;
      }
      const { bounds } = target;
      const horizontal = (x - area.left - bounds.left) / bounds.width;
      const vertical = (y - area.top - bounds.top) / bounds.height;
      const side: TabDropSide =
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
      // Small pointer movements along a zone boundary should not flip the preview.
      if (
        destination?.targetId === target.layout.id &&
        distances[destination.side] - distances[side] <
          12 / Math.min(bounds.width, bounds.height)
      )
        return;
      const moved = movePane(layout, id, target.layout.id, side, area);
      const permission =
        moved !== layout
          ? (options?.canMove?.(id, target.layout.id, side) ?? true)
          : false;
      const allowed = moved !== layout && permission === true;
      const proposed = allowed
        ? layoutPositions(moved, area).find(({ layout }) => layout.id === id)!
            .bounds
        : bounds;
      destination = allowed ? { targetId: target.layout.id, side } : undefined;
      indicator.removeAttribute("style");
      indicator.className = `pane-drop-preview${allowed ? "" : " is-blocked"}`;
      indicator.dataset.side = side;
      indicator.textContent = allowed
        ? "Move panel here"
        : typeof permission === "string"
          ? permission
          : "Not enough room for these panels";
      Object.assign(indicator.style, {
        left: `${area.left + proposed.left}px`,
        top: `${area.top + proposed.top}px`,
        width: `${proposed.width}px`,
        height: `${proposed.height}px`,
      });
      indicator.hidden = false;
    };
    const tick = (time: number) => {
      const strip = latest.current.tabDrop?.strip.current;
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
      if (!event.altKey && !event.ctrlKey) {
        clean();
        return;
      }
      if (
        !ghost &&
        Math.hypot(event.clientX - startX, event.clientY - startY) < 6
      )
        return;
      if (!ghost) {
        suppressClick.current = true;
        ghost = document.createElement("div");
        ghost.className = "pane-drag-ghost";
        ghost.textContent = handle.textContent;
        indicator = document.createElement("div");
        indicator.className = "pane-drop-preview";
        indicator.hidden = true;
        document.body.append(indicator, ghost);
        document.body.classList.add("dragging-pane");
        handle.classList.add("is-dragging");
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
      handle.removeEventListener("lostpointercapture", clean);
      observer.disconnect();
      if (handle.hasPointerCapture(pointerId))
        handle.releasePointerCapture(pointerId);
      ghost?.remove();
      ghost = undefined;
      indicator?.remove();
      document.body.classList.remove("dragging-pane");
      handle.classList.remove("is-dragging");
      cleanup.current = null;
    };
    const up = (event: PointerEvent) => {
      if (event.pointerId !== pointerId) return;
      x = event.clientX;
      y = event.clientY;
      preview();
      const drop =
        (event.altKey || event.ctrlKey) && ghost ? destination : undefined;
      const tabDrop =
        (event.altKey || event.ctrlKey) && ghost ? tabDestination : undefined;
      clean();
      if (tabDrop) latest.current.tabDrop?.onDrop(id, tabDrop);
      else if (drop) latest.current.onMove(id, drop.targetId, drop.side);
    };
    const cancel = (event: PointerEvent) => {
      if (event.pointerId === pointerId) clean();
    };
    const area = container.getBoundingClientRect();
    const observer = new ResizeObserver(() => {
      const resized = container.getBoundingClientRect();
      if (area.width !== resized.width || area.height !== resized.height)
        clean();
    });
    observer.observe(container);
    cleanup.current = clean;
    document.addEventListener("pointermove", move, { passive: false });
    document.addEventListener("pointerup", up);
    document.addEventListener("pointercancel", cancel);
    handle.addEventListener("lostpointercapture", clean);
  };
  return {
    beginDrag,
    moveModifierHeld: enabled && moveModifierHeld,
    suppressClick,
  };
}
