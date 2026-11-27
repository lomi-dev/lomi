import assert from "node:assert/strict";
import test from "node:test";
import { sanitizeDictation, spaceForInsert } from "./stt-insert.ts";

test("strips every control that could escape bracketed paste or run a command", () => {
  assert.equal(sanitizeDictation("ls\r\nrm -rf /\n"), "ls rm -rf /");
  assert.equal(sanitizeDictation("a\x1b[201~b"), "a [201~b");
  assert.equal(sanitizeDictation("a\x9b201~\x07b\x7f"), "a 201~ b");
  assert.equal(sanitizeDictation("x\u{2028}y\u{2029}z\tw"), "x y z w");
});

test("keeps ordinary Unicode and collapses whitespace", () => {
  assert.equal(
    sanitizeDictation("  Zażółć   gęślą jaźń  "),
    "Zażółć gęślą jaźń",
  );
  assert.equal(sanitizeDictation("<b>&amp;</p>"), "<b>&amp;</p>");
  assert.equal(sanitizeDictation("\n\n"), "");
});

test("spaces words apart at the insertion point", () => {
  assert.equal(spaceForInsert("", "hello", ""), "hello");
  assert.equal(spaceForInsert("git", "status", ""), " status");
  assert.equal(spaceForInsert("git ", "status", ""), "status");
  assert.equal(spaceForInsert("", "Zrób", "commit"), "Zrób ");
  assert.equal(spaceForInsert("a", "b", "."), " b");
  assert.equal(spaceForInsert("a", "", "b"), "");
});
