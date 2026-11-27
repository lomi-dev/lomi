import { msg, t } from "./i18n";
import { keyNames, validShortcut } from "@lomi-dev/plugin-sdk/shortcuts";
export { validShortcut } from "@lomi-dev/plugin-sdk/shortcuts";
export const actions = [
  {
    id: "commandPicker",
    label: msg("Show commands"),
    description: msg("Find and run a workspace or plugin command."),
    group: msg("Workspace"),
    shortcut: "Ctrl+Shift+KeyP",
  },
  {
    id: "saveFile",
    label: msg("Save file"),
    description: msg("Save the active editor file."),
    group: msg("Editor"),
    shortcut: "Ctrl+KeyS",
  },
  {
    id: "findFile",
    label: msg("Find in file"),
    description: msg("Find and replace text in the active editor."),
    group: msg("Editor"),
    shortcut: "Ctrl+KeyF",
  },
  {
    id: "goToLine",
    label: msg("Go to line"),
    description: msg("Jump to a line in the active editor."),
    group: msg("Editor"),
    shortcut: "Ctrl+KeyG",
  },
  {
    id: "toggleWordWrap",
    label: msg("Toggle word wrap"),
    description: msg("Wrap long lines in the active editor."),
    group: msg("Editor"),
    shortcut: "Alt+KeyZ",
  },
  {
    id: "terminalOverview",
    label: msg("Toggle terminal overview"),
    description: msg("Show terminal titles in place of their contents."),
    group: msg("Terminals"),
    shortcut: "Ctrl+Tab",
  },
  {
    id: "newTerminal",
    label: msg("New terminal"),
    description: msg("Split the active panel side by side."),
    group: msg("Terminals"),
    shortcut: "Ctrl+KeyD",
  },
  {
    id: "splitVertical",
    label: msg("Split terminal vertically"),
    description: msg("Place a terminal below the active panel."),
    group: msg("Terminals"),
    shortcut: "Ctrl+Shift+KeyD",
  },
  {
    id: "closeTerminal",
    label: msg("Close terminal"),
    description: msg("Close the active panel."),
    group: msg("Terminals"),
    shortcut: "Ctrl+KeyW",
  },
  {
    id: "searchTerminal",
    label: msg("Find in terminal"),
    description: msg("Search the active terminal's output."),
    group: msg("Terminals"),
    shortcut: "Ctrl+Shift+KeyF",
  },
  {
    id: "copyTerminal",
    label: msg("Copy terminal selection"),
    description: msg("Copy selected terminal text."),
    group: msg("Terminals"),
    shortcut: "Ctrl+Shift+KeyC",
  },
  {
    id: "pasteTerminal",
    label: msg("Paste into terminal"),
    description: msg("Paste clipboard text or images into the terminal."),
    group: msg("Terminals"),
    shortcut: "Ctrl+Shift+KeyV",
  },
  {
    id: "commandInput",
    label: msg("Command input"),
    description: msg("Show or hide the multiline command input."),
    group: msg("Terminals"),
    shortcut: "Ctrl+Shift+KeyI",
  },
  {
    id: "runCommand",
    label: msg("Run command input"),
    description: msg("Run a command while the command input is focused."),
    group: msg("Terminals"),
    shortcut: "Ctrl+Enter",
  },
  {
    id: "commandBlocks",
    label: msg("Command blocks"),
    description: msg("Show or hide command history and output positions."),
    group: msg("Terminals"),
    shortcut: "Ctrl+Shift+KeyH",
  },
  {
    id: "changeEnvironment",
    label: msg("Change terminal environment"),
    description: msg(
      "Choose the shell or WSL distribution for the active tab.",
    ),
    group: msg("Terminals"),
    shortcut: "Ctrl+Shift+KeyL",
  },
  {
    id: "newTab",
    label: msg("New tab"),
    description: msg("Open a tab with one terminal."),
    group: msg("Tabs"),
    shortcut: "Ctrl+Shift+KeyT",
  },
  {
    id: "newChat",
    label: msg("New chat"),
    description: msg("Open a chat thread tab."),
    group: msg("Tabs"),
    shortcut: "Ctrl+Shift+KeyN",
  },
  {
    id: "toggleDictation",
    label: msg("Toggle dictation"),
    description: msg(
      "Dictate into the active chat or terminal. Tap to start and stop, or hold to talk.",
    ),
    group: msg("Workspace"),
    shortcut: "Ctrl+Shift+KeyM",
  },
  {
    id: "focusChat",
    label: msg("Focus chat message"),
    description: msg("Move focus to the message field of the active chat."),
    group: msg("Tabs"),
    shortcut: "Ctrl+Shift+KeyJ",
  },
  {
    id: "chatModelPicker",
    label: msg("Choose chat model"),
    description: msg("Open the model picker of the active chat."),
    group: msg("Tabs"),
    shortcut: "Ctrl+Shift+KeyK",
  },
  {
    id: "closeTab",
    label: msg("Close tab"),
    description: msg("Close the active tab and all its terminals."),
    group: msg("Tabs"),
    shortcut: "Ctrl+Shift+KeyW",
  },
  {
    id: "nextTab",
    label: msg("Next tab"),
    description: msg("Switch to the next tab in this workspace."),
    group: msg("Tabs"),
    shortcut: "Ctrl+PageDown",
  },
  {
    id: "previousTab",
    label: msg("Previous tab"),
    description: msg("Switch to the previous tab in this workspace."),
    group: msg("Tabs"),
    shortcut: "Ctrl+PageUp",
  },
  {
    id: "toggleExplorer",
    label: msg("Toggle file explorer"),
    description: msg("Show or hide the file explorer."),
    group: msg("Workspace"),
    shortcut: "Ctrl+Shift+KeyE",
  },
  {
    id: "toggleSourceControl",
    label: msg("Toggle source control"),
    description: msg("Show or hide Source Control when Git is available."),
    group: msg("Workspace"),
    shortcut: "Ctrl+Shift+KeyG",
  },
  {
    id: "toggleWorkspaces",
    label: msg("Toggle workspaces"),
    description: msg("Show or hide the workspace list for all folders."),
    group: msg("Workspace"),
    shortcut: null,
  },
  {
    id: "openSettings",
    label: msg("Open settings"),
    description: msg("Open the Settings tab."),
    group: msg("Workspace"),
    shortcut: "Ctrl+Comma",
  },
  {
    id: "zoomIn",
    label: msg("Zoom in"),
    description: msg(
      "Increase the size of the entire interface, including terminals and editors.",
    ),
    group: msg("Workspace"),
    shortcut: "Ctrl+Equal",
  },
  {
    id: "zoomOut",
    label: msg("Zoom out"),
    description: msg(
      "Decrease the size of the entire interface, including terminals and editors.",
    ),
    group: msg("Workspace"),
    shortcut: "Ctrl+Minus",
  },
  {
    id: "resetZoom",
    label: msg("Reset zoom"),
    description: msg("Restore the interface to its original size (100%)."),
    group: msg("Workspace"),
    shortcut: "Ctrl+Digit0",
  },
  {
    id: "movePanel",
    label: msg("Move active panel"),
    description: msg("Choose another panel and a docking side in this tab."),
    group: msg("Workspace"),
    shortcut: null,
  },
  {
    id: "dockTab",
    label: msg("Dock current tab"),
    description: msg("Dock this tab into another terminal tab."),
    group: msg("Workspace"),
    shortcut: null,
  },
] as const;

