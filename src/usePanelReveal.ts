import { useLayoutEffect, useRef, type RefObject } from "react";

export interface PanelRevealRequest {
  workspaceId: string;
  tabId: string;
  paneId: string;
  revision: number;
}

export interface PanelRevealSelection {
  workspaceId?: string;
  tabId?: string;
  paneId?: string;
  hidden?: boolean;
}

const REVEAL_CLASS = "is-panel-revealed";
const REVEAL_DURATION_MS = 1000;
const TARGET_WAIT_MS = 2000;

export function usePanelReveal(
  stage: RefObject<HTMLElement | null>,
  request: PanelRevealRequest | undefined,
  selection: PanelRevealSelection,
) {
  const lastTarget = useRef<HTMLElement | null>(null);
  const handledRevision = useRef<number | undefined>(undefined);
  const startedRevision = useRef<number | undefined>(undefined);
  const workspaceId = request?.workspaceId;
  const tabId = request?.tabId;
  const paneId = request?.paneId;
  const revision = request?.revision;
  const selectedWorkspaceId = selection.workspaceId;
  const selectedTabId = selection.tabId;
  const selectedPaneId = selection.paneId;
  const hidden = selection.hidden;

  useLayoutEffect(() => {
    if (
      workspaceId === undefined ||
      tabId === undefined ||
      paneId === undefined ||
      workspaceId !== selectedWorkspaceId ||
      tabId !== selectedTabId ||
      paneId !== selectedPaneId ||
      hidden === true
    ) {
      if (revision !== undefined && startedRevision.current === revision)
        handledRevision.current = revision;
      return;
    }
    if (handledRevision.current === revision) return;

    const stageElement = stage.current;
    if (!stageElement) return;
    startedRevision.current = revision;

    let cancelled = false;
    let frame: number | undefined;
    let timer: number | undefined;
    let observer: MutationObserver | undefined;
    let revealedTarget: HTMLElement | null = null;
    const deadline = performance.now() + TARGET_WAIT_MS;

    const removeReveal = () => {
      if (!revealedTarget) return;
      revealedTarget.classList.remove(REVEAL_CLASS);
      revealedTarget = null;
    };

    const stop = () => {
      cancelled = true;
      if (frame !== undefined) cancelAnimationFrame(frame);
      if (timer !== undefined) window.clearTimeout(timer);
      observer?.disconnect();
      removeReveal();
    };

    const findTarget = () => {
      const currentStage = stage.current;
      if (!currentStage) return null;
      if (currentStage.dataset.desktopPanelId === paneId) return currentStage;
      return currentStage.querySelector<HTMLElement>(
        `[data-desktop-panel-id="${CSS.escape(paneId)}"]`,
      );
    };

    const hasBounds = (target: HTMLElement) => {
      if (!target.isConnected) return false;
      const bounds = target.getBoundingClientRect();
      return bounds.width > 0 && bounds.height > 0;
    };

    const reveal = (target: HTMLElement) => {
      revealedTarget = target;
      if (lastTarget.current === target) void target.offsetWidth;
      target.classList.add(REVEAL_CLASS);
      lastTarget.current = target;
      timer = window.setTimeout(() => {
        handledRevision.current = revision;
        if (revealedTarget === target) removeReveal();
        observer?.disconnect();
        observer = undefined;
      }, REVEAL_DURATION_MS);
    };

    const checkTarget = () => {
      frame = undefined;
      if (cancelled) return;

      if (revealedTarget) {
        if (!revealedTarget.isConnected) stop();
        return;
      }

      const target = findTarget();
      if (target && hasBounds(target)) {
        reveal(target);
        return;
      }

      if (performance.now() >= deadline) {
        handledRevision.current = revision;
        observer?.disconnect();
        observer = undefined;
        return;
      }

      frame = requestAnimationFrame(checkTarget);
    };

    if (typeof MutationObserver !== "undefined") {
      observer = new MutationObserver(() => {
        if (revealedTarget && !revealedTarget.isConnected) stop();
      });
      const documentRoot = stageElement.ownerDocument.documentElement;
      if (documentRoot)
        observer.observe(documentRoot, { childList: true, subtree: true });
    }

    checkTarget();
    return stop;
  }, [
    stage,
    workspaceId,
    tabId,
    paneId,
    revision,
    selectedWorkspaceId,
    selectedTabId,
    selectedPaneId,
    hidden,
  ]);
}
