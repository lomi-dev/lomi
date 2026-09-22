import { test } from "node:test";
import assert from "node:assert/strict";
import {
  parseThemeText,
  editThemeText,
  formatThemeText,
  migrateTheme,
  resolveTheme,
} from "../src/theme/format.ts";
test("JSONC rejects duplicate/malformed/unknown fields and preserves comments through edits and formatting", () => {
  const raw =
    '{ // Keep this note\r\n"version":2,"name":"Example", "common":{"editor":{"syntax":{"keyword":{"color":"#abcdef"}}}},}';
  const edited = editThemeText(
    raw,
    ["common", "tokens", "--radius-control"],
    "9px",
  );
  const formatted = formatThemeText(edited);
  assert.ok(formatted.includes("// Keep this note\r\n"));
  assert.equal(
    parseThemeText(formatted).common?.tokens?.["--radius-control"],
    "9px",
  );
  assert.equal(
    parseThemeText(formatted).common?.editor?.syntax?.keyword?.color,
    "#abcdef",
  );
  for (const raw of [
    '{"version":2,"version":2,"name":"X"}',
    '{name:"X",version:2}',
    '{"version":2,"name":"X","common":{"editor":{"syntax":{"secretTag":{}}}}}',
    '{"version":2,"name":"X","common":{"terminal":{"fontSize":1e999}}}',
    '{"version":2,"name":"X","common":{"terminal":{"colors":{"red":"#1234567"}}}}',
    '{"version":2,"name":"X","resources":{"stylesheets":["../escape.css"]}}',
  ])
    assert.throws(() => parseThemeText(raw));
  assert.throws(
    () => parseThemeText('{\n"version":2,"name":"one", "name":"two"}'),
    /theme.jsonc:2:.*Duplicate key name/,
  );
});
test("common and appearance variants resolve deterministically without mutating input; legacy conversion preserves supported sections", () => {
  const theme = parseThemeText(
    JSON.stringify({
      version: 2,
      name: "Adaptive",
      common: {
        tokens: { "--radius-control": "4px" },
        terminal: { fontSize: 17, colors: { red: "#aabbcc" } },
        editor: { syntax: { keyword: { fontWeight: "bold" } } },
        plugins: { "sample.context": { surface: "var(--color-surface)" } },
      },
      light: {
        terminal: { colors: { blue: "#334455" } },
        editor: { colors: { background: "#fafafa" } },
      },
    }),
  );
  const light = resolveTheme(theme, "light"),
    dark = resolveTheme(theme, "dark");
  assert.equal(light.terminal?.fontSize, 17);
  assert.deepEqual(light.terminal?.colors, { red: "#aabbcc", blue: "#334455" });
  assert.deepEqual(dark.terminal?.colors, { red: "#aabbcc" });
  assert.equal(theme.common?.terminal?.colors?.blue, undefined);
  const old = {
    version: 1,
    name: "Legacy",
    appearance: "dark",
    tokens: { "--pane-spacing": "4px" },
    layout: { tabs: "below" },
    assets: { wall: "wall.svg" },
    stylesheet: "theme.css",
    backgrounds: { terminal: { image: "wall.svg", opacity: 0.2 } },
  };
  const result = migrateTheme(old);
  assert.equal(result.manifest.version, 2);
  assert.equal(result.manifest.common?.layout?.tabs, "below");
  assert.deepEqual(result.manifest.resources, {
    assets: { wall: "wall.svg" },
    stylesheets: ["theme.css"],
  });
  assert.equal(old.version, 1);
  assert.ok(result.report[0].includes("preserved"));
  assert.throws(
    () => migrateTheme({ ...old, unknown: true }),
    /Unknown theme field/,
  );
});

test("shipped baselines and shared native/frontend grammar fixtures validate", async () => {
  const { readFile } = await import("node:fs/promises");
  const { builtinTheme, deepmonoTheme, parseTheme } =
    await import("../src/theme/format.ts");
  assert.equal(parseTheme(builtinTheme).name, "Lomi");
  assert.equal(parseTheme(deepmonoTheme).name, "DeepMono");
  const fixtures = JSON.parse(
    await readFile(
      new URL("fixtures/themes/grammar.json", import.meta.url),
      "utf8",
    ),
  );
  for (const fixture of fixtures) {
    if (fixture.valid)
      assert.doesNotThrow(() => parseThemeText(fixture.raw), fixture.name);
    else assert.throws(() => parseThemeText(fixture.raw), fixture.name);
  }
  const theme = parseTheme({
    version: 2,
    name: "Merge",
    common: {
      editor: { syntax: { keyword: { fontWeight: "bold" } } },
      backgrounds: { terminal: { opacity: 0.2 } },
      styles: { ".tab": { padding: "2px" } },
    },
    dark: {
      editor: { syntax: { keyword: { color: "#abc" } } },
      backgrounds: { terminal: { blur: 2 } },
      styles: { ".tab": { color: "#fff" } },
    },
  });
  const resolved = resolveTheme(theme, "dark");
  assert.deepEqual(resolved.editor?.syntax?.keyword, {
    fontWeight: "bold",
    color: "#abc",
  });
  assert.deepEqual(resolved.backgrounds?.terminal, { opacity: 0.2, blur: 2 });
  assert.deepEqual(resolved.styles?.[".tab"], {
    padding: "2px",
    color: "#fff",
  });
});
