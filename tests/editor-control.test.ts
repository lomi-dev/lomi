import assert from "node:assert/strict";
import { test } from "node:test";
import { EditorState } from "@codemirror/state";
import { bufferChanges, readBufferSlice } from "../src/editor-control.ts";

test("buffer pagination uses UTF-16 without splitting supplementary characters", () => {
  const doc = EditorState.create({ doc: "a🙂\nb" }).doc;
  assert.deepEqual(readBufferSlice(doc, 0, 2), {
    content: "a",
    startUtf16: 0,
    totalUtf16: 5,
    nextUtf16: 1,
    truncated: true,
  });
  assert.equal(readBufferSlice(doc, 1, 2).content, "🙂");
  assert.equal(readBufferSlice(doc, 3, 2).content, "\nb");
  assert.equal(readBufferSlice(doc, 5, 2).nextUtf16, null);
  for (const [start, count] of [
    [2, 2],
    [-1, 2],
    [6, 2],
    [0.5, 2],
    [0, 1],
    [0, 8193],
  ]) {
    assert.throws(
      () => readBufferSlice(doc, start, count),
      /RESOURCE_EXHAUSTED/,
    );
  }
});

test("buffer edits validate every range before constructing a transaction", () => {
  const original = EditorState.create({ doc: "a🙂\nb" });
  const edits = [
    { fromUtf16: 0, toUtf16: 1, insert: "A" },
    { fromUtf16: 4, toUtf16: 5, insert: "B\nnew" },
  ];
  const changed = original.update({
    changes: bufferChanges(original.doc, edits),
  }).state;
  assert.equal(changed.doc.toString(), "A🙂\nB\nnew");
  for (const bad of [
    { fromUtf16: 2, toUtf16: 3, insert: "x" },
    { fromUtf16: 4, toUtf16: 6, insert: "x" },
    { fromUtf16: 0, toUtf16: 1, insert: "x" },
    { fromUtf16: 4, toUtf16: 5, insert: "\ud800" },
    { fromUtf16: 4, toUtf16: 5, insert: "\r\n" },
    { fromUtf16: 4, toUtf16: 5, insert: "\0" },
    { fromUtf16: 4, toUtf16: 5, insert: "🙂".repeat(20000) },
  ]) {
    assert.throws(() => bufferChanges(original.doc, [edits[0], bad]));
    assert.equal(original.doc.toString(), "a🙂\nb");
  }
  assert.throws(() => bufferChanges(original.doc, []));
  assert.throws(() =>
    bufferChanges(original.doc, [{ fromUtf16: 0, toUtf16: 0, insert: "" }]),
  );
});
