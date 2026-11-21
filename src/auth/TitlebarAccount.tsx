import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Bell, LogOut } from "lucide-react";
import { api, errorMessage, native } from "../api";
import ContextMenu from "../ContextMenu";
import NotificationCenter, { useNotifications } from "../NotificationCenter";
import { notificationBadge } from "../notifications";
import { ChevronDown, ExternalLink, Github, Settings } from "../icons";
import { authStatusLabel, newestAuthState, unavailableState } from "./model";
import type { AuthState } from "./model";
import "./titlebar-account.css";

export default function TitlebarAccount({
  onError,
  onOpenSettings,
}: {
  onError: (message: string) => void;
  onOpenSettings: () => void;
}) {
  const [state, setState] = useState<AuthState | null>(
    native ? null : unavailableState,
  );
  const current = useRef(state);
  const mounted = useRef(false);
  const pending = useRef(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const notifications = useNotifications();
  const [inboxOpen, setInboxOpen] = useState(false);
  const closeInbox = useCallback(() => setInboxOpen(false), []);
  const [opening, setOpening] = useState(false);
  const [failedAvatar, setFailedAvatar] = useState<string | null>(null);
  const accept = useCallback((incoming: AuthState) => {
    const next = newestAuthState(current.current, incoming);
    if (next !== current.current) {
      const hadAccount = Boolean(
        current.current?.user || current.current?.session,
      );
      const hasAccount = Boolean(next?.user || next?.session);
      current.current = next;
      setState(next);
      if (hadAccount !== hasAccount) setMenu(null);
    }
    return next;
  }, []);

  useEffect(() => {
    mounted.current = true;
    let active = true;
    let stop: (() => void) | undefined;

    if (!native) {
      return () => {
        active = false;
        mounted.current = false;
      };
    }

    void (async () => {
      try {
        const unlisten = await listen<AuthState>(
          "auth-state-changed",
          ({ payload }) => {
            if (active) accept(payload);
          },
        );
        if (!active) {
          unlisten();
          return;
        }
        stop = unlisten;
        const snapshot = await api<AuthState>("auth_get_state");
        if (active) accept(snapshot);
      } catch {
        if (active) accept(unavailableState);
      }
    })();

    return () => {
      active = false;
      mounted.current = false;
      stop?.();
    };
  }, [accept]);

  const account = state?.user ?? null;
  const hasAccount = Boolean(account || state?.session);
  const login = account?.githubLogin?.trim() || null;
  const displayName = account?.displayName.trim() || login || "";
  const initials = displayName
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0])
    .join("")
    .toLocaleUpperCase();
  const status = state ? authStatusLabel(state.status) : "Checking account…";
  const buttonLabel = account
    ? `Open account menu for ${displayName || "your account"}`
    : hasAccount
      ? "Open account menu"
      : "Sign in";
  const title = opening
    ? hasAccount
      ? "Updating account…"
      : "Starting sign-in…"
    : !native
      ? "Sign-in is available in the Lomi desktop app"
      : hasAccount
        ? `${buttonLabel} · ${status}`
        : `Sign in to your Lomi account · ${status}`;

  const showMenu = () => {
    if (!native || pending.current || !trigger.current) return;
    setInboxOpen(false);
    const bounds = trigger.current.getBoundingClientRect();
    setMenu({ x: bounds.right, y: bounds.bottom + 6 });
  };

  const toggleMenu = () => {
    if (!native || pending.current) return;
    if (menu) setMenu(null);
    else showMenu();
  };

  const runAccountAction = async (
    command: "open_settings" | "auth_open_account_portal" | "auth_sign_out",
  ) => {
    if (!native || pending.current) return;
    pending.current = true;
    setOpening(true);
    try {
      if (command === "open_settings") {
        await api(command, { page: "account" });
      } else {
        const result = await api<AuthState>(command);
        if (!mounted.current) return;
        const latest = accept(result);
        if (
          command === "auth_sign_out" &&
          latest?.revision === result.revision
        ) {
          if (result.remoteRevocationConfirmed === false) {
            const localMessage =
              result.message ||
              (result.status === "signed-out"
                ? "Signed out on this device."
                : "");
            onError(
              `${localMessage} The server could not confirm that its session was revoked. You can remove it from your account in a browser.`.trim(),
            );
          } else if (result.message) onError(result.message);
        }
      }
    } catch (error) {
      if (mounted.current) onError(errorMessage(error));
    } finally {
      pending.current = false;
      if (mounted.current) setOpening(false);
    }
  };

  const signIn = () => {
    if (!native || pending.current) return;
    if (
      current.current?.status === "storage-locked" &&
      !current.current.user &&
      !current.current.session
    ) {
      void runAccountAction("open_settings");
      return;
    }
    pending.current = true;
    setOpening(true);
    void (async () => {
      try {
        let attempt =
          current.current?.status === "authorizing"
            ? current.current.attempt
            : null;
        if (!attempt) {
          const started = await api<AuthState>("auth_begin_login", {
            storage: "persistent",
          });
          if (!mounted.current) return;
          const latest = accept(started);
          if (latest && latest.revision > started.revision) return;
          if (
            started.status === "storage-locked" &&
            latest?.status === "storage-locked" &&
            latest.revision === started.revision
          ) {
            await api("open_settings", { page: "account" });
            return;
          }
          if (started.status === "unavailable" || !started.attempt) {
            if (
              started.status === "unavailable" ||
              started.status === "error"
            ) {
              onError(started.message || "Sign-in is not available right now.");
            }
            return;
          }
          if (
            latest?.status !== "authorizing" ||
            latest.attempt?.id !== started.attempt.id
          ) {
            return;
          }
          attempt = latest.attempt;
        }

        if (!attempt || !mounted.current) return;
        const latest = current.current;
        if (
          latest?.status !== "authorizing" ||
          latest.attempt?.id !== attempt.id
        ) {
          return;
        }

        const opened = await api<AuthState>("auth_open_verification", {
          attemptId: attempt.id,
        });
        if (!mounted.current) return;
        const accepted = accept(opened);
        if (
          opened.status === "unavailable" &&
          !opened.attempt &&
          accepted === opened
        ) {
          onError(opened.message || "The sign-in page could not be opened.");
        }
      } catch (error) {
        if (mounted.current) onError(errorMessage(error));
      } finally {
        pending.current = false;
        if (mounted.current) setOpening(false);
      }
    })();
  };

  return (
    <>
      <div
        className={`titlebar-account-control${hasAccount ? " titlebar-account-control-avatar" : ""}${menu ? " titlebar-account-menu-open" : ""}`}
      >
        {hasAccount ? (
          <button
            ref={trigger}
            type="button"
            className="titlebar-account titlebar-account-avatar"
            title={title}
            aria-label={`${buttonLabel}${notifications.unread ? `, ${notifications.unread} unread notifications` : ""}`}
            aria-busy={opening || undefined}
            aria-haspopup="menu"
            aria-expanded={Boolean(menu) || inboxOpen}
            disabled={!native || opening}
            onClick={toggleMenu}
            onContextMenu={(event) => {
              event.preventDefault();
              showMenu();
            }}
            onKeyDown={(event) => {
              if (
                event.key === "ArrowDown" ||
                event.key === "ContextMenu" ||
                (event.shiftKey && event.key === "F10")
              ) {
                event.preventDefault();
                event.stopPropagation();
                showMenu();
              }
            }}
          >
            <span className="titlebar-account-image" aria-hidden="true">
              {login && failedAvatar !== login ? (
                <img
                  src={`https://avatars.githubusercontent.com/${encodeURIComponent(login)}?s=64`}
                  referrerPolicy="no-referrer"
                  alt=""
                  onError={() => setFailedAvatar(login)}
                />
              ) : initials ? (
                <span>{initials}</span>
              ) : (
                <Github size={13} />
              )}
            </span>
            {notifications.unread > 0 && (
              <span
                className="notification-badge titlebar-account-badge"
                aria-hidden="true"
              >
                {notificationBadge(notifications.unread)}
              </span>
            )}
          </button>
        ) : (
          <>
            <button
              type="button"
              className="titlebar-account titlebar-account-sign-in"
              title={title}
              aria-label="Sign in"
              aria-busy={opening || undefined}
              disabled={!native || opening}
              onClick={signIn}
            >
              Sign in
            </button>
            <button
              ref={trigger}
              type="button"
              className="titlebar-account titlebar-account-menu-trigger"
              title={`Open account menu · ${status}`}
              aria-label="Open account menu"
              aria-haspopup="menu"
              aria-expanded={Boolean(menu) || inboxOpen}
              disabled={!native || opening}
              onClick={toggleMenu}
              onContextMenu={(event) => {
                event.preventDefault();
                showMenu();
              }}
              onKeyDown={(event) => {
                if (
                  event.key === "ArrowDown" ||
                  event.key === "ContextMenu" ||
                  (event.shiftKey && event.key === "F10")
                ) {
                  event.preventDefault();
                  event.stopPropagation();
                  showMenu();
                }
              }}
            >
              <ChevronDown size={14} aria-hidden="true" />
            </button>
          </>
        )}
      </div>
      {inboxOpen && (
        <NotificationCenter
          trigger={trigger}
          notifications={notifications}
          onClose={closeInbox}
        />
      )}
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          className="account-context-menu"
          align="end"
          trigger={trigger}
          label="Account menu"
          onClose={() => setMenu(null)}
          actions={[
            {
              label: "Settings",
              icon: <Settings size={15} aria-hidden="true" />,
              run: onOpenSettings,
            },
            {
              label: "Notifications",
              description: notifications.unread
                ? `${notifications.unread} unread notifications`
                : "No unread notifications",
              icon: <Bell size={15} aria-hidden="true" />,
              badge:
                notifications.unread > 0 ? (
                  <span
                    className="notification-badge"
                    aria-label={`${notifications.unread} unread notifications`}
                  >
                    {notificationBadge(notifications.unread)}
                  </span>
                ) : undefined,
              run: () => setInboxOpen(true),
            },
            ...(hasAccount || state?.status === "storage-locked"
              ? [
                  null,
                  {
                    label: "Account settings",
                    icon: <Settings size={15} aria-hidden="true" />,
                    run: () => void runAccountAction("open_settings"),
                  },
                  ...(hasAccount
                    ? [
                        {
                          label: "Manage account",
                          icon: <ExternalLink size={15} aria-hidden="true" />,
                          run: () =>
                            void runAccountAction("auth_open_account_portal"),
                        },
                      ]
                    : []),
                  null,
                  {
                    label: "Sign out",
                    icon: <LogOut size={15} aria-hidden="true" />,
                    run: () => void runAccountAction("auth_sign_out"),
                  },
                ]
              : []),
          ]}
        />
      )}
    </>
  );
}
