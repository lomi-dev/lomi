import { useEffect, useRef } from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import { CircleAlert, Info, TriangleAlert, X } from "./icons";
import { IconButton } from "./ui";
import "./app-toast.css";

export function ToastViewport({ children }: { children: ReactNode }) {
  return createPortal(
    <section className="app-toasts" aria-label="Application messages">
      {children}
    </section>,
    document.body,
  );
}

export function AppToast({
  children,
  tone = "info",
  action,
  onDismiss,
  dismissLabel = "Dismiss message",
  duration,
  className = "",
}: {
  children: ReactNode;
  tone?: "info" | "warning" | "error";
  action?: ReactNode;
  onDismiss?: () => void;
  dismissLabel?: string;
  duration?: number;
  className?: string;
}) {
  const dismiss = useRef(onDismiss);
  const hovered = useRef(false);
  const focused = useRef(false);
  const timerControls = useRef<{
    pause: () => void;
    resume: () => void;
  } | null>(null);
  const timeout =
    tone === "error"
      ? undefined
      : (duration ?? (tone === "info" ? 5000 : undefined));

  useEffect(() => {
    dismiss.current = onDismiss;
  }, [onDismiss]);

  useEffect(() => {
    if (timeout === undefined || !Number.isFinite(timeout) || timeout <= 0)
      return;

    let remaining = timeout;
    let started = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let expired = false;
    const pause = () => {
      if (timer === undefined) return;
      clearTimeout(timer);
      timer = undefined;
      remaining = Math.max(0, remaining - (Date.now() - started));
    };
    const resume = () => {
      if (hovered.current || focused.current || timer !== undefined || expired)
        return;
      started = Date.now();
      timer = setTimeout(() => {
        timer = undefined;
        expired = true;
        dismiss.current?.();
      }, remaining);
    };
    timerControls.current = { pause, resume };
    resume();
    return () => {
      pause();
      timerControls.current = null;
    };
  }, [children, timeout]);

  const Icon =
    tone === "error" ? CircleAlert : tone === "warning" ? TriangleAlert : Info;

  return (
    <div
      className={`app-toast${className ? ` ${className}` : ""}`}
      data-tone={tone}
      onMouseEnter={() => {
        hovered.current = true;
        timerControls.current?.pause();
      }}
      onMouseLeave={() => {
        hovered.current = false;
        timerControls.current?.resume();
      }}
      onFocusCapture={() => {
        focused.current = true;
        timerControls.current?.pause();
      }}
      onBlurCapture={(event) => {
        if (event.currentTarget.contains(event.relatedTarget)) return;
        focused.current = false;
        timerControls.current?.resume();
      }}
    >
      <Icon className="app-toast-icon" size={16} aria-hidden="true" />
      <div className="app-toast-body">
        <div
          className="app-toast-message"
          role={tone === "error" ? "alert" : "status"}
          aria-atomic="true"
        >
          {children}
        </div>
        {action && <div className="app-toast-action">{action}</div>}
      </div>
      {onDismiss && (
        <IconButton
          className="icon-button app-toast-dismiss"
          title={dismissLabel}
          onClick={onDismiss}
        >
          <X size={14} aria-hidden="true" />
        </IconButton>
      )}
    </div>
  );
}
