import { useId } from "react";
import { useT } from "../i18n";
import { ChevronRight, Smartphone } from "../icons";
import { deviceStatusLabels } from "./device-status";
import { imageTitle } from "./settings-model";
import type { Snapshot } from "./types";

export default function PhonePicker({
  snapshot,
  missing,
  disabled,
  onChoose,
  onManage,
}: {
  snapshot: Snapshot;
  missing: boolean;
  disabled: boolean;
  onChoose: (id: string) => void;
  onManage: () => void;
}) {
  const { t } = useT();
  const devices = snapshot.devices?.devices ?? [];
  const id = useId();
  return (
    <div className="android-panel-state android-phone-picker">
      <header className="android-state-heading">
        <span className="android-state-icon" aria-hidden="true">
          <Smartphone size={22} />
        </span>
        <div className="android-state-copy">
          <h2>
            {devices.length
              ? t("Choose a phone")
              : t("Create your first phone")}
          </h2>
          <p>
            {missing
              ? t(
                  "This phone is unavailable. Choose another one or manage your devices in Settings.",
                )
              : devices.length
                ? t("Open a phone to use its apps and data here.")
                : t(
                    "Set up a virtual phone in Android settings to get started.",
                  )}
          </p>
        </div>
      </header>
      {!!devices.length && (
        <ul
          className="android-phone-choices"
          aria-label={t("Available phones")}
        >
          {devices.map((device) => {
            const phase =
              snapshot.statuses.find((status) => status.deviceId === device.id)
                ?.phase ?? "stopped";
            return (
              <li key={device.id}>
                <button
                  className="android-phone-choice"
                  aria-label={t("Open {name}", { name: device.name })}
                  aria-describedby={`${id}-${device.id} ${id}-${device.id}-status`}
                  disabled={disabled || phase === "stopping"}
                  onClick={() => onChoose(device.id)}
                >
                  <Smartphone size={18} aria-hidden="true" />
                  <span className="android-phone-choice-info">
                    <strong>{device.name}</strong>
                    <span id={`${id}-${device.id}`}>
                      {imageTitle({ id: device.image })}
                    </span>
                    <span
                      className="android-phone-choice-status"
                      id={`${id}-${device.id}-status`}
                    >
                      <span
                        className={`android-status-dot${phase === "running" ? " is-running" : ""}`}
                        aria-hidden="true"
                      />
                      {t(deviceStatusLabels[phase])}
                    </span>
                  </span>
                  <ChevronRight
                    size={14}
                    className="android-phone-choice-action"
                    aria-hidden="true"
                  />
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <div className="android-state-actions">
        <button
          className={devices.length ? "text-button" : "button button-primary"}
          disabled={disabled}
          onClick={onManage}
        >
          {devices.length ? t("Manage devices") : t("Create a phone")}
        </button>
      </div>
    </div>
  );
}
