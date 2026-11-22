import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import type { ControlStartupState } from "./agent-control-startup";
import { useT } from "./i18n";
import { Modal } from "./ui";
import {
  registerNotificationAction,
  reportAppNotification,
} from "./app-notifications";

export default function AgentControlStartup() {
  const { t } = useT();
  const [state, setState] = useState<ControlStartupState>();
  const [loadError, setLoadError] = useState("");
  const reportedLoadError = useRef("");
  const reportedStartupError = useRef("");
  const [saveError, setSaveError] = useState("");
  const [saving, setSaving] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const [readyToPrompt, setReadyToPrompt] = useState(false);
  const saveLock = useRef(false);
  const stateRef = useRef<ControlStartupState | undefined>(undefined);
  const snapshotRevision = useRef(0);
  const decisionRevision = useRef(0);
  const disabledButton = useRef<HTMLButtonElement>(null);
  const applyState = useCallback((next: ControlStartupState) => {
    const previous = stateRef.current;
    if (next.autoStart !== null && next.autoStart !== previous?.autoStart) {
      ++decisionRevision.current;
      setSaveError("");
    }
    stateRef.current = next;
    setState(next);
  }, []);

  const refresh = useCallback(async () => {
    const request = ++snapshotRevision.current;
    try {
      const next = await api<ControlStartupState>(
        "agent_control_startup_state",
      );
      if (request !== snapshotRevision.current) return;
      applyState(next);
      setLoadError("");
    } catch (error) {
      if (request === snapshotRevision.current)
        setLoadError(errorMessage(error));
    }
  }, [applyState]);

  useEffect(() => {
    if (!native) return;
    void refresh();
    const stop = listen<ControlStartupState>(
      "agent-control-startup-changed",
      ({ payload }) => {
        ++snapshotRevision.current;
        applyState(payload);
        setLoadError("");
      },
    );
    const focused = () => void refresh();
    window.addEventListener("focus", focused);
    return () => {
      ++snapshotRevision.current;
      ++decisionRevision.current;
      window.removeEventListener("focus", focused);
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [applyState, refresh]);

  const eligible =
    native &&
    state?.supported === true &&
    state.autoStart === null &&
    !state.error &&
    !dismissed;

  useEffect(() => {
    if (!eligible) {
      setReadyToPrompt(false);
      return;
    }
    if (readyToPrompt) return;

    let observer: MutationObserver | undefined;
    const offerWhenClear = () => {
      if (document.querySelector("dialog[open]")) return;
      setReadyToPrompt(true);
      observer?.disconnect();
    };
    offerWhenClear();
    if (!document.querySelector("dialog[open]")) return;
    observer = new MutationObserver(offerWhenClear);
    observer.observe(document.body, {
      attributes: true,
      attributeFilter: ["open"],
      childList: true,
      subtree: true,
    });
    return () => observer?.disconnect();
  }, [eligible, readyToPrompt]);

  const decide = async (enabled: boolean) => {
    if (saveLock.current) return;
    saveLock.current = true;
    const request = ++decisionRevision.current;
    ++snapshotRevision.current;
    setSaving(true);
    setSaveError("");
    try {
      const next = await api<ControlStartupState>(
        "agent_control_startup_decide",
        { enabled },
      );
      if (request !== decisionRevision.current) return;
      ++snapshotRevision.current;
      applyState(next);
      setLoadError("");
      setSaveError("");
      if (next.autoStart === null)
        setSaveError(t("The startup choice was not saved."));
    } catch (error) {
      if (request === decisionRevision.current)
        setSaveError(errorMessage(error));
    } finally {
      saveLock.current = false;
      setSaving(false);
    }
  };

  const startupError = state?.error;
  const startupMessage =
    startupError && state
      ? state.autoStart
        ? t(
            "Automatic MCP startup is enabled, but the server could not start: {error} The saved choice is still enabled. Open Settings → Agent control to change it or enable control for this session.",
            { error: startupError },
          )
        : state.autoStart === false
          ? t("Automatic MCP startup is disabled. {error}", {
              error: startupError,
            })
          : t(
              "Could not load the saved automatic MCP startup choice: {error}",
              { error: startupError },
            )
      : "";
  useEffect(() => {
    if (loadError && loadError !== reportedLoadError.current)
      void reportAppNotification(
        {
          source: "mcp",
          body: t("Could not load automatic MCP startup settings: {error}", {
            error: loadError,
          }),
          action: "retry-mcp-startup",
        },
        { deduplicate: false },
      );
    reportedLoadError.current = loadError;
  }, [loadError, t]);
  useEffect(() => {
    if (startupMessage && startupMessage !== reportedStartupError.current)
      void reportAppNotification(
        {
          source: "mcp",
          body: startupMessage,
          action: "open-agent-control-settings",
        },
        { deduplicate: false },
      );
    reportedStartupError.current = startupMessage;
  }, [startupMessage]);
  useEffect(
    () =>
      registerNotificationAction("retry-mcp-startup", {
        label: t("Retry"),
        run: refresh,
      }),
    [refresh, t],
  );
  return (
    <>
      {eligible && readyToPrompt && (
        <Modal
          protectTheme
          tone="warning"
          title={t("Start MCP server on launch?")}
          className="agent-control-startup-dialog"
          descriptionId="agent-control-startup-description"
          initialFocus={disabledButton}
          closeDisabled={saving}
          onClose={() => {
            if (saving) return;
            setDismissed(true);
            setReadyToPrompt(false);
          }}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p id="agent-control-startup-description">
                {t(
                  "Local AI agents can then request access to your workspaces. Change this anytime in Settings → Agent control.",
                )}
              </p>
              {saveError && (
                <p className="keybindings-error" role="alert">
                  {t("Could not save your choice: {error}", {
                    error: saveError,
                  })}
                </p>
              )}
            </div>
            <div className="dialog-actions">
              <button
                ref={disabledButton}
                type="button"
                className="button"
                disabled={saving}
                onClick={() => void decide(false)}
              >
                {t("Not now")}
              </button>
              <button
                type="button"
                className="button button-primary"
                disabled={saving}
                onClick={() => void decide(true)}
              >
                {saving ? t("Saving…") : t("Turn on")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </>
  );
}
