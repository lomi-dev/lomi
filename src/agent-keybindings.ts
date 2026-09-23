import {
  actions,
  restoreKeybindings,
  validShortcut,
  type ActionId,
  type Keybindings,
  type KeybindingSettings,
  type ShortcutAction,
} from "./keybindings.ts";

export type KeybindingPatch =
  | { type: "keybinding_set"; action: string; shortcut: string | null }
  | { type: "keybinding_reset"; action: string }
  | { type: "keybinds_focus_follows_pointer"; value: boolean };

export interface KeybindingSource {
  data: KeybindingSettings | null;
  sourceRevision: string | null;
  definitionsRevision: string;
  contributions: Pick<ShortcutAction, "id" | "label" | "shortcut">[];
}

interface RetainedKeybindings {
  actions: readonly ShortcutAction[];
  bindings: Keybindings;
  defaults: Keybindings;
  focusFollowsPointer: boolean;
}

function sameBindings(a: Keybindings, b: Keybindings) {
  return (
    Object.keys(a).length === Object.keys(b).length &&
    Object.entries(a).every(([id, value]) => b[id as ActionId] === value)
  );
}

// Normalize both sides with the provider's existing migration/default/conflict
// rules. Reject fallback changes to unrelated actions instead of saving them.
export function stageKeybindingUpdate(
  source: KeybindingSource,
  patch: KeybindingPatch,
  retained: RetainedKeybindings,
  mac: boolean,
) {
  const contributions = source.contributions.map((action) => ({
    ...action,
    group: "Plugin",
    description: "",
  }));
  let before: Keybindings;
  let defaults: Keybindings;
  try {
    before = restoreKeybindings(source.data, mac, contributions);
    defaults = restoreKeybindings(null, mac, contributions);
  } catch {
    throw Error("UNSUPPORTED_CAPABILITY");
  }
  const focusFollowsPointer = source.data?.focusFollowsPointer ?? false;
  if (
    !sameBindings(before, retained.bindings) ||
    !sameBindings(defaults, retained.defaults) ||
    focusFollowsPointer !== retained.focusFollowsPointer
  )
    throw Error("REVISION_CONFLICT");
  const target =
    patch.type === "keybinds_focus_follows_pointer"
      ? null
      : [...actions, ...contributions].find((a) => a.id === patch.action);
  if (
    patch.type !== "keybinds_focus_follows_pointer" &&
    (!target ||
      !retained.actions.some(
        (a) => a.id === target.id && a.label === target.label,
      ))
  )
    throw Error("TARGET_NOT_FOUND");
  if (
    patch.type === "keybinding_set" &&
    patch.shortcut !== null &&
    !validShortcut(patch.shortcut)
  )
    throw Error("RESOURCE_EXHAUSTED");
  const next: KeybindingSettings = structuredClone(
    source.data ?? { version: 1, bindings: {} },
  );
  if (patch.type === "keybinds_focus_follows_pointer")
    next.focusFollowsPointer = patch.value;
  else if (patch.type === "keybinding_set")
    next.bindings[patch.action as ActionId] = patch.shortcut;
  else delete next.bindings[patch.action as ActionId];
  let after: Keybindings;
  try {
    after = restoreKeybindings(next, mac, contributions);
  } catch {
    throw Error("REVISION_CONFLICT");
  }
  const expected = { ...before };
  if (target && patch.type !== "keybinds_focus_follows_pointer")
    expected[target.id] =
      patch.type === "keybinding_reset" ? defaults[target.id] : patch.shortcut;
  if (!sameBindings(after, expected)) throw Error("REVISION_CONFLICT");
  return {
    focusFollowsPointer,
    action: target
      ? {
          id: target.id,
          label: target.label,
          shortcut: before[target.id],
          defaultShortcut: defaults[target.id],
        }
      : null,
    sourceRevision: source.sourceRevision,
    definitionsRevision: source.definitionsRevision,
  };
}
