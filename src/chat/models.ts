import capabilities from "./model-capabilities.json";
import type { Connection } from "./types";
import {
  providerIds,
  providerPresets,
  type Provider,
} from "./provider-presets";

export const providers = Object.fromEntries(
  providerIds.map((id) => [id, providerPresets[id].name]),
) as Record<Provider, string>;

export function modelOptions(
  connection?: Pick<Connection, "provider" | "models">,
) {
  if (!connection) return [];
  return [
    ...new Set([
      ...providerPresets[connection.provider].models,
      ...capabilities.models
        .filter((model) => model.provider === connection.provider)
        .map((model) => model.id),
      ...connection.models,
    ]),
  ];
}

export function suggestedModel(
  connection?: Pick<Connection, "provider" | "models" | "testedModel">,
) {
  if (!connection) return "";
  if (connection.provider === "custom")
    return connection.models[0] || connection.testedModel || "";
  return connection.testedModel || modelOptions(connection)[0] || "";
}

export function modelLabel(id: string) {
  const labels: Record<string, string> = {
    "gpt-4.1": "GPT-4.1",
    "gpt-4.1-2025-04-14": "GPT-4.1 · Apr 2025",
    "claude-opus-5": "Claude Opus 5",
    "gemini-2.5-flash": "Gemini 2.5 Flash",
    "gemini-2.5-pro": "Gemini 2.5 Pro",
  };
  return labels[id] ?? id;
}
