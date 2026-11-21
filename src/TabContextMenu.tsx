import { Fragment, useEffect, useLayoutEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { tabsToClose } from "./model";
import type { Tab, TabCloseAction } from "./model";

export interface TabMenuAnchor {
  id: string;
  x: number;
  y: number;
}

const actions: { action: TabCloseAction; label: string; divider?: boolean }[] =
  [
    { action: "close", label: "Close" },
    { action: "others", label: "Close others" },
    { action: "left", label: "Close left", divider: true },
    { action: "right", label: "Close right" },
    { action: "clean", label: "Close clean", divider: true },
    { action: "all", label: "Close all" },
  ];

export default function TabContextMenu({
  anchor,
  tabs,
  modified,
  onClose,
  onRename,
  onDismiss,
}: {
  anchor: TabMenuAnchor;
  tabs: Tab[];
  modified: ReadonlySet<string>;
  onClose: (id: string, action: TabCloseAction) => void;
  onRename: (tab: Tab) => void;
  onDismiss: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const menu = ref.current!;
    const bounds = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(8, Math.min(anchor.x, innerWidth - bounds.width - 8))}px`;
    menu.style.top = `${Math.max(8, Math.min(anchor.y, innerHeight - bounds.height - 8))}px`;
    menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [anchor]);

  useEffect(() => {
    const outside = (event: Event) => {
      if (!ref.current?.contains(event.target as Node)) onDismiss();
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("scroll", outside, true);
    window.addEventListener("resize", onDismiss);
    window.addEventListener("blur", onDismiss);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("scroll", outside, true);
      window.removeEventListener("resize", onDismiss);
      window.removeEventListener("blur", onDismiss);
    };
  }, [onDismiss]);

  const dismiss = () => {
    onDismiss();
    document.getElementById(`tab-${anchor.id}`)?.focus({ preventScroll: true });
  };

  return createPortal(
    <div
      ref={ref}
      className="menu tab-context-menu"
      role="menu"
      aria-label="Tab actions"
      style={{ left: anchor.x, top: anchor.y }}
      onContextMenu={(event) => event.preventDefault()}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) onDismiss();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" || event.key === "Tab") {
          if (event.key === "Escape") event.preventDefault();
          event.stopPropagation();
          dismiss();
          return;
        }
        if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey)
          return;
        const items = Array.from(
          event.currentTarget.querySelectorAll<HTMLButtonElement>(
            "button:not(:disabled)",
          ),
        );
        const index = items.indexOf(
          document.activeElement as HTMLButtonElement,
        );
        let next: number;
        if (event.key === "ArrowDown") next = (index + 1) % items.length;
        else if (event.key === "ArrowUp")
          next = (index - 1 + items.length) % items.length;
        else if (event.key === "Home") next = 0;
        else if (event.key === "End") next = items.length - 1;
        else return;
        event.preventDefault();
        event.stopPropagation();
        items[next]?.focus();
      }}
    >
      <button
        type="button"
        role="menuitem"
        className="menu-item"
        tabIndex={-1}
        onClick={() => {
          const tab = tabs.find((tab) => tab.id === anchor.id);
          dismiss();
          if (tab) onRename(tab);
        }}
      >
        Rename tab…
      </button>
      <div className="menu-divider" role="separator" />
      {actions.map(({ action, label, divider }) => (
        <Fragment key={action}>
          {divider && <div className="menu-divider" role="separator" />}
          <button
            type="button"
            role="menuitem"
            className="menu-item"
            tabIndex={-1}
            disabled={!tabsToClose(tabs, anchor.id, action, modified).length}
            onClick={() => {
              dismiss();
              onClose(anchor.id, action);
              requestAnimationFrame(() => {
                if (document.activeElement === document.body)
                  document
                    .querySelector<HTMLElement>(
                      "[role='tab'][aria-selected='true']",
                    )
                    ?.focus();
              });
            }}
          >
            {label}
          </button>
        </Fragment>
      ))}
    </div>,
    document.body,
  );
}
