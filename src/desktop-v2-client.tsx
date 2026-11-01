import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { RemoteV2ConnectionRequest } from "../../remote-web/src/RemoteV2PairingPanel";
import { RemoteV2PairingPanel } from "../../remote-web/src/RemoteV2PairingPanel";
import { RemoteV2Workspace } from "../../remote-web/src/RemoteV2Workspace";
import type { RemoteV2ViewReference } from "../../remote-web/src/remote-v2-view-reference";
import { clearViewRestorationIntent } from "../../remote-web/src/remote-v2-view-restoration";
import { canShareRemotely } from "./auth/model";
import { useAuthState } from "./auth/useAuthState";
import {
  remoteAccountLookupIsCurrent,
  remoteAuthSessionKey,
} from "./desktop-remote-account";
import { native } from "./api";
import {
  createDesktopRemoteV2PlatformAdapter,
  desktopRemoteV2AccountId,
  openDesktopRemoteV2View,
} from "./remote-v2-native";
import { SettingsNotice } from "./settings-ui";
import "../../remote-web/src/remote-v2-embed.css";

type AccountLookupState =
  | "checking"
  | "authorization-required"
  | "ready"
  | "lookup-error"
  | "authorization-error";

type DesktopRemoteV2ClientProps = { available: boolean } & (
  | { entry: "selector"; initialTarget?: never }
  | { entry: "workspace"; initialTarget: RemoteV2ViewReference | undefined }
);

