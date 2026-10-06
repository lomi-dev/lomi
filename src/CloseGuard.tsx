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
import { closeChatViews, hasActiveChatRequests } from "./chat/chat-service";
import {
  closeAgentTaskViews,
  hasActiveAgentTasks,
  type TaskCloseLease,
} from "./agent-runtime/task-runtime";
import { Modal } from "./ui";

export function useCloseGuard() {
  const editor = useEditorCloseGuard();
  const checking = useRef(false);
  const cliCloses = useRef(new WeakMap<ReadonlySet<string>, TaskCloseLease>());
  const release = useCallback(async (fileIds: ReadonlySet<string>) => {
    const lease = cliCloses.current.get(fileIds);
    if (!lease) return;
    try {
      await lease.release();
      cliCloses.current.delete(fileIds);
    } catch (error) {
      setChatError(errorMessage(error));
    }
  }, []);
  const resolve = useRef<(close: boolean) => void>(undefined);
  const [request, setRequest] = useState<{
    message: string;
    application: boolean;
  }>();
  const [chatError, setChatError] = useState("");
  const descriptionId = useId();
  const confirmButton = useRef<HTMLButtonElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const confirm = useCallback(
    async (
      fileIds?: ReadonlySet<string>,
      terminalIds?: readonly string[],
      decision?: EditorCloseDecision,
      deferChats = false,
    ) => {
      if (checking.current) return false;
      checking.current = true;
      let cliClose: TaskCloseLease | undefined;
      try {
        const application = fileIds === undefined && terminalIds === undefined;
        let message = "";
        try {
          const count = await terminalsWithProcesses(terminalIds);
          if (count)
            message = `${count === 1 ? "This terminal has" : `${count} terminals have`} running processes.`;
        } catch (error) {
          message = errorMessage(error);
        }
        if (application && hasActiveChatRequests())
          message += `${message ? " " : ""}Chat AI is still generating a response.`;
        if (hasActiveAgentTasks(fileIds))
          message += `${message ? " " : ""}CLI Agent work is still running.`;
        if (message) {
          message += application
            ? " Quitting will stop active agents, terminal processes and AI responses. Are you sure you want to quit?"
            : " Closing will stop work in the final views of these runs and end any terminal sessions. Close anyway?";
          const approved = await new Promise<boolean>((finish) => {
            resolve.current = finish;
            setRequest({ message, application });
          });
          if (!approved) return false;
        }
        if (!(await editor.confirm(fileIds, decision))) return false;
        if (!application && fileIds)
          cliClose = await closeAgentTaskViews(fileIds);
        if (!application)
          await prepareOwnedTerminalClose(terminalIds ?? [...(fileIds ?? [])]);
        if (!deferChats) await closeChatViews(fileIds);
        if (fileIds && cliClose) cliCloses.current.set(fileIds, cliClose);
        return true;
      } catch (error) {
        try {
          await cliClose?.release();
          setChatError(errorMessage(error));
        } catch (releaseError) {
          setChatError(errorMessage(releaseError));
        }
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
    release,
    dialog: (
      <>
        {request && (
          <Modal
            protectTheme
            tone="warning"
            title={
              request.application ? "Quit Lomi?" : "Close running processes?"
            }
            descriptionId={descriptionId}
            initialFocus={request.application ? cancelButton : confirmButton}
            onClose={() => finish(false)}
          >
            <div className="dialog-form">
              <p id={descriptionId}>{request.message}</p>
              <div className="dialog-actions">
                <button
                  ref={cancelButton}
                  type="button"
                  className="button"
                  onClick={() => finish(false)}
                >
                  Cancel
                </button>
                <button
                  ref={confirmButton}
                  type="button"
                  className="button button-primary button-danger"
                  onClick={() => finish(true)}
                >
                  {request.application ? "Quit anyway" : "Close anyway"}
                </button>
              </div>
            </div>
          </Modal>
        )}
        {chatError && (
          <Modal
            protectTheme
            tone="danger"
            title="Views could not be closed"
            onClose={() => setChatError("")}
          >
            <div className="dialog-form">
              <p>{chatError}</p>
              <p>The views remain open. Resolve the error and retry closing.</p>
              <button className="button" onClick={() => setChatError("")}>
                Keep open
              </button>
            </div>
          </Modal>
        )}
        {editor.dialog}
      </>
    ),
  };
}
