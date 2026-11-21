import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errorMessage, native } from "./api";
import { msg, useT } from "./i18n";
import { useThemes } from "./ThemeProvider";
import { prepareTheme } from "./theme/runtime";
import {
  builtinTheme,
  editThemeText,
  parseThemeText,
  readThemeDraft,
  syntaxNames,
  type ThemeBundle,
  type ThemeManifest,
} from "./theme/format";
import { terminalPresets } from "./theme/palette";
import { vscodeCompatibility, vscodeValues } from "./theme/vscode";
import Select from "./Select";
import { DisclosureSummary, Modal } from "./ui";
const JsonEditor = lazy(() => import("./ThemeJsonEditor"));
export default function ThemeEditor({
  initial,
  onClose,
  onSaved,
}: {
  initial: ThemeBundle;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const { t } = useT();
  const themes = useThemes();
  const [raw, setRaw] = useState(initial.raw),
    [saved, setSaved] = useState(initial);
  const [json, setJson] = useState(() => {
      if (initial.readOnly) return true;
      try {
        parseThemeText(initial.raw);
        return false;
      } catch {
        return true;
      }
    }),
    [section, setSection] = useState<"common" | "light" | "dark">("common"),
    [search, setSearch] = useState(""),
    [overridesOnly, setOverridesOnly] = useState(false);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [status, setStatus] = useState(""),
    [discard, setDiscard] = useState(false),
    [previewing, setPreviewing] = useState(false),
    [presets, setPresets] = useState<string[]>([]);
  const working = useRef(false),
    closing = useRef({ dirty: false, busy: false });
  const dirty = raw !== saved.raw;
  closing.current = { dirty, busy };
  let draft: ThemeManifest | undefined;
  let diagnostic = "";
  try {
    draft = parseThemeText(raw);
  } catch (error) {
    diagnostic = errorMessage(error);
    if (!json) draft = readThemeDraft(raw) as ThemeManifest;
  }
  const close = async () => {
    await themes.cancelPreview();
    onClose();
  };
  const requestClose = () => {
    if (working.current) return;
    if (dirty) setDiscard(true);
    else void close();
  };
  useEffect(() => {
    if (!native) return;
    const stop = getCurrentWindow().onCloseRequested((event) => {
      if (closing.current.dirty || closing.current.busy) {
        event.preventDefault();
        if (!closing.current.busy) setDiscard(true);
      }
    });
    return () => {
      void stop.then((stop) => stop()).catch(() => {});
    };
  }, []);
  useEffect(
    () => () => {
      void themes.cancelPreview();
    },
    [],
  );
  const run = async (action: () => Promise<void>) => {
    if (working.current) return;
    working.current = true;
    setBusy(true);
    setError("");
    setStatus("");
    try {
      await action();
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      working.current = false;
      setBusy(false);
    }
  };
  const change = (path: (string | number)[], value: unknown) => {
    try {
      setRaw(editThemeText(raw, path, value));
      setError("");
    } catch (error) {
      setError(errorMessage(error));
    }
  };
  const save = () =>
    run(async () => {
      parseThemeText(raw);
      const bundle = { ...saved, raw };
      const prepared = await prepareTheme(bundle, themes.preferences);
      prepared.dispose();
      const next = await api<ThemeBundle>("save_theme_manifest", {
        id: saved.id,
        expected: saved.revision,
        raw,
      });
      setSaved(next);
      setRaw(next.raw);
      setPreviewing(false);
      await themes.cancelPreview();
      await onSaved();
      setStatus(
        t(
          "Theme saved. Open windows have been notified; each keeps its last working appearance if loading fails.",
        ),
      );
    });
  let savedName = saved.id;
  try {
    savedName = parseThemeText(saved.raw).name;
  } catch {}
  const values = draft?.[section];
  const mode =
    section === "common"
      ? draft?.appearance === "light" || draft?.appearance === "dark"
        ? draft.appearance
        : themes.snapshot.appearance
      : section;
  const tokens = {
    ...builtinTheme.common?.tokens,
    ...builtinTheme[mode]?.tokens,
    ...(draft?.vscode && !diagnostic ? vscodeValues(draft.vscode).tokens : {}),
    ...(section !== "common" ? draft?.common?.tokens : {}),
    ...values?.tokens,
  };
  return (
    <Modal
      title={t("Edit theme: {name}", { name: savedName })}
      wide
      className={`theme-editor${json ? " theme-editor-json" : ""}`}
      closeDisabled={busy}
      onClose={requestClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <div className="theme-editor-fields dialog-editor-content">
          <fieldset disabled={busy} className="theme-editor-controls">
            <div className="theme-editor-toolbar">
              <p className="settings-help">
                {t(
                  "One JSONC draft. Comments and trailing commas are supported. Preview affects this window only.",
                )}
              </p>
              <button
                type="button"
                className="button"
                disabled={saved.readOnly}
                onClick={() => {
                  if (json && diagnostic) {
                    setError(diagnostic);
                    return;
                  }
                  setJson(!json);
                }}
              >
                {json ? t("Show controls") : t("Edit JSON")}
              </button>
            </div>
            {saved.migration?.map((message) => (
              <p className="settings-help" key={message}>
                {message}
              </p>
            ))}
            {draft?.vscode && !diagnostic && (
              <details className="settings-help">
                <DisclosureSummary>
                  {t("VS Code compatibility")}
                </DisclosureSummary>
                <ul>
                  {vscodeCompatibility(draft.vscode).messages.map((message) => (
                    <li key={message}>{message}</li>
                  ))}
                </ul>
                <p>
                  {t(
                    "Original VS Code data is stored in the vscode section. Lomi overrides are applied above it.",
                  )}
                </p>
              </details>
            )}
            {saved.readOnly && (
              <p role="status">
                {t(
                  "This theme belongs to an immutable plugin package. Duplicate it to edit.",
                )}
              </p>
            )}
            <Suspense
              fallback={<p role="status">{t("Opening JSONC editor…")}</p>}
            >
              <JsonEditor
                value={json ? raw : null}
                disabled={busy || !!saved.readOnly}
                onChange={setRaw}
                onSave={() => void save()}
                onError={setError}
              />
            </Suspense>
            {!json && draft && (
              <>
                <label className="theme-token-row">
                  <span>{t("Name")}</span>
                  <input
                    required
                    maxLength={160}
                    value={draft.name}
                    onChange={(event) => change(["name"], event.target.value)}
                  />
                </label>
                <div className="theme-layout-controls">
                  <label>
                    {t("Theme appearance")}
                    <Select
                      aria-label={t("Theme appearance")}
                      value={draft.appearance ?? "adaptive"}
                      onChange={(value) => change(["appearance"], value)}
                      options={(
                        [
                          ["adaptive", msg("Adaptive")],
                          ["light", msg("Light")],
                          ["dark", msg("Dark")],
                        ] as const
                      ).map(([value, label]) => ({ value, label: t(label) }))}
                    />
                  </label>
                  <label>
                    {t("Edit variant")}
                    <Select
                      aria-label={t("Edit variant")}
                      value={section}
                      onChange={(value) => setSection(value as typeof section)}
                      options={(
                        [
                          ["common", msg("Common")],
                          ["light", msg("Light")],
                          ["dark", msg("Dark")],
                        ] as const
                      ).map(([value, label]) => ({ value, label: t(label) }))}
                    />
                  </label>
                  {(
                    [
                      [
                        "tabs",
                        msg("Tab placement"),
                        [
                          ["inline", msg("Inline")],
                          ["above", msg("Above")],
                          ["below", msg("Below")],
                        ],
                        "inline",
                      ],
                      [
                        "statusbar",
                        msg("Status bar placement"),
                        [
                          ["top", msg("Top")],
                          ["bottom", msg("Bottom")],
                        ],
                        "bottom",
                      ],
                      [
                        "settingsNavigation",
                        msg("Settings navigation"),
                        [
                          ["left", msg("Left")],
                          ["right", msg("Right")],
                          ["top", msg("Top")],
                          ["bottom", msg("Bottom")],
                        ],
                        "left",
                      ],
                    ] as const
                  ).map(([key, label, options, fallback]) => (
                    <label key={key}>
                      {t(label)}
                      <Select
                        aria-label={t(label)}
                        value={values?.layout?.[key] ?? fallback}
                        onChange={(value) =>
                          change([section, "layout", key], value)
                        }
                        options={options.map(([value, text]) => ({
                          value,
                          label: t(text),
                        }))}
                      />
                    </label>
                  ))}
                </div>
                <div className="theme-layout-controls">
                  <label>
                    {t("Terminal palette")}
                    <Select
                      aria-label={t("Terminal palette")}
                      value={values?.terminal?.preset ?? ""}
                      onChange={(value) =>
                        change(
                          [section, "terminal", "preset"],
                          value || undefined,
                        )
                      }
                      options={[
                        {
                          value: "",
                          label: draft.vscode
                            ? t("Inherit theme")
                            : t("Inherit Lomi"),
                        },
                        ...presets.map((value) => ({ value, label: value })),
                      ]}
                    />
                  </label>
                  <button
                    type="button"
                    className="button"
                    onClick={() =>
                      void run(async () =>
                        setPresets(Object.keys(await terminalPresets()).sort()),
                      )
                    }
                  >
                    {t("Load terminal presets")}
                  </button>
                </div>
                <details>
                  <DisclosureSummary>
                    {t("Editor syntax colors")}
                  </DisclosureSummary>
                  <div className="theme-token-list">
                    {syntaxNames.map((name) => (
                      <label className="theme-token-row" key={name}>
                        <span>{name}</span>
                        <input
                          aria-label={t("Syntax {name}", { name })}
                          placeholder={t("Inherit")}
                          value={values?.editor?.syntax?.[name]?.color ?? ""}
                          onChange={(event) =>
                            change(
                              [section, "editor", "syntax", name, "color"],
                              event.target.value || undefined,
                            )
                          }
                        />
                      </label>
                    ))}
                  </div>
                </details>
                <div className="theme-token-filter">
                  <input
                    type="search"
                    aria-label={t("Search theme tokens")}
                    placeholder={t("Search spacing, radius, font, color…")}
                    value={search}
                    onChange={(event) => setSearch(event.target.value)}
                  />
                  <label>
                    <input
                      type="checkbox"
                      checked={overridesOnly}
                      onChange={(event) =>
                        setOverridesOnly(event.target.checked)
                      }
                    />{" "}
                    {t("Overrides only")}
                  </label>
                </div>
                <div className="theme-token-list">
                  {Object.entries(tokens)
                    .filter(
                      ([name]) =>
                        (!overridesOnly ||
                          values?.tokens?.[name] !== undefined) &&
                        name.includes(
                          search.toLowerCase().trim().replaceAll(" ", "-"),
                        ),
                    )
                    .map(([name, fallback]) => (
                      <label key={name} className="theme-token-row">
                        <code>{name}</code>
                        <input
                          aria-label={name}
                          spellCheck={false}
                          value={values?.tokens?.[name] ?? ""}
                          placeholder={fallback}
                          onChange={(event) =>
                            change(
                              [section, "tokens", name],
                              event.target.value || undefined,
                            )
                          }
                        />
                      </label>
                    ))}
                </div>
              </>
            )}
          </fieldset>
        </div>
        <footer className="theme-editor-footer dialog-footer">
          {(error || diagnostic) && (
            <p className="text-error" role="alert">
              {error || diagnostic}
            </p>
          )}
          {status && <p role="status">{status}</p>}
          <div className="dialog-actions">
            <button
              type="button"
              className="button"
              disabled={busy}
              onClick={requestClose}
            >
              {t("Close")}
            </button>
            <button
              type="button"
              className="button"
              disabled={busy || !!diagnostic}
              onClick={() =>
                void run(async () => {
                  await themes.preview({ ...saved, raw });
                  setPreviewing(true);
                })
              }
            >
              {t("Preview")}
            </button>
            {previewing && (
              <button
                type="button"
                className="button"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await themes.cancelPreview();
                    setPreviewing(false);
                  })
                }
              >
                {t("Cancel preview")}
              </button>
            )}
            <button
              type="submit"
              className="button button-primary"
              disabled={busy || !dirty || !!diagnostic || saved.readOnly}
            >
              {busy ? t("Saving…") : t("Save theme")}
            </button>
          </div>
        </footer>
      </form>
      {discard && (
        <Modal
          protectTheme
          title={t("Discard theme changes?")}
          tone="warning"
          onClose={() => setDiscard(false)}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p>{t("Your unsaved theme changes will be lost.")}</p>
            </div>
            <div className="dialog-actions">
              <button className="button" onClick={() => setDiscard(false)}>
                {t("Keep editing")}
              </button>
              <button
                className="button button-danger"
                onClick={() => void close()}
              >
                {t("Discard changes")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </Modal>
  );
}
