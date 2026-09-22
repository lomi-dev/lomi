import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  parsePlugin,
  jsonState,
  packagePath,
} from "../src/plugins/manifest.ts";
import {
  PluginHost,
  emptyContext,
  type PluginEntry,
} from "../src/plugins/host.ts";
import { normalizePalette, terminalPresets } from "../src/theme/palette.ts";
const manifest = parsePlugin(
  JSON.parse(
    readFileSync(
      new URL("fixtures/context-plugin/plugin.json", import.meta.url),
      "utf8",
    ),
  ),
);
const entry: PluginEntry = {
  id: manifest.id,
  manifest,
  revision: "a".repeat(64),
  source: "/example",
  enabled: true,
  trustedRevision: "a".repeat(64),
  error: null,
  restartRequired: false,
  evaluated: false,
};
test("plugin manifests and persisted state reject unsupported contracts and escaping paths", () => {
  for (const patch of [
    { schemaVersion: 2 },
    { hostApi: 2 },
    { dependencies: ["other.plugin"] },
    { entry: "../index.js" },
    { entry: "src.ts" },
    {
      contributes: {
        views: [
          {
            id: "foreign.view",
            title: "Bad",
            placement: "central",
            multiple: false,
            stateVersion: 1,
          },
        ],
      },
    },
  ])
    assert.throws(() => parsePlugin({ ...manifest, ...patch }));
  for (const path of [
    "/root.js",
    "../a",
    "a/../b",
    "a\\b",
    "C:/file.js",
    "a%2fb.js",
    "a/CON.txt",
    "a/aux",
    "a/.",
    "a//b",
    "a?b",
    "a:stream",
  ])
    assert.throws(() => packagePath(path));
  packagePath("dist/space żółć.js");
  jsonState({ expanded: true, values: [null, 1, "文字"] });
  for (const value of [
    NaN,
    () => {},
    new Date(),
    { x: undefined },
    JSON.parse('{"__proto__":{}}'),
    "x".repeat(65537),
  ])
    assert.throws(() => jsonState(value));
});
test("listing never imports code; concurrent lazy activation runs once and cleanup survives disposer errors", async () => {
  let imports = 0,
    activations = 0,
    cleanup = 0,
    handled = 0;
  const host = new PluginHost({
    prepare: async () => manifest,
    url: () => "/asset",
    css: async () => () => {
      cleanup++;
    },
    import: async () => {
      imports++;
      return {
        activate(ctx) {
          activations++;
          ctx.registerCommand("lomi.context.open", () => {
            handled++;
          });
          ctx.add(() => {
            cleanup++;
            throw Error("disposer failure");
          });
          ctx.add(() => {
            cleanup++;
          });
        },
      };
    },
  });
  await host.discover({
    directory: "",
    entries: [{ ...entry }],
    safeMode: false,
  });
  assert.equal(imports, 0);
  host.setContext({ ...emptyContext, workspaceName: "Example" });
  await Promise.all([
    host.activate(entry.id),
    host.activate(entry.id),
    host.execute("lomi.context.open"),
  ]);
  assert.equal(imports, 1);
  assert.equal(activations, 1);
  assert.equal(handled, 1);
  await host.deactivate(entry.id);
  assert.equal(host.commands.size, 0);
  assert.equal(cleanup, 3);
  await host.discover({
    directory: "",
    entries: [{ ...entry }],
    safeMode: true,
  });
  await assert.rejects(host.activate(entry.id));
  assert.equal(imports, 1);
});
test("partial and canceled activation cannot retain or recreate owned registrations", async () => {
  let finish!: () => void;
  let lateCleanup = 0;
  const gate = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const host = new PluginHost({
    prepare: async () => manifest,
    url: () => "/asset",
    css: async () => () => {},
    import: async () => ({
      async activate(ctx) {
        ctx.registerCommand("lomi.context.open", () => {});
        await gate;
        assert.throws(() =>
          ctx.registerFill("lomi.context.status", () => null),
        );
        return () => {
          lateCleanup++;
        };
      },
    }),
  });
  await host.discover({
    directory: "",
    entries: [{ ...entry }],
    safeMode: false,
  });
  const active = host.activate(entry.id);
  const rejected = assert.rejects(active);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(host.commands.size, 1);
  await host.deactivate(entry.id);
  finish();
  await rejected;
  assert.equal(host.commands.size, 0);
  assert.equal(host.fills.size, 0);
  assert.equal(lateCleanup, 1);
  const failed = new PluginHost({
    prepare: async () => manifest,
    url: () => "/asset",
    css: async () => () => {},
    import: async () => ({
      activate(ctx) {
        ctx.registerCommand("lomi.context.open", () => {});
        throw Error("fault fixture");
      },
    }),
  });
  await failed.discover({
    directory: "",
    entries: [{ ...entry }],
    safeMode: false,
  });
  await assert.rejects(failed.activate(entry.id));
  assert.equal(failed.commands.size, 0);
  assert.equal(failed.statuses.get(entry.id)?.phase, "failed");
});
test("xterm-theme palettes normalize to xterm 6 without legacy or unknown fields", async () => {
  const palette = normalizePalette({
    foreground: "#ffffff",
    background: "#000000",
    selection: "#334455",
    unexpected: "#aaaaaa",
  });
  assert.equal(palette.selectionBackground, "#334455");
  assert.equal("selection" in palette, false);
  assert.equal("unexpected" in palette, false);
  assert.throws(() =>
    normalizePalette({ foreground: "red", background: "#000000" }),
  );
  const all = await terminalPresets();
  assert.ok(Object.keys(all).length > 100);
  assert.ok(all.AdventureTime);
});

