import { useCallback, useLayoutEffect, useRef } from "react";
import { flushSync } from "react-dom";
import type { RefObject } from "react";
import { synchronizeVisibleTerminalFits } from "./terminal-runtime";

type Motion = "panes" | "sidebars" | false;
type PaneRect = Pick<DOMRect, "left" | "top" | "width" | "height">;
type MotionTarget = { element: HTMLElement; rect: PaneRect };
type MotionRun = {
  animations: Set<Animation>;
  willChange: Map<HTMLElement, string>;
  cancelled: boolean;
};

const MOTION_ID = "lomi-layout-motion";
const MOTION_DURATION = 120;
const MOTION_EASING = "cubic-bezier(0.2, 0.7, 0.2, 1)";
// Dockview measures its hosts to maintain split ratios; animate only their contents.
const MOTION_TARGETS =
  ".dock-pane-host > .split-child, .sidebar, .split-divider, .sidebar-divider";

function collectTargets(root: HTMLElement): MotionTarget[] {
  return [...root.querySelectorAll<HTMLElement>(MOTION_TARGETS)]
    .filter(
      (element) =>
        !element.matches(".browser-pane") &&
        !element.querySelector(".browser-pane"),
    )
    .map((element) => {
      const rect = element.getBoundingClientRect();
      return {
        element,
        rect: {
          left: rect.left,
          top: rect.top,
          width: rect.width,
          height: rect.height,
        },
      };
    });
}

function clamp(value: number, limit: number) {
  return Math.max(-limit, Math.min(limit, value));
}

function retainedOffset(from: PaneRect, to: PaneRect) {
  return {
    x: clamp(from.left - to.left + (from.width - to.width) * 0.04, 8),
    y: clamp(from.top - to.top + (from.height - to.height) * 0.04, 8),
  };
}

function enteringOffset(element: HTMLElement) {
  const sidebar = element.matches(".sidebar")
    ? element
    : element.previousElementSibling instanceof HTMLElement &&
        element.previousElementSibling.matches(".sidebar")
      ? element.previousElementSibling
      : null;
  if (sidebar)
    return {
      x: sidebar.dataset.side === "left" ? -6 : 6,
      y: 0,
    };
  return { x: 0, y: 6 };
}

function translate(x: number, y: number) {
  return `translate3d(${x}px, ${y}px, 0)`;
}

export function usePaneMotion(root: RefObject<HTMLDivElement | null>) {
  const activeRun = useRef<MotionRun | null>(null);

  const clearRun = useCallback((run: MotionRun) => {
    if (activeRun.current === run) activeRun.current = null;
    run.cancelled = true;
    for (const animation of run.animations) {
      animation.onfinish = null;
      animation.oncancel = null;
      animation.cancel();
    }
    run.animations.clear();
    for (const [element, previous] of run.willChange)
      element.style.willChange = previous;
    run.willChange.clear();
  }, []);

  const cancelCurrent = useCallback(() => {
    const run = activeRun.current;
    if (run) clearRun(run);
  }, [clearRun]);

  useLayoutEffect(() => () => cancelCurrent(), [cancelCurrent]);

  return useCallback(
    function renderLayout(
      render: () => void,
      motion: Motion = false,
      interrupt = false,
    ) {
      if (interrupt) cancelCurrent();
      if (!motion) {
        render();
        return;
      }

      const container = root.current;
      const reducedMotion =
        typeof matchMedia === "function" &&
        matchMedia("(prefers-reduced-motion: reduce)").matches;
      if (!container) {
        cancelCurrent();
        flushSync(render);
        return;
      }
      if (reducedMotion || typeof container.animate !== "function") {
        cancelCurrent();
        flushSync(render);
        synchronizeVisibleTerminalFits(container);
        return;
      }

      const previousTargets = collectTargets(container);
      const previousRects = new Map(
        previousTargets.map(({ element, rect }) => [element, rect]),
      );
      // A new request starts from the current visual geometry of the old motion.
      cancelCurrent();
      flushSync(render);

      const nextTargets = collectTargets(container);
      // Queue xterm's render for this frame before motion writes can change its glyph rasterization.
      synchronizeVisibleTerminalFits(container);
      const plans = nextTargets.flatMap(({ element, rect }) => {
        const previous = previousRects.get(element);
        if (
          previous &&
          element.matches(".dock-pane-host > .split-child") &&
          element.querySelector(".terminal-pane[data-pane-id]")
        )
          return [];
        const offset = previous
          ? retainedOffset(previous, rect)
          : enteringOffset(element);
        if (previous && Math.abs(offset.x) < 0.1 && Math.abs(offset.y) < 0.1)
          return [];
        return [
          {
            element,
            from: translate(offset.x, offset.y),
            opacity: previous ? "1" : "0",
          },
        ];
      });
      if (!plans.length) return;

      const run: MotionRun = {
        animations: new Set(),
        willChange: new Map(),
        cancelled: false,
      };
      activeRun.current = run;
      for (const plan of plans) {
        const { element } = plan;
        const previousWillChange = element.style.willChange;
        run.willChange.set(element, previousWillChange);
        element.style.willChange = "transform, opacity";
        let animation: Animation;
        try {
          animation = element.animate(
            [
              { transform: plan.from, opacity: plan.opacity },
              { transform: translate(0, 0), opacity: "1" },
            ],
            {
              id: MOTION_ID,
              duration: MOTION_DURATION,
              easing: MOTION_EASING,
              fill: "both",
            },
          );
        } catch {
          element.style.willChange = previousWillChange;
          run.willChange.delete(element);
          continue;
        }
        run.animations.add(animation);
        const finish = () => {
          if (run.cancelled || activeRun.current !== run) return;
          animation.onfinish = null;
          animation.oncancel = null;
          animation.cancel();
          run.animations.delete(animation);
          element.style.willChange = previousWillChange;
          run.willChange.delete(element);
          if (run.animations.size === 0) {
            activeRun.current = null;
            run.willChange.clear();
          }
        };
        animation.onfinish = finish;
        animation.oncancel = finish;
      }
      if (run.animations.size === 0) {
        activeRun.current = null;
        run.willChange.clear();
      }
    },
    [cancelCurrent, root],
  );
}
