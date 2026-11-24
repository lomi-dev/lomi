import assert from "node:assert/strict";
import test from "node:test";
import { splitBlocks } from "./markdown-blocks.ts";

const roundTrip = (text: string) => {
  const blocks = splitBlocks(text);
  assert.equal(blocks.join(""), text);
  return blocks;
};

test("splits paragraphs at blank lines, keeping separators", () => {
  assert.deepEqual(roundTrip("a\n\nb\n\n\nc"), ["a\n\n", "b\n\n\n", "c"]);
});

test("does not split inside fences", () => {
  assert.equal(roundTrip("```ts\na\n\nb\n```\n\nafter").length, 2);
  assert.equal(roundTrip("~~~\na\n\nb\n~~~\n\nafter").length, 2);
});

test("a longer outer fence is not closed by a shorter inner one", () => {
  const text = "````md\n```js\n\n```\n\nstill inside\n````\n\nafter";
  assert.equal(roundTrip(text).length, 2);
});

test("an unterminated fence stays one block while streaming", () => {
  assert.equal(roundTrip("intro\n\n```js\nx\n\ny").length, 2);
});

test("keeps lists, quotes, tables and indented content together", () => {
  assert.equal(roundTrip("- a\n\n- b\n\n1. c").length, 1);
  assert.equal(roundTrip("> a\n\n> b").length, 1);
  assert.equal(roundTrip("| a |\n\n| b |").length, 1);
  assert.equal(roundTrip("- a\n\n  more").length, 1);
});

test("handles CRLF, trailing blanks and empty input", () => {
  assert.equal(roundTrip("a\r\n\r\nb").length, 2);
  assert.equal(roundTrip("a\n\n").length, 1);
  assert.deepEqual(splitBlocks(""), []);
});
