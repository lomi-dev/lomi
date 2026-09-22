import assert from "node:assert/strict";
import test from "node:test";
import { SourceControlState } from "../src/source-control-state.ts";

test("pending work excludes concurrent actions only in the same repository and preserves newer drafts", async () => {
  const state = new SourceControlState();
  let release!: () => void;
  state.update("/first", { message: "Submitted" });
  const pending = state.run("/first", async () => {
    await new Promise<void>((resolve) => {
      release = resolve;
    });
    state.clearSubmittedMessage("/first", "Submitted");
  });
  await assert.rejects(
    state.run("/first", async () => assert.fail("Concurrent action ran")),
  );
  await state.run("/second", async () =>
    state.update("/second", { message: "Independent draft" }),
  );
  state.update("/first", { message: "Newer draft" });
  release();
  await pending;
  assert.equal(state.repository("/first").message, "Newer draft");
  assert.equal(state.repository("/second").message, "Independent draft");
  assert.equal(state.repository("/first").busy, false);
  await assert.rejects(
    state.run("/first", async () => {
      throw new Error("Rejected commit");
    }),
  );
  assert.equal(state.repository("/first").busy, false);
  assert.equal(state.repository("/first").message, "Newer draft");
});
