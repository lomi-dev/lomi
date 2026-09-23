import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";

const version = execFileSync("codex", ["--version"], {
  encoding: "utf8",
}).trim();
assert.equal(version, "codex-cli 0.156.1");
const directory = await mkdtemp(join(tmpdir(), "lomi-mcp-codex-"));
const control =
  process.argv[2] === "--control"
    ? JSON.parse(await readFile(process.argv[3], "utf8"))
    : null;
const binary =
  control?.helper.command ??
  resolve("src-tauri/target/debug/examples/protocol-probe");
const configured = JSON.parse(
  execFileSync("codex", ["mcp", "list", "--json"], { encoding: "utf8" }),
);
const disabled = configured.flatMap(({ name, transport }) => {
  assert.match(name, /^[A-Za-z0-9_-]+$/);
  const endpoint =
    transport.type === "streamable_http"
      ? 'url="http://127.0.0.1:1"'
      : `command=${JSON.stringify(binary)}`;
  return ["-c", `mcp_servers.${name}={${endpoint},enabled=false}`];
});
const child = spawn(
  "codex",
  [
    ...disabled,
    "-c",
    `mcp_servers.lomi_probe={command=${JSON.stringify(binary)},args=${JSON.stringify(control?.helper.args ?? [])},required=true}`,
    "-c",
    "plugins={}",
    "-c",
    "features.apps=false",
    "app-server",
  ],
  {
    cwd: directory,
    stdio: ["pipe", "pipe", "pipe"],
  },
);
const exited = new Promise((accept) => child.once("exit", accept));
const pending = new Map();
let counter = 0;
let stderr = "";
child.stderr.on("data", (bytes) => {
  if (stderr.length < 16_384) stderr += bytes;
});
child.stdin.on("error", () => {});
createInterface({ input: child.stdout }).on("line", (line) => {
  const message = JSON.parse(line);
  if (message.id !== undefined && pending.has(message.id)) {
    const resolve = pending.get(message.id);
    pending.delete(message.id);
    resolve(message);
  } else if (message.id !== undefined) {
    child.stdin.write(
      `${JSON.stringify({ id: message.id, error: { code: -32601, message: "Unexpected fixture request" } })}\n`,
    );
  }
});
async function call(method, params) {
  const id = ++counter;
  const response = new Promise((resolve) => pending.set(id, resolve));
  child.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
  let timeout;
  try {
    const message = await Promise.race([
      response,
      exited.then((code) => {
        throw Error(`Codex exited (${code}): ${stderr.slice(0, 4096)}`);
      }),
      new Promise((_, reject) => {
        timeout = setTimeout(
          () => reject(Error(`Timed out: ${method}`)),
          25_000,
        );
      }),
    ]);
    assert.equal(
      message.error,
      undefined,
      `${method}: ${JSON.stringify(message.error)}`,
    );
    return message.result;
  } finally {
    clearTimeout(timeout);
  }
}
try {
  await call("initialize", {
    clientInfo: { name: "lomi-qualification", version: "0.1.0" },
    capabilities: { experimentalApi: true },
  });
  child.stdin.write(`${JSON.stringify({ method: "initialized" })}\n`);
  const started = await call("thread/start", {
    cwd: directory,
    ephemeral: true,
    approvalPolicy: "never",
    sandbox: "read-only",
  });
  const threadId = started.thread.id;
  const inventory = await call("mcpServerStatus/list", {
    threadId,
    detail: "full",
  });
  const usable = inventory.data.filter(
    (entry) => Object.keys(entry.tools).length > 0,
  );
  assert.equal(usable.length, 1, "Only the isolated probe has callable tools");
  const server = usable[0];
  assert.equal(server.name, "lomi_probe");
  if (control) {
    console.error("Codex control: catalog ready");
    assert.ok(server.tools.lomi_status && server.tools.lomi_workspace_list);
    const invoke = (tool, args = {}) =>
      call("mcpServer/tool/call", {
        threadId,
        server: "lomi_probe",
        tool,
        arguments: args,
      });
    const unapproved = await invoke("lomi_workspace_list");
    assert.equal(unapproved.isError, true);
    console.error("Codex control: unapproved read denied");
    const ownStatus = await invoke("lomi_status");
    const pairingRequestId =
      ownStatus.structuredContent?.data?.pairingRequestId;
    assert.equal(
      typeof pairingRequestId,
      "string",
      "Status identifies this helper's pending request",
    );
    if (control.approvalReadyPath)
      await writeFile(
        control.approvalReadyPath,
        JSON.stringify({ pairingRequestId }),
      );
    let result;
    let previousCode;
    const deadline = Date.now() + 90_000;
    while (Date.now() < deadline) {
      result = await invoke("lomi_workspace_list");
      const code =
        result.structuredContent?.code ??
        result.structuredContent?.status ??
        JSON.stringify(Object.keys(result));
      if (code !== previousCode) {
        console.error("Codex control phase:", code);
        previousCode = code;
      }
      if (result.structuredContent?.status === "ok") break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    assert.equal(result.structuredContent?.status, "ok");
    assert.equal(result.structuredContent.data.items.length, 1);
    assert.equal(
      result.structuredContent.data.items[0].id,
      control.expectedWorkspaceId,
    );
    const selected = await invoke("lomi_connect", {
      workspaceId: control.expectedWorkspaceId,
    });
    assert.equal(selected.structuredContent.status, "ok");
    assert.ok(selected.structuredContent.data.retryEpoch);
    const denied = await invoke("lomi_connect", {
      workspaceId: "foreign-workspace",
    });
    assert.equal(denied.structuredContent.code, "TARGET_NOT_FOUND");
    const report = {
      version,
      tools: Object.keys(server.tools),
      nativeBroker: true,
      workspaceFiltered: true,
      foreignDenied: true,
      providerCalls: 0,
      privateConfigWrites: 0,
    };
    await writeFile(
      join(directory, "result.json"),
      JSON.stringify(report, null, 2),
    );
    console.log(JSON.stringify({ directory, ...report }, null, 2));
  } else {
    assert.ok(server.tools.probe_image && server.tools.probe_error);
    const image = await call("mcpServer/tool/call", {
      threadId,
      server: "lomi_probe",
      tool: "probe_image",
      arguments: {},
    });
    assert.ok(
      image.content.some((part) => part.type === "image"),
      JSON.stringify(image),
    );
    const failure = await call("mcpServer/tool/call", {
      threadId,
      server: "lomi_probe",
      tool: "probe_error",
      arguments: {},
    });
    assert.equal(failure.isError, true);
    const report = {
      version,
      tools: Object.keys(server.tools),
      image: true,
      typedError: true,
      providerCalls: 0,
      privateConfigWrites: 0,
    };
    await writeFile(
      join(directory, "result.json"),
      JSON.stringify(report, null, 2),
    );
    console.log(JSON.stringify({ directory, ...report }, null, 2));
  }
} finally {
  child.stdin.end();
  const timer = setTimeout(() => child.kill(), 3000);
  await exited;
  clearTimeout(timer);
  // Codex's ephemeral thread runs no turn and therefore invokes no model.
  await rm(join(directory, ".codex"), { recursive: true, force: true });
}
