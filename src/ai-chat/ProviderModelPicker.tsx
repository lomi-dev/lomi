import { useEffect, useState, type CSSProperties } from "react";
import { api } from "../api";
import { useT } from "../i18n";
import type { CliAgent } from "../cli-agents";
import { Check, ChevronDown } from "../icons";
import { ProviderIcon } from "../ProviderIcon";
import { Menu } from "../ui";
import { applyModelPrefs, pickableModels } from "./providers";
import type { CliModel, ProviderEntry } from "./types";

export interface Choice {
  instanceId: string;
  model: string;
}

const cliModels = new Map<CliAgent, Promise<CliModel[]>>();
// Settled lists, read synchronously on mount to avoid an empty first render.
const loadedCliModels = new Map<CliAgent, CliModel[]>();
export const cachedCliModels = (cli: CliAgent) => loadedCliModels.get(cli);
export function loadCliModels(cli: CliAgent, refresh = false) {
  if (refresh) cliModels.delete(cli);
  let models = cliModels.get(cli);
  if (!models) {
    models = api<CliModel[]>("ai_cli_models", { cli }).then(
      (list) => {
        // An empty list is usually transient (a CLI rewriting its model cache, a slow
        // probe), so it is not kept: the next load asks the CLI again.
        if (list.length) loadedCliModels.set(cli, list);
        else cliModels.delete(cli);
        return list;
      },
      () => {
        cliModels.delete(cli);
        return [];
      },
    );
    cliModels.set(cli, models);
  }
  return models;
}

/** Model ids offered for an entry; CLI lists come from the installed CLI. */
export function useEntryModels(entry: ProviderEntry | undefined) {
  const cli = entry?.cli ?? null;
  const [fetched, setFetched] = useState<CliModel[]>(
    () => (cli && loadedCliModels.get(cli)) || [],
  );
  useEffect(() => {
    let live = true;
    setFetched((cli && loadedCliModels.get(cli)) || []);
    if (cli) void loadCliModels(cli).then((next) => live && setFetched(next));
    return () => {
      live = false;
    };
  }, [cli]);
  return cli
    ? pickableModels(
        applyModelPrefs(
          fetched.map((model) => model.id),
          entry?.modelPrefs ?? [],
        ),
      )
    : (entry?.models ?? []);
}

interface ModelOption {
  id: string;
  label: string;
}

/** Pickable models (with display names) for every entry, keyed by instance id. */
function useModelGroups(entries: readonly ProviderEntry[], open: boolean) {
  const [cli, setCli] = useState<Partial<Record<CliAgent, CliModel[]>>>(() =>
    Object.fromEntries(loadedCliModels),
  );
  const clis = [
    ...new Set(entries.flatMap((entry) => (entry.cli ? [entry.cli] : []))),
  ];
  const key = clis.join(",");
  useEffect(() => {
    let live = true;
    for (const id of key ? (key.split(",") as CliAgent[]) : [])
      void loadCliModels(id).then(
        (list) => live && setCli((current) => ({ ...current, [id]: list })),
      );
    return () => {
      live = false;
    };
    // Reopening the picker retries CLIs whose list was empty.
  }, [key, open]);
  const groups = new Map<string, ModelOption[]>();
  for (const entry of entries) {
    if (!entry.cli) {
      groups.set(
        entry.instanceId,
        entry.models.map((id) => ({ id, label: id })),
      );
      continue;
    }
    const reported = cli[entry.cli] ?? [];
    const labels = new Map(reported.map((model) => [model.id, model.label]));
    groups.set(
      entry.instanceId,
      pickableModels(
        applyModelPrefs(
          reported.map((model) => model.id),
          entry.modelPrefs,
        ),
      ).map((id) => ({ id, label: labels.get(id) ?? id })),
    );
  }
  return groups;
}

/** One searchable list of models, grouped by provider. */
export default function ProviderModelPicker({
  entries,
  value,
  onChange,
  disabled,
}: {
  entries: readonly ProviderEntry[];
  value: Choice;
  onChange: (choice: Choice) => void;
  disabled: boolean;
}) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const groups = useModelGroups(entries, open);
  const entry = entries.find((item) => item.instanceId === value.instanceId);
  const own = groups.get(value.instanceId) ?? [];
  const first = own[0]?.id;
  useEffect(() => {
    if (!value.model && first && entry)
      onChange({ instanceId: entry.instanceId, model: first });
  }, [value.model, first, entry, onChange]);
  const current = own.find((model) => model.id === value.model);
  const visible = entries
    .filter((item) => item.available)
    .map((item) => ({ item, models: groups.get(item.instanceId) ?? [] }))
    .filter((group) => group.models.length > 0);
  // Provider whose models fill the right column; follows the selection when opened.
  const [browse, setBrowse] = useState(value.instanceId);
  const shown =
    visible.find((group) => group.item.instanceId === browse) ?? visible[0];
  const close = () => setOpen(false);
  // Sized for the longest list so switching providers never resizes (and moves) the menu.
  const rows = Math.max(
    visible.length > 1 ? visible.length : 0,
    ...visible.map((group) => group.models.length),
  );
  const pick = (instanceId: string, model: string) => {
    onChange({ instanceId, model });
    close();
  };
  return (
    <div className="thread-picker">
      <button
        type="button"
        className="thread-picker-trigger"
        data-menu-trigger
        disabled={disabled}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={entry ? `${entry.label} · ${value.model}` : undefined}
        onClick={() => {
          if (open) return close();
          setBrowse(value.instanceId);
          setOpen(true);
        }}
      >
        {entry && <ProviderIcon kind={entry.kind} cli={entry.cli} />}
        <span className="thread-picker-label">
          {current?.label || value.model || t("Choose a model")}
        </span>
        <ChevronDown size={13} aria-hidden="true" />
      </button>
      {open && (
        <Menu
          className={`thread-picker-menu${visible.length > 1 ? " has-providers" : ""}`}
          onClose={close}
        >
          {!shown ? (
            <p className="thread-picker-empty">{t("No models available.")}</p>
          ) : (
            <div
              className="thread-picker-body"
              style={{ "--picker-rows": rows } as CSSProperties}
            >
              {visible.length > 1 && (
                <ul
                  className="thread-picker-providers"
                  aria-label={t("Providers")}
                >
                  {visible.map(({ item, models }) => (
                    <li key={item.instanceId}>
                      <button
                        type="button"
                        aria-pressed={item.instanceId === shown.item.instanceId}
                        onClick={() => setBrowse(item.instanceId)}
                      >
                        <ProviderIcon kind={item.kind} cli={item.cli} />
                        <span className="thread-picker-name">{item.label}</span>
                        {item.instanceId === value.instanceId && (
                          <span
                            className="thread-picker-dot"
                            aria-hidden="true"
                          />
                        )}
                        <span className="thread-picker-count">
                          {models.length}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <ul
                className="thread-picker-models"
                aria-label={shown.item.label}
              >
                {shown.models.map((model) => {
                  const active =
                    shown.item.instanceId === value.instanceId &&
                    model.id === value.model;
                  return (
                    <li key={model.id}>
                      <button
                        type="button"
                        aria-current={active || undefined}
                        title={model.id}
                        onClick={() => pick(shown.item.instanceId, model.id)}
                      >
                        <span className="thread-picker-model">
                          <span className="thread-picker-name">
                            {model.label}
                          </span>
                          {model.label !== model.id && <code>{model.id}</code>}
                        </span>
                        {active && <Check size={14} aria-hidden="true" />}
                      </button>
                    </li>
                  );
                })}
              </ul>
            </div>
          )}
        </Menu>
      )}
    </div>
  );
}
