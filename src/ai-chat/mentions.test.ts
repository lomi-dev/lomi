import assert from "node:assert/strict";
import test from "node:test";
import { applyMention, mentionQuery, rankFiles } from "./mentions.ts";

test("mentionQuery finds the token before the caret", () => {
  assert.deepEqual(mentionQuery("@src/a", 6), { start: 0, query: "src/a" });
  assert.deepEqual(mentionQuery("see @ab cd", 7), { start: 4, query: "ab" });
  assert.deepEqual(mentionQuery("@", 1), { start: 0, query: "" });
  assert.equal(mentionQuery("mail@x", 6), null);
  assert.equal(mentionQuery("@ab cd", 6), null);
  assert.equal(mentionQuery("plain", 5), null);
});

test("rankFiles prefers basename, runs and short paths", () => {
  const files = ["docs/a/b/readme.md", "src/main.rs", "src/ma/in.rs", "x.ts"];
  assert.equal(rankFiles(files, "main")[0], "src/main.rs");
  assert.deepEqual(rankFiles(files, "zzz"), []);
  assert.equal(rankFiles(files, "").length, 4);
  assert.equal(rankFiles(files, "rs", 1).length, 1);
  assert.equal(rankFiles(["a/x/foo.ts", "foo.ts"], "foo")[0], "foo.ts");
});

test("applyMention replaces the token and moves the caret", () => {
  assert.deepEqual(applyMention("hi @sr there", 6, "src/a.ts"), {
    text: "hi @src/a.ts  there",
    caret: 13,
  });
  assert.deepEqual(applyMention("@a", 2, "my file.md"), {
    text: "@`my file.md` ",
    caret: 14,
  });
  assert.deepEqual(applyMention("none", 4, "x"), { text: "none", caret: 4 });
});
