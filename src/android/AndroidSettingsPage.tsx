import { useEffect, useRef, useState } from "react";
import { api, errorMessage, native } from "../api";
import { listen } from "@tauri-apps/api/event";
import { DisclosureSummary, IconButton, Modal } from "../ui";
import ContextMenu from "../ContextMenu";
import {
  RefreshCw,
  Plus,
  Download,
  Layers,
  Check,
  ChevronRight,
  Ellipsis,
  Play,
  Square,
  Smartphone,
} from "../icons";
import DeviceForm from "./DeviceForm";
import Select from "../Select";
import { deviceStatusLabels } from "./device-status";
import { refreshAndroid, useAndroid } from "./state";
import {
  formatBytes,
  imageLabel,
  imageTitle,
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
        <details className="android-install-details">
          <DisclosureSummary>Installation details</DisclosureSummary>
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
        </details>
        <p>Review and accept the provider terms to continue.</p>
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
            className="button button-primary"
            disabled={busy || accepted.length !== plan.licenses.length}
          >
            {busy ? "Starting…" : "Accept and install"}
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
  imageId,
  onRetry,
  retryDisabled,
}: {
  progress: Progress;
  onCancel: () => Promise<unknown>;
  disabled: boolean;
  imageId?: string;
  onRetry?: () => void;
  retryDisabled?: boolean;
}) {
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState("");
  const active =
    progress.phase === "running" || progress.phase === "cancelling";
  const stopping = cancelling || progress.phase === "cancelling";
  const title = imageId ? imageTitle({ id: imageId }) : progress.stage;
  const percent =
    progress.total > 0
      ? Math.min(100, Math.floor((progress.received / progress.total) * 100))
      : null;
  return (
    <section
      className={`android-operation${imageId ? " android-image-download" : ""}`}
      aria-label={imageId ? `${title} installation` : "Android operation"}
      data-android-package={imageId}
    >
      <div className="android-row">
        <div>
          <strong>{title}</strong>
          <p className="settings-help" role="status">
            {stopping
              ? "Cancelling…"
              : active
                ? imageId
                  ? progress.stage
                  : "This operation continues when Settings closes."
                : progress.phase === "succeeded"
                  ? "Completed"
                  : progress.phase === "cancelled"
                    ? "Cancelled safely"
                    : "Could not finish. Try again or open Advanced for repair options."}
          </p>
        </div>
        {active ? (
          <button
            className="button"
            aria-label={imageId && !stopping ? "Cancel download" : undefined}
            disabled={disabled || stopping}
            onClick={() => {
              setCancelling(true);
              setCancelError("");
              void onCancel()
                .catch((error) => setCancelError(errorMessage(error)))
                .finally(() => setCancelling(false));
            }}
          >
            {stopping ? "Cancelling…" : imageId ? "Cancel" : "Cancel operation"}
          </button>
        ) : (
          onRetry && (
            <button
              className="button"
              disabled={retryDisabled}
              onClick={onRetry}
            >
              {progress.phase === "cancelled" ? "Download again" : "Try again"}
            </button>
          )
        )}
      </div>
      {active && (
        <>
          <progress
            aria-label={
              imageId ? `Download progress for ${title}` : progress.stage
            }
            max={progress.total || undefined}
            value={progress.total ? progress.received : undefined}
          />
          {percent !== null && (
            <div className="android-download-meta">
              <span className="settings-help">
                {formatBytes(progress.received)} / {formatBytes(progress.total)}
              </span>
              <span className="settings-help">{percent}%</span>
            </div>
          )}
        </>
      )}
      {(cancelError || (progress.phase === "failed" && progress.error)) && (
        <p className="keybindings-error" role="alert">
          {cancelError || progress.error}
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
  const [imagesOpen, setImagesOpen] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const imagesHeading = useRef<HTMLButtonElement>(null);
  const imageDownload = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<{
    deviceId: string;
    x: number;
    y: number;
    trigger: HTMLButtonElement;
  } | null>(null);
  const closeMenu = () => {
    menu?.trigger.focus();
    setMenu(null);
  };
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
  const operationImages =
    progress?.packageIds?.filter((id) => id.startsWith("system-images;")) ?? [];
  const operationImageId =
    progress?.phase !== "succeeded" && operationImages.length === 1
      ? operationImages[0]
      : undefined;
  const downloading =
    progress?.phase === "running" || progress?.phase === "cancelling";
  useEffect(() => {
    if (operationImageId) setImagesOpen(true);
  }, [operationImageId, progress?.operationId]);
  useEffect(() => {
    if (imagesOpen && operationImageId)
      imageDownload.current?.scrollIntoView({ block: "center" });
  }, [imagesOpen, operationImageId, progress?.operationId]);
  const cancelOperation = () =>
    api("android_cancel_operation", { operationId: progress!.operationId });
  const working =
    busy || progress?.phase === "running" || progress?.phase === "cancelling";
  const devices = snapshot?.devices?.devices ?? [];
  const installed = snapshot?.packages?.packages ?? {};
  const alive = snapshot?.statuses.some(
    (status) => status.processAlive || status.phase === "starting",
  );
  const toolsReady =
    !!snapshot?.toolchainReady &&
    snapshot.requiredTools.every((id) => installed[id]);
  const installedImages = Object.values(installed).filter((pkg) =>
    pkg.id.startsWith("system-images;"),
  );
  const setupStep = !toolsReady ? 1 : !installedImages.length ? 2 : 3;
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
      pkg.id !== operationImageId &&
      (!installed[pkg.id] ||
        (installed[pkg.id].revision !== pkg.revision &&
          !devices.some((device) => device.image === pkg.id))) &&
      (family === "all" || imageFamily(pkg) === family) &&
      (version === "all" ||
        (version === "recent"
          ? imageInfo(pkg).api >= (versions[0] ?? 0) - 2
          : imageInfo(pkg).api === Number(version))),
  );

  return (
    <main className="keybindings-page android-settings-page">
      <header className="settings-page-heading">
        <div>
          <h1>Android</h1>
          <p>Local virtual phones, inside your workspace.</p>
        </div>
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
          {progress && progress.phase !== "succeeded" && !operationImageId && (
            <Operation
              key={progress.operationId}
              progress={progress}
              disabled={busy}
              onCancel={cancelOperation}
            />
          )}
          {!snapshot.qualified && (
            <p className="keybindings-error" role="status">
              Android setup is not available on this computer yet. You can still
              stop existing phones and recover their data.
            </p>
          )}
          {snapshot.acceleration.action && (
            <p className="keybindings-error" role="alert">
              {snapshot.acceleration.action}
            </p>
          )}
          {Object.entries(snapshot.errors).map(([name, message]) => (
            <p key={name} className="keybindings-error" role="alert">
              {name}: {message}
            </p>
          ))}
          {!!snapshot.recovery.length && !advancedOpen && (
            <button
              className="text-button"
              onClick={() => setAdvancedOpen(true)}
            >
              Review recovery options
            </button>
          )}
          {(!toolsReady || !installedImages.length || !devices.length) && (
            <section className="android-setup" aria-label="Android setup">
              <span className="android-step" role="status">
                Step {setupStep} of 3
              </span>
              <h2>
                {setupStep === 1
                  ? "Set up Android"
                  : setupStep === 2
                    ? "Choose your Android version"
                    : "Create your first phone"}
              </h2>
              <p className="settings-help">
                {setupStep === 1
                  ? "Install the tools to run a virtual phone in your workspace. You only need to do this once."
                  : setupStep === 2
                    ? "Download a version of Android for your phone. Choose Google Play if you need the Play Store."
                    : "Pick a phone model and give it a name. Your apps and data will stay on this phone."}
              </p>
              <button
                className="button button-primary"
                disabled={
                  working ||
                  !editable ||
                  !snapshot.qualified ||
                  !snapshot.acceleration.available ||
                  (setupStep < 3 && alive) ||
                  (setupStep === 3 && !snapshot.profiles.length)
                }
                onClick={() => {
                  if (setupStep === 1)
                    void perform(() => review(snapshot.requiredTools));
                  else if (setupStep === 2) {
                    setImagesOpen(true);
                    imagesHeading.current?.focus();
                    if (!catalog) void perform(() => loadCatalog());
                  } else setForm("new");
                }}
              >
                {setupStep === 3 ? (
                  <Plus size={14} />
                ) : setupStep === 2 ? (
                  <Layers size={14} />
                ) : (
                  <Download size={14} />
                )}
                {setupStep === 1
                  ? "Install Android tools"
                  : setupStep === 2
                    ? "Choose Android version"
                    : "Create device"}
              </button>
              {setupStep === 3 && !snapshot.profiles.length && (
                <p className="settings-help">
                  Phone models could not be loaded. Repair Android tools in
                  Advanced.
                </p>
              )}
            </section>
          )}
          {!!devices.length && (
            <section className="android-phones" aria-label="Your phones">
              <div className="android-section-heading">
                <h2>
                  Your phones <span>{devices.length}</span>
                </h2>
                <button
                  className="button"
                  disabled={
                    working ||
                    !editable ||
                    !snapshot.qualified ||
                    !toolsReady ||
                    !snapshot.profiles.length ||
                    !installedImages.length
                  }
                  onClick={() => setForm("new")}
                >
                  <Plus size={14} /> Create device
                </button>
              </div>
              {setup && (
                <p className="settings-help">
                  Open a phone to return to the workspace where you started
                  setup.
                </p>
              )}
              <div className="android-phone-list">
                {devices.map((device) => {
                  const status = snapshot.statuses.find(
                    (item) => item.deviceId === device.id,
                  );
                  const active =
                    status?.processAlive || status?.phase === "starting";
                  const phase = status?.phase ?? "stopped";
                  const statusLabel = deviceStatusLabels[phase];
                  return (
                    <article
                      className="android-device-card"
                      key={device.id}
                      aria-label={device.name}
                    >
                      <div className="android-phone-row">
                        <span className="android-phone-icon" aria-hidden="true">
                          <Smartphone size={23} />
                        </span>
                        <div className="android-phone-info">
                          <h3>{device.name}</h3>
                          <p className="settings-help">
                            {imageTitle({ id: device.image })}
                          </p>
                          <div className="android-phone-status">
                            <span
                              className={`android-status-dot${phase === "running" ? " is-running" : ""}`}
                              aria-hidden="true"
                            />
                            <span>{statusLabel}</span>
                            {snapshot.preferences?.defaultDeviceId ===
                              device.id && (
                              <span className="android-default">Default</span>
                            )}
                          </div>
                        </div>
                        <div className="android-phone-actions">
                          {active && (
                            <button
                              className="button"
                              disabled={busy}
                              onClick={() => void perform(() => stop(device))}
                            >
                              <Square size={12} />
                              {status?.phase === "stopping"
                                ? "Retry Stop"
                                : "Stop"}
                            </button>
                          )}
                          <button
                            className="button button-primary"
                            disabled={
                              working || !editable || !snapshot.qualified
                            }
                            onClick={() =>
                              void perform(() => open(device, false))
                            }
                          >
                            <Play size={13} /> Open
                          </button>
                          <IconButton
                            title={`Options for ${device.name}`}
                            aria-haspopup="menu"
                            aria-expanded={menu?.deviceId === device.id}
                            onClick={(event) => {
                              const rect =
                                event.currentTarget.getBoundingClientRect();
                              setMenu({
                                deviceId: device.id,
                                x: rect.right,
                                y: rect.bottom + 4,
                                trigger: event.currentTarget,
                              });
                            }}
                          >
                            <Ellipsis size={18} />
                          </IconButton>
                        </div>
                      </div>
                      {status?.error && (
                        <p className="keybindings-error" role="alert">
                          {status.error}
                        </p>
                      )}
                      {menu?.deviceId === device.id && (
                        <ContextMenu
                          x={menu.x}
                          y={menu.y}
                          label={`Options for ${device.name}`}
                          onClose={closeMenu}
                          actions={[
                            {
                              label: "Configure",
                              disabled: working || !editable,
                              run: () => setForm(device),
                            },
                            ...(setup
                              ? [
                                  {
                                    label: "Open in new tab",
                                    disabled:
                                      working ||
                                      !editable ||
                                      !snapshot.qualified,
                                    run: () =>
                                      void perform(() =>
                                        open(device, false, true),
                                      ),
                                  },
                                ]
                              : []),
                            {
                              label: "Use as default",
                              disabled:
                                working ||
                                !snapshot.preferences ||
                                snapshot.preferences.defaultDeviceId ===
                                  device.id,
                              run: () =>
                                void perform(async () => {
                                  await api("save_android_preferences", {
                                    data: {
                                      ...snapshot.preferences,
                                      defaultDeviceId: device.id,
                                    },
                                    expectedRevision:
                                      snapshot.preferences!.revision,
                                  });
                                  await refreshAndroid();
                                }),
                            },
                            {
                              label: "Cold boot",
                              disabled:
                                working || !editable || !snapshot.qualified,
                              run: () => void perform(() => open(device, true)),
                            },
                            ...(active && status?.phase === "failed"
                              ? [
                                  {
                                    label: "Force stop",
                                    danger: true,
                                    disabled: busy,
                                    run: () =>
                                      setConfirmation({
                                        title: "Force stop",
                                        name: device.name,
                                        description: `Force ${device.name} to stop. Unsaved Android data can be lost.`,
                                        action: () => stop(device, true),
                                      }),
                                  },
                                ]
                              : []),
                            null,
                            {
                              label: "Wipe data…",
                              danger: true,
                              disabled:
                                working ||
                                active ||
                                !editable ||
                                !snapshot.qualified,
                              run: () => destructive(device, "wipe"),
                            },
                            {
                              label: "Delete device…",
                              danger: true,
                              disabled: working || active || !editable,
                              run: () => destructive(device, "delete"),
                            },
                          ]}
                        />
                      )}
                    </article>
                  );
                })}
              </div>
            </section>
          )}
          {alive && (
            <p className="settings-help android-install-hint">
              Stop running phones before changing Android tools or versions.
            </p>
          )}
          <div className="android-disclosure">
            <button
              ref={imagesHeading}
              className="android-disclosure-toggle"
              aria-expanded={imagesOpen}
              aria-controls="android-versions"
              onClick={() => {
                setImagesOpen(!imagesOpen);
                if (!imagesOpen && !catalog) void perform(() => loadCatalog());
              }}
            >
              <ChevronRight size={15} />
              <span>
                <strong>Android versions</strong>
                <span>Download and manage phone systems</span>
              </span>
              {operationImageId && downloading ? (
                <span className="android-disclosure-meta android-download-indicator">
                  {progress?.phase === "cancelling"
                    ? "Cancelling…"
                    : progress && progress.total > 0
                      ? "Downloading…"
                      : "Installing…"}
                </span>
              ) : (
                !!installedImages.length && (
                  <span className="android-disclosure-meta">
                    {installedImages.length} installed
                  </span>
                )
              )}
            </button>
            <section
              className="android-disclosure-body"
              id="android-versions"
              hidden={!imagesOpen}
              aria-label="Android versions"
            >
              {operationImageId && progress && (
                <div ref={imageDownload}>
                  <Operation
                    key={progress.operationId}
                    progress={progress}
                    imageId={operationImageId}
                    disabled={busy}
                    onCancel={cancelOperation}
                    retryDisabled={
                      working ||
                      alive ||
                      !toolsReady ||
                      !editable ||
                      !snapshot.qualified ||
                      !snapshot.acceleration.available
                    }
                    onRetry={() =>
                      void perform(() =>
                        review(
                          [operationImageId],
                          devices.some(
                            (device) => device.image === operationImageId,
                          ),
                        ),
                      )
                    }
                  />
                </div>
              )}
              {installedImages.some((pkg) => pkg.id !== operationImageId) && (
                <h3 className="android-versions-heading">Installed</h3>
              )}
              {installedImages
                .filter((pkg) => pkg.id !== operationImageId)
                .sort(compareImages)
                .map((pkg) => {
                  const users = devices.filter(
                    (device) => device.image === pkg.id,
                  );
                  return (
                    <div className="android-row" key={pkg.id}>
                      <div>
                        <strong>{imageTitle(pkg)}</strong>
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
              <h3 className="android-versions-heading">
                Available to download
              </h3>
              <p className="settings-help">
                Google Play includes the Play Store. Google APIs provides Google
                services for testing. AOSP is a minimal system without Google
                services.
              </p>
              <div className="android-actions">
                <button
                  className="button"
                  disabled={busy}
                  onClick={() => void perform(() => loadCatalog(!!catalog))}
                >
                  {busy
                    ? "Loading…"
                    : catalog
                      ? "Refresh catalog"
                      : "Load Android versions"}
                </button>
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
              {catalog && (
                <div className="android-image-list">
                  {images.map((pkg) => (
                    <div
                      className="android-row"
                      key={`${pkg.id}:${pkg.revision}`}
                      data-android-package={pkg.id}
                    >
                      <div>
                        <strong title={imageLabel(pkg)}>
                          {imageTitle(pkg)}
                        </strong>
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
                        {installed[pkg.id] ? "Update image" : "Download image"}
                      </button>
                    </div>
                  ))}
                  {!images.length && (
                    <p className="settings-help">
                      No more Android versions match these filters.
                    </p>
                  )}
                </div>
              )}
            </section>
          </div>
          <div className="android-disclosure">
            <button
              className="android-disclosure-toggle"
              aria-expanded={advancedOpen}
              aria-controls="android-advanced"
              onClick={() => setAdvancedOpen(!advancedOpen)}
            >
              <ChevronRight size={15} />
              <span>
                <strong>Advanced</strong>
                <span>Tools, storage and troubleshooting</span>
              </span>
              {toolsReady && (
                <Check
                  className="android-disclosure-meta"
                  size={14}
                  aria-label="Tools installed"
                />
              )}
            </button>
            <div
              className="android-disclosure-body"
              id="android-advanced"
              hidden={!advancedOpen}
            >
              <button
                className="button"
                disabled={busy || state.loading}
                onClick={() => void perform(reload)}
              >
                <RefreshCw size={14} /> Refresh status
              </button>
              <section className="keybindings-group" aria-label="Environment">
                <h3>Android tools</h3>
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
                      {formatBytes(usage.growthReserveBytes)}. Sparse files can
                      grow up to their configured data capacity.
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
                    Stop running phones before installing, repairing, removing
                    or rolling back components.
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
                                    expectedRevision:
                                      snapshot.packages!.revision,
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
              </section>
              {!!installedImages.length && (
                <section
                  className="keybindings-group"
                  aria-label="Image repairs"
                >
                  <h3>Repair Android versions</h3>
                  <p className="settings-help">
                    Re-download the installed version. Your phones and their
                    data are kept.
                  </p>
                  {installedImages.map((pkg) => (
                    <div className="android-row" key={pkg.id}>
                      <strong>{imageTitle(pkg)}</strong>
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
                    </div>
                  ))}
                </section>
              )}
              <section className="keybindings-group" aria-label="Maintenance">
                <h3>Storage and troubleshooting</h3>
                <p className="settings-help">
                  Free up space or fix an incomplete installation. Your phones
                  and their data are kept.
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
                        The original damaged file is preserved before
                        replacement. Device data is kept.
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
            </div>
          </div>
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
