import { t } from "../i18n";
import type { ITheme } from "@xterm/xterm";
const fields = [
  "foreground",
  "background",
  "cursor",
  "cursorAccent",
  "selectionBackground",
  "selectionForeground",
  "selectionInactiveBackground",
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const;
export function normalizePalette(value: unknown): ITheme {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error(t("Invalid terminal palette."));
  const source = value as Record<string, unknown>;
  const result: ITheme = {};
  for (const key of fields) {
    const color =
      source[key] ??
      (key === "selectionBackground" ? source.selection : undefined);
    if (color === undefined) continue;
    if (
      typeof color !== "string" ||
      !/^#[\da-f]{6}(?:[\da-f]{2})?$/i.test(color)
    )
      throw new Error(t("Invalid palette color: {key}", { key }));
    result[key] = color;
  }
  if (!result.foreground || !result.background)
    throw new Error(
      t("A terminal palette needs foreground and background colors."),
    );
  return result;
}
export async function terminalPresets() {
  const { default: palettes } = await import("xterm-theme");
  return Object.fromEntries(
    Object.entries(palettes)
      .filter(([name]) => name !== "default")
      .map(([name, palette]) => [name, normalizePalette(palette)]),
  );
}
