import type { SyntaxName, SyntaxStyle, ThemeValues } from "./format.ts";

export interface TokenRule {
  name?: string;
  scope?: string | string[];
  settings: { foreground?: string; background?: string; fontStyle?: string };
}
export interface VSCodeTheme {
  name?: string;
  type?: "dark" | "light" | "hcDark" | "hcLight";
  colors?: Record<string, string | null>;
  tokenColors?: TokenRule[];
  semanticHighlighting?: boolean;
  semanticTokenColors?: Record<
    string,
    | string
    | {
        foreground?: string;
        fontStyle?: string;
        bold?: boolean;
        italic?: boolean;
        underline?: boolean;
        strikethrough?: boolean;
      }
  >;
  [key: string]: unknown;
}
const hex = /^#(?:[\da-f]{3,4}|[\da-f]{6}|[\da-f]{8})$/i;
function record(value: unknown, path: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error(`${path} must be an object.`);
  return value as Record<string, unknown>;
}
function checkColor(value: unknown, path: string) {
  if (typeof value !== "string" || !hex.test(value))
    throw new Error(`${path} must be a hexadecimal color.`);
}
export function parseVSCodeTheme(value: unknown): VSCodeTheme {
  const data = record(value, "vscode");
  if (data.include !== undefined || typeof data.tokenColors === "string")
    throw new Error(
      "Import the VS Code file or extension to resolve include and TextMate paths first.",
    );
  if (
    data.type !== undefined &&
    !["dark", "light", "hcDark", "hcLight"].includes(String(data.type))
  )
    throw new Error("Invalid VS Code theme type.");
  if (
    data.semanticHighlighting !== undefined &&
    typeof data.semanticHighlighting !== "boolean"
  )
    throw new Error("vscode.semanticHighlighting must be a boolean.");
  for (const [key, value] of Object.entries(
    record(data.colors ?? {}, "vscode.colors"),
  )) {
    if (!/^[a-zA-Z][\w.-]*$/.test(key))
      throw new Error(`Invalid VS Code color identifier: ${key}`);
    if (value !== null && value !== "default")
      checkColor(value, `vscode.colors.${key}`);
  }
  const fontStyle = (style: unknown) => {
    if (
      typeof style !== "string" ||
      style
        .split(/\s+/)
        .some(
          (s) =>
            s && !["bold", "italic", "underline", "strikethrough"].includes(s),
        )
    )
      throw new Error("Invalid VS Code token fontStyle.");
  };
  if (data.tokenColors !== undefined) {
    if (!Array.isArray(data.tokenColors))
      throw new Error("vscode.tokenColors must be an array.");
    for (const rule of data.tokenColors) {
      const token = record(rule, "vscode.tokenColors rule");
      if (
        token.scope !== undefined &&
        typeof token.scope !== "string" &&
        !(
          Array.isArray(token.scope) &&
          token.scope.every((s) => typeof s === "string")
        )
      )
        throw new Error(
          "TextMate scope must be a string or an array of strings.",
        );
      const settings = record(token.settings, "TextMate settings");
      for (const name of ["foreground", "background"])
        if (settings[name] !== undefined)
          checkColor(settings[name], `TextMate ${name}`);
      if (settings.fontStyle !== undefined) fontStyle(settings.fontStyle);
    }
  }
  for (const value of Object.values(
    record(data.semanticTokenColors ?? {}, "vscode.semanticTokenColors"),
  )) {
    if (typeof value === "string") checkColor(value, "Semantic token color");
    else {
      const style = record(value, "Semantic token style");
      if (style.foreground !== undefined)
        checkColor(style.foreground, "Semantic token foreground");
      if (style.fontStyle !== undefined) fontStyle(style.fontStyle);
      for (const name of ["bold", "italic", "underline", "strikethrough"])
        if (style[name] !== undefined && typeof style[name] !== "boolean")
          throw new Error(`Semantic token ${name} must be a boolean.`);
    }
  }
  return data as VSCodeTheme;
}

