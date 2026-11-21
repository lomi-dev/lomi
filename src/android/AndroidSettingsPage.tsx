import { useEffect, useRef, useState } from "react";
import { api, errorMessage, native } from "../api";
import { useT } from "../i18n";
import { listen } from "@tauri-apps/api/event";
import { IconButton, Modal } from "../ui";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "../settings-ui";
import ContextMenu from "../ContextMenu";
import {
  RefreshCw,
  Plus,
  Download,
  Layers,
  Check,
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
  const { t } = useT();
  const [text, setText] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <Modal
      protectTheme
      className="android-dialog"
      title={value.title}
      tone="warning"
      closeDisabled={busy}
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <form
        className="dialog-form dialog-layout"
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
        <div className="dialog-content android-device-form">
          <p>{value.description}</p>
          {value.name && (
            <label>
              {t("Type “{name}” to confirm", { name: value.name })}
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
        </div>
        <div className="dialog-actions">
          <button
            className="button"
            type="button"
            disabled={busy}
            onClick={onClose}
          >
            {t("Cancel")}
          </button>
          <button
            className="button button-danger"
            disabled={busy || (!!value.name && text !== value.name)}
          >
            {busy ? t("Applying…") : value.title}
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
  const { t } = useT();
  const [accepted, setAccepted] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return (
    <Modal
      protectTheme
      className="android-dialog"
      title={t("Review Android installation")}
      wide
      closeDisabled={busy}
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <form
        className="dialog-form dialog-layout"
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
        <div className="dialog-content android-device-form">
          <p>
            {t(
              "{size} to download. Extraction and virtual phone data need additional space.",
              { size: formatBytes(plan.downloadBytes) },
            )}
          </p>
          <div className="android-install-details">
            <h3>{t("Installation details")}</h3>
            <p className="settings-help">
              {t("Managed SDK:")} <code>{snapshot.sdkPath}</code>.{" "}
              {t(
                "Existing SDKs, shell settings and system Java remain separate.",
              )}
            </p>
            <ul className="android-install-list">
              {plan.bootstrap && (
                <>
                  <li>
                    {t("Android CLI {version}", {
                      version: plan.bootstrap.cli.version,
                    })}
                  </li>
                  <li>
                    {t("Private Java runtime {version}", {
                      version: plan.bootstrap.java.version,
                    })}
                  </li>
                </>
              )}
              {plan.packages.map((pkg) => (
                <li key={pkg.id}>
                  {pkg.name} · {pkg.revision} · {formatBytes(pkg.size)}
                </li>
              ))}
            </ul>
          </div>
          <p>{t("Review and accept the provider terms to continue.")}</p>
          {plan.licenses.map((license) => (
            <section
              className="android-license"
              key={license.digest}
              aria-label={license.id}
            >
              <h3>{license.id}</h3>
              <pre
                tabIndex={0}
                aria-label={t("{id} terms", { id: license.id })}
              >
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
                {t("I accept {id}", { id: license.id })}
              </label>
            </section>
          ))}
          {error && (
            <p className="keybindings-error" role="alert">
              {error}
            </p>
          )}
        </div>
        <div className="dialog-actions">
          <button
            className="button"
            type="button"
            disabled={busy}
            onClick={onClose}
          >
            {t("Cancel")}
          </button>
          <button
            className="button button-primary"
            disabled={busy || accepted.length !== plan.licenses.length}
          >
            {busy ? t("Starting…") : t("Accept and install")}
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
  const { t } = useT();
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
      aria-label={
        imageId ? t("{title} installation", { title }) : t("Android operation")
      }
      data-android-package={imageId}
    >
      <div className="android-row">
        <div>
          <strong>{title}</strong>
          <p className="settings-help" role="status">
            {stopping
              ? t("Cancelling…")
              : active
                ? imageId
                  ? progress.stage
                  : t("This operation continues when Settings closes.")
                : progress.phase === "succeeded"
                  ? t("Completed")
                  : progress.phase === "cancelled"
                    ? t("Cancelled safely")
                    : t(
                        "Could not finish. Try again or open Advanced for repair options.",
                      )}
          </p>
        </div>
        {active ? (
          <button
            className="button"
            aria-label={imageId && !stopping ? t("Cancel download") : undefined}
            disabled={disabled || stopping}
            onClick={() => {
              setCancelling(true);
              setCancelError("");
              void onCancel()
                .catch((error) => setCancelError(errorMessage(error)))
                .finally(() => setCancelling(false));
            }}
          >
            {stopping
              ? t("Cancelling…")
              : imageId
                ? t("Cancel")
                : t("Cancel operation")}
          </button>
        ) : (
          onRetry && (
            <button
              className="button"
              disabled={retryDisabled}
              onClick={onRetry}
            >
              {progress.phase === "cancelled"
                ? t("Download again")
                : t("Try again")}
            </button>
          )
        )}
      </div>
      {active && (
        <>
          <progress
            aria-label={
              imageId
                ? t("Download progress for {title}", { title })
                : progress.stage
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
  const { t } = useT();
  const state = useAndroid();
  const snapshot = state.snapshot;
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [usage, setUsage] = useState<Usage | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const locked = useRef(false);
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
    if (operationImageId)
      imageDownload.current?.scrollIntoView({ block: "center" });
  }, [operationImageId, progress?.operationId]);
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
  // No phones yet means first-run setup: show only the setup card (and step 2's list).
  const setupMode = !devices.length;
  const profilesMissing = setupStep === 3 && !snapshot?.profiles.length;
  const full = !setupMode || profilesMissing;
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
      title: t("Remove component"),
      description: t(
        "Remove {id} and its retained previous version from the managed SDK. Device data is preserved.",
        { id },
      ),
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
      title: type === "wipe" ? t("Wipe device data") : t("Delete device"),
      name: device.name,
      description:
        type === "wipe"
          ? t(
              "Erase all apps, accounts and data from {name}. The device ID stays the same. This cannot be undone.",
              { name: device.name },
            )
          : t(
              "Permanently delete {name} and all its apps and data. Open panels will show a missing device. This cannot be undone.",
              { name: device.name },
            ),
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
    <SettingsPage
      title="Android"
      className="android-settings-page"
      busy={busy}
      status={!snapshot && state.loading ? t("Loading…") : notice}
    >
      {(error || state.error) && (
        <SettingsNotice
          tone="error"
          action={
            <button
              className="text-button"
              onClick={() => void perform(reload)}
            >
              {t("Retry")}
            </button>
          }
        >
          {error || state.error}
        </SettingsNotice>
      )}
      {!snapshot ? (
        !state.loading &&
        !(error || state.error) && (
          <SettingsNotice tone="error">
            {t("Android settings couldn’t be loaded.")}
          </SettingsNotice>
        )
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
            <SettingsNotice tone="warning" role="status">
              {t(
                "Android setup isn’t available on this computer yet. Existing phones can still be stopped and recovered.",
              )}
            </SettingsNotice>
          )}
          {snapshot.acceleration.action && (
            <SettingsNotice tone="warning" role="alert">
              {snapshot.acceleration.action}
            </SettingsNotice>
          )}
          {Object.entries(snapshot.errors).map(([name, message]) => (
            <SettingsNotice key={name} tone="error">
              {name}: {message}
            </SettingsNotice>
          ))}
          {!!snapshot.recovery.length && (
            <SettingsNotice tone="warning">
              {t("Some Android files need recovery.")}
            </SettingsNotice>
          )}
          {setupMode && (
            <SettingsSection
              title={t("Get started")}
              className="android-setup"
              description={t(
                "Run Android apps on virtual phones inside your workspace.",
              )}
            >
              {[
                {
                  label: t("Install Android tools"),
                  help: t(
                    "A one-time download of the tools that run virtual phones.",
                  ),
                  action: t("Install"),
                  icon: <Download size={14} />,
                },
                {
                  label: t("Choose Android version"),
                  help: t("Pick Google Play if you need the Play Store."),
                  action: t("Choose version"),
                  icon: <Layers size={14} />,
                },
                {
                  label: t("Create a phone"),
                  help: t(
                    "Pick a phone model and a name. Apps and data stay on the phone.",
                  ),
                  action: t("Create phone"),
                  icon: <Plus size={14} />,
                },
              ].map((step, index) => {
                const number = index + 1;
                const state =
                  number < setupStep
                    ? "done"
                    : number === setupStep
                      ? "current"
                      : "next";
                return (
                  <SettingRow
                    key={number}
                    className={`android-setup-step is-${state}`}
                    label={step.label}
                    description={
                      state === "current" && profilesMissing
                        ? t(
                            "Phone models couldn’t be loaded. Use Repair tools below.",
                          )
                        : step.help
                    }
                  >
                    {state === "done" ? (
                      <span className="android-setup-done">
                        <Check size={14} aria-hidden="true" />
                        {t("Done")}
                      </span>
                    ) : state === "current" && !(setupStep === 2 && catalog) ? (
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
                          else if (setupStep === 2)
                            void perform(() => loadCatalog());
                          else setForm("new");
                        }}
                      >
                        {step.icon}
                        {step.action}
                      </button>
                    ) : null}
                  </SettingRow>
                );
              })}
            </SettingsSection>
          )}
          {!!devices.length && (
            <SettingsSection
              title={t("Your phones")}
              count={devices.length}
              className="android-phones"
              description={
                setup
                  ? t(
                      "Open a phone to return to the workspace where you started.",
                    )
                  : undefined
              }
              actions={
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
                  <Plus size={14} /> {t("Create device")}
                </button>
              }
            >
              <div className="android-phone-list">
                {devices.map((device) => {
                  const status = snapshot.statuses.find(
                    (item) => item.deviceId === device.id,
                  );
                  const active =
                    status?.processAlive || status?.phase === "starting";
                  const phase = status?.phase ?? "stopped";
                  const statusLabel = t(deviceStatusLabels[phase]);
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
                              <span className="android-default">
                                {t("Default")}
                              </span>
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
                                ? t("Retry stop")
                                : t("Stop")}
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
                            <Play size={13} /> {t("Open")}
                          </button>
                          <IconButton
                            title={t("Options for {name}", {
                              name: device.name,
                            })}
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
                          label={t("Options for {name}", { name: device.name })}
                          onClose={closeMenu}
                          actions={[
                            {
                              label: t("Configure"),
                              disabled: working || !editable,
                              run: () => setForm(device),
                            },
                            ...(setup
                              ? [
                                  {
                                    label: t("Open in new tab"),
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
                              label: t("Use as default"),
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
                              label: t("Cold boot"),
                              disabled:
                                working || !editable || !snapshot.qualified,
                              run: () => void perform(() => open(device, true)),
                            },
                            ...(active && status?.phase === "failed"
                              ? [
                                  {
                                    label: t("Force stop"),
                                    danger: true,
                                    disabled: busy,
                                    run: () =>
                                      setConfirmation({
                                        title: t("Force stop"),
                                        name: device.name,
                                        description: t(
                                          "Force {name} to stop. Unsaved Android data can be lost.",
                                          { name: device.name },
                                        ),
                                        action: () => stop(device, true),
                                      }),
                                  },
                                ]
                              : []),
                            null,
                            {
                              label: t("Wipe data…"),
                              danger: true,
                              disabled:
                                working ||
                                active ||
                                !editable ||
                                !snapshot.qualified,
                              run: () => destructive(device, "wipe"),
                            },
                            {
                              label: t("Delete device…"),
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
            </SettingsSection>
          )}
          {alive && (
            <p className="settings-help android-install-hint">
              {t(
                "Stop running phones before changing Android tools or versions.",
              )}
            </p>
          )}
          {(full || (setupStep === 2 && (catalog || operationImageId))) && (
            <SettingsSection
              title={t("Android versions")}
              actions={
                operationImageId && downloading ? (
                  <span className="android-download-indicator">
                    {progress?.phase === "cancelling"
                      ? t("Cancelling…")
                      : progress && progress.total > 0
                        ? t("Downloading…")
                        : t("Installing…")}
                  </span>
                ) : (
                  !!installedImages.length && (
                    <span>
                      {t("{count} installed", {
                        count: installedImages.length,
                      })}
                    </span>
                  )
                )
              }
            >
              <div id="android-versions">
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
                  <h3 className="android-versions-heading">{t("Installed")}</h3>
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
                              ? t("Used by {devices}", {
                                  devices: users
                                    .map((device) => device.name)
                                    .join(", "),
                                })
                              : t("Installed · not used by a device")}
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
                            {t("Remove image")}
                          </button>
                          <button
                            className="button"
                            disabled={
                              working ||
                              alive ||
                              !editable ||
                              !snapshot.qualified
                            }
                            onClick={() =>
                              void perform(() => review([pkg.id], true))
                            }
                          >
                            {t("Repair image")}
                          </button>
                        </div>
                      </div>
                    );
                  })}
                {full && (
                  <>
                    <h3 className="android-versions-heading">
                      {t("Available to download")}
                    </h3>
                    <div className="android-actions">
                      <button
                        className="button"
                        disabled={busy}
                        onClick={() =>
                          void perform(() => loadCatalog(!!catalog))
                        }
                      >
                        {busy
                          ? t("Loading…")
                          : catalog
                            ? t("Refresh catalog")
                            : t("Load Android versions")}
                      </button>
                    </div>
                  </>
                )}
                {catalog && (
                  <div className="android-image-filters">
                    <label>
                      {t("Android version")}
                      <Select
                        aria-label={t("Filter Android version")}
                        value={version}
                        onChange={setVersion}
                        options={[
                          { value: "recent", label: t("Recent versions") },
                          { value: "all", label: t("All versions") },
                          ...versions.map((api) => ({
                            value: String(api),
                            label: androidVersion(api),
                          })),
                        ]}
                      />
                    </label>
                    <label>
                      {t("Included apps")}
                      <Select
                        aria-label={t("Filter included apps")}
                        value={family}
                        onChange={setFamily}
                        options={[
                          { value: "all", label: t("All variants") },
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
                          <p className="settings-help">
                            {imageDescription(pkg)}
                          </p>
                          <p className="settings-help">
                            {installed[pkg.id]
                              ? t("{size} download · installed r{revision}", {
                                  size: formatBytes(pkg.size),
                                  revision: installed[pkg.id].revision,
                                })
                              : t("{size} download", {
                                  size: formatBytes(pkg.size),
                                })}
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
                          {installed[pkg.id]
                            ? t("Update image")
                            : t("Download image")}
                        </button>
                      </div>
                    ))}
                    {!images.length && (
                      <p className="settings-help">
                        {t("No more Android versions match these filters.")}
                      </p>
                    )}
                  </div>
                )}
              </div>
            </SettingsSection>
          )}
          {full && (
            <SettingsSection title={t("Status")}>
              <SettingRow label={t("Hardware acceleration")}>
                <span
                  title={`${snapshot.host} · ${snapshot.acceleration.backend}`}
                >
                  {snapshot.acceleration.available
                    ? t("Available")
                    : t("System preparation required")}
                </span>
                <button
                  className="button"
                  disabled={busy || state.loading}
                  onClick={() => void perform(reload)}
                >
                  <RefreshCw size={14} /> {t("Refresh status")}
                </button>
              </SettingRow>
              {usage && (
                <SettingRow label={t("Disk space")}>
                  {t("{used} used · {free} free", {
                    used: formatBytes(
                      Object.values(usage.directories).reduce(
                        (total, size) => total + size.allocatedBytes,
                        0,
                      ),
                    ),
                    free: formatBytes(usage.freeBytes),
                  })}
                </SettingRow>
              )}
              <SettingRow label={t("Android tools")}>
                <span
                  title={`${snapshot.sdkPath}\n${t(
                    "SDK manager {cli} · Java {java} · {state}",
                    {
                      cli: snapshot.toolchain.cli.version,
                      java: snapshot.toolchain.java.version,
                      state: snapshot.toolchainReady
                        ? t("Prepared")
                        : t("Not prepared"),
                    },
                  )}`}
                >
                  {toolsReady
                    ? t("Installed")
                    : snapshot.requiredTools.some((id) => installed[id])
                      ? t("Needs repair")
                      : t("Not installed")}
                </span>
                {(!snapshot.requiredTools.every((id) => installed[id]) ||
                  snapshot.toolchainUpdateAvailable) && (
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
                      ? t("Update tools")
                      : t("Install Android tools")}
                  </button>
                )}
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
                  {t("Repair tools")}
                </button>
              </SettingRow>
              {snapshot.requiredTools
                .filter((id) => installed[id])
                .map((id) => (
                  <div className="android-row" key={id}>
                    <div>
                      <strong>{id}</strong>
                      <p className="settings-help">
                        {t("Installed {revision}", {
                          revision: installed[id].revision,
                        })}
                      </p>
                    </div>
                    <div className="android-actions">
                      {snapshot.rollbacks.find((pkg) => pkg.id === id) && (
                        <button
                          className="button"
                          disabled={working || alive}
                          onClick={() =>
                            setConfirmation({
                              title: t("Roll back tool"),
                              description: t(
                                "Restore {id} {revision}. The current version will be kept as the previous copy.",
                                {
                                  id,
                                  revision: snapshot.rollbacks.find(
                                    (pkg) => pkg.id === id,
                                  )!.revision,
                                },
                              ),
                              action: () =>
                                maintain({
                                  type: "rollback",
                                  packageId: id,
                                  expectedRevision: snapshot.packages!.revision,
                                }),
                            })
                          }
                        >
                          {t("Roll back")}
                        </button>
                      )}
                      <button
                        className="button"
                        disabled={working || alive || !editable}
                        onClick={() => remove(id)}
                      >
                        {t("Remove")}
                      </button>
                    </div>
                  </div>
                ))}
            </SettingsSection>
          )}
          {(full || !!snapshot.recovery.length) && (
            <SettingsSection title={t("Troubleshooting")}>
              <p className="settings-help">
                {t("Repairs and cleanup keep your phones and their data.")}
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
                  {t("Repair interrupted operation")}
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
                  {t("Clean temporary files")}
                </button>
                <button
                  className="button"
                  disabled={busy}
                  onClick={() =>
                    void perform(async () => {
                      if (await api<boolean>("android_export_diagnostics"))
                        setNotice(t("Diagnostic logs exported."));
                    })
                  }
                >
                  {t("Export diagnostics")}
                </button>
              </div>
              {snapshot.recovery.map((item) => (
                <div className="android-row" key={item.file}>
                  <div>
                    <strong>{t("Recover {file}", { file: item.file })}</strong>
                    <p className="settings-help">
                      {t("The damaged file is kept.")}{" "}
                      {item.backupRevision !== null
                        ? t("Backup revision {revision} is available.", {
                            revision: item.backupRevision,
                          })
                        : t("No valid backup is available.")}
                    </p>
                  </div>
                  <div className="android-actions">
                    {item.backupRevision !== null && (
                      <button
                        className="button"
                        disabled={working || alive}
                        onClick={() =>
                          setConfirmation({
                            title: t("Restore metadata backup"),
                            description: t(
                              "Restore the previous {file} file. Newer configuration edits may not be in this backup. All AVD files are preserved.",
                              { file: item.file },
                            ),
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
                        {t("Restore backup")}
                      </button>
                    )}
                    {item.file === "preferences" && (
                      <button
                        className="button"
                        disabled={working || alive}
                        onClick={() =>
                          setConfirmation({
                            title: t("Reset Android preferences"),
                            description: t(
                              "Clear the default-device preference and preserve the damaged preferences file for recovery. Devices and their data are unchanged.",
                            ),
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
                        {t("Reset preferences")}
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </SettingsSection>
          )}
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
    </SettingsPage>
  );
}
