import { keyNames, validShortcut } from "@lomi-dev/plugin-sdk/shortcuts";
export { validShortcut } from "@lomi-dev/plugin-sdk/shortcuts";
export const actions = [
  {
    id: "chatNew",
    label: "New Chat AI",
    description: "Open a new AI conversation.",
    group: "Chat AI",
    shortcut: null,
  },
  {
    id: "chatFocusInput",
    label: "Focus chat input",
    description: "Focus the active conversation composer.",
    group: "Chat AI",
    shortcut: null,
  },
  {
    id: "chatStop",
    label: "Stop AI response",
    description:
      "Stop the active conversation request and keep its partial response.",
    group: "Chat AI",
    shortcut: null,
  },
  {
    id: "chatHistory",
    label: "Chat history",
    description: "Search saved conversations in the active chat panel.",
    group: "Chat AI",
    shortcut: null,
  },
  {
    id: "commandPicker",
    label: "Show commands",
    description: "Find and run a workspace or plugin command.",
    group: "Workspace",
    shortcut: "Ctrl+Shift+KeyP",
  },
  {
    id: "saveFile",
    label: "Save file",
    description: "Save the active editor file.",
    group: "Editor",
    shortcut: "Ctrl+KeyS",
  },
  {
    id: "findFile",
    label: "Find in file",
    description: "Find and replace text in the active editor.",
    group: "Editor",
    shortcut: "Ctrl+KeyF",
  },
  {
    id: "goToLine",
    label: "Go to line",
    description: "Jump to a line in the active editor.",
    group: "Editor",
    shortcut: "Ctrl+KeyG",
  },
  {
    id: "toggleWordWrap",
    label: "Toggle word wrap",
    description: "Wrap long lines in the active editor.",
    group: "Editor",
    shortcut: "Alt+KeyZ",
  },
  {
    id: "terminalOverview",
    label: "Toggle terminal overview",
    description: "Show terminal titles in place of their contents.",
    group: "Terminals",
    shortcut: "Ctrl+Tab",
  },
  {
    id: "newTerminal",
    label: "New terminal",
    description: "Split the active panel side by side.",
    group: "Terminals",
    shortcut: "Ctrl+KeyD",
  },
  {
    id: "splitVertical",
    label: "Split terminal vertically",
    description: "Place a terminal below the active panel.",
    group: "Terminals",
    shortcut: "Ctrl+Shift+KeyD",
  },
  {
    id: "closeTerminal",
    label: "Close terminal",
    description: "Close the active panel.",
    group: "Terminals",
    shortcut: "Ctrl+KeyW",
  },
  {
    id: "searchTerminal",
    label: "Find in terminal",
    description: "Search the active terminal's output.",
    group: "Terminals",
    shortcut: "Ctrl+Shift+KeyF",
  },
  {
    id: "copyTerminal",
    label: "Copy terminal selection",
    description: "Copy selected terminal text.",
    group: "Terminals",
    shortcut: "Ctrl+Shift+KeyC",
  },
  {
    id: "pasteTerminal",
    label: "Paste into terminal",
    description: "Paste clipboard text or images into the terminal.",
    group: "Terminals",
    shortcut: "Ctrl+Shift+KeyV",
  },
  {
    id: "commandInput",
    label: "Command input",
    description: "Show or hide the multiline command input.",
    group: "Terminals",
    shortcut: "Ctrl+Shift+KeyI",
  },
  {
    id: "runCommand",
    label: "Run command input",
    description: "Run a command while the command input is focused.",
    group: "Terminals",
    shortcut: "Ctrl+Enter",
  },
  {
    id: "commandBlocks",
    label: "Command blocks",
    description: "Show or hide command history and output positions.",
    group: "Terminals",
    shortcut: "Ctrl+Shift+KeyH",
  },
  {
    id: "changeEnvironment",
    label: "Change terminal environment",
    description: "Choose the shell or WSL distribution for the active tab.",
    group: "Terminals",
    shortcut: "Ctrl+Shift+KeyL",
  },
  {
    id: "newTab",
    label: "New tab",
    description: "Open a tab with one terminal.",
    group: "Tabs",
    shortcut: "Ctrl+Shift+KeyT",
  },
  {
    id: "closeTab",
    label: "Close tab",
    description: "Close the active tab and all its terminals.",
    group: "Tabs",
    shortcut: "Ctrl+Shift+KeyW",
  },
  {
    id: "nextTab",
    label: "Next tab",
    description: "Switch to the next tab in this workspace.",
    group: "Tabs",
    shortcut: "Ctrl+PageDown",
  },
  {
    id: "previousTab",
    label: "Previous tab",
    description: "Switch to the previous tab in this workspace.",
    group: "Tabs",
    shortcut: "Ctrl+PageUp",
  },
  {
    id: "toggleExplorer",
    label: "Toggle file explorer",
    description: "Show or hide the file explorer.",
    group: "Workspace",
    shortcut: "Ctrl+Shift+KeyE",
  },
  {
    id: "toggleSourceControl",
    label: "Toggle source control",
    description: "Show or hide Source Control when Git is available.",
    group: "Workspace",
    shortcut: "Ctrl+Shift+KeyG",
  },
  {
    id: "toggleWorkspaces",
    label: "Toggle workspaces",
    description: "Show or hide the workspace list for all folders.",
    group: "Workspace",
    shortcut: null,
  },
  {
    id: "openSettings",
    label: "Open settings",
    description: "Open the settings window.",
    group: "Workspace",
    shortcut: "Ctrl+Comma",
  },
  {
    id: "zoomIn",
    label: "Zoom in",
    description:
      "Increase the size of the entire interface, including terminals and editors.",
    group: "Workspace",
    shortcut: "Ctrl+Equal",
  },
  {
    id: "zoomOut",
    label: "Zoom out",
    description:
      "Decrease the size of the entire interface, including terminals and editors.",
    group: "Workspace",
    shortcut: "Ctrl+Minus",
  },
  {
    id: "resetZoom",
    label: "Reset zoom",
    description: "Restore the interface to its original size (100%).",
    group: "Workspace",
    shortcut: "Ctrl+Digit0",
  },
  {
    id: "movePanel",
    label: "Move active panel",
    description: "Choose another panel and a docking side in this tab.",
    group: "Workspace",
    shortcut: null,
  },
  {
    id: "dockTab",
    label: "Dock current tab",
    description: "Dock this tab into another terminal tab.",
    group: "Workspace",
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
  if (!shortcut) return "Not set";
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
  return other
    ? (actions.find((action) => action.id === other)?.label ?? other)
    : undefined;
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
      typeof value.focusFollowsPointer !== "boolean")
  ) {
    throw new Error(
      "The saved keybindings use an unsupported format. The file has been left intact.",
    );
  }
  const savedEntries = Object.entries(value.bindings);
  if (
    savedEntries.length > 2048 ||
    new TextEncoder().encode(JSON.stringify(value)).length > 256 * 1024
  )
    throw new Error(
      "Keybindings exceed the extension settings limit. The file was preserved.",
    );
  for (const [key] of savedEntries) {
    if (!actions.some((action) => action.id === key) && !key.includes("."))
      continue;
    if (
      key.length > 160 ||
      (!actions.some((action) => action.id === key) &&
        !/^[a-z][a-z0-9-]*(?:\.[a-z][a-z0-9-]*)+$/.test(key))
    )
      throw new Error(`Invalid saved command ID: ${key}`);
    const id = key as ActionId;
    const shortcut = (value.bindings as Record<string, unknown>)[id];
    if (
      shortcut !== null &&
      (typeof shortcut !== "string" || !validShortcut(shortcut))
    )
      throw new Error(
        `The saved shortcut for ${id} is invalid. The file has been left intact.`,
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
    const label =
      [...actions, ...contributions].find((action) => action.id === id)
        ?.label ?? id;
    const conflict = bindingConflict(result, id, result[id]);
    if (conflict)
      throw new Error(
        `The saved shortcuts for ${label} and ${conflict} conflict. The file has been left intact.`,
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
