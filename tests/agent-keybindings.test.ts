import assert from "node:assert/strict";
import { test } from "node:test";
import {
  stageKeybindingUpdate,
  type KeybindingSource,
} from "../src/agent-keybindings.ts";
import {
  actions,
  restoreKeybindings,
  type KeybindingSettings,
  type ShortcutAction,
} from "../src/keybindings.ts";

function fixture(
  data: KeybindingSettings | null = null,
  contributions: ShortcutAction[] = [],
  mac = true,
) {
  const source: KeybindingSource = {
    data,
    contributions,
    sourceRevision: data ? "a".repeat(64) : null,
    definitionsRevision: "b".repeat(64),
  };
  const retained = {
    actions: [...actions, ...contributions],
    bindings: restoreKeybindings(data, mac, contributions),
    defaults: restoreKeybindings(null, mac, contributions),
    focusFollowsPointer: data?.focusFollowsPointer ?? false,
  };
  return { source, retained, mac };
}

test("shortcut set, disable, reset and focus plans retain source identity on both platforms", () => {
  for (const mac of [false, true]) {
    const { source, retained } = fixture(
      {
        version: 1,
        bindings: { saveFile: "Ctrl+Alt+F20", "missing.plugin": null },
      },
      [],
      mac,
    );
    const original = structuredClone(source);
    for (const shortcut of ["Ctrl+Alt+F21", null]) {
      const plan = stageKeybindingUpdate(
        source,
        { type: "keybinding_set", action: "saveFile", shortcut },
        retained,
        mac,
      );
      assert.equal(plan.action?.shortcut, "Ctrl+Alt+F20");
      assert.equal(
        plan.action?.defaultShortcut,
        mac ? "Meta+KeyS" : "Ctrl+KeyS",
      );
      assert.equal(plan.sourceRevision, source.sourceRevision);
    }
    assert.equal(
      stageKeybindingUpdate(
        source,
        { type: "keybinding_reset", action: "saveFile" },
        retained,
        mac,
      ).action?.id,
      "saveFile",
    );
    assert.equal(
      stageKeybindingUpdate(
        source,
        { type: "keybinds_focus_follows_pointer", value: true },
        retained,
        mac,
      ).action,
      null,
    );
    assert.deepEqual(source, original);
  }
});

test("shortcut collisions cannot disable an unrelated default through restore fallback", () => {
  const { source, retained, mac } = fixture();
  assert.throws(
    () =>
      stageKeybindingUpdate(
        source,
        {
          type: "keybinding_set",
          action: "chatNew",
          shortcut: retained.bindings.saveFile,
        },
        retained,
        mac,
      ),
    /REVISION_CONFLICT/,
  );
  assert.throws(
    () =>
      stageKeybindingUpdate(
        source,
        { type: "keybinding_set", action: "saveFile", shortcut: "KeyA" },
        retained,
        mac,
      ),
    /RESOURCE_EXHAUSTED/,
  );
  const changed = fixture({
    version: 1,
    bindings: { saveFile: null, chatNew: "Meta+KeyS" },
  });
  assert.throws(
    () =>
      stageKeybindingUpdate(
        changed.source,
        { type: "keybinding_reset", action: "saveFile" },
        changed.retained,
        changed.mac,
      ),
    /REVISION_CONFLICT/,
  );
});

test("shortcut writes compare native defaults, retained values and installed actions", () => {
  const contribution: ShortcutAction = {
    id: "fixture.open",
    label: "Open fixture",
    group: "Fixture",
    description: "",
    shortcut: "Ctrl+Alt+F20",
  };
  const { source, retained, mac } = fixture(null, [contribution]);
  assert.equal(
    stageKeybindingUpdate(
      source,
      {
        type: "keybinding_set",
        action: contribution.id,
        shortcut: "Ctrl+Alt+F21",
      },
      retained,
      mac,
    ).action?.label,
    contribution.label,
  );
  assert.throws(
    () =>
      stageKeybindingUpdate(
        source,
        { type: "keybinding_set", action: "missing.plugin", shortcut: null },
        retained,
        mac,
      ),
    /TARGET_NOT_FOUND/,
  );
  assert.throws(
    () =>
      stageKeybindingUpdate(
        { ...source, contributions: [] },
        { type: "keybinding_set", action: "saveFile", shortcut: null },
        retained,
        mac,
      ),
    /REVISION_CONFLICT/,
  );
  assert.throws(
    () =>
      stageKeybindingUpdate(
        source,
        { type: "keybinds_focus_follows_pointer", value: true },
        { ...retained, focusFollowsPointer: true },
        mac,
      ),
    /REVISION_CONFLICT/,
  );
  assert.throws(
    () =>
      stageKeybindingUpdate(
        { ...source, data: { version: 2 } as unknown as KeybindingSettings },
        { type: "keybinding_reset", action: "saveFile" },
        retained,
        mac,
      ),
    /UNSUPPORTED_CAPABILITY/,
  );
});
