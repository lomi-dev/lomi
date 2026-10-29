import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type { RefObject } from "react";
import { createPortal } from "react-dom";
import {
  Bell,
  CircleAlert,
  CircleCheck,
  Info,
  Smartphone,
  Plug,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import {
  clearNotificationError,
  acceptNotificationSnapshot,
  getNotificationSnapshot,
  notificationAgent,
  notificationRelativeTime,
  notificationKindLabel,
  notificationSource,
  subscribeNotificationErrors,
  subscribeNotificationSnapshot,
  unreadNotificationCount,
} from "./notifications";
import type { NotificationSnapshot } from "./notifications";
import { CliAgentIcon } from "./CliAgentIcon";
import {
  notificationAction,
  clearUnsavedApplicationNotifications,
  dismissUnsavedApplicationNotification,
  hasPendingApplicationNotifications,
  notificationActionsRevision,
  reportAppNotification,
  retryApplicationNotifications,
  subscribeApplicationNotifications,
  subscribeNotificationActions,
  subscribeUnsavedApplicationNotifications,
  unsavedApplicationNotifications,
} from "./app-notifications";
import "./notification-center.css";

export function useNotifications() {
  const unsaved = useSyncExternalStore(
    subscribeUnsavedApplicationNotifications,
    unsavedApplicationNotifications,
  );
  const snapshot = useSyncExternalStore(
    subscribeNotificationSnapshot,
    getNotificationSnapshot,
  );
  const [error, setError] = useState<string | null>(null);
  const errorVersion = useRef(0);
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const mounted = useRef(false);
  const [attempt, setAttempt] = useState(0);
  const accept = useCallback((incoming: NotificationSnapshot) => {
    acceptNotificationSnapshot(incoming);
  }, []);
  useEffect(() => {
    mounted.current = true;
    let active = true;
    let stop: (() => void) | undefined;
    const stopErrors = subscribeNotificationErrors((message) => {
      errorVersion.current++;
      setError(message);
    });
    const stopApplications = subscribeApplicationNotifications(accept);
    const startedErrorVersion = ++errorVersion.current;
    const retrying = attempt > 0 && pending.current;
    if (native)
      void (async () => {
        try {
          const unlisten = await listen<NotificationSnapshot>(
            "notifications-changed",
            ({ payload }) => {
              if (active) accept(payload);
            },
          );
          if (!active) {
            unlisten();
            return;
          }
          stop = unlisten;
          if (attempt > 0) await retryApplicationNotifications();
          const incoming =
            await api<NotificationSnapshot>("load_notifications");
          if (active) {
            accept(incoming);
            if (errorVersion.current === startedErrorVersion && attempt > 0) {
              setError(null);
              clearNotificationError();
            }
          }
        } catch (failure) {
          if (active && errorVersion.current === startedErrorVersion)
            setError(`Could not load notifications: ${errorMessage(failure)}`);
        } finally {
          if (retrying && active) {
            pending.current = false;
            setBusy(false);
          }
        }
      })();
    return () => {
      active = false;
      mounted.current = false;
      stop?.();
      stopErrors();
      stopApplications();
    };
  }, [accept, attempt]);

  const mutate = async (command: string, args?: Record<string, unknown>) => {
    if (!native || pending.current) return false;
    pending.current = true;
    setBusy(true);
    const startedErrorVersion = ++errorVersion.current;
    try {
      const incoming = await api<NotificationSnapshot>(command, args);
      if (mounted.current) {
        accept(incoming);
        if (
          errorVersion.current === startedErrorVersion &&
          !hasPendingApplicationNotifications()
        ) {
          setError(null);
          clearNotificationError();
        }
      }
      return true;
    } catch (failure) {
      if (mounted.current && errorVersion.current === startedErrorVersion)
        setError(`Could not update notifications: ${errorMessage(failure)}`);
      return false;
    } finally {
      pending.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  return {
    snapshot,
    unsaved,
    error,
    busy,
    unread: unreadNotificationCount(snapshot) + unsaved.length,
    retry: () => {
      if (pending.current) return;
      if (hasPendingApplicationNotifications()) {
        pending.current = true;
        setBusy(true);
      }
      setAttempt((value) => value + 1);
    },
    markRead: (ids: string[]) =>
      void mutate("mark_notifications_read", { ids }),
    dismiss: (id: string) => {
      if (pending.current) return;
      if (id.startsWith("unsaved-")) dismissUnsavedApplicationNotification(id);
      else void mutate("dismiss_notification", { id });
    },
    clearAll: async () => {
      const startedErrorVersion = errorVersion.current;
      if (await mutate("clear_notifications")) {
        if (errorVersion.current === startedErrorVersion + 1) {
          clearUnsavedApplicationNotifications();
          setError(null);
          clearNotificationError();
        }
      }
    },
  };
}

type Notifications = ReturnType<typeof useNotifications>;
export default function NotificationCenter({
  trigger,
  notifications,
  onClose,
}: {
  trigger: RefObject<HTMLButtonElement | null>;
  notifications: Notifications;
  onClose: () => void;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const [position, setPosition] = useState({ left: 8, top: 8 });
  const [now, setNow] = useState(Date.now);
  useSyncExternalStore(
    subscribeNotificationActions,
    notificationActionsRevision,
  );
  const [actionBusy, setActionBusy] = useState(false);
  const actionPending = useRef(false);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const {
    snapshot,
    unsaved,
    error,
    busy,
    unread,
    retry,
    markRead,
    dismiss,
    clearAll,
  } = notifications;
  const items = [...unsaved, ...(snapshot?.items ?? [])];
  const recovery = notificationAction("recover-session");
  const runAction = (
    action: NonNullable<ReturnType<typeof notificationAction>>,
    source: "app" | "android" | "mcp" = "app",
  ) => {
    if (actionPending.current) return;
    actionPending.current = true;
    setActionBusy(true);
    void Promise.resolve()
      .then(() => action.run())
      .catch((reason) =>
        reportAppNotification({ body: errorMessage(reason), source }),
      )
      .finally(() => {
        actionPending.current = false;
        setActionBusy(false);
      });
  };
  const close = useCallback(
    (restoreFocus: boolean) => {
      onClose();
      if (restoreFocus)
        requestAnimationFrame(() =>
          trigger.current?.focus({ preventScroll: true }),
        );
    },
    [onClose, trigger],
  );
  useLayoutEffect(() => {
    if (!trigger.current || !panel.current) return;
    const anchor = trigger.current.getBoundingClientRect();
    const bounds = panel.current.getBoundingClientRect();
    setPosition({
      left: Math.max(
        8,
        Math.min(anchor.right - bounds.width, innerWidth - bounds.width - 8),
      ),
      top: Math.max(
        8,
        Math.min(anchor.bottom + 6, innerHeight - bounds.height - 8),
      ),
    });
  }, [trigger, snapshot, error, unsaved]);
  useLayoutEffect(() => {
    closeButton.current?.focus({ preventScroll: true });
  }, []);
  // A successful dismissal can remove the focused row; keep keyboard focus in
  // the overlay rather than leaving it on the document body.
  useLayoutEffect(() => {
    if (document.activeElement === document.body)
      closeButton.current?.focus({ preventScroll: true });
  }, [snapshot, busy, unsaved]);
  useEffect(() => {
    const outside = (event: Event) => {
      const target = event.target as Node;
      if (
        !panel.current?.contains(target) &&
        !trigger.current?.contains(target)
      )
        close(false);
    };
    const dismissPanel = () => close(false);
    document.addEventListener("pointerdown", outside);
    window.addEventListener("resize", dismissPanel);
    window.addEventListener("blur", dismissPanel);
    window.visualViewport?.addEventListener("resize", dismissPanel);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("resize", dismissPanel);
      window.removeEventListener("blur", dismissPanel);
      window.visualViewport?.removeEventListener("resize", dismissPanel);
    };
  }, [close, trigger]);
  return createPortal(
    <div
      ref={panel}
      className="menu notification-center"
      role="dialog"
      aria-label="Notifications"
      style={position}
      onContextMenu={(event) => event.preventDefault()}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          close(true);
        }
        if (event.key !== "Tab") return;
        const focusable = [
          ...event.currentTarget.querySelectorAll<HTMLElement>(
            'button:not(:disabled), [tabindex="0"]',
          ),
        ];
        const index = focusable.indexOf(document.activeElement as HTMLElement);
        event.preventDefault();
        const next = event.shiftKey
          ? index <= 0
            ? focusable.length - 1
            : index - 1
          : (index + 1) % focusable.length;
        focusable[next]?.focus();
        event.stopPropagation();
      }}
    >
      <div className="notification-center-heading">
        <div>
          <strong>Notifications</strong>
          <span>
            {!snapshot
              ? "Loading…"
              : unread
                ? `${unread} unread`
                : "All caught up"}
          </span>
        </div>
        <button
          ref={closeButton}
          type="button"
          className="notification-icon-button"
          aria-label="Close notifications"
          onClick={() => close(true)}
        >
          <X size={15} aria-hidden="true" />
        </button>
      </div>
      {error && (
        <div className="notification-center-error" role="status">
          <span>{error}</span>
          <button type="button" disabled={busy} onClick={retry}>
            Retry
          </button>
        </div>
      )}
      <div
        className="notification-center-list"
        role="region"
        aria-label="Notification list"
        tabIndex={0}
        aria-busy={busy || undefined}
      >
        {!items.length && (
          <div className="notification-center-empty">
            <Bell size={24} aria-hidden="true" />
            <strong>
              {snapshot
                ? "No notifications yet"
                : error
                  ? "Notifications unavailable"
                  : "Loading notifications…"}
            </strong>
            <span>Application and agent updates will appear here.</span>
          </div>
        )}
        {items.map((item) => {
          const agent = notificationAgent(item);
          const notSaved = item.id.startsWith("unsaved-");
          const date = new Date(item.createdAt);
          const validDate = Number.isFinite(date.getTime());
          const action = item.action
            ? notificationAction(item.action)
            : undefined;
          const SourceIcon =
            item.source === "android"
              ? Smartphone
              : item.source === "mcp"
                ? Plug
                : item.source === "app"
                  ? Bell
                  : Terminal;
          return (
            <article
              className="notification-item"
              key={item.id}
              data-unread={!item.read || undefined}
              data-unsaved={notSaved || undefined}
            >
              <button
                type="button"
                className="notification-item-open"
                disabled={busy}
                aria-label={`${item.title}${item.read ? "" : ", unread"}`}
                aria-description={`${notificationSource(item)}. ${notificationKindLabel(item)}. ${notificationRelativeTime(item.createdAt, now)}. ${item.body}`}
                title={item.body}
                onClick={() => {
                  if (!item.read && !notSaved) markRead([item.id]);
                }}
              >
                <span className="notification-agent">
                  {agent ? (
                    <CliAgentIcon cli={agent} />
                  ) : (
                    <SourceIcon size={23} aria-hidden="true" />
                  )}
                  <span
                    className="notification-kind"
                    data-kind={item.kind}
                    aria-label={notificationKindLabel(item)}
                  >
                    {item.kind === "attention" ? (
                      <CircleAlert size={13} aria-hidden="true" />
                    ) : item.kind === "info" ? (
                      <Info size={13} aria-hidden="true" />
                    ) : (
                      <CircleCheck size={13} aria-hidden="true" />
                    )}
                  </span>
                </span>
                <strong>{item.title}</strong>
                <time
                  dateTime={validDate ? date.toISOString() : undefined}
                  title={validDate ? date.toLocaleString() : "Unknown time"}
                >
                  {notificationRelativeTime(item.createdAt, now)}
                </time>
                <span className="notification-source">
                  {notificationSource(item)}
                  {notSaved && <span> · Not saved yet</span>}
                  {item.body && (
                    <span className="notification-context"> · {item.body}</span>
                  )}
                </span>
              </button>
              {action && (
                <button
                  type="button"
                  className="notification-item-action"
                  disabled={busy || actionBusy}
                  onClick={() => runAction(action, item.source ?? "app")}
                >
                  {action.label}
                </button>
              )}
              <button
                type="button"
                className="notification-item-dismiss notification-icon-button"
                disabled={busy}
                aria-label={`Dismiss ${item.title}`}
                title="Dismiss notification"
                onClick={() => dismiss(item.id)}
              >
                <X size={14} aria-hidden="true" />
              </button>
            </article>
          );
        })}
      </div>
      <div className="notification-center-footer">
        {recovery &&
          !items.some((item) => item.action === "recover-session") && (
            <>
              <span>Session recovery required</span>
              <button
                type="button"
                disabled={busy || actionBusy}
                onClick={() => runAction(recovery)}
              >
                {recovery.label}
              </button>
            </>
          )}
        <button
          type="button"
          disabled={busy || !items.length}
          onClick={clearAll}
        >
          <Trash2 size={14} aria-hidden="true" />
          Clear all notifications
        </button>
      </div>
    </div>,
    document.body,
  );
}
