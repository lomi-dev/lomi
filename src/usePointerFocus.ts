import { useCallback, useEffect, useRef } from "react";
import type { RefObject } from "react";
import { isTextInput } from "./keybindings";

const panels =
  ".terminal-pane[data-pane-id], [data-file-pane-id], [data-plugin-pane-id], [data-android-pane-id]";

export function usePointerFocus(
  enabled: boolean,
  root: RefObject<HTMLElement | null>,
) {
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const pressed = useRef(false);
  const composing = useRef(false);
  const focusHovered = useCallback(() => {
    const container = root.current;
    if (
      !enabled ||
      !container ||
      !document.hasFocus() ||
      pressed.current ||
      composing.current ||
      document.querySelector("dialog[open], .menu")
    )
      return;
    const active = document.activeElement;
    if (isTextInput(active) && !container.contains(active)) return;
    const hovered = pointer.current
      ? document.elementFromPoint(pointer.current.x, pointer.current.y)
      : container.querySelector(
          ".terminal-pane[data-pane-id]:hover, [data-file-pane-id]:hover, [data-plugin-pane-id]:hover, [data-android-pane-id]:hover",
        );
    const panel = hovered?.closest(panels);
    if (!panel || !container.contains(panel) || panel.contains(active)) return;
    (
      panel.querySelector<HTMLElement>(
        ".xterm-helper-textarea, .cm-content, .image-viewport, [data-plugin-focus], [data-android-input]",
      ) ??
      (panel.matches("[data-plugin-pane-id]") ? (panel as HTMLElement) : null)
    )?.focus({ preventScroll: true });
  }, [enabled, root]);

  useEffect(() => {
    let frame = 0;
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(focusHovered);
    };
    const move = (event: PointerEvent) => {
      if (event.pointerType === "touch") return;
      pointer.current = { x: event.clientX, y: event.clientY };
      pressed.current = event.buttons !== 0;
      focusHovered();
    };
    const press = (event: PointerEvent) => {
      pointer.current = { x: event.clientX, y: event.clientY };
      pressed.current = event.buttons !== 0;
    };
    const leave = (event: PointerEvent) => {
      if (!event.relatedTarget) pointer.current = null;
    };
    const blur = () => {
      pressed.current = false;
      composing.current = false;
      pointer.current = null;
    };
    const compositionStart = () => {
      composing.current = true;
    };
    const compositionEnd = () => {
      composing.current = false;
      // Let the final IME input reach its original editor before moving focus.
      schedule();
    };
    window.addEventListener("pointermove", move, true);
    window.addEventListener("pointerover", move, true);
    window.addEventListener("pointerdown", press, true);
    window.addEventListener("pointerup", press, true);
    window.addEventListener("pointercancel", blur, true);
    window.addEventListener("pointerout", leave, true);
    window.addEventListener("compositionstart", compositionStart, true);
    window.addEventListener("compositionend", compositionEnd, true);
    window.addEventListener("focus", schedule);
    window.addEventListener("blur", blur);
    window.addEventListener("resize", schedule);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("pointermove", move, true);
      window.removeEventListener("pointerover", move, true);
      window.removeEventListener("pointerdown", press, true);
      window.removeEventListener("pointerup", press, true);
      window.removeEventListener("pointercancel", blur, true);
      window.removeEventListener("pointerout", leave, true);
      window.removeEventListener("compositionstart", compositionStart, true);
      window.removeEventListener("compositionend", compositionEnd, true);
      window.removeEventListener("focus", schedule);
      window.removeEventListener("blur", blur);
      window.removeEventListener("resize", schedule);
    };
  }, [focusHovered]);

  useEffect(() => {
    // Splits, restored views, and tab switches can move panels under a stationary pointer.
    focusHovered();
    const frame = requestAnimationFrame(focusHovered);
    return () => cancelAnimationFrame(frame);
  });
}
