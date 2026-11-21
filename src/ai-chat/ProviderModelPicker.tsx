import { useEffect, useState } from "react";
import { api } from "../api";
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
export function loadCliModels(cli: CliAgent, refresh = false) {
  if (refresh) cliModels.delete(cli);
  let models = cliModels.get(cli);
  if (!models) {
    models = api<CliModel[]>("ai_cli_models", { cli }).catch(() => {
      cliModels.delete(cli);
      return [];
    });
    cliModels.set(cli, models);
  }
  return models;
}

/** Model ids offered for an entry; CLI lists come from the installed CLI. */
export function useEntryModels(entry: ProviderEntry | undefined) {
  const [fetched, setFetched] = useState<CliModel[]>([]);
  const cli = entry?.cli ?? null;
  useEffect(() => {
    let live = true;
    setFetched([]);
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
function useModelGroups(entries: readonly ProviderEntry[]) {
  const [cli, setCli] = useState<Partial<Record<CliAgent, CliModel[]>>>({});
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
  }, [key]);
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
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const groups = useModelGroups(entries);
  const entry = entries.find((item) => item.instanceId === value.instanceId);
  const own = groups.get(value.instanceId) ?? [];
  const first = own[0]?.id;
  useEffect(() => {
    if (!value.model && first && entry)
      onChange({ instanceId: entry.instanceId, model: first });
  }, [value.model, first, entry, onChange]);
  const current = own.find((model) => model.id === value.model);
  const needle = query.trim().toLowerCase();
  const visible = entries
    .filter((item) => item.available)
    .map((item) => ({
      item,
      models: (groups.get(item.instanceId) ?? []).filter(
        (model) =>
          !needle ||
          model.label.toLowerCase().includes(needle) ||
          model.id.toLowerCase().includes(needle) ||
          item.label.toLowerCase().includes(needle),
      ),
    }))
    .filter((group) => group.models.length > 0);
  const close = () => {
    setOpen(false);
    setQuery("");
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
        onClick={() => (open ? close() : setOpen(true))}
      >
        {entry && <ProviderIcon kind={entry.kind} cli={entry.cli} />}
        <span className="thread-picker-label">
          {current?.label || value.model || "Choose a model"}
        </span>
        <ChevronDown size={13} aria-hidden="true" />
      </button>
      {open && (
        <Menu className="thread-picker-menu" onClose={close}>
          <input
            className="thread-picker-search"
            type="search"
            autoFocus
            aria-label="Search models"
            placeholder="Search models"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              const hit = visible[0];
              if (!hit) return;
              event.preventDefault();
              onChange({
                instanceId: hit.item.instanceId,
                model: hit.models[0].id,
              });
              close();
            }}
          />
          <div className="thread-picker-groups">
            {visible.length === 0 && (
              <p className="thread-picker-empty">
                {needle ? "No models match." : "No models available."}
              </p>
            )}
            {visible.map(({ item, models }) => (
              <section key={item.instanceId} aria-label={item.label}>
                <h3>
                  <ProviderIcon kind={item.kind} cli={item.cli} />
                  {item.label}
                </h3>
                <ul>
                  {models.map((model) => {
                    const active =
                      item.instanceId === value.instanceId &&
                      model.id === value.model;
                    return (
                      <li key={model.id}>
                        <button
                          type="button"
                          aria-current={active || undefined}
                          onClick={() => {
                            onChange({
                              instanceId: item.instanceId,
                              model: model.id,
                            });
                            close();
                          }}
                        >
                          <span className="thread-picker-name">
                            {model.label}
                          </span>
                          {model.label !== model.id && <code>{model.id}</code>}
                          {active && <Check size={14} aria-hidden="true" />}
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </section>
            ))}
          </div>
        </Menu>
      )}
    </div>
  );
}
