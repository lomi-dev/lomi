import assert from "node:assert/strict";
import test from "node:test";
import {
  applyCommand,
  isNewCommand,
  rankCommands,
  slashQuery,
} from "./commands.ts";

test("slashQuery only matches the leading word", () => {
  assert.equal(slashQuery("/co", 3), "co");
  assert.equal(slashQuery("/", 1), "");
  assert.equal(slashQuery("/compact now", 12), null);
  assert.equal(slashQuery("hi /co", 6), null);
  assert.equal(slashQuery("/co", 0), null);
});

test("rankCommands filters by subsequence", () => {
  const list = [{ name: "review" }, { name: "compact" }, { name: "cost" }];
  assert.deepEqual(
    rankCommands(list, "co")
      .map((c) => c.name)
      .sort(),
    ["compact", "cost"],
  );
  assert.equal(rankCommands(list, "").length, 3);
  assert.equal(rankCommands(list, "zz").length, 0);
});

test("applyCommand replaces the token and keeps the rest", () => {
  assert.deepEqual(applyCommand("/co tail", 3, "compact"), {
    text: "/compact tail",
    caret: 9,
  });
});

test("isNewCommand matches only the bare command", () => {
  assert.ok(isNewCommand("/new"));
  assert.ok(isNewCommand("/new  "));
  assert.ok(!isNewCommand("/new x"));
});
