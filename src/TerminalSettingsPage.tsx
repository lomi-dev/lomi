import { DisclosureSummary } from "./ui";
import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import Select from "./Select";
import { RotateCcw } from "./icons";
import { api, errorMessage, native, windows } from "./api";
import { useTerminalPreferences } from "./TerminalPreferencesProvider";
import {
  defaultTerminalPreferences,
  terminalBehaviorDefaults,
  terminalBehaviorNumbers,
} from "./terminal-preferences";
import type { TerminalPreferences } from "./terminal-preferences";
import { terminalColors, terminalEnums, terminalNumbers } from "./theme/format";
import type { ThemeTerminal } from "./theme/format";
import {
  terminalAppearance,
  terminalPalette,
  themeAppliedEvent,
} from "./theme/runtime";

const jetBrainsFont = '"JetBrains Mono", monospace';
type FontChoice = "theme" | "jetbrains" | "custom";

function fontChoice(fontFamily?: string): FontChoice {
  if (!fontFamily) return "theme";
  return fontFamily === jetBrainsFont ? "jetbrains" : "custom";
}

const labels: Record<string, string> = {
  alwaysShowTitles: "Always show terminal titles",
  agentNotifications: "Agent notifications",
  windowsShell: "Default shell",
  powershell: "PowerShell",
  cmd: "CMD",
  fontSize: "Font size",
  fontWeight: "Font weight",
  fontWeightBold: "Bold font weight",
  lineHeight: "Line height",
  letterSpacing: "Letter spacing",
  cursorStyle: "Cursor style",
  cursorInactiveStyle: "Inactive cursor",
  cursorBlink: "Blink cursor",
  cursorWidth: "Cursor width",
  minimumContrastRatio: "Minimum contrast ratio",
  drawBoldTextInBrightColors: "Use bright colors for bold text",
  background: "Background",
  foreground: "Text",
  cursor: "Cursor",
  cursorAccent: "Text under cursor",
  selectionBackground: "Selection background",
  selectionForeground: "Selection text",
  selectionInactiveBackground: "Inactive selection",
  searchMatchBackground: "Search match",
  searchActiveMatchBackground: "Active search match",
  searchMatchBorder: "Search border",
  searchActiveMatchBorder: "Active search border",
  scrollback: "Scrollback lines",
  scrollSensitivity: "Scroll speed",
  fastScrollSensitivity: "Fast scroll speed (Alt)",
  smoothScrollDuration: "Scroll animation time",
  tabStopWidth: "Tab width",
  scrollOnUserInput: "Scroll to bottom on input",
  scrollOnEraseInDisplay: "Keep cleared screen in scrollback",
  altClickMovesCursor: "Alt-click moves cursor",
  rightClickSelectsWord: "Right-click selects a word",
  macOptionIsMeta: "Use Option as Meta (macOS)",
  macOptionClickForcesSelection: "Option-click forces selection (macOS)",
  screenReaderMode: "Screen reader support",
  customGlyphs: "Draw continuous box and block characters",
  rescaleOverlappingGlyphs: "Rescale overlapping characters",
  wordSeparator: "Word separators",
};
const help: Record<string, string> = {
  alwaysShowTitles: "When off, hold Control to reveal a title.",
  agentNotifications:
    "Show Claude Code alerts while Lomi is in the background.",
  windowsShell: "Used for new terminals. Existing terminals keep their shell.",
  fontSize: "The size of terminal text, in pixels.",
  lineHeight: "1–3 times the font height.",
  letterSpacing: "−2 to 20 px between characters.",
  cursorWidth: "1–10 px; applies to the bar cursor.",
  minimumContrastRatio:
    "1 keeps exact colors. Higher values increase text contrast, up to 21.",
  scrollback:
    "0–100,000 lines per terminal. Lowering this discards older output; it cannot be restored.",
  smoothScrollDuration: "0 disables animation; up to 1,000 ms.",
  tabStopWidth:
    "Sets the number of spaces in a terminal tab stop. It does not change shell completion or editor indentation.",
  wordSeparator:
    "Characters that separate words when you double-click. Spaces count too.",
  customGlyphs: "Used by the accelerated renderer.",
  rescaleOverlappingGlyphs: "Used by the accelerated renderer.",
};
const labelFor = (key: string) =>
  labels[key] ??
  key
    .replace(/([A-Z])/g, " $1")
    .replace(/^./, (letter) => letter.toUpperCase());

