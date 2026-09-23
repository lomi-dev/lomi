import { useEditorPreferences } from "./EditorPreferencesProvider";
import { useTerminalPreferences } from "./TerminalPreferencesProvider";
import { useKeybindings } from "./KeybindingsProvider";
import { useThemes } from "./ThemeProvider";
import { macOS } from "./api";
import {
  stageKeybindingUpdate,
  type KeybindingSource,
  type KeybindingPatch,
} from "./agent-keybindings";

export interface SettingsReadInput {
  workspaceId: string;
  section: "editor" | "terminal" | "keybinds" | "themes";
  offset: number;
  limit: number;
  expectedRevision: string | null;
}
export interface SettingsReadRequest {
  requestId: string;
  uiEpoch: string;
  projectId: string;
  input: SettingsReadInput;
}

// Read the same retained preference providers used by the workbench. A read
// never reloads, repairs, saves or imports a settings file or plugin package.
export function useAgentSettingsReader() {
  const editor = useEditorPreferences();
  const terminal = useTerminalPreferences();
  const keys = useKeybindings();
  const themes = useThemes();
  const read = async (input: SettingsReadInput) => {
    const provider = { editor, terminal, keybinds: keys, themes }[
      input.section
    ];
    if (!provider?.ready) throw Error("UI_NOT_READY");
    const readiness = provider.error ? "recovery_required" : "ready";
    let values: Record<string, unknown>;
    switch (input.section) {
      case "editor":
        values = {
          section: "editor",
          tabSize: editor.value.tabSize,
          insertSpaces: editor.value.insertSpaces,
        };
        break;
      case "terminal":
        values = {
          section: "terminal",
          appearanceOverrides: terminal.value.appearance,
          behavior: terminal.value.behavior,
          windowsShell: terminal.value.windowsShell,
          agentNotifications: terminal.value.agentNotifications,
          alwaysShowTitles: terminal.value.alwaysShowTitles,
        };
        break;
      case "keybinds": {
        const entries = Object.entries(keys.bindings).sort(([a], [b]) =>
          a < b ? -1 : a > b ? 1 : 0,
        );
        if (
          entries.length > 4096 ||
          entries.some(
            ([id, value]) =>
              id.length > 160 || (value !== null && value.length > 64),
          )
        )
          throw Error("RESOURCE_EXHAUSTED");
        values = {
          section: "keybinds",
          focusFollowsPointer: keys.focusFollowsPointer,
          items: entries.map(([action, shortcut]) => ({
            action,
            shortcut,
            defaultShortcut:
              keys.defaults[action as keyof typeof keys.defaults] ?? null,
          })),
        };
        break;
      }
      case "themes":
        values = {
          section: "themes",
          active: themes.preferences.active,
          fileIcons: themes.preferences.fileIcons ?? null,
          productIcons: themes.preferences.productIcons ?? null,
          appearance: themes.preferences.appearance,
          effectiveAppearance: themes.snapshot.appearance,
          safeMode: themes.safeMode,
        };
        break;
    }
    // Copy before the asynchronous hash so a subsequent provider render cannot
    // change the snapshot or its pagination independently from the revision.
    values = structuredClone(values);
    const bytes = new TextEncoder().encode(
      JSON.stringify({ readiness, values }),
    );
    if (bytes.length > 1024 * 1024) throw Error("RESOURCE_EXHAUSTED");
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    const revision = [...new Uint8Array(digest)]
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
    if (input.expectedRevision && input.expectedRevision !== revision)
      throw Error("REVISION_CONFLICT");
    if (input.section === "keybinds") {
      const entries = values.items as unknown[];
      if (input.offset > entries.length) throw Error("REVISION_CONFLICT");
      const items = entries.slice(input.offset, input.offset + input.limit);
      const end = input.offset + items.length;
      values = {
        ...values,
        items,
        total: entries.length,
        offset: input.offset,
        nextOffset: end < entries.length ? end : null,
      };
    }
    const snapshot = {
      workspaceId: input.workspaceId,
      section: input.section,
      revision,
      readiness,
      values,
    };
    if (new TextEncoder().encode(JSON.stringify(snapshot)).length > 48 * 1024)
      throw Error("RESOURCE_EXHAUSTED");
    return snapshot;
  };
  return Object.assign(read, {
    prepareKeybinds: (source: KeybindingSource, patch: KeybindingPatch) =>
      stageKeybindingUpdate(source, patch, keys, macOS),
  });
}
