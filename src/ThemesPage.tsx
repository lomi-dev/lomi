import type { IconKind } from "./theme/icon-theme";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  Check,
  Copy,
  Download,
  FolderOpen,
  Import,
  Monitor,
  Moon,
  Palette,
  Plus,
  Pencil,
  RefreshCw,
  Search,
  Sun,
  X,
} from "./icons";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { api, errorMessage, native } from "./api";
import { useThemes } from "./ThemeProvider";
import {
  builtinTheme,
  builtinThemes,
  isBuiltinTheme,
  parseThemeText,
} from "./theme/format";
import type { ThemeBundle, ThemeCatalog } from "./theme/format";
import ThemeEditor from "./ThemeEditor";
import Select from "./Select";
import { IconButton } from "./ui";

export default function ThemesPage() {
  const themes = useThemes();
  const [catalog, setCatalog] = useState<ThemeCatalog>({
    directory: "",
    themes: [],
  });
  const [busy, setBusy] = useState(false);
  const [appearanceDraft, setAppearanceDraft] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const [dragging, setDragging] = useState(false);
  const [editing, setEditing] = useState<ThemeBundle | null>(null);
  const [search, setSearch] = useState("");
  const [kind, setKind] = useState<"color" | IconKind>("color");
  const selectedId =
    (kind === "color"
      ? themes.preferences.active
      : kind === "file"
        ? themes.preferences.fileIcons
        : themes.preferences.productIcons) ?? null;
  const [filter, setFilter] = useState("all");
  const [source, setSource] = useState("all");
  const searchInput = useRef<HTMLInputElement>(null);
  const clearFilters = useCallback(() => {
    setSearch("");
    setFilter("all");
    setSource("all");
  }, []);
  const working = useRef(false);
  const mounted = useRef(false);
  const refreshCatalog = useCallback(async () => {
    if (!native) return;
    const next = await api<ThemeCatalog>("list_themes");
    if (mounted.current) setCatalog(next);
  }, []);
  const run = useCallback(
    async (operation: () => Promise<void>, message: string) => {
      if (working.current) return;
      working.current = true;
      setBusy(true);
      setError("");
      setStatus("");
      try {
        await operation();
        if (mounted.current && message) setStatus(message);
      } catch (error) {
        if (mounted.current) setError(errorMessage(error));
      } finally {
        working.current = false;
        if (mounted.current) setBusy(false);
      }
    },
    [],
  );
  const importFolder = useCallback(
    (path: string) =>
      run(async () => {
        await api<string>("import_theme", { path });
        await refreshCatalog();
        clearFilters();
      }, "Theme imported. Select it below to apply it."),
    [run, refreshCatalog, clearFilters],
  );
  useEffect(() => {
    mounted.current = true;
    void refreshCatalog().catch((error) => {
      if (mounted.current) setError(errorMessage(error));
    });
    const focused = () =>
      void refreshCatalog().catch((error) => {
        if (mounted.current) setError(errorMessage(error));
      });
    window.addEventListener("focus", focused);
    const changes = listen("plugins-changed", focused);
    void changes.catch((error) => setError(errorMessage(error)));
    const unlisten = native
      ? getCurrentWebviewWindow().onDragDropEvent(({ payload }) => {
          if (!mounted.current) return;
          setDragging(payload.type === "enter" || payload.type === "over");
          if (payload.type === "drop") {
            if (payload.paths.length !== 1)
              setError("Drop one theme file or folder at a time.");
            else void importFolder(payload.paths[0]);
          }
        })
      : Promise.resolve(() => {});
    void unlisten.catch((error) => {
      if (mounted.current) setError(errorMessage(error));
    });
    return () => {
      mounted.current = false;
      void changes.then((stop) => stop()).catch(() => {});
      window.removeEventListener("focus", focused);
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [refreshCatalog, importFolder]);
  const select = (id: string | null) =>
    void run(
      () =>
        kind === "color" ? themes.select(id) : themes.selectIcons(kind, id),
      "Theme selected. Open windows have been notified.",
    );
  const edit = async (id: string) => {
    const bundle = await api<ThemeBundle>("load_theme", { id });
    setEditing(bundle);
  };
  const allEntries = [
    ...(kind === "color"
      ? builtinThemes.map(({ id, manifest }) => ({
          id,
          name: manifest.name,
          description: manifest.description,
          author: "Built in",
          owner: null,
          error: null,
        }))
      : [
          {
            id: null,
            name: kind === "file" ? "Lomi file icons" : "Lomi interface icons",
            description:
              "Built-in Lucide icons. Export or duplicate to create a portable VS Code icon theme.",
            author: "Built in",
            owner: null,
            error: null,
          },
        ]),
    ...catalog.themes.filter((entry) => (entry.kind ?? "color") === kind),
  ];
  const query = search.trim().toLowerCase();
  const entries = allEntries.filter(
    (entry) =>
      (filter === "all" ||
        (filter === "active" && entry.id === selectedId) ||
        (filter === "attention" && entry.error)) &&
      (source === "all" ||
        (source === "builtin" && isBuiltinTheme(entry.id)) ||
        (source === "local" && !isBuiltinTheme(entry.id) && !entry.owner) ||
        (source === "package" && entry.owner)) &&
      `${entry.name} ${entry.description ?? ""} ${entry.author} ${entry.id ?? "lomi"} ${entry.owner ?? ""}`
        .toLowerCase()
        .includes(query),
  );
  return (
    <main
      className={`themes-page catalog-page${dragging ? " theme-dragging" : ""}`}
      aria-busy={busy}
    >
      <div className="catalog-content">
        <header className="settings-page-heading">
          <div>
            <h1>Themes</h1>
            <p>Make Lomi feel like your workspace.</p>
          </div>
          <div className="catalog-heading-actions">
            <IconButton
              title="Refresh"
              disabled={busy || !native}
              onClick={() =>
                void run(async () => {
                  await refreshCatalog();
                  await api("refresh_themes");
                  await themes.reload();
                }, "Theme files refreshed.")
              }
            >
              <RefreshCw size={16} />
            </IconButton>
            <button
              className="button"
              disabled={busy || !native}
              onClick={() =>
                void run(async () => {
                  const id =
                    kind === "color"
                      ? await api<string>("create_theme")
                      : await api<string>("duplicate_theme", {
                          id: null,
                          kind,
                        });
                  await refreshCatalog();
                  clearFilters();
                  if (kind === "color") await edit(id);
                  else await api("open_themes_folder", { id });
                }, "Starter theme created. Customize it, then select it below.")
              }
            >
              <Plus size={15} aria-hidden="true" />{" "}
              {kind === "color" ? "Create theme" : "Create icon theme"}
            </button>
            <button
              className="button"
              disabled={busy || !native}
              onClick={() =>
                void run(async () => {
                  const path = await open({
                    multiple: false,
                    title: "Import a VS Code theme, package.json, or VSIX",
                    filters: [
                      {
                        name: "VS Code themes",
                        extensions: ["json", "jsonc", "tmTheme", "vsix"],
                      },
                    ],
                  });
                  if (typeof path !== "string") return;
                  const ids = await api<string[]>("import_vscode_themes", {
                    path,
                  });
                  await refreshCatalog();
                  clearFilters();
                  setStatus(
                    `${ids.length} VS Code theme${ids.length === 1 ? "" : "s"} imported. Select a theme below to apply it.`,
                  );
                }, "")
              }
            >
              <Import size={15} aria-hidden="true" /> Import VS Code
            </button>
            <button
              className="button button-primary"
              disabled={busy || !native}
              onClick={() =>
                void run(async () => {
                  const path = await open({
                    directory: true,
                    multiple: false,
                    title: "Import a Lomi theme or VS Code extension folder",
                  });
                  if (typeof path !== "string") return;
                  await api("import_theme", { path });
                  await refreshCatalog();
                  clearFilters();
                  setStatus("Theme imported. Select it below to apply it.");
                }, "")
              }
            >
              <Import size={15} aria-hidden="true" /> Import folder
            </button>
          </div>
        </header>
        {(error || themes.error) && (
          <div className="catalog-notice text-error" role="alert">
            {error || themes.error}
          </div>
        )}
        {themes.safeMode && (
          <p className="catalog-notice" role="status">
            Safe startup is active. Third-party themes are skipped. Restart
            normally to restore your saved selection.
          </p>
        )}
        {!native && (
          <p className="settings-help">
            Theme folders are available in the desktop application.
          </p>
        )}
        <fieldset
          className="theme-appearance theme-mode-row"
          disabled={busy || !themes.ready || !!themes.fixedAppearance}
        >
          <legend>Color mode</legend>
          <div className="theme-appearance-options">
            {(
              [
                ["system", "System", Monitor],
                ["light", "Light", Sun],
                ["dark", "Dark", Moon],
              ] as const
            ).map(([appearance, label, Icon]) => (
              <label key={appearance}>
                <input
                  type="radio"
                  name="appearance"
                  value={appearance}
                  checked={
                    (themes.fixedAppearance ??
                      appearanceDraft ??
                      themes.preferences.appearance) === appearance
                  }
                  onChange={() => {
                    setAppearanceDraft(appearance);
                    void run(
                      () =>
                        themes.select(themes.preferences.active, appearance),
                      "Color mode saved for all windows.",
                    ).finally(() => setAppearanceDraft(null));
                  }}
                />
                <Icon size={16} aria-hidden="true" />
                {label}
              </label>
            ))}
          </div>
          {themes.fixedAppearance && (
            <p className="settings-help">
              This theme defines its own {themes.fixedAppearance} appearance.
              Choose Lomi or DeepMono to use the color mode setting.
            </p>
          )}
        </fieldset>

        <div
          className="catalog-categories"
          role="group"
          aria-label="Theme types"
        >
          {(
            [
              ["color", "Colors"],
              ["file", "File icons"],
              ["product", "Interface icons"],
            ] as const
          ).map(([value, label]) => (
            <button
              key={value}
              aria-pressed={kind === value}
              onClick={() => {
                setKind(value);
                clearFilters();
              }}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="catalog-toolbar">
          <div className="catalog-search">
            <Search size={16} aria-hidden="true" />
            <input
              ref={searchInput}
              type="search"
              aria-label="Search themes"
              placeholder="Search themes…"
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
            aria-label="Theme status"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: "All themes" },
              { value: "active", label: "Active" },
              { value: "attention", label: "Needs attention" },
            ]}
          />
        </div>
        <div className="catalog-filter-row">
          <div
            className="catalog-categories"
            role="group"
            aria-label="Theme sources"
          >
            {[
              { value: "all", label: "All sources" },
              { value: "builtin", label: "Built in" },
              { value: "local", label: "Local" },
              { value: "package", label: "Packages" },
            ].map(({ value, label }) => (
              <button
                key={value}
                aria-pressed={source === value}
                onClick={() => setSource(value)}
              >
                {label}
              </button>
            ))}
          </div>
          <span className="catalog-count" role="status">
            {entries.length === allEntries.length
              ? `${entries.length} available`
              : `${entries.length} of ${allEntries.length}`}
          </span>
        </div>
        {(busy || !themes.ready || status) && (
          <p className="settings-help theme-status" role="status">
            {busy
              ? "Updating themes…"
              : !themes.ready
                ? "Loading themes…"
                : status}
          </p>
        )}
        {entries.length === 0 ? (
          <div className="catalog-empty">
            <Search size={30} aria-hidden="true" />
            <h2>No matching themes</h2>
            <p>Try a different search or reset your filters.</p>
            <button
              className="button"
              onClick={() => {
                clearFilters();
                searchInput.current?.focus();
              }}
            >
              Clear filters
            </button>
          </div>
        ) : (
          <div
            className="catalog-list theme-list"
            aria-label="Available themes"
          >
            {entries.map((entry) => {
              const active = entry.id === selectedId;
              return (
                <article
                  className={`catalog-card theme-card${active ? " active-theme" : ""}`}
                  key={entry.id ?? "builtin"}
                  aria-label={entry.name}
                >
                  <header className="catalog-card-heading">
                    <span className="theme-symbol" aria-hidden="true">
                      <Palette size={19} />
                    </span>
                    <div className="catalog-card-title">
                      <h2>{entry.name}</h2>
                      <span>
                        {entry.author ||
                          (entry.owner ? "Package theme" : "Local theme")}
                      </span>
                    </div>
                    <button
                      className={`button theme-choice${active ? " button-primary" : ""}`}
                      aria-pressed={active}
                      aria-label={`Use ${entry.name} theme`}
                      disabled={busy || !themes.ready || !!entry.error}
                      onClick={() => select(entry.id)}
                    >
                      {active && <Check size={13} aria-hidden="true" />}
                      {active ? "Active" : "Use theme"}
                    </button>
                  </header>
                  <p className="catalog-description">
                    {entry.description ||
                      (entry.owner
                        ? "A theme supplied by an installed package."
                        : "A custom theme for your workspace.")}
                  </p>
                  <div className="catalog-tags">
                    <span>
                      {isBuiltinTheme(entry.id)
                        ? "Built in"
                        : entry.owner
                          ? "Package"
                          : "Local"}
                    </span>
                    {(isBuiltinTheme(entry.id) || entry.owner) && (
                      <span>Read only</span>
                    )}
                    {kind === "color" && entry.id === null && (
                      <span>Default</span>
                    )}
                  </div>
                  {entry.error && (
                    <p className="text-error" role="alert">
                      {entry.error}
                    </p>
                  )}
                  {entry.owner && (
                    <p className="settings-help theme-package-owner">
                      From {entry.owner}. Duplicate to customize.
                    </p>
                  )}
                  <footer className="catalog-card-footer theme-card-actions">
                    <button
                      className="theme-action"
                      disabled={busy || !native || !!entry.error}
                      onClick={() =>
                        void run(async () => {
                          const directory = await open({
                            directory: true,
                            multiple: false,
                            title: "Choose a folder for the VS Code extension",
                          });
                          if (typeof directory !== "string") return;
                          if (kind !== "color") {
                            const path = await api<string>(
                              "export_vscode_icon_theme",
                              { directory, id: entry.id, kind },
                            );
                            setStatus(
                              `Exported to ${path}. In VS Code, run “Extensions: Install from VSIX”.`,
                            );
                            return;
                          }
                          const bundle = entry.id
                            ? await api<ThemeBundle>("load_theme", {
                                id: entry.id,
                              })
                            : null;
                          const manifest = bundle
                            ? parseThemeText(bundle.raw)
                            : builtinTheme;
                          const { exportVSCodeThemes } =
                            await import("./theme/vscode-export");
                          const exported = await exportVSCodeThemes(
                            manifest,
                            bundle,
                          );
                          const path = await api<string>(
                            "export_vscode_theme",
                            {
                              directory,
                              name: manifest.name,
                              themes: exported,
                            },
                          );
                          setStatus(
                            `Exported to ${path}. In VS Code, run “Extensions: Install from VSIX”.`,
                          );
                        }, "")
                      }
                    >
                      <Download size={13} aria-hidden="true" /> Export to VS
                      Code
                    </button>
                    {entry.id && !isBuiltinTheme(entry.id) && (
                      <button
                        className="theme-action"
                        disabled={busy}
                        onClick={() =>
                          void run(
                            () =>
                              kind === "color"
                                ? edit(entry.id!)
                                : api("open_themes_folder", { id: entry.id }),
                            "",
                          )
                        }
                      >
                        <Pencil size={13} aria-hidden="true" />
                        {kind !== "color"
                          ? "Open icon definitions"
                          : entry.owner
                            ? "View theme"
                            : "Edit theme"}
                      </button>
                    )}
                    <button
                      className="theme-action"
                      aria-label="Duplicate theme"
                      disabled={busy || !!entry.error}
                      onClick={() =>
                        void run(async () => {
                          const id = await api<string>("duplicate_theme", {
                            id: entry.id,
                            kind,
                          });
                          await refreshCatalog();
                          clearFilters();
                          if (kind === "color") await edit(id);
                          else await api("open_themes_folder", { id });
                        }, "Theme copied.")
                      }
                    >
                      <Copy size={13} aria-hidden="true" /> Duplicate
                    </button>
                    {entry.id && !isBuiltinTheme(entry.id) && (
                      <IconButton
                        title="Open theme folder"
                        disabled={busy}
                        onClick={() =>
                          void run(
                            () => api("open_themes_folder", { id: entry.id }),
                            "",
                          )
                        }
                      >
                        <FolderOpen size={14} />
                      </IconButton>
                    )}
                  </footer>
                </article>
              );
            })}
          </div>
        )}
      </div>
      {editing && (
        <ThemeEditor
          key={editing.id}
          initial={editing}
          onClose={() => setEditing(null)}
          onSaved={refreshCatalog}
        />
      )}
    </main>
  );
}
