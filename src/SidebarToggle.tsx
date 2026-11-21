import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import {
  Check,
  GitBranch,
  LayoutGrid,
  Layers,
  PanelLeft,
  PanelRight,
} from "./icons";
import type { SidebarPanel, SidebarSide } from "./model";
import { IconButton } from "./ui";
import { useT } from "./i18n";

export default function SidebarToggle({
  panel,
  side,
  active,
  disabled,
  title,
  className = "icon-button",
  children,
  onToggle,
  onMove,
}: {
  panel: SidebarPanel | "terminalOverview";
  side: SidebarSide;
  active: boolean;
  disabled?: boolean;
  title: string;
  className?: string;
  children?: ReactNode;
  onToggle: () => void;
  onMove: (side: SidebarSide) => void;
}) {
  const { t } = useT();
  const [anchor, setAnchor] = useState<{ x: number; y: number }>();
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const name =
    panel === "files"
      ? t("Explorer")
      : panel === "git"
        ? t("Source Control")
        : panel === "terminalOverview"
          ? t("Terminal overview")
          : panel === "workspaces"
            ? t("Workspaces")
            : title;
  const dismiss = () => {
    setAnchor(undefined);
    trigger.current?.focus({ preventScroll: true });
  };
  useLayoutEffect(() => {
    if (!anchor) return;
    const element = menu.current!;
    const bounds = element.getBoundingClientRect();
    element.style.left = `${Math.max(8, Math.min(anchor.x, innerWidth - bounds.width - 8))}px`;
    element.style.top = `${Math.max(8, Math.min(anchor.y, innerHeight - bounds.height - 8))}px`;
    element.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus();
  }, [anchor]);
  useEffect(() => {
    if (!anchor) return;
    const close = () => setAnchor(undefined);
    const outside = (event: Event) => {
      if (!menu.current?.contains(event.target as Node)) close();
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("scroll", outside, true);
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("scroll", outside, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
    };
  }, [anchor]);
  return (
    <>
      <IconButton
        title={title}
        className={className}
        data-sidebar-toggle={panel}
        aria-pressed={active}
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={!!anchor}
        onClick={onToggle}
        onContextMenu={(event) => {
          event.preventDefault();
          trigger.current = event.currentTarget;
          const bounds = event.currentTarget.getBoundingClientRect();
          setAnchor({
            x: event.clientX || bounds.left,
            y: event.clientY || bounds.top,
          });
        }}
        onKeyDown={(event) => {
          if (
            event.key === "ContextMenu" ||
            (event.shiftKey && event.key === "F10")
          ) {
            event.preventDefault();
            trigger.current = event.currentTarget;
            const bounds = event.currentTarget.getBoundingClientRect();
            setAnchor({ x: bounds.left, y: bounds.top });
          }
        }}
      >
        {children ??
          (panel === "terminalOverview" ? (
            <LayoutGrid size={15} />
          ) : panel === "workspaces" ? (
            <Layers size={15} />
          ) : panel === "git" ? (
            <GitBranch size={15} />
          ) : side === "left" ? (
            <PanelLeft size={15} />
          ) : (
            <PanelRight size={15} />
          ))}
      </IconButton>
      {anchor &&
        createPortal(
          <div
            ref={menu}
            className="menu sidebar-context-menu"
            role="menu"
            aria-label={t("{name} panel position", { name })}
            style={{ left: anchor.x, top: anchor.y }}
            onContextMenu={(event) => event.preventDefault()}
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget))
                setAnchor(undefined);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape" || event.key === "Tab") {
                if (event.key === "Escape") event.preventDefault();
                event.stopPropagation();
                dismiss();
              } else if (
                ["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)
              ) {
                event.preventDefault();
                const items = [
                  ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
                    "button",
                  ),
                ];
                const index = items.indexOf(
                  document.activeElement as HTMLButtonElement,
                );
                items[
                  event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? 1
                      : (index + 1) % 2
                ]?.focus();
              }
            }}
          >
            {(["left", "right"] as const).map((position) => (
              <button
                key={position}
                type="button"
                role="menuitemradio"
                className="menu-item"
                aria-checked={side === position}
                tabIndex={-1}
                onClick={() => {
                  dismiss();
                  onMove(position);
                }}
              >
                {position === "left" ? (
                  <PanelLeft size={15} />
                ) : (
                  <PanelRight size={15} />
                )}
                <span>
                  {position === "left"
                    ? t("Panel on the left")
                    : t("Panel on the right")}
                </span>
                {side === position && <Check size={14} />}
              </button>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
