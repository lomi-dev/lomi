import { useEffect, useId, useRef, useState } from "react";
import type { ComponentType } from "react";
import "../thread.css";
import "./providers-settings.css";
import { macOS } from "../../api";
import { msg, t, useT } from "../../i18n";
import { Eye, Folder, Gauge, Lock, Pencil, ShieldAlert, X } from "../../icons";
import Select from "../../Select";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "../../settings-ui";
import { Modal } from "../../ui";
import ProviderModelPicker, {
  loadCliModels,
  type Choice,
} from "../ProviderModelPicker";
import {
  applyModelPrefs,
  pickableModels,
  providerDefault,
  useChatProviders,
} from "../providers";
import type {
  ChatBehavior,
  ChatDefaults,
  Permissions,
  ProviderEntry,
  Workspace,
} from "../types";

const MAX_INSTRUCTIONS = 32 * 1024;
type Icon = ComponentType<{
  size?: number;
  className?: string;
  "aria-hidden"?: boolean;
}>;

const PERMISSIONS: { value: Permissions; label: string; icon: Icon }[] = [
  { value: "read-only", label: msg("Read only"), icon: Eye },
  { value: "edit", label: msg("Edit files"), icon: Pencil },
  { value: "full", label: msg("Full access"), icon: ShieldAlert },
];
const WORKSPACES: { value: Workspace; label: string; icon: Icon }[] = [
  { value: "project", label: msg("Project folder"), icon: Folder },
  { value: "private", label: msg("Private folder"), icon: Lock },
];
const EFFORT_LABELS: Record<string, string> = {
  minimal: msg("Minimal"),
  low: msg("Low"),
  medium: msg("Medium"),
  high: msg("High"),
  max: msg("Max"),
};
const effortLabel = (id: string) =>
  Object.hasOwn(EFFORT_LABELS, id)
    ? t(EFFORT_LABELS[id])
    : id.charAt(0).toUpperCase() + id.slice(1);

/** Reasoning efforts the CLI reports for a model; empty for API providers. */
function useEfforts(entry: ProviderEntry | undefined, model: string) {
  const [efforts, setEfforts] = useState<string[]>([]);
  const cli = entry?.cli ?? null;
  useEffect(() => {
    let live = true;
    setEfforts([]);
    if (cli && model)
      void loadCliModels(cli).then(
        (models) =>
          live &&
          setEfforts(models.find((item) => item.id === model)?.efforts ?? []),
      );
    return () => {
      live = false;
    };
  }, [cli, model]);
  return efforts;
}

/** A quiet select with the current option's icon in front. */
function IconSelect<T extends string>({
  id,
  value,
  options,
  disabled,
  onChange,
}: {
  id?: string;
  value: T;
  options: { value: T; label: string; icon: Icon }[];
  disabled: boolean;
  onChange: (value: T) => void;
}) {
  const { t } = useT();
  const Current = options.find((option) => option.value === value)?.icon;
  return (
    <div className="defaults-select">
      {Current && (
        <Current className="defaults-select-icon" size={15} aria-hidden />
      )}
      <Select
        id={id}
        value={value}
        disabled={disabled}
        options={options.map((option) => ({
          ...option,
          label: t(option.label),
        }))}
        onChange={(next) => onChange(next as T)}
      />
    </div>
  );
}

