import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { resolve } from "node:path";
import { test } from "node:test";
import Ajv from "ajv/dist/2020.js";

const binary = resolve(
  `src-tauri/target/debug/examples/protocol-probe${process.platform === "win32" ? ".exe" : ""}`,
);

function client(t) {
  const child = spawn(binary, [], { stdio: ["pipe", "pipe", "pipe"] });
  const pending = new Map();
  let next = 0;
  let stderr = "";
  child.stderr.on("data", (bytes) => {
    stderr += bytes;
  });
  child.stdin.on("error", () => {});
  const exit = new Promise((accept) =>
    child.on("exit", (code, signal) => accept({ code, signal })),
  );
  createInterface({ input: child.stdout }).on("line", (line) => {
    const message = JSON.parse(line);
    assert.equal(message.jsonrpc, "2.0", "stdout contains only JSON-RPC");
    if (message.id !== undefined) {
      const entry = pending.get(message.id);
      assert.ok(entry, `unexpected response ${message.id}`);
      pending.delete(message.id);
      entry(message);
    }
  });
  t.after(async () => {
    child.stdin.end();
    child.kill();
    await exit;
  });
  return {
    child,
    exit,
    stderr: () => stderr,
    notify(method, params) {
      child.stdin.write(
        `${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`,
      );
    },
    async call(method, params, fragmented = false) {
      const id = ++next;
      const response = new Promise((accept) => pending.set(id, accept));
      const bytes = Buffer.from(
        `${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`,
      );
      if (fragmented) {
        for (let offset = 0; offset < bytes.length; offset += 3) {
          child.stdin.write(bytes.subarray(offset, offset + 3));
          await new Promise((accept) => setImmediate(accept));
        }
      } else child.stdin.write(bytes);
      return response;
    },
  };
}

for (const version of ["2025-11-25"]) {
  test(
    `rmcp process: ${version} initialize, schemas, image and typed failure`,
    { timeout: 15_000 },
    async (t) => {
      const c = client(t);
      const init = await c.call(
        "initialize",
        {
          protocolVersion: version,
          capabilities: {},
          clientInfo: { name: "independent-node-fixture", version: "1" },
        },
        true,
      );
      assert.equal(init.result.protocolVersion, version);
      assert.deepEqual(Object.keys(init.result.capabilities), ["tools"]);
      c.notify("notifications/initialized");
      const listed = await c.call("tools/list", {});
      assert.equal(listed.result.tools.length, 2);
      const ajv = new Ajv({
        strict: false,
        formats: {
          uint32: {
            type: "number",
            validate: (value) =>
              Number.isInteger(value) && value >= 0 && value <= 4294967295,
          },
        },
      });
      for (const tool of listed.result.tools) {
        assert.equal(tool.inputSchema.additionalProperties, false);
        const response = await c.call(
          "tools/call",
          { name: tool.name, arguments: {} },
          true,
        );
        assert.ok(response.result, JSON.stringify(response));
        const validate = ajv.compile(tool.outputSchema);
        assert.ok(
          validate(response.result.structuredContent),
          JSON.stringify(validate.errors),
        );
        if (tool.name === "probe_image") {
          const image = response.result.content.find(
            (part) => part.type === "image",
          );
          assert.equal(image.mimeType, "image/png");
          assert.equal(Buffer.from(image.data, "base64").readUInt32BE(16), 1);
          assert.equal(response.result.structuredContent.fixture, true);
        } else {
          assert.equal(response.result.isError, true);
          assert.equal(
            response.result.structuredContent.code,
            "TARGET_NOT_FOUND",
          );
        }
        const invalid = await c.call("tools/call", {
          name: tool.name,
          arguments: { approved: true },
        });
        assert.equal(invalid.error.code, -32602);
      }
      assert.equal(
        (await c.call("tools/list", { cursor: "foreign" })).error.code,
        -32602,
      );
      c.child.stdin.end();
      assert.deepEqual(await c.exit, { code: 0, signal: null });
      assert.equal(c.stderr(), "");
    },
  );
}

test(
  "2026-07-28 discovery without legacy initialize",
  { timeout: 15_000 },
  async (t) => {
    const c = client(t);
    const meta = {
      "io.modelcontextprotocol/protocolVersion": "2026-07-28",
      "io.modelcontextprotocol/clientCapabilities": {},
    };
    const discovery = await c.call("server/discover", { _meta: meta });
    assert.ok(discovery.result, JSON.stringify(discovery));
    assert.ok(discovery.result.supportedVersions.includes("2026-07-28"));
    assert.equal(discovery.result.ttlMs, 0);
    assert.equal(discovery.result.cacheScope, "private");
    const result = await c.call("tools/call", {
      name: "probe_image",
      arguments: {},
      _meta: meta,
    });
    assert.ok(result.result, JSON.stringify(result));
    assert.equal(result.result.resultType, "complete");
    assert.ok(result.result.content.some((part) => part.type === "image"));
  },
);

test(
  "oversized MCP line closes before parsing",
  { timeout: 15_000 },
  async (t) => {
    const c = client(t);
    const chunk = Buffer.alloc(8192, 120);
    for (let index = 0; index < 1025; index++) {
      if (!c.child.stdin.write(chunk)) {
        await Promise.race([
          new Promise((accept) => c.child.stdin.once("drain", accept)),
          c.exit,
        ]);
      }
    }
    const result = await c.exit;
    assert.equal(result.signal, null);
    assert.notEqual(result.code, 0);
    assert.ok(c.stderr().length < 2048);
  },
);
