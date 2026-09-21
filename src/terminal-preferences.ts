import { parseLegacyTheme as parseTheme } from "./theme/format.ts";
import type { ThemeTerminal } from "./theme/format.ts";
import type { AppInfo } from "./model.ts";

export const terminalBehaviorDefaults = {
  scrollback: 10_000,
  scrollSensitivity: 1,
  fastScrollSensitivity: 5,
  smoothScrollDuration: 0,
  tabStopWidth: 8,
  scrollOnUserInput: true,
  scrollOnEraseInDisplay: false,
  altClickMovesCursor: true,
  rightClickSelectsWord: false,
  macOptionIsMeta: false,
  macOptionClickForcesSelection: false,
  screenReaderMode: false,
  customGlyphs: true,
  rescaleOverlappingGlyphs: false,
  wordSeparator: " ()[]{}',\"`",
};
export const terminalBehaviorNumbers = {
  scrollback: [0, 100_000],
  scrollSensitivity: [0.1, 100],
  fastScrollSensitivity: [0.1, 100],
  smoothScrollDuration: [0, 1000],
  tabStopWidth: [1, 32],
} as const;
export interface TerminalPreferences {
  appearance: ThemeTerminal;
  behavior: typeof terminalBehaviorDefaults;
  windowsShell: "powershell" | "cmd";
  agentNotifications: boolean;
  alwaysShowTitles: boolean;
}
export const defaultTerminalPreferences: TerminalPreferences = {
  appearance: {},
  behavior: { ...terminalBehaviorDefaults },
  windowsShell: "powershell",
  agentNotifications: true,
  alwaysShowTitles: false,
};
export const terminalColorPattern = /^#[\da-f]{6}([\da-f]{2})?$/i;

export function defaultTerminalProfile(
  info: AppInfo,
  windowsShell: TerminalPreferences["windowsShell"],
): string {
  const candidates =
    info.platform !== "windows"
      ? []
      : windowsShell === "cmd"
        ? ["local:cmd"]
        : ["local:pwsh", "local:powershell"];
  return (
    candidates.find((id) =>
      info.profiles.some((profile) => profile.id === id),
    ) ??
    info.profiles[0]?.id ??
    ""
  );
}

export function restoreTerminalPreferences(
  value: unknown,
): TerminalPreferences {
  if (value === null) return structuredClone(defaultTerminalPreferences);
  try {
    if (!value || typeof value !== "object" || Array.isArray(value))
      throw new Error("Expected an object.");
    const data = value as Record<string, unknown>;
    if (
      data.version !== 1 ||
      Object.keys(data).some(
        (key) =>
          ![
            "version",
            "appearance",
            "behavior",
            "windowsShell",
            "agentNotifications",
            "alwaysShowTitles",
          ].includes(key),
      )
    )
      throw new Error("Unsupported format.");
    if (
      data.windowsShell !== undefined &&
      data.windowsShell !== "powershell" &&
      data.windowsShell !== "cmd"
    )
      throw new Error("Invalid Windows shell.");
    if (
      data.agentNotifications !== undefined &&
      typeof data.agentNotifications !== "boolean"
    )
      throw new Error("Invalid agent notifications setting.");
    if (
      data.alwaysShowTitles !== undefined &&
      typeof data.alwaysShowTitles !== "boolean"
    )
      throw new Error("Invalid terminal title visibility setting.");
    if (
      !data.appearance ||
      !data.behavior ||
      typeof data.behavior !== "object" ||
      Array.isArray(data.behavior)
    )
      throw new Error("Missing terminal settings.");
    const appearance = parseTheme({
      version: 1,
      name: "Terminal",
      terminal: data.appearance,
    }).terminal!;
    if (
      appearance.fontFamily !== undefined &&
      (appearance.fontFamily.length > 500 ||
        /[\x00-\x1f\x7f;{}<>]/.test(appearance.fontFamily))
    )
      throw new Error("Invalid font family.");
    if (
      appearance.cursorWidth !== undefined &&
      !Number.isInteger(appearance.cursorWidth)
    )
      throw new Error("Cursor width must be a whole number.");
    for (const color of Object.values(appearance.colors ?? {}))
      if (!terminalColorPattern.test(color))
        throw new Error("Use #RRGGBB or #RRGGBBAA colors.");
    const behavior = data.behavior as Record<string, unknown>;
    for (const key of Object.keys(behavior))
      if (!Object.hasOwn(terminalBehaviorDefaults, key))
        throw new Error(`Unknown terminal setting: ${key}.`);
    for (const [key, fallback] of Object.entries(terminalBehaviorDefaults)) {
      const entry = behavior[key];
      if (typeof entry !== typeof fallback) throw new Error(`Invalid ${key}.`);
      if (typeof entry === "number") {
        const [min, max] =
          terminalBehaviorNumbers[key as keyof typeof terminalBehaviorNumbers];
        if (
          !Number.isFinite(entry) ||
          entry < min ||
          entry > max ||
          (!["scrollSensitivity", "fastScrollSensitivity"].includes(key) &&
            !Number.isInteger(entry))
        )
          throw new Error(`${key} must be between ${min} and ${max}.`);
      }
    }
    if (
      (behavior.wordSeparator as string).length > 200 ||
      /[\x00-\x1f\x7f]/.test(behavior.wordSeparator as string)
    )
      throw new Error("Invalid word separators.");
    return {
      appearance: structuredClone(appearance),
      behavior: { ...behavior } as TerminalPreferences["behavior"],
      windowsShell: data.windowsShell ?? "powershell",
      agentNotifications: data.agentNotifications ?? true,
      alwaysShowTitles: data.alwaysShowTitles ?? false,
    };
  } catch (error) {
    throw new Error(
      `Invalid or unsupported terminal settings (${error instanceof Error ? error.message : String(error)}). The file has been left intact. Retry loading or reset defaults to replace it.`,
    );
  }
}
