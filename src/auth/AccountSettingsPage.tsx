import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "../api";
import { ExternalLink, Github, RefreshCw } from "../icons";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "../settings-ui";
import { authStatusLabel, newestAuthState, unavailableState } from "./model";
import type { AuthState } from "./model";
import "./styles.css";

type AuthCommand =
  | "auth_begin_login"
  | "auth_open_verification"
  | "auth_cancel_login"
  | "auth_refresh_state"
  | "auth_sign_out"
  | "auth_open_account_portal";

export default function AccountSettingsPage() {
  const [state, setState] = useState<AuthState | null>(
    native ? null : unavailableState,
  );
  const snapshot = useRef(state);
  const mounted = useRef(false);
  const pending = useRef(false);
  const [ready, setReady] = useState(!native);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [remember, setRemember] = useState(true);
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

  const status = state?.status;
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
  const hasAccount = Boolean(state?.user || state?.session);
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
      ? expiry.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
      : null;

  return (
    <SettingsPage
      title="Account"
      description="Your Lomi account. Local projects and terminals remain available without signing in."
      status={
        !ready
          ? "Loading…"
          : state
            ? authStatusLabel(state.status)
            : "Unavailable"
      }
      busy={busy || !ready}
      className="account-settings"
      actions={
        native &&
        ready &&
        status !== "authorizing" && (
          <button
            className="button"
            disabled={busy}
            onClick={() => void run("auth_refresh_state")}
          >
            <RefreshCw size={14} aria-hidden="true" />
            Check connection
          </button>
        )
      }
    >
      {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
      {state?.message && (
        <SettingsNotice
          tone={
            status === "error" || status === "storage-locked"
              ? "warning"
              : "info"
          }
          role="status"
        >
          {state.message}
        </SettingsNotice>
      )}
      {!native && (
        <SettingsNotice>
          Account sign-in is available in the desktop app.
        </SettingsNotice>
      )}
      {native && status === "unavailable" && !state?.message && (
        <SettingsNotice>
          Account sign-in is not available in this version.
        </SettingsNotice>
      )}
      {status === "offline" && (
        <SettingsNotice tone="warning">
          Your account could not be checked. Online services will resume after a
          successful connection. Your local work is unaffected.
        </SettingsNotice>
      )}
      {status === "storage-locked" && (
        <SettingsNotice tone="warning" role="status">
          Use Check connection to retry access. To replace the saved sign-in,
          choose Sign out of this device. To sign in only until Lomi closes,
          turn off Remember me on this device.
        </SettingsNotice>
      )}
      {state?.remoteRevocationConfirmed === false && !hasAccount && (
        <SettingsNotice tone="warning" role="status">
          {status === "signed-out" && "Signed out on this device. "}
          The server could not confirm that its session was revoked. You can
          remove it from your account in a browser.
        </SettingsNotice>
      )}
      {state?.user && (
        <SettingsSection title="Your account">
          <SettingRow label="Name">
            <span className="setting-value">{state.user.displayName}</span>
          </SettingRow>
          {state.user.githubLogin && (
            <SettingRow label="GitHub">
              <span className="setting-value">@{state.user.githubLogin}</span>
            </SettingRow>
          )}
          <SettingRow label="Email">
            <span className="setting-value account-email">
              {state.user.email}
            </span>
          </SettingRow>
          {state.storage === "session" && (
            <SettingRow
              label="Storage"
              description="You will need to sign in again after closing Lomi."
            >
              <span className="setting-value">This session only</span>
            </SettingRow>
          )}
        </SettingsSection>
      )}
      {attempt && status === "authorizing" && (
        <SettingsSection
          title="Finish signing in"
          description="Approve sign-in in your browser. Lomi will finish automatically when you return here."
        >
          <div className="account-verification">
            <p role="status">Waiting for browser approval.</p>
            {expiryText && (
              <p>
                Sign-in request expires at{" "}
                <time dateTime={attempt.expiresAt}>{expiryText}</time>.
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
                Open browser again
              </button>
              <button
                className="button"
                disabled={busy}
                onClick={() =>
                  void run("auth_cancel_login", { attemptId: attempt.id })
                }
              >
                Cancel sign-in
              </button>
            </div>
          </div>
        </SettingsSection>
      )}
      {canLogin && (
        <SettingsSection
          title="Sign in with GitHub"
          description="Connect your GitHub identity to Lomi. Signing in does not grant access to your private repositories."
        >
          <SettingRow
            label="Remember me on this device"
            htmlFor="account-remember"
            description={
              remember
                ? "Store your session in the system credential store."
                : "Keep your session only until Lomi closes. Nothing is saved to the credential store."
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
          <div className="account-actions">
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
              Sign in with GitHub
            </button>
          </div>
        </SettingsSection>
      )}
      {native && ready && status !== "unavailable" && (
        <SettingsSection
          title="Account access"
          description="Manage sessions in your browser. Signing out leaves your projects, files, and AI connections on this device."
        >
          <div className="account-actions">
            <button
              className="button"
              disabled={busy}
              onClick={() => void run("auth_open_account_portal")}
            >
              <ExternalLink size={14} aria-hidden="true" />
              Manage account
            </button>
            {canSignOut && (
              <button
                className="button"
                disabled={busy}
                onClick={() => void run("auth_sign_out")}
              >
                Sign out of this device
              </button>
            )}
          </div>
        </SettingsSection>
      )}
    </SettingsPage>
  );
}
