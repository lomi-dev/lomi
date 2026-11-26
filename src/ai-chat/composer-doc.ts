import { mentionRef } from "./mentions.ts";

/** Minimal Tiptap/ProseMirror JSON shape used by the composer. */
export interface DocNode {
  type: string;
  text?: string;
  attrs?: Record<string, unknown>;
  content?: DocNode[];
}

function inlineText(node: DocNode): string {
  switch (node.type) {
    case "text":
      return node.text ?? "";
    case "hardBreak":
      return "\n";
    case "fileMention":
      return mentionRef(String(node.attrs?.path ?? ""));
    case "slashCommand":
      return `/${String(node.attrs?.name ?? "")}`;
    default:
      return (node.content ?? []).map(inlineText).join("");
  }
}

/**
 * Serializes the editor document to the plain draft string: paragraphs and
 * hard breaks become "\n", pills become what the popups used to insert.
 * Serializing `doc.cut(0, pos)` yields a prefix of the full text, so its
 * length is the caret offset used by the mention and slash helpers.
 */
export function docToText(doc: DocNode): string {
  return (doc.content ?? []).map(inlineText).join("\n");
}

/** Plain-text document: one paragraph per line, no pills. */
export function textToDoc(text: string): DocNode {
  return {
    type: "doc",
    content: text.split("\n").map((line) => ({
      type: "paragraph",
      ...(line && { content: [{ type: "text", text: line }] }),
    })),
  };
}
