import assert from "node:assert/strict";
import test from "node:test";
import { parseUnifiedDiff } from "./unified-diff.ts";

const modified = [
  "diff --git a/a.txt b/a.txt",
  "index 1..2 100644",
  "--- a/a.txt",
  "+++ b/a.txt",
  "@@ -1,3 +1,3 @@",
  " one",
  "-two",
  "+TWO",
  " three",
  "@@ -10,2 +10,3 @@ fn",
  " ten",
  "+ten and a half",
  " eleven",
  "",
].join("\n");

test("parses hunks with line numbers", () => {
  const rows = parseUnifiedDiff(modified);
  assert.equal(rows.filter((row) => row.kind === "hunk").length, 2);
  assert.deepEqual(rows[5], {
    kind: "context",
    oldLine: 1,
    newLine: 1,
    text: "one",
  });
  assert.deepEqual(rows[6], { kind: "del", oldLine: 2, text: "two" });
  assert.deepEqual(rows[7], { kind: "add", newLine: 2, text: "TWO" });
  assert.deepEqual(rows[8], {
    kind: "context",
    oldLine: 3,
    newLine: 3,
    text: "three",
  });
  assert.deepEqual(rows[11], {
    kind: "add",
    newLine: 11,
    text: "ten and a half",
  });
  assert.deepEqual(rows[12], {
    kind: "context",
    oldLine: 11,
    newLine: 12,
    text: "eleven",
  });
});

test("keeps header lines as meta rows", () => {
  const rows = parseUnifiedDiff(modified);
  assert.deepEqual(
    rows.slice(0, 4).map((row) => row.kind),
    ["meta", "meta", "meta", "meta"],
  );
});

test("parses new and deleted files", () => {
  const added = parseUnifiedDiff(
    "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,2 @@\n+a\n+b\n",
  );
  assert.deepEqual(added.slice(-2), [
    { kind: "add", newLine: 1, text: "a" },
    { kind: "add", newLine: 2, text: "b" },
  ]);
  const deleted = parseUnifiedDiff(
    "diff --git a/d b/d\ndeleted file mode 100644\n--- a/d\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n",
  );
  assert.deepEqual(deleted.slice(-2), [
    { kind: "del", oldLine: 1, text: "a" },
    { kind: "del", oldLine: 2, text: "b" },
  ]);
});

test("does not number the no-newline marker", () => {
  const rows = parseUnifiedDiff(
    "@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b\n\\ No newline at end of file\n",
  );
  assert.deepEqual(
    rows.map((row) => row.kind),
    ["hunk", "del", "meta", "add", "meta"],
  );
  assert.equal(rows[3].newLine, 1);
});

test("tolerates the truncation marker", () => {
  const rows = parseUnifiedDiff(
    "@@ -1 +1 @@\n-a\n+b\n[diff truncated at 512 KiB]",
  );
  assert.deepEqual(rows.at(-1), {
    kind: "meta",
    text: "[diff truncated at 512 KiB]",
  });
  assert.equal(parseUnifiedDiff("").length, 0);
});
