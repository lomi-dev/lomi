import { lazy, Suspense, useEffect, useRef, useState } from "react";
import {
  MessageSquare,
  Monitor,
  Info,
  Keyboard,
  Palette,
  Puzzle,
  Terminal,
  RotateCcw,
  ShieldCheck,
  X,
} from "./icons";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import ChatSettingsPage from "./chat/ChatSettingsPage";
import AboutSettingsPage from "./AboutSettingsPage";
import PluginsPage from "./plugins/PluginsPage";
import ThemesPage from "./ThemesPage";
import TerminalSettingsPage from "./TerminalSettingsPage";
import { IconButton, WindowControls } from "./ui";
import { errorMessage, macOS, native } from "./api";
import {
  bindingConflict,
  formatShortcut,
  shortcutFromEvent,
} from "./keybindings";
import type { ActionId, Keybindings } from "./keybindings";
import { useKeybindings } from "./KeybindingsProvider";
import ReadyWindow from "./ReadyWindow";
import { useWindowZoom } from "./useWindowZoom";
import { useProtectedTheme } from "./useProtectedTheme";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";

const AndroidSettingsPage = lazy(() => import("./android/AndroidSettingsPage"));
const AgentControlSettingsPage = lazy(
  () => import("./AgentControlSettingsPage"),
);

function SettingsLoading({ title }: { title: string }) {
  return <SettingsPage title={title} status="Loading…" />;
}

