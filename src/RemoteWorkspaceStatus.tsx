import { useId, useRef, useState } from "react";
import { Globe } from "./icons";
import { Modal } from "./ui";

export default function RemoteWorkspaceStatus({
  workspace,
  available,
  connectionMessage,
  busy,
  error,
  stopSharing,
}: {
  workspace: { id: string; name: string };
  available: boolean;
  connectionMessage: string;
  busy: boolean;
  error: string;
  stopSharing: () => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const cancel = useRef<HTMLButtonElement>(null);
  const descriptionId = useId();
  return (
    <>
      <div
        className="remote-workspace-status"
        title={`${workspace.name}: ${connectionMessage}`}
      >
        <span className="remote-workspace-status-label">
          <Globe size={12} aria-hidden="true" />
          Shared remotely
        </span>
        <button
          type="button"
          className="remote-workspace-stop"
          aria-label="Stop sharing remotely"
          aria-haspopup="dialog"
          disabled={!available || busy}
          onClick={(event) => {
            event.currentTarget.focus();
            setAttempted(false);
            setOpen(true);
          }}
        >
          Stop sharing
        </button>
      </div>
      {error && <span role="alert">{error}</span>}
      {open && (
        <Modal
          className="remote-sharing-dialog"
          title="Stop sharing remotely?"
          role="alertdialog"
          tone="warning"
          protectTheme
          descriptionId={descriptionId}
          initialFocus={cancel}
          closeDisabled={busy}
          onClose={() => setOpen(false)}
        >
          <div className="dialog-form dialog-layout" aria-busy={busy}>
            <div className="dialog-content">
              <p id={descriptionId}>
                Stop sharing “{workspace.name}” remotely? Existing local
                terminals will keep running.
                {!available && " Remote sharing requires the Lomi desktop app."}
              </p>
              {attempted && error && <p role="alert">{error}</p>}
            </div>
            <div className="dialog-actions">
              <button
                ref={cancel}
                type="button"
                className="button"
                disabled={busy}
                onClick={() => setOpen(false)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="button button-primary button-danger"
                disabled={!available || busy}
                onClick={() => {
                  if (!available) return;
                  setAttempted(true);
                  void stopSharing();
                }}
              >
                {busy ? "Stopping…" : "Stop sharing"}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </>
  );
}
