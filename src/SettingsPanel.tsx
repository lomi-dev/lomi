import {
  Fragment,
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { RotateCcw, Search, X } from "./icons";
import SettingsNavigation from "./SettingsNavigation";
import AboutSettingsPage from "./AboutSettingsPage";
import TerminalSettingsPage from "./TerminalSettingsPage";
import { DisclosureSummary, IconButton } from "./ui";
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
  remoteAvailable: boolean;
  remoteDisabledReason?: string;
}

export default function SettingsPanel({
  active,
  requestedPage,
  requestRevision,
  remoteAvailable,
  remoteDisabledReason = "Remote is disabled in this version of Lomi.",
}: SettingsPanelProps) {
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
  const [shortcutQuery, setShortcutQuery] = useState("");
  const shortcutSearch = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const busyRef = useRef(false);
  const recordingButton = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => {
    setRecording(null);
    if (requestedPage !== null) setPage(requestedPage);
  }, [requestedPage, requestRevision]);
  useLayoutEffect(() => {
    if (!active) setRecording(null);
  }, [active]);
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
    setRecording(null);
    try {
      await preferences.save(
        bindings,
        focusFollowsPointer,
        explorerOpenOnSingleClick,
      );
    } catch (error) {
      setError(errorMessage(error));
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
  const shortcutTerms = shortcutQuery
    .toLocaleLowerCase()
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  const filteredActions = preferences.actions.filter((action) =>
    shortcutTerms.every((term) =>
      `${action.label} ${action.description} ${action.group} ${formatShortcut(preferences.bindings[action.id])}`
        .toLocaleLowerCase()
        .includes(term),
    ),
  );
  return (
    <div className="settings-panel">
      <div className="settings-layout">
        <SettingsNavigation
          active={active}
          page={page}
          requestRevision={requestRevision}
          remoteAvailable={remoteAvailable}
          remoteDisabledReason={remoteDisabledReason}
          onSelect={setPage}
        />
        {page === "notifications" ? (
          <Suspense fallback={<SettingsLoading title="Notifications" />}>
            <NotificationSettingsPage />
          </Suspense>
        ) : page === "remote" ? (
          <Suspense fallback={<SettingsLoading title="Remote" />}>
            <RemoteSettingsPage
              available={remoteAvailable}
              disabledReason={remoteDisabledReason}
            />
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
            description="Click a shortcut to record new keys. Escape cancels. Cleared shortcuts pass their keys to the terminal."
            busy={!preferences.ready || busy}
          >
            {(error || preferences.error) && (
              <SettingsNotice
                tone="error"
                action={
                  preferences.error && (
                    <>
                      <button
                        className="text-button"
                        onClick={() => void preferences.reload()}
                      >
                        Retry loading
                      </button>
                      <button
                        className="text-button"
                        disabled={!preferences.ready || busy}
                        onClick={() =>
                          void persist(preferences.defaults, false, false)
                        }
                      >
                        Reset to defaults
                      </button>
                    </>
                  )
                }
              >
                {error || preferences.error}
              </SettingsNotice>
            )}
            <div className="settings-shortcut-search">
              <Search size={14} aria-hidden="true" />
              <input
                ref={shortcutSearch}
                type="text"
                aria-label="Search shortcuts"
                placeholder="Search shortcuts…"
                value={shortcutQuery}
                onChange={(event) => {
                  setRecording(null);
                  setShortcutQuery(event.target.value);
                }}
                onKeyDown={(event) => {
                  if (event.key !== "Escape" || event.nativeEvent.isComposing)
                    return;
                  event.preventDefault();
                  event.stopPropagation();
                  setShortcutQuery("");
                }}
              />
            </div>
            {shortcutTerms.length === 0 && (
              <>
                <SettingsSection title="Interaction">
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
                      disabled={
                        !preferences.ready || busy || !!preferences.error
                      }
                      onChange={(event) =>
                        void persist(preferences.bindings, event.target.checked)
                      }
                    />
                  </SettingRow>
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
                      disabled={
                        !preferences.ready || busy || !!preferences.error
                      }
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
              </>
            )}
            {filteredActions.length === 0 && preferences.ready && (
              <div className="settings-shortcut-empty">
                <p role="status">No matching shortcuts.</p>
                <button
                  className="text-button"
                  type="button"
                  onClick={() => {
                    setShortcutQuery("");
                    shortcutSearch.current?.focus();
                  }}
                >
                  Clear shortcut search
                </button>
              </div>
            )}
            {[...new Set(filteredActions.map((action) => action.group))].map(
              (group) => (
                <details
                  className="settings-disclosure keybinding-group"
                  key={group}
                  open={shortcutTerms.length > 0 || group === "Terminals"}
                >
                  <DisclosureSummary>
                    {group}
                    <span className="settings-count">
                      {
                        filteredActions.filter(
                          (action) => action.group === group,
                        ).length
                      }
                    </span>
                  </DisclosureSummary>
                  <div className="settings-disclosure-body">
                    {filteredActions
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
                                !preferences.ready ||
                                busy ||
                                !!preferences.error
                              }
                              onClick={() => {
                                setRecording(action.id);
                                setError("");
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
                              {recording === action.id ? (
                                <kbd>Press keys…</kbd>
                              ) : preferences.bindings[action.id] ? (
                                preferences.bindings[action.id]
                                  ?.split("+")
                                  .map((key, index) => (
                                    <Fragment key={index}>
                                      {index > 0 && (
                                        <span className="shortcut-separator">
                                          +
                                        </span>
                                      )}
                                      <kbd>{formatShortcut(key)}</kbd>
                                    </Fragment>
                                  ))
                              ) : (
                                <span className="shortcut-unassigned">
                                  Not set
                                </span>
                              )}
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
                                assign(
                                  action.id,
                                  preferences.defaults[action.id],
                                )
                              }
                            >
                              <RotateCcw size={14} />
                            </IconButton>
                          </div>
                        </SettingRow>
                      ))}
                  </div>
                </details>
              ),
            )}
          </SettingsPage>
        )}
      </div>
    </div>
  );
}
