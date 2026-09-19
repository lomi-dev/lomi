import { useEffect, useRef, useState } from "react";
import { api, errorMessage, native } from "../api";
import { listen } from "@tauri-apps/api/event";
import { DisclosureSummary, Modal } from "../ui";
import { RefreshCw, Plus, Download } from "../icons";
import DeviceForm from "./DeviceForm";
import Select from "../Select";
import { refreshAndroid, useAndroid } from "./state";
import {
  formatBytes,
  imageLabel,
  installationSelection,
  compareImages,
  imageInfo,
  imageFamily,
  imageDescription,
  androidVersion,
} from "./settings-model";
import type { Catalog, Device, Plan, Progress, Snapshot } from "./types";

interface Usage {
  freeBytes: number;
  growthReserveBytes: number;
  directories: Record<string, { logicalBytes: number; allocatedBytes: number }>;
}
type Confirm = {
  title: string;
  description: string;
  name?: string;
  action: () => Promise<unknown>;
};

function Confirmation({
  value,
  onClose,
  onDone,
}: {
  value: Confirm;
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <Modal
      className="android-dialog"
      title={value.title}
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <form
        className="android-device-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (busy || (value.name && text !== value.name)) return;
          setBusy(true);
          void value
            .action()
            .then(onDone)
            .then(onClose)
            .catch((error) => setError(errorMessage(error)))
            .finally(() => setBusy(false));
        }}
      >
        <p>{value.description}</p>
        {value.name && (
          <label>
            Type “{value.name}” to confirm
            <input
              autoFocus
              value={text}
              onChange={(event) => setText(event.target.value)}
              disabled={busy}
              autoComplete="off"
            />
          </label>
        )}
        {error && (
          <p className="keybindings-error" role="alert">
            {error}
          </p>
        )}
        <div className="dialog-actions">
          <button
            className="button"
            type="button"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            className="button danger"
            disabled={busy || (!!value.name && text !== value.name)}
          >
            {busy ? "Applying…" : value.title}
          </button>
        </div>
      </form>
    </Modal>
  );
}

function Installation({
  plan,
  snapshot,
  onClose,
  onDone,
}: {
  plan: Plan;
  snapshot: Snapshot;
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const [accepted, setAccepted] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return (
    <Modal
      className="android-dialog"
      title="Review Android installation"
      wide
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <form
        className="android-device-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (busy || accepted.length !== plan.licenses.length) return;
          setBusy(true);
          void api("android_install", { planId: plan.id, accepted })
            .then(onDone)
            .then(onClose)
            .catch((error) => setError(errorMessage(error)))
            .finally(() => setBusy(false));
        }}
      >
        <p>
          {formatBytes(plan.downloadBytes)} to download. Extraction and virtual
          phone data need additional space.
        </p>
        <p className="settings-help">
          Managed SDK: <code>{snapshot.sdkPath}</code>. Existing SDKs, shell
          settings and system Java remain separate.
        </p>
        <ul className="android-install-list">
          {plan.bootstrap && (
            <>
              <li>Android CLI {plan.bootstrap.cli.version}</li>
              <li>Private Java runtime {plan.bootstrap.java.version}</li>
            </>
          )}
          {plan.packages.map((pkg) => (
            <li key={pkg.id}>
              {pkg.name} · {pkg.revision} · {formatBytes(pkg.size)}
            </li>
          ))}
        </ul>
        <p>Read the required provider terms before accepting them.</p>
        {plan.licenses.map((license) => (
          <section
            className="android-license"
            key={license.digest}
            aria-label={license.id}
          >
            <h3>{license.id}</h3>
            <pre tabIndex={0} aria-label={`${license.id} terms`}>
              {license.text}
            </pre>
            <label className="android-checkbox">
              <input
                type="checkbox"
                checked={accepted.includes(license.digest)}
                disabled={busy}
                onChange={(event) =>
                  setAccepted((before) =>
                    event.target.checked
                      ? [...before, license.digest]
                      : before.filter((digest) => digest !== license.digest),
                  )
                }
              />
              I accept {license.id}
            </label>
          </section>
        ))}
        {error && (
          <p className="keybindings-error" role="alert">
            {error}
          </p>
        )}
        <div className="dialog-actions">
          <button
            className="button"
            type="button"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            className="button primary"
            disabled={busy || accepted.length !== plan.licenses.length}
          >
            {busy ? "Starting…" : "Install selected components"}
          </button>
        </div>
      </form>
    </Modal>
  );
}

