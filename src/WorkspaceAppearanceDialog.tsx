import { useId, useState } from "react";
import { api } from "./api";
import type { Workspace } from "./model";
import type { WorkspaceAppearance } from "./workspace-appearance";
import { useT } from "./i18n";
import { Modal } from "./ui";
import WorkspaceAvatar from "./WorkspaceAvatar";

/** Name and logo of a workspace; without a custom logo the project favicon or GitHub avatar is used. */
export default function WorkspaceAppearanceDialog({
  workspace,
  project,
  onSave,
  onClose,
}: {
  workspace: Workspace;
  project: string;
  onSave: (name: string, appearance?: WorkspaceAppearance) => void;
  onClose: () => void;
}) {
  const { t } = useT();
  const nameId = useId();
  const [name, setName] = useState(workspace.name);
  const [logo, setLogo] = useState(workspace.appearance?.logo);
  const [error, setError] = useState("");
  const pick = () => {
    setError("");
    api<string | null>("workspace_logo_pick")
      .then((picked) => picked && setLogo(picked))
      .catch((reason) => setError(String(reason)));
  };
  return (
    <Modal
      title={t("Edit workspace")}
      onClose={onClose}
      className="workspace-appearance-dialog"
    >
      <form
        className="dialog-form dialog-layout"
        onSubmit={(event) => {
          event.preventDefault();
          if (!name.trim()) return;
          onSave(name.trim(), logo ? { logo } : undefined);
          onClose();
        }}
      >
        <div className="dialog-content workspace-appearance-body">
          <div className="workspace-appearance-logo">
            <WorkspaceAvatar appearance={{ logo }} project={project} />
            <div className="workspace-appearance-logo-actions">
              <button type="button" className="button" onClick={pick}>
                {t("Upload logo")}
              </button>
              {logo && (
                <button
                  type="button"
                  className="text-button"
                  onClick={() => setLogo(undefined)}
                >
                  {t("Remove")}
                </button>
              )}
            </div>
          </div>
          {error && (
            <p className="workspace-appearance-error" role="alert">
              {error}
            </p>
          )}
          <label className="workspace-appearance-name" htmlFor={nameId}>
            {t("Name")}
            <input
              id={nameId}
              autoFocus
              maxLength={80}
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </label>
        </div>
        <div className="dialog-actions">
          <button type="button" className="button" onClick={onClose}>
            {t("Cancel")}
          </button>
          <button
            type="submit"
            className="button button-primary"
            disabled={!name.trim()}
          >
            {t("Save")}
          </button>
        </div>
      </form>
    </Modal>
  );
}
