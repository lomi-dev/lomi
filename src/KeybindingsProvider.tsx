import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, macOS, native } from "./api";
import { useT } from "./i18n";
import { usePlugins } from "./plugins/PluginsProvider";
import { defaultKeybindings, restoreKeybindings, actions } from "./keybindings";
import type {
  ActionId,
  ShortcutAction,
  Keybindings,
  KeybindingSettings,
} from "./keybindings";

interface Preferences {
  actions: readonly ShortcutAction[];
  bindings: Keybindings;
  defaults: Keybindings;
  focusFollowsPointer: boolean;
  explorerOpenOnSingleClick: boolean;
  ready: boolean;
  error: string;
  reload: () => Promise<void>;
  save: (
    bindings: Keybindings,
    focusFollowsPointer?: boolean,
    explorerOpenOnSingleClick?: boolean,
  ) => Promise<void>;
}
const Context = createContext<Preferences | null>(null);
const builtinDefaults = defaultKeybindings(macOS);

export function KeybindingsProvider({ children }: { children: ReactNode }) {
  const { t } = useT();
  const { catalog } = usePlugins();
  const contributions: ShortcutAction[] = catalog.entries.flatMap(
    (entry) =>
      entry.manifest?.contributes?.commands?.map((command) => ({
        id: command.id as ActionId,
        label: command.label,
        description: command.description,
        group: entry.manifest!.name,
        shortcut:
          entry.manifest?.contributes?.keybindings?.find(
            (binding) => binding.command === command.id,
          )?.shortcut ?? null,
      })) ?? [],
  );
  const contributionKey = JSON.stringify(contributions);
  const defaults = restoreKeybindings(
    { version: 1, bindings: {} },
    macOS,
    contributions,
  );
  for (const action of contributions)
    if (action.shortcut && defaults[action.id] === null)
      action.description = t(
        "{description} Default {shortcut} conflicts with another command; assign a free shortcut.",
        { description: action.description, shortcut: action.shortcut },
      );
  const [bindings, setBindings] = useState(builtinDefaults);
  const [focusFollowsPointer, setFocusFollowsPointer] = useState(false);
  const [explorerOpenOnSingleClick, setExplorerOpenOnSingleClick] =
    useState(false);
  const [ready, setReady] = useState(!native);
  const [error, setError] = useState("");
  const revision = useRef(0);
  const mounted = useRef(false);
  const reload = useCallback(async () => {
    if (!native) return;
    const request = ++revision.current;
    try {
      const data = await api<KeybindingSettings | null>("load_keybindings");
      const value = restoreKeybindings(data, macOS, contributions);
      if (mounted.current && request === revision.current) {
        setBindings(value);
        setFocusFollowsPointer(data?.focusFollowsPointer ?? false);
        setExplorerOpenOnSingleClick(data?.explorerOpenOnSingleClick ?? false);
        setError("");
      }
    } catch (error) {
      if (mounted.current && request === revision.current)
        setError(errorMessage(error));
    } finally {
      if (mounted.current && request === revision.current) setReady(true);
    }
  }, [contributionKey]);
  useEffect(() => {
    mounted.current = true;
    if (!native)
      return () => {
        mounted.current = false;
      };
    let current = true;
    const unlisten = listen("keybindings-changed", () => void reload());
    void unlisten
      .then(() => {
        if (current) void reload();
      })
      .catch((error) => {
        if (current) {
          setError(errorMessage(error));
          setReady(true);
        }
      });
    const focused = () => void reload();
    window.addEventListener("focus", focused);
    return () => {
      current = false;
      mounted.current = false;
      ++revision.current;
      window.removeEventListener("focus", focused);
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [reload]);
  const save = async (
    next: Keybindings,
    pointerFocus = focusFollowsPointer,
    explorerSingleClick = explorerOpenOnSingleClick,
  ) => {
    const data = {
      version: 1,
      bindings: next,
      focusFollowsPointer: pointerFocus,
      explorerOpenOnSingleClick: explorerSingleClick,
    };
    restoreKeybindings(data, macOS, contributions);
    if (native) await api("save_keybindings", { data });
    if (mounted.current) {
      ++revision.current;
      setBindings(next);
      setFocusFollowsPointer(pointerFocus);
      setExplorerOpenOnSingleClick(explorerSingleClick);
      setError("");
    }
  };
  return (
    <Context.Provider
      value={{
        actions: [...actions, ...contributions],
        bindings,
        defaults,
        focusFollowsPointer,
        explorerOpenOnSingleClick,
        ready,
        error,
        reload,
        save,
      }}
    >
      {children}
    </Context.Provider>
  );
}

export function useKeybindings() {
  const preferences = useContext(Context);
  if (!preferences) throw new Error("KeybindingsProvider is missing.");
  return preferences;
}
