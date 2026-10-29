import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import { api, native } from "../api";
import { SettingRow, SettingsNotice, SettingsSection } from "../settings-ui";
import type { AuthState } from "./model";

interface LocalState {
  revision: number;
  accountId: string | null;
  available: boolean;
  preferences: {
    schemaVersion: number;
    enabled: boolean;
    backgroundOnly: boolean;
    consentEpoch: number;
    revision: number;
  };
  permissionStatus: "unknown" | "denied";
  errorCode: string | null;
}
interface Source {
  id: string;
  label: string;
  enabled: boolean;
  revision: number;
  revoked: boolean;
  current: boolean;
}
interface Installation {
  id: string;
  label: string;
  platform: string;
  enabled: boolean;
  revision: number;
  retired: boolean;
  disabledReason: string | null;
}
interface PushState {
  revision: number;
  accountId: string | null;
  available: boolean;
  featureEnabled: boolean;
  preferences: { enabled: boolean; consentEpoch: number; revision: number };
  sources: Source[];
  installations: Installation[];
  currentSourceId: string | null;
  errorCode: string | null;
}

function message(reason: unknown, phone = false): string {
  const code =
    typeof reason === "object" && reason && "code" in reason
      ? reason.code
      : reason;
  switch (code) {
    case "REVISION_CONFLICT":
    case "BINDING_CHANGED":
      return "Settings changed on another device. The current state has been refreshed; try again.";
    case "TOKEN_BINDING_CONFLICT":
      return "This phone is linked to another active account. Disable its previous binding first.";
    case "FEATURE_DISABLED":
      return "Phone notifications are not available in this environment yet.";
    case "UPDATE_REQUIRED":
      return "Update Lomi to use phone notifications.";
    case "NOT_FOUND":
      return phone
        ? "The server could not find the phone notification settings. It may need an update to support phone notifications. Try again after the service is updated."
        : "Notification settings could not be found. Restart Lomi and try again.";
    case "SERVICE_UNAVAILABLE":
      return "The notification service is temporarily unavailable. Try again later.";
    case "TRANSPORT_UNKNOWN":
      return "Could not reach the notification service. Check your connection and try again.";
    case "INVALID_RESPONSE":
      return "The notification service returned an unexpected response. Update Lomi and try again; if this continues, the service may need an update.";
    case "STORAGE_UNAVAILABLE":
      return "Lomi could not access notification preferences on this computer. Restart Lomi and try again.";
    case "ACCOUNT_UNAVAILABLE":
    case "ACCESS_EXPIRED":
      return "Your account connection needs attention. Check your Lomi account and sign in again if needed.";
    case "RATE_LIMITED":
      return "Too many requests. Wait a moment and try again.";
    case "SESSION_REQUIRED":
    case "AUTHORITY_CHANGED":
      return "Check your account connection before continuing.";
    default:
      return "Could not update notification settings. Check your connection and try again.";
  }
}

function useNativeState<
  T extends { accountId: string | null; revision: number },