// A single mapping drives import, export, and the compatibility report.
export const workbenchColors: Record<string, string[]> = {
  "editor.background": [
    "--color-background",
    "--settings-background",
    "--editor-background",
    "--editor-gutter-background",
    "--terminal-background",
  ],
  foreground: ["--color-background-text", "--color-surface-text"],
  "editor.foreground": ["--editor-foreground", "--terminal-foreground"],
  descriptionForeground: ["--color-surface-variant-text", "--color-muted-text"],
  focusBorder: ["--color-primary", "--color-accent-text"],
  errorForeground: ["--color-error"],
  "editorWarning.foreground": ["--color-warning"],
  "editorInfo.foreground": ["--color-info"],
  "widget.border": ["--color-outline"],
  "sideBar.background": ["--sidebar-background", "--color-surface"],
  "titleBar.activeBackground": ["--titlebar-background"],
  "statusBar.background": ["--statusbar-background"],
  "menu.background": ["--menu-background", "--color-surface-container"],
  "editorWidget.background": ["--modal-background"],
  "input.background": ["--input-background"],
  "button.background": ["--color-primary"],
  "button.foreground": ["--color-primary-text"],
  "button.secondaryBackground": [
    "--button-background",
    "--color-surface-container-high",
  ],
  "list.hoverBackground": ["--color-surface-variant"],
  "list.activeSelectionBackground": ["--color-surface-container-highest"],
  "tab.activeBackground": ["--tab-background-active"],
  "editorLineNumber.foreground": ["--editor-gutter-foreground"],
  "editor.lineHighlightBackground": ["--editor-active-line"],
  "editor.selectionBackground": ["--editor-selection"],
  "editorCursor.foreground": ["--editor-cursor"],
};
export const terminalColorIds: Record<string, string> = {
  background: "terminal.background",
  foreground: "terminal.foreground",
  cursor: "terminalCursor.foreground",
  cursorAccent: "terminalCursor.background",
  selectionBackground: "terminal.selectionBackground",
  selectionForeground: "terminal.selectionForeground",
  selectionInactiveBackground: "terminal.inactiveSelectionBackground",
  searchMatchBackground: "terminal.findMatchHighlightBackground",
  searchActiveMatchBackground: "terminal.findMatchBackground",
  searchMatchBorder: "terminal.findMatchHighlightBorder",
  searchActiveMatchBorder: "terminal.findMatchBorder",
  ...Object.fromEntries(
    [
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
    ].map((name) => [
      name,
      `terminal.ansi${name[0].toUpperCase()}${name.slice(1)}`,
    ]),
  ),
};
export const componentColors: Record<string, [string, string]> = {
  "sideBar.foreground": [".sidebar", "color"],
  "sideBarTitle.foreground": [".sidebar-heading", "color"],
  "sideBar.border": [".sidebar", "border-color"],
  "titleBar.activeForeground": [".titlebar", "color"],
  "titleBar.border": [".titlebar", "border-color"],
  "statusBar.foreground": [".statusbar", "color"],
  "statusBar.border": [".statusbar", "border-color"],
  "statusBarItem.hoverBackground": [
    ".statusbar .icon-button:hover",
    "background-color",
  ],
  "tab.activeForeground": [".tab.active-tab", "color"],
  "tab.inactiveForeground": [".tab:not(.active-tab)", "color"],
  "tab.inactiveBackground": [".tab:not(.active-tab)", "background-color"],
  "tab.hoverBackground": [".tab:hover:not(.active-tab)", "background-color"],
  "tab.border": [".tab", "border-color"],
  "editorGroupHeader.tabsBackground": [".tab-bar", "background-color"],
  "input.foreground": ["input, textarea, .select-control", "color"],
  "input.border": ["input, textarea, .select-control", "border-color"],
  "input.placeholderForeground": [
    "input::placeholder, textarea::placeholder",
    "color",
  ],
  "button.hoverBackground": [
    ".button-primary:hover:not(:disabled)",
    "background-color",
  ],
  "button.secondaryForeground": [".button:not(.button-primary)", "color"],
  "button.secondaryHoverBackground": [
    ".button:not(.button-primary):hover:not(:disabled)",
    "background-color",
  ],
  "menu.foreground": [".menu", "color"],
  "menu.border": [".menu", "border-color"],
  "menu.selectionBackground": [
    ".menu-item:hover:not(:disabled), .menu-item.selected",
    "background-color",
  ],
  "menu.selectionForeground": [
    ".menu-item:hover:not(:disabled), .menu-item.selected",
    "color",
  ],
  "list.hoverForeground": [".tree-row:hover", "color"],
  "list.activeSelectionForeground": [
    ".workspace-item.active-workspace",
    "color",
  ],
  "textLink.foreground": ["a", "color"],
  "scrollbarSlider.background": [
    "::-webkit-scrollbar-thumb",
    "background-color",
  ],
  "scrollbarSlider.hoverBackground": [
    "::-webkit-scrollbar-thumb:hover",
    "background-color",
  ],
  "scrollbarSlider.activeBackground": [
    "::-webkit-scrollbar-thumb:active",
    "background-color",
  ],
  "editorLineNumber.activeForeground": [".cm-activeLineGutter", "color"],
  "editor.inactiveSelectionBackground": [
    ".cm-editor:not(.cm-focused) .cm-selectionBackground",
    "background-color",
  ],
  "editor.findMatchBackground": [
    ".cm-searchMatch.cm-searchMatch-selected",
    "background-color",
  ],
  "editor.findMatchHighlightBackground": [
    ".cm-searchMatch",
    "background-color",
  ],
  "editorBracketMatch.background": [".cm-matchingBracket", "background-color"],
  "editorBracketMatch.border": [".cm-matchingBracket", "outline-color"],
};
export const syntaxScopes: Record<SyntaxName, string[]> = {
  keyword: ["keyword", "storage.type", "storage.modifier"],
  string: ["string", "string.quoted", "string.regexp"],
  number: ["constant.numeric", "constant.language"],
  comment: ["comment", "comment.line", "comment.block"],
  type: [
    "entity.name.type",
    "entity.name.class",
    "support.type",
    "support.class",
  ],
  function: ["entity.name.function", "support.function"],
  variable: ["variable", "variable.other.readwrite"],
  property: ["variable.other.property", "support.type.property-name"],
  operator: ["keyword.operator"],
  punctuation: ["punctuation"],
  heading: ["markup.heading"],
  strong: ["markup.bold"],
  emphasis: ["markup.italic"],
  link: ["markup.underline.link"],
  invalid: ["invalid"],
};
function selectors(rule: TokenRule): string[] {
  return (Array.isArray(rule.scope) ? rule.scope : [rule.scope ?? ""]).flatMap(
    (s) => s.split(",").map((s) => s.trim()),
  );
}
function score(selector: string, scope: string) {
  // Lezer has no TextMate scope stack. Contextual selectors must not color every token of a category.
  if (!/^[\w.-]+$/.test(selector)) return -1;
  return scope === selector || scope.startsWith(`${selector}.`)
    ? selector.split(".").length
    : -1;
}
export function vscodeSyntax(
  theme: VSCodeTheme,
): Partial<Record<SyntaxName, SyntaxStyle>> {
  const rules = theme.tokenColors ?? [];
  return Object.fromEntries(
    Object.entries(syntaxScopes).map(([name, scopes]) => {
      const style: SyntaxStyle = {
        fontStyle: "normal",
        fontWeight: "normal",
        textDecoration: "none",
      };
      let foregroundScore = -1,
        fontScore = -1;
      for (const rule of rules) {
        const parts = selectors(rule);
        const rank = parts.includes("")
          ? 0
          : Math.max(
              ...parts.flatMap((s) => scopes.map((scope) => score(s, scope))),
            );
        if (rank < 0) continue;
        if (rule.settings.foreground && rank >= foregroundScore) {
          style.color = rule.settings.foreground;
          foregroundScore = rank;
        }
        if (rule.settings.fontStyle !== undefined && rank >= fontScore) {
          const font = rule.settings.fontStyle.split(/\s+/);
          style.fontStyle = font.includes("italic") ? "italic" : "normal";
          style.fontWeight = font.includes("bold") ? "bold" : "normal";
          style.textDecoration = font.includes("underline")
            ? "underline"
            : "none";
          fontScore = rank;
        }
      }
      const foreground = theme.colors?.["editor.foreground"];
      style.color ??=
        foreground && hex.test(foreground)
          ? foreground
          : "var(--editor-foreground)";
      return [name, style];
    }),
  );
}
export function vscodeValues(theme: VSCodeTheme): ThemeValues {
  const tokens: Record<string, string> = {};
  const styles: Record<string, Record<string, string>> = {};
  const usable = (value: string | null | undefined): value is string =>
    !!value && hex.test(value);
  for (const [id, names] of Object.entries(workbenchColors))
    if (usable(theme.colors?.[id]))
      for (const name of names) tokens[name] = theme.colors![id]!;
  for (const [name, id] of Object.entries(terminalColorIds))
    if (usable(theme.colors?.[id]))
      tokens[
        `--terminal-${name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`
      ] = theme.colors![id]!;
  for (const [id, [selector, property]] of Object.entries(componentColors))
    if (usable(theme.colors?.[id]))
      (styles[selector] ??= {})[property] = theme.colors![id]!;
  for (const [id, value] of Object.entries(theme.colors ?? {}))
    if (usable(value)) tokens[`--vscode-${id.replaceAll(".", "-")}`] = value;
  const contrast = theme.colors?.contrastBorder;
  if (usable(contrast))
    styles[".button, input, .menu, .sidebar"] = {
      border: `1px solid ${contrast}`,
    };
  return { tokens, styles, editor: { syntax: vscodeSyntax(theme) } };
}
export function vscodeCompatibility(theme: VSCodeTheme) {
  const mapped = new Set([
    ...Object.keys(workbenchColors),
    ...Object.keys(componentColors),
    ...Object.values(terminalColorIds),
    "contrastBorder",
  ]);
  const unmatchedColors = Object.keys(theme.colors ?? {}).filter(
    (id) => !mapped.has(id),
  );
  return {
    unmatchedColors,
    messages: [
      "Editor syntax uses a TextMate-to-CodeMirror approximation; contextual and language-specific scopes may look different.",
      ...(Object.keys(theme.semanticTokenColors ?? {}).length ||
      theme.semanticHighlighting
        ? [
            "Semantic rules are preserved for VS Code export. Lomi does not provide VS Code language-service tokens.",
          ]
        : []),
      ...(unmatchedColors.length
        ? [
            `${unmatchedColors.length} color identifiers have no mapped Lomi component. They are retained for export.`,
          ]
        : []),
    ],
  };
}
