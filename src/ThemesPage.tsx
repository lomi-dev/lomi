import "./settings-themes.css";
import type { IconKind } from "./theme/icon-theme";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import {
  Check,
  Copy,
  Download,
  Ellipsis,
  FolderOpen,
  Import,
  Monitor,
  Moon,
  Plus,
  Pencil,
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
import LomiSymbol from "./LomiSymbol";
import Select from "./Select";
import { IconButton, Menu } from "./ui";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";

const PreviewScene = ({ tone }: { tone: "light" | "dark" }) => (
  <span className={`tp tp-${tone}`}>
    <span className="tp-bar">
      <i />
      <i />
    </span>
    <span className="tp-side">
      <i />
      <i />
      <i />
      <i />
      <i />
    </span>
    <span className="tp-stage">
      <b />
      <b />
      <b />
      <b />
    </span>
    <span className="tp-status" />
  </span>
);

function ThemePreview({ mode }: { mode: "system" | "light" | "dark" }) {
  return (
    <span className="theme-preview" aria-hidden="true">
      {mode !== "dark" && (
        <span className={mode === "system" ? "tp-clip-light" : "tp-whole"}>
          <PreviewScene tone="light" />
        </span>
      )}
      {mode !== "light" && (
        <span className={mode === "system" ? "tp-clip" : "tp-whole"}>
          <PreviewScene tone="dark" />
        </span>
      )}
    </span>
  );
}

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
  const [notice, setNotice] = useState("");
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
  const [manageOpen, setManageOpen] = useState(false);
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
      setNotice("");
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
      }, "Theme imported"),
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
      "Theme applied",
    );
  const edit = async (id: string) => {
    const bundle = await api<ThemeBundle>("load_theme", { id });
    setEditing(bundle);
  };
  const createTheme = () =>
    void run(async () => {
      const id =
        kind === "color"
          ? await api<string>("create_theme")
          : await api<string>("duplicate_theme", { id: null, kind });
      await refreshCatalog();
      clearFilters();
      if (kind === "color") await edit(id);
      else await api("open_themes_folder", { id });
    }, "Theme created");
  const importVSCode = () =>
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
      const ids = await api<string[]>("import_vscode_themes", { path });
      await refreshCatalog();
      clearFilters();
      setStatus(
        `${ids.length} VS Code theme${ids.length === 1 ? "" : "s"} imported`,
      );
    }, "");
  const importFolderDialog = () =>
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
      setStatus("Theme imported");
    }, "");
  const allEntries = [
    ...(kind === "color"
      ? builtinThemes.map(({ id, manifest }) => ({
          id,
          name: manifest.name,
          description: manifest.description,
          author: "",
          owner: null,
          error: null,
        }))
      : [
          {
            id: null,
            name: kind === "file" ? "Lomi file icons" : "Lomi interface icons",
            description:
              "Built-in Lucide icons. Duplicate or export to create a VS Code icon theme.",
            author: "",
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
    <SettingsPage
      title="Themes"
      busy={busy}
      className={`themes-page catalog-page${dragging ? " theme-dragging" : ""}`}
      contentClassName="catalog-content"
      status={busy ? "Updating…" : !themes.ready ? "Loading…" : status}
    >
      {(error || themes.error) && (
        <SettingsNotice tone="error">{error || themes.error}</SettingsNotice>
      )}
      {themes.safeMode && (
        <SettingsNotice tone="warning" role="status">
          Safe startup: third-party themes are skipped. Restart normally to
          restore your selection.
        </SettingsNotice>
      )}
      {!native && (
        <SettingsNotice>
          Theme folders are available in the desktop application.
        </SettingsNotice>
      )}
      {notice && (
        <SettingsNotice
          className="theme-status"
          role="status"
          action={
            <button className="text-button" onClick={() => setNotice("")}>
              Dismiss
            </button>
          }
        >
          {notice}
        </SettingsNotice>
      )}
      <SettingsSection title="Appearance">
        <SettingRow
          label="Color mode"
          description={
            themes.fixedAppearance
              ? `Set to ${themes.fixedAppearance} by the selected theme.`
              : undefined
          }
          className="theme-mode-row"
        >
          <fieldset
            className="theme-appearance"
            aria-label="Color mode"
            disabled={busy || !themes.ready || !!themes.fixedAppearance}
          >
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
                        "Color mode saved",
                      ).finally(() => setAppearanceDraft(null));
                    }}
                  />
                  <Icon size={16} aria-hidden="true" />
                  <ThemePreview mode={appearance} />
                  {label}
                </label>
              ))}
            </div>
          </fieldset>
        </SettingRow>
      </SettingsSection>

      <SettingsSection
        title="Theme library"
        grouped={false}
        actions={
          <div className="theme-manage">
            <button
              type="button"
              className="button theme-manage-trigger"
              data-menu-trigger
              aria-haspopup="menu"
              aria-expanded={manageOpen}
              aria-label="Manage themes"
              title="Manage themes"
              onClick={() => setManageOpen((value) => !value)}
            >
              <Ellipsis size={16} aria-hidden="true" />
            </button>
            {manageOpen && (
              <Menu
                className="theme-manage-menu"
                onClose={() => setManageOpen(false)}
              >
                {[
                  {
                    Icon: Plus,
                    label:
                      kind === "color" ? "Create theme" : "Create icon theme",
                    action: createTheme,
                  },
                  {
                    Icon: Import,
                    label: "Import VS Code…",
                    action: importVSCode,
                  },
                  {
                    Icon: Import,
                    label: "Import folder…",
                    action: importFolderDialog,
                  },
                ].map(({ Icon, label, action }) => (
                  <button
                    key={label}
                    type="button"
                    role="menuitem"
                    className="menu-item"
                    disabled={busy || !native}
                    onClick={() => {
                      setManageOpen(false);
                      action();
                    }}
                  >
                    <Icon size={15} aria-hidden="true" />
                    <span>{label}</span>
                  </button>
                ))}
              </Menu>
            )}
          </div>
        }
      >
        <div className="catalog-toolbar theme-toolbar">
          <div className="theme-kinds" role="group" aria-label="Theme types">
            {(
              [
                ["color", "Colors"],
                ["file", "File icons"],
                ["product", "Interface icons"],
              ] as const
            ).map(([value, label]) => (
              <button
                key={value}
                type="button"
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
          <Select
            aria-label="Theme sources"
            value={source}
            onChange={setSource}
            options={[
              { value: "all", label: "All sources" },
              { value: "builtin", label: "Built in" },
              { value: "local", label: "Local" },
              { value: "package", label: "Packages" },
            ]}
          />
        </div>
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
                  className={`theme-row${active ? " active-theme" : ""}`}
                  key={entry.id ?? "builtin"}
                  aria-label={entry.name}
                >
                  <ThemeLogo
                    builtin={isBuiltinTheme(entry.id)}
                    name={entry.name}
                  />
                  <div className="theme-row-main">
                    <div className="theme-row-name">
                      <h3>{entry.name}</h3>
                      {(isBuiltinTheme(entry.id) || entry.owner) && (
                        <span className="theme-tag">Read only</span>
                      )}
                      {kind === "color" && entry.id === null && (
                        <span className="theme-tag">Default</span>
                      )}
                    </div>
                    <p className="theme-row-meta">
                      {!isBuiltinTheme(entry.id) &&
                        `${entry.author || (entry.owner ? "Package theme" : "Local theme")} · `}
                      {entry.description ||
                        (entry.owner
                          ? "A theme supplied by an installed package."
                          : "A custom theme for your workspace.")}
                    </p>
                    {entry.error && (
                      <p className="text-error" role="alert">
                        {entry.error}
                      </p>
                    )}
                    {entry.owner && (
                      <p className="theme-row-meta theme-package-owner">
                        From {entry.owner}. Duplicate to customize.
                      </p>
                    )}
                  </div>
                  <div className="theme-row-actions">
                    <IconButton
                      title="Export to VS Code"
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
                            setNotice(
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
                          setNotice(
                            `Exported to ${path}. In VS Code, run “Extensions: Install from VSIX”.`,
                          );
                        }, "")
                      }
                    >
                      <Download size={15} />
                    </IconButton>
                    {entry.id && !isBuiltinTheme(entry.id) && (
                      <IconButton
                        title={
                          kind !== "color"
                            ? "Open icon definitions"
                            : entry.owner
                              ? "View theme"
                              : "Edit theme"
                        }
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
                        <Pencil size={15} />
                      </IconButton>
                    )}
                    <IconButton
                      title="Duplicate theme"
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
                        }, "Theme copied")
                      }
                    >
                      <Copy size={15} />
                    </IconButton>
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
                        <FolderOpen size={15} />
                      </IconButton>
                    )}
                  </div>
                  <button
                    type="button"
                    className={active ? "theme-active-pill" : "button"}
                    aria-pressed={active}
                    aria-label={`Use ${entry.name} theme`}
                    disabled={busy || !themes.ready || !!entry.error}
                    onClick={() => select(entry.id)}
                  >
                    {active && <Check size={13} aria-hidden="true" />}
                    {active ? "Active" : "Use"}
                  </button>
                </article>
              );
            })}
          </div>
        )}
      </SettingsSection>
      {editing && (
        <ThemeEditor
          key={editing.id}
          initial={editing}
          onClose={() => setEditing(null)}
          onSaved={refreshCatalog}
        />
      )}
    </SettingsPage>
  );
}

/** Extension-style logo tile: the Lomi mark for the built-in theme, otherwise
 *  initials on a hue derived from the theme name (stable across sessions). */
function ThemeLogo({ builtin, name }: { builtin: boolean; name: string }) {
  if (builtin)
    return (
      <span className="theme-logo theme-logo-lomi" aria-hidden="true">
        <LomiSymbol size={20} />
      </span>
    );
  const initials =
    name
      .split(/[\s_-]+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((word) => word[0])
      .join("")
      .toLocaleUpperCase() || "?";
  let hue = 0;
  for (const char of name) hue = (hue * 31 + char.codePointAt(0)!) % 360;
  return (
    <span
      className="theme-logo"
      style={{ "--logo-hue": hue } as CSSProperties}
      aria-hidden="true"
    >
      {initials}
    </span>
  );
}
