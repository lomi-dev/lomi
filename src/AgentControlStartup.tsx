import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import type { ControlStartupState } from "./agent-control-startup";
import { Modal } from "./ui";

export default function AgentControlStartup() {
  const [state, setState] = useState<ControlStartupState>();
  const [loadError, setLoadError] = useState("");
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
        setSaveError("The startup choice was not saved.");
    } catch (error) {
      if (request === decisionRevision.current)
        setSaveError(errorMessage(error));
    } finally {
      saveLock.current = false;
      setSaving(false);
    }
  };

  const startupError = state?.error;
  return (
    <>
      {loadError && (
        <div className="notice" role="alert">
          <span>
            Could not load automatic MCP startup settings: {loadError}
          </span>
          <button className="text-button" onClick={() => void refresh()}>
            Retry
          </button>
        </div>
      )}
      {startupError && state && (
        <div className="notice" role="alert">
          <span>
            {state.autoStart
              ? `Automatic MCP startup is enabled, but the server could not start: ${startupError} The saved choice is still enabled. Open Settings → Agent control to change it or enable control for this session.`
              : state.autoStart === false
                ? `Automatic MCP startup is disabled. ${startupError}`
                : `Could not load the saved automatic MCP startup choice: ${startupError}`}
          </span>
        </div>
      )}
      {eligible && readyToPrompt && (
        <Modal
          protectTheme
          title="Start the MCP server automatically?"
          className="agent-control-startup-dialog"
          descriptionId="agent-control-startup-description"
          initialFocus={disabledButton}
          onClose={() => {
            if (saving) return;
            setDismissed(true);
            setReadyToPrompt(false);
          }}
        >
          <div className="dialog-form">
            <p id="agent-control-startup-description">
              Enabling starts the MCP server now and on every Lomi launch. Local
              clients may request access to Lomi workspaces. Lomi approvals
              follow the YOLO mode choice in Settings → Agent control; your MCP
              client's own confirmation prompts remain independent. You can
              change this startup choice later in Settings → Agent control.
            </p>
            {saveError && (
              <p className="keybindings-error" role="alert">
                Could not save your choice: {saveError}
              </p>
            )}
            <div className="dialog-actions">
              <button
                ref={disabledButton}
                type="button"
                className="button"
                disabled={saving}
                onClick={() => void decide(false)}
              >
                Keep disabled
              </button>
              <button
                type="button"
                className="button button-primary"
                disabled={saving}
                onClick={() => void decide(true)}
              >
                {saving ? "Saving…" : "Enable automatic start"}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </>
  );
}
