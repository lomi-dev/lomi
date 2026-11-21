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
import { msg, useT } from "./i18n";
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
  const { t, tp } = useT();
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
      }, t("Theme imported")),
    [run, refreshCatalog, clearFilters, t],
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
              setError(t("Drop one theme file or folder at a time."));
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
  }, [refreshCatalog, importFolder, t]);
  const select = (id: string | null) =>
    void run(
      () =>
        kind === "color" ? themes.select(id) : themes.selectIcons(kind, id),
      t("Theme applied"),
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
    }, t("Theme created"));
  const importVSCode = () =>
    void run(async () => {
      const path = await open({
        multiple: false,
        title: t("Import a VS Code theme, package.json, or VSIX"),
        filters: [
          {
            name: t("VS Code themes"),
            extensions: ["json", "jsonc", "tmTheme", "vsix"],
          },
        ],
      });
      if (typeof path !== "string") return;
      const ids = await api<string[]>("import_vscode_themes", { path });
      await refreshCatalog();
      clearFilters();
      setStatus(
        tp(
          ids.length,
          "{count} VS Code theme imported",
          "{count} VS Code themes imported",
        ),
      );
    }, "");
  const importFolderDialog = () =>
    void run(async () => {
      const path = await open({
        directory: true,
        multiple: false,
        title: t("Import a Lomi theme or VS Code extension folder"),
      });
      if (typeof path !== "string") return;
      await api("import_theme", { path });
      await refreshCatalog();
      clearFilters();
      setStatus(t("Theme imported"));
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
            name:
              kind === "file"
                ? t("Lomi file icons")
                : t("Lomi interface icons"),
            description: t(
              "Built-in Lucide icons. Duplicate or export to create a VS Code icon theme.",
            ),
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
      title={t("Themes")}
      busy={busy}
      className={`themes-page catalog-page${dragging ? " theme-dragging" : ""}`}
      contentClassName="catalog-content"
      status={busy ? t("Updating…") : !themes.ready ? t("Loading…") : status}
    >
      {(error || themes.error) && (
        <SettingsNotice tone="error">{error || themes.error}</SettingsNotice>
      )}
      {themes.safeMode && (
        <SettingsNotice tone="warning" role="status">
          {t(
            "Safe startup: third-party themes are skipped. Restart normally to restore your selection.",
          )}
        </SettingsNotice>
      )}
      {!native && (
        <SettingsNotice>
          {t("Theme folders are available in the desktop application.")}
        </SettingsNotice>
      )}
      {notice && (
        <SettingsNotice
          className="theme-status"
          role="status"
          action={
            <button className="text-button" onClick={() => setNotice("")}>
              {t("Dismiss")}
            </button>
          }
        >
          {notice}
        </SettingsNotice>
      )}
      <SettingsSection title={t("Appearance")}>
        <SettingRow
          label={t("Color mode")}
          description={
            themes.fixedAppearance
              ? themes.fixedAppearance === "dark"
                ? t("Set to dark by the selected theme.")
                : t("Set to light by the selected theme.")
              : undefined
          }
          className="theme-mode-row"
        >
          <fieldset
            className="theme-appearance"
            aria-label={t("Color mode")}
            disabled={busy || !themes.ready || !!themes.fixedAppearance}
          >
            <div className="theme-appearance-options">
              {(
                [
                  ["system", msg("System"), Monitor],
                  ["light", msg("Light"), Sun],
                  ["dark", msg("Dark"), Moon],
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
                        t("Color mode saved"),
                      ).finally(() => setAppearanceDraft(null));
                    }}
                  />
                  <Icon size={16} aria-hidden="true" />
                  <ThemePreview mode={appearance} />
                  {t(label)}
                </label>
              ))}
            </div>
          </fieldset>
        </SettingRow>
      </SettingsSection>

      <SettingsSection
        title={t("Theme library")}
        grouped={false}
        actions={
          <div className="theme-manage">
            <button
              type="button"
              className="button theme-manage-trigger"
              data-menu-trigger
              aria-haspopup="menu"
              aria-expanded={manageOpen}
              aria-label={t("Manage themes")}
              title={t("Manage themes")}
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
                      kind === "color"
                        ? t("Create theme")
                        : t("Create icon theme"),
                    action: createTheme,
                  },
                  {
                    Icon: Import,
                    label: t("Import VS Code…"),
                    action: importVSCode,
                  },
                  {
                    Icon: Import,
                    label: t("Import folder…"),
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
          <div
            className="theme-kinds"
            role="group"
            aria-label={t("Theme types")}
          >
            {(
              [
                ["color", msg("Colors")],
                ["file", msg("File icons")],
                ["product", msg("Interface icons")],
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
                {t(label)}
              </button>
            ))}
          </div>
          <div className="catalog-search">
            <Search size={16} aria-hidden="true" />
            <input
              ref={searchInput}
              type="search"
              aria-label={t("Search themes")}
              placeholder={t("Search themes…")}
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
            aria-label={t("Theme status")}
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: t("All themes") },
              { value: "active", label: t("Active") },
              { value: "attention", label: t("Needs attention") },
            ]}
          />
          <Select
            aria-label={t("Theme sources")}
            value={source}
            onChange={setSource}
            options={[
              { value: "all", label: t("All sources") },
              { value: "builtin", label: t("Built in") },
              { value: "local", label: t("Local") },
              { value: "package", label: t("Packages") },
            ]}
          />
        </div>
        {entries.length === 0 ? (
          <div className="catalog-empty">
            <Search size={30} aria-hidden="true" />
            <h2>{t("No matching themes")}</h2>
            <p>{t("Try a different search or reset your filters.")}</p>
            <button
              className="button"
              onClick={() => {
                clearFilters();
                searchInput.current?.focus();
              }}
            >
              {t("Clear filters")}
            </button>
          </div>
        ) : (
          <div
            className="catalog-list theme-list"
            aria-label={t("Available themes")}
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
                        <span className="theme-tag">{t("Read only")}</span>
                      )}
                      {kind === "color" && entry.id === null && (
                        <span className="theme-tag">{t("Default")}</span>
                      )}
                    </div>
                    <p className="theme-row-meta">
                      {!isBuiltinTheme(entry.id) &&
                        `${entry.author || (entry.owner ? t("Package theme") : t("Local theme"))} · `}
                      {entry.description ||
                        (entry.owner
                          ? t("A theme supplied by an installed package.")
                          : t("A custom theme for your workspace."))}
                    </p>
                    {entry.error && (
                      <p className="text-error" role="alert">
                        {entry.error}
                      </p>
                    )}
                    {entry.owner && (
                      <p className="theme-row-meta theme-package-owner">
                        {t("From {owner}. Duplicate to customize.", {
                          owner: entry.owner,
                        })}
                      </p>
                    )}
                  </div>
                  <div className="theme-row-actions">
                    <IconButton
                      title={t("Export to VS Code")}
                      disabled={busy || !native || !!entry.error}
                      onClick={() =>
                        void run(async () => {
                          const directory = await open({
                            directory: true,
                            multiple: false,
                            title: t(
                              "Choose a folder for the VS Code extension",
                            ),
                          });
                          if (typeof directory !== "string") return;
                          if (kind !== "color") {
                            const path = await api<string>(
                              "export_vscode_icon_theme",
                              { directory, id: entry.id, kind },
                            );
                            setNotice(
                              t(
                                "Exported to {path}. In VS Code, run “Extensions: Install from VSIX”.",
                                { path },
                              ),
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
                            t(
                              "Exported to {path}. In VS Code, run “Extensions: Install from VSIX”.",
                              { path },
                            ),
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
                            ? t("Open icon definitions")
                            : entry.owner
                              ? t("View theme")
                              : t("Edit theme")
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
                      title={t("Duplicate theme")}
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
                        }, t("Theme copied"))
                      }
                    >
                      <Copy size={15} />
                    </IconButton>
                    {entry.id && !isBuiltinTheme(entry.id) && (
                      <IconButton
                        title={t("Open theme folder")}
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
                    aria-label={t("Use {name} theme", { name: entry.name })}
                    disabled={busy || !themes.ready || !!entry.error}
                    onClick={() => select(entry.id)}
                  >
                    {active && <Check size={13} aria-hidden="true" />}
                    {active ? t("Active") : t("Use")}
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
