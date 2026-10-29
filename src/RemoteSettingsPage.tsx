import { useEffect, useRef, useState } from "react";
import { api, errorMessage, native } from "./api";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  configureSharedHostAuth,
  logoutSharedHost,
  sharedHostAuthSetup,
  sharedHostLoginPresentation,
  sharedHostLoginStatus,
  sharedHostStatus,
  startSharedHostLogin,
} from "./shared-host";
import type {
  SharedHostAuthSetup,
  SharedHostAuthProvider,
  SharedHostLoginPresentation,
  SharedHostLoginStatus,
  SharedHostStatus,
} from "./shared-host";
import type { RemoteState } from "./remote-workspace-domain";
import { DesktopRemoteV2Client } from "./desktop-v2-client";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";

const authProviderLabels: Record<SharedHostAuthProvider, string> = {
  macOsKeychain: "macOS Keychain",
  windowsDpapi: "Windows DPAPI",
  linuxSecretService: "Linux Secret Service (system keyring)",
  protectedFile: "Protected file",
};

export default function RemoteSettingsPage({
  available,
  disabledReason,
}: {
  available: boolean;
  disabledReason: string;
}) {
  const [state, setState] = useState<RemoteState | null>(null);
  const [error, setError] = useState("");
  const [hostStatus, setHostStatus] = useState<SharedHostStatus | null>(null);
  const [authSetup, setAuthSetup] = useState<SharedHostAuthSetup | null>(null);
  const [selectedAuthProvider, setSelectedAuthProvider] =
    useState<SharedHostAuthProvider | null>(null);
  const [loginStatus, setLoginStatus] = useState<SharedHostLoginStatus | null>(
    null,
  );
  const [loginPresentation, setLoginPresentation] =
    useState<SharedHostLoginPresentation | null>(null);
  const [acknowledgeProtectedFile, setAcknowledgeProtectedFile] =
    useState(false);
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState("");
  const [authRevision, setAuthRevision] = useState(0);
  const [presentationBusy, setPresentationBusy] = useState(false);
  const loginOperationId = useRef<string | null>(null);

  useEffect(() => {
    if (!native || !available) {
      setState(null);
      setError("");
      setHostStatus(null);
      setAuthSetup(null);
      setLoginStatus(null);
      setLoginPresentation(null);
      setAuthError("");
      return;
    }
    let current = true;
    let refreshing = false;
    const refresh = async () => {
      if (refreshing) return;
      refreshing = true;
      try {
        const status = await sharedHostStatus();
        if (!current) return;
        setHostStatus(status);
        if (
          status.state === "unavailable" ||
          status.state === "updateRequired"
        ) {
          setState(null);
          setError(status.message ?? "The local Host is unavailable.");
          return;
        }
        if (!status.hostId) {
          setState(null);
          setError("");
          return;
        }
        const workspaces = await api<
          {
            workspaceId: string;
            name: string;
            sharing: "disabled" | "enabled";
          }[]
        >("shared_host_workspaces");
        if (!current) return;
        setState({
          qualified: true,
          enabled: status.sharing === "enabled",
          message: null,
          workspaces: workspaces.map((workspace) => ({
            id: workspace.workspaceId,
            name: workspace.name,
            shared: workspace.sharing === "enabled",
            message: null,
          })),
        });
        setError("");
      } catch (cause) {
        if (current) {
          setHostStatus(null);
          setState(null);
          setError(errorMessage(cause));
        }
      } finally {
        refreshing = false;
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 5000);
    return () => {
      current = false;
      clearInterval(timer);
    };
  }, [available]);

  useEffect(() => {
    if (!native || !available) return;
    let current = true;
    let refreshing = false;
    const refresh = async () => {
      if (refreshing) return;
      refreshing = true;
      try {
        const setup = await sharedHostAuthSetup();
        if (!current) return;
        setAuthSetup(setup);
        setSelectedAuthProvider((selected) =>
          selected && setup.providers.includes(selected)
            ? selected
            : (setup.providers[0] ?? null),
        );
        const status = setup.bootstrapConfigured
          ? await sharedHostLoginStatus()
          : null;
        if (!current) return;
        setLoginStatus(status);
        if (status?.phase === "ready") setLoginPresentation(null);
        setAuthError("");
      } catch (cause) {
        if (current) {
          setAuthSetup(null);
          setLoginStatus(null);
          setLoginPresentation(null);
          setAuthError(errorMessage(cause));
        }
      } finally {
        refreshing = false;
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 3000);
    return () => {
      current = false;
      clearInterval(timer);
    };
  }, [authRevision, available]);

  const configureAuth = async () => {
    if (!available || !selectedAuthProvider || authBusy) return;
    setAuthBusy(true);
    setAuthError("");
    try {
      const configured = await configureSharedHostAuth(
        selectedAuthProvider,
        acknowledgeProtectedFile,
      );
      if (configured) {
        setAcknowledgeProtectedFile(false);
        setAuthRevision((revision) => revision + 1);
      }
    } catch (cause) {
      setAuthError(errorMessage(cause));
    } finally {
      setAuthBusy(false);
    }
  };

  const startLogin = async () => {
    if (!available || authBusy) return;
    setAuthBusy(true);
    setAuthError("");
    try {
      loginOperationId.current ??= crypto.randomUUID();
      await startSharedHostLogin(loginOperationId.current);
      loginOperationId.current = null;
      setLoginPresentation(null);
      setAuthRevision((revision) => revision + 1);
    } catch (cause) {
      setAuthError(errorMessage(cause));
    } finally {
      setAuthBusy(false);
    }
  };

  const revealLoginPresentation = async () => {
    if (!available || presentationBusy) return;
    setPresentationBusy(true);
    setAuthError("");
    try {
      setLoginPresentation(await sharedHostLoginPresentation());
    } catch (cause) {
      setAuthError(errorMessage(cause));
    } finally {
      setPresentationBusy(false);
    }
  };

  const signOut = async () => {
    if (!available || authBusy) return;
    setAuthBusy(true);
    setAuthError("");
    try {
      await logoutSharedHost();
      setLoginPresentation(null);
      setAuthRevision((revision) => revision + 1);
    } catch (cause) {
      setAuthError(errorMessage(cause));
    } finally {
      setAuthBusy(false);
    }
  };

  const sharedWorkspaces =
    state?.workspaces.filter((workspace) => workspace.shared) ?? [];
  return (
    <SettingsPage
      title="Remote"
      description={
        available
          ? "Connect to another Lomi host or manage this device’s shared workspaces."
          : undefined
      }
    >
      {available && error && (
        <SettingsNotice tone="error" role="alert">
          {error}
        </SettingsNotice>
      )}
      {available && hostStatus?.state === "administratorInstallRequired" && (
        <SettingsNotice tone="error" role="alert">
          Local Host startup requires the signed Lomi Remote administrator
          installation for this macOS account. Run the signed installer with{" "}
          <code>sudo ./install.sh admin-install ACCOUNT</code>, replacing{" "}
          <code>ACCOUNT</code> with this account’s name.
        </SettingsNotice>
      )}
      {!native ? (
        <SettingsNotice>
          Remote hosting is available in the desktop app.
        </SettingsNotice>
      ) : !available ? (
        <SettingsNotice>{disabledReason}</SettingsNotice>
      ) : (
        <>
          {state?.message && <SettingsNotice>{state.message}</SettingsNotice>}
          <SettingsSection
            title="Connect to a shared workspace"
            description="Choose an online host, request approval, and open its existing workspace without starting a shell."
          >
            <DesktopRemoteV2Client available={available} entry="selector" />
          </SettingsSection>
          <SettingsSection
            title="Host account"
            description="Sign in through the local host to publish remote workspace access."
          >
            {authError && (
              <SettingsNotice tone="error" role="alert">
                {authError}
              </SettingsNotice>
            )}
            {!authSetup ? (
              <p className="settings-help">Loading host authentication…</p>
            ) : !authSetup.bootstrapConfigured ? (
              <SettingsNotice>
                This build does not include the public identity, API, and relay
                settings required for host authentication. Remote sign-in is
                unavailable in this build.
              </SettingsNotice>
            ) : loginStatus?.phase === "unconfigured" ? (
              <>
                <p className="settings-help">
                  Set up the host using this device’s selected{" "}
                  {selectedAuthProvider
                    ? authProviderLabels[selectedAuthProvider]
                    : "secret storage"}{" "}
                  secret provider. Public service addresses are supplied by the
                  trusted desktop build.
                </p>
                {authSetup.providers.length > 1 && (
                  <label className="settings-help">
                    Secret provider{" "}
                    <select
                      value={selectedAuthProvider ?? ""}
                      onChange={(event) => {
                        setSelectedAuthProvider(
                          event.target.value as SharedHostAuthProvider,
                        );
                        setAcknowledgeProtectedFile(false);
                      }}
                    >
                      {authSetup.providers.map((provider) => (
                        <option key={provider} value={provider}>
                          {authProviderLabels[provider]}
                        </option>
                      ))}
                    </select>
                  </label>
                )}
                {selectedAuthProvider === "protectedFile" && (
                  <label className="settings-help">
                    <input
                      type="checkbox"
                      checked={acknowledgeProtectedFile}
                      onChange={(event) =>
                        setAcknowledgeProtectedFile(event.target.checked)
                      }
                    />{" "}
                    I understand that this provider relies on operating system
                    file permissions to protect host credentials.
                  </label>
                )}
                <button
                  className="button"
                  disabled={
                    authBusy ||
                    (selectedAuthProvider === "protectedFile" &&
                      !acknowledgeProtectedFile)
                  }
                  onClick={() => void configureAuth()}
                >
                  {authBusy ? "Setting up…" : "Set up host authentication"}
                </button>
              </>
            ) : !loginStatus ? (
              <p className="settings-help">Loading sign-in status…</p>
            ) : loginStatus.phase === "ready" ? (
              <>
                <SettingRow label="Sign-in status">
                  <span className="setting-value">Signed in</span>
                </SettingRow>
                <button
                  className="button"
                  disabled={authBusy}
                  onClick={() => void signOut()}
                >
                  Sign out of the host
                </button>
              </>
            ) : (
              <>
                <SettingRow label="Sign-in status">
                  <span className="setting-value">
                    {loginStatus.phase === "awaitingApproval"
                      ? "Waiting for approval"
                      : loginStatus.phase === "locked"
                        ? "Secret storage is locked"
                        : loginStatus.phase === "unavailable"
                          ? "Authentication unavailable"
                          : loginStatus.phase === "initializing" ||
                              loginStatus.phase === "exchanging" ||
                              loginStatus.phase === "registering"
                            ? "Signing in…"
                            : "Not signed in"}
                  </span>
                </SettingRow>
                {loginStatus.phase === "unavailable" && (
                  <p className="settings-help">
                    Check this device’s network and secret storage, then retry.
                  </p>
                )}
                <button
                  className="button"
                  disabled={
                    authBusy ||
                    !["idle", "unavailable"].includes(loginStatus.phase)
                  }
                  onClick={() => void startLogin()}
                >
                  {authBusy ? "Starting…" : "Sign in to the host"}
                </button>
                {loginStatus.phase === "awaitingApproval" && (
                  <button
                    className="button"
                    disabled={presentationBusy}
                    onClick={() => void revealLoginPresentation()}
                  >
                    {presentationBusy
                      ? "Loading code…"
                      : "Show verification code"}
                  </button>
                )}
                {loginPresentation && (
                  <div className="settings-help" role="status">
                    <p>Enter this code at the verification page:</p>
                    <p>
                      <strong>{loginPresentation.userCode}</strong>
                    </p>
                    <p>
                      Verification page: {loginPresentation.verificationUrl}
                    </p>
                    <button
                      className="text-button"
                      onClick={() =>
                        void openUrl(loginPresentation.verificationUrl).catch(
                          (cause) => setAuthError(errorMessage(cause)),
                        )
                      }
                    >
                      Open verification page
                    </button>
                  </div>
                )}
              </>
            )}
          </SettingsSection>
          <SettingsSection
            title="Workspace access"
            description={
              <>
                Share a workspace from the Workspaces panel. The local host
                publishes its shared terminal tree while the host is running.
                Stopping sharing ends remote access and leaves local terminals
                running.
              </>
            }
          >
            <p className="settings-help">
              Editors, chats, browsers and other desktop-only panels stay on
              this device. Workspace sharing applies to the host-owned
              terminals.
            </p>
          </SettingsSection>
          <SettingsSection
            title="Shared workspaces"
            count={sharedWorkspaces.length}
          >
            {!sharedWorkspaces.length && (
              <p className="settings-help">No workspaces are shared.</p>
            )}
            {sharedWorkspaces.map((workspace) => (
              <SettingRow
                key={workspace.id}
                label={workspace.name}
                description={workspace.message ?? "Shared by the local host"}
              >
                <span className="setting-value">Shared</span>
              </SettingRow>
            ))}
          </SettingsSection>
          <SettingsSection
            title="Background sessions"
            description="Closing a Lomi window keeps host-owned terminals running."
          >
            <SettingRow label="Host sharing">
              <span className="setting-value">
                {!state ? "Loading…" : state.enabled ? "Enabled" : "Inactive"}
              </span>
            </SettingRow>
            <SettingRow label="Keep Lomi running">
              <button
                className="button"
                onClick={() =>
                  void api("reopen_main_window").catch((cause) =>
                    setError(errorMessage(cause)),
                  )
                }
              >
                Show workspace
              </button>
              <button
                className="button"
                onClick={() =>
                  void api("request_quit").catch((cause) =>
                    setError(errorMessage(cause)),
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
