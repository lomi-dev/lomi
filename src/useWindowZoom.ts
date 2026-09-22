import { useEffect } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { errorMessage, native } from "./api";
import { useKeybindings } from "./KeybindingsProvider";
import { actionForEvent, isZoomAction } from "./keybindings";

const storageKey =
  new URLSearchParams(window.location.search).get("window") === "settings"
    ? "lomi.zoom.settings"
    : "lomi.zoom.main";
let percentage = 100;
try {
  const saved = Number(localStorage.getItem(storageKey));
  if (
    Number.isInteger(saved) &&
    saved >= 50 &&
    saved <= 200 &&
    saved % 10 === 0
  )
    percentage = saved;
} catch {
  // Zoom remains available when webview storage cannot be read.
}

let pending = Promise.resolve();
let appliedPercentage = 100;

export const windowZoom = () => appliedPercentage / 100;

function applyZoom(next: number) {
  const root = document.documentElement;
  root.style.setProperty("--app-zoom", String(next / 100));
  return getCurrentWebview()
    .setZoom(next / 100)
    .then(
      () => {
        percentage = next;
        appliedPercentage = next;
        // Menus and terminal measurements must follow the new CSS viewport.
        window.dispatchEvent(new Event("resize"));
      },
      (error: unknown) => {
        percentage = appliedPercentage;
        root.style.setProperty("--app-zoom", String(appliedPercentage / 100));
        throw error;
      },
    );
}

export function changeWindowZoom(action: "zoomIn" | "zoomOut" | "resetZoom") {
  pending = pending.then(async () => {
    const next =
      action === "resetZoom"
        ? 100
        : Math.max(
            50,
            Math.min(200, percentage + (action === "zoomIn" ? 10 : -10)),
          );
    if (next === percentage) return;
    await applyZoom(next);
    try {
      localStorage.setItem(storageKey, String(next));
    } catch {
      // A storage failure must not prevent resizing the current window's content.
    }
  });
  const result = pending;
  pending = pending.catch(() => {});
  return result;
}

export function useWindowZoom(onError: (message: string) => void) {
  const { bindings, ready } = useKeybindings();
  useEffect(() => {
    if (!native) return;
    let current = true;
    pending = pending
      .then(() => applyZoom(percentage))
      .catch((error) => {
        if (current) onError(`Could not restore zoom: ${errorMessage(error)}`);
      });
    return () => {
      current = false;
    };
  }, [onError]);

  useEffect(() => {
    if (!native || !ready) return;
    const keyboard = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        (event.target instanceof Element &&
          event.target.closest(".shortcut-recorder.recording"))
      )
        return;
      const action = actionForEvent(event, bindings);
      if (!isZoomAction(action)) return;
      event.preventDefault();
      event.stopPropagation();
      void changeWindowZoom(action).catch((error) =>
        onError(`Could not change zoom: ${errorMessage(error)}`),
      );
    };
    window.addEventListener("keydown", keyboard, true);
    return () => window.removeEventListener("keydown", keyboard, true);
  }, [bindings, ready, onError]);
}
