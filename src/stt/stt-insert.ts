// Pure helpers that turn a transcript into text safe to insert.

// C0/C1 controls, DEL and the Unicode line/paragraph separators. In a terminal
// any of them could end bracketed paste or submit a command.
const CONTROLS = /[\u0000-\u001f\u007f-\u009f\u{2028}\u{2029}]/gu;

/** Single-line plain text: controls become spaces, whitespace collapses. */
export function sanitizeDictation(text: string): string {
  return text.replace(CONTROLS, " ").replace(/\s+/g, " ").trim();
}

/** Adds the spaces needed to keep words apart at the insertion point. */
export function spaceForInsert(
  before: string,
  text: string,
  after: string,
): string {
  if (!text) return text;
  const lead = before !== "" && !/\s$/.test(before) ? " " : "";
  const trail = /^[\p{L}\p{N}]/u.test(after) ? " " : "";
  return lead + text + trail;
}
