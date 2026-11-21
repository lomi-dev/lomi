import { useCallback, useId, useRef, useState } from "react";
import {
  useEditorCloseGuard,
  type EditorCloseDecision,
} from "./EditorCloseGuard";
import {
  prepareOwnedTerminalClose,
  terminalsWithProcesses,
} from "./terminal-runtime";
import { errorMessage } from "./api";
import { Modal } from "./ui";
import { useT } from "./i18n";

export function useCloseGuard() {
  const { t, tp } = useT();
  const editor = useEditorCloseGuard();
  const checking = useRef(false);
  const resolve = useRef<(close: boolean) => void>(undefined);
  const [request, setRequest] = useState<{
    message: string;
    application: boolean;
  }>();
  const [closeError, setCloseError] = useState("");
  const descriptionId = useId();
  const confirmButton = useRef<HTMLButtonElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const confirm = useCallback(
    async (
      fileIds?: ReadonlySet<string>,
      terminalIds?: readonly string[],
      decision?: EditorCloseDecision,
    ) => {
      if (checking.current) return false;
      checking.current = true;
      try {
        const application = fileIds === undefined && terminalIds === undefined;
        let message = "";
        try {
          const count = await terminalsWithProcesses(terminalIds);
          if (count)
            message = tp(
              count,
              "This terminal has running processes.",
              "{count} terminals have running processes.",
            );
        } catch (error) {
          message = errorMessage(error);
        }
        if (message) {
          message += application
            ? ` ${t("Quitting will stop active agents and terminal processes. Are you sure you want to quit?")}`
            : ` ${t("Closing will stop work in the final views of these runs and end any terminal sessions. Close anyway?")}`;
          const approved = await new Promise<boolean>((finish) => {
            resolve.current = finish;
            setRequest({ message, application });
          });
          if (!approved) return false;
        }
        if (!(await editor.confirm(fileIds, decision))) return false;
        if (!application)
          await prepareOwnedTerminalClose(terminalIds ?? [...(fileIds ?? [])]);
        return true;
      } catch (error) {
        setCloseError(errorMessage(error));
        return false;
      } finally {
        checking.current = false;
      }
    },
    [editor.confirm],
  );
  const finish = (close: boolean) => {
    resolve.current?.(close);
    resolve.current = undefined;
    setRequest(undefined);
  };
  return {
    confirm,
    dialog: (
      <>
        {request && (
          <Modal
            protectTheme
            tone="warning"
            title={
              request.application
                ? t("Quit Lomi?")
                : t("Close running processes?")
            }
            descriptionId={descriptionId}
            initialFocus={request.application ? cancelButton : confirmButton}
            onClose={() => finish(false)}
          >
            <div className="dialog-form dialog-layout">
              <div className="dialog-content">
                <p id={descriptionId}>{request.message}</p>
              </div>
              <div className="dialog-actions">
                <button
                  ref={cancelButton}
                  type="button"
                  className="button"
                  onClick={() => finish(false)}
                >
                  {t("Cancel")}
                </button>
                <button
                  ref={confirmButton}
                  type="button"
                  className="button button-primary button-danger"
                  onClick={() => finish(true)}
                >
                  {request.application ? t("Quit anyway") : t("Close anyway")}
                </button>
              </div>
            </div>
          </Modal>
        )}
        {closeError && (
          <Modal
            protectTheme
            tone="danger"
            title={t("Views could not be closed")}
            onClose={() => setCloseError("")}
          >
            <div className="dialog-form dialog-layout">
              <div className="dialog-content">
                <p>{closeError}</p>
                <p>
                  {t(
                    "The views remain open. Resolve the error and retry closing.",
                  )}
                </p>
              </div>
              <div className="dialog-actions">
                <button className="button" onClick={() => setCloseError("")}>
                  {t("Keep open")}
                </button>
              </div>
            </div>
          </Modal>
        )}
        {editor.dialog}
      </>
    ),
  };
}
