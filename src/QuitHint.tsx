import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useT } from "./i18n";

/** Shown after the first ⌘Q; native code quits on a second press within 2 s. */
export function QuitHint() {
  const { t } = useT();
  // Incremented per press so the countdown bar restarts its animation.
  const [shown, setShown] = useState(0);
  const hint = useRef<HTMLDivElement>(null);
  // A modal <dialog> (Settings) sits in the top layer above any z-index; a popover shown
  // after it joins the top layer above the dialog.
  useLayoutEffect(() => {
    const element = hint.current;
    if (element && !element.matches(":popover-open")) element.showPopover?.();
  }, [shown]);
  useEffect(() => {
    let timer: number | undefined;
    const unlisten = listen("lomi-quit-armed", () => {
      setShown((count) => count + 1);
      clearTimeout(timer);
      timer = window.setTimeout(() => setShown(0), 2000);
    });
    return () => {
      clearTimeout(timer);
      void unlisten.then((stop) => stop());
    };
  }, []);
  return shown ? (
    <div
      ref={hint}
      className="quit-hint"
      role="status"
      popover="manual"
      key={shown}
    >
      <span>{t("Hold")}</span>
      <span className="keycaps" aria-label="Command Q">
        <kbd>⌘</kbd>
        <kbd>Q</kbd>
      </span>
      <span>{t("or press twice to quit")}</span>
      <span className="quit-hint-countdown" aria-hidden="true" />
    </div>
  ) : null;
}