export type BuiltinActionId = (typeof actions)[number]["id"];
export type ActionId = BuiltinActionId | `${string}.${string}`;
export interface ShortcutAction {
  id: ActionId;
  label: string;
  description: string;
  group: string;
  shortcut: string | null;
}

export type Keybindings = Record<ActionId, string | null>;
export interface KeybindingSettings {
  version: 1;
  bindings: Partial<Keybindings>;
  focusFollowsPointer?: boolean;
  explorerOpenOnSingleClick?: boolean;
}

export function defaultKeybindings(
  mac = false,
  contributions: ShortcutAction[] = [],
): Keybindings {
  return Object.fromEntries(
    [...actions, ...contributions].map(({ id, shortcut }) => [
      id,
      mac && id !== "terminalOverview"
        ? (shortcut?.replace("Ctrl", "Meta") ?? null)
        : shortcut,
    ]),
  ) as Keybindings;
}

export interface KeyEvent {
  code: string;
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  isComposing?: boolean;
  getModifierState?: (key: string) => boolean;
}

export function shortcutFromEvent(event: KeyEvent): string | null {
  if (
    event.isComposing ||
    ["Dead", "Process"].includes(event.key) ||
    event.getModifierState?.("AltGraph")
  )
    return null;
  const shortcut = [
    event.ctrlKey && "Ctrl",
    event.altKey && "Alt",
    event.metaKey && "Meta",
    event.shiftKey && "Shift",
    event.code,
  ]
    .filter(Boolean)
    .join("+");
  return validShortcut(shortcut) ? shortcut : null;
}

export function actionForEvent(
  event: KeyEvent,
  bindings: Keybindings,
): ActionId | undefined {
  const shortcut = shortcutFromEvent(event);
  if (!shortcut) return;
  const exact = Object.entries(bindings).find(
    ([, binding]) => binding === shortcut,
  )?.[0] as ActionId | undefined;
  if (exact) return exact;
  // Plus may require Shift or a different physical key on the user's layout.
  // Explicit assignments take precedence over these standard zoom aliases.
  if (
    (event.key === "+" || event.code === "NumpadAdd") &&
    bindings.zoomIn ===
      shortcut.slice(0, shortcut.lastIndexOf("+") + 1).replace("Shift+", "") +
        "Equal"
  )
    return "zoomIn";
  if (
    event.code === "NumpadSubtract" &&
    bindings.zoomOut === shortcut.replace(/NumpadSubtract$/, "Minus")
  )
    return "zoomOut";
}

