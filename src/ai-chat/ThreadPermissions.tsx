import { useState } from "react";
import { useT } from "../i18n";
import Select from "../Select";
import { Modal } from "../ui";
import { PERMISSIONS } from "./settings/DefaultsSettingsPage";
import type { Permissions } from "./types";

/** Permission mode of a CLI thread; full access needs the same confirmation as the defaults. */
export default function ThreadPermissions({
  value,
  disabled,
  onChange,
}: {
  value: Permissions;
  disabled: boolean;
  onChange: (next: Permissions) => void;
}) {
  const { t } = useT();
  const [confirmFull, setConfirmFull] = useState(false);
  const Current = PERMISSIONS.find((option) => option.value === value)?.icon;
  return (
    <div
      className="thread-effort"
      title={
        disabled ? t("Stop the reply before changing permissions.") : undefined
      }
    >
      {Current && (
        <Current className="thread-effort-icon" size={15} aria-hidden />
      )}
      <Select
        aria-label={t("Permissions")}
        value={value}
        disabled={disabled}
        options={PERMISSIONS.map((option) => ({
          value: option.value,
          label: t(option.label),
        }))}
        onChange={(next) =>
          next === "full" ? setConfirmFull(true) : onChange(next as Permissions)
        }
      />
      {confirmFull && (
        <Modal
          title={t("Allow full access?")}
          tone="danger"
          onClose={() => setConfirmFull(false)}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p>
                {t(
                  "This thread will be able to run any command and edit files without asking you first. Use this only for projects you trust.",
                )}
              </p>
            </div>
            <div className="dialog-actions">
              <button
                type="button"
                className="button"
                onClick={() => setConfirmFull(false)}
              >
                {t("Cancel")}
              </button>
              <button
                type="button"
                className="button button-danger"
                onClick={() => {
                  setConfirmFull(false);
                  onChange("full");
                }}
              >
                {t("Allow full access")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}
