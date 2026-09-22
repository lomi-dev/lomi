import type { IconKind, IconTheme } from "./icon-theme";
import deepmono from "../../themes/deepmono.json" with { type: "json" };
import lomi from "../../themes/lomi.json" with { type: "json" };
import { parseVSCodeTheme, vscodeValues, type VSCodeTheme } from "./vscode.ts";
import {
  parseTree,
  getNodeValue,
  printParseErrorCode,
  getNodePath,
  modify,
  applyEdits,
  format,
  type Node,
  type ParseError,
} from "jsonc-parser";
export const backgroundAreas = [
  "app",
  "terminal",
  "sidebar",
  "titlebar",
  "statusbar",
  "settings",
  "modal",
] as const;
export type BackgroundArea = (typeof backgroundAreas)[number];
export interface ThemeBackground {
  image?: string;
  opacity?: number;
  size?: string;
  position?: string;
  repeat?: "no-repeat" | "repeat" | "repeat-x" | "repeat-y" | "space" | "round";
  blur?: number;
  overlay?: string;
  blendMode?: string;
}
export const terminalColors = [
  "background",
  "foreground",
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
  "searchMatchBackground",
  "searchActiveMatchBackground",
  "searchMatchBorder",
  "searchActiveMatchBorder",
] as const;
export const terminalNumbers = {
  fontSize: [6, 72],
  lineHeight: [1, 3],
  letterSpacing: [-2, 20],
  cursorWidth: [1, 10],
  minimumContrastRatio: [1, 21],
} as const;
export const terminalEnums = {
  cursorStyle: ["bar", "block", "underline"],
  cursorInactiveStyle: ["outline", "bar", "block", "underline", "none"],
} as const;
export const terminalBooleans = [
  "cursorBlink",
  "drawBoldTextInBrightColors",
] as const;
export interface ThemeTerminal {
  fontFamily?: string;
  fontSize?: number;
  lineHeight?: number;
  letterSpacing?: number;
  fontWeight?: string | number;
  fontWeightBold?: string | number;
  cursorWidth?: number;
  minimumContrastRatio?: number;
  cursorStyle?: "bar" | "block" | "underline";
  cursorInactiveStyle?: "outline" | "bar" | "block" | "underline" | "none";
  cursorBlink?: boolean;
  drawBoldTextInBrightColors?: boolean;
  colors?: Partial<Record<(typeof terminalColors)[number], string>>;
}
export interface LegacyTheme {
  $schema?: string;
  version: 1;
  name: string;
  author?: string;
  description?: string;
  appearance?: "dark" | "light";
  layout?: {
    tabs?: "inline" | "above" | "below";
    statusbar?: "top" | "bottom";
    settingsNavigation?: "left" | "right" | "top" | "bottom";
  };
  tokens?: Record<string, string>;
  styles?: Record<string, Record<string, string>>;
  assets?: Record<string, string>;
  backgrounds?: Partial<Record<BackgroundArea, ThemeBackground>>;
  terminal?: ThemeTerminal;
  stylesheets?: string[];
  stylesheet?: string;
}
export type Appearance = "light" | "dark";
export type AppearancePreference = "system" | Appearance;

export function resolveAppearance(
  preference: AppearancePreference,
  system: Appearance,
): Appearance {
  return preference === "system" ? system : preference;
}

export interface ThemePreferences {
  version: 1;
  active: string | null;
  fileIcons?: string | null;
  productIcons?: string | null;
  appearance: AppearancePreference;
}
export interface ThemeBundle {
  id: string;
  raw: string;
  iconTheme?: IconTheme;
  revision: string;
  directory: string;
  readOnly?: boolean;
  migration?: string[];
}
export interface ThemeCurrent {
  preferences: ThemePreferences;
  theme: ThemeBundle | null;
  fileIcons?: ThemeBundle | null;
  productIcons?: ThemeBundle | null;
  safeMode: boolean;
  revision: number;
}
export interface ThemeEntry {
  kind?: "color" | IconKind;
  id: string;
  name: string;
  author: string;
  description: string;
  error: string | null;
  owner?: string | null;
}
export interface ThemeCatalog {
  directory: string;
  themes: ThemeEntry[];
}
export const builtinPreferences: ThemePreferences = {
  version: 1,
  active: null,
  appearance: "system",
};
export const kebab = (value: string) =>
  value.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);

