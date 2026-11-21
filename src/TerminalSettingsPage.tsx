import "./settings-sections.css";
import "./settings-terminal.css";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import Select from "./Select";
import { RotateCcw } from "./icons";
import { errorMessage, windows } from "./api";
import { msg, t, useT } from "./i18n";
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
  alwaysShowTitles: msg("Always show terminal titles"),
  windowsShell: msg("Default shell"),
  powershell: msg("PowerShell"),
  cmd: msg("CMD"),
  fontSize: msg("Font size"),
  fontWeight: msg("Font weight"),
  fontWeightBold: msg("Bold font weight"),
  lineHeight: msg("Line height"),
  letterSpacing: msg("Letter spacing"),
  cursorStyle: msg("Cursor style"),
  cursorInactiveStyle: msg("Inactive cursor"),
  cursorBlink: msg("Blink cursor"),
  cursorWidth: msg("Cursor width"),
  minimumContrastRatio: msg("Minimum contrast ratio"),
  drawBoldTextInBrightColors: msg("Use bright colors for bold text"),
  background: msg("Background"),
  foreground: msg("Text"),
  cursor: msg("Cursor"),
  cursorAccent: msg("Text under cursor"),
  selectionBackground: msg("Selection background"),
  selectionForeground: msg("Selection text"),
  selectionInactiveBackground: msg("Inactive selection"),
  searchMatchBackground: msg("Search match"),
  searchActiveMatchBackground: msg("Active search match"),
  searchMatchBorder: msg("Search border"),
  searchActiveMatchBorder: msg("Active search border"),
  scrollback: msg("Scrollback lines"),
  scrollSensitivity: msg("Scroll speed"),
  fastScrollSensitivity: msg("Fast scroll speed (Alt)"),
  smoothScrollDuration: msg("Scroll animation time"),
  tabStopWidth: msg("Tab width"),
  scrollOnUserInput: msg("Scroll to bottom on input"),
  scrollOnEraseInDisplay: msg("Keep cleared screen in scrollback"),
  altClickMovesCursor: msg("Alt-click moves cursor"),
  rightClickSelectsWord: msg("Right-click selects a word"),
  macOptionIsMeta: msg("Use Option as Meta (macOS)"),
  macOptionClickForcesSelection: msg("Option-click forces selection (macOS)"),
  screenReaderMode: msg("Screen reader support"),
  customGlyphs: msg("Draw continuous box and block characters"),
  rescaleOverlappingGlyphs: msg("Rescale overlapping characters"),
  wordSeparator: msg("Word separators"),
  bar: msg("Bar"),
  block: msg("Block"),
  underline: msg("Underline"),
  outline: msg("Outline"),
  none: msg("None"),
  normal: msg("Normal"),
  bold: msg("Bold"),
};
const help: Record<string, string> = {
  alwaysShowTitles: msg("Otherwise hold Control to reveal titles."),
  windowsShell: msg("Applies to new terminals."),
  cursorWidth: msg("Bar cursor only."),
  minimumContrastRatio: msg("1 keeps exact colors."),
  scrollback: msg("Lowering it drops older output."),
  smoothScrollDuration: msg("0 turns animation off."),
  wordSeparator: msg("Characters that end a word on double-click."),
  customGlyphs: msg("Accelerated renderer only."),
  rescaleOverlappingGlyphs: msg("Accelerated renderer only."),
};
export const labelFor = (key: string) =>
  (labels[key] && t(labels[key])) ||
  key
    .replace(/([A-Z])/g, " $1")
    .replace(/^./, (letter) => letter.toUpperCase());

const units: Record<string, string> = {
  lineHeight: "×",
  letterSpacing: "px",
  cursorWidth: "px",
  scrollback: msg("lines"),
  smoothScrollDuration: "ms",
  tabStopWidth: msg("spaces"),
  scrollSensitivity: "×",
  fastScrollSensitivity: "×",
};

function Control({
  name,
  value,
  disabled,
  choices,
  range,
  reset,
  resetKind = "theme",
  change,
  unit,
}: {
  name: string;
  value: string | number | boolean;
  disabled: boolean;
  choices?: readonly string[];
  range?: readonly [number, number];
  reset?: () => void;
  resetKind?: "theme" | "behavior";
  change: (value: string | number | boolean) => void;
  unit?: string;
}) {
  useT();
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value, disabled]);
  const label = labelFor(name);
  unit ??= range ? units[name] : undefined;
  if (unit) unit = t(unit);
  const id = `terminal-setting-${name}`;
  const resetLabel =
    resetKind === "theme"
      ? t("Restore theme default for {label}", { label })
      : t("Restore default behavior for {label}", { label });
  const field = (
    <input
      id={id}
      type={range ? "number" : "text"}
      value={text}
      disabled={disabled}
      aria-label={unit ? `${label} (${unit})` : undefined}
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
      maxLength={name === "wordSeparator" ? 200 : 500}
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
  );
  return (
    <>
      {reset && (
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
      )}
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
          options={[...new Set([String(value), ...choices])].map((choice) => ({
            value: choice,
            label: labelFor(choice),
          }))}
        />
      ) : unit ? (
        <span className="terminal-unit-field" data-unit={unit}>
          {field}
        </span>
      ) : (
        field
      )}
    </>
  );
}

