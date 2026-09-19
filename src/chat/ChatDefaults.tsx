import { DisclosureSummary } from "../ui";
import { useId, useState } from "react";
import Select from "../Select";
import { macOS } from "../api";
import { ModelSelect } from "./ModelSelect";
import { ProviderIcon } from "./ProviderIcon";
import { suggestedModel, modelLabel } from "./models";
import type { Preferences } from "./types";

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
  const sendModeId = useId();
  const instructionsId = useId();
  const tokensId = useId();
  const [draft, setDraft] = useState<Preferences>();
  const data = draft ?? saved;
  const connection = data.connections.find(
    (item) => item.id === data.defaults.connectionId,
  );
  return (
    <details className="chat-defaults-section">
      <DisclosureSummary>
        Chat preferences
        <span className="chat-defaults-model">
          {modelLabel(data.defaults.model) || "Choose a default model"}
        </span>
      </DisclosureSummary>
      <form
        className="chat-defaults"
        onSubmit={(e) => {
          e.preventDefault();
          void onSave({
            ...data,
            defaults: {
              ...data.defaults,
              configured: !!data.defaults.connectionId && !!data.defaults.model,
            },
          }).then((saved) => {
            if (saved) setDraft(undefined);
          });
        }}
      >
        <p className="chat-defaults-intro">
          Model and instructions apply to new conversations.
        </p>
        <div className="chat-defaults-models">
          <div className="chat-select-field">
            <label htmlFor={connectionId}>Connection</label>
            <div className="chat-defaults-connection">
              {connection && <ProviderIcon provider={connection.provider} />}
              <Select
                id={connectionId}
                value={data.defaults.connectionId ?? ""}
                disabled={busy}
                onChange={(value) =>
                  setDraft({
                    ...data,
                    defaults: {
                      ...data.defaults,
                      connectionId: value || null,
                      model: suggestedModel(
                        data.connections.find((c) => c.id === value),
                      ),
                      temperature: null,
                    },
                  })
                }
                options={[
                  { value: "", label: "Choose connection" },
                  ...data.connections
                    .filter((c) => c.enabled)
                    .map((c) => ({ value: c.id, label: c.name })),
                ]}
              />
            </div>
          </div>
          <ModelSelect
            key={data.defaults.connectionId}
            connection={connection}
            disabled={busy || !connection}
            value={data.defaults.model}
            onChange={(model) =>
              setDraft({
                ...data,
                defaults: { ...data.defaults, model, temperature: null },
              })
            }
          />
        </div>
        <div className="chat-defaults-instructions">
          <label htmlFor={instructionsId}>System instructions</label>
          <p className="chat-defaults-help" id={`${instructionsId}-help`}>
            Set a preferred language, tone, or style of response.
          </p>
          <textarea
            id={instructionsId}
            aria-describedby={`${instructionsId}-help`}
            rows={3}
            disabled={busy}
            placeholder="e.g. Answer in Polish and keep replies concise."
            value={data.defaults.system}
            onChange={(e) =>
              setDraft({
                ...data,
                defaults: { ...data.defaults, system: e.target.value },
              })
            }
          />
        </div>
        <div className="chat-defaults-row chat-defaults-send">
          <div>
            <label htmlFor={sendModeId}>Send message with</label>
            <p className="chat-defaults-help" id={`${sendModeId}-help`}>
              {data.sendMode === "enter" ? "Shift+Enter" : "Enter"} adds a new
              line.
            </p>
          </div>
          <Select
            id={sendModeId}
            aria-describedby={`${sendModeId}-help`}
            disabled={busy}
            value={data.sendMode}
            onChange={(value) =>
              setDraft({
                ...data,
                sendMode: value as Preferences["sendMode"],
              })
            }
            options={[
              { value: "enter", label: "Enter" },
              {
                value: "modifier-enter",
                label: macOS ? "⌘ Enter" : "Ctrl+Enter",
              },
            ]}
          />
        </div>
        <details className="chat-defaults-advanced">
          <DisclosureSummary>Advanced</DisclosureSummary>
          <div className="chat-defaults-row">
            <div>
              <label htmlFor={tokensId}>Maximum output tokens</label>
              <p className="chat-defaults-help" id={`${tokensId}-help`}>
                Limit the length of each response. Longer replies use more
                tokens.
              </p>
            </div>
            <div className="chat-defaults-tokens">
              <input
                id={tokensId}
                aria-describedby={`${tokensId}-help`}
                type="number"
                min={1}
                max={32768}
                disabled={busy}
                value={data.defaults.maxOutputTokens}
                onChange={(e) =>
                  setDraft({
                    ...data,
                    defaults: {
                      ...data.defaults,
                      maxOutputTokens: Number(e.target.value),
                    },
                  })
                }
              />
              <span aria-hidden="true">tokens</span>
            </div>
          </div>
        </details>
        <div className="chat-defaults-footer">
          <span className="chat-defaults-help" aria-live="polite">
            {draft ? "Unsaved changes" : ""}
          </span>
          <div className="dialog-actions">
            {draft && (
              <button
                className="button"
                type="button"
                disabled={busy}
                onClick={() => setDraft(undefined)}
              >
                Discard changes
              </button>
            )}
            <button
              className="button chat-primary-button"
              type="submit"
              disabled={busy || !draft}
            >
              Save defaults
            </button>
          </div>
        </div>
      </form>
    </details>
  );
}
