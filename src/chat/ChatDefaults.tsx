import { useId, useState } from "react";
import Select from "../Select";
import { macOS } from "../api";
import { SettingRow, SettingsNotice, SettingsSection } from "../settings-ui";
import { DisclosureSummary } from "../ui";
import { defaultConfig } from "./types";
import { ModelSelect } from "./ModelSelect";
import { ProviderIcon } from "./ProviderIcon";
import { validModelId } from "./provider-presets";
import { suggestedModel } from "./models";
import type { Preferences } from "./types";

const validTokens = (value: string) =>
  /^\d+$/.test(value) && Number(value) >= 1 && Number(value) <= 32768;

/**
 * Saves each field independently from the latest saved preferences. Text
 * drafts are kept until their own save succeeds, so a failed or concurrent
 * save never discards typed input.
 */
export function ChatDefaults({
  saved,
  busy,
  onSave,
}: {
  saved: Preferences;
  busy: boolean;
  onSave: (data: Preferences) => Promise<boolean>;
}) {
  const connectionId = useId();
  const modelId = useId();
  const sendModeId = useId();
  const instructionsId = useId();
  const tokensId = useId();
  const [model, setModel] = useState<string>();
  const [system, setSystem] = useState<string>();
  const [tokens, setTokens] = useState<string>();
  const defaults = saved.defaults;
  const connection = saved.connections.find(
    (item) => item.id === defaults.connectionId,
  );
  const usable =
    !!connection?.enabled &&
    (!!connection.secretId || connection.provider === "custom");
  const defaultHelp = connection
    ? !connection.enabled
      ? `${connection.name} is turned off. Open Manage in AI services to enable it.`
      : !usable
        ? `${connection.name} needs an API key. Add one in AI services before choosing a model.`
        : ""
    : defaults.connectionId
      ? "The saved default service is unavailable. Choose another service."
      : "";
  const save = (
    changes: Partial<Preferences["defaults"]>,
    sendMode = saved.sendMode,
  ) => {
    const next = { ...defaults, ...changes };
    return onSave({
      ...saved,
      sendMode,
      defaults: {
        ...next,
        configured: !!next.connectionId && !!next.model,
      },
    });
  };
  const commit = <T,>(
    value: T,
    current: T,
    clear: (update: (draft: T | undefined) => T | undefined) => void,
    changes: Partial<Preferences["defaults"]>,
  ) => {
    if (value === current) {
      clear(() => undefined);
      return;
    }
    void save(changes).then((ok) => {
      if (ok) clear((draft) => (draft === value ? undefined : draft));
    });
  };
  const modelValue = model ?? defaults.model;
  const systemValue = system ?? defaults.system;
  const tokensValue = tokens ?? String(defaults.maxOutputTokens);
  const advancedSummary = [
    defaults.maxOutputTokens !== defaultConfig.maxOutputTokens
      ? `${defaults.maxOutputTokens.toLocaleString()} output tokens`
      : null,
    defaults.system ? "Custom system instructions" : null,
    defaults.temperature !== defaultConfig.temperature
      ? `Temperature ${defaults.temperature}`
      : null,
  ].filter(Boolean);
  return (
    <>
      <SettingsSection
        title="Default for new chats"
        description="Choose the service and model used when you create a conversation. Existing chats keep their own settings."
        className="chat-defaults"
      >
        {defaultHelp && (
          <SettingsNotice tone="warning" role="note">
            {defaultHelp}
          </SettingsNotice>
        )}
        <SettingRow label="AI service" htmlFor={connectionId}>
          <div className="chat-defaults-connection">
            {connection && <ProviderIcon provider={connection.provider} />}
            <Select
              id={connectionId}
              value={defaults.connectionId ?? ""}
              disabled={busy}
              onChange={(value) => {
                setModel(undefined);
                void save({
                  connectionId: value || null,
                  model: suggestedModel(
                    saved.connections.find((c) => c.id === value),
                  ),
                  temperature: null,
                });
              }}
              options={[
                { value: "", label: "Choose a service" },
                ...saved.connections.map((c) => ({
                  value: c.id,
                  label: !c.enabled
                    ? `${c.name} (turned off)`
                    : !c.secretId && c.provider !== "custom"
                      ? `${c.name} (add API key first)`
                      : c.name,
                  disabled:
                    !c.enabled || (!c.secretId && c.provider !== "custom"),
                })),
              ]}
            />
          </div>
        </SettingRow>
        <SettingRow label="Default model" htmlFor={modelId}>
          <ModelSelect
            key={defaults.connectionId}
            id={modelId}
            hideLabel
            connection={connection}
            disabled={busy || !usable}
            value={modelValue}
            onChange={setModel}
            onCommit={(value) => {
              if (validModelId(value))
                commit(value, defaults.model, setModel, {
                  model: value,
                  temperature: null,
                });
            }}
          />
        </SettingRow>
      </SettingsSection>
      <SettingsSection
        title="Chat behavior"
        className="chat-defaults chat-behavior"
      >
        <SettingRow
          label="Send message with"
          htmlFor={sendModeId}
          description={`${saved.sendMode === "enter" ? "Shift+Enter" : "Enter"} adds a new line.`}
          descriptionId={`${sendModeId}-help`}
        >
          <Select
            id={sendModeId}
            aria-describedby={`${sendModeId}-help`}
            disabled={busy}
            value={saved.sendMode}
            onChange={(value) =>
              void save({}, value as Preferences["sendMode"])
            }
            options={[
              { value: "enter", label: "Enter" },
              {
                value: "modifier-enter",
                label: macOS ? "⌘ Enter" : "Ctrl+Enter",
              },
            ]}
          />
        </SettingRow>
        {advancedSummary.length > 0 && (
          <p className="chat-defaults-summary">
            Advanced defaults: {advancedSummary.join(" · ")}
          </p>
        )}
        <details className="settings-disclosure chat-defaults-advanced">
          <DisclosureSummary>Advanced defaults</DisclosureSummary>
          <div className="settings-disclosure-body">
            <SettingRow
              label="Maximum output tokens"
              htmlFor={tokensId}
              description={
                validTokens(tokensValue) ? (
                  "Limits the length of each reply."
                ) : (
                  <span className="text-error">Enter 1 to 32,768.</span>
                )
              }
              descriptionId={`${tokensId}-help`}
            >
              <input
                id={tokensId}
                aria-describedby={`${tokensId}-help`}
                aria-invalid={!validTokens(tokensValue)}
                type="number"
                inputMode="numeric"
                min={1}
                max={32768}
                value={tokensValue}
                onChange={(e) => setTokens(e.target.value)}
                onBlur={() => {
                  if (tokens !== undefined && validTokens(tokens))
                    commit(
                      tokens,
                      String(defaults.maxOutputTokens),
                      setTokens,
                      {
                        maxOutputTokens: Number(tokens),
                      },
                    );
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") e.currentTarget.blur();
                }}
              />
            </SettingRow>
            <SettingRow
              label="System instructions"
              htmlFor={instructionsId}
              description="Preferred language, tone or style for replies."
              descriptionId={`${instructionsId}-help`}
              stacked
            >
              <textarea
                id={instructionsId}
                aria-describedby={`${instructionsId}-help`}
                rows={3}
                placeholder="e.g. Answer in Polish and keep replies concise."
                value={systemValue}
                onChange={(e) => setSystem(e.target.value)}
                onBlur={() => {
                  if (system !== undefined)
                    commit(system, defaults.system, setSystem, { system });
                }}
              />
            </SettingRow>
          </div>
        </details>
      </SettingsSection>
    </>
  );
}