function object(value: unknown, path: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error(`${path} must be an object.`);
  return value as Record<string, unknown>;
}
function keys(
  value: Record<string, unknown>,
  allowed: readonly string[],
  path: string,
) {
  for (const key of Object.keys(value))
    if (!allowed.includes(key))
      throw new Error(`Unknown ${path} field: ${key}`);
}
function string(
  value: unknown,
  path: string,
  limit = 2000,
): asserts value is string {
  if (
    typeof value !== "string" ||
    !value.trim() ||
    value.length > limit ||
    /[\x00-\x08\x0b\x0c\x0e-\x1f]/.test(value)
  )
    throw new Error(
      `${path} must be a nonempty string of at most ${limit} characters.`,
    );
}
function number(value: unknown, path: string, min: number, max: number) {
  if (
    typeof value !== "number" ||
    !Number.isFinite(value) ||
    value < min ||
    value > max
  )
    throw new Error(`${path} must be between ${min} and ${max}.`);
}
export function relativeAsset(value: unknown): asserts value is string {
  string(value, "Asset path", 1024);
  if (
    /[\\:%?#<>"|*\x00-\x1f]/.test(value) ||
    value
      .split("/")
      .some(
        (part) =>
          !part ||
          part === "." ||
          part === ".." ||
          /[. ]$/.test(part) ||
          /^(con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\.|$)/i.test(part),
      )
  )
    throw new Error(`Use a relative path inside the theme folder: ${value}`);
}
export function parseLegacyTheme(value: unknown): LegacyTheme {
  const data = object(value, "theme.json");
  keys(
    data,
    [
      "$schema",
      "version",
      "name",
      "author",
      "description",
      "appearance",
      "layout",
      "tokens",
      "styles",
      "assets",
      "backgrounds",
      "terminal",
      "stylesheet",
      "stylesheets",
    ],
    "theme",
  );
  if (data.version !== 1)
    throw new Error("Unsupported theme version. Expected version 1.");
  string(data.name, "name", 160);
  for (const field of ["$schema", "author", "description"])
    if (data[field] !== undefined) string(data[field], field);
  if (
    data.appearance !== undefined &&
    !["dark", "light"].includes(data.appearance as string)
  )
    throw new Error("appearance must be dark or light.");
  if (data.layout !== undefined) {
    const layout = object(data.layout, "layout");
    const options = {
      tabs: ["inline", "above", "below"],
      statusbar: ["top", "bottom"],
      settingsNavigation: ["left", "right", "top", "bottom"],
    };
    keys(layout, Object.keys(options), "layout");
    for (const [key, values] of Object.entries(options))
      if (layout[key] !== undefined && !values.includes(layout[key] as string))
        throw new Error(`Invalid layout.${key}.`);
  }
  if (data.stylesheet !== undefined && data.stylesheets !== undefined)
    throw new Error(
      "Use stylesheets or the legacy stylesheet field, not both.",
    );
  if (data.stylesheets !== undefined && !Array.isArray(data.stylesheets))
    throw new Error("stylesheets must be an array of relative CSS paths.");
  const stylesheets =
    (data.stylesheets as unknown[] | undefined) ??
    (data.stylesheet !== undefined ? [data.stylesheet] : []);
  for (const path of stylesheets) {
    relativeAsset(path);
    if (!path.endsWith(".css"))
      throw new Error("Stylesheet paths must name CSS files.");
  }
  if (new Set(stylesheets).size !== stylesheets.length)
    throw new Error("stylesheets must not contain duplicate paths.");
  for (const [name, token] of Object.entries(
    object(data.tokens ?? {}, "tokens"),
  )) {
    if (!/^--[a-z][a-z0-9-]*$/.test(name))
      throw new Error(`Invalid CSS token: ${name}`);
    string(token, `tokens.${name}`, 4000);
  }
  for (const [name, path] of Object.entries(
    object(data.assets ?? {}, "assets"),
  )) {
    if (!/^[a-z][a-z0-9-]*$/.test(name))
      throw new Error(`Invalid asset name: ${name}`);
    relativeAsset(path);
  }
  for (const [selector, declarations] of Object.entries(
    object(data.styles ?? {}, "styles"),
  )) {
    string(selector, "CSS selector", 1000);
    for (const [property, value] of Object.entries(
      object(declarations, `styles.${selector}`),
    )) {
      if (!/^(--)?[a-z][a-z0-9-]*$/.test(property))
        throw new Error(`Use kebab-case CSS property names: ${property}`);
      string(value, `styles.${selector}.${property}`, 4000);
    }
  }
  const backgrounds = object(data.backgrounds ?? {}, "backgrounds");
  keys(backgrounds, backgroundAreas, "backgrounds");
  for (const [area, value] of Object.entries(backgrounds)) {
    const background = object(value, `backgrounds.${area}`);
    keys(
      background,
      [
        "image",
        "opacity",
        "size",
        "position",
        "repeat",
        "blur",
        "overlay",
        "blendMode",
      ],
      `backgrounds.${area}`,
    );
    if (background.image !== undefined) relativeAsset(background.image);
    if (background.opacity !== undefined)
      number(background.opacity, `${area}.opacity`, 0, 1);
    if (background.blur !== undefined)
      number(background.blur, `${area}.blur`, 0, 100);
    for (const key of ["size", "position", "repeat", "overlay", "blendMode"])
      if (background[key] !== undefined)
        string(background[key], `${area}.${key}`);
  }
  const terminal = object(data.terminal ?? {}, "terminal");
  keys(
    terminal,
    [
      "colors",
      "fontFamily",
      "fontWeight",
      "fontWeightBold",
      ...Object.keys(terminalNumbers),
      ...Object.keys(terminalEnums),
      ...terminalBooleans,
    ],
    "terminal",
  );
  for (const [key, [min, max]] of Object.entries(terminalNumbers))
    if (terminal[key] !== undefined)
      number(terminal[key], `terminal.${key}`, min, max);
  for (const [key, values] of Object.entries(terminalEnums))
    if (
      terminal[key] !== undefined &&
      !(values as readonly unknown[]).includes(terminal[key])
    )
      throw new Error(`Invalid terminal.${key}.`);
  for (const key of terminalBooleans)
    if (terminal[key] !== undefined && typeof terminal[key] !== "boolean")
      throw new Error(`terminal.${key} must be a boolean.`);
  if (terminal.fontFamily !== undefined)
    string(terminal.fontFamily, "terminal.fontFamily");
  for (const key of ["fontWeight", "fontWeightBold"])
    if (
      terminal[key] !== undefined &&
      !["normal", "bold"].includes(terminal[key] as string)
    )
      number(terminal[key], `terminal.${key}`, 1, 1000);
  const colors = object(terminal.colors ?? {}, "terminal.colors");
  keys(colors, terminalColors, "terminal.colors");
  for (const [key, value] of Object.entries(colors))
    string(value, `terminal.colors.${key}`);
  return data as unknown as LegacyTheme;
}

export const syntaxNames = [
  "keyword",
  "string",
  "number",
  "comment",
  "type",
  "function",
  "variable",
  "property",
  "operator",
  "punctuation",
  "heading",
  "strong",
  "emphasis",
  "link",
  "invalid",
] as const;
export type SyntaxName = (typeof syntaxNames)[number];
export interface SyntaxStyle {
  color?: string;
  fontStyle?: "normal" | "italic";
  fontWeight?: "normal" | "bold" | number;
  textDecoration?: "none" | "underline";
}
export interface ThemeEditorAppearance {
  colors?: Partial<
    Record<
      | "background"
      | "foreground"
      | "gutterBackground"
      | "gutterForeground"
      | "activeLine"
      | "selection"
      | "cursor",
      string
    >
  >;
  fontFamily?: string;
  fontSize?: number;
  syntax?: Partial<Record<SyntaxName, SyntaxStyle>>;
}
export interface ThemeValues {
  tokens?: Record<string, string>;
  styles?: LegacyTheme["styles"];
  backgrounds?: LegacyTheme["backgrounds"];
  layout?: LegacyTheme["layout"];
  terminal?: ThemeTerminal & { preset?: string };
  editor?: ThemeEditorAppearance;
  plugins?: Record<string, Record<string, string>>;
}
export interface ThemeManifest {
  $schema?: string;
  version: 2;
  name: string;
  author?: string;
  description?: string;
  appearance?: "adaptive" | Appearance;
  common?: ThemeValues;
  light?: ThemeValues;
  dark?: ThemeValues;
  resources?: { assets?: Record<string, string>; stylesheets?: string[] };
  vscode?: VSCodeTheme;
  iconTheme?: { kind: IconKind; path: string };
}
export type ResolvedTheme = ThemeValues & {
  assets?: Record<string, string>;
  stylesheets?: string[];
};
export const builtinTheme = lomi as ThemeManifest;
export const deepmonoTheme = deepmono as ThemeManifest;
export const deepmonoThemeId = "@builtin-deepmono";
export const builtinThemes = [
  { id: null, manifest: builtinTheme },
  { id: deepmonoThemeId, manifest: deepmonoTheme },
] as const;
export function isBuiltinTheme(id: string | null) {
  return builtinThemes.some((theme) => theme.id === id);
}
export function color(value: unknown, path: string) {
  if (typeof value === "string") {
    const mix =
      /^color-mix\(\s*in srgb,\s*var\(--[a-z][a-z0-9-]*\)\s+(\d+(?:\.\d+)?)%,\s*transparent\s*\)$/.exec(
        value,
      );
    if (mix && Number(mix[1]) <= 100) return;
  }
  if (
    typeof value !== "string" ||
    !/^(#[\da-f]{3,4}|#[\da-f]{6}(?:[\da-f]{2})?|var\(--[a-z][a-z0-9-]*\)|transparent)$/i.test(
      value,
    )
  )
    throw new Error(
      `${path} must be a hex color, transparent, or a semantic token reference.`,
    );
}
function validateValues(value: unknown, path: string) {
  const data = object(value, path);
  keys(
    data,
    [
      "tokens",
      "styles",
      "backgrounds",
      "layout",
      "terminal",
      "editor",
      "plugins",
    ],
    path,
  );
  const { editor, plugins, terminal, ...surface } = data;
  const term = object(terminal ?? {}, `${path}.terminal`);
  const { preset, ...options } = term;
  if (preset !== undefined) string(preset, `${path}.terminal.preset`, 160);
  parseLegacyTheme({
    version: 1,
    name: "Surface",
    ...surface,
    terminal: options,
  });
  for (const [key, value] of Object.entries(
    object(term.colors ?? {}, `${path}.terminal.colors`),
  ))
    color(value, `${path}.terminal.colors.${key}`);
  for (const [key, value] of Object.entries(
    object(data.tokens ?? {}, `${path}.tokens`),
  ))
    if (key.startsWith("--color-")) color(value, `${path}.tokens.${key}`);
  const edit = object(editor ?? {}, `${path}.editor`);
  keys(edit, ["colors", "fontFamily", "fontSize", "syntax"], `${path}.editor`);
  if (edit.fontFamily !== undefined)
    string(edit.fontFamily, `${path}.editor.fontFamily`);
  if (edit.fontSize !== undefined)
    number(edit.fontSize, `${path}.editor.fontSize`, 6, 72);
  const colors = object(edit.colors ?? {}, `${path}.editor.colors`);
  keys(
    colors,
    [
      "background",
      "foreground",
      "gutterBackground",
      "gutterForeground",
      "activeLine",
      "selection",
      "cursor",
    ],
    `${path}.editor.colors`,
  );
  for (const [key, value] of Object.entries(colors))
    color(value, `${path}.editor.colors.${key}`);
  const syntax = object(edit.syntax ?? {}, `${path}.editor.syntax`);
  keys(syntax, syntaxNames, `${path}.editor.syntax`);
  for (const [key, value] of Object.entries(syntax)) {
    const style = object(value, `${path}.editor.syntax.${key}`);
    keys(
      style,
      ["color", "fontStyle", "fontWeight", "textDecoration"],
      `${path}.editor.syntax.${key}`,
    );
    if (style.color !== undefined)
      color(style.color, `${path}.editor.syntax.${key}.color`);
    for (const [name, values] of Object.entries({
      fontStyle: ["normal", "italic"],
      fontWeight: ["normal", "bold"],
      textDecoration: ["none", "underline"],
    }))
      if (
        style[name] !== undefined &&
        !(
          name === "fontWeight" &&
          typeof style[name] === "number" &&
          style[name] >= 100 &&
          style[name] <= 900 &&
          style[name] % 100 === 0
        ) &&
        !values.includes(style[name] as string)
      )
        throw new Error(`Invalid ${path}.editor.syntax.${key}.${name}.`);
  }
  for (const [owner, tokens] of Object.entries(
    object(plugins ?? {}, `${path}.plugins`),
  )) {
    if (!/^[a-z][a-z0-9-]*(?:\.[a-z][a-z0-9-]*)+$/.test(owner))
      throw new Error(`Invalid plugin namespace: ${owner}`);
    for (const [key, value] of Object.entries(
      object(tokens, `${path}.plugins.${owner}`),
    )) {
      if (!/^[a-z][a-z0-9-]*$/.test(key))
        throw new Error(`Invalid plugin token: ${key}`);
      string(value, `${path}.plugins.${owner}.${key}`, 4000);
    }
  }
}
export function parseTheme(value: unknown): ThemeManifest {
  const data = object(value, "theme.jsonc");
  keys(
    data,
    [
      "$schema",
      "version",
      "name",
      "author",
      "description",
      "appearance",
      "common",
      "light",
      "dark",
      "resources",
      "vscode",
      "iconTheme",
    ],
    "theme",
  );
  if (data.version !== 2)
    throw new Error(
      "Unsupported theme version. Expected version 2; import a legacy package to migrate version 1.",
    );
  string(data.name, "name", 160);
  for (const field of ["$schema", "author", "description"])
    if (data[field] !== undefined) string(data[field], field);
  if (
    data.appearance !== undefined &&
    !["adaptive", "light", "dark"].includes(data.appearance as string)
  )
    throw new Error("appearance must be adaptive, light, or dark.");
  for (const field of ["common", "light", "dark"])
    if (data[field] !== undefined) validateValues(data[field], field);
  if (data.iconTheme !== undefined) {
    const icons = object(data.iconTheme, "iconTheme");
    keys(icons, ["kind", "path"], "iconTheme");
    if (icons.kind !== "file" && icons.kind !== "product")
      throw new Error("Unknown icon theme kind.");
    relativeAsset(icons.path);
    if (
      ["common", "light", "dark", "resources", "vscode"].some(
        (field) => data[field] !== undefined,
      )
    )
      throw new Error("Icon themes cannot also define color surfaces.");
  }
  if (data.vscode !== undefined) parseVSCodeTheme(data.vscode);
  const resources = object(data.resources ?? {}, "resources");
  keys(resources, ["assets", "stylesheets"], "resources");
  parseLegacyTheme({ version: 1, name: "Resources", ...resources });
  return data as unknown as ThemeManifest;
}
function jsonTree(raw: string, file: string) {
  if (new TextEncoder().encode(raw).length > 256 * 1024)
    throw new Error(`${file}: exceeds 256 KiB.`);
  const errors: ParseError[] = [];
  const tree = parseTree(raw, errors, {
    allowTrailingComma: true,
    disallowComments: false,
    allowEmptyContent: false,
  });
  const fail = (offset: number, message: string): never => {
    const before = raw.slice(0, offset);
    const line = before.split("\n").length,
      column = offset - before.lastIndexOf("\n");
    throw new Error(`${file}:${line}:${column}: ${message}`);
  };
  if (errors.length)
    fail(errors[0].offset, printParseErrorCode(errors[0].error));
  if (!tree) fail(0, "Expected a theme object.");
  const visit = (node: Node, depth: number) => {
    if (depth > 32) fail(node.offset, "Theme nesting exceeds 32 levels.");
    if (node.type === "object") {
      const seen = new Set<string>();
      for (const property of node.children ?? []) {
        const key = property.children![0];
        if (seen.has(key.value))
          fail(
            key.offset,
            `Duplicate key ${[...getNodePath(node), key.value].join(".")}`,
          );
        seen.add(key.value);
      }
    }
    for (const child of node.children ?? [])
      visit(child, depth + (node.type === "property" ? 0 : 1));
  };
  visit(tree!, 0);
  return tree!;
}
export function readThemeDraft(raw: string): unknown {
  return getNodeValue(jsonTree(raw, "theme.jsonc"));
}
export function parseThemeText(
  raw: string,
  file = "theme.jsonc",
): ThemeManifest {
  const tree = jsonTree(raw, file);
  try {
    return parseTheme(getNodeValue(tree));
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    let match: Node = tree;
    const visit = (node: Node) => {
      const path = getNodePath(node).join(".");
      if (
        path &&
        message.includes(path) &&
        path.length > getNodePath(match).join(".").length
      )
        match = node;
      for (const child of node.children ?? []) visit(child);
    };
    visit(tree);
    const before = raw.slice(0, match.offset);
    throw new Error(
      `${file}:${before.split("\n").length}:${match.offset - before.lastIndexOf("\n")}: ${message}`,
    );
  }
}
export function editThemeText(
  raw: string,
  path: (string | number)[],
  value: unknown,
) {
  jsonTree(raw, "theme.jsonc");
  return applyEdits(
    raw,
    modify(raw, path, value, {
      formattingOptions: {
        insertSpaces: true,
        tabSize: 2,
        eol: raw.includes("\r\n") ? "\r\n" : "\n",
      },
    }),
  );
}
export function formatThemeText(raw: string, tabSize = 2, insertSpaces = true) {
  jsonTree(raw, "theme.jsonc");
  return applyEdits(
    raw,
    format(raw, undefined, {
      tabSize,
      insertSpaces,
      eol: raw.includes("\r\n") ? "\r\n" : "\n",
    }),
  );
}
export function migrateTheme(value: unknown): {
  manifest: ThemeManifest;
  report: string[];
} {
  const old = parseLegacyTheme(value);
  const {
    version: _,
    name,
    author,
    description,
    appearance,
    $schema: _schema,
    assets,
    stylesheet,
    stylesheets,
    ...common
  } = old;
  return {
    manifest: parseTheme({
      version: 2,
      name,
      ...(author ? { author } : {}),
      ...(description ? { description } : {}),
      appearance: appearance ?? "adaptive",
      common,
      resources: {
        ...(assets ? { assets } : {}),
        ...(stylesheets || stylesheet
          ? { stylesheets: stylesheets ?? [stylesheet!] }
          : {}),
      },
    }),
    report: [
      "Converted version 1 to version 2. The original theme.json is preserved; saving creates theme.jsonc.",
    ],
  };
}
function mergeValues(
  common: ThemeValues = {},
  variant: ThemeValues = {},
): ResolvedTheme {
  const result = { ...common, ...variant };
  for (const field of ["tokens", "styles", "backgrounds", "layout"] as const)
    result[field] = { ...common[field], ...variant[field] } as never;
  result.terminal = {
    ...common.terminal,
    ...variant.terminal,
    colors: { ...common.terminal?.colors, ...variant.terminal?.colors },
  };
  result.editor = {
    ...common.editor,
    ...variant.editor,
    colors: { ...common.editor?.colors, ...variant.editor?.colors },
    syntax: { ...common.editor?.syntax, ...variant.editor?.syntax },
  };
  for (const field of ["styles", "backgrounds"] as const)
    for (const [key, value] of Object.entries(variant[field] ?? {}))
      result[field]![key as never] = {
        ...(common[field] as Record<string, object> | undefined)?.[key],
        ...value,
      } as never;
  for (const [key, value] of Object.entries(variant.editor?.syntax ?? {}))
    result.editor.syntax![key as SyntaxName] = {
      ...common.editor?.syntax?.[key as SyntaxName],
      ...value,
    };
  result.plugins = { ...common.plugins };
  for (const [owner, tokens] of Object.entries(variant.plugins ?? {}))
    result.plugins[owner] = { ...result.plugins[owner], ...tokens };
  return result;
}
export function resolveTheme(
  manifest: ThemeManifest,
  appearance: Appearance,
): ResolvedTheme {
  const values = mergeValues(
    manifest.vscode ? vscodeValues(manifest.vscode) : {},
    mergeValues(manifest.common, manifest[appearance]),
  );
  return { ...values, ...manifest.resources };
}
