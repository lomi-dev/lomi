export interface DiffRow {
  kind: "hunk" | "context" | "add" | "del" | "meta";
  oldLine?: number;
  newLine?: number;
  text: string;
}

const hunkHeader = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

/**
 * Parses unified diff text into display rows. Tolerant by design: unknown
 * lines before the first hunk, the "\ No newline" marker and a trailing
 * truncation marker never throw.
 */
export function parseUnifiedDiff(diff: string): DiffRow[] {
  const rows: DiffRow[] = [];
  let oldLine = 0;
  let newLine = 0;
  let inHunk = false;
  const lines = diff.split("\n");
  if (lines.at(-1) === "") lines.pop();
  for (const line of lines) {
    const header = hunkHeader.exec(line);
    if (header) {
      oldLine = Number(header[1]);
      newLine = Number(header[2]);
      inHunk = true;
      rows.push({ kind: "hunk", text: line });
    } else if (line.startsWith("diff --git ")) {
      inHunk = false;
      rows.push({ kind: "meta", text: line });
    } else if (!inHunk || line.startsWith("\\") || line.startsWith("[diff ")) {
      // Header lines (---/+++/index), the no-newline marker and truncation notice.
      rows.push({ kind: "meta", text: line });
    } else if (line.startsWith("+")) {
      rows.push({ kind: "add", newLine: newLine++, text: line.slice(1) });
    } else if (line.startsWith("-")) {
      rows.push({ kind: "del", oldLine: oldLine++, text: line.slice(1) });
    } else {
      rows.push({
        kind: "context",
        oldLine: oldLine++,
        newLine: newLine++,
        text: line.slice(1),
      });
    }
  }
  return rows;
}
