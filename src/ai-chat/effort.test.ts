import assert from "node:assert/strict";
import test from "node:test";
import { effortFor, initialEffort } from "./effort.ts";
import type { ChatDefaults } from "./types.ts";

const defaults: ChatDefaults = {
  instanceId: "codex",
  model: "m1",
  effort: "high",
  fallback: { instanceId: "claude", model: "opus", effort: "low" },
  permissions: "read-only",
  workspace: "project",
};

test("initialEffort uses the default, then the fallback, else none", () => {
  const pick = (instanceId: string, model: string) =>
    initialEffort(defaults, { instanceId, model });
  assert.equal(pick("codex", "m1"), "high");
  assert.equal(pick("claude", "opus"), "low");
  assert.equal(pick("claude", "sonnet"), null);
  assert.equal(pick("codex", "m2"), null);
  assert.equal(initialEffort(undefined, { instanceId: "a", model: "b" }), null);
});

test("effortFor resets an effort the model lacks", () => {
  assert.equal(effortFor("high", ["low", "high"]), "high");
  assert.equal(effortFor("max", ["low", "high"]), null);
  assert.equal(effortFor("high", []), null);
  assert.equal(effortFor(null, ["high"]), null);
});
