import assert from "node:assert/strict";
import test from "node:test";
import {
  collapseGroup,
  compactNumber,
  hasPendingApproval,
  interleave,
  lineDiff,
  mergeActivity,
  relativeTitle,
  upsertActivity,
} from "./activity.ts";
import type { Activity } from "./types.ts";

const act = (
  id: string,
  at: number,
  status: Activity["status"] = "completed",
): Activity => ({
  id,
  kind: "other",
  title: id,
  status,
  at,
  detail: null,
  diff: null,
});

test("interleave clamps offsets beyond the text", () => {
  const parts = interleave("abc", [act("a", 99), act("b", -4)]);
  assert.deepEqual(
    parts.map((p) =>
      p.type === "text" ? p.text : p.activities.map((x) => x.id).join(),
    ),
    ["b", "abc", "a", ""],
  );
});

test("interleave groups activities at the same offset", () => {
  const parts = interleave("hello", [act("a", 2), act("b", 2), act("c", 4)]);
  assert.equal(parts.length, 5);
  assert.equal(parts[1].type === "group" && parts[1].activities.length, 2);
  assert.deepEqual(parts[2], { type: "text", text: "ll" });
});

test("interleave handles surrogate pairs", () => {
  const text = "a😀b";
  const after = interleave(text, [act("x", 3)]);
  assert.deepEqual(after[0], { type: "text", text: "a😀" });
  const middle = interleave(text, [act("x", 2)]);
  assert.deepEqual(middle[0], { type: "text", text: "a😀" });
});

test("interleave without activity yields one text part", () => {
  assert.deepEqual(interleave("hi", []), [{ type: "text", text: "hi" }]);
});

test("merge lets live win and keeps first-appearance order", () => {
  const live = { ...act("a", 0, "failed"), title: "new" };
  const merged = mergeActivity([act("a", 0), act("b", 1)], [act("c", 2), live]);
  assert.deepEqual(
    merged.map((m) => m.id),
    ["a", "b", "c"],
  );
  assert.equal(merged[0].title, "new");
  assert.deepEqual(upsertActivity([act("a", 0)], live)[0], live);
  assert.equal(upsertActivity([act("a", 0)], act("b", 0)).length, 2);
});

test("lineDiff marks a new file as all added", () => {
  assert.deepEqual(lineDiff(null, "x\ny"), [
    { sign: "+", text: "x" },
    { sign: "+", text: "y" },
  ]);
});

test("lineDiff trims unchanged leading and trailing lines", () => {
  assert.deepEqual(lineDiff("a\nb\nc\nd", "a\nB\nc\nd"), [
    { sign: "-", text: "b" },
    { sign: "+", text: "B" },
  ]);
  assert.deepEqual(lineDiff("a\nb", "a\nb"), []);
  assert.deepEqual(lineDiff("a\nb", "a"), [{ sign: "-", text: "b" }]);
});

test("collapseGroup collapses only above the threshold", () => {
  const three = [act("1", 0), act("2", 0), act("3", 0)];
  assert.equal(collapseGroup(three).collapsed.length, 0);
  const many = [
    ...three,
    act("4", 0),
    act("5", 0, "running"),
    act("6", 0, "failed"),
  ];
  const { collapsed, visible } = collapseGroup(many);
  assert.equal(collapsed.length, 4);
  assert.deepEqual(
    visible.map((v) => v.id),
    ["5", "6"],
  );
});

test("relativeTitle and compactNumber", () => {
  assert.equal(relativeTitle("/p/q/src/a.ts", "/p/q"), "src/a.ts");
  assert.equal(relativeTitle("/other/a.ts", "/p/q"), "/other/a.ts");
  assert.equal(compactNumber(12345, "en"), "12.3K");
  assert.equal(compactNumber(999, "en"), "999");
});

test("pending approvals are never collapsed", () => {
  const pending: Activity = {
    ...act("p", 0, "completed"),
    approval: { requestId: "r", state: "pending", canRemember: false },
  };
  const group = [act("a", 0), act("b", 0), act("c", 0), act("d", 0), pending];
  const { collapsed, visible } = collapseGroup(group);
  assert.deepEqual(
    visible.map((item) => item.id),
    ["p"],
  );
  assert.equal(collapsed.length, 4);
  assert.equal(collapseGroup([pending]).visible.length, 1);
});

test("hasPendingApproval detects only pending prompts", () => {
  const withState = (state: "pending" | "allowed"): Activity => ({
    ...act("x", 0),
    approval: { requestId: "r", state, canRemember: true },
  });
  assert.equal(hasPendingApproval([act("a", 0), withState("pending")]), true);
  assert.equal(hasPendingApproval([act("a", 0), withState("allowed")]), false);
});
