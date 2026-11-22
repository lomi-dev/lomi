import { useState } from "react";
import { api } from "./api";
import type { Workspace } from "./model";
import {
  sanitizeWorkspaceAppearance,
  type WorkspaceAppearance,
} from "./workspace-appearance";
import { useT } from "./i18n";
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
  project,
  onSave,
  onClose,
}: {
  workspace: Workspace;
  project: string;
  onSave: (appearance?: WorkspaceAppearance) => void;
  onClose: () => void;
}) {
  const { t } = useT();
  const [appearance, setAppearance] = useState<WorkspaceAppearance>(
    workspace.appearance ?? {},
  );
  const [error, setError] = useState("");
  return (
    <Modal
      title={t("Customize workspace")}
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
            <WorkspaceAvatar appearance={appearance} project={project} />
            <div>
              <strong>{workspace.name}</strong>
              <p>{t("Give this workspace its own identity.")}</p>
            </div>
          </div>
          <fieldset className="workspace-appearance-field">
            <legend>{t("Icon")}</legend>
            <div className="workspace-icon-options">
              {workspaceIconOptions.map(({ id, label, Icon }) => (
                <button
                  type="button"
                  key={id}
                  title={t(label)}
                  aria-label={t("{label} icon", { label: t(label) })}
                  aria-pressed={
                    !appearance.image &&
                    !appearance.logo &&
                    appearance.icon === id
                  }
                  onClick={() => {
                    setAppearance(
                      ({ image: _image, logo: _logo, ...current }) => ({
                        ...current,
                        icon: id,
                      }),
                    );
                  }}
                >
                  <Icon size={20} />
                </button>
              ))}
            </div>
          </fieldset>
          <fieldset className="workspace-appearance-field">
            <legend>{t("Logo")}</legend>
            <div className="workspace-logo-options">
              <button
                type="button"
                className="button"
                aria-pressed={!appearance.logo && !appearance.icon}
                onClick={() => {
                  setError("");
                  setAppearance(
                    ({ image: _image, logo: _logo, icon: _icon, ...rest }) =>
                      rest,
                  );
                }}
              >
                {t("Automatic")}
              </button>
              <button
                type="button"
                className="button"
                aria-pressed={!!appearance.logo}
                onClick={() => {
                  setError("");
                  api<string | null>("workspace_logo_pick")
                    .then((logo) => {
                      if (logo)
                        setAppearance(
                          ({ image: _image, icon: _icon, ...rest }) => ({
                            ...rest,
                            logo,
                          }),
                        );
                    })
                    .catch((reason) => setError(String(reason)));
                }}
              >
                {t("Custom image")}
              </button>
            </div>
            {error && <p role="alert">{error}</p>}
          </fieldset>
          <fieldset className="workspace-appearance-field">
            <legend>{t("Color")}</legend>
            <div className="workspace-color-options">
              {colors.map((color) => (
                <button
                  type="button"
                  key={color}
                  aria-label={t("Use color {color}", { color })}
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
            {t("Reset")}
          </button>
          <button type="button" className="button" onClick={onClose}>
            {t("Cancel")}
          </button>
          <button type="submit" className="button button-primary">
            {t("Save")}
          </button>
        </div>
      </form>
    </Modal>
  );
}
