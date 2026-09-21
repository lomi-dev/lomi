import { DisclosureSummary } from "./ui";
import { useEffect, useRef, useState } from "react";
import Select from "./Select";
import { RotateCcw } from "./icons";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
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
  loadTerminalFonts,
  terminalAppearance,
  terminalPalette,
  themeAppliedEvent,
} from "./theme/runtime";

const labels: Record<string, string> = {
  alwaysShowTitles: "Always show terminal titles",
  agentNotifications: "Agent notifications",
  windowsShell: "Default shell",
  powershell: "PowerShell",
  cmd: "CMD",
  fontFamily: "Font family",
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
  smoothScrollDuration: "Smooth scrolling (ms)",
  tabStopWidth: "Tab stop width",
  scrollOnUserInput: "Scroll to bottom on input",
  scrollOnEraseInDisplay: "Keep cleared screen in scrollback",
  altClickMovesCursor: "Alt-click moves cursor",
  rightClickSelectsWord: "Right-click selects word",
  macOptionIsMeta: "Use Option as Meta (macOS)",
  macOptionClickForcesSelection: "Option-click forces selection (macOS)",
  screenReaderMode: "Screen reader support",
  customGlyphs: "Draw continuous box and block characters",
  rescaleOverlappingGlyphs: "Rescale overlapping characters",
  wordSeparator: "Word separators",
};
const help: Record<string, string> = {
  alwaysShowTitles:
    "When off, press Control to show titles for 5 seconds. Hold Control for more than 1 second to show them until you release it.",
  agentNotifications:
    "Notify when Claude Code finishes responding or needs your input while SimpleBench is in the background. Configure Claude Code once below.",
  windowsShell:
    "Used for new terminal tabs and workspaces. Existing terminals keep their shell, including when split or restored. PowerShell uses version 7 when installed, otherwise Windows PowerShell.",
  fontFamily:
    "Use an installed font or a comma-separated fallback list. JetBrains Mono and fallback symbol fonts are bundled.",
  fontSize: "6–72 px.",
  lineHeight: "1–3 times the font height.",
  letterSpacing: "−2 to 20 px between characters.",
  cursorWidth: "1–10 px; applies to the bar cursor.",
  minimumContrastRatio:
    "1 keeps exact colors. Higher values increase text contrast, up to 21.",
  scrollback:
    "0–100,000 lines per terminal. Lowering this discards older output; it cannot be restored.",
  smoothScrollDuration: "0 disables animation; up to 1,000 ms.",
  tabStopWidth:
    "1–32 columns. Changes terminal tab stops, not shell completion or editor indentation.",
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
  change,
  color = false,
}: {
  name: string;
  value: string | number | boolean;
  disabled: boolean;
  choices?: readonly string[];
  range?: readonly [number, number];
  reset?: () => void;
  change: (value: string | number | boolean) => void;
  color?: boolean;
}) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value, disabled]);
  const label = labelFor(name);
  const id = `terminal-setting-${name}`;
  return (
    <div className="terminal-setting-row">
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
              list={
                name === "fontFamily" ? "terminal-font-families" : undefined
              }
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
        <button
          type="button"
          className="icon-button"
          aria-label={`Reset ${label}`}
          title="Restore default"
          disabled={disabled || !reset}
          onClick={reset}
        >
          <RotateCcw size={14} />
        </button>
      </div>
    </div>
  );
}

