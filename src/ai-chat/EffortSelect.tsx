import { useEffect, useState } from "react";
import type { CliAgent } from "../cli-agents";
import { msg, t, useT } from "../i18n";
import { Gauge } from "../icons";
import Select from "../Select";
import { loadCliModels } from "./ProviderModelPicker";
import type { ProviderEntry } from "./types";

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
export const loadEfforts = (cli: CliAgent, model: string) =>
  loadCliModels(cli).then(
    (models) => models.find((item) => item.id === model)?.efforts ?? [],
  );

export function useEfforts(entry: ProviderEntry | undefined, model: string) {
  const [efforts, setEfforts] = useState<string[]>([]);
  const cli = entry?.cli ?? null;
  useEffect(() => {
    let live = true;
    setEfforts([]);
    if (cli && model)
      void loadEfforts(cli, model).then((next) => live && setEfforts(next));
    return () => {
      live = false;
    };
  }, [cli, model]);
  return efforts;
}

/** Gauge + select of "Default" and the model's efforts; hidden without efforts. */
export default function EffortSelect({
  efforts,
  value,
  onChange,
  disabled,
  className,
  id,
}: {
  efforts: readonly string[];
  value: string | null;
  onChange: (effort: string | null) => void;
  disabled: boolean;
  /** Wrapper class; the icon uses `${className}-icon`. */
  className: string;
  id?: string;
}) {
  const { t } = useT();
  if (!efforts.length) return null;
  return (
    <div className={className}>
      <Gauge className={`${className}-icon`} size={15} aria-hidden />
      <Select
        id={id}
        aria-label={t("Reasoning effort")}
        value={value && efforts.includes(value) ? value : ""}
        disabled={disabled}
        options={[
          { value: "", label: t("Default") },
          ...efforts.map((effort) => ({
            value: effort,
            label: effortLabel(effort),
          })),
        ]}
        onChange={(next) => onChange(next || null)}
      />
    </div>
  );
}