export function DesktopRemoteV2Client({
  available,
  entry,
  initialTarget,
}: DesktopRemoteV2ClientProps) {
  const [accountId, setAccountId] = useState<string>();
  const [accountSessionKey, setAccountSessionKey] = useState<string>();
  const [accountState, setAccountState] =
    useState<AccountLookupState>("checking");
  const [accountMessage, setAccountMessage] = useState("");
  const [authorizationBusy, setAuthorizationBusy] = useState(false);
  const [lookupRequest, setLookupRequest] = useState(0);
  const [connection, setConnection] =
    useState<RemoteV2ConnectionRequest | null>(null);
  const [reconnectRequest, setReconnectRequest] = useState(0);
  const [windowError, setWindowError] = useState("");
  const { state: authState, current: currentAuthState } = useAuthState();
  const authSessionKey = remoteAuthSessionKey(authState);
  const platform = useMemo(
    () => createDesktopRemoteV2PlatformAdapter(() => authState?.session?.id),
    [authSessionKey],
  );
  const lookupGeneration = useRef(0);

  useEffect(() => {
    if (!native || !available) return;
    const generation = ++lookupGeneration.current;
    let active = true;
    setAccountSessionKey(authSessionKey);
    setAccountId(undefined);
    setConnection(null);
    setAccountMessage("");
    setAuthorizationBusy(false);

    if (!authState) {
      setAccountState("checking");
      return () => {
        active = false;
        if (lookupGeneration.current === generation)
          lookupGeneration.current += 1;
      };
    }
    if (authState.status === "signed-out") {
      setAccountState("checking");
      return () => {
        active = false;
        if (lookupGeneration.current === generation)
          lookupGeneration.current += 1;
      };
    }
    if (!canShareRemotely(authState) || !authSessionKey) {
      setAccountState("checking");
      return () => {
        active = false;
        if (lookupGeneration.current === generation)
          lookupGeneration.current += 1;
      };
    }

    const lookupRevision = authState.revision;
    const current = () =>
      active &&
      lookupGeneration.current === generation &&
      remoteAccountLookupIsCurrent(
        lookupRevision,
        authSessionKey,
        currentAuthState.current,
      );
    setAccountState("checking");
    void desktopRemoteV2AccountId()
      .then((nextAccountId) => {
        if (!current()) return;
        setAccountId(nextAccountId);
        setAccountState(nextAccountId ? "ready" : "authorization-required");
      })
      .catch(() => {
        if (!current()) return;
        setAccountState("lookup-error");
        setAccountMessage("The Remote account could not be checked.");
      });

    return () => {
      active = false;
      if (lookupGeneration.current === generation)
        lookupGeneration.current += 1;
    };
  }, [available, authSessionKey, currentAuthState, lookupRequest]);

  const enableWorkspaceAccess = useCallback(async () => {
    const current = authState;
    if (!current || !canShareRemotely(current)) return;
    const sessionKey = remoteAuthSessionKey(current);
    if (!sessionKey) return;
    const revision = current.revision;
    const generation = ++lookupGeneration.current;
    const isCurrent = () =>
      lookupGeneration.current === generation &&
      remoteAccountLookupIsCurrent(
        revision,
        sessionKey,
        currentAuthState.current,
      );
    setAuthorizationBusy(true);
    setAccountState("checking");
    setAccountMessage("");
    try {
      await platform.authorizeConsumerAccess();
      if (!isCurrent()) return;
      const nextAccountId = await desktopRemoteV2AccountId();
      if (!isCurrent()) return;
      if (!nextAccountId) {
        setAccountState("authorization-error");
        setAccountMessage("Workspace access could not be confirmed.");
        return;
      }
      setAccountId(nextAccountId);
      setAccountSessionKey(sessionKey);
      setAccountState("ready");
    } catch {
      if (!isCurrent()) return;
      setAccountState("authorization-error");
      setAccountMessage("Workspace access could not be enabled. Try again.");
    } finally {
      if (isCurrent()) setAuthorizationBusy(false);
    }
  }, [authState, currentAuthState, platform]);

  const stopWorkspace = useCallback((windowIdentityId: string) => {
    setConnection((current) =>
      current?.registration.windowIdentityId === windowIdentityId
        ? null
        : current,
    );
  }, []);
  const reconnect = useCallback(() => {
    clearViewRestorationIntent();
    setConnection(null);
    setReconnectRequest((request) => request + 1);
  }, []);
  const openView = useCallback((hostId: string, workspaceId: string) => {
    setWindowError("");
    void openDesktopRemoteV2View(hostId, workspaceId).catch(() => {
      setWindowError("The remote workspace window could not be opened.");
    });
  }, []);

  if (!native) {
    return (
      <SettingsNotice>
        Remote workspace viewing is available in the desktop app.
      </SettingsNotice>
    );
  }
  if (!available) {
    return (
      <SettingsNotice>
        The identity, Remote API, and relay services are not configured in this
        build.
      </SettingsNotice>
    );
  }
  if (entry === "workspace" && !initialTarget) {
    return (
      <SettingsNotice>
        This remote workspace reference is invalid. Open a workspace again from
        its current view.
      </SettingsNotice>
    );
  }
  if (!authState) {
    return (
      <SettingsNotice>Checking the signed-in Lomi account…</SettingsNotice>
    );
  }
  if (authState.status === "signed-out") {
    return (
      <SettingsNotice>
        Sign in to your Lomi account on this device to view workspaces shared by
        another Lomi host.
      </SettingsNotice>
    );
  }
  if (!canShareRemotely(authState)) {
    const message =
      authState.status === "offline"
        ? "An online Lomi account session is required to view shared workspaces."
        : authState.status === "error" ||
            authState.status === "storage-locked" ||
            authState.status === "unavailable"
          ? (authState.message ?? "The Lomi account session is unavailable.")
          : "Checking the signed-in Lomi account…";
    return (
      <SettingsNotice tone={authState.status === "error" ? "error" : "info"}>
        {message}
      </SettingsNotice>
    );
  }
  if (accountSessionKey !== authSessionKey || accountState === "checking") {
    return (
      <SettingsNotice>Checking the signed-in Lomi account…</SettingsNotice>
    );
  }
  if (
    accountState === "authorization-required" ||
    accountState === "authorization-error"
  ) {
    return (
      <SettingsNotice
        tone={accountState === "authorization-error" ? "error" : "info"}
        role={accountState === "authorization-error" ? "alert" : "note"}
        action={
          <button
            type="button"
            className="button button-primary"
            disabled={authorizationBusy}
            onClick={() => void enableWorkspaceAccess()}
          >
            {authorizationBusy ? "Enabling…" : "Enable workspace access"}
          </button>
        }
      >
        {accountState === "authorization-error"
          ? accountMessage || "Workspace access could not be enabled."
          : "Enable this device to view workspaces shared by another Lomi host."}
      </SettingsNotice>
    );
  }
  if (accountState === "lookup-error") {
    return (
      <SettingsNotice
        tone="error"
        role="alert"
        action={
          <button
            type="button"
            className="button"
            onClick={() => setLookupRequest((request) => request + 1)}
          >
            Retry
          </button>
        }
      >
        {accountMessage || "The Remote account could not be checked."}
      </SettingsNotice>
    );
  }
  if (!accountId || accountState !== "ready") {
    return (
      <SettingsNotice>The Remote account could not be checked.</SettingsNotice>
    );
  }

  return (
    <div
      className="desktop-remote-v2-client"
      style={{
        display: "flex",
        flexDirection: "column",
        width: "100%",
        height:
          entry === "workspace" ? "100dvh" : connection ? "75dvh" : "auto",
        minHeight: 0,
        overflow: "hidden",
      }}
    >
      <div
        hidden={!!connection}
        style={{ flex: "1 1 auto", minHeight: 0, overflow: "auto" }}
      >
        <RemoteV2PairingPanel
          accountId={accountId}
          platform={platform}
          initialTarget={initialTarget}
          onConnect={setConnection}
          onStopWorkspace={stopWorkspace}
          reconnectRequest={reconnectRequest}
        />
      </div>
      {windowError && (
        <p className="error px-4 text-[13px]" role="alert">
          {windowError}
        </p>
      )}
      {connection && (
        <RemoteV2Workspace
          key={`${connection.registration.windowIdentityId}:${connection.workspace.workspaceId}`}
          connection={connection}
          onDisconnect={() => {
            clearViewRestorationIntent();
            setConnection(null);
          }}
          onReconnect={reconnect}
          onOpenView={() =>
            openView(connection.host.hostId, connection.workspace.workspaceId)
          }
        />
      )}
    </div>
  );
}
