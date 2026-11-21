import { useId, useState } from "react";
import { useT } from "../../i18n";
import { ArrowDown, ArrowUp, Plus, RefreshCw, Star, X } from "../../icons";
import { SettingsSection } from "../../settings-ui";
import type { ModelEntry } from "../types";

/** Model list for one provider: filter, enable, favorite, reorder and custom ids. */
export default function ModelsSection({
  models,
  labels,
  onChange,
  onRefresh,
  refreshing,
  disabled,
}: {
  models: ModelEntry[];
  /** Display names reported by the provider, keyed by model id. */
  labels?: ReadonlyMap<string, string>;
  onChange: (models: ModelEntry[]) => void;
  onRefresh?: () => void;
  refreshing?: boolean;
  disabled?: boolean;
}) {
  const { t } = useT();
  const [filter, setFilter] = useState("");
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const draftId = useId();
  const query = filter.trim().toLowerCase();
  const visible = query
    ? models.filter(
        (model) =>
          model.id.toLowerCase().includes(query) ||
          labels?.get(model.id)?.toLowerCase().includes(query),
      )
    : models;
  const enabledCount = models.filter((model) => model.enabled).length;
  const allOn = enabledCount === models.length;
  const patch = (id: string, next: Partial<ModelEntry>) =>
    onChange(
      models.map((model) => (model.id === id ? { ...model, ...next } : model)),
    );
  const move = (index: number, by: number) => {
    const next = [...models];
    [next[index], next[index + by]] = [next[index + by], next[index]];
    onChange(next);
  };
  const add = () => {
    const id = draft.trim();
    if (!id) return;
    if (!models.some((model) => model.id === id))
      onChange([
        ...models,
        { id, source: "manual", enabled: true, favorite: false },
      ]);
    setDraft("");
    setAdding(false);
  };
  return (
    <SettingsSection
      title={t("Models")}
      className="prov-models-section"
      actions={
        onRefresh && (
          <button
            type="button"
            className="icon-button"
            title={t("Refresh models")}
            aria-label={t("Refresh models")}
            disabled={disabled || refreshing}
            data-spinning={refreshing || undefined}
            onClick={onRefresh}
          >
            <RefreshCw size={14} aria-hidden="true" />
          </button>
        )
      }
    >
      <div className="prov-models-toolbar">
        <input
          type="search"
          aria-label={t("Filter models")}
          placeholder={t("Filter models")}
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        />
        <button
          type="button"
          className="prov-text-button"
          disabled={disabled || models.length === 0}
          onClick={() =>
            onChange(models.map((model) => ({ ...model, enabled: !allOn })))
          }
        >
          {allOn ? t("Disable all") : t("Enable all")}
        </button>
        <span className="prov-models-count">
          {t("{enabled}/{total} enabled", {
            enabled: enabledCount,
            total: models.length,
          })}
        </span>
        <button
          type="button"
          className="prov-text-button"
          disabled={disabled}
          onClick={() => setAdding(true)}
        >
          <Plus size={14} aria-hidden="true" />
          {t("Custom model")}
        </button>
      </div>
      {adding && (
        <form
          className="prov-models-add"
          onSubmit={(event) => {
            event.preventDefault();
            add();
          }}
        >
          <input
            id={draftId}
            autoFocus
            className="prov-mono"
            aria-label={t("Model id")}
            placeholder="model-id"
            maxLength={256}
            spellCheck={false}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.stopPropagation();
                setAdding(false);
              }
            }}
          />
          <button type="submit" className="button" disabled={!draft.trim()}>
            {t("Add")}
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label={t("Cancel")}
            onClick={() => {
              setAdding(false);
              setDraft("");
            }}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </form>
      )}
      {visible.length === 0 ? (
        <p className="prov-models-empty">
          {models.length === 0
            ? t("No models yet. Refresh or add a custom model.")
            : t("No models match the filter.")}
        </p>
      ) : (
        <ul className="prov-models-list" aria-label={t("Models")}>
          {visible.map((model) => {
            const index = models.indexOf(model);
            const label = labels?.get(model.id);
            return (
              <li key={model.id} data-off={!model.enabled || undefined}>
                <button
                  type="button"
                  className="prov-star"
                  aria-pressed={model.favorite}
                  title={model.favorite ? t("Unfavorite") : t("Favorite")}
                  aria-label={t("Favorite {model}", { model: model.id })}
                  disabled={disabled}
                  onClick={() => patch(model.id, { favorite: !model.favorite })}
                >
                  <Star size={14} aria-hidden="true" />
                </button>
                <span className="prov-model-name">
                  {label && label !== model.id && <span>{label}</span>}
                  <code>{model.id}</code>
                  {model.source === "manual" && (
                    <span className="prov-model-tag">{t("custom")}</span>
                  )}
                </span>
                <span className="prov-model-actions">
                  {model.source === "manual" && (
                    <button
                      type="button"
                      className="icon-button"
                      title={t("Remove")}
                      aria-label={t("Remove {model}", { model: model.id })}
                      disabled={disabled}
                      onClick={() =>
                        onChange(models.filter((item) => item !== model))
                      }
                    >
                      <X size={14} aria-hidden="true" />
                    </button>
                  )}
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={t("Move {model} up", { model: model.id })}
                    disabled={disabled || !!query || index === 0}
                    onClick={() => move(index, -1)}
                  >
                    <ArrowUp size={14} aria-hidden="true" />
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={t("Move {model} down", { model: model.id })}
                    disabled={
                      disabled || !!query || index === models.length - 1
                    }
                    onClick={() => move(index, 1)}
                  >
                    <ArrowDown size={14} aria-hidden="true" />
                  </button>
                </span>
                <input
                  type="checkbox"
                  role="switch"
                  className="settings-switch"
                  aria-label={t("Enable {model}", { model: model.id })}
                  checked={model.enabled}
                  disabled={disabled}
                  onChange={(event) =>
                    patch(model.id, { enabled: event.target.checked })
                  }
                />
              </li>
            );
          })}
        </ul>
      )}
    </SettingsSection>
  );
}