export default function DefaultsSettingsPage() {
  const { t } = useT();
  const { entries, settings, error, busy, run } = useChatProviders();
  const [confirmFull, setConfirmFull] = useState(false);
  const [instructions, setInstructions] = useState("");
  const [tokens, setTokens] = useState("");
  const ids = {
    effort: useId(),
    permissions: useId(),
    workspace: useId(),
    key: useId(),
    instructions: useId(),
    tokens: useId(),
  };
  const usable = entries.filter((entry) => entry.available);
  const defaults = settings?.defaults;
  const behavior = settings?.behavior;
  const entry = providerDefault(entries, settings);
  const model =
    defaults?.instanceId === entry?.instanceId ? (defaults?.model ?? "") : "";
  const efforts = useEfforts(entry, model);
  const fallbackEntry = usable.find(
    (item) => item.instanceId === defaults?.fallback?.instanceId,
  );
  useEffect(() => {
    if (!behavior) return;
    setInstructions(behavior.systemInstructions);
    setTokens(behavior.maxOutputTokens?.toString() ?? "");
  }, [behavior]);
  const parsed = tokens.trim() === "" ? null : Number(tokens);
  const tokensOk =
    parsed === null ||
    (Number.isInteger(parsed) && parsed >= 1 && parsed <= 1_000_000);
  // Every save sends both halves so neither drops the other's fields.
  const save = (
    patch: Partial<ChatDefaults>,
    behaviorPatch: Partial<ChatBehavior> = {},
  ) => {
    if (!settings) return Promise.resolve();
    return run("ai_chat_preferences_update", {
      expectedRevision: settings.revision,
      defaults: { ...settings.defaults, ...patch },
      behavior: { ...settings.behavior, ...behaviorPatch },
    }).then(
      () => {},
      () => {},
    );
  };
  // The picker auto-fills an empty model; only persist choices the user made.
  const touched = useRef(false);
  const pickDefault = (choice: Choice) => {
    if (!touched.current) return;
    if (choice.instanceId === defaults?.instanceId && choice.model === model)
      return;
    void save({
      instanceId: choice.instanceId,
      model: choice.model,
      effort: null,
    });
  };
  const mod = macOS ? "⌘Enter" : "Ctrl+Enter";
  const off = !settings || busy;
  return (
    <SettingsPage
      title={t("Defaults")}
      busy={busy}
      status={!settings && !error ? t("Loading…") : undefined}
    >
      {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
      <SettingsSection title={t("New threads")} className="defaults-section">
        <SettingRow
          label={t("Model")}
          description={t("Default model for new threads.")}
        >
          <div className="defaults-controls">
            <div
              onPointerDownCapture={() => (touched.current = true)}
              onKeyDownCapture={() => (touched.current = true)}
            >
              <ProviderModelPicker
                entries={usable}
                value={{ instanceId: entry?.instanceId ?? "", model }}
                onChange={pickDefault}
                disabled={off}
              />
            </div>
            {efforts.length > 0 && (
              <div className="defaults-select">
                <Gauge className="defaults-select-icon" size={15} aria-hidden />
                <Select
                  id={ids.effort}
                  aria-label={t("Reasoning effort")}
                  value={
                    efforts.includes(defaults?.effort ?? "")
                      ? (defaults?.effort ?? "")
                      : ""
                  }
                  disabled={off}
                  options={[
                    { value: "", label: t("Default") },
                    ...efforts.map((id) => ({
                      value: id,
                      label: effortLabel(id),
                    })),
                  ]}
                  onChange={(value) => void save({ effort: value || null })}
                />
              </div>
            )}
          </div>
        </SettingRow>
        <SettingRow
          label={t("Fallback model")}
          description={t(
            "Used when the default model's provider is unavailable.",
          )}
        >
          <div className="defaults-controls">
            {fallbackEntry ? (
              <>
                <ProviderModelPicker
                  entries={usable}
                  value={{
                    instanceId: fallbackEntry.instanceId,
                    model: defaults?.fallback?.model ?? "",
                  }}
                  onChange={(choice) => void save({ fallback: choice })}
                  disabled={off}
                />
                <button
                  type="button"
                  className="icon-button"
                  title={t("Remove fallback")}
                  aria-label={t("Remove fallback")}
                  disabled={off}
                  onClick={() => void save({ fallback: null })}
                >
                  <X size={14} aria-hidden="true" />
                </button>
              </>
            ) : (
              <button
                type="button"
                className="button"
                disabled={off || usable.length === 0}
                onClick={() => {
                  const first =
                    usable.find(
                      (item) => item.instanceId !== entry?.instanceId,
                    ) ?? usable[0];
                  if (!first) return;
                  // The backend requires a concrete model; start from the provider's first one.
                  void (
                    first.cli
                      ? loadCliModels(first.cli).then((list) =>
                          pickableModels(
                            applyModelPrefs(
                              list.map((m) => m.id),
                              first.modelPrefs,
                            ),
                          ),
                        )
                      : Promise.resolve(first.models)
                  ).then(([model]) => {
                    if (model)
                      void save({
                        fallback: { instanceId: first.instanceId, model },
                      });
                  });
                }}
              >
                {t("None")}
              </button>
            )}
          </div>
        </SettingRow>
        <SettingRow
          label={t("Permissions")}
          htmlFor={ids.permissions}
          description={t("What CLI agents may do in new threads.")}
        >
          <IconSelect
            id={ids.permissions}
            value={defaults?.permissions ?? "read-only"}
            options={PERMISSIONS}
            disabled={off}
            onChange={(value) =>
              value === "full"
                ? setConfirmFull(true)
                : void save({ permissions: value })
            }
          />
        </SettingRow>
        <SettingRow
          label={t("Workspace")}
          htmlFor={ids.workspace}
          description={t("Where new threads start.")}
        >
          <IconSelect
            id={ids.workspace}
            value={defaults?.workspace ?? "project"}
            options={WORKSPACES}
            disabled={off}
            onChange={(value) => void save({ workspace: value })}
          />
        </SettingRow>
      </SettingsSection>
      <SettingsSection title={t("Composer")}>
        <SettingRow
          label={t("Send message with")}
          htmlFor={ids.key}
          description={t("The other key inserts a new line.")}
        >
          <Select
            id={ids.key}
            value={behavior?.sendKey ?? "enter"}
            disabled={off}
            options={[
              { value: "enter", label: t("Enter") },
              { value: "mod-enter", label: mod },
            ]}
            onChange={(value) =>
              void save({}, { sendKey: value as ChatBehavior["sendKey"] })
            }
          />
        </SettingRow>
      </SettingsSection>
      <SettingsSection title={t("Responses")}>
        <SettingRow
          label={t("System instructions")}
          htmlFor={ids.instructions}
          description={t("Sent to API providers and Claude Code.")}
          stacked
        >
          <textarea
            id={ids.instructions}
            rows={6}
            value={instructions}
            disabled={!settings}
            maxLength={MAX_INSTRUCTIONS}
            onChange={(event) => setInstructions(event.target.value)}
            onBlur={() => {
              if (instructions !== behavior?.systemInstructions)
                void save({}, { systemInstructions: instructions });
            }}
          />
        </SettingRow>
        <SettingRow
          label={t("Max output tokens")}
          htmlFor={ids.tokens}
          description={
            tokensOk
              ? t("Leave empty for the provider default.")
              : t("Enter a whole number from 1 to 1,000,000.")
          }
        >
          <input
            id={ids.tokens}
            type="number"
            min={1}
            max={1000000}
            placeholder={t("Default")}
            value={tokens}
            disabled={!settings}
            aria-invalid={!tokensOk}
            onChange={(event) => setTokens(event.target.value)}
            onBlur={() => {
              if (tokensOk && parsed !== behavior?.maxOutputTokens)
                void save({}, { maxOutputTokens: parsed });
            }}
          />
        </SettingRow>
      </SettingsSection>
      {confirmFull && (
        <Modal
          title={t("Allow full access?")}
          tone="danger"
          closeDisabled={busy}
          onClose={() => setConfirmFull(false)}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p>
                {t(
                  "New CLI threads will be able to run any command and edit files without asking you first. Use this only for projects you trust.",
                )}
              </p>
            </div>
            <div className="dialog-actions">
              <button
                type="button"
                className="button"
                onClick={() => setConfirmFull(false)}
              >
                {t("Cancel")}
              </button>
              <button
                type="button"
                className="button button-danger"
                disabled={busy}
                onClick={() =>
                  void save({ permissions: "full" }).then(() =>
                    setConfirmFull(false),
                  )
                }
              >
                {t("Allow full access")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </SettingsPage>
  );
}
