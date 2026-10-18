import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";
import { validateSnapshot } from "../src/snapshot.mjs";
const sessionId = "11111111-1111-4111-8111-111111111111",
  epoch = "22222222-2222-4222-8222-222222222222";
function helper() {
  const child = spawn(process.execPath, ["src/index.mjs"], {
    cwd: new URL("..", import.meta.url),
    stdio: ["pipe", "pipe", "pipe"],
    env: { PATH: process.env.PATH },
  });
  const reader = createInterface({ input: child.stdout });
  const replies = [];
  reader.on("line", (v) => {
    const resolve = replies.shift();
    if (resolve) resolve(JSON.parse(v));
  });
  return {
    child,
    request: (v) =>
      new Promise((resolve) => {
        replies.push(resolve);
        child.stdin.write(JSON.stringify(v) + "\n");
      }),
  };
}
test("private subprocess processes ordered output and snapshot barriers", async () => {
  const h = helper();
  try {
    let r = await h.request({
      id: 1,
      op: "create",
      sessionId,
      epoch,
      seq: 0,
      cols: 20,
      rows: 5,
    });
    assert.equal(r.ok, true);
    r = await h.request({
      id: 2,
      op: "write",
      sessionId,
      epoch,
      seq: 1,
      data: Buffer.from("PTY zażółć界\r\n\x1b[31mRED").toString("base64"),
    });
    assert.equal(r.ok, true);
    r = await h.request({ id: 3, op: "snapshot", sessionId, epoch, seq: 1 });
    assert.equal(r.ok, true);
    validateSnapshot(r.result.state);
    assert.equal(r.result.throughSeq, 1);
    r = await h.request({
      id: 4,
      op: "write",
      sessionId,
      epoch: "33333333-3333-4333-8333-333333333333",
      seq: 2,
      data: "",
    });
    assert.equal(r.ok, false);
    assert.equal(r.error, "STALE_SESSION");
    r = await h.request({
      id: 5,
      op: "write",
      sessionId,
      epoch,
      seq: 4,
      data: "",
    });
    assert.equal(r.error, "STALE_SEQUENCE");
    h.child.stdin.end();
    await once(h.child, "exit");
    assert.equal(h.child.exitCode, 0);
  } finally {
    h.child.kill();
  }
});
test("oversized private IPC fails closed without exposing request bytes", async () => {
  const h = helper();
  let output = "";
  h.child.stdout.on("data", (v) => (output += v));
  h.child.stdin.on("error", () => {});
  h.child.stdin.write("X".repeat(12 * 1024 * 1024 + 2));
  await once(h.child, "exit");
  assert.equal(h.child.exitCode, 1);
  assert.equal(output.includes("XXXX"), false);
});

test("native restore accepts bounded large checkpoints and preserves suffix parser", async () => {
  const original = helper(),
    restored = helper();
  try {
    const base = { sessionId, epoch };
    assert.equal(
      (
        await original.request({
          ...base,
          id: 1,
          op: "create",
          seq: 0,
          cols: 500,
          rows: 24,
        })
      ).ok,
      true,
    );
    await original.request({
      ...base,
      id: 2,
      op: "write",
      seq: 1,
      data: Buffer.from("before\x1b[3").toString("base64"),
    });
    const snapshot = (
      await original.request({ ...base, id: 3, op: "snapshot", seq: 1 })
    ).result;
    const request = {
      ...base,
      id: 1,
      op: "restore",
      seq: snapshot.throughSeq,
      cols: snapshot.cols,
      rows: snapshot.rows,
      state: snapshot.state,
      suffix: snapshot.suffix,
    };
    assert.ok(Buffer.byteLength(JSON.stringify(request)) > 32768);
    assert.equal((await restored.request(request)).ok, true);
    for (const h of [original, restored]) {
      assert.equal(
        (
          await h.request({
            ...base,
            id: 4,
            op: "resize",
            seq: 2,
            cols: 80,
            rows: 24,
          })
        ).ok,
        true,
      );
      assert.equal(
        (
          await h.request({
            ...base,
            id: 5,
            op: "write",
            seq: 3,
            data: Buffer.from("1mRED").toString("base64"),
          })
        ).ok,
        true,
      );
    }
    assert.deepEqual(
      (await original.request({ ...base, id: 6, op: "snapshot", seq: 3 }))
        .result,
      (await restored.request({ ...base, id: 6, op: "snapshot", seq: 3 }))
        .result,
    );
    assert.equal(
      (
        await restored.request({
          ...base,
          id: 7,
          op: "write",
          seq: 4,
          data: Buffer.alloc(16385).toString("base64"),
        })
      ).error,
      "INVALID_DATA",
    );
  } finally {
    original.child.kill();
    restored.child.kill();
  }
});

test("invalid native restore does not consume a session slot", async () => {
  const h = helper();
  try {
    const base = { sessionId, epoch };
    assert.equal(
      (
        await h.request({
          ...base,
          id: 1,
          op: "restore",
          seq: 7,
          cols: 20,
          rows: 5,
          state: {},
          suffix: "",
        })
      ).ok,
      false,
    );
    assert.equal(
      (
        await h.request({
          ...base,
          id: 2,
          op: "create",
          seq: 0,
          cols: 20,
          rows: 5,
        })
      ).ok,
      true,
    );
  } finally {
    h.child.kill();
  }
});
