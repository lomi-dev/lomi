import { useState, useRef } from "react";
import { api, errorMessage } from "../api";
import { newId } from "../model";
import { Modal } from "../ui";
import { acquireSessionPublication } from "./migration-runtime";
import { refresh } from "./task-runtime";
interface Preview {
  migrationId: string;
  digest: string;
  accountCount: number;
  taskCount: number;
  missingCount: number;
  phase: string;
}
export default function LegacyImport() {
  const [preview, setPreview] = useState<Preview>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const [exportId, setExportId] = useState<string>();
  const cancel = useRef<HTMLButtonElement>(null);
  const publication = useRef<(() => Promise<void>) | undefined>(undefined);
  async function cancelImport() {
    try {
      await publication.current?.();
      publication.current = undefined;
      setPreview(undefined);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }
  async function load() {
    setBusy(true);
    setError("");
    try {
      publication.current = await acquireSessionPublication();
      setPreview(await api<Preview>("agent_legacy_migration_preview"));
    } catch (cause) {
      await cancelImport();
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }
  async function apply() {
    if (!preview) return;
    setBusy(true);
    setError("");
    try {
      await api("agent_legacy_migration_apply", {
        request: {
          operationId: newId(),
          migrationId: preview.migrationId,
          expectedDigest: preview.digest,
        },
      });
      setStatus(
        "Saved accounts and task history imported. No task was started.",
      );
      await refresh();
      await cancelImport();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }
  async function exportArchive() {
    if (!preview) return;
    setBusy(true);
    setError("");
    try {
      const result = await api<{ exportId: string }>(
        "agent_legacy_migration_rollback",
        {
          request: {
            operationId: newId(),
            migrationId: preview.migrationId,
            expectedDigest: preview.digest,
          },
        },
      );
      setExportId(result.exportId);
      setStatus(
        "Rollback archive exported. Original data and newer task history are preserved.",
      );
      await cancelImport();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div>
      <button className="button" disabled={busy} onClick={() => void load()}>
        Import saved CLI history
      </button>
      {status && <p role="status">{status}</p>}
      {exportId && (
        <button
          className="button"
          onClick={() =>
            void api("agent_legacy_migration_export_open", { exportId }).catch(
              (cause) => setError(errorMessage(cause)),
            )
          }
        >
          Open rollback archive folder
        </button>
      )}
      {error && !preview && <p role="alert">{error}</p>}
      {preview && (
        <Modal
          title="Import saved CLI history?"
          initialFocus={cancel}
          closeDisabled={busy}
          onClose={() => void cancelImport()}
        >
          <div className="dialog-form">
            <p>
              Preserve original account logins and archive saved task history.
              Import does not start tasks or replay old inputs.
            </p>
            <p>
              {preview.accountCount} accounts · {preview.taskCount} tasks ·{" "}
              {preview.missingCount} missing history records
            </p>
            <p role="status">
              Workspace layout changes, task dispatch and autosave are paused
              until you finish or cancel this import.
            </p>
            <p>
              Original data and exact session bytes remain backed up. Archived
              tasks require an explicit review before continuing.
            </p>
            <p>
              Export a rollback archive to review original data alongside newer
              task history. Export does not replace this workspace.
            </p>
            {error && <p role="alert">{error}</p>}
            <div className="dialog-actions">
              <button
                ref={cancel}
                className="button"
                disabled={busy}
                onClick={() => void cancelImport()}
              >
                Cancel
              </button>
              <button
                className="button"
                disabled={busy}
                onClick={() => void exportArchive()}
              >
                Export rollback archive
              </button>
              <button
                className="button button-primary"
                disabled={busy}
                onClick={() => void apply()}
              >
                Import reviewed archive
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}