export function isZoomAction(
  action: ActionId | undefined,
): action is "zoomIn" | "zoomOut" | "resetZoom" {
  return action === "zoomIn" || action === "zoomOut" || action === "resetZoom";
}

export function formatShortcut(shortcut: string | null): string {
  if (!shortcut) return t("Not set");
  return shortcut
    .split("+")
    .map((part) =>
      part === "Meta"
        ? "Cmd"
        : (keyNames[part] ?? part.replace(/^(Key|Digit)/, "")),
    )
    .join("+");
}

export function shortcutTitle(label: string, binding: string | null): string {
  return binding ? `${label} (${formatShortcut(binding)})` : label;
}

export function bindingConflict(
  bindings: Keybindings,
  id: ActionId,
  shortcut: string | null,
): string | undefined {
  if (!shortcut) return;
  const other = Object.entries(bindings).find(
    ([key, value]) => key !== id && value === shortcut,
  )?.[0];
  const label = actions.find((action) => action.id === other)?.label;
  return other ? (label ? t(label) : other) : undefined;
}

export function restoreKeybindings(
  value: unknown,
  mac = false,
  contributions: ShortcutAction[] = [],
): Keybindings {
  const result = defaultKeybindings(mac, contributions);
  if (value === null || value === undefined)
    value = { version: 1, bindings: {} };
  if (
    !value ||
    typeof value !== "object" ||
    !("version" in value) ||
    value.version !== 1 ||
    !("bindings" in value) ||
    !value.bindings ||
    typeof value.bindings !== "object" ||
    Array.isArray(value.bindings) ||
    ("focusFollowsPointer" in value &&
      typeof value.focusFollowsPointer !== "boolean") ||
    ("explorerOpenOnSingleClick" in value &&
      typeof value.explorerOpenOnSingleClick !== "boolean")
  ) {
    throw new Error(
      t(
        "The saved keybindings use an unsupported format. The file has been left intact.",
      ),
    );
  }
  const savedEntries = Object.entries(value.bindings);
  if (
    savedEntries.length > 2048 ||
    new TextEncoder().encode(JSON.stringify(value)).length > 256 * 1024
  )
    throw new Error(
      t(
        "Keybindings exceed the extension settings limit. The file was preserved.",
      ),
    );
  for (const [key] of savedEntries) {
    if (!actions.some((action) => action.id === key) && !key.includes("."))
      continue;
    if (
      key.length > 160 ||
      (!actions.some((action) => action.id === key) &&
        !/^[a-z][a-z0-9-]*(?:\.[a-z][a-z0-9-]*)+$/.test(key))
    )
      throw new Error(t("Invalid saved command ID: {key}", { key }));
    const id = key as ActionId;
    const shortcut = (value.bindings as Record<string, unknown>)[id];
    if (
      shortcut !== null &&
      (typeof shortcut !== "string" || !validShortcut(shortcut))
    )
      throw new Error(
        t(
          "The saved shortcut for {id} is invalid. The file has been left intact.",
          { id },
        ),
      );
    result[id] = shortcut;
  }
  const migrated = new Set<ActionId>();
  if (!Object.hasOwn(value.bindings, "terminalOverview")) {
    const defaults = defaultKeybindings(mac);
    for (const [id, previous] of [
      ["nextTab", mac ? "Meta+Tab" : "Ctrl+Tab"],
      ["previousTab", mac ? "Meta+Shift+Tab" : "Ctrl+Shift+Tab"],
    ] as const) {
      if (result[id] === previous) {
        result[id] = defaults[id];
        migrated.add(id);
      }
    }
  }
  // New defaults must not invalidate existing custom shortcuts.
  for (const { id, group } of actions) {
    if (
      (((group === "Editor" ||
        [
          "commandPicker",
          "terminalOverview",
          "nextTab",
          "previousTab",
        ].includes(id) ||
        isZoomAction(id)) &&
        !Object.hasOwn(value.bindings, id)) ||
        migrated.has(id)) &&
      bindingConflict(result, id, result[id])
    )
      result[id] = null;
  }
  for (const { id } of contributions)
    if (
      !Object.hasOwn(value.bindings, id) &&
      bindingConflict(result, id, result[id])
    )
      result[id] = null;
  for (const [id] of Object.entries(result) as [ActionId, string | null][]) {
    const found = [...actions, ...contributions].find(
      (action) => action.id === id,
    );
    const label = found ? t(found.label) : id;
    const conflict = bindingConflict(result, id, result[id]);
    if (conflict)
      throw new Error(
        t(
          "The saved shortcuts for {label} and {conflict} conflict. The file has been left intact.",
          { label, conflict },
        ),
      );
  }
  return result;
}

export function isTextInput(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    !target.classList.contains("xterm-helper-textarea") &&
    !!target.closest(
      "input, textarea, select, [role='combobox'], [contenteditable]:not([contenteditable='false'])",
    )
  );
}