function Operation({
  progress,
  onCancel,
  disabled,
}: {
  progress: Progress;
  onCancel: () => void;
  disabled: boolean;
}) {
  const active =
    progress.phase === "running" || progress.phase === "cancelling";
  return (
    <section className="android-operation" aria-label="Android operation">
      <div className="android-row">
        <div>
          <strong role="status">{progress.stage}</strong>
          <p className="settings-help">
            {active
              ? "This operation continues when Settings closes."
              : progress.phase === "succeeded"
                ? "Completed"
                : progress.phase === "cancelled"
                  ? "Cancelled safely"
                  : "Failed — review the error and retry or repair below."}
          </p>
        </div>
        {active && (
          <button
            className="button"
            disabled={disabled || progress.phase === "cancelling"}
            onClick={onCancel}
          >
            {progress.phase === "cancelling"
              ? "Cancelling…"
              : "Cancel operation"}
          </button>
        )}
      </div>
      {active && (
        <progress
          aria-label={progress.stage}
          max={progress.total || undefined}
          value={progress.total ? progress.received : undefined}
        />
      )}
      {active && progress.total > 0 && (
        <p className="settings-help">
          {formatBytes(progress.received)} / {formatBytes(progress.total)}
        </p>
      )}
      {progress.error && (
        <p className="keybindings-error" role="alert">
          {progress.error}
        </p>
      )}
    </section>
  );
}

