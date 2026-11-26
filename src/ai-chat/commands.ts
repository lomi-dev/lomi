import { rankFiles } from "./mentions.ts";

/** The `/query` token when the caret is inside the leading word, else null. */
export function slashQuery(text: string, caret: number): string | null {
  if (!text.startsWith("/") || caret < 1) return null;
  const head = text.slice(1, caret);
  return /\s/.test(head) ? null : head;
}

/** Commands matching `query` (prefix and subsequence), best first. */
export function rankCommands<T extends { name: string }>(
  commands: readonly T[],
  query: string,
): T[] {
  const byName = new Map(commands.map((command) => [command.name, command]));
  return rankFiles([...byName.keys()], query).map((name) => byName.get(name)!);
}

/** Replaces the leading `/query` with `/name `. */
export function applyCommand(text: string, caret: number, name: string) {
  const rest = text.slice(caret);
  const insert = `/${name}${rest.startsWith(" ") ? "" : " "}`;
  return { text: insert + rest, caret: name.length + 2 };
}

/** `/new` runs locally instead of being sent. */
export const isNewCommand = (text: string) => /^\/new\s*$/.test(text);
