import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ChevronDown,
  Download,
  Puzzle,
  RefreshCw,
  Search,
  ShieldCheck,
  Trash2,
  X,
} from "../icons";
import { api, errorMessage } from "../api";
import { useThemes } from "../ThemeProvider";
import Select from "../Select";
import { IconButton, Modal } from "../ui";
import { usePlugins } from "./PluginsProvider";
import type { PluginEntry } from "./host";
interface Finished {
  token: string;
  approved: boolean;
  error?: string | null;
}
const categories = [
  { value: "views", label: "Views" },
  { value: "commands", label: "Commands" },
  { value: "fills", label: "Interface" },
] as const;
export default function PluginsPage() {
  const plugins = usePlugins(),
    themes = useThemes();
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [trust, setTrust] = useState<PluginEntry | null>(null),
    [fallback, setFallback] = useState<PluginEntry | null>(null),
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
          "The operation was canceled. The plugin and its views remain available.",
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
  return (
    <main className="keybindings-page plugins-page catalog-page">
      <div className="catalog-content">
        <header className="settings-page-heading">
          <div>
            <h1>Plugins</h1>
            <p>Customize your workspace with local plugins.</p>
          </div>
          <div className="catalog-heading-actions">
            <IconButton
              title="Refresh plugins"
              disabled={busy || !!pending}
              onClick={() => void plugins.reload()}
            >
              <RefreshCw size={16} />
            </IconButton>
            <button
              className="button button-primary"
              disabled={busy || !!pending}
              onClick={() =>
                void run(async () => {
                  const path = await open({
                    directory: true,
                    multiple: false,
                    title: "Import plugin package",
                  });
                  if (typeof path === "string") {
                    await api("import_plugin", { path });
                    clearFilters();
                  }
                })
              }
            >
              <Download size={15} aria-hidden="true" />
              Import plugin
            </button>
          </div>
        </header>
        {plugins.catalog.safeMode && (
          <p className="catalog-notice" role="status">
            Safe startup is active. Third-party code is skipped. Restart
            normally after disabling the faulty plugin.
          </p>
        )}
        {(error || plugins.error) && (
          <p className="catalog-notice text-error" role="alert">
            {error || plugins.error}
          </p>
        )}
        {plugins.error && (
          <p className="settings-help">
            Recovery: close Lomi, back up plugins/installed.json by renaming it
            to installed.backup.json in the application data folder, then
            restart with --safe-mode and reimport your packages. Keep package
            folders and the session file.
          </p>
        )}
        {pending && (
          <p className="catalog-notice" role="status">
            Resolve unsaved plugin views in the workspace window to finish.
          </p>
        )}
        <div className="catalog-toolbar">
          <div className="catalog-search">
            <Search size={16} aria-hidden="true" />
            <input
              ref={searchInput}
              type="search"
              aria-label="Search plugins"
              placeholder="Search plugins…"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
            {search && (
              <IconButton
                title="Clear search"
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
            aria-label="Plugin status"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: "All plugins" },
              { value: "enabled", label: "Enabled" },
              { value: "disabled", label: "Disabled" },
              { value: "attention", label: "Needs attention" },
            ]}
          />
        </div>
        <div className="catalog-filter-row">
          <div
            className="catalog-categories"
            role="group"
            aria-label="Plugin categories"
          >
            {[{ value: "all", label: "All types" }, ...categories].map(
              ({ value, label }) => (
                <button
                  key={value}
                  aria-pressed={category === value}
                  onClick={() => setCategory(value)}
                >
                  {label}
                </button>
              ),
            )}
          </div>
          <span className="catalog-count" role="status">
            {entries.length === installed.length
              ? `${entries.length} installed`
              : `${entries.length} of ${installed.length}`}
          </span>
        </div>
        {entries.length === 0 ? (
          <div className="catalog-empty">
            {installed.length ? (
              <Search size={30} aria-hidden="true" />
            ) : (
              <Puzzle size={30} aria-hidden="true" />
            )}
            <h2>
              {installed.length
                ? "No matching plugins"
                : "No plugins installed."}
            </h2>
            <p>
              {installed.length
                ? "Try a different search or reset your filters."
                : "Import a local package to get started."}
            </p>
            {(search || filter !== "all" || category !== "all") && (
              <button className="button" onClick={clearFilters}>
                Clear filters
              </button>
            )}
          </div>
        ) : (
          <div className="catalog-list">
            {entries.map((entry) => {
              const name = entry.manifest?.name ?? entry.id;
              const problem = entry.error || entry.status?.error;
              const state = problem
                ? "Needs attention"
                : entry.restartRequired
                  ? "Restart required"
                  : entry.enabled && plugins.catalog.safeMode
                    ? "Paused in safe startup"
                    : entry.enabled
                      ? "Enabled"
                      : "Disabled";
              return (
                <article
                  className="catalog-card"
                  key={entry.id}
                  aria-label={name}
                >
                  <header className="catalog-card-heading">
                    <span className="theme-symbol" aria-hidden="true">
                      <Puzzle size={19} />
                    </span>
                    <div className="catalog-card-title">
                      <h2>{name}</h2>
                      <span>
                        {entry.manifest?.version
                          ? `v${entry.manifest.version}`
                          : "Unknown version"}
                      </span>
                    </div>
                    {entry.manifest?.entry && (
                      <input
                        type="checkbox"
                        role="switch"
                        className="settings-switch"
                        aria-label={`Enable ${name}`}
                        checked={entry.enabled}
                        disabled={busy || !!pending || !!entry.error}
                        onChange={() =>
                          entry.enabled
                            ? void run(() => remove(entry, false))
                            : setTrust(entry)
                        }
                      />
                    )}
                  </header>
                  {entry.manifest?.description && (
                    <p className="catalog-description">
                      {entry.manifest.description}
                    </p>
                  )}
                  <div className="catalog-card-meta">
                    <div className="catalog-tags">
                      {categories
                        .filter(
                          ({ value }) =>
                            entry.manifest?.contributes?.[value]?.length,
                        )
                        .map(({ value, label }) => (
                          <span key={value}>{label}</span>
                        ))}
                    </div>
                    <span
                      className={`plugin-state${problem ? " text-error" : ""}`}
                      data-enabled={
                        entry.enabled && !plugins.catalog.safeMode && !problem
                      }
                    >
                      {state}
                    </span>
                  </div>
                  {(entry.error || entry.status?.error) && (
                    <p className="text-error" role="alert">
                      {entry.error || entry.status?.error}
                    </p>
                  )}
                  {entry.restartRequired && (
                    <p className="plugin-restart" role="status">
                      Replacement code requires a restart. Running terminals
                      will end after the close checks succeed.{" "}
                      <button
                        className="button"
                        disabled={busy || !!pending}
                        onClick={() =>
                          void run(() => api("request_plugin_restart"))
                        }
                      >
                        Restart Lomi
                      </button>
                    </p>
                  )}
                  <footer className="catalog-card-footer">
                    <details className="plugin-details">
                      <summary>
                        Details <ChevronDown size={13} aria-hidden="true" />
                      </summary>
                      <dl>
                        <dt>Plugin ID</dt>
                        <dd>{entry.id}</dd>
                        <dt>Source</dt>
                        <dd>{entry.source}</dd>
                        {entry.manifest?.entry && (
                          <>
                            <dt>Trust</dt>
                            <dd>
                              {entry.trustedRevision === entry.revision
                                ? "Trusted revision"
                                : "Not trusted"}
                            </dd>
                          </>
                        )}
                        {entry.status && (
                          <>
                            <dt>Runtime</dt>
                            <dd>{entry.status.phase}</dd>
                          </>
                        )}
                      </dl>
                    </details>
                    <button
                      className="plugin-uninstall"
                      disabled={busy || !!pending}
                      onClick={() =>
                        entry.themeIds?.includes(
                          themes.preferences.active ?? "",
                        )
                          ? setFallback(entry)
                          : void run(() => remove(entry, true))
                      }
                    >
                      <Trash2 size={13} aria-hidden="true" />
                      Uninstall
                    </button>
                  </footer>
                </article>
              );
            })}
          </div>
        )}
        <p className="plugins-trust-note settings-help">
          <ShieldCheck size={15} aria-hidden="true" />
          Only enable plugins you trust. Importing a package does not run its
          code.
        </p>
      </div>
      {trust && (
        <Modal
          title={`Enable ${trust.manifest?.name ?? trust.id}?`}
          onClose={() => {
            if (!busy) setTrust(null);
          }}
        >
          <div className="dialog-form">
            <p>
              {trust.id} · {trust.manifest?.version}
            </p>
            <p>Source: {trust.source}</p>
            <p>
              Enabling this plugin executes trusted code with application
              access, including files and terminals available to Lomi. It is not
              sandboxed. Approve only code you trust.
            </p>
            <p className="settings-help">
              Approval applies to this installed revision. Changed code requires
              a new review.
            </p>
            <div className="dialog-actions">
              <button
                className="button"
                disabled={busy}
                onClick={() => setTrust(null)}
              >
                Cancel
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
                Trust and enable
              </button>
            </div>
          </div>
        </Modal>
      )}
      {fallback && (
        <Modal
          title="Replace the active theme?"
          onClose={() => {
            if (!busy) setFallback(null);
          }}
        >
          <div className="dialog-form">
            <p>
              This package supplies the selected theme. Switch to DeepMono
              before uninstalling it. The package stays installed if the switch
              or its close checks fail.
            </p>
            <div className="dialog-actions">
              <button
                className="button"
                disabled={busy}
                onClick={() => setFallback(null)}
              >
                Cancel
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
                Switch to DeepMono and uninstall
              </button>
            </div>
          </div>
        </Modal>
      )}
    </main>
  );
}
