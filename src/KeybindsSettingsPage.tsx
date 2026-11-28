import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { t as translate, useT } from "./i18n";
import { Plus, RotateCcw, Search, X } from "./icons";
import { IconButton } from "./ui";
import { api, errorMessage } from "./api";
import Keycaps from "./Keycaps";
import Select from "./Select";
import {
  bindingConflict,
  formatShortcut,
  shortcutFromEvent,
} from "./keybindings";
import type { ActionId, Keybindings } from "./keybindings";
import { useKeybindings } from "./KeybindingsProvider";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";

type Recording = ActionId | "new" | null;

/** Turns a key press in a recorder into a shortcut, a cancel, or an error. */
function recordKey(
  event: KeyboardEvent<HTMLButtonElement>,
  onShortcut: (shortcut: string) => void,
  onCancel: () => void,
  onError: (message: string) => void,
) {
  event.preventDefault();
  event.stopPropagation();
  if (
    event.key === "Escape" &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.metaKey &&
    !event.shiftKey
  ) {
    onCancel();
    return;
  }
  if (event.repeat || ["Control", "Shift", "Alt", "Meta"].includes(event.key))
    return;
  const shortcut = shortcutFromEvent(event.nativeEvent);
  if (!shortcut) {
    onError(
      translate("Use Ctrl, Alt, or Cmd with a key, or use a function key."),
    );
    return;
  }
  onShortcut(shortcut);
}