export default function SettingsWindow() {
  const [page, setPage] = useState(() => {
    const requested = new URLSearchParams(window.location.search).get("page");
    return requested === "android" ||
      requested === "agent-control" ||
      requested === "chat-ai" ||
      requested === "plugins" ||
      requested === "themes" ||
      requested === "terminal" ||
      requested === "about"
      ? requested
      : "keybinds";
  });
  const preferences = useKeybindings();
  useProtectedTheme(page === "agent-control");
  const [listening, setListening] = useState(!native);
  const [recording, setRecording] = useState<ActionId | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useWindowZoom(setError);
  const [status, setStatus] = useState("");
  const busyRef = useRef(false);
  const recordingButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!native) return;
    let current = true;
    const unlisten = listen<string>("settings-page-changed", ({ payload }) => {
      if (
        current &&
        [
          "android",
          "agent-control",
          "chat-ai",
          "keybinds",
          "themes",
          "terminal",
          "about",
          "plugins",
        ].includes(payload)
      ) {
        setRecording(null);
        setPage(payload);
      }
    });
    void unlisten
      .then(() => {
        if (current) setListening(true);
      })
      .catch((error) => {
        if (current) {
          setError(errorMessage(error));
          setListening(true);
        }
      });
    const blurred = () => setRecording(null);
    window.addEventListener("blur", blurred);
    return () => {
      current = false;
      window.removeEventListener("blur", blurred);
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, []);
  useEffect(() => {
    recordingButton.current?.focus();
  }, [recording]);
  useEffect(() => {
    if (!native || !macOS || recording) return;
    const close = (event: KeyboardEvent) => {
      if (
        event.code !== "KeyW" ||
        !event.metaKey ||
        event.ctrlKey ||
        event.altKey ||
        event.shiftKey ||
        event.isComposing ||
        event.defaultPrevented ||
        document.querySelector("dialog[open]")
      )
        return;
      event.preventDefault();
      void getCurrentWindow()
        .close()
        .catch((error) => setError(errorMessage(error)));
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [recording]);

  const persist = async (
    bindings: Keybindings,
    focusFollowsPointer = preferences.focusFollowsPointer,
  ) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError("");
    setStatus("Saving…");
    setRecording(null);
    try {
      await preferences.save(bindings, focusFollowsPointer);
      setStatus("Saved");
    } catch (error) {
      setError(errorMessage(error));
      setStatus("");
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };
  const assign = (id: ActionId, shortcut: string | null) => {
    const conflict = bindingConflict(preferences.bindings, id, shortcut);
    if (conflict) {
      setError(
        `${formatShortcut(shortcut)} is already assigned to “${conflict}”. Change or clear that shortcut first.`,
      );
      return;
    }
    void persist({ ...preferences.bindings, [id]: shortcut });
  };
  return (
    <div className="app-shell settings-window">
      {listening && <ReadyWindow />}
      <header className="titlebar" data-tauri-drag-region>
        <span className="settings-title" data-tauri-drag-region>
          Settings
        </span>
        <div className="titlebar-space" data-tauri-drag-region />
        <WindowControls onError={setError} />
      </header>
      <div className="settings-layout">
        <nav className="settings-navigation" aria-label="Settings pages">
          <button
            className="settings-nav-item"
            aria-current={page === "keybinds" ? "page" : undefined}
            onClick={() => setPage("keybinds")}
          >
            <Keyboard size={16} />
            Keybinds
          </button>
          <button
            className="settings-nav-item"
            aria-current={page === "terminal" ? "page" : undefined}
            onClick={() => setPage("terminal")}
          >
            <Terminal size={16} />
            Terminal
          </button>
          <button
            className="settings-nav-item"
            aria-current={page === "themes" ? "page" : undefined}
            onClick={() => setPage("themes")}
          >
            <Palette size={16} />
            Themes
          </button>
          <button
            className="settings-nav-item"
            aria-current={page === "plugins" ? "page" : undefined}
            onClick={() => setPage("plugins")}
          >
            <Puzzle size={16} />
            Plugins
          </button>
          <button
            className="settings-nav-item"
            aria-current={page === "chat-ai" ? "page" : undefined}
            onClick={() => setPage("chat-ai")}
          >
            <MessageSquare size={16} />
            Chat AI
          </button>
          <button
            className="settings-nav-item"
            aria-current={page === "android" ? "page" : undefined}
            onClick={() => setPage("android")}
          >
            <Monitor size={16} />
            Android
          </button>
          <button
            className="settings-nav-item"
            aria-current={page === "agent-control" ? "page" : undefined}
            onClick={() => setPage("agent-control")}
          >
            <ShieldCheck size={16} />
            Agent control
          </button>
          <button
            className="settings-nav-item settings-nav-about"
            aria-current={page === "about" ? "page" : undefined}
            onClick={() => setPage("about")}
          >
            <Info size={16} />
            About
          </button>
        </nav>
        {page === "agent-control" ? (
          <Suspense fallback={<SettingsLoading title="Agent control" />}>
            <AgentControlSettingsPage />
          </Suspense>
        ) : page === "android" ? (
          <Suspense fallback={<SettingsLoading title="Android" />}>
            <AndroidSettingsPage />
          </Suspense>
        ) : page === "chat-ai" ? (
          <ChatSettingsPage />
        ) : page === "about" ? (
          <AboutSettingsPage
            error={error}
            onError={setError}
            onClearError={() => setError("")}
          />
        ) : page === "plugins" ? (
          <PluginsPage />
        ) : page === "terminal" ? (
          <TerminalSettingsPage />
        ) : page === "themes" ? (
          <ThemesPage />
        ) : (
          <SettingsPage
            title="Keybinds"
            className="keybindings-page"
            description="Click a shortcut to record new keys. Cleared shortcuts pass their keys to the terminal."
            status={
              !preferences.ready
                ? "Loading…"
                : recording
                  ? "Press keys · Escape cancels"
                  : status
            }
            actions={
              <button
                className="button"
                disabled={!preferences.ready || busy}
                onClick={() => void persist(preferences.defaults, false)}
              >
                <RotateCcw size={14} />
                Reset all
              </button>
            }
          >
            {(error || preferences.error) && (
              <SettingsNotice
                tone="error"
                action={
                  preferences.error && (
                    <button
                      className="text-button"
                      onClick={() => void preferences.reload()}
                    >
                      Retry loading
                    </button>
                  )
                }
              >
                {error || preferences.error}
              </SettingsNotice>
            )}
            <SettingsSection title="Panel focus">
              <SettingRow
                label="Focus follows pointer"
                htmlFor="focus-follows-pointer"
                description="Typing, paste and shortcuts go to the panel under the mouse."
                descriptionId="pointer-focus-help"
              >
                <input
                  id="focus-follows-pointer"
                  className="settings-switch"
                  type="checkbox"
                  role="switch"
                  aria-describedby="pointer-focus-help"
                  checked={preferences.focusFollowsPointer}
                  disabled={!preferences.ready || busy || !!preferences.error}
                  onChange={(event) =>
                    void persist(preferences.bindings, event.target.checked)
                  }
                />
              </SettingRow>
            </SettingsSection>
            {[
              ...new Set(preferences.actions.map((action) => action.group)),
            ].map((group) => (
              <SettingsSection title={group} key={group}>
                {preferences.actions
                  .filter((action) => action.group === group)
                  .map((action) => (
                    <SettingRow
                      key={action.id}
                      label={action.label}
                      description={action.description}
                    >
                      <div className="keybinding-controls">
                        <button
                          ref={
                            recording === action.id
                              ? recordingButton
                              : undefined
                          }
                          className={`shortcut-recorder${recording === action.id ? " recording" : ""}`}
                          aria-label={`Shortcut for ${action.label}`}
                          disabled={
                            !preferences.ready || busy || !!preferences.error
                          }
                          onClick={() => {
                            setRecording(action.id);
                            setError("");
                            setStatus("");
                          }}
                          onBlur={() => {
                            if (recording === action.id) setRecording(null);
                          }}
                          onKeyDown={(event) => {
                            if (recording !== action.id) return;
                            event.preventDefault();
                            event.stopPropagation();
                            if (
                              event.key === "Escape" &&
                              !event.ctrlKey &&
                              !event.altKey &&
                              !event.metaKey &&
                              !event.shiftKey
                            ) {
                              setRecording(null);
                              setError("");
                              return;
                            }
                            if (
                              event.repeat ||
                              ["Control", "Shift", "Alt", "Meta"].includes(
                                event.key,
                              )
                            )
                              return;
                            const shortcut = shortcutFromEvent(
                              event.nativeEvent,
                            );
                            if (!shortcut) {
                              setError(
                                "Use Ctrl, Alt, or Cmd with a key, or use a function key.",
                              );
                              return;
                            }
                            assign(action.id, shortcut);
                          }}
                        >
                          <kbd>
                            {recording === action.id
                              ? "Press keys…"
                              : formatShortcut(preferences.bindings[action.id])}
                          </kbd>
                        </button>
                        <IconButton
                          title={`Clear shortcut for ${action.label}`}
                          disabled={
                            !preferences.ready ||
                            busy ||
                            !!preferences.error ||
                            !preferences.bindings[action.id]
                          }
                          onClick={() => assign(action.id, null)}
                        >
                          <X size={14} />
                        </IconButton>
                        <IconButton
                          title={`Reset shortcut for ${action.label}`}
                          disabled={
                            !preferences.ready ||
                            busy ||
                            !!preferences.error ||
                            preferences.bindings[action.id] ===
                              preferences.defaults[action.id]
                          }
                          onClick={() =>
                            assign(action.id, preferences.defaults[action.id])
                          }
                        >
                          <RotateCcw size={14} />
                        </IconButton>
                      </div>
                    </SettingRow>
                  ))}
              </SettingsSection>
            ))}
          </SettingsPage>
        )}
      </div>
    </div>
  );
}
