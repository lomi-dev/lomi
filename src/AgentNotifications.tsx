import { useEffect, useId, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import { api, errorMessage, native } from "./api";
import {
  notificationContext,
  subscribeAgentNotifications,
} from "./agent-notifications";
import type { Session } from "./model";
import { runningTerminal } from "./terminal-runtime";
import { Modal } from "./ui";

interface NotificationSetup {
  path: string;
  revision: string | null;
  configured: boolean;
}

export function useAgentNotifications(
  session: Session | undefined,
  enabled: boolean,
  onError: (message: string) => void,
  onConfigured: (message: string) => void,
) {
  const current = useRef({ session, enabled, onError });
  current.current = { session, enabled, onError };
  const [setup, setSetup] = useState<NotificationSetup>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [reviewRequired, setReviewRequired] = useState(false);
  const descriptionId = useId();
  const cancel = useRef<HTMLButtonElement>(null);
  const permissionReported = useRef(false);
  const saving = useRef(false);
  useEffect(() => {
    if (!native) return;
    let alive = true;
    const pending = new Set<string>();
    const unsubscribe = subscribeAgentNotifications((event) => {
      if (!current.current.enabled) return;
      const key = `${event.sessionId}:${event.kind}`;
      if (pending.has(key)) return;
      pending.add(key);
      void (async () => {
        const granted = await isPermissionGranted();
        if (
          !alive ||
          !current.current.enabled ||
          runningTerminal(event.paneId)?.sessionId !== event.sessionId
        )
          return;
        if (!granted) {
          if (!permissionReported.current) {
            permissionReported.current = true;
            current.current.onError(
              "Agent notifications are blocked. Allow notifications for Lomi in your system settings, or use the CLI notification button to check permission.",
            );
          }
          return;
        }
        const context = notificationContext(
          current.current.session,
          event.paneId,
        );
        if (context === null) return;
        await api("notify_agent", { kind: event.kind, context });
      })()
        .catch((error) => {
          if (alive)
            current.current.onError(
              `Could not send agent notification: ${errorMessage(error)}`,
            );
        })
        .finally(() => pending.delete(key));
    });
    let inspecting = false;
    const stop = listen("agent-notification-setup", () => {
      if (inspecting || saving.current) return;
      if (document.querySelector("dialog[open]")) {
        current.current.onError(
          "Close the current dialog before configuring agent notifications.",
        );
        return;
      }
      inspecting = true;
      void api<NotificationSetup>("inspect_agent_notifications")
        .then((next) => {
          if (!alive) return;
          if (document.querySelector("dialog[open]")) {
            current.current.onError(
              "Close the current dialog and try configuring agent notifications again.",
            );
            return;
          }
          setError("");
          setReviewRequired(false);
          setSetup(next);
        })
        .catch((error) => {
          if (alive) current.current.onError(errorMessage(error));
        })
        .finally(() => {
          inspecting = false;
        });
    });
    void stop.catch((error) => {
      if (alive) current.current.onError(errorMessage(error));
    });
    return () => {
      alive = false;
      unsubscribe();
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  const save = async () => {
    if (!setup || saving.current) return;
    saving.current = true;
    setBusy(true);
    setError("");
    try {
      if (reviewRequired) {
        setSetup(await api<NotificationSetup>("inspect_agent_notifications"));
        setReviewRequired(false);
        return;
      }
      await api("enable_agent_notifications", {
        path: setup.path,
        revision: setup.revision,
      });
      permissionReported.current = false;
      const granted =
        (await isPermissionGranted()) ||
        (await requestPermission()) === "granted";
      setSetup(undefined);
      onConfigured(
        granted
          ? "Claude Code notifications are configured. Start a new Claude Code session to apply the hooks."
          : "Claude Code hooks are configured, but notifications are blocked. Allow Lomi notifications in your system settings.",
      );
    } catch (error) {
      setError(errorMessage(error));
      setReviewRequired(true);
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };

  return {
    dialog: setup && (
      <Modal
        title="Configure Claude Code notifications"
        descriptionId={descriptionId}
        initialFocus={cancel}
        className="cli-title-dialog"
        onClose={() => {
          if (!busy) setSetup(undefined);
        }}
      >
        <div className="dialog-form">
          <div className="cli-title-description" id={descriptionId}>
            <p>
              {setup.configured
                ? "Lomi notification hooks are already configured. You can check notification permission below."
                : "Add hooks that notify Lomi when Claude Code finishes responding or needs your input. They only send signals inside Lomi terminals."}
            </p>
            <p>
              <code>{setup.path}</code>
            </p>
            <p>
              Existing settings and hooks are preserved. A backup is created
              before changing the file.
            </p>
            <p>
              This configures local Claude Code sessions using this file. Start
              a new Claude Code session afterward. You can turn alerts off in
              Settings → Terminal.
            </p>
          </div>
          {error && (
            <p className="error" role="alert">
              {error}
            </p>
          )}
          <div className="dialog-actions">
            <button
              ref={cancel}
              type="button"
              className="button"
              disabled={busy}
              onClick={() => setSetup(undefined)}
            >
              Cancel
            </button>
            <button
              type="button"
              className="button button-primary"
              disabled={busy}
              onClick={() => void save()}
            >
              {busy
                ? "Configuring…"
                : reviewRequired
                  ? "Review again"
                  : setup.configured
                    ? "Check permission"
                    : "Enable integration"}
            </button>
          </div>
        </div>
      </Modal>
    ),
  };
}