function Setting({
  name,
  value,
  disabled,
  choices,
  range,
  reset,
  resetKind = "theme",
  change,
  color = false,
}: {
  name: string;
  value: string | number | boolean;
  disabled: boolean;
  choices?: readonly string[];
  range?: readonly [number, number];
  reset?: () => void;
  resetKind?: "theme" | "behavior";
  change: (value: string | number | boolean) => void;
  color?: boolean;
}) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value, disabled]);
  const label = labelFor(name);
  const id = `terminal-setting-${name}`;
  const resetLabel =
    resetKind === "theme"
      ? `Restore theme default for ${label}`
      : `Restore default behavior for ${label}`;
  return (
    <div
      className={`terminal-setting-row${typeof value === "boolean" ? " terminal-setting-toggle-row" : ""}`}
    >
      <div className="keybinding-label">
        <label htmlFor={id}>{label}</label>
        {help[name] && <small id={`${id}-help`}>{help[name]}</small>}
      </div>
      <div className="terminal-setting-controls">
        {typeof value === "boolean" ? (
          <input
            id={id}
            type="checkbox"
            role="switch"
            className="settings-switch"
            checked={value}
            aria-describedby={help[name] ? `${id}-help` : undefined}
            disabled={disabled}
            onChange={(event) => change(event.target.checked)}
          />
        ) : choices ? (
          <Select
            id={id}
            value={String(value)}
            disabled={disabled}
            aria-describedby={help[name] ? `${id}-help` : undefined}
            onChange={change}
            options={[...new Set([String(value), ...choices])].map(
              (choice) => ({ value: choice, label: labelFor(choice) }),
            )}
          />
        ) : (
          <>
            {color && (
              <input
                type="color"
                aria-label={`${label} picker`}
                disabled={disabled}
                value={
                  /^#[\da-f]{6}/i.test(text) ? text.slice(0, 7) : "#000000"
                }
                onChange={(event) =>
                  change(
                    event.target.value +
                      (text.length === 9 ? text.slice(7) : ""),
                  )
                }
              />
            )}
            <input
              id={id}
              type={range ? "number" : "text"}
              value={text}
              disabled={disabled}
              aria-describedby={help[name] ? `${id}-help` : undefined}
              min={range?.[0]}
              max={range?.[1]}
              step={
                [
                  "cursorWidth",
                  "scrollback",
                  "smoothScrollDuration",
                  "tabStopWidth",
                ].includes(name)
                  ? 1
                  : "any"
              }
              maxLength={color ? 9 : name === "wordSeparator" ? 200 : 500}
              placeholder={color ? "Automatic" : undefined}
              spellCheck={false}
              onChange={(event) => setText(event.target.value)}
              onBlur={(event) => {
                if (text !== String(value)) {
                  if (event.target.checkValidity())
                    change(range ? (text.trim() ? Number(text) : NaN) : text);
                  else event.target.reportValidity();
                }
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
                if (event.key === "Escape") {
                  event.preventDefault();
                  setText(String(value));
                }
              }}
            />
          </>
        )}
        {reset ? (
          <button
            type="button"
            className="icon-button"
            aria-label={resetLabel}
            title={resetLabel}
            disabled={disabled}
            onClick={reset}
          >
            <RotateCcw size={14} />
          </button>
        ) : typeof value !== "boolean" ? (
          <span
            className="terminal-setting-reset-placeholder"
            aria-hidden="true"
          />
        ) : null}
      </div>
    </div>
  );
}