function Setting(props: Parameters<typeof Control>[0]) {
  useT();
  const id = `terminal-setting-${props.name}`;
  return (
    <SettingRow
      label={labelFor(props.name)}
      htmlFor={id}
      description={help[props.name] && t(help[props.name])}
      descriptionId={`${id}-help`}
    >
      <Control {...props} />
    </SettingRow>
  );
}

export type ColorKey = (typeof terminalColors)[number];

export default function TerminalSettingsPage() {
  const { t } = useT();
  const preferences = useTerminalPreferences();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
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
    try {
      await preferences.save(value);
      return true;
    } catch (error) {
      setError(errorMessage(error));
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
  const effectiveColor = (key: ColorKey) => palette[key];
  const effective = <K extends keyof ThemeTerminal>(key: K) =>
    preferences.value.appearance[key] ?? (appearance as ThemeTerminal)[key];
  const fontFamilyShown =
    fontFamilyOverride ?? String(appearance.fontFamily ?? "monospace");
  const cursorStyle = effective("cursorStyle") ?? "bar";
  const previewColor = (key: ColorKey, fallback = "currentColor") =>
    effectiveColor(key) ?? fallback;
  const ansi = (key: ColorKey) => ({ color: previewColor(key) });
  const cursorColor = previewColor("cursor");
  const previewStyle: CSSProperties = {
    background: previewColor("background", "transparent"),
    color: previewColor("foreground"),
    fontFamily: fontFamilyShown,
    fontSize: `${effective("fontSize") ?? 14}px`,
    lineHeight: effective("lineHeight") ?? 1.25,
    letterSpacing: `${effective("letterSpacing") ?? 0}px`,
    fontWeight: effective("fontWeight") ?? "normal",
  };
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

  return (
    <SettingsPage
      title={t("Terminal")}
      className="terminal-settings-page"
      busy={!preferences.ready || busy}
    >
      {(error || preferences.error) && (
        <SettingsNotice
          tone="error"
          action={
            preferences.error ? (
              <>
                <button
                  className="text-button"
                  disabled={!preferences.ready || busy}
                  onClick={() => {
                    setError("");
                    void preferences.reload();
                  }}
                >
                  {t("Retry loading")}
                </button>
                <button
                  className="text-button"
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
                  {t("Reset to defaults")}
                </button>
              </>
            ) : (
              <button className="text-button" onClick={() => setError("")}>
                {t("Dismiss")}
              </button>
            )
          }
        >
          {preferences.error || error}
        </SettingsNotice>
      )}

      <SettingsSection title={t("Font")}>
        <SettingRow
          label={t("Terminal font")}
          htmlFor="terminal-setting-font"
          description={t("Used by every terminal.")}
        >
          <Select
            id="terminal-setting-font"
            value={fontMode}
            disabled={disabled}
            onChange={(value) => void changeFontChoice(value)}
            options={[
              { value: "theme", label: t("Theme default") },
              { value: "jetbrains", label: "JetBrains Mono" },
              { value: "custom", label: t("Custom font…") },
            ]}
          />
          <Control
            name="fontSize"
            unit="px"
            disabled={disabled}
            value={
              preferences.value.appearance.fontSize ?? appearance.fontSize ?? ""
            }
            range={terminalNumbers.fontSize}
            reset={
              preferences.value.appearance.fontSize === undefined
                ? undefined
                : () => void setAppearanceValue("fontSize")
            }
            change={(value) => void setAppearanceValue("fontSize", value)}
          />
        </SettingRow>
        {fontMode === "custom" && (
          <SettingRow
            label={t("Font family")}
            htmlFor="terminal-setting-font-family"
            description={t(
              "An installed font. Separate fallbacks with commas.",
            )}
            descriptionId="terminal-setting-font-family-help"
          >
            {fontFamilyOverride !== undefined && (
              <button
                type="button"
                className="icon-button"
                aria-label={t("Restore theme default for Font family")}
                title={t("Restore theme default")}
                disabled={disabled}
                onClick={resetFont}
              >
                <RotateCcw size={14} />
              </button>
            )}
            <input
              id="terminal-setting-font-family"
              type="text"
              value={fontDraft}
              disabled={disabled}
              aria-describedby="terminal-setting-font-family-help"
              maxLength={500}
              placeholder={t("e.g. Iosevka")}
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
          </SettingRow>
        )}
        <div className="terminal-preview-frame">
          <div
            className="terminal-preview"
            role="img"
            aria-label={t("Live preview of the terminal font and colors")}
            style={previewStyle}
          >
            <div>
              <span style={ansi("brightGreen")}>➜ </span>
              <span style={ansi("cyan")}>lomi </span>
              <span style={ansi("blue")}>git:(</span>
              <span style={ansi("red")}>main</span>
              <span style={ansi("blue")}>) </span>
              <span style={ansi("yellow")}>✗ </span>pnpm dev
            </div>
            <div>
              {"  "}
              <span style={ansi("brightGreen")}>VITE v7</span> ready in{" "}
              <b>1.24s</b>
            </div>
            <div>
              {"  "}
              <span style={ansi("brightGreen")}>➜ </span>
              <b>Local:</b>{" "}
              <span
                style={{ ...ansi("brightBlue"), textDecoration: "underline" }}
              >
                http://127.0.0.1:5173/
              </span>
            </div>
            <div>
              {"  "}
              <span style={ansi("green")}>✓ 85 passed</span>
              {"  "}
              <span style={ansi("yellow")}>△ 2 warnings</span>
              {"  "}
              <span style={ansi("red")}>✗ 0 failed</span>
            </div>
            <div>
              <span style={ansi("brightGreen")}>➜ </span>
              <span style={ansi("cyan")}>lomi </span>
              <span style={ansi("blue")}>git:(</span>
              <span style={ansi("red")}>main</span>
              <span style={ansi("blue")}>) </span>
              rg{" "}
              <span
                style={{
                  background: previewColor(
                    "selectionBackground",
                    "transparent",
                  ),
                  color: previewColor("selectionForeground"),
                }}
              >
                formatUser
              </span>
            </div>
            <div>
              <span style={ansi("magenta")}>src/user.ts</span>
              <span style={ansi("brightBlack")}>:12 </span> export function{" "}
              <span
                style={{
                  background: previewColor(
                    "searchActiveMatchBackground",
                    "transparent",
                  ),
                  outline: `1px solid ${previewColor("searchActiveMatchBorder", "transparent")}`,
                }}
              >
                formatUser
              </span>
              (user){" "}
              <span
                style={{
                  background: previewColor(
                    "searchMatchBackground",
                    "transparent",
                  ),
                  outline: `1px solid ${previewColor("searchMatchBorder", "transparent")}`,
                }}
              >
                {"{"}
              </span>
            </div>
            <div>
              <span style={ansi("brightGreen")}>➜ </span>
              <span style={ansi("cyan")}>lomi </span>
              <span style={ansi("blue")}>git:(</span>
              <span style={ansi("red")}>main</span>
              <span style={ansi("blue")}>) </span>
              <span
                className="terminal-preview-cursor"
                data-style={cursorStyle}
                style={
                  {
                    "--cursor": cursorColor,
                    "--cursor-width": `${effective("cursorWidth") ?? 1}px`,
                    "--cursor-text": previewColor("cursorAccent"),
                  } as CSSProperties
                }
              >
                {"\u00a0"}
              </span>
            </div>
          </div>
        </div>
        {appearanceSetting("cursorStyle")}
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
      </SettingsSection>

      <SettingsSection title={t("Text")}>
        {(
          [
            "fontWeight",
            "fontWeightBold",
            "lineHeight",
            "letterSpacing",
          ] as const
        ).map(appearanceSetting)}
      </SettingsSection>
      <SettingsSection title={t("Cursor")}>
        {(["cursorInactiveStyle", "cursorBlink", "cursorWidth"] as const).map(
          appearanceSetting,
        )}
      </SettingsSection>
      <SettingsSection title={t("Scrolling")}>
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
      </SettingsSection>
      <SettingsSection title={t("Input")}>
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
      </SettingsSection>
      <SettingsSection title={t("Rendering")}>
        {(["minimumContrastRatio", "drawBoldTextInBrightColors"] as const).map(
          appearanceSetting,
        )}
        {(
          [
            "screenReaderMode",
            "customGlyphs",
            "rescaleOverlappingGlyphs",
          ] as const
        ).map(behaviorSetting)}
      </SettingsSection>
    </SettingsPage>
  );
}
