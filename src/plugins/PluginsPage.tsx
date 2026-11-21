import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { Download, Info, Puzzle, Search, Trash2, X } from "../icons";
import "../settings-plugins.css";
import { api, errorMessage } from "../api";
import { useThemes } from "../ThemeProvider";
import Select from "../Select";
import { IconButton, Modal } from "../ui";
import { SettingsNotice, SettingsPage, SettingsSection } from "../settings-ui";
import { usePlugins } from "./PluginsProvider";
import type { PluginEntry } from "./host";
import { msg, useT } from "../i18n";
interface Finished {
  token: string;
  approved: boolean;
  error?: string | null;
}
const categories = [
  { value: "views", label: msg("Views") },
  { value: "commands", label: msg("Commands") },
  { value: "fills", label: msg("Interface") },
] as const;
export default function PluginsPage() {
  const { t } = useT();
  const plugins = usePlugins(),
    themes = useThemes();
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [trust, setTrust] = useState<PluginEntry | null>(null),
    [fallback, setFallback] = useState<PluginEntry | null>(null),
    [details, setDetails] = useState<PluginEntry | null>(null),
    [pending, setPending] = useState<string | null>(null);
  const [search, setSearch] = useState(""),
    [filter, setFilter] = useState("all"),
    [category, setCategory] = useState("all");
  const searchInput = useRef<HTMLInputElement>(null);
  const working = useRef(false),
    pendingToken = useRef<string | null>(null),
    finished = useRef(new Map<string, Finished>()),
    listener = useRef<Promise<UnlistenFn> | null>(null);
  const complete = (result: Finished) => {
    pendingToken.current = null;
    setPending(null);
    if (!result.approved)
      setError(
        result.error ??
          t(
            "The operation was canceled. The plugin and its views remain available.",
          ),
      );
    void plugins.reload();
  };
  useEffect(() => {
    const stop = listen<Finished>(
      "plugin-operation-finished",
      ({ payload }) => {
        if (payload.token === pendingToken.current) complete(payload);
        else {
          finished.current.set(payload.token, payload);
          if (finished.current.size > 128)
            finished.current.delete(finished.current.keys().next().value!);
        }
      },
    );
    listener.current = stop;
    void stop.catch((error) => setError(errorMessage(error)));
    return () => {
      void stop.then((stop) => stop()).catch(() => {});
    };
  }, [plugins.reload]);
  const run = async (action: () => Promise<unknown>) => {
    if (working.current || pendingToken.current) return;
    working.current = true;
    setBusy(true);
    setError("");
    try {
      await action();
      await plugins.reload();
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      working.current = false;
      setBusy(false);
    }
  };
  const remove = async (entry: PluginEntry, uninstall: boolean) => {
    await listener.current;
    const token = await api<string>("request_plugin_removal", {
      id: entry.id,
      expected: entry.revision,
      uninstall,
    });
    const result = finished.current.get(token);
    if (result) {
      finished.current.delete(token);
      complete(result);
    } else {
      pendingToken.current = token;
      setPending(token);
    }
  };
  const query = search.trim().toLowerCase();
  const installed = plugins.catalog.entries.filter(
    (entry) => !entry.manifest || !!entry.manifest.entry,
  );
  const entries = installed
    .filter((entry) => {
      const matchesState =
        filter === "all" ||
        (filter === "enabled" && entry.enabled) ||
        (filter === "disabled" && !!entry.manifest?.entry && !entry.enabled) ||
        (filter === "attention" &&
          (entry.error || entry.status?.error || entry.restartRequired));
      return (
        matchesState &&
        (category === "all" ||
          categories.some(
            ({ value }) =>
              value === category &&
              entry.manifest?.contributes?.[value]?.length,
          )) &&
        `${entry.manifest?.name ?? ""} ${entry.id} ${entry.manifest?.description ?? ""}`
          .toLowerCase()
          .includes(query)
      );
    })
    .sort((a, b) =>
      (a.manifest?.name ?? a.id).localeCompare(b.manifest?.name ?? b.id),
    );
  const clearFilters = () => {
    setSearch("");
    setFilter("all");
    setCategory("all");
    searchInput.current?.focus();
  };
  const importButton = (
    <button
      className="button"
      disabled={busy || !!pending}
      onClick={() =>
        void run(async () => {
          const path = await open({
            directory: true,
            multiple: false,
            title: t("Import plugin package"),
          });
          if (typeof path === "string") {
            await api("import_plugin", { path });
            clearFilters();
          }
        })
      }
    >
      <Download size={15} aria-hidden="true" />
      {t("Import plugin")}
    </button>
  );
  return (
    <SettingsPage
      title={t("Plugins")}
      className="plugins-page catalog-page"
      contentClassName="catalog-content"
    >
      {plugins.catalog.safeMode && (
        <SettingsNotice tone="warning" role="status">
          {t(
            "Safe startup: third-party code is skipped. Disable the faulty plugin, then restart normally.",
          )}
        </SettingsNotice>
      )}
      {(error || plugins.error) && (
        <SettingsNotice tone="error">
          <p>{error || plugins.error}</p>
          {plugins.error && (
            <p className="settings-help">
              {t(
                "To recover, close Lomi, rename plugins/installed.json to installed.backup.json, restart with --safe-mode and reimport your packages.",
              )}
            </p>
          )}
        </SettingsNotice>
      )}
      {pending && (
        <SettingsNotice role="status">
          {t("Resolve unsaved plugin views in the workspace window to finish.")}
        </SettingsNotice>
      )}
      <SettingsSection
        title={t("Installed plugins")}
        grouped={false}
        actions={importButton}
      >
        <div className="plugins-toolbar">
          <div className="catalog-search">
            <Search size={16} aria-hidden="true" />
            <input
              ref={searchInput}
              type="search"
              aria-label={t("Search plugins")}
              placeholder={t("Search plugins…")}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
            {search && (
              <IconButton
                title={t("Clear search")}
                onClick={() => {
                  setSearch("");
                  searchInput.current?.focus();
                }}
              >
                <X size={14} />
              </IconButton>
            )}
          </div>
          <Select
            aria-label={t("Plugin categories")}
            value={category}
            onChange={setCategory}
            options={[
              { value: "all", label: t("All types") },
              ...categories.map(({ value, label }) => ({
                value,
                label: t(label),
              })),
            ]}
          />
          <Select
            aria-label={t("Plugin status")}
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: t("All plugins") },
              { value: "enabled", label: t("Enabled") },
              { value: "disabled", label: t("Disabled") },
              { value: "attention", label: t("Needs attention") },
            ]}
          />
        </div>
        <div className="settings-group plugin-list">
          {entries.length === 0 ? (
            <div className="plugins-empty">
              <span className="plugins-empty-icon" aria-hidden="true">
                {installed.length ? <Search size={20} /> : <Puzzle size={20} />}
              </span>
              <h3>
                {installed.length
                  ? t("No matching plugins")
                  : t("No plugins yet")}
              </h3>
              <p>
                {installed.length
                  ? t("Try a different search or reset your filters.")
                  : t("Import a local plugin package to get started.")}
              </p>
              {installed.length ? (
                <button className="text-button" onClick={clearFilters}>
                  {t("Clear filters")}
                </button>
              ) : (
                importButton
              )}
            </div>
          ) : (
            entries.map((entry) => {
              const name = entry.manifest?.name ?? entry.id;
              const problem = entry.error || entry.status?.error;
              const state = problem
                ? t("Needs attention")
                : entry.restartRequired
                  ? t("Restart required")
                  : entry.enabled && plugins.catalog.safeMode
                    ? t("Paused in safe startup")
                    : entry.enabled
                      ? t("Enabled")
                      : t("Disabled");
              const types = categories
                .filter(
                  ({ value }) => entry.manifest?.contributes?.[value]?.length,
                )
                .map(({ label }) => t(label))
                .join(" · ");
              return (
                <article
                  className="setting-row plugin-row"
                  key={entry.id}
                  aria-label={name}
                >
                  <span className="plugin-icon" aria-hidden="true">
                    <Puzzle size={18} />
                  </span>
                  <div className="plugin-main">
                    <div className="plugin-title">
                      <h3>{name}</h3>
                      <span>
                        {entry.manifest?.version
                          ? `v${entry.manifest.version}`
                          : t("Unknown version")}
                      </span>
                      {types && <span className="plugin-type">{types}</span>}
                    </div>
                    {entry.manifest?.description && (
                      <p className="plugin-description">
                        {entry.manifest.description}
                      </p>
                    )}
                  </div>
                  <span
                    className={`plugin-state${problem ? " text-error" : ""}`}
                    data-enabled={
                      entry.enabled && !plugins.catalog.safeMode && !problem
                    }
                  >
                    {state}
                  </span>
                  <div className="plugin-actions">
                    <IconButton
                      title={t("Details for {name}", { name })}
                      onClick={() => setDetails(entry)}
                    >
                      <Info size={15} />
                    </IconButton>
                    <IconButton
                      title={t("Uninstall {name}", { name })}
                      className="icon-button plugin-uninstall"
                      disabled={busy || !!pending}
                      onClick={() =>
                        entry.themeIds?.includes(
                          themes.preferences.active ?? "",
                        )
                          ? setFallback(entry)
                          : void run(() => remove(entry, true))
                      }
                    >
                      <Trash2 size={15} />
                    </IconButton>
                  </div>
                  {entry.manifest?.entry && (
                    <input
                      type="checkbox"
                      role="switch"
                      className="settings-switch"
                      aria-label={t("Enable {name}", { name })}
                      checked={entry.enabled}
                      disabled={busy || !!pending || !!entry.error}
                      onChange={() =>
                        entry.enabled
                          ? void run(() => remove(entry, false))
                          : setTrust(entry)
                      }
                    />
                  )}
                  {problem && (
                    <p className="plugin-extra text-error" role="alert">
                      {problem}
                    </p>
                  )}
                  {entry.restartRequired && (
                    <SettingsNotice
                      tone="warning"
                      role="status"
                      className="plugin-extra plugin-restart"
                      action={
                        <button
                          className="button"
                          disabled={busy || !!pending}
                          onClick={() =>
                            void run(() => api("request_plugin_restart"))
                          }
                        >
                          {t("Restart Lomi")}
                        </button>
                      }
                    >
                      {t(
                        "Restart to load the new code. Running terminals will end.",
                      )}
                    </SettingsNotice>
                  )}
                </article>
              );
            })
          )}
        </div>
      </SettingsSection>
      {details && (
        <Modal
          protectTheme
          title={details.manifest?.name ?? details.id}
          onClose={() => setDetails(null)}
        >
          <div className="dialog-form dialog-layout">
            <dl className="plugin-details">
              <dt>{t("Plugin ID")}</dt>
              <dd>{details.id}</dd>
              <dt>{t("Source")}</dt>
              <dd>{details.source}</dd>
              {details.manifest?.entry && (
                <>
                  <dt>{t("Trust")}</dt>
                  <dd>
                    {details.trustedRevision === details.revision
                      ? t("Trusted revision")
                      : t("Not trusted")}
                  </dd>
                </>
              )}
              {details.status && (
                <>
                  <dt>{t("Runtime")}</dt>
                  <dd>{details.status.phase}</dd>
                </>
              )}
            </dl>
            <div className="dialog-actions">
              <button className="button" onClick={() => setDetails(null)}>
                {t("Close")}
              </button>
            </div>
          </div>
        </Modal>
      )}
      {trust && (
        <Modal
          protectTheme
          title={t("Enable {name}?", {
            name: trust.manifest?.name ?? trust.id,
          })}
          tone="warning"
          closeDisabled={busy}
          onClose={() => {
            if (!busy) setTrust(null);
          }}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p className="settings-help">
                {trust.id} · {trust.manifest?.version} · {trust.source}
              </p>
              <p>
                {t(
                  "This plugin runs unsandboxed with the same access as Lomi, including your files and terminals. Enable only code you trust.",
                )}
              </p>
              <p className="settings-help">
                {t("Approval covers this version only.")}
              </p>
            </div>
            <div className="dialog-actions">
              <button
                className="button"
                disabled={busy}
                onClick={() => setTrust(null)}
              >
                {t("Cancel")}
              </button>
              <button
                className="button button-primary"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await api("enable_plugin", {
                      id: trust.id,
                      expected: trust.revision,
                    });
                    setTrust(null);
                  })
                }
              >
                {t("Trust and enable")}
              </button>
            </div>
          </div>
        </Modal>
      )}
      {fallback && (
        <Modal
          protectTheme
          title={t("Replace the active theme?")}
          tone="warning"
          closeDisabled={busy}
          onClose={() => {
            if (!busy) setFallback(null);
          }}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p>
                {t(
                  "This package provides the active theme. Lomi switches to its default theme first.",
                )}
              </p>
            </div>
            <div className="dialog-actions">
              <button
                className="button"
                disabled={busy}
                onClick={() => setFallback(null)}
              >
                {t("Cancel")}
              </button>
              <button
                className="button button-primary"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await themes.select(null);
                    await remove(fallback, true);
                    setFallback(null);
                  })
                }
              >
                {t("Switch to Lomi and uninstall")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </SettingsPage>
  );
}
