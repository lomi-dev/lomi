import { rankFiles } from "./mentions.ts";

/** The `/query` token when the caret is inside the leading word, else null. */
export function slashQuery(text: string, caret: number): string | null {
  if (!text.startsWith("/") || caret < 1) return null;
  const head = text.slice(1, caret);
  return /\s/.test(head) ? null : head;
}

/** `plugin:name` skills split into plugin and bare name; others have no plugin. */
export function splitCommandName(name: string): {
  plugin: string;
  base: string;
} {
  const colon = name.indexOf(":");
  return colon > 0
    ? { plugin: name.slice(0, colon), base: name.slice(colon + 1) }
    : { plugin: "", base: name };
}

/** Commands matching `query`: bare skill names first, then full `plugin:name`. */
export function rankCommands<T extends { name: string }>(
  commands: readonly T[],
  query: string,
): T[] {
  const byName = new Map(commands.map((command) => [command.name, command]));
  const byBase = new Map<string, T[]>();
  for (const command of byName.values()) {
    const { base } = splitCommandName(command.name);
    byBase.set(base, [...(byBase.get(base) ?? []), command]);
  }
  const hits = new Set([
    ...rankFiles([...byBase.keys()], query, Infinity).flatMap((base) =>
      byBase.get(base)!,
    ),
    ...rankFiles([...byName.keys()], query, Infinity).map((name) =>
      byName.get(name)!,
    ),
  ]);
  return [...hits];
}

export type CommandSectionId = "lomi" | "commands" | "skills";

/** `/new` is handled by Lomi itself instead of the agent. */
const localCommands = new Set(["new"]);

/**
 * Splits ranked commands into the Lomi, Commands and Skills sections (empty
 * ones dropped). Skills of one plugin stay together, plugins in rank order.
 */
export function groupCommands<T extends { name: string }>(
  ranked: readonly T[],
): { id: CommandSectionId; items: T[] }[] {
  const plugins = [
    ...new Set(ranked.map((item) => splitCommandName(item.name).plugin)),
  ].filter(Boolean);
  const skills = plugins.flatMap((plugin) =>
    ranked.filter((item) => splitCommandName(item.name).plugin === plugin),
  );
  return [
    {
      id: "lomi" as const,
      items: ranked.filter((c) => localCommands.has(c.name)),
    },
    {
      id: "commands" as const,
      items: ranked.filter(
        (c) => !localCommands.has(c.name) && !splitCommandName(c.name).plugin,
      ),
    },
    { id: "skills" as const, items: skills },
  ].filter((section) => section.items.length > 0);
}

/** Skill descriptions often start with "(plugin)"; the row shows a badge instead. */
export const commandDescription = (name: string, description: string) =>
  splitCommandName(name).plugin
    ? description.replace(/^\([^)]*\)\s*/, "")
    : description;

/** `/new` runs locally instead of being sent. */
export const isNewCommand = (text: string) => /^\/new\s*$/.test(text);
