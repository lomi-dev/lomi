import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, Columns2, Eye, FileCode } from "./icons";
import type { FilePreviewView } from "./model";
import { useT } from "./i18n";

export default function FilePreviewToggle({
  kind,
  view,
  onChange,
}: {
  kind: "Markdown" | "SVG";
  view: FilePreviewView;
  onChange: (view: FilePreviewView) => void;
}) {
  const { t } = useT();
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const dismiss = () => {
    setOpen(false);
    trigger.current?.focus({ preventScroll: true });
  };
  useLayoutEffect(() => {
    if (!open) return;
    const element = menu.current!;
    const bounds = element.getBoundingClientRect();
    const button = trigger.current!.getBoundingClientRect();
    element.style.left = `${Math.max(8, Math.min(button.right - bounds.width, innerWidth - bounds.width - 8))}px`;
    element.style.top = `${Math.max(8, button.top - bounds.height - 6)}px`;
    (
      element.querySelector<HTMLButtonElement>('[aria-checked="true"]') ??
      element.querySelector<HTMLButtonElement>("button")
    )?.focus();
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    const outside = (event: Event) => {
      if (
        !menu.current?.contains(event.target as Node) &&
        !trigger.current?.contains(event.target as Node)
      )
        close();
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
  }, [open]);
  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="button markdown-preview-toggle"
        aria-label={
          view === "editor"
            ? t("Preview {kind}", { kind })
            : t("Close {kind} preview", { kind })
        }
        title={
          view === "editor"
            ? t("Preview beside editor. Right-click for preview options.")
            : t("Back to editor. Right-click for preview options.")
        }
        aria-pressed={view !== "editor"}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          setOpen(false);
          onChange(view === "editor" ? "split" : "editor");
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          setOpen(true);
        }}
        onKeyDown={(event) => {
          if (
            event.key === "ContextMenu" ||
            event.key === "ArrowDown" ||
            (event.shiftKey && event.key === "F10")
          ) {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        {view === "editor" ? <Eye size={15} /> : <FileCode size={15} />}
        {view === "editor" ? t("Preview") : t("Editor")}
      </button>
      {open &&
        createPortal(
          <div
            ref={menu}
            className="menu markdown-preview-menu"
            role="menu"
            aria-label={t("{kind} preview options", { kind })}
            onContextMenu={(event) => event.preventDefault()}
            onBlur={(event) => {
              if (
                !event.currentTarget.contains(event.relatedTarget) &&
                !trigger.current?.contains(event.relatedTarget)
              )
                setOpen(false);
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
            {(["split", "preview"] as const).map((mode) => (
              <button
                key={mode}
                className="menu-item"
                role="menuitemradio"
                aria-checked={view === mode}
                tabIndex={-1}
                onClick={() => {
                  dismiss();
                  onChange(mode);
                }}
              >
                {mode === "split" ? <Columns2 size={15} /> : <Eye size={15} />}
                <span>
                  {mode === "split"
                    ? t("Preview beside editor")
                    : t("Preview only")}
                </span>
                {view === mode && <Check size={14} />}
              </button>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
