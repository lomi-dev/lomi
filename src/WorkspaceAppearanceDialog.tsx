import { useState } from "react";
import type { Workspace } from "./model";
import {
  sanitizeWorkspaceAppearance,
  type WorkspaceAppearance,
} from "./workspace-appearance";
import { Modal } from "./ui";
import WorkspaceAvatar, { workspaceIconOptions } from "./WorkspaceAvatar";

const colors = [
  "#b6ef5b",
  "#6ba8ff",
  "#66c9b6",
  "#f6bf66",
  "#ef8aab",
  "#b09aef",
  "#f08066",
  "#98a6b8",
];

export default function WorkspaceAppearanceDialog({
  workspace,
  onSave,
  onClose,
}: {
  workspace: Workspace;
  onSave: (appearance?: WorkspaceAppearance) => void;
  onClose: () => void;
}) {
  const [appearance, setAppearance] = useState<WorkspaceAppearance>(
    workspace.appearance ?? {},
  );
  return (
    <Modal
      title="Customize workspace"
      onClose={onClose}
      className="workspace-appearance-dialog"
    >
      <form
        className="dialog-form dialog-layout"
        onSubmit={(event) => {
          event.preventDefault();
          onSave(sanitizeWorkspaceAppearance(appearance));
          onClose();
        }}
      >
        <div className="dialog-content workspace-appearance-body">
          <div className="workspace-appearance-preview">
            <WorkspaceAvatar appearance={appearance} />
            <div>
              <strong>{workspace.name}</strong>
              <p>Give this workspace its own identity.</p>
            </div>
          </div>
          <fieldset className="workspace-appearance-field">
            <legend>Icon</legend>
            <div className="workspace-icon-options">
              {workspaceIconOptions.map(({ id, label, Icon }) => (
                <button
                  type="button"
                  key={id}
                  title={label}
                  aria-label={`${label} icon`}
                  aria-pressed={
                    !appearance.image && (appearance.icon || "layers") === id
                  }
                  onClick={() => {
                    setAppearance(({ image: _image, ...current }) => ({
                      ...current,
                      icon: id,
                    }));
                  }}
                >
                  <Icon size={20} />
                </button>
              ))}
            </div>
          </fieldset>
          <fieldset className="workspace-appearance-field">
            <legend>Color</legend>
            <div className="workspace-color-options">
              {colors.map((color) => (
                <button
                  type="button"
                  key={color}
                  aria-label={`Use color ${color}`}
                  aria-pressed={appearance.color === color}
                  style={{ backgroundColor: color }}
                  onClick={() =>
                    setAppearance((current) => ({ ...current, color }))
                  }
                />
              ))}
            </div>
          </fieldset>
        </div>
        <div className="dialog-actions">
          <button
            type="button"
            className="button workspace-appearance-reset"
            onClick={() => setAppearance({})}
          >
            Reset
          </button>
          <button type="button" className="button" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="button button-primary">
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
}