export default function AndroidSettingsPage() {
  const state = useAndroid();
  const snapshot = state.snapshot;
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [usage, setUsage] = useState<Usage | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const locked = useRef(false);
  const [query, setQuery] = useState("");
  const [version, setVersion] = useState("recent");
  const [family, setFamily] = useState("all");
  const [plan, setPlan] = useState<Plan | null>(null);
  const [form, setForm] = useState<Device | "new" | null>(null);
  const [confirmation, setConfirmation] = useState<Confirm | null>(null);
  const [setup, setSetup] = useState<{ requestId: string } | null>(null);
  useEffect(() => {
    let current = true;
    let changed = false;
    const listener = native
      ? listen<{ requestId: string }>(
          "android-setup-changed",
          ({ payload }) => {
            changed = true;
            if (current) setSetup(payload);
          },
        )
      : Promise.resolve(() => {});
    void listener
      .then(() => api<{ requestId: string } | null>("android_setup_context"))
      .then((value) => {
        if (current && !changed) setSetup(value);
      })
      .catch((error) => {
        if (current) setError(errorMessage(error));
      });
    return () => {
      current = false;
      void listener.then((stop) => stop()).catch(() => {});
    };
  }, []);
  const progress = snapshot?.operation;
  const working =
    busy || progress?.phase === "running" || progress?.phase === "cancelling";
  const devices = snapshot?.devices?.devices ?? [];
  const installed = snapshot?.packages?.packages ?? {};
  const alive = snapshot?.statuses.some(
    (status) => status.processAlive || status.phase === "starting",
  );
  const editable =
    !!snapshot?.devices && !!snapshot.preferences && !!snapshot.packages;

  const perform = async (action: () => Promise<unknown>) => {
    if (locked.current) return;
    locked.current = true;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      locked.current = false;
      setBusy(false);
    }
  };
  const measure = async () => setUsage(await api<Usage>("android_storage"));
  const reload = async () => {
    await refreshAndroid();
    await measure();
  };
  useEffect(() => {
    void api<Usage>("android_storage")
      .then(setUsage)
      .catch((error) => setError(errorMessage(error)));
  }, []);
  const loadCatalog = async (refresh = false) => {
    const result = await api<Catalog>("android_catalog", { refresh });
    setCatalog(result);
    return result;
  };
  const review = async (
    ids: string[],
    repair = false,
    prepareTools = !snapshot?.toolchainReady ||
      snapshot.toolchainUpdateAvailable,
  ) => {
    if (!snapshot) return;
    const available = catalog ?? (await loadCatalog());
    const packages = installationSelection(available, snapshot, ids, repair);
    setPlan(
      await api<Plan>("android_install_plan", {
        catalogRevision: available.revision,
        packages,
        prepareTools,
      }),
    );
  };
  const maintain = (action: object) => api("android_maintenance", { action });
  const remove = (id: string) =>
    setConfirmation({
      title: "Remove component",
      description: `Remove ${id} and its retained previous version from the managed SDK. Device data is preserved.`,
      action: () =>
        maintain({
          type: "remove",
          packageId: id,
          expectedRevision: snapshot!.packages!.revision,
        }),
    });
  const manage = (action: object) => api("android_manage_device", { action });
  const stop = (device: Device, force = false) =>
    api("android_stop", { deviceId: device.id, force });
  const open = (device: Device, coldBoot: boolean, newTab = false) =>
    api("android_request_open", {
      requestId: newTab ? null : (setup?.requestId ?? null),
      deviceId: device.id,
      coldBoot,
    });
  const destructive = (device: Device, type: "wipe" | "delete") =>
    setConfirmation({
      title: type === "wipe" ? "Wipe device data" : "Delete device",
      name: device.name,
      description:
        type === "wipe"
          ? `Erase all apps, accounts and data from ${device.name}. The device ID stays the same. This cannot be undone.`
          : `Permanently delete ${device.name} and all its apps and data. Open panels will show a missing device. This cannot be undone.`,
      action: () =>
        manage({
          type,
          deviceId: device.id,
          confirmation: device.name,
          expectedRevision: snapshot!.devices!.revision,
        }),
    });
  const availableImages = (
    catalog?.packages.filter((pkg) => pkg.image) ?? []
  ).sort(compareImages);
  const versions = [
    ...new Set(availableImages.map((pkg) => imageInfo(pkg).api)),
  ].sort((a, b) => b - a);
  const images = availableImages.filter(
    (pkg) =>
      (family === "all" || imageFamily(pkg) === family) &&
      (version === "all" ||
        (version === "recent"
          ? imageInfo(pkg).api >= (versions[0] ?? 0) - 2
          : imageInfo(pkg).api === Number(version))) &&
      `${pkg.name} ${imageLabel(pkg)}`
        .toLowerCase()
        .includes(query.toLowerCase()),
  );

  return (
    <main className="keybindings-page android-settings-page">
      <header className="settings-page-heading">
        <div>
          <h1>Android</h1>
          <p>Local virtual phones, inside your workspace.</p>
        </div>
        <button
          className="button"
          disabled={busy || state.loading}
          onClick={() => void perform(reload)}
        >
          <RefreshCw size={14} />
          Refresh
        </button>
      </header>
      {(error || state.error) && (
        <div className="keybindings-error" role="alert">
          <span>{error || state.error}</span>
          <button className="text-button" onClick={() => void perform(reload)}>
            Retry
          </button>
        </div>
      )}
      {notice && (
        <p role="status" className="settings-help">
          {notice}
        </p>
      )}
      {!snapshot ? (
        <p role="status" className="settings-help">
          {state.loading
            ? "Checking the managed Android environment…"
            : "Android environment could not be loaded. Retry after resolving the error above."}
        </p>
      ) : (
        <>
          {progress && (
            <Operation
              progress={progress}
              disabled={busy}
              onCancel={() =>
                void perform(() =>
                  api("android_cancel_operation", {
                    operationId: progress.operationId,
                  }),
                )
              }
            />
          )}
          <section className="keybindings-group" aria-label="Environment">
            <h2>Environment</h2>
            <div className="android-row">
              <div>
                <strong>
                  {snapshot.acceleration.available
                    ? "Hardware acceleration available"
                    : "System preparation required"}
                </strong>
                <p className="settings-help">
                  {snapshot.host} · {snapshot.acceleration.backend}
                </p>
              </div>
            </div>
            {snapshot.acceleration.action && (
              <p className="keybindings-error" role="alert">
                {snapshot.acceleration.action}
              </p>
            )}
            {!snapshot.qualified && (
              <p className="settings-help">
                Android setup and Start are unavailable for this OS and CPU in
                this build. Support requires native verification; downloadable
                tools alone do not confirm compatibility.
              </p>
            )}
            <p className="settings-help android-path">
              Managed SDK: <code>{snapshot.sdkPath}</code>
            </p>
            <p className="settings-help">
              SDK manager {snapshot.toolchain.cli.version} and private Java{" "}
              {snapshot.toolchain.java.version}:{" "}
              {snapshot.toolchainReady ? "prepared" : "not prepared"}.
            </p>
            {usage && (
              <details className="android-storage">
                <DisclosureSummary>
                  {formatBytes(
                    Object.values(usage.directories).reduce(
                      (total, size) => total + size.allocatedBytes,
                      0,
                    ),
                  )}{" "}
                  used · {formatBytes(usage.freeBytes)} free
                </DisclosureSummary>
                <p className="settings-help">
                  Reserve for growth of existing phones:{" "}
                  {formatBytes(usage.growthReserveBytes)}. Sparse files can grow
                  up to their configured data capacity.
                </p>
                <dl>
                  {Object.entries(usage.directories).map(([name, size]) => (
                    <div key={name}>
                      <dt>{name}</dt>
                      <dd>
                        {formatBytes(size.allocatedBytes)} used ·{" "}
                        {formatBytes(size.logicalBytes)} logical
                      </dd>
                    </div>
                  ))}
                </dl>
              </details>
            )}
            <div className="android-actions">
              <button
                className="button"
                disabled={
                  working ||
                  alive ||
                  !editable ||
                  !snapshot.qualified ||
                  !snapshot.acceleration.available
                }
                onClick={() =>
                  void perform(() => review(snapshot.requiredTools))
                }
              >
                <Download size={14} />
                {snapshot.requiredTools.every((id) => installed[id])
                  ? "Update tools"
                  : "Install Android tools"}
              </button>
              <button
                className="button"
                disabled={
                  working ||
                  alive ||
                  !editable ||
                  !snapshot.qualified ||
                  !snapshot.acceleration.available
                }
                onClick={() =>
                  void perform(() =>
                    review(
                      snapshot.requiredTools.filter((id) => installed[id]),
                      true,
                      true,
                    ),
                  )
                }
              >
                Repair tools
              </button>
            </div>
            {alive && (
              <p className="settings-help">
                Stop running phones before installing, repairing, removing or
                rolling back components.
              </p>
            )}
            {snapshot.requiredTools
              .filter((id) => installed[id])
              .map((id) => (
                <div className="android-row" key={id}>
                  <div>
                    <strong>{id}</strong>
                    <p className="settings-help">
                      Installed {installed[id].revision}
                    </p>
                  </div>
                  <div className="android-actions">
                    {snapshot.rollbacks.find((pkg) => pkg.id === id) && (
                      <button
                        className="button"
                        disabled={working || alive}
                        onClick={() =>
                          setConfirmation({
                            title: "Roll back tool",
                            description: `Restore ${id} ${snapshot.rollbacks.find((pkg) => pkg.id === id)!.revision}. The current version will be kept as the previous copy.`,
                            action: () =>
                              maintain({
                                type: "rollback",
                                packageId: id,
                                expectedRevision: snapshot.packages!.revision,
                              }),
                          })
                        }
                      >
                        Roll back
                      </button>
                    )}
                    <button
                      className="button"
                      disabled={working || alive || !editable}
                      onClick={() => remove(id)}
                    >
                      Remove
                    </button>
                  </div>
                </div>
              ))}
            {Object.entries(snapshot.errors).map(([name, message]) => (
              <p key={name} className="keybindings-error" role="alert">
                {name}: {message}
              </p>
            ))}
          </section>
          <section className="keybindings-group" aria-label="System images">
            <h2>System images</h2>
            <p className="settings-help">
              Choose an image compatible with this host. Images used by any
              phone keep their installed revision, including while the phone is
              stopped.
            </p>
            <p className="settings-help">
              Choose Google Play for a familiar Android experience with Play
              Store, Google APIs for app testing, or AOSP for a minimal system.
              Stable versions are listed newest first; preview and Canary builds
              are excluded.
            </p>
            <div className="android-actions">
              <button
                className="button"
                disabled={busy}
                onClick={() => void perform(() => loadCatalog(!!catalog))}
              >
                {catalog ? "Refresh image catalog" : "Browse available images"}
              </button>
              {catalog && (
                <input
                  type="search"
                  aria-label="Filter system images"
                  placeholder="Android version or variant"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                />
              )}
            </div>
            {catalog && (
              <div className="android-image-filters">
                <label>
                  Android version
                  <Select
                    aria-label="Filter Android version"
                    value={version}
                    onChange={setVersion}
                    options={[
                      { value: "recent", label: "Recent versions" },
                      { value: "all", label: "All versions" },
                      ...versions.map((api) => ({
                        value: String(api),
                        label: androidVersion(api),
                      })),
                    ]}
                  />
                </label>
                <label>
                  Included apps
                  <Select
                    aria-label="Filter included apps"
                    value={family}
                    onChange={setFamily}
                    options={[
                      { value: "all", label: "All variants" },
                      { value: "play", label: "Google Play" },
                      { value: "google", label: "Google APIs" },
                      { value: "aosp", label: "AOSP" },
                    ]}
                  />
                </label>
              </div>
            )}
            {Object.values(installed)
              .filter((pkg) => pkg.id.startsWith("system-images;"))
              .sort(compareImages)
              .map((pkg) => {
                const users = devices.filter(
                  (device) => device.image === pkg.id,
                );
                return (
                  <div className="android-row" key={pkg.id}>
                    <div>
                      <strong>{imageLabel({ ...pkg, image: null })}</strong>
                      <p className="settings-help">
                        {users.length
                          ? `Used by ${users.map((device) => device.name).join(", ")}`
                          : "Installed · not used by a device"}
                      </p>
                    </div>
                    <div className="android-actions">
                      <button
                        className="button"
                        disabled={
                          working || alive || !editable || !snapshot.qualified
                        }
                        onClick={() =>
                          void perform(() => review([pkg.id], true))
                        }
                      >
                        Repair image
                      </button>
                      <button
                        className="button"
                        disabled={
                          working || alive || !!users.length || !editable
                        }
                        onClick={() => remove(pkg.id)}
                      >
                        Remove image
                      </button>
                    </div>
                  </div>
                );
              })}
            {catalog && (
              <div className="android-image-list">
                {images.map((pkg) => (
                  <div
                    className="android-row"
                    key={`${pkg.id}:${pkg.revision}`}
                    data-android-package={pkg.id}
                  >
                    <div>
                      <strong>{imageLabel(pkg)}</strong>
                      <p className="settings-help">{imageDescription(pkg)}</p>
                      <p className="settings-help">
                        {formatBytes(pkg.size)} download
                        {installed[pkg.id]
                          ? ` · installed r${installed[pkg.id].revision}`
                          : ""}
                      </p>
                    </div>
                    <button
                      className="button"
                      disabled={
                        working ||
                        alive ||
                        !snapshot.toolchainReady ||
                        !editable ||
                        !snapshot.qualified ||
                        !snapshot.acceleration.available ||
                        installed[pkg.id]?.revision === pkg.revision ||
                        devices.some((device) => device.image === pkg.id)
                      }
                      onClick={() => void perform(() => review([pkg.id]))}
                    >
                      {installed[pkg.id]?.revision === pkg.revision
                        ? "Installed"
                        : "Download image"}
                    </button>
                  </div>
                ))}
                {!images.length && (
                  <p className="settings-help">
                    No compatible images match this filter.
                  </p>
                )}
              </div>
            )}
          </section>
          <section className="keybindings-group" aria-label="Devices">
            <h2>Devices</h2>
            {setup && (
              <p className="settings-help">
                Open in workspace returns to the Android panel that requested
                setup. If it was closed, use Open in new tab explicitly.
              </p>
            )}
            <div className="android-actions">
              <button
                className="button"
                disabled={
                  working ||
                  !editable ||
                  !snapshot.qualified ||
                  !snapshot.toolchainReady ||
                  !snapshot.profiles.length ||
                  !Object.keys(installed).some((id) =>
                    id.startsWith("system-images;"),
                  )
                }
                onClick={() => setForm("new")}
              >
                <Plus size={14} />
                Create device
              </button>
            </div>
            {!devices.length && (
              <p className="settings-help">
                Install the tools and a system image, then create a phone. Two
                views of one phone share its apps and data; create another phone
                for an independent environment.
              </p>
            )}
            {devices.map((device) => {
              const status = snapshot.statuses.find(
                (item) => item.deviceId === device.id,
              );
              const active =
                status?.processAlive || status?.phase === "starting";
              return (
                <article
                  className="android-device-card"
                  key={device.id}
                  aria-label={device.name}
                >
                  <div className="android-row">
                    <div>
                      <h3>
                        {device.name}
                        {snapshot.preferences?.defaultDeviceId ===
                          device.id && (
                          <span className="android-default">Default</span>
                        )}
                      </h3>
                      <p className="settings-help">
                        {status?.phase ?? "stopped"} ·{" "}
                        {imageLabel({
                          id: device.image,
                          revision: String(device.imageRevision),
                          image: null,
                        })}
                      </p>
                    </div>
                  </div>
                  <div className="android-actions">
                    <button
                      className="button"
                      disabled={working || !editable}
                      onClick={() => setForm(device)}
                    >
                      Configure
                    </button>
                    <button
                      className="button"
                      disabled={working || !editable || !snapshot.qualified}
                      onClick={() => void perform(() => open(device, false))}
                    >
                      Open in workspace
                    </button>
                    {setup && (
                      <button
                        className="button"
                        disabled={working || !editable || !snapshot.qualified}
                        onClick={() =>
                          void perform(() => open(device, false, true))
                        }
                      >
                        Open in new tab
                      </button>
                    )}
                    <button
                      className="button"
                      disabled={working || !editable || !snapshot.qualified}
                      onClick={() => void perform(() => open(device, true))}
                    >
                      Cold boot
                    </button>
                    <button
                      className="button"
                      disabled={
                        working ||
                        !snapshot.preferences ||
                        snapshot.preferences.defaultDeviceId === device.id
                      }
                      onClick={() =>
                        void perform(async () => {
                          await api("save_android_preferences", {
                            data: {
                              ...snapshot.preferences,
                              defaultDeviceId: device.id,
                            },
                            expectedRevision: snapshot.preferences!.revision,
                          });
                          await refreshAndroid();
                        })
                      }
                    >
                      Use as default
                    </button>
                    {active && (
                      <button
                        className="button"
                        disabled={busy}
                        onClick={() => void perform(() => stop(device))}
                      >
                        {status?.phase === "stopping" ? "Retry Stop" : "Stop"}
                      </button>
                    )}
                    {active && status?.phase === "failed" && (
                      <button
                        className="button"
                        disabled={busy}
                        onClick={() =>
                          setConfirmation({
                            title: "Force stop",
                            name: device.name,
                            description: `Force the owned process for ${device.name} to stop. Unsaved Android data can be lost.`,
                            action: () => stop(device, true),
                          })
                        }
                      >
                        Force stop
                      </button>
                    )}
                  </div>
                  <div className="android-actions">
                    <button
                      className="text-button"
                      disabled={
                        working || active || !editable || !snapshot.qualified
                      }
                      onClick={() => destructive(device, "wipe")}
                    >
                      Wipe data…
                    </button>
                    <button
                      className="text-button"
                      disabled={working || active || !editable}
                      onClick={() => destructive(device, "delete")}
                    >
                      Delete device…
                    </button>
                  </div>
                  {status?.serial && (
                    <p className="settings-help android-path">
                      ADB serial: <code>{status.serial}</code> · ADB:{" "}
                      <code>{snapshot.adbPath}</code>
                    </p>
                  )}
                  {status?.error && (
                    <p className="keybindings-error" role="alert">
                      {status.error}
                    </p>
                  )}
                </article>
              );
            })}
          </section>
          <section className="keybindings-group" aria-label="Maintenance">
            <h2>Maintenance</h2>
            <p className="settings-help">
              Repair interrupted publications, remove temporary downloads, or
              export limited diagnostic logs. Cleanup preserves all device data.
            </p>
            <div className="android-actions">
              <button
                className="button"
                disabled={working || alive}
                onClick={() =>
                  void perform(async () => {
                    await manage({ type: "recover" });
                    await refreshAndroid();
                  })
                }
              >
                Repair interrupted operation
              </button>
              <button
                className="button"
                disabled={working || alive}
                onClick={() =>
                  void perform(async () => {
                    await maintain({ type: "cleanup" });
                    await refreshAndroid();
                  })
                }
              >
                Clean temporary files
              </button>
              <button
                className="button"
                disabled={busy}
                onClick={() =>
                  void perform(async () => {
                    if (await api<boolean>("android_export_diagnostics"))
                      setNotice("Diagnostic logs exported.");
                  })
                }
              >
                Export diagnostics
              </button>
            </div>
            {snapshot.recovery.map((item) => (
              <div className="android-row" key={item.file}>
                <div>
                  <strong>Recover {item.file}</strong>
                  <p className="settings-help">
                    The original damaged file is preserved before replacement.
                    Device data is kept.
                    {item.backupRevision !== null
                      ? ` Saved backup revision: ${item.backupRevision}.`
                      : " No valid previous backup is available."}
                  </p>
                </div>
                <div className="android-actions">
                  {item.backupRevision !== null && (
                    <button
                      className="button"
                      disabled={working || alive}
                      onClick={() =>
                        setConfirmation({
                          title: "Restore metadata backup",
                          description: `Restore the previous ${item.file} file. Newer configuration edits may not be in this backup. All AVD files are preserved.`,
                          action: () =>
                            maintain({
                              type: "restoreMetadata",
                              file: item.file,
                              digest: item.digest,
                              reset: false,
                            }),
                        })
                      }
                    >
                      Restore backup
                    </button>
                  )}
                  {item.file === "preferences" && (
                    <button
                      className="button"
                      disabled={working || alive}
                      onClick={() =>
                        setConfirmation({
                          title: "Reset Android preferences",
                          description:
                            "Clear the default-device preference and preserve the damaged preferences file for recovery. Devices and their data are unchanged.",
                          action: () =>
                            maintain({
                              type: "restoreMetadata",
                              file: item.file,
                              digest: item.digest,
                              reset: true,
                            }),
                        })
                      }
                    >
                      Reset preferences
                    </button>
                  )}
                </div>
              </div>
            ))}
          </section>
        </>
      )}
      {plan && snapshot && (
        <Installation
          plan={plan}
          snapshot={snapshot}
          onClose={() => setPlan(null)}
          onDone={refreshAndroid}
        />
      )}
      {form && snapshot && (
        <DeviceForm
          snapshot={snapshot}
          device={form === "new" ? undefined : form}
          onClose={() => setForm(null)}
          onSave={async (draft) => {
            await manage(
              form === "new"
                ? {
                    type: "create",
                    expectedRevision: snapshot.devices!.revision,
                    draft,
                  }
                : {
                    type: "update",
                    expectedRevision: snapshot.devices!.revision,
                    device: { ...form, ...draft },
                  },
            );
            await refreshAndroid();
          }}
        />
      )}
      {confirmation && (
        <Confirmation
          value={confirmation}
          onClose={() => setConfirmation(null)}
          onDone={reload}
        />
      )}
    </main>
  );
}
