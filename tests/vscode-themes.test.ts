import { test } from "node:test";
import assert from "node:assert/strict";
import { parseThemeText, resolveTheme } from "../src/theme/format.ts";
import {
  parseVSCodeTheme,
  vscodeSyntax,
  vscodeCompatibility,
  terminalColorIds,
} from "../src/theme/vscode.ts";

const source = {
  type: "dark",
  colors: {
    "editor.background": "#1234",
    "editor.foreground": "#dedede",
    "sideBar.background": "#221144",
    "titleBar.activeForeground": "#ffeedd",
    "terminal.ansiBlue": "#346789",
    "extension.futureColor": "#abcdef",
  },
  tokenColors: [
    {
      scope: "comment",
      settings: { foreground: "#aaaaaa", fontStyle: "italic bold" },
    },
    { scope: "comment.line", settings: { foreground: "#bbbbbb" } },
    { scope: "comment", settings: { foreground: "#cccccc", fontStyle: "" } },
    { scope: "source.python comment", settings: { foreground: "#ff0000" } },
  ],
  semanticHighlighting: true,
  semanticTokenColors: {
    "variable.readonly:typescript": { foreground: "#abcdef", italic: true },
  },
};
test("VS Code data survives validation, mapping and Lomi overrides", () => {
  const manifest = parseThemeText(
    JSON.stringify({
      version: 2,
      name: "Portable",
      vscode: source,
      common: { editor: { colors: { background: "#999999" } } },
    }),
  );
  const resolved = resolveTheme(manifest, "dark");
  assert.deepEqual(JSON.parse(JSON.stringify(manifest.vscode)), source);
  assert.equal(resolved.tokens?.["--terminal-blue"], "#346789");
  assert.equal(resolved.tokens?.["--sidebar-background"], "#221144");
  assert.equal(resolved.styles?.[".titlebar"]?.color, "#ffeedd");
  assert.equal(resolved.editor?.colors?.background, "#999999");
  assert.deepEqual(vscodeCompatibility(manifest.vscode!).unmatchedColors, [
    "extension.futureColor",
  ]);
  assert.equal(
    Object.keys(terminalColorIds).filter((s) =>
      /^(bright)?(black|red|green|yellow|blue|magenta|cyan|white)$/i.test(s),
    ).length,
    16,
  );
});
test("scope specificity is per property; empty fontStyle resets and context cannot bleed globally", () => {
  const syntax = vscodeSyntax(parseVSCodeTheme(source));
  assert.deepEqual(syntax.comment, {
    color: "#bbbbbb",
    fontStyle: "normal",
    fontWeight: "normal",
    textDecoration: "none",
  });
  const precise = vscodeSyntax(
    parseVSCodeTheme({
      tokenColors: [
        { settings: { foreground: "#123456", fontStyle: "italic" } },
        {
          scope: ["entity.name.function", "support.function"],
          settings: { foreground: "#fedcba" },
        },
      ],
    }),
  );
  assert.equal(precise.function?.color, "#fedcba");
  assert.equal(precise.function?.fontStyle, "italic");
  assert.equal(precise.number?.color, "#123456");
});
test("invalid VS Code data fails before activation while future color identifiers are retained", () => {
  for (const bad of [
    { include: "../base.json" },
    { tokenColors: "./syntax.tmTheme" },
    { type: "pink" },
    { semanticHighlighting: "yes" },
    { colors: { foreground: "url(file:///secret)" } },
    { colors: [] },
    { colors: { "bad; id": "#112233" } },
    { tokenColors: [{ scope: [23], settings: {} }] },
    { tokenColors: [{ scope: "comment", settings: { fontStyle: "blink" } }] },
    { semanticTokenColors: { variable: { bold: "true" } } },
  ])
    assert.throws(() => parseVSCodeTheme(bad));
  assert.doesNotThrow(() =>
    parseVSCodeTheme({ colors: { custom: null, foreground: "default" } }),
  );
  assert.throws(
    () =>
      parseThemeText(
        '{"version":2,"name":"Bad","vscode":{"colors":{"foreground":"#fff","foreground":"#000"}}}',
      ),
    /Duplicate/,
  );
});
