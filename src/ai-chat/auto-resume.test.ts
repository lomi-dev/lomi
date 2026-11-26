import assert from "node:assert/strict";
import test from "node:test";
import { due, without } from "./auto-resume.ts";

const entries = new Map([
  ["a", { messageId: "m1", retryAtMs: 100, text: "x" }],
  ["b", { messageId: "m2", retryAtMs: 300, text: "y" }],
]);

test("due lists only entries whose reset time has passed", () => {
  assert.deepEqual(due(entries, 99), []);
  assert.deepEqual(due(entries, 100), ["a"]);
  assert.deepEqual(due(entries, 400), ["a", "b"]);
});

test("without removes one thread and leaves the original intact", () => {
  assert.deepEqual([...without(entries, "a").keys()], ["b"]);
  assert.equal(entries.size, 2);
});
