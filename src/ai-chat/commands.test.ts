import assert from "node:assert/strict";
import test from "node:test";
import {
  commandDescription,
  groupCommands,
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

test("isNewCommand matches only the bare command", () => {
  assert.ok(isNewCommand("/new"));
  assert.ok(isNewCommand("/new  "));
  assert.ok(!isNewCommand("/new x"));
});

test("rankCommands matches a skill name without its plugin prefix", () => {
  const list = [{ name: "ponytail:audit" }, { name: "review" }];
  assert.deepEqual(
    rankCommands(list, "aud").map((c) => c.name),
    ["ponytail:audit"],
  );
  assert.deepEqual(
    rankCommands(list, "ponytail:").map((c) => c.name),
    ["ponytail:audit"],
  );
});

test("groupCommands sections and keeps a plugin's skills together", () => {
  const names = ["a:x", "compact", "new", "b:y", "a:z"];
  const groups = groupCommands(names.map((name) => ({ name })));
  assert.deepEqual(
    groups.map((g) => [g.id, g.items.map((c) => c.name)]),
    [
      ["lomi", ["new"]],
      ["commands", ["compact"]],
      ["skills", ["a:x", "a:z", "b:y"]],
    ],
  );
  assert.deepEqual(groupCommands([{ name: "compact" }]).length, 1);
});

test("commandDescription strips a leading plugin tag from skills only", () => {
  assert.equal(commandDescription("p:s", "(p) Does a thing"), "Does a thing");
  assert.equal(commandDescription("compact", "(x) keep"), "(x) keep");
});
