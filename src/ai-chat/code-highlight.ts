import { classHighlighter, highlightTree } from "@lezer/highlight";
import { editorLanguage, loadEditorLanguage } from "../editor-languages";

export const maxHighlightBytes = 100_000;

type Parser = { parse(input: string): Parameters<typeof highlightTree>[0] };
const aliases: Record<string, string> = {
  typescript: "ts",
  javascript: "js",
  node: "js",
  rust: "rs",
  python: "py",
  python3: "py",
  golang: "go",
  "c++": "cpp",
  shell: "sh",
  console: "sh",
  shellscript: "sh",
  yml: "yaml",
  markdown: "md",
};
const parsers = new Map<string, Promise<Parser | null>>();

function parserFor(language: string): Promise<Parser | null> {
  const key = language.toLowerCase();
  let pending = parsers.get(key);
  if (!pending) {
    const definition = editorLanguage(`x.${aliases[key] ?? key}`);
    pending = definition
      ? loadEditorLanguage(definition).then(
          (extension) => {
            // LanguageSupport wraps a Language; StreamLanguage is one already.
            const found = extension as {
              language?: { parser: Parser };
              parser?: Parser;
            };
            return found.language?.parser ?? found.parser ?? null;
          },
          () => null,
        )
      : Promise.resolve(null);
    parsers.set(key, pending);
  }
  return pending;
}

export interface HighlightSpan {
  text: string;
  className?: string;
}

/** Resolves to token spans covering `code`, or null when it cannot be highlighted. */
export async function highlightCode(
  code: string,
  language: string | undefined,
): Promise<HighlightSpan[] | null> {
  if (!language || code.length > maxHighlightBytes) return null;
  const parser = await parserFor(language);
  if (!parser) return null;
  const spans: HighlightSpan[] = [];
  let at = 0;
  try {
    highlightTree(
      parser.parse(code),
      classHighlighter,
      (from, to, className) => {
        if (from > at) spans.push({ text: code.slice(at, from) });
        spans.push({ text: code.slice(from, to), className });
        at = to;
      },
    );
  } catch {
    return null;
  }
  if (at < code.length) spans.push({ text: code.slice(at) });
  return spans;
}