export default function KeybindsSettingsPage({ active }: { active: boolean }) {
  const { t } = useT();
  const preferences = useKeybindings();
  const [dictation, setDictation] = useState(false);
  useEffect(() => {
    api<{ available: boolean }>("stt_status").then(
      (status) => setDictation(status.available),
      () => {},
    );
  }, []);
  const [recording, setRecording] = useState<Recording>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [draft, setDraft] = useState<{
    action: ActionId | "";
    shortcut: string | null;
  } | null>(null);
  const busyRef = useRef(false);
  const search = useRef<HTMLInputElement>(null);
  const recordingButton = useRef<HTMLButtonElement>(null);
  const locked = !preferences.ready || busy || !!preferences.error;

  useEffect(() => {
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
    if (busyRef.current) return false;
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
      return true;
    } catch (cause) {
      setError(errorMessage(cause));
      return false;
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  const conflictMessage = (id: ActionId, shortcut: string | null) => {
    const conflict = bindingConflict(preferences.bindings, id, shortcut);
    return conflict
      ? t(
          "{shortcut} is already assigned to “{conflict}”. Change or remove that keybinding first.",
          { shortcut: formatShortcut(shortcut), conflict },
        )
      : "";
  };

  const assign = (id: ActionId, shortcut: string | null) => {
    const conflict = conflictMessage(id, shortcut);
    if (conflict) {
      setError(conflict);
      return;
    }
    void persist({ ...preferences.bindings, [id]: shortcut });
  };

  const terms = query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
  // Dictation exists only in builds with the `stt` feature.
  const actions = preferences.actions.filter(
    (action) => action.id !== "toggleDictation" || dictation,
  );
  // Only assigned keybindings are listed; unassigned actions are added explicitly.
  const assigned = actions.filter(
    (action) =>
      preferences.bindings[action.id] &&
      terms.every((term) =>
        `${t(action.label)} ${t(action.description)} ${t(action.group)} ${formatShortcut(preferences.bindings[action.id])}`
          .toLocaleLowerCase()
          .includes(term),
      ),
  );
  const unassigned = actions.filter(
    (action) => !preferences.bindings[action.id],
  );
  const groups = [...new Set(assigned.map((action) => action.group))];

  const openDraft = () => {
    setError("");
    setDraft({ action: "", shortcut: null });
    setRecording(null);
  };
  const saveDraft = async () => {
    if (!draft?.action || !draft.shortcut) return;
    const conflict = conflictMessage(draft.action, draft.shortcut);
    if (conflict) {
      setError(conflict);
      return;
    }
    if (
      await persist({
        ...preferences.bindings,
        [draft.action]: draft.shortcut,
      })
    )
      setDraft(null);
  };

  return (
    <SettingsPage
      title={t("Keybinds")}
      className="keybindings-page"
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
                  {t("Retry loading")}
                </button>
                <button
                  className="text-button"
                  disabled={!preferences.ready || busy}
                  onClick={() =>
                    void persist(preferences.defaults, false, false)
                  }
                >
                  {t("Reset to defaults")}
                </button>
              </>
            )
          }
        >
          {error || preferences.error}
        </SettingsNotice>
      )}

      <div className="keybindings-toolbar">
        <div className="settings-shortcut-search">
          <Search size={14} aria-hidden="true" />
          <input
            ref={search}
            type="text"
            aria-label={t("Search keybindings")}
            placeholder={t("Search keybindings")}
            value={query}
            onChange={(event) => {
              setRecording(null);
              setQuery(event.target.value);
            }}
            onKeyDown={(event) => {
              if (event.key !== "Escape" || event.nativeEvent.isComposing)
                return;
              event.preventDefault();
              event.stopPropagation();
              setQuery("");
            }}
          />
        </div>
        <button
          type="button"
          className="button"
          disabled={locked || !!draft || unassigned.length === 0}
          title={
            unassigned.length === 0
              ? t("Every action already has a keybinding")
              : undefined
          }
          onClick={openDraft}
        >
          <Plus size={15} aria-hidden="true" />
          {t("Add keybinding")}
        </button>
      </div>

      {draft && (
        <div
          className="keybinding-draft"
          role="group"
          aria-label={t("New keybinding")}
        >
          <strong>{t("New keybinding")}</strong>
          <Select
            className="keybinding-draft-command"
            aria-label={t("Action")}
            autoFocus
            value={draft.action}
            options={[
              { value: "", label: t("Choose an action"), disabled: true },
              ...unassigned.map((action) => ({
                value: action.id,
                label: `${t(action.group)}: ${t(action.label)}`,
              })),
            ]}
            onChange={(value) => {
              setDraft({ ...draft, action: value as ActionId });
              setRecording("new");
            }}
          />
          <button
            ref={recording === "new" ? recordingButton : undefined}
            type="button"
            className={`shortcut-recorder keybinding-draft-keys${recording === "new" ? " recording" : ""}`}
            aria-label={t("Shortcut")}
            disabled={locked}
            onClick={() => {
              setRecording("new");
              setError("");
            }}
            onBlur={() => {
              if (recording === "new") setRecording(null);
            }}
            onKeyDown={(event) => {
              if (recording !== "new") return;
              recordKey(
                event,
                (shortcut) => {
                  setDraft({ ...draft, shortcut });
                  setRecording(null);
                  setError("");
                },
                () => setRecording(null),
                setError,
              );
            }}
          >
            {recording === "new" ? (
              <span className="shortcut-unassigned">{t("Press keys…")}</span>
            ) : draft.shortcut ? (
              <Keycaps binding={draft.shortcut} />
            ) : (
              <span className="shortcut-unassigned">
                {t("Record shortcut")}
              </span>
            )}
          </button>
          <button
            type="button"
            className="button button-primary"
            disabled={locked || !draft.action || !draft.shortcut}
            onClick={() => void saveDraft()}
          >
            {t("Save")}
          </button>
          <IconButton
            title={t("Cancel")}
            onClick={() => {
              setDraft(null);
              setRecording(null);
              setError("");
            }}
          >
            <X size={14} />
          </IconButton>
        </div>
      )}

      {assigned.length === 0 && preferences.ready && (
        <div className="settings-shortcut-empty">
          <p role="status">
            {terms.length
              ? t("No matching keybindings.")
              : t("No keybindings yet.")}
          </p>
          {terms.length > 0 && (
            <button
              className="text-button"
              type="button"
              onClick={() => {
                setQuery("");
                search.current?.focus();
              }}
            >
              {t("Clear search")}
            </button>
          )}
        </div>
      )}

      {groups.map((group) => (
        <SettingsSection key={group} title={t(group)}>
          {assigned
            .filter((action) => action.group === group)
            .map((action) => {
              const binding = preferences.bindings[action.id];
              const isRecording = recording === action.id;
              return (
                <SettingRow
                  key={action.id}
                  label={t(action.label)}
                  description={t(action.description)}
                  className="keybinding-row"
                >
                  <div className="keybinding-controls">
                    <IconButton
                      className="icon-button keybinding-row-action"
                      title={t("Restore default for {label}", {
                        label: t(action.label),
                      })}
                      disabled={
                        locked || binding === preferences.defaults[action.id]
                      }
                      onClick={() =>
                        assign(action.id, preferences.defaults[action.id])
                      }
                    >
                      <RotateCcw size={14} />
                    </IconButton>
                    <IconButton
                      className="icon-button keybinding-row-action"
                      title={t("Remove keybinding for {label}", {
                        label: t(action.label),
                      })}
                      disabled={locked}
                      onClick={() => assign(action.id, null)}
                    >
                      <X size={14} />
                    </IconButton>
                    <button
                      ref={isRecording ? recordingButton : undefined}
                      type="button"
                      className={`shortcut-recorder${isRecording ? " recording" : ""}`}
                      aria-label={t(
                        "Shortcut for {label}: {shortcut}. Activate to change.",
                        {
                          label: t(action.label),
                          shortcut: formatShortcut(binding),
                        },
                      )}
                      title={t("Click to change")}
                      disabled={locked}
                      onClick={() => {
                        setRecording(action.id);
                        setError("");
                      }}
                      onBlur={() => {
                        if (isRecording) setRecording(null);
                      }}
                      onKeyDown={(event) => {
                        if (!isRecording) return;
                        recordKey(
                          event,
                          (shortcut) => assign(action.id, shortcut),
                          () => {
                            setRecording(null);
                            setError("");
                          },
                          setError,
                        );
                      }}
                    >
                      {isRecording ? (
                        <span className="shortcut-unassigned">
                          {t("Press keys…")}
                        </span>
                      ) : (
                        <Keycaps binding={binding} />
                      )}
                    </button>
                  </div>
                </SettingRow>
              );
            })}
        </SettingsSection>
      ))}
      {terms.length === 0 && (
        <SettingsSection title={t("Interaction")}>
          <SettingRow
            label={t("Focus follows pointer")}
            htmlFor="focus-follows-pointer"
            description={t(
              "Typing, paste and shortcuts go to the panel under the mouse.",
            )}
            descriptionId="pointer-focus-help"
          >
            <input
              id="focus-follows-pointer"
              className="settings-switch"
              type="checkbox"
              role="switch"
              aria-describedby="pointer-focus-help"
              checked={preferences.focusFollowsPointer}
              disabled={locked}
              onChange={(event) =>
                void persist(preferences.bindings, event.target.checked)
              }
            />
          </SettingRow>
          <SettingRow
            label={t("Open with one click")}
            htmlFor="explorer-open-on-single-click"
            description={t(
              "On: one click opens after a brief pause; a double click selects without opening. Off: one click selects and a double click opens.",
            )}
            descriptionId="explorer-click-mode-help"
          >
            <input
              id="explorer-open-on-single-click"
              className="settings-switch"
              type="checkbox"
              role="switch"
              aria-describedby="explorer-click-mode-help"
              checked={preferences.explorerOpenOnSingleClick}
              disabled={locked}
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
      )}
    </SettingsPage>
  );
}
