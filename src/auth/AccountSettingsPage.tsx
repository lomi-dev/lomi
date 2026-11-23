import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api, errorMessage, native } from "../api";
import { useT } from "../i18n";
import {
  Bell,
  Copy,
  ExternalLink,
  Github,
  Globe,
  Lock,
  Monitor,
  RefreshCw,
  Send,
} from "../icons";
import type { SettingsPageId } from "../settings-pages";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "../settings-ui";
import { Modal } from "../ui";
import { authStatusLabel, newestAuthState, unavailableState } from "./model";
import type { AccountSession, AuthState, RevokeOutcome } from "./model";
import "./styles.css";

type AuthCommand =
  | "auth_begin_login"
  | "auth_open_verification"
  | "auth_cancel_login"
  | "auth_refresh_state"
  | "auth_sign_out"
  | "auth_open_account_portal";

export default function AccountSettingsPage({
  remoteAvailable = false,
  onNavigate,
}: {
  remoteAvailable?: boolean;
  onNavigate?: (page: SettingsPageId) => void;
}) {
  const { t, locale } = useT();
  const [state, setState] = useState<AuthState | null>(
    native ? null : unavailableState,
  );
  const snapshot = useRef(state);
  const mounted = useRef(false);
  const pending = useRef(false);
  const [ready, setReady] = useState(!native);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [toast, setToast] = useState("");
  const [remember, setRemember] = useState(true);
  const [sessions, setSessions] = useState<AccountSession[] | null>(null);
  const [sessionsError, setSessionsError] = useState("");
  const [sessionsBusy, setSessionsBusy] = useState(false);
  const [revoking, setRevoking] = useState<string | null>(null);
  const [reauthRequired, setReauthRequired] = useState(false);
  const [confirmRevokeAll, setConfirmRevokeAll] = useState(false);
  const [failedAvatar, setFailedAvatar] = useState<string | null>(null);
  const cancelRevokeAll = useRef<HTMLButtonElement>(null);
  const accept = useCallback((value: AuthState) => {
    if (!mounted.current) return;
    const next = newestAuthState(snapshot.current, value);
    if (next !== snapshot.current) {
      snapshot.current = next;
      setState(next);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!native)
      return () => {
        mounted.current = false;
      };
    let current = true;
    let stop: (() => void) | undefined;
    void (async () => {
      try {
        const unlisten = await listen<AuthState>(
          "auth-state-changed",
          ({ payload }) => {
            if (current) accept(payload);
          },
        );
        if (!current) {
          unlisten();
          return;
        }
        stop = unlisten;
        const value = await api<AuthState>("auth_get_state");
        if (current) accept(value);
      } catch (reason) {
        if (current) setError(errorMessage(reason));
      } finally {
        if (current) setReady(true);
      }
    })();
    return () => {
      current = false;
      mounted.current = false;
      stop?.();
    };
  }, [accept]);

  const status = state?.status;
  const signedIn = status === "signed-in" && Boolean(state?.session);
  const sessionId = signedIn ? (state?.session?.id ?? null) : null;

  const loadSessions = useCallback(async () => {
    if (!native) return;
    setSessionsBusy(true);
    setSessionsError("");
    try {
      const list = await api<AccountSession[]>("auth_list_sessions");
      if (mounted.current) setSessions(list);
    } catch (reason) {
      if (mounted.current) setSessionsError(errorMessage(reason));
    } finally {
      if (mounted.current) setSessionsBusy(false);
    }
  }, []);

  useEffect(() => {
    setSessions(null);
    setSessionsError("");
    setReauthRequired(false);
    if (sessionId) void loadSessions();
  }, [sessionId, loadSessions]);

  const run = async (
    command: AuthCommand,
    args?: Record<string, unknown>,
    openVerification = false,
  ) => {
    if (!native || pending.current) return;
    pending.current = true;
    setBusy(true);
    setError("");
    try {
      const value = await api<AuthState>(command, args);
      accept(value);
      if (
        openVerification &&
        mounted.current &&
        value.attempt &&
        snapshot.current?.status === "authorizing" &&
        snapshot.current.attempt?.id === value.attempt.id
      ) {
        accept(
          await api<AuthState>("auth_open_verification", {
            attemptId: value.attempt.id,
          }),
        );
      }
    } catch (reason) {
      if (mounted.current) setError(errorMessage(reason));
    } finally {
      pending.current = false;
      if (mounted.current) setBusy(false);
    }
  };

  // A fresh sign-in satisfies the server's 15-minute rule for revoking other sessions.
  const signInAgain = async () => {
    const storage = state?.storage ?? "persistent";
    await run("auth_sign_out");
    if (snapshot.current?.status === "signed-out")
      await run("auth_begin_login", { storage }, true);
  };

  const revoke = async (id: string) => {
    if (!native || revoking) return;
    setRevoking(id);
    setSessionsError("");
    try {
      const outcome = await api<RevokeOutcome>("auth_revoke_session", {
        sessionId: id,
      });
      if (!mounted.current) return;
      if (outcome === "reauth-required") setReauthRequired(true);
      else {
        setSessions((list) => list?.filter((item) => item.id !== id) ?? null);
        setToast(t("Session revoked"));
      }
    } catch (reason) {
      if (mounted.current) setSessionsError(errorMessage(reason));
    } finally {
      if (mounted.current) setRevoking(null);
    }
  };

  const revokeAll = async () => {
    if (!native || revoking) return;
    setRevoking("all");
    setSessionsError("");
    try {
      const outcome = await api<RevokeOutcome>("auth_revoke_all_sessions");
      if (!mounted.current) return;
      setConfirmRevokeAll(false);
      if (outcome === "reauth-required") setReauthRequired(true);
      else setToast(t("Signed out everywhere"));
    } catch (reason) {
      if (mounted.current) setSessionsError(errorMessage(reason));
    } finally {
      if (mounted.current) setRevoking(null);
    }
  };

  const copy = async (value: string, label: string) => {
    try {
      await writeText(value);
      setToast(t("{label} copied", { label }));
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const formatDate = (value: string, withTime = true) => {
    const date = new Date(value);
    if (!Number.isFinite(date.getTime())) return value;
    return date.toLocaleString(locale, {
      dateStyle: "medium",
      ...(withTime ? { timeStyle: "short" } : {}),
    });
  };
  const formatRelative = (value: string) => {
    const seconds = (new Date(value).getTime() - Date.now()) / 1000;
    if (!Number.isFinite(seconds)) return value;
    const format = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
    const abs = Math.abs(seconds);
    if (abs < 60) return format.format(0, "second");
    if (abs < 3600) return format.format(Math.round(seconds / 60), "minute");
    if (abs < 86400) return format.format(Math.round(seconds / 3600), "hour");
    return format.format(Math.round(seconds / 86400), "day");
  };

  const attempt = state?.attempt;
  const checking = status === "checking";
  const canLogin =
    ready &&
    native &&
    state !== null &&
    status !== "unavailable" &&
    status !== "authorizing" &&
    !checking &&
    status !== "signed-in" &&
    status !== "offline";
  const user = state?.user ?? null;
  const hasAccount = Boolean(user || state?.session);
  const canSignOut =
    native &&
    ready &&
    (hasAccount ||
      status === "storage-locked" ||
      status === "offline" ||
      status === "error" ||
      status === "checking");
  const expiry = attempt ? new Date(attempt.expiresAt) : null;
  const expiryText =
    expiry && Number.isFinite(expiry.getTime())
      ? expiry.toLocaleTimeString(locale, {
          hour: "2-digit",
          minute: "2-digit",
        })
      : null;
  const login = user?.githubLogin?.trim() || null;
  const displayName = user?.displayName.trim() || login || "";
  const initials = displayName
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0])
    .join("")
    .toLocaleUpperCase();
  const badgeState =
    status === "signed-in"
      ? "ready"
      : status === "authorizing" || checking
        ? "waiting"
        : undefined;
  const portalAvailable = native && ready && status !== "unavailable";
  const otherSessions = sessions?.filter((item) => !item.isCurrent) ?? [];
  const sortedSessions =
    sessions &&
    [...sessions].sort((a, b) => Number(b.isCurrent) - Number(a.isCurrent));

  const retryAction = native && ready && status !== "authorizing" && (
    <button
      className="text-button"
      disabled={busy}
      onClick={() => void run("auth_refresh_state")}
    >
      {status === "storage-locked" ? t("Retry access") : t("Retry connection")}
    </button>
  );

  const portalButton = portalAvailable && (
    <button
      type="button"
      className="button"
      disabled={busy}
      onClick={() => void run("auth_open_account_portal")}
    >
      <ExternalLink size={14} aria-hidden="true" />
      {t("Manage account")}
    </button>
  );
  const signOutButton = canSignOut && (
    <button
      type="button"
      className="button"
      disabled={busy}
      onClick={() => void run("auth_sign_out")}
    >
      {t("Sign out of this device")}
    </button>
  );

  const sessionKind = (kind: AccountSession["kind"]) =>
    kind === "desktop"
      ? { Icon: Monitor, label: t("Lomi desktop app") }
      : kind === "browser"
        ? { Icon: Globe, label: t("Web browser") }
        : { Icon: Lock, label: t("Other sign-in") };

  return (
    <SettingsPage
      title={t("Account")}
      busy={busy || !ready}
      status={toast}
      className="account-settings"
    >
      {error && (
        <SettingsNotice
          tone="error"
          action={
            status !== "offline" && status !== "storage-locked" && retryAction
          }
        >
          {error}
        </SettingsNotice>
      )}
      {state?.message && (
        <SettingsNotice
          tone={
            status === "error" || status === "storage-locked"
              ? "warning"
              : "info"
          }
          role="status"
          action={status === "error" && !error && retryAction}
        >
          {state.message}
        </SettingsNotice>
      )}
      {!native && (
        <SettingsNotice>
          {t("Account sign-in is available in the desktop app.")}
        </SettingsNotice>
      )}
      {native && status === "unavailable" && !state?.message && (
        <SettingsNotice>
          {t("Account sign-in is not available in this version.")}
        </SettingsNotice>
      )}
      {status === "offline" && (
        <SettingsNotice tone="warning" action={retryAction}>
          {t(
            "Your account could not be checked. Online services will resume after a successful connection. Your local work is unaffected.",
          )}
        </SettingsNotice>
      )}
      {status === "storage-locked" && (
        <SettingsNotice tone="warning" role="status" action={retryAction}>
          {t(
            "Unlock your system credential store and retry access. To replace the saved sign-in, choose Sign out of this device. To sign in only until Lomi closes, turn off Remember me on this device.",
          )}
        </SettingsNotice>
      )}
      {status === "error" && !state?.message && !error && (
        <SettingsNotice tone="warning" action={retryAction}>
          {t(
            "Your account could not be checked. Retry the connection to restore account access. Your local work is unaffected.",
          )}
        </SettingsNotice>
      )}
      {state?.remoteRevocationConfirmed === false && !hasAccount && (
        <SettingsNotice tone="warning" role="status">
          {status === "signed-out" && t("Signed out on this device.") + " "}
          {t(
            "The server could not confirm that its session was revoked. You can remove it from your account in a browser.",
          )}
        </SettingsNotice>
      )}

      {user && (
        <SettingsSection title={t("Your account")}>
          <div className="account-profile">
            <span className="account-avatar" aria-hidden="true">
              {login && failedAvatar !== login ? (
                <img
                  src={`https://avatars.githubusercontent.com/${encodeURIComponent(login)}?s=128`}
                  alt=""
                  referrerPolicy="no-referrer"
                  onError={() => setFailedAvatar(login)}
                />
              ) : (
                initials || <Github size={22} />
              )}
            </span>
            <div className="account-profile-text">
              <strong>{displayName}</strong>
              {login && <span>@{login}</span>}
            </div>
            {status && (
              <span className="agent-control-badge" data-state={badgeState}>
                {authStatusLabel(status)}
              </span>
            )}
          </div>
          {user.email && (
            <SettingRow label={t("Email")}>
              <span className="setting-value account-email">{user.email}</span>
              <CopyButton
                label={t("Copy email")}
                onClick={() => void copy(user.email, t("Email"))}
              />
            </SettingRow>
          )}
          <SettingRow
            label={t("Account ID")}
            description={t("Share this with Lomi support if they ask for it.")}
          >
            <code className="setting-value account-id">{user.id}</code>
            <CopyButton
              label={t("Copy account ID")}
              onClick={() => void copy(user.id, t("Account ID"))}
            />
          </SettingRow>
        </SettingsSection>
      )}

      {hasAccount && (
        <SettingsSection
          title={t("This device")}
          description={t(
            "Signing out leaves your projects, files, and AI connections on this device.",
          )}
          actions={
            <>
              <button
                type="button"
                className="icon-button"
                aria-label={t("Check now")}
                title={t("Check now")}
                disabled={busy || checking}
                onClick={() => void run("auth_refresh_state")}
              >
                <RefreshCw size={14} aria-hidden="true" />
              </button>
              {signOutButton}
            </>
          }
        >
          {!user && status && (
            <SettingRow label={t("Status")}>
              <span className="agent-control-badge" data-state={badgeState}>
                {authStatusLabel(status)}
              </span>
            </SettingRow>
          )}
          {state?.storage && (
            <SettingRow
              label={t("Sign-in storage")}
              description={
                state.storage === "session"
                  ? t("You will need to sign in again after closing Lomi.")
                  : t("Kept in the system credential store between launches.")
              }
            >
              <span className="setting-value">
                {state.storage === "session"
                  ? t("This session only")
                  : t("Credential store")}
              </span>
            </SettingRow>
          )}
          {state?.session && (
            <SettingRow label={t("Session expires")}>
              <span className="setting-value">
                {formatDate(state.session.expiresAt)}
                <span className="account-muted">
                  {" · "}
                  {formatRelative(state.session.expiresAt)}
                </span>
              </span>
            </SettingRow>
          )}
        </SettingsSection>
      )}

      {signedIn && (
        <SettingsSection
          title={t("Sessions")}
          count={sessions?.length}
          description={t(
            "Everywhere your Lomi account is signed in. Revoke a session you do not recognize.",
          )}
          actions={
            <button
              type="button"
              className="icon-button"
              aria-label={t("Refresh sessions")}
              title={t("Refresh sessions")}
              disabled={sessionsBusy}
              onClick={() => void loadSessions()}
            >
              <RefreshCw
                size={14}
                aria-hidden="true"
                className={sessionsBusy ? "account-spin" : undefined}
              />
            </button>
          }
        >
          {reauthRequired && (
            <SettingsNotice
              tone="warning"
              role="status"
              action={
                <button
                  className="text-button"
                  disabled={busy}
                  onClick={() => void signInAgain()}
                >
                  {t("Sign in again")}
                </button>
              }
            >
              {t(
                "For your security, revoking sessions requires a sign-in from the last 15 minutes. Sign in again on this device, then retry.",
              )}
            </SettingsNotice>
          )}
          {sessionsError && (
            <SettingsNotice
              tone="error"
              action={
                <button
                  className="text-button"
                  disabled={sessionsBusy}
                  onClick={() => void loadSessions()}
                >
                  {t("Retry")}
                </button>
              }
            >
              {sessionsError}
            </SettingsNotice>
          )}
          {!sessions && !sessionsError && (
            <div
              className="account-skeleton"
              role="status"
              aria-label={t("Loading sessions…")}
            >
              {[0, 1].map((index) => (
                <div key={index} className="setting-row" aria-hidden="true">
                  <span className="account-skeleton-line" />
                </div>
              ))}
            </div>
          )}
          {sortedSessions?.map((item) => {
            const { Icon, label } = sessionKind(item.kind);
            return (
              <SettingRow
                key={item.id}
                className={`account-session${item.isCurrent ? " account-session-current" : ""}`}
                label={
                  <span className="account-session-label">
                    <span className="account-session-icon" aria-hidden="true">
                      <Icon size={15} />
                    </span>
                    {label}
                    {item.isCurrent && (
                      <span className="agent-control-badge" data-state="ready">
                        {t("This device")}
                      </span>
                    )}
                  </span>
                }
                description={t(
                  "Signed in {created} · Active {active} · Expires {expires}",
                  {
                    created: formatDate(item.createdAt, false),
                    active: item.isCurrent
                      ? t("now")
                      : formatRelative(item.updatedAt),
                    expires: formatDate(item.expiresAt, false),
                  },
                )}
              >
                {!item.isCurrent && (
                  <button
                    type="button"
                    className="button"
                    disabled={Boolean(revoking)}
                    onClick={() => void revoke(item.id)}
                  >
                    {revoking === item.id ? t("Revoking…") : t("Revoke")}
                  </button>
                )}
              </SettingRow>
            );
          })}
          {sessions && otherSessions.length === 0 && (
            <p className="settings-help account-sessions-empty">
              {t("No other sessions are signed in.")}
            </p>
          )}
          {sessions && (
            <SettingRow
              label={t("Sign out everywhere")}
              description={t(
                "Revoke every session, including this device. Use this if you think your account was accessed by someone else.",
              )}
            >
              <button
                type="button"
                className="button button-danger"
                disabled={Boolean(revoking)}
                onClick={() => setConfirmRevokeAll(true)}
              >
                {t("Sign out everywhere")}
              </button>
            </SettingRow>
          )}
        </SettingsSection>
      )}

      {attempt && status === "authorizing" && (
        <SettingsSection
          title={t("Finish signing in")}
          description={t(
            "Approve sign-in in your browser. Lomi will finish automatically when you return here.",
          )}
        >
          <div className="account-verification">
            <p role="status">{t("Waiting for browser approval.")}</p>
            {expiryText && (
              <p>
                {t("Sign-in request expires at {time}.", {
                  time: expiryText,
                })}
              </p>
            )}
            <div className="account-actions">
              <button
                className="button button-primary"
                disabled={busy}
                onClick={() =>
                  void run("auth_open_verification", { attemptId: attempt.id })
                }
              >
                <ExternalLink size={14} aria-hidden="true" />
                {t("Open browser again")}
              </button>
              <button
                className="button"
                disabled={busy}
                onClick={() =>
                  void run("auth_cancel_login", { attemptId: attempt.id })
                }
              >
                {t("Cancel sign-in")}
              </button>
            </div>
          </div>
        </SettingsSection>
      )}
      {canLogin && (
        <SettingsSection
          title={t("Sign in with GitHub")}
          description={t(
            "Connect your GitHub identity to Lomi. Signing in does not grant access to your private repositories.",
          )}
        >
          <div className="account-signin">
            <span className="account-session-icon" aria-hidden="true">
              <Github size={18} />
            </span>
            <div className="account-profile-text">
              <strong>{t("Not signed in")}</strong>
              <span>
                {t(
                  "Sign in to use Lomi online services. Your local projects work without an account.",
                )}
              </span>
            </div>
            <button
              className="button button-primary"
              disabled={busy || (status === "storage-locked" && remember)}
              onClick={() =>
                void run(
                  "auth_begin_login",
                  { storage: remember ? "persistent" : "session" },
                  true,
                )
              }
            >
              <Github size={15} aria-hidden="true" />
              {t("Sign in with GitHub")}
            </button>
          </div>
          <SettingRow
            label={t("Remember me on this device")}
            htmlFor="account-remember"
            description={
              remember
                ? t(
                    "Store your session in the system credential store when available; otherwise sign in only until Lomi closes.",
                  )
                : t(
                    "Keep your session only until Lomi closes. Nothing is saved to the credential store.",
                  )
            }
            descriptionId="account-storage-description"
          >
            <input
              id="account-remember"
              type="checkbox"
              role="switch"
              className="settings-switch"
              aria-describedby="account-storage-description"
              checked={remember}
              disabled={busy}
              onChange={(event) => setRemember(event.target.checked)}
            />
          </SettingRow>
        </SettingsSection>
      )}

      {(user || portalAvailable) && (
        <SettingsSection title={t("More")}>
          {user && onNavigate && (
            <SettingRow
              label={t("Notifications")}
              description={t("Choose which agent alerts reach your devices.")}
            >
              <button
                type="button"
                className="button"
                onClick={() => onNavigate("notifications")}
              >
                <Bell size={14} aria-hidden="true" />
                {t("Open")}
              </button>
            </SettingRow>
          )}
          {user && remoteAvailable && onNavigate && (
            <SettingRow
              label={t("Remote")}
              description={t("Share workspaces with your other devices.")}
            >
              <button
                type="button"
                className="button"
                onClick={() => onNavigate("remote")}
              >
                <Send size={14} aria-hidden="true" />
                {t("Open")}
              </button>
            </SettingRow>
          )}
          {portalAvailable && (
            <SettingRow
              label={t("Account portal")}
              description={t(
                "Edit your profile or delete your Lomi account in your browser.",
              )}
            >
              {portalButton}
            </SettingRow>
          )}
          {!user && signOutButton && (
            <SettingRow label={t("Sign out")}>{signOutButton}</SettingRow>
          )}
        </SettingsSection>
      )}

      {confirmRevokeAll && (
        <Modal
          protectTheme
          role="alertdialog"
          tone="danger"
          title={t("Sign out everywhere?")}
          descriptionId="account-revoke-all-description"
          initialFocus={cancelRevokeAll}
          closeDisabled={revoking === "all"}
          onClose={() => {
            if (revoking !== "all") setConfirmRevokeAll(false);
          }}
        >
          <div
            className="dialog-form dialog-layout"
            aria-busy={revoking === "all"}
          >
            <div className="dialog-content">
              <p id="account-revoke-all-description">
                {t(
                  "Every session is revoked, including this device and paired Remote hosts. Your local projects and files stay on each device.",
                )}
              </p>
            </div>
            <div className="dialog-actions">
              <button
                ref={cancelRevokeAll}
                type="button"
                className="button"
                disabled={revoking === "all"}
                onClick={() => setConfirmRevokeAll(false)}
              >
                {t("Cancel")}
              </button>
              <button
                type="button"
                className="button button-primary button-danger"
                disabled={revoking === "all"}
                onClick={() => void revokeAll()}
              >
                {revoking === "all"
                  ? t("Signing out…")
                  : t("Sign out everywhere")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </SettingsPage>
  );
}

function CopyButton({
  label,
  onClick,
}: {
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="icon-button"
      aria-label={label}
      title={label}
      onClick={onClick}
    >
      <Copy size={14} aria-hidden="true" />
    </button>
  );
}