export default function TerminalSettingsPage() {
  const preferences = useTerminalPreferences();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const [appearance, setAppearance] = useState(terminalAppearance);
  const [palette, setPalette] = useState(terminalPalette);
  const [fontMode, setFontMode] = useState<FontChoice>(() =>
    fontChoice(preferences.value.appearance.fontFamily),
  );
  const [fontDraft, setFontDraft] = useState(
    preferences.value.appearance.fontFamily ?? "",
  );
  const saving = useRef(false);
  const fontFamilyOverride = preferences.value.appearance.fontFamily;

  useEffect(() => {
    const update = () => {
      setAppearance(terminalAppearance());
      setPalette(terminalPalette());
    };
    window.addEventListener(themeAppliedEvent, update);
    update();
    return () => window.removeEventListener(themeAppliedEvent, update);
  }, []);

  useEffect(() => {
    setFontMode(fontChoice(fontFamilyOverride));
    setFontDraft(
      fontChoice(fontFamilyOverride) === "custom" ? fontFamilyOverride! : "",
    );
  }, [fontFamilyOverride]);

  const persist = async (value: TerminalPreferences) => {
    if (saving.current) return false;
    saving.current = true;
    setBusy(true);
    setError("");
    setStatus("Saving…");
    try {
      await preferences.save(value);
      setStatus("Saved");
      return true;
    } catch (error) {
      setError(errorMessage(error));
      setStatus("");
      return false;
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };
  const disabled = !preferences.ready || busy || !!preferences.error;
  const setAppearanceValue = async (
    key: keyof ThemeTerminal,
    value?: string | number | boolean,
  ) => {
    const next = { ...preferences.value.appearance };
    if (value === undefined) delete next[key];
    else Object.assign(next, { [key]: value });
    return persist({ ...preferences.value, appearance: next });
  };
  const appearanceSetting = (key: Exclude<keyof ThemeTerminal, "colors">) => (
    <Setting
      key={key}
      name={key}
      disabled={disabled}
      value={preferences.value.appearance[key] ?? appearance[key] ?? ""}
      range={terminalNumbers[key as keyof typeof terminalNumbers]}
      choices={
        key === "fontWeight" || key === "fontWeightBold"
          ? [
              "normal",
              "bold",
              "100",
              "200",
              "300",
              "400",
              "500",
              "600",
              "700",
              "800",
              "900",
            ]
          : terminalEnums[key as keyof typeof terminalEnums]
      }
      resetKind="theme"
      reset={
        preferences.value.appearance[key] === undefined
          ? undefined
          : () => void setAppearanceValue(key)
      }
      change={(value) =>
        void setAppearanceValue(
          key,
          (key === "fontWeight" || key === "fontWeightBold") &&
            !["normal", "bold"].includes(String(value))
            ? Number(value)
            : value,
        )
      }
    />
  );
  const setColor = (key: (typeof terminalColors)[number], value?: string) => {
    const colors = { ...preferences.value.appearance.colors };
    if (value === undefined) delete colors[key];
    else colors[key] = value;
    void persist({
      ...preferences.value,
      appearance: { ...preferences.value.appearance, colors },
    });
  };
  const colorSetting = (key: (typeof terminalColors)[number]) => (
    <Setting
      key={key}
      name={key}
      color
      disabled={disabled}
      value={preferences.value.appearance.colors?.[key] ?? palette[key] ?? ""}
      resetKind="theme"
      reset={
        preferences.value.appearance.colors?.[key] === undefined
          ? undefined
          : () => setColor(key)
      }
      change={(value) => setColor(key, String(value))}
    />
  );
  const behaviorSetting = (key: keyof TerminalPreferences["behavior"]) => (
    <Setting
      key={key}
      name={key}
      disabled={disabled}
      value={preferences.value.behavior[key]}
      range={
        terminalBehaviorNumbers[key as keyof typeof terminalBehaviorNumbers]
      }
      resetKind="behavior"
      change={(value) =>
        void persist({
          ...preferences.value,
          behavior: { ...preferences.value.behavior, [key]: value },
        })
      }
      reset={
        preferences.value.behavior[key] === terminalBehaviorDefaults[key]
          ? undefined
          : () =>
              void persist({
                ...preferences.value,
                behavior: {
                  ...preferences.value.behavior,
                  [key]: terminalBehaviorDefaults[key],
                },
              })
      }
    />
  );

  const changeFontChoice = async (choice: string) => {
    const next = choice as FontChoice;
    const current = fontChoice(fontFamilyOverride);
    if (next === "custom") {
      setFontMode("custom");
      setFontDraft(current === "custom" ? (fontFamilyOverride ?? "") : "");
      return;
    }
    setFontMode(next);
    const saved = await setAppearanceValue(
      "fontFamily",
      next === "theme" ? undefined : jetBrainsFont,
    );
    if (!saved) {
      setFontMode(current);
      setFontDraft(current === "custom" ? (fontFamilyOverride ?? "") : "");
    }
  };
  const saveCustomFont = async () => {
    if (!fontDraft.trim()) {
      setFontDraft(
        fontChoice(fontFamilyOverride) === "custom"
          ? (fontFamilyOverride ?? "")
          : "",
      );
      return;
    }
    if (fontDraft === fontFamilyOverride) return;
    const saved = await setAppearanceValue("fontFamily", fontDraft);
    if (!saved) setFontDraft(fontFamilyOverride ?? "");
  };
  const resetFont = () => void changeFontChoice("theme");

  const theme = appearance.theme as Record<string, string | undefined>;
  const previewStyle: CSSProperties = {
    backgroundColor:
      palette.background ?? theme.background ?? "var(--color-surface)",
    color:
      palette.foreground ?? theme.foreground ?? "var(--color-surface-text)",
    fontFamily: appearance.fontFamily,
    fontSize: `${appearance.fontSize}px`,
    fontWeight: appearance.fontWeight,
    lineHeight: appearance.lineHeight,
    letterSpacing: `${appearance.letterSpacing}px`,
  };
  const cursorColor = palette.cursor ?? theme.cursor ?? theme.foreground;
  const cursorAccent = palette.cursorAccent ?? theme.cursorAccent;
  const cursorStyle = appearance.cursorStyle;
  const cursorWidth = `${appearance.cursorWidth}px`;
  const cursorGlyphWidth = `max(1px, calc(1ch + ${appearance.letterSpacing}px))`;
  const cursorShapeStyle: CSSProperties = {
    backgroundColor: cursorColor,
    color: cursorAccent ?? previewStyle.backgroundColor,
    width: cursorStyle === "bar" ? cursorWidth : cursorGlyphWidth,
    height: cursorStyle === "underline" ? "2px" : "1em",
    alignSelf: cursorStyle === "underline" ? "end" : "stretch",
  };

  return (
    <main className="terminal-settings-page">
      <header className="settings-page-heading">
        <div className="terminal-settings-heading-copy">
          <h1>Terminal</h1>
          <p>
            Make your terminal comfortable to read. Changes save automatically.
          </p>
        </div>
        <div className="terminal-settings-header-actions">
          <span className="keybindings-status" role="status">
            {!preferences.ready ? "Loading…" : busy ? "Saving…" : status || " "}
          </span>
        </div>
      </header>

      {(error || preferences.error) && (
        <div className="keybindings-error terminal-settings-error" role="alert">
          <span>{preferences.error || error}</span>
          {preferences.error ? (
            <button
              className="text-button"
              disabled={!preferences.ready || busy}
              onClick={() => {
                setError("");
                void preferences.reload();
              }}
            >
              Retry loading
            </button>
          ) : (
            <button className="text-button" onClick={() => setError("")}>
              Dismiss
            </button>
          )}
        </div>
      )}

      <section
        className="terminal-settings-section terminal-settings-preview"
        aria-labelledby="terminal-preview-title"
      >
        <h2 id="terminal-preview-title">Preview</h2>
        <div
          className="terminal-preview-surface"
          role="img"
          aria-label="Terminal preview showing Welcome to Lomi and a prompt"
          style={previewStyle}
        >
          <div className="terminal-preview-content">
            <div className="terminal-preview-line">Welcome to Lomi</div>
            <div className="terminal-preview-line terminal-preview-prompt">
              <span aria-hidden="true">$ </span>
              <span
                className="terminal-preview-cursor"
                style={cursorShapeStyle}
              >
                &nbsp;
              </span>
            </div>
          </div>
        </div>
      </section>

      <section className="terminal-settings-section" aria-label="Text & cursor">
        <h2>Text &amp; cursor</h2>
        <div className="terminal-setting-row">
          <div className="keybinding-label">
            <label htmlFor="terminal-setting-font">Font</label>
            <small id="terminal-setting-font-help">
              Choose a font or keep the theme default.
            </small>
          </div>
          <div className="terminal-setting-controls">
            <Select
              id="terminal-setting-font"
              value={fontMode}
              disabled={disabled}
              aria-describedby="terminal-setting-font-help"
              onChange={(value) => void changeFontChoice(value)}
              options={[
                { value: "theme", label: "Theme default" },
                { value: "jetbrains", label: "JetBrains Mono" },
                { value: "custom", label: "Custom font…" },
              ]}
            />
            <span
              className="terminal-setting-reset-placeholder"
              aria-hidden="true"
            />
          </div>
        </div>
        {fontMode === "custom" && (
          <div className="terminal-setting-row terminal-font-family-row">
            <div className="keybinding-label">
              <label htmlFor="terminal-setting-font-family">Font family</label>
              <small id="terminal-setting-font-family-help">
                Use an installed font. Advanced users can enter comma-separated
                fallback families.
              </small>
            </div>
            <div className="terminal-setting-controls">
              <input
                id="terminal-setting-font-family"
                type="text"
                value={fontDraft}
                disabled={disabled}
                aria-describedby="terminal-setting-font-family-help"
                maxLength={500}
                placeholder="e.g. Iosevka"
                spellCheck={false}
                autoComplete="off"
                onChange={(event) => setFontDraft(event.target.value)}
                onBlur={() => void saveCustomFont()}
                onKeyDown={(event) => {
                  if (event.key === "Enter") event.currentTarget.blur();
                  if (event.key === "Escape") {
                    event.preventDefault();
                    setFontDraft(fontFamilyOverride ?? "");
                  }
                }}
              />
              {fontFamilyOverride !== undefined && (
                <button
                  type="button"
                  className="icon-button"
                  aria-label="Restore theme default for Font family"
                  title="Restore theme default"
                  disabled={disabled}
                  onClick={resetFont}
                >
                  <RotateCcw size={14} />
                </button>
              )}
            </div>
          </div>
        )}
        {(["fontSize", "cursorStyle"] as const).map(appearanceSetting)}
      </section>

      <section className="terminal-settings-section" aria-label="Everyday use">
        <h2>Everyday use</h2>
        <Setting
          name="alwaysShowTitles"
          value={preferences.value.alwaysShowTitles}
          disabled={disabled}
          resetKind="behavior"
          change={(value) =>
            void persist({
              ...preferences.value,
              alwaysShowTitles: Boolean(value),
            })
          }
          reset={
            preferences.value.alwaysShowTitles
              ? () =>
                  void persist({
                    ...preferences.value,
                    alwaysShowTitles: false,
                  })
              : undefined
          }
        />
        <Setting
          name="agentNotifications"
          value={preferences.value.agentNotifications}
          disabled={disabled}
          resetKind="behavior"
          change={(value) =>
            void persist({
              ...preferences.value,
              agentNotifications: Boolean(value),
            })
          }
          reset={
            preferences.value.agentNotifications
              ? undefined
              : () =>
                  void persist({
                    ...preferences.value,
                    agentNotifications: true,
                  })
          }
        />
        <div className="terminal-setting-row terminal-settings-helper-action">
          <div className="keybinding-label">
            <span>Claude Code setup</span>
            <small>
              Set up alerts in the main window. Lomi asks before changing Claude
              Code settings.
            </small>
          </div>
          <button
            type="button"
            className="button"
            disabled={
              disabled || !native || !preferences.value.agentNotifications
            }
            onClick={() => {
              setError("");
              void api("request_agent_notification_setup").catch((error) =>
                setError(errorMessage(error)),
              );
            }}
          >
            Configure Claude Code…
          </button>
        </div>
      </section>

      <details className="terminal-settings-details terminal-settings-disclosure">
        <DisclosureSummary>Customize colors</DisclosureSummary>
        <p className="settings-help">
          Terminal colors follow your theme until you change them. Custom colors
          stay the same in light and dark mode. Enter #RRGGBB or #RRGGBBAA for
          opacity.
        </p>
        {terminalColors.slice(0, 7).map(colorSetting)}
        <details className="terminal-settings-details terminal-settings-nested">
          <DisclosureSummary>ANSI palette and search colors</DisclosureSummary>
          {terminalColors.slice(7).map(colorSetting)}
        </details>
      </details>

      <details className="terminal-settings-details terminal-settings-disclosure">
        <DisclosureSummary>Advanced settings</DisclosureSummary>
        <section
          className="terminal-settings-advanced-group"
          aria-label="Text and cursor details"
        >
          <h3>Text and cursor details</h3>
          {(
            [
              "fontWeight",
              "fontWeightBold",
              "lineHeight",
              "letterSpacing",
              "cursorInactiveStyle",
              "cursorBlink",
              "cursorWidth",
            ] as const
          ).map(appearanceSetting)}
        </section>
        <section
          className="terminal-settings-advanced-group"
          aria-label="Scrolling"
        >
          <h3>Scrolling</h3>
          {(
            [
              "scrollback",
              "scrollSensitivity",
              "fastScrollSensitivity",
              "smoothScrollDuration",
              "scrollOnUserInput",
              "scrollOnEraseInDisplay",
            ] as const
          ).map(behaviorSetting)}
        </section>
        <section
          className="terminal-settings-advanced-group"
          aria-label="Keyboard and selection"
        >
          <h3>Keyboard and selection</h3>
          {windows && (
            <Setting
              name="windowsShell"
              value={preferences.value.windowsShell}
              choices={["powershell", "cmd"]}
              disabled={disabled}
              resetKind="behavior"
              change={(value) =>
                void persist({
                  ...preferences.value,
                  windowsShell: value as TerminalPreferences["windowsShell"],
                })
              }
              reset={
                preferences.value.windowsShell === "powershell"
                  ? undefined
                  : () =>
                      void persist({
                        ...preferences.value,
                        windowsShell: "powershell",
                      })
              }
            />
          )}
          {(
            [
              "tabStopWidth",
              "altClickMovesCursor",
              "rightClickSelectsWord",
              "macOptionIsMeta",
              "macOptionClickForcesSelection",
              "wordSeparator",
            ] as const
          ).map(behaviorSetting)}
        </section>
        <section
          className="terminal-settings-advanced-group"
          aria-label="Accessibility and rendering"
        >
          <h3>Accessibility and rendering</h3>
          {(
            ["minimumContrastRatio", "drawBoldTextInBrightColors"] as const
          ).map(appearanceSetting)}
          {(
            [
              "screenReaderMode",
              "customGlyphs",
              "rescaleOverlappingGlyphs",
            ] as const
          ).map(behaviorSetting)}
        </section>
      </details>

      <footer className="terminal-settings-footer">
        <div>
          <strong>Reset defaults</strong>
          <p>
            Restore all terminal text, colors, and behavior to their defaults.
          </p>
        </div>
        <button
          type="button"
          className="text-button terminal-settings-reset"
          disabled={!preferences.ready || busy}
          onClick={() => {
            void persist(defaultTerminalPreferences).then((saved) => {
              if (saved) {
                setFontMode("theme");
                setFontDraft("");
              }
            });
          }}
        >
          <RotateCcw size={14} /> Reset defaults
        </button>
      </footer>
    </main>
  );
}
