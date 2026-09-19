import { useId } from "react";
import { Smartphone } from "../icons";
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
  const devices = snapshot.devices?.devices ?? [];
  const id = useId();
  return (
    <div className="android-phone-picker">
      <header>
        <h2>{devices.length ? "Choose a phone" : "Create your first phone"}</h2>
        <p>
          {missing
            ? "This phone is unavailable. Choose another one or manage your devices in Settings."
            : devices.length
              ? "Open a phone to use its apps and data here."
              : "Set up a virtual phone in Android settings to get started."}
        </p>
      </header>
      {!!devices.length && (
        <ul className="android-phone-choices" aria-label="Available phones">
          {devices.map((device) => {
            const phase =
              snapshot.statuses.find((status) => status.deviceId === device.id)
                ?.phase ?? "stopped";
            return (
              <li key={device.id}>
                <button
                  className="android-phone-choice"
                  aria-label={`Open ${device.name}`}
                  aria-describedby={`${id}-${device.id} ${id}-${device.id}-status`}
                  disabled={disabled || phase === "stopping"}
                  onClick={() => onChoose(device.id)}
                >
                  <Smartphone size={22} aria-hidden="true" />
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
                      {deviceStatusLabels[phase]}
                    </span>
                  </span>
                  <span
                    className="android-phone-choice-action"
                    aria-hidden="true"
                  >
                    Open
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <button
        className={devices.length ? "text-button" : "button button-primary"}
        disabled={disabled}
        onClick={onManage}
      >
        {devices.length ? "Manage devices" : "Create a phone"}
      </button>
    </div>
  );
}
