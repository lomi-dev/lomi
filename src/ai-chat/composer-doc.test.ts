import assert from "node:assert/strict";
import test from "node:test";
import { docToText, textToDoc, type DocNode } from "./composer-doc.ts";

const p = (...content: DocNode[]): DocNode => ({ type: "paragraph", content });
const text = (value: string): DocNode => ({ type: "text", text: value });

test("textToDoc and docToText round-trip plain text", () => {
  for (const sample of ["", "one", "a\nb", "a\n\nb\n", "\n"])
    assert.equal(docToText(textToDoc(sample)), sample);
});

test("hard breaks and paragraphs both serialize to newlines", () => {
  const doc: DocNode = {
    type: "doc",
    content: [p(text("a"), { type: "hardBreak" }, text("b")), p(text("c"))],
  };
  assert.equal(docToText(doc), "a\nb\nc");
});

test("pills serialize like the popups insert them", () => {
  const doc: DocNode = {
    type: "doc",
    content: [
      p(
        { type: "slashCommand", attrs: { name: "new" } },
        text(" "),
        { type: "fileMention", attrs: { path: "src/a.ts" } },
        text(" "),
        { type: "fileMention", attrs: { path: "my dir/b.ts" } },
        text(" "),
      ),
    ],
  };
  assert.equal(docToText(doc), "/new @src/a.ts @`my dir/b.ts` ");
});
