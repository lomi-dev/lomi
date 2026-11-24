const fenceOpen = /^ {0,3}(`{3,}|~{3,})/;
// Lines that may continue a list, quote, table or indented block after a blank line.
const continuation = /^(?:[-*+]\s|\d+[.)]\s|>|\||\s{2,}|\t)/;

/**
 * Splits markdown at top-level blank lines so settled blocks can be memoized.
 * Separators stay attached to the preceding block: `splitBlocks(t).join("") === t`.
 */
export function splitBlocks(text: string): string[] {
  const lines = text.split(/(?<=\n)/);
  const blocks: string[] = [];
  let start = 0;
  let offset = 0;
  let fence: { char: string; length: number } | null = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    offset += line.length;
    const body = line.replace(/\r?\n$/, "");
    if (fence) {
      const close = /^ {0,3}(`{3,}|~{3,})\s*$/.exec(body);
      if (close?.[1][0] === fence.char && close[1].length >= fence.length)
        fence = null;
      continue;
    }
    const open = fenceOpen.exec(body);
    // A backtick fence's info string cannot contain backticks.
    if (
      open &&
      !(open[1][0] === "`" && body.slice(open[0].length).includes("`"))
    )
      fence = { char: open[1][0], length: open[1].length };
    if (body.trim() || i === lines.length - 1) continue;
    let next = i + 1;
    while (next < lines.length && !lines[next].trim()) next++;
    if (next >= lines.length) continue;
    if (continuation.test(lines[next])) continue;
    // Absorb the whole run of blank lines into this block.
    offset = lines.slice(0, next).join("").length;
    i = next - 1;
    blocks.push(text.slice(start, offset));
    start = offset;
  }
  if (start < text.length) blocks.push(text.slice(start));
  return blocks;
}
