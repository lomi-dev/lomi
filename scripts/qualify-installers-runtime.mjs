import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawn } from "node:child_process";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";

const [executable, resources, label, originalNode] = process.argv.slice(2);
assert.ok(executable && resources && /^[a-z-]+$/.test(label));
const binary = resolve(executable);
const node = join(
  dirname(binary),
  process.platform === "win32" ? "lomi-node.exe" : "lomi-node",
);
const ai = join(resolve(resources), "ai-runtime");
const metadata = JSON.parse(await readFile(join(ai, "node.json"), "utf8"));
const manifest = JSON.parse(
  await readFile(
    new URL("../packages/ai-runtime/node-artifacts.json", import.meta.url),
    "utf8",
  ),
);
const target = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
}[`${process.platform}-${process.arch}`];
assert.equal(metadata.target, target);
assert.equal(metadata.version, manifest.version);
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
for (const directory of [ai, join(resolve(resources), "remote-terminal")]) {
  const bundle = await readFile(join(directory, "index.cjs"));
  const expected = (
    await readFile(join(directory, "index.cjs.sha256"), "utf8")
  ).trim();
  assert.equal(digest(bundle), expected, `${directory} bundle integrity`);
  assert.ok(
    (await readFile(join(directory, "THIRD-PARTY-NOTICES"))).length > 100,
  );
  assert.ok(
    !(await readdir(directory)).includes("fixture.cjs"),
    "No probe fixture in production package",
  );
}
assert.ok((await readFile(join(ai, "NODE-LICENSE"))).length > 100);
function elfSections(bytes) {
  assert.deepEqual(bytes.subarray(0, 6), Buffer.from([127, 69, 76, 70, 2, 1]));
  assert.equal(bytes.readUInt16LE(18), 62, "Expected x86_64 ELF");
  const offset = Number(bytes.readBigUInt64LE(40));
  const stride = bytes.readUInt16LE(58);
  const count = bytes.readUInt16LE(60);
  const sections = Array.from({ length: count }, (_, index) => {
    const header = offset + index * stride;
    return {
      nameOffset: bytes.readUInt32LE(header),
      type: bytes.readUInt32LE(header + 4),
      flags: bytes.readBigUInt64LE(header + 8),
      offset: Number(bytes.readBigUInt64LE(header + 24)),
      size: Number(bytes.readBigUInt64LE(header + 32)),
    };
  });
  const names = sections[bytes.readUInt16LE(62)];
  return new Map(
    sections.map((section) => {
      const start = names.offset + section.nameOffset;
      const name = bytes.toString("utf8", start, bytes.indexOf(0, start));
      return [
        name,
        {
          type: section.type,
          flags: section.flags,
          size: section.size,
          // SHT_NOBITS has a memory size but no bytes in the file.
          bytes:
            section.type === 8
              ? Buffer.alloc(0)
              : bytes.subarray(section.offset, section.offset + section.size),
        },
      ];
    }),
  );
}

if (originalNode) {
  assert.equal(process.platform, "linux");
  assert.equal(label, "appimage-extracted");
  const original = await readFile(originalNode);
  assert.equal(digest(original), metadata.sha256, "Reference Node integrity");
  const expected = elfSections(original);
  const actual = elfSections(await readFile(node));
  assert.deepEqual([...actual.keys()].sort(), [...expected.keys()].sort());
  // linuxdeploy relocates ELF linking/symbol metadata when adding RUNPATH.
  // All other sections, including every code and data section, stay identical.
  for (const [name, section] of expected) {
    assert.equal(actual.get(name).type, section.type, name);
    assert.equal(actual.get(name).flags, section.flags, name);
    if ([".dynamic", ".dynstr"].includes(name)) continue;
    assert.equal(actual.get(name).size, section.size, name);
    if ([".symtab", ".dynsym"].includes(name)) continue;
    assert.equal(digest(actual.get(name).bytes), digest(section.bytes), name);
  }
  assert.deepEqual(
    actual.get(".dynstr").bytes,
    Buffer.concat([
      expected.get(".dynstr").bytes,
      Buffer.from("$ORIGIN/../lib\0"),
    ]),
  );
  assert.equal(
    execFileSync("patchelf", ["--print-rpath", node], {
      encoding: "utf8",
    }).trim(),
    "$ORIGIN/../lib",
  );
  assert.equal(
    execFileSync("patchelf", ["--print-needed", node], { encoding: "utf8" }),
    execFileSync("patchelf", ["--print-needed", originalNode], {
      encoding: "utf8",
    }),
  );
} else if (process.platform !== "darwin") {
  // Ad-hoc signing legitimately changes Mach-O bytes after preparation.
  assert.equal(digest(await readFile(node)), metadata.sha256);
}
const env = Object.fromEntries(
  ["SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR", "LANG"]
    .filter((key) => process.env[key])
    .map((key) => [key, process.env[key]]),
);
assert.equal(
  execFileSync(node, ["--version"], {
    env,
    encoding: "utf8",
    timeout: 15000,
  }).trim(),
  `v${manifest.version}`,
);
assert.match(
  execFileSync(binary, ["--mcp", "--version"], {
    encoding: "utf8",
    timeout: 15000,
  }),
  /^lomi-mcp \S+ \(control API 1\.0, IPC 1\)/,
);

