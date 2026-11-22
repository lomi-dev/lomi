import type { ChatDefaults } from "./types";

/** Effort prefilled for a new chat: the default model's, else the fallback's, else none. */
export function initialEffort(
  defaults: ChatDefaults | undefined,
  choice: { instanceId: string; model: string },
): string | null {
  const same = (item?: { instanceId: string | null; model: string | null }) =>
    item?.instanceId === choice.instanceId && item?.model === choice.model;
  if (same(defaults)) return defaults?.effort ?? null;
  if (same(defaults?.fallback ?? undefined))
    return defaults?.fallback?.effort ?? null;
  return null;
}

/** The effort when the model accepts it; otherwise null (Default). */
export const effortFor = (
  effort: string | null | undefined,
  efforts: readonly string[],
) => (effort && efforts.includes(effort) ? effort : null);
