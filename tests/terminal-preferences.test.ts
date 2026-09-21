import assert from "node:assert/strict";
import { test } from "node:test";
import {
  defaultTerminalPreferences,
  defaultTerminalProfile,
  restoreTerminalPreferences,
} from "../src/terminal-preferences.ts";
import type { AppInfo } from "../src/model.ts";

test("terminal defaults inherit appearance and validate bounded, independent overrides", () => {
  const defaults = restoreTerminalPreferences(null);
  assert.deepEqual(defaults, defaultTerminalPreferences);
  defaults.behavior.scrollback = 0;
  assert.equal(defaultTerminalPreferences.behavior.scrollback, 10_000);
  const saved = {
    version: 1,
    ...defaultTerminalPreferences,
    appearance: {
      fontFamily: '"JetBrains Mono", monospace',
      fontSize: 24,
      cursorBlink: false,
      colors: { selectionBackground: "#12345680" },
    },
  };
  assert.deepEqual(restoreTerminalPreferences(saved), {
    appearance: saved.appearance,
    behavior: saved.behavior,
    windowsShell: "powershell",
    agentNotifications: true,
    alwaysShowTitles: false,
  });
  for (const invalid of [
    undefined,
    [],
    {},
    { ...saved, version: 2 },
    { ...saved, future: true },
    { ...saved, windowsShell: "bash" },
    { ...saved, windowsShell: null },
    { ...saved, agentNotifications: "true" },
    { ...saved, agentNotifications: null },
    { ...saved, alwaysShowTitles: "true" },
    { ...saved, alwaysShowTitles: null },
    ...[
      { fontSize: 0 },
      { fontFamily: "" },
      { fontFamily: "mono; color: red" },
      { colors: { red: "invalid" } },
      { colors: { unknown: "#123456" } },
      { cursorWidth: 1.5 },
    ].map((appearance) => ({ ...saved, appearance })),
    ...[
      { scrollback: -1 },
      { scrollback: 100001 },
      { scrollback: 1.5 },
      { scrollSensitivity: NaN },
      { tabStopWidth: 0 },
      { smoothScrollDuration: Infinity },
      { wordSeparator: "\n" },
      { screenReaderMode: "true" },
      { unknown: 1 },
    ].map((patch) => ({ ...saved, behavior: { ...saved.behavior, ...patch } })),
  ])
    assert.throws(() => restoreTerminalPreferences(invalid), /left intact/);
});

test("Windows defaults migrate old settings and select installed local shells only", () => {
  const legacy = {
    version: 1,
    appearance: {},
    behavior: defaultTerminalPreferences.behavior,
  };
  assert.equal(restoreTerminalPreferences(legacy).windowsShell, "powershell");
  assert.equal(restoreTerminalPreferences(legacy).agentNotifications, true);
  assert.equal(restoreTerminalPreferences(legacy).alwaysShowTitles, false);
  assert.equal(
    restoreTerminalPreferences({ ...legacy, alwaysShowTitles: true })
      .alwaysShowTitles,
    true,
  );
  assert.equal(
    restoreTerminalPreferences({ ...legacy, agentNotifications: false })
      .agentNotifications,
    false,
  );
  assert.equal(
    restoreTerminalPreferences({ ...legacy, windowsShell: "cmd" }).windowsShell,
    "cmd",
  );
  const info: AppInfo = {
    directory: "/project",
    home: "/home/test",
    platform: "windows",
    profiles: ["bash", "powershell", "cmd", "pwsh"].map((kind) => ({
      id: `local:${kind}`,
      name: kind,
      kind,
      program: `${kind}.exe`,
      distro: null,
      home: "/home/test",
    })),
  };
  assert.equal(defaultTerminalProfile(info, "powershell"), "local:pwsh");
  assert.equal(defaultTerminalProfile(info, "cmd"), "local:cmd");
  info.profiles.pop();
  assert.equal(defaultTerminalProfile(info, "powershell"), "local:powershell");
  for (const platform of ["linux", "macos"])
    assert.equal(
      defaultTerminalProfile({ ...info, platform }, "cmd"),
      "local:bash",
    );
  info.profiles = [info.profiles[0]];
  assert.equal(defaultTerminalProfile(info, "cmd"), "local:bash");
  assert.equal(defaultTerminalProfile({ ...info, profiles: [] }, "cmd"), "");
});
