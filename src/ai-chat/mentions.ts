export interface MentionToken {
  /** Index of the `@`. */
  start: number;
  query: string;
}

/** The `@query` token ending at the caret, or null. */
export function mentionQuery(text: string, caret: number): MentionToken | null {
  for (let i = caret - 1; i >= 0; i--) {
    const ch = text[i];
    if (/\s/.test(ch)) return null;
    if (ch === "@") {
      if (i > 0 && !/\s/.test(text[i - 1])) return null;
      return { start: i, query: text.slice(i + 1, caret) };
    }
  }
  return null;
}

/** Score of `query` as a subsequence of `s` (both lowercase); -1 when absent. */
function subsequence(s: string, query: string): number {
  let score = 0;
  let pos = 0;
  let run = 0;
  for (const ch of query) {
    const at = s.indexOf(ch, pos);
    if (at < 0) return -1;
    run = at === pos && pos > 0 ? run + 1 : 0;
    score += 1 + run * 2 + (at === 0 || "/-_. ".includes(s[at - 1]) ? 3 : 0);
    pos = at + 1;
  }
  return score;
}

export function rankFiles(
  files: readonly string[],
  query: string,
  limit = 50,
): string[] {
  const q = query.toLowerCase();
  if (!q) return files.slice(0, limit);
  const scored: { file: string; score: number }[] = [];
  for (const file of files) {
    const lower = file.toLowerCase();
    const slash = lower.lastIndexOf("/") + 1;
    const base = subsequence(lower.slice(slash), q);
    const full = subsequence(lower, q);
    if (base < 0 && full < 0) continue;
    // Basename matches outrank path matches; shorter paths break ties.
    const score = (base >= 0 ? 1000 + base * 4 : full) - file.length * 0.01;
    scored.push({ file, score });
  }
  return scored
    .sort((a, b) => b.score - a.score)
    .slice(0, limit)
    .map((item) => item.file);
}

/** Replaces the token before the caret with `@path ` (backticked if spaced). */
export function applyMention(
  text: string,
  caret: number,
  path: string,
): { text: string; caret: number } {
  const token = mentionQuery(text, caret);
  if (!token) return { text, caret };
  const ref = /\s/.test(path) ? `@\`${path}\` ` : `@${path} `;
  return {
    text: text.slice(0, token.start) + ref + text.slice(caret),
    caret: token.start + ref.length,
  };
}