const text = "Installed runtime works 日本語";
const sse = (event) =>
  `${event.type ? `event: ${event.type}\n` : ""}data: ${JSON.stringify(event)}\n\n`;
function response(path) {
  if (path === "/v1/chat/completions")
    return (
      [
        {
          id: "reply",
          object: "chat.completion.chunk",
          created: 1,
          model: "local/model",
          choices: [
            {
              index: 0,
              delta: { role: "assistant", content: text },
              finish_reason: null,
            },
          ],
        },
        {
          id: "reply",
          object: "chat.completion.chunk",
          created: 1,
          model: "local/model",
          choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
        },
      ]
        .map(sse)
        .join("") + "data: [DONE]\n\n"
    );
  if (path === "/v1/responses")
    return (
      [
        {
          type: "response.output_item.added",
          output_index: 0,
          item: { type: "message", id: "message-1" },
        },
        {
          type: "response.output_text.delta",
          item_id: "message-1",
          output_index: 0,
          delta: text,
        },
        {
          type: "response.output_item.done",
          output_index: 0,
          item: { type: "message", id: "message-1" },
        },
        {
          type: "response.completed",
          response: {
            usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
          },
        },
      ]
        .map(sse)
        .join("") + "data: [DONE]\n\n"
    );
  assert.equal(path, "/v1/messages");
  return [
    {
      type: "message_start",
      message: {
        id: "message-1",
        type: "message",
        role: "assistant",
        content: [],
        model: "local/model",
        stop_reason: null,
        stop_sequence: null,
        usage: { input_tokens: 1, output_tokens: 0 },
      },
    },
    {
      type: "content_block_start",
      index: 0,
      content_block: { type: "text", text: "" },
    },
    {
      type: "content_block_delta",
      index: 0,
      delta: { type: "text_delta", text },
    },
    { type: "content_block_stop", index: 0 },
    {
      type: "message_delta",
      delta: { stop_reason: "end_turn", stop_sequence: null },
      usage: { output_tokens: 1 },
    },
    { type: "message_stop" },
  ]
    .map(sse)
    .join("");
}
const requests = [];
let serverError;
const server = createServer(async (req, res) => {
  try {
    const parts = [];
    for await (const part of req) parts.push(part);
    const body = JSON.parse(Buffer.concat(parts).toString());
    assert.equal(req.method, "POST");
    assert.equal(body.model, "local/model");
    assert.equal(body.stream, true);
    assert.ok(
      req.headers.authorization === undefined ||
        req.headers.authorization === "Bearer packaging-test-key",
    );
    assert.ok(
      req.headers["x-api-key"] === undefined ||
        req.headers["x-api-key"] === "packaging-test-key",
    );
    requests.push({
      path: req.url,
      authorization: req.headers.authorization,
      apiKey: req.headers["x-api-key"],
    });
    res.writeHead(200, { "content-type": "text/event-stream" });
    res.end(response(req.url));
  } catch (error) {
    serverError = error;
    res.writeHead(500).end();
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const child = spawn(node, ["--no-warnings", join(ai, "index.cjs")], {
  env,
  stdio: "pipe",
});
const events = [];
let childError;
let stderr = "";
child.on("error", (error) => {
  childError = error;
});
child.stdin.on("error", (error) => {
  childError = error;
});
child.stderr.on("data", (data) => {
  stderr += data;
});
const lines = createInterface({ input: child.stdout });
lines.on("line", (line) => {
  try {
    events.push(JSON.parse(line));
  } catch (error) {
    childError = error;
  }
});
const send = (type, requestId, payload) =>
  child.stdin.write(
    JSON.stringify({ protocolVersion: 1, requestId, type, payload }) + "\n",
  );
async function waitFor(predicate) {
  const deadline = Date.now() + 20000;
  while (!predicate()) {
    if (serverError || childError) throw serverError || childError;
    assert.equal(child.exitCode, null, `Runtime exited: ${stderr}`);
    assert.ok(
      Date.now() < deadline,
      `Runtime timeout: ${JSON.stringify(events.slice(-3))}`,
    );
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}
const formats = ["chat-completions", "responses", "anthropic-messages"];
try {
  send("hello", "hello");
  await waitFor(() => events.some((event) => event.type === "ready"));
  assert.equal(events[0].payload.node, manifest.version);
  for (const apiFormat of formats)
    for (const apiKey of ["", "packaging-test-key"]) {
      const requestId = `${apiFormat}-${apiKey ? "key" : "anonymous"}`;
      const generation = {
        provider: "custom",
        apiFormat,
        apiKey,
        baseUrl: `http://127.0.0.1:${server.address().port}/v1`,
        model: "local/model",
        assistantId: "assistant",
        messages: [
          {
            id: "user",
            role: "user",
            parts: [{ type: "text", text: "Hello" }],
          },
        ],
      };
      send("begin", requestId);
      send("append", requestId, {
        data: Buffer.from(JSON.stringify(generation)).toString("base64"),
      });
      send("generate", requestId);
      await waitFor(() =>
        events.some(
          (event) =>
            event.requestId === requestId &&
            ["completed", "failed"].includes(event.type),
        ),
      );
      const result = events.filter((event) => event.requestId === requestId);
      assert.equal(
        result.at(-1).type,
        "completed",
        JSON.stringify(result.at(-1)),
      );
      assert.equal(
        result
          .filter(
            (event) =>
              event.type === "chunk" && event.payload.type === "text-delta",
          )
          .map((event) => event.payload.delta)
          .join(""),
        text,
      );
      assert.equal(
        requests.at(-1).path,
        `/v1/${{ "chat-completions": "chat/completions", responses: "responses", "anthropic-messages": "messages" }[apiFormat]}`,
      );
      assert.equal(
        requests.at(-1).authorization,
        apiKey && apiFormat !== "anthropic-messages"
          ? `Bearer ${apiKey}`
          : undefined,
      );
      assert.equal(
        requests.at(-1).apiKey,
        apiKey && apiFormat === "anthropic-messages" ? apiKey : undefined,
      );
    }
  assert.equal(requests.length, 6);
  send("shutdown", "shutdown");
  child.stdin.end();
  await waitFor(() => child.exitCode !== null);
  assert.equal(child.exitCode, 0);
  assert.equal(stderr, "");
  const report = {
    label,
    target,
    node: metadata.version,
    sha: process.env.LOMI_PACKAGE_SHA ?? process.env.GITHUB_SHA ?? "local",
    qualificationSha: process.env.GITHUB_SHA ?? "local",
    checks: [
      "resource integrity",
      "native binary load",
      "bundled Node without PATH",
      "production runtime handshake",
      "three Custom API formats with and without a key",
      "clean runtime shutdown",
    ],
    passed: true,
  };
  await writeFile(
    `installer-results/${label}-runtime.json`,
    JSON.stringify(report, null, 2) + "\n",
  );
  console.log(JSON.stringify(report));
} finally {
  if (child.exitCode === null) child.kill();
  lines.close();
  server.closeAllConnections();
  server.close();
}