test("plugin panels and sidebar state restore without shells, including missing view types", async () => {
  const {
    newSession,
    newProject,
    restoreSession,
    pluginPanels,
    updatePluginPanel,
    layoutPanes,
    mergeTabs,
  } = await import("../src/model.ts");
  const project = newProject("/project", "local:bash");
  const workspace = project.workspaces[0];
  const panel = {
    type: "plugin" as const,
    id: "panel-a",
    title: "Unavailable",
    owner: "sample.missing",
    viewType: "sample.missing.view",
    stateVersion: 3,
    state: { expanded: true },
  };
  workspace.tabs.push(panel);
  workspace.pluginSidebars = [
    { ...panel, id: "sidebar-a", viewType: "sample.missing.sidebar" },
  ];
  workspace.activeTabId = panel.id;
  const saved = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
    sidebar: "sample.missing.sidebar" as const,
    sidebarSides: {
      files: "left" as const,
      git: "left" as const,
      workspaces: "left" as const,
      "sample.missing.sidebar": "left" as const,
    },
  };
  const info = {
    directory: "/project",
    home: "/home",
    platform: "linux",
    profiles: [],
  };
  const restored = restoreSession(saved, info);
  assert.deepEqual(pluginPanels(restored), [
    workspace.pluginSidebars[0],
    panel,
  ]);
  assert.equal(restored.sidebar, "sample.missing.sidebar");
  assert.equal(restored.projects[0].workspaces[0].activeTabId, panel.id);
  const changed = updatePluginPanel(restored, panel.id, { expanded: false });
  assert.deepEqual(
    pluginPanels(changed).find((p) => p.id === panel.id)?.state,
    { expanded: false },
  );
  const tab = workspace.tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal");
  assert.throws(
    () =>
      restoreSession(
        {
          ...saved,
          projects: [
            {
              ...project,
              workspaces: [
                { ...workspace, tabs: [{ ...panel, type: "unknown" }] },
              ],
            },
          ],
        },
        info,
      ),
    /unsupported|Unsupported|Unknown/,
  );
});

test("the packaged fault fixture removes all partial registrations", async () => {
  const fault = parsePlugin(
    JSON.parse(
      readFileSync(
        new URL("fixtures/fault-plugin/plugin.json", import.meta.url),
        "utf8",
      ),
    ),
  );
  let cssRemoved = false;
  const host = new PluginHost({
    prepare: async () => fault,
    url: () => "",
    css: async () => () => {
      cssRemoved = true;
    },
    import: () => import("./fixtures/fault-plugin/index.js"),
  });
  await host.discover({
    directory: "",
    entries: [{ ...entry, id: fault.id, manifest: fault }],
    safeMode: false,
  });
  await assert.rejects(host.activate(fault.id), /Intentional failure/);
  assert.equal(host.commands.size, 0);
  assert.equal(host.fills.size, 0);
  assert.equal(cssRemoved, true);
});
test("plugin defaults never steal built-in or user shortcuts and missing overrides survive", async () => {
  const { restoreKeybindings } = await import("../src/keybindings.ts");
  const contributions = [
    {
      id: "sample.tools.first" as const,
      label: "First",
      description: "First",
      group: "Plugin",
      shortcut: "Ctrl+KeyW",
    },
    {
      id: "sample.tools.second" as const,
      label: "Second",
      description: "Second",
      group: "Plugin",
      shortcut: "Ctrl+Shift+KeyY",
    },
  ];
  const settings = {
    version: 1 as const,
    bindings: {
      "sample.absent.action": "Ctrl+Shift+KeyY",
      "sample.tools.first": null,
    },
  };
  const restored = restoreKeybindings(settings, false, contributions);
  assert.equal(restored.closeTerminal, "Ctrl+KeyW");
  assert.equal(restored["sample.tools.first"], null);
  assert.equal(restored["sample.absent.action"], "Ctrl+Shift+KeyY");
  assert.equal(restored["sample.tools.second"], null);
  const missing = restoreKeybindings({ version: 1, bindings: restored });
  assert.equal(missing["sample.tools.first"], null);
  assert.equal(missing["sample.absent.action"], "Ctrl+Shift+KeyY");
});
