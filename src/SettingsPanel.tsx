import {
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  Bell,
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
import AboutSettingsPage from "./AboutSettingsPage";
import TerminalSettingsPage from "./TerminalSettingsPage";
import { IconButton } from "./ui";
import { errorMessage } from "./api";
import {
  bindingConflict,
  formatShortcut,
  shortcutFromEvent,
} from "./keybindings";
import type { ActionId, Keybindings } from "./keybindings";
import { useKeybindings } from "./KeybindingsProvider";
import { useProtectedTheme } from "./useProtectedTheme";
import { defaultSettingsPage } from "./settings-pages";
import type { SettingsPageId } from "./settings-pages";
import { canShareRemotely } from "./auth/model";
import { useAuthState } from "./auth/useAuthState";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";

const ChatSettingsPage = lazy(() => import("./chat/ChatSettingsPage"));
const PluginsPage = lazy(() => import("./plugins/PluginsPage"));
const ThemesPage = lazy(() => import("./ThemesPage"));
const NotificationSettingsPage = lazy(
  () => import("./auth/NotificationSettingsPage"),
);
const RemoteSettingsPage = lazy(() => import("./RemoteSettingsPage"));
const AccountSettingsPage = lazy(() => import("./auth/AccountSettingsPage"));
const AndroidSettingsPage = lazy(() => import("./android/AndroidSettingsPage"));
const AgentControlSettingsPage = lazy(
  () => import("./AgentControlSettingsPage"),
);

function SettingsLoading({ title }: { title: string }) {
  return <SettingsPage title={title} status="Loading…" />;
}

export interface SettingsPanelProps {
  active: boolean;
  requestedPage: SettingsPageId | null;
  requestRevision: number;
}

export default function SettingsPanel({
  active,
  requestedPage,
  requestRevision,
}: SettingsPanelProps) {
  const auth = useAuthState();
  const remoteAvailable = canShareRemotely(auth.state);
  const [page, setPage] = useState<SettingsPageId>(
    requestedPage ?? defaultSettingsPage,
  );
  const preferences = useKeybindings();
  useProtectedTheme(
    active &&
      (page === "agent-control" ||
        page === "account" ||
        page === "remote" ||
        page === "notifications"),
  );
  const [recording, setRecording] = useState<ActionId | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const busyRef = useRef(false);
  const recordingButton = useRef<HTMLButtonElement>(null);
  const navigation = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    setRecording(null);
    if (requestedPage !== null) setPage(requestedPage);
  }, [requestedPage, requestRevision]);
  useLayoutEffect(() => {
    if (!active) setRecording(null);
  }, [active]);
  useLayoutEffect(() => {
    if (active && !document.querySelector("dialog[open]"))
      navigation.current
        ?.querySelector<HTMLButtonElement>('[aria-current="page"]')
        ?.focus({ preventScroll: true });
  }, [active, page, requestRevision]);
  useEffect(() => {
    const blurred = () => setRecording(null);
    window.addEventListener("blur", blurred);
    return () => window.removeEventListener("blur", blurred);
  }, []);
  useEffect(() => {
    if (active) recordingButton.current?.focus();
  }, [active, recording]);

  const persist = async (
    bindings: Keybindings,
    focusFollowsPointer = preferences.focusFollowsPointer,
    explorerOpenOnSingleClick = preferences.explorerOpenOnSingleClick,
  ) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError("");
    setStatus("Saving…");
    setRecording(null);
    try {
      await preferences.save(
        bindings,
        focusFollowsPointer,
        explorerOpenOnSingleClick,
      );
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
    <div className="settings-panel">
      <div className="settings-layout">
        <nav
          ref={navigation}
          className="settings-navigation"
          aria-label="Settings pages"
        >
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
            aria-current={page === "notifications" ? "page" : undefined}
            onClick={() => setPage("notifications")}
          >
            <Bell size={16} aria-hidden="true" />
            Notifications
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
            className="settings-nav-item"
            aria-current={page === "remote" ? "page" : undefined}
            disabled={!remoteAvailable}
            title={
              remoteAvailable ? undefined : "Sign in to use remote sharing"
            }
            onClick={() => setPage("remote")}
          >
            <Monitor size={16} aria-hidden="true" />
            Remote
          </button>
          <button
            className="settings-nav-item settings-nav-account"
            aria-current={page === "account" ? "page" : undefined}
            onClick={() => setPage("account")}
          >
            <ShieldCheck size={16} aria-hidden="true" />
            Account
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
        {page === "notifications" ? (
          <Suspense fallback={<SettingsLoading title="Notifications" />}>
            <NotificationSettingsPage />
          </Suspense>
        ) : page === "remote" ? (
          <Suspense fallback={<SettingsLoading title="Remote" />}>
            <RemoteSettingsPage available={remoteAvailable} />
          </Suspense>
        ) : page === "account" ? (
          <Suspense fallback={<SettingsLoading title="Account" />}>
            <AccountSettingsPage />
          </Suspense>
        ) : page === "agent-control" ? (
          <Suspense fallback={<SettingsLoading title="Agent control" />}>
            <AgentControlSettingsPage active={active} />
          </Suspense>
        ) : page === "android" ? (
          <Suspense fallback={<SettingsLoading title="Android" />}>
            <AndroidSettingsPage />
          </Suspense>
        ) : page === "chat-ai" ? (
          <Suspense fallback={<SettingsLoading title="Chat AI" />}>
            <ChatSettingsPage />
          </Suspense>
        ) : page === "about" ? (
          <AboutSettingsPage
            error={error}
            onError={setError}
            onClearError={() => setError("")}
          />
        ) : page === "plugins" ? (
          <Suspense fallback={<SettingsLoading title="Plugins" />}>
            <PluginsPage />
          </Suspense>
        ) : page === "terminal" ? (
          <TerminalSettingsPage />
        ) : page === "themes" ? (
          <Suspense fallback={<SettingsLoading title="Themes" />}>
            <ThemesPage />
          </Suspense>
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
                onClick={() => void persist(preferences.defaults, false, false)}
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
            <SettingsSection title="File Explorer">
              <SettingRow
                label="Open with one click"
                htmlFor="explorer-open-on-single-click"
                description="On: one click opens after a brief pause; a double click selects without opening. Off: one click selects and a double click opens."
                descriptionId="explorer-click-mode-help"
              >
                <input
                  id="explorer-open-on-single-click"
                  className="settings-switch"
                  type="checkbox"
                  role="switch"
                  aria-describedby="explorer-click-mode-help"
                  checked={preferences.explorerOpenOnSingleClick}
                  disabled={!preferences.ready || busy || !!preferences.error}
                  onChange={(event) =>
                    void persist(
                      preferences.bindings,
                      preferences.focusFollowsPointer,
                      event.target.checked,
                    )
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
