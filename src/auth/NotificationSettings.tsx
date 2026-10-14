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

function message(reason: unknown): string {
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
        if (active) setError(message(reason));
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
        if (active) setError(message(reason));
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
  return { state, accept, error, setError, mounted };
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
  const [localResult, setLocalResult] = useState("");
  const [pushResult, setPushResult] = useState("");
  const localPending = useRef(false);
  const pushPending = useRef(false);
  const qualified =
    !!auth?.user &&
    !!auth.session &&
    !auth.attempt &&
    ["signed-in", "offline"].includes(auth.status);
  const canLocal = native && qualified && !!local.state?.available;
  const canCloud =
    native && auth?.status === "signed-in" && !!push.state?.available;
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
  const runLocal = async (
    enabled: boolean,
    backgroundOnly: boolean,
    test = false,
  ) => {
    if (!canLocal || !local.state || localPending.current) return;
    localPending.current = true;
    setLocalBusy(true);
    local.setError("");
    setLocalResult("");
    try {
      if (
        (enabled || test) &&
        !(await permission(!test && !local.state.preferences.enabled))
      )
        return;
      if (!local.mounted.current) return;
      if (test) {
        await api("desktop_notifications_send_test");
        if (local.mounted.current)
          setLocalResult(
            "Test requested. Check for a Lomi alert in your system notifications.",
          );
      } else
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
  const runCloud = async (
    command: string,
    args: Record<string, unknown>,
    test = false,
  ) => {
    if (!canCloud || pushPending.current) return;
    pushPending.current = true;
    setPushBusy(true);
    push.setError("");
    setPushResult("");
    try {
      const result = await api<PushState & { status?: string }>(command, {
        ...args,
        operationId: crypto.randomUUID(),
      });
      if (!push.mounted.current) return;
      if (test)
        setPushResult(
          result.status === "suppressed"
            ? "No eligible phone could receive this test."
            : "Test queued. Check your phone; queue acceptance does not confirm delivery.",
        );
      else push.accept(result);
    } catch (reason) {
      if (push.mounted.current) {
        push.setError(message(reason));
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
        title="System notifications on this computer"
        description="Get a generic alert when a managed agent turn or Chat response finishes. Your conversation and project details stay private."
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
        {qualified && (
          <SettingsNotice role="status">
            {preference?.enabled
              ? "Enabled — check receipt with a test. "
              : "Disabled. "}
            System permissions and Focus settings can prevent an alert. An
            already requested alert can appear after sign-out.
          </SettingsNotice>
        )}
        {local.error && (
          <SettingsNotice tone="error" role="alert">
            {local.error}
          </SettingsNotice>
        )}
        {localResult && (
          <SettingsNotice role="status">{localResult}</SettingsNotice>
        )}
        <button
          className="button"
          disabled={!canLocal || !preference?.enabled || localBusy}
          onClick={() =>
            void runLocal(true, preference?.backgroundOnly ?? true, true)
          }
        >
          Send test to this computer
        </button>
      </SettingsSection>
      <SettingsSection
        title="Phone notifications"
        description="Expo Push sends a generic completion alert to your enabled phones. This does not require workspace sharing."
      >
        <Toggle
          id="completion-push-enabled"
          label="Push notifications on my phones"
          checked={push.state?.preferences.enabled ?? false}
          disabled={
            !canCloud ||
            pushBusy ||
            (!canEnableCloud && !push.state?.preferences.enabled)
          }
          change={(enabled) =>
            void runCloud("push_update_account_preferences", {
              expectedRevision: push.state?.preferences.revision,
              enabled,
            })
          }
        />
        <Toggle
          id="completion-push-source"
          label="Send from this computer"
          checked={currentSource?.enabled ?? false}
          disabled={
            !canCloud ||
            pushBusy ||
            !currentSource ||
            (!canEnableCloud && !currentSource.enabled)
          }
          change={(enabled) =>
            void runCloud("push_set_source_enabled", {
              sourceId: currentSource?.id,
              expectedRevision: currentSource?.revision,
              enabled,
            })
          }
        />
        {!canCloud && (
          <SettingsNotice>
            Phone settings require a verified online Lomi account.
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
        {push.error && (
          <SettingsNotice tone="error" role="alert">
            {push.error}
          </SettingsNotice>
        )}
        {pushResult && (
          <SettingsNotice role="status">{pushResult}</SettingsNotice>
        )}
        {push.state?.installations
          .filter((phone) => !phone.retired)
          .map((phone) => (
            <SettingRow
              key={phone.id}
              label={phone.label || "Phone"}
              description={`${phone.platform} · ${phone.enabled ? "Receiving enabled" : "Receiving disabled"}`}
            >
              <button
                className="button"
                disabled={
                  !canEnableCloud ||
                  pushBusy ||
                  !phone.enabled ||
                  !push.state?.preferences.enabled
                }
                aria-label={`Send test to ${phone.label || "phone"}`}
                onClick={() =>
                  void runCloud(
                    "push_send_test",
                    { installationId: phone.id },
                    true,
                  )
                }
              >
                Send test
              </button>
              <button
                className="button"
                disabled={!canCloud || pushBusy || !phone.enabled}
                aria-label={`Disable receiving on ${phone.label || "phone"}`}
                onClick={() =>
                  void runCloud("push_disable_installation", {
                    installationId: phone.id,
                    expectedRevision: phone.revision,
                  })
                }
              >
                Disable receiving
              </button>
            </SettingRow>
          ))}
      </SettingsSection>
    </>
  );
}