function TerminalPreview() {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const terminal = new Terminal({
      ...terminalAppearance(),
      disableStdin: true,
      allowTransparency: true,
      rows: 6,
      cols: 60,
      scrollback: 0,
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host.current!);
    const render = () => {
      fit.fit();
      const width = terminal.cols - 1;
      const lines = [
        "simplebench ~/project".slice(0, width),
        "$ echo 'Your terminal, your style'".slice(0, width),
        `\x1b[1mBold\x1b[0m  \x1b[3mItalic\x1b[0m${width >= 32 ? "  Zażółć  0O 1Il" : ""}`,
        Array.from(
          { length: Math.min(16, Math.floor(width / 3)) },
          (_, index) => `\x1b[${index < 8 ? 30 + index : 90 + index - 8}m██ `,
        ).join("") + "\x1b[0m",
        "$ ",
      ];
      terminal.write(
        "\x1b[2J\x1b[H" + lines.slice(0, terminal.rows).join("\r\n"),
      );
    };
    let revision = 0;
    const update = () => {
      const request = ++revision;
      const options = terminalAppearance();
      void loadTerminalFonts(options).then(() => {
        if (request !== revision) return;
        terminal.options.fontFamily = `${options.fontFamily} `;
        terminal.options = { ...options, scrollback: 0 };
        render();
      });
    };
    const observer = new ResizeObserver(render);
    observer.observe(host.current!);
    window.addEventListener(themeAppliedEvent, update);
    update();
    return () => {
      ++revision;
      observer.disconnect();
      window.removeEventListener(themeAppliedEvent, update);
      terminal.dispose();
    };
  }, []);
  return (
    <div
      className="terminal-pane terminal-preview"
      aria-label="Terminal preview"
    >
      <div className="terminal-host" ref={host} />
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
  const saving = useRef(false);
  useEffect(() => {
    const update = () => {
      setAppearance(terminalAppearance());
      setPalette(terminalPalette());
    };
    window.addEventListener(themeAppliedEvent, update);
    update();
    return () => window.removeEventListener(themeAppliedEvent, update);
  }, []);
  const persist = async (value: TerminalPreferences) => {
    if (saving.current) return;
    saving.current = true;
    setBusy(true);
    setError("");
    setStatus("Saving…");
    try {
      await preferences.save(value);
      setStatus("Saved");
    } catch (error) {
      setError(errorMessage(error));
      setStatus("");
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };
  const disabled = !preferences.ready || busy || !!preferences.error;
  const setAppearanceValue = (
    key: keyof ThemeTerminal,
    value?: string | number | boolean,
  ) => {
    const next = { ...preferences.value.appearance };
    if (value === undefined) delete next[key];
    else Object.assign(next, { [key]: value });
    void persist({ ...preferences.value, appearance: next });
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
      reset={
        preferences.value.appearance[key] === undefined
          ? undefined
          : () => setAppearanceValue(key)
      }
      change={(value) =>
        setAppearanceValue(
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
      reset={
        preferences.value.appearance.colors?.[key] === undefined
          ? undefined
          : () => setColor(key)
      }
      change={(value) => setColor(key, String(value))}
    />
  );
  return (
    <main className="terminal-settings-page">
      <header className="settings-page-heading">
        <div>
          <h1>Terminal</h1>
          <p>Make every terminal feel like yours.</p>
        </div>
        <button
          className="button"
          disabled={!preferences.ready || busy}
          onClick={() => void persist(defaultTerminalPreferences)}
        >
          <RotateCcw size={14} /> Reset defaults
        </button>
      </header>
      <p className="settings-help">
        Changes save automatically. Appearance and behavior apply to open
        terminals. Appearance follows your theme until you override a setting.
        Reset restores theme defaults.
      </p>
      {(error || preferences.error) && (
        <div className="keybindings-error" role="alert">
          <span>{preferences.error || error}</span>
          {preferences.error && (
            <button
              className="text-button"
              onClick={() => {
                setError("");
                void preferences.reload();
              }}
            >
              Retry loading
            </button>
          )}
        </div>
      )}
      <div className="keybindings-status" role="status">
        {!preferences.ready ? "Loading terminal settings…" : status}
      </div>
      <section className="keybindings-group" aria-label="Titles">
        <h2>Titles</h2>
        <Setting
          name="alwaysShowTitles"
          value={preferences.value.alwaysShowTitles}
          disabled={disabled}
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
      </section>
      <section className="keybindings-group" aria-label="Notifications">
        <h2>Notifications</h2>
        <Setting
          name="agentNotifications"
          value={preferences.value.agentNotifications}
          disabled={disabled}
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
        <div className="terminal-setting-row">
          <div className="keybinding-label">
            <span>Claude Code integration</span>
            <small>
              Review and approve the configuration in the main window. Existing
              settings are preserved with a backup.
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
      {windows && (
        <section className="keybindings-group" aria-label="Shell">
          <h2>Shell</h2>
          <Setting
            name="windowsShell"
            value={preferences.value.windowsShell}
            choices={["powershell", "cmd"]}
            disabled={disabled}
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
        </section>
      )}
      {preferences.ready && <TerminalPreview />}
      <datalist id="terminal-font-families">
        <option value='"JetBrains Mono", monospace' />
        <option value="monospace" />
        <option value='"Fira Code", monospace' />
        <option value='"Cascadia Code", monospace' />
        <option value='"DejaVu Sans Mono", monospace' />
        <option value='"Liberation Mono", monospace' />
      </datalist>
      <section className="keybindings-group" aria-label="Font">
        <h2>Font</h2>
        {(
          [
            "fontFamily",
            "fontSize",
            "fontWeight",
            "fontWeightBold",
            "lineHeight",
            "letterSpacing",
          ] as const
        ).map(appearanceSetting)}
      </section>
      <section className="keybindings-group" aria-label="Cursor">
        <h2>Cursor</h2>
        {(
          [
            "cursorStyle",
            "cursorInactiveStyle",
            "cursorBlink",
            "cursorWidth",
          ] as const
        ).map(appearanceSetting)}
      </section>
      <section className="keybindings-group" aria-label="Colors">
        <h2>Colors</h2>
        <p className="settings-help">
          Choose a color or enter #RRGGBB / #RRGGBBAA, including opacity. Custom
          colors stay the same in light and dark mode.
        </p>
        {terminalColors.slice(0, 7).map(colorSetting)}
        <details className="terminal-settings-details">
          <DisclosureSummary>ANSI palette and search colors</DisclosureSummary>
          {terminalColors.slice(7).map(colorSetting)}
        </details>
      </section>
      <details className="terminal-settings-details">
        <DisclosureSummary>Advanced</DisclosureSummary>
        {(["minimumContrastRatio", "drawBoldTextInBrightColors"] as const).map(
          appearanceSetting,
        )}
        {(
          Object.keys(
            terminalBehaviorDefaults,
          ) as (keyof TerminalPreferences["behavior"])[]
        ).map((key) => (
          <Setting
            key={key}
            name={key}
            disabled={disabled}
            value={preferences.value.behavior[key]}
            range={
              terminalBehaviorNumbers[
                key as keyof typeof terminalBehaviorNumbers
              ]
            }
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
        ))}
      </details>
    </main>
  );
}
