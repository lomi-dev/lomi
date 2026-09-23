import { useEffect, useId, useLayoutEffect, useRef } from "react";
import type { ButtonHTMLAttributes, ReactNode, RefObject } from "react";
import { ChevronRight, CircleAlert, Minus, Square, X } from "./icons";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { errorMessage, macOS, native } from "./api";
import { useProtectedTheme } from "./useProtectedTheme";

export function DisclosureSummary({ children }: { children: ReactNode }) {
  return (
    <summary className="disclosure-summary">
      <ChevronRight
        size={14}
        className="disclosure-summary-icon"
        aria-hidden="true"
      />
      <span className="disclosure-summary-content">{children}</span>
    </summary>
  );
}

export function IconButton({
  title,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { title: string }) {
  return (
    <button
      type="button"
      className="icon-button"
      title={title}
      aria-label={title}
      {...props}
    >
      {children}
    </button>
  );
}

export function WindowControls({
  onError,
}: {
  onError: (message: string) => void;
}) {
  if (native && macOS) return null;
  const act = (action: "minimize" | "toggleMaximize" | "close") => {
    if (native)
      void getCurrentWindow()
        [action]()
        .catch((error) => onError(errorMessage(error)));
  };
  return (
    <div className="window-controls">
      <IconButton
        title="Minimize window"
        disabled={!native}
        onClick={() => act("minimize")}
      >
        <Minus size={14} iconId="chrome-minimize" />
      </IconButton>
      <IconButton
        title="Maximize or restore window"
        disabled={!native}
        onClick={() => act("toggleMaximize")}
      >
        <Square size={12} iconId="chrome-maximize" />
      </IconButton>
      <IconButton
        title="Close window"
        disabled={!native}
        onClick={() => act("close")}
      >
        <X size={15} iconId="chrome-close" />
      </IconButton>
    </div>
  );
}

export function Modal({
  title,
  children,
  onClose,
  wide = false,
  className = "",
  descriptionId,
  initialFocus,
  tone,
  protectTheme = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
  className?: string;
  descriptionId?: string;
  initialFocus?: RefObject<HTMLElement | null>;
  tone?: "warning" | "danger";
  protectTheme?: boolean;
}) {
  useProtectedTheme(protectTheme);
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useLayoutEffect(() => {
    const element = dialog.current;
    element?.showModal();
    initialFocus?.current?.focus();
    // Close before DOM removal so the dialog can restore focus to its trigger.
    return () => element?.close();
  }, [initialFocus]);
  return (
    <dialog
      ref={dialog}
      className={`modal${wide ? " modal-wide" : ""}${className ? ` ${className}` : ""}`}
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      data-tone={tone}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="modal-surface">
        <header>
          {tone && (
            <span className="modal-symbol" aria-hidden="true">
              <CircleAlert size={21} />
            </span>
          )}
          <h2 id={titleId}>{title}</h2>
          <IconButton title="Close dialog" onClick={onClose}>
            <X size={16} iconId="dialog-close" />
          </IconButton>
        </header>
        {children}
      </div>
    </dialog>
  );
}

export function Menu({
  children,
  onClose,
  className = "",
}: {
  children: ReactNode;
  onClose: () => void;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const close = (event: PointerEvent) => {
      if (
        !ref.current?.contains(event.target as Node) &&
        !(event.target as Element).closest?.("[data-menu-trigger]")
      )
        onClose();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [onClose]);
  return (
    <div ref={ref} className={`menu ${className}`}>
      {children}
    </div>
  );
}