>(
  accountId: string | null,
  command: string,
  event: string,
  refreshCommand = command,
) {
  const [state, setState] = useState<T | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const mounted = useRef(false);
  const current = useRef<T | null>(null);
  const accept = (value: T) => {
    if (
      !mounted.current ||
      !value ||
      value.accountId !== accountId ||
      !Number.isSafeInteger(value.revision)
    )
      return;
    if (current.current && value.revision < current.current.revision) return;
    current.current = value;
    setState(value);
  };
  useEffect(() => {
    mounted.current = true;
    if (!native || !accountId)
      return () => {
        mounted.current = false;
      };
    let active = true;
    let stop: (() => void) | undefined;
    const refresh = async (initial = false) => {
      try {
        const value = await api<T>(initial ? command : refreshCommand);
        if (active) {
          accept(value);
          setError("");
        }
      } catch (reason) {
        if (active) setError(message(reason, command.startsWith("push_")));
      } finally {
        if (active) setLoading(false);
      }
    };
    void (async () => {
      try {
        const unsubscribe = await listen<T>(event, ({ payload }) => {
          if (active) accept(payload);
        });
        if (!active) {
          unsubscribe();
          return;
        }
        stop = unsubscribe;
        await refresh(true);
        if (active && command !== refreshCommand) await refresh();
      } catch (reason) {
        if (active) setError(message(reason, command.startsWith("push_")));
      }
    })();
    const timer = window.setInterval(() => void refresh(), 10_000);
    return () => {
      active = false;
      mounted.current = false;
      stop?.();
      window.clearInterval(timer);
    };
    // The enclosing account/session key remounts this component on ownership changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId, command, event, refreshCommand]);
  return { state, accept, error, setError, mounted, loading };
}

function Toggle({
  id,
  label,
  description,
  checked,
  disabled,
  change,
}: {
  id: string;
  label: string;
  description?: string;
  checked: boolean;
  disabled: boolean;
  change: (value: boolean) => void;
}) {
  return (
    <SettingRow
      label={label}
      htmlFor={id}
      description={description}
      descriptionId={`${id}-help`}
    >
      <input
        id={id}
        type="checkbox"
        role="switch"
        className="settings-switch"
        checked={checked}
        disabled={disabled}
        aria-describedby={description ? `${id}-help` : undefined}
        onChange={(event) => change(event.target.checked)}
      />
    </SettingRow>
  );
}

export default function NotificationSettings({
  auth,
}: {
  auth: AuthState | null;
}) {
  const accountId = auth?.user?.id ?? null;
  const local = useNativeState<LocalState>(
    accountId,
    "desktop_notifications_get_state",
    "desktop-notifications-state-changed",
  );
  const push = useNativeState<PushState>(
    accountId,
    "push_get_state",
    "push-state-changed",
    "push_refresh_state",
  );
  const [localBusy, setLocalBusy] = useState(false);
  const [pushBusy, setPushBusy] = useState(false);
  const localPending = useRef(false);
  const pushPending = useRef(false);
  const qualified =
    !!auth?.user &&
    !!auth.session &&
    !auth.attempt &&
    ["signed-in", "offline"].includes(auth.status);
  const canLocal = native && qualified && !!local.state?.available;
  const onlineAccount = qualified && auth?.status === "signed-in";
  const pushError =
    push.error ||
    (push.state?.errorCode ? message(push.state.errorCode, true) : "");
  const canCloud =
    native && onlineAccount && !!push.state?.available && !pushError;
  const canEnableCloud = canCloud && !!push.state?.featureEnabled;

  const permission = async (prompt: boolean) => {
    let granted = await isPermissionGranted();
    if (!local.mounted.current) return false;
    if (!granted && prompt) granted = (await requestPermission()) === "granted";
    if (!local.mounted.current) return false;
    local.accept(
      await api<LocalState>("desktop_notifications_report_permission", {
        denied: !granted,
      }),
    );
    if (!granted)
      local.setError(
        "Notifications are blocked. Check notification permissions in your system settings.",
      );
    return granted;
  };
  const runLocal = async (enabled: boolean, backgroundOnly: boolean) => {
    if (!canLocal || !local.state || localPending.current) return;
    localPending.current = true;
    setLocalBusy(true);
    local.setError("");
    try {
      if (enabled && !(await permission(!local.state.preferences.enabled)))
        return;
      if (!local.mounted.current) return;
      local.accept(
        await api<LocalState>("desktop_notifications_save_preferences", {
          expectedRevision: local.state.preferences.revision,
          enabled,
          backgroundOnly,
        }),
      );
    } catch (reason) {
      if (local.mounted.current) {
        local.setError(message(reason));
        try {
          local.accept(
            await api<LocalState>("desktop_notifications_get_state"),
          );
        } catch {
          /* Keep the confirmed state. */
        }
      }
    } finally {
      localPending.current = false;
      if (local.mounted.current) setLocalBusy(false);
    }
  };
  const runCloud = async (enabled: boolean) => {
    if (!canCloud || pushPending.current) return;
    pushPending.current = true;
    setPushBusy(true);
    push.setError("");
    try {
      const result = await api<PushState>("push_set_current_source_enabled", {
        enabled,
      });
      if (push.mounted.current) push.accept(result);
    } catch (reason) {
      if (push.mounted.current) {
        push.setError(message(reason, true));
        try {
          push.accept(await api<PushState>("push_refresh_state"));
        } catch {
          /* Retry remains explicit. */
        }
      }
    } finally {
      pushPending.current = false;
      if (push.mounted.current) setPushBusy(false);
    }
  };
  const currentSource = push.state?.sources.find(
    (source) => source.id === push.state?.currentSourceId && !source.revoked,
  );
  const preference = local.state?.preferences;
  return (
    <>
      <SettingsSection
        title="On this computer"
        description="Show system notifications when tasks finish."
      >
        <Toggle
          id="completion-desktop-enabled"
          label="Notify when tasks finish"
          checked={preference?.enabled ?? false}
          disabled={!canLocal || localBusy}
          change={(enabled) =>
            void runLocal(enabled, preference?.backgroundOnly ?? true)
          }
        />
        <Toggle
          id="completion-desktop-background"
          label="Only when the main Lomi window is inactive"
          checked={preference?.backgroundOnly ?? true}
          disabled={!canLocal || localBusy}
          change={(backgroundOnly) =>
            void runLocal(preference?.enabled ?? false, backgroundOnly)
          }
        />
        {!qualified && (
          <SettingsNotice>
            Sign in to Lomi to enable completion notifications.
          </SettingsNotice>
        )}
        {local.state?.permissionStatus === "denied" && !local.error && (
          <SettingsNotice role="status">
            Notifications are blocked in system settings.
          </SettingsNotice>
        )}
        {local.error && (
          <SettingsNotice tone="error" role="alert">
            {local.error}
          </SettingsNotice>
        )}
      </SettingsSection>
      <SettingsSection
        title="On my phones"
        description="Send completion notifications from this computer to your phones."
      >
        <Toggle
          id="completion-push-enabled"
          label="Push notifications on my phones"
          checked={
            !!push.state?.preferences.enabled && !!currentSource?.enabled
          }
          disabled={
            !canCloud ||
            pushBusy ||
            (!canEnableCloud &&
              !(push.state?.preferences.enabled && currentSource?.enabled))
          }
          change={(enabled) => void runCloud(enabled)}
        />
        {!onlineAccount && (
          <SettingsNotice>
            {auth?.status === "offline" && qualified
              ? "Reconnect your Lomi account to manage phone notifications."
              : auth?.status === "checking"
                ? "Checking your Lomi account before loading phone settings…"
                : auth?.user
                  ? "Check your Lomi account connection to manage phone notifications."
                  : "Sign in to Lomi to manage phone notifications."}
          </SettingsNotice>
        )}
        {onlineAccount && !pushError && !canCloud && (
          <SettingsNotice role="status">
            {push.loading
              ? "Loading phone notification settings…"
              : "Phone notification settings are unavailable. Try again later."}
          </SettingsNotice>
        )}
        {canCloud && !push.state?.featureEnabled && (
          <SettingsNotice>
            Phone notifications are not available in this environment yet.
          </SettingsNotice>
        )}
        {canCloud &&
          push.state?.featureEnabled &&
          !push.state.installations.some(
            (phone) => !phone.retired && phone.enabled,
          ) && (
            <SettingsNotice>
              Enable receiving in the Lomi app on your phone.
            </SettingsNotice>
          )}
        {onlineAccount && pushError && (
          <SettingsNotice tone="error" role="alert">
            {pushError}
          </SettingsNotice>
        )}
      </SettingsSection>
    </>
  );
}
