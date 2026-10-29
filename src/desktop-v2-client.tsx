import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { RemoteV2ConnectionRequest } from "../../remote-web/src/RemoteV2PairingPanel";
import { RemoteV2PairingPanel } from "../../remote-web/src/RemoteV2PairingPanel";
import { RemoteV2Workspace } from "../../remote-web/src/RemoteV2Workspace";
import type { RemoteV2ViewReference } from "../../remote-web/src/remote-v2-view-reference";
import { clearViewRestorationIntent } from "../../remote-web/src/remote-v2-view-restoration";
import { native } from "./api";
import {
  createDesktopRemoteV2PlatformAdapter,
  desktopRemoteV2AccountId,
  openDesktopRemoteV2View,
} from "./remote-v2-native";
import { SettingsNotice } from "./settings-ui";
import "../../remote-web/src/remote-v2-embed.css";

interface AuthStateEvent {
  status?: unknown;
  user?: { id?: unknown } | null;
}

const ACCOUNT_ID = /^[0-9a-f]{32}$/;

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
  const [authReady, setAuthReady] = useState(false);
  const [connection, setConnection] =
    useState<RemoteV2ConnectionRequest | null>(null);
  const [reconnectRequest, setReconnectRequest] = useState(0);
  const [windowError, setWindowError] = useState("");
  const platform = useMemo(() => createDesktopRemoteV2PlatformAdapter(), []);

  useEffect(() => {
    if (!native || !available) {
      setAccountId(undefined);
      setAuthReady(false);
      setConnection(null);
      return;
    }
    let active = true;
    let receivedAuthEvent = false;
    let eventRevision = 0;
    let unlisten: (() => void) | undefined;
    void (async () => {
      unlisten = await listen<AuthStateEvent>(
        "auth-state-changed",
        ({ payload }) => {
          receivedAuthEvent = true;
          eventRevision += 1;
          const signedIn =
            payload.status === "signed-in" || payload.status === "signedin";
          const next =
            signedIn &&
            typeof payload.user?.id === "string" &&
            ACCOUNT_ID.test(payload.user.id)
              ? payload.user.id
              : undefined;
          setAccountId(next);
          setAuthReady(true);
        },
      );
      if (!active) {
        unlisten();
        return;
      }
      const revisionBeforeSnapshot = eventRevision;
      const initialAccountId = await desktopRemoteV2AccountId();
      if (
        active &&
        !receivedAuthEvent &&
        eventRevision === revisionBeforeSnapshot
      ) {
        setAccountId(initialAccountId);
      }
      if (active) setAuthReady(true);
    })().catch(() => {
      if (active) setAuthReady(true);
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [available]);

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
  if (!authReady) {
    return (
      <SettingsNotice>Checking the signed-in Lomi account…</SettingsNotice>
    );
  }
  if (!accountId) {
    return (
      <SettingsNotice>
        Sign in to your Lomi account on this device to view workspaces shared by
        another Lomi host.
      </SettingsNotice>
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
