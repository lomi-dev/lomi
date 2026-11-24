import assert from "node:assert/strict";
import test from "node:test";
import { decide, editable, enqueue, remove } from "./send-queue.ts";

const a = { id: "a", text: "one", attachments: [] };
const b = {
  id: "b",
  text: "two",
  attachments: [
    {
      id: "f",
      name: "f.txt",
      mime: "text/plain",
      kind: "text" as const,
      size: 1,
    },
  ],
};
const base = {
  wasRunning: true,
  running: false,
  lastState: "completed",
  queued: 1,
  paused: false,
};

test("enqueue keeps order and remove drops by id", () => {
  const queue = enqueue(enqueue([], a), b);
  assert.deepEqual(
    queue.map((item) => item.id),
    ["a", "b"],
  );
  assert.deepEqual(remove(queue, "a"), [b]);
});

test("editable needs an empty composer and no files", () => {
  const queue = [a, b];
  assert.equal(editable(queue, "a", true), a);
  assert.equal(editable(queue, "a", false), undefined);
  assert.equal(editable(queue, "b", true), undefined);
  assert.equal(editable(queue, "x", true), undefined);
});

test("terminal states drain, stopped pauses", () => {
  for (const lastState of ["completed", "failed", "interrupted"])
    assert.equal(decide({ ...base, lastState }), "send");
  assert.equal(decide({ ...base, lastState: "stopped" }), "pause");
});

test("no action while running, paused or empty", () => {
  assert.equal(decide({ ...base, running: true }), "none");
  assert.equal(decide({ ...base, paused: true }), "none");
  assert.equal(decide({ ...base, queued: 0 }), "none");
});

test("an idle queue without a fresh stop is sent", () => {
  assert.equal(
    decide({ ...base, wasRunning: false, lastState: "stopped" }),
    "send",
  );
});
