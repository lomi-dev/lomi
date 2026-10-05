import { useEffect, useState } from "react";
import { api, errorMessage, native } from "./api";
import type { RemoteState } from "./remote-workspace-domain";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";

export default function RemoteSettingsPage({
  available,
}: {
  available: boolean;
}) {
  const [state, setState] = useState<RemoteState | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!native) return;
    let current = true;
    const refresh = () =>
      void api<RemoteState>("remote_get_state")
        .then((next) => {
          if (current) setState(next);
        })
        .catch((e) => {
          if (current) setError(errorMessage(e));
        });
    refresh();
    const timer = setInterval(refresh, 2000);
    return () => {
      current = false;
      clearInterval(timer);
    };
  }, []);
  const revoke = async (grantId: string) => {
    if (!available || busy) return;
    setBusy(true);
    setError("");
    try {
      setState(await api<RemoteState>("remote_revoke_grant", { grantId }));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const resume = async () => {
    if (!available || !state?.qualified || busy) return;
    setBusy(true);
    setError("");
    try {
      setState(await api<RemoteState>("remote_resume"));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const sharedWorkspaces = state?.workspaces?.filter((w) => w.shared) ?? [];
  const connectedBrowsers =
    state?.grants?.filter((grant) => !grant.revoked) ?? [];
  return (
    <SettingsPage
      title="Remote"
      description="Access your workspace terminals from a browser."
    >
      {!available && (
        <SettingsNotice>Sign in to use remote sharing.</SettingsNotice>
      )}
      {error && (
        <SettingsNotice tone="error" role="alert">
          {error}
        </SettingsNotice>
      )}
      {!native ? (
        <SettingsNotice>
          Remote hosting is available in the desktop app.
        </SettingsNotice>
      ) : (
        <>
          {state?.message && <SettingsNotice>{state.message}</SettingsNotice>}
          {state?.qualified === false && (
            <SettingsNotice>
              Remote hosting currently requires Lomi on a Mac with Apple
              silicon. You can still stop sharing and revoke browser access.
            </SettingsNotice>
          )}
          {state?.paused && (
            <button
              type="button"
              className="button"
              disabled={!available || !state?.qualified || busy}
              onClick={() => void resume()}
            >
              {busy ? "Resuming…" : "Resume remote"}
            </button>
          )}
          <SettingsSection
            title="Workspace access"
            description={
              <>
                Right-click a workspace and choose{" "}
                <strong>Share remotely</strong>. Browsers signed in to your
                account can view and control all its terminals, including
                inactive tabs and new splits. Choose{" "}
                <strong>Stop sharing remotely</strong> to end access.
              </>
            }
          >
            <p className="settings-help">
              Terminal contents and input are encrypted between your devices.
              Local typing immediately takes control back. Remote pauses after
              an hour without terminal data or activity in Lomi windows. Resume
              it here or from the workspace. Remote supports up to 32 shared
              workspaces and 32 active desktop terminals in total, including
              terminals in unshared workspaces.
            </p>
          </SettingsSection>
          <SettingsSection
            title="Shared workspaces"
            count={sharedWorkspaces.length}
          >
            {!sharedWorkspaces.length && (
              <p className="settings-help">No workspaces are shared.</p>
            )}
            {sharedWorkspaces.map((workspace, index) => (
              <SettingRow
                key={workspace.id}
                label={`Workspace ${index + 1}`}
                description={
                  workspace.message ??
                  (workspace.online
                    ? "Available to your account"
                    : "Waiting for connection")
                }
              >
                <span className="setting-value">
                  {workspace.online ? "Online" : "Offline"}
                </span>
              </SettingRow>
            ))}
          </SettingsSection>
          <SettingsSection
            title="Connected browsers"
            count={connectedBrowsers.length}
          >
            {!connectedBrowsers.length && (
              <p className="settings-help">No browsers have access.</p>
            )}
            {connectedBrowsers.map((grant, index) => (
              <SettingRow
                key={grant.id}
                label={`Browser ${index + 1}`}
                description={`${grant.sessionIds.length} terminals · expires ${new Date(grant.expiresAt * 1000).toLocaleString()}`}
              >
                <button
                  className="button"
                  disabled={!available || busy}
                  onClick={() => void revoke(grant.id)}
                >
                  Revoke access
                </button>
              </SettingRow>
            ))}
          </SettingsSection>
          <SettingsSection title="Background sessions">
            <SettingRow
              label="Keep Lomi running"
              description="Closing the workspace window keeps its terminals running. Quit Lomi saves and stops sessions after checking for unsaved work."
            >
              <button
                className="button"
                onClick={() =>
                  void api("reopen_main_window").catch((e) =>
                    setError(errorMessage(e)),
                  )
                }
              >
                Show workspace
              </button>
              <button
                className="button"
                onClick={() =>
                  void api("request_quit").catch((e) =>
                    setError(errorMessage(e)),
                  )
                }
              >
                Quit Lomi
              </button>
            </SettingRow>
          </SettingsSection>
        </>
      )}
    </SettingsPage>
  );
}
