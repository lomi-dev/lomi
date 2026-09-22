import { useCallback, useEffect, useId, useRef, useState } from "react";
import { api, errorMessage } from "./api";
import type { TerminalContext, TitleProcess } from "./terminal-runtime";
import { Modal } from "./ui";

const cliNames = {
  codex: "Codex",
  agy: "agy",
  cursor: "Cursor CLI",
  claude: "Claude Code",
};

interface TitleSetup {
  cli: TitleProcess["cli"];
  path: string;
  revision: string | null;
}

export function useCliTitleSetup(
  onError: (message: string) => void,
  onConfigured: (message: string) => void,
) {
  const [request, setRequest] = useState<
    TitleSetup & { id: string; process: TitleProcess }
  >();
  const [busy, setBusy] = useState(false);
  const [activationRequired, setActivationRequired] = useState(false);
  const [error, setError] = useState("");
  const descriptionId = useId();
  const cancelButton = useRef<HTMLButtonElement>(null);
  const mounted = useRef(true);
  const checking = useRef(false);
  const checked = useRef(new Map<string, TitleProcess>());
  const offered = useRef(new Set<string>());
  const current = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const observe = useCallback(
    async (contexts: Record<string, TerminalContext>) => {
      for (const [id, process] of checked.current)
        if (
          contexts[id]?.titleCli?.pid !== process.pid ||
          contexts[id]?.titleCli?.cli !== process.cli
        )
          checked.current.delete(id);
      if (
        checking.current ||
        current.current ||
        document.visibilityState === "hidden" ||
        document.querySelector("dialog[open]")
      )
        return;
      checking.current = true;
      try {
        for (const [id, context] of Object.entries(contexts)) {
          const process = context.titleCli;
          if (!process || checked.current.has(id)) continue;
          try {
            const next = await api<TitleSetup | null>("inspect_cli_titles", {
              id,
              process,
            });
            if (!mounted.current) return;
            // Defer automatic prompts if another action opened a dialog during the read.
            if (document.querySelector("dialog[open]")) return;
            checked.current.set(id, process);
            if (!next || offered.current.has(next.path)) continue;
            offered.current.add(next.path);
            current.current = true;
            setError("");
            setRequest({ ...next, id, process });
            return;
          } catch (error) {
            checked.current.set(id, process);
            if (mounted.current)
              onError(
                `Could not check ${cliNames[process.cli]} title settings: ${errorMessage(error)}`,
              );
          }
        }
      } finally {
        checking.current = false;
      }
    },
    [onError],
  );

  const close = () => {
    if (busy) return;
    current.current = false;
    setRequest(undefined);
    setActivationRequired(false);
  };
  const save = async () => {
    if (!request || busy) return;
    setBusy(true);
    try {
      if (error) {
        const next = await api<TitleSetup | null>("inspect_cli_titles", {
          id: request.id,
          process: request.process,
        });
        setError("");
        if (next) {
          offered.current.add(next.path);
          setRequest({ ...next, id: request.id, process: request.process });
          return;
        }
      } else {
        const { cli: _cli, ...args } = request;
        await api("enable_cli_titles", args);
      }
      setRequest(undefined);
      if (request.cli === "agy") {
        setActivationRequired(true);
        return;
      }
      current.current = false;
      onConfigured(
        `${cliNames[request.cli]} title settings are ready. Restart ${cliNames[request.cli]} and resume your conversation to apply them.`,
      );
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  if (activationRequired)
    return {
      observe,
      dialog: (
        <Modal
          key="activation"
          className="cli-title-dialog"
          title="Activate agy terminal titles"
          descriptionId={descriptionId}
          initialFocus={cancelButton}
          onClose={close}
        >
          <div className="dialog-form">
            <div className="cli-title-description">
              <p id={descriptionId}>
                Settings saved. Enter <code>/title on</code> in your running agy
                session to activate terminal titles.
              </p>
              <p>
                <code>/resume</code> alone does not activate titles. Your
                current conversation can stay open.
              </p>
              <p>You can also restart agy and resume your conversation.</p>
            </div>
            <div className="dialog-actions">
              <button
                ref={cancelButton}
                type="button"
                className="button button-primary"
                onClick={close}
              >
                Got it
              </button>
            </div>
          </div>
        </Modal>
      ),
    };

  return {
    observe,
    dialog: request && (
      <Modal
        key="consent"
        className="cli-title-dialog"
        title={`Enable ${cliNames[request.cli]} terminal titles?`}
        descriptionId={descriptionId}
        initialFocus={cancelButton}
        onClose={close}
      >
        <div className="dialog-form">
          <div className="cli-title-description">
            <p id={descriptionId}>
              {cliNames[request.cli]} is running. Allow Lomi to enable terminal
              titles in its settings for all terminals?
            </p>
            <p style={{ overflowWrap: "anywhere" }}>
              <code>{request.path}</code>
            </p>
            {request.cli === "agy" && (
              <p>
                agy will use its existing title command, or Lomi’s local title
                formatter if none is configured.
              </p>
            )}
            <p>A backup of the existing file will be saved beside it.</p>
            <p>
              {request.cli === "agy" ? (
                <>
                  After allowing this change, enter <code>/title on</code> in
                  agy to activate titles for the current conversation.
                </>
              ) : (
                <>
                  Restart {cliNames[request.cli]} after allowing this change.
                  You can then resume your conversation.
                </>
              )}
            </p>
            {error && <p role="alert">{error}</p>}
          </div>
          <div className="dialog-actions">
            <button
              ref={cancelButton}
              type="button"
              className="button"
              disabled={busy}
              onClick={close}
            >
              Not now
            </button>
            <button
              type="button"
              className="button button-primary"
              disabled={busy}
              onClick={() => void save()}
            >
              {busy ? "Checking…" : error ? "Check again" : "Allow changes"}
            </button>
          </div>
        </div>
      </Modal>
    ),
  };
}
