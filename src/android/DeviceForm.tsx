import { useId, useState } from "react";
import Select from "../Select";
import { DisclosureSummary, Modal } from "../ui";
import { errorMessage } from "../api";
import {
  imageLabel,
  imageDescription,
  compareImages,
  compareProfiles,
  compatibleProfile,
  androidVersion,
} from "./settings-model";
import { parseDevice } from "./types";
import type { Device, Hardware, Snapshot } from "./types";

export default function DeviceForm({
  snapshot,
  device,
  onClose,
  onSave,
}: {
  snapshot: Snapshot;
  device?: Device;
  onClose: () => void;
  onSave: (
    draft: Pick<Device, "name" | "image" | "profile" | "hardware">,
  ) => Promise<void>;
}) {
  const id = useId();
  const images = Object.values(snapshot.packages?.packages ?? {})
    .filter((pkg) => pkg.id.startsWith("system-images;"))
    .sort(compareImages);
  const profiles = [...snapshot.profiles].sort(compareProfiles);
  const defaultProfile = profiles.find(
    (item) => images[0] && compatibleProfile(item, images[0]),
  );
  const [name, setName] = useState(
    device?.name ?? defaultProfile?.name ?? "Android phone",
  );
  const [customName, setCustomName] = useState(!!device);
  const [image, setImage] = useState(device?.image ?? images[0]?.id ?? "");
  const [profile, setProfile] = useState(
    device?.profile ?? defaultProfile?.id ?? "",
  );
  const [hardware, setHardware] = useState<Hardware>(
    device?.hardware ?? {
      ramMib: 2560,
      cpuCount: 2,
      dataGib: 4,
      gpu: "host",
      quickBoot: false,
    },
  );
  const [input, setInput] = useState(!!device?.inputBridge);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const selected = snapshot.profiles.find((item) => item.id === profile);
  const selectedImage = images.find((item) => item.id === image);
  const compatible =
    !!selected && !!selectedImage && compatibleProfile(selected, selectedImage);
  const chooseProfile = (value: string) => {
    setProfile(value);
    if (!customName)
      setName(
        profiles.find((item) => item.id === value)?.name ?? "Android phone",
      );
  };
  return (
    <Modal
      className="android-dialog"
      title={device ? `Configure ${device.name}` : "Create Android device"}
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <form
        className="android-device-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (busy || !input || !compatible) return;
          setError("");
          try {
            parseDevice({
              id: device?.id ?? "00000000-0000-0000-0000-000000000001",
              name: name.trim(),
              image,
              imageRevision: Number(
                snapshot.packages?.packages[image]?.revision,
              ),
              profile,
              hardware,
              inputBridge: input,
            });
          } catch (error) {
            setError(errorMessage(error));
            return;
          }
          setBusy(true);
          void onSave({ name: name.trim(), image, profile, hardware })
            .then(onClose)
            .catch((error) => setError(errorMessage(error)))
            .finally(() => setBusy(false));
        }}
      >
        <label htmlFor={`${id}-name`}>Name</label>
        <input
          id={`${id}-name`}
          autoFocus
          value={name}
          onChange={(event) => {
            setName(event.target.value);
            setCustomName(true);
          }}
          required
          disabled={busy}
        />
        <label htmlFor={`${id}-image`}>Android version</label>
        <Select
          id={`${id}-image`}
          value={image}
          disabled={busy || !!device}
          aria-describedby={`${id}-image-help`}
          onChange={(value) => {
            setImage(value);
            const next = images.find((item) => item.id === value);
            if (next && (!selected || !compatibleProfile(selected, next)))
              chooseProfile(
                profiles.find((item) => compatibleProfile(item, next))?.id ??
                  "",
              );
          }}
          options={images.map((pkg) => ({
            value: pkg.id,
            label: imageLabel({ ...pkg, image: null }),
          }))}
        />
        <p id={`${id}-image-help`} className="settings-help">
          {selectedImage
            ? imageDescription(selectedImage)
            : "Download an Android version from System images first."}
        </p>
        {device && (
          <p className="settings-help">
            To change Android version, variant, architecture or data capacity,
            create a new device. Existing apps and data stay on this phone.
          </p>
        )}
        <label htmlFor={`${id}-profile`}>Phone profile</label>
        <Select
          id={`${id}-profile`}
          value={profile}
          disabled={busy}
          aria-describedby={`${id}-profile-help`}
          onChange={chooseProfile}
          options={profiles.map((item) => ({
            value: item.id,
            label:
              selectedImage && compatibleProfile(item, selectedImage)
                ? item.name
                : `${item.name} · needs ${androidVersion(item.minApi, item.minMinorApi)}`,
            disabled: !selectedImage || !compatibleProfile(item, selectedImage),
          }))}
        />
        {selected && (
          <p className="settings-help" id={`${id}-profile-help`}>
            {selected.width} × {selected.height} · {selected.dpi} DPI · API{" "}
            {selected.minApi}
            {selected.minMinorApi ? `.${selected.minMinorApi}` : ""} or newer.
            The profile defines screen size and density, not the manufacturer’s
            complete hardware or exclusive apps. Foldable and resizable profiles
            are not available yet.
          </p>
        )}
        <details>
          <DisclosureSummary>Hardware and startup</DisclosureSummary>
          <div className="android-form-grid">
            {(
              [
                ["ramMib", "Memory (MiB)", 2560, 32768],
                ["cpuCount", "CPU cores", 1, 32],
                ["dataGib", "Data capacity (GiB)", 2, 128],
              ] as const
            ).map(([key, label, min, max]) => (
              <label key={key}>
                {label}
                <input
                  type="number"
                  min={min}
                  max={max}
                  step={1}
                  value={hardware[key]}
                  required
                  disabled={busy || (key === "dataGib" && !!device)}
                  onChange={(event) =>
                    setHardware({
                      ...hardware,
                      [key]: event.target.valueAsNumber,
                    })
                  }
                />
              </label>
            ))}
            <label htmlFor={`${id}-graphics`}>
              Graphics
              <Select
                id={`${id}-graphics`}
                value={hardware.gpu}
                disabled={busy}
                onChange={(gpu) =>
                  setHardware({ ...hardware, gpu: gpu as Hardware["gpu"] })
                }
                options={[
                  { value: "auto", label: "Automatic" },
                  { value: "host", label: "Host GPU" },
                  { value: "software", label: "Software" },
                ]}
              />
            </label>
          </div>
          <p className="settings-help">
            Host GPU is recommended for responsive graphics. Automatic can
            select software rendering when the emulator has no window; software
            rendering uses more CPU.
          </p>
          <p className="settings-help">
            Cold boot is used for this configuration. Quick Boot is not
            qualified. Audio, cameras, microphone and additional SD storage are
            disabled.
          </p>
        </details>
        {device && (
          <p className="settings-help">
            Name changes apply immediately. Hardware changes apply on the next
            start after Stop; the running phone keeps its current settings.
          </p>
        )}
        <label className="android-checkbox">
          <input
            type="checkbox"
            checked={input}
            disabled={busy || !!device}
            onChange={(event) => setInput(event.target.checked)}
            required
          />
          Enable SimpleBench text input
        </label>
        <p className="settings-help">
          SimpleBench installs and selects its bundled input method in this
          virtual phone for Unicode and composition. Typed text stays local and
          is sent only to the focused Android panel.
        </p>
        {error && (
          <p className="keybindings-error" role="alert">
            {error}
          </p>
        )}
        <div className="dialog-actions">
          <button
            type="button"
            className="button"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            className="button primary"
            disabled={busy || !input || !image || !profile || !compatible}
          >
            {busy ? "Saving…" : device ? "Save configuration" : "Create device"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
