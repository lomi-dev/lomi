import assert from "node:assert/strict";
import { test } from "node:test";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { VERSION, type Event } from "../src/protocol.ts";

function runtime() {
  const child = spawn(
    process.execPath,
    [
      "--experimental-strip-types",
      fileURLToPath(new URL("../src/fixture.ts", import.meta.url)),
    ],
    { stdio: "pipe" },
  );
  let stderr = "";
  let exited:
    { code: number | null; signal: NodeJS.Signals | null } | undefined;
  child.stderr.on("data", (data) => {
    stderr += data;
  });
  child.on("exit", (code, signal) => {
    exited = { code, signal };
  });
  let pending = "";
  const events: Event[] = [];
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (data) => {
    pending += data;
    for (;;) {
      const end = pending.indexOf("\n");
      if (end < 0) break;
      events.push(JSON.parse(pending.slice(0, end)));
      pending = pending.slice(end + 1);
    }
  });
  const send = (type: string, requestId = "request-1", payload?: unknown) =>
    child.stdin.write(
      JSON.stringify({ protocolVersion: VERSION, requestId, type, payload }) +
        "\n",
    );
  const waitFor = async (predicate: () => boolean) => {
    const end = Date.now() + 10000;
    while (!predicate()) {
      assert.ok(
        Date.now() < end,
        "Fixture deadline exceeded: " +
          JSON.stringify({ exited, stderr, events }),
      );
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
  };
  const start = (
    model = "fixture",
    requestId = "request-1",
    extra: Record<string, unknown> = {},
  ) => {
    const data = Buffer.from(
      JSON.stringify({
        provider: "openai",
        apiKey: "FIXTURE_PRIVATE_KEY",
        model,
        assistantId: "assistant-1",
        messages: [
          {
            id: "user-1",
            role: "user",
            parts: [{ type: "text", text: "PRIVATE_QUESTION 日本語" }],
          },
        ],
        ...extra,
      }),
    );
    send("begin", requestId);
    // Deliberately split the JSON's UTF-8 code points between transfer frames.
    for (let i = 0; i < data.length; i += 7)
      send("append", requestId, {
        data: data.subarray(i, i + 7).toString("base64"),
      });
    send("generate", requestId);
  };
  return {
    child,
    events,
    send,
    start,
    waitFor,
    stderr: () => stderr,
    exited: () => exited,
  };
}

function mcpConfig() {
  return {
    instructions: "Use the workspace catalog when needed.",
    tools: [
      {
        name: "lomi_workspace_list",
        description: "List approved workspaces.",
        inputSchema: {
          type: "object",
          properties: {},
          additionalProperties: false,
        },
      },
    ],
  };
}

function sendToolResult(
  send: (type: string, requestId?: string, payload?: unknown) => void,
  requestId: string,
  toolCallId: string,
  result: unknown,
) {
  const data = Buffer.from(JSON.stringify(result));
  send("tool-result-begin", requestId, { toolCallId });
  for (let offset = 0; offset < data.length; offset += 32 * 1024) {
    send("tool-result-append", requestId, {
      toolCallId,
      data: data.subarray(offset, offset + 32 * 1024).toString("base64"),
    });
  }
  send("tool-result-end", requestId, { toolCallId });
}

function fixtureWorkspaceResult(name = "Fixture Workspace") {
  return {
    content: [{ type: "text", text: "Found the approved workspace." }],
    structuredContent: {
      status: "ok",
      controlApiVersion: "1.0",
      data: {
        kind: "workspaces",
        items: [{ id: "workspace-1", name }],
        nextCursor: null,
        domainRevision: 1,
      },
    },
    isError: false,
  };
}

test("SDK stream preserves reserved IDs and UTF-8; stdout/stderr omit input and errors", async () => {
  const r = runtime();
  try {
    r.start("fixture-error");
    await r.waitFor(() => r.events.some((event) => event.type === "failed"));
    assert.equal(
      r.events.filter((event) =>
        ["completed", "failed", "cancelled"].includes(event.type),
      ).length,
      1,
    );
    assert.deepEqual(
      r.events.map((e) => e.sequence),
      r.events.map((_, i) => i + 1),
    );
    const snapshot = r.events.at(-2)?.payload as {
      message: {
        id: string;
        parts: { type: string; text?: string }[];
      };
    };
    assert.equal(snapshot.message.id, "assistant-1");
    assert.equal(
      snapshot.message.parts.find((part) => part.type === "text")?.text,
      "Zażółć 日本語 👩🏽‍💻\n".repeat(40),
    );
    assert.ok(!JSON.stringify(r.events).includes("PRIVATE"));
    assert.ok(!r.stderr().includes("PRIVATE"));
    assert.equal(r.stderr(), "");
  } finally {
    r.child.kill();
  }
});

test("Stop aborts a started request with one terminal event and a final snapshot", async () => {
  const r = runtime();
  try {
    r.start();
    await r.waitFor(() =>
      r.events.some(
        (e) =>
          e.type === "chunk" &&
          (e.payload as { type: string }).type === "text-delta",
      ),
    );
    r.send("cancel");
    await r.waitFor(() => r.events.some((e) => e.type === "cancelled"));
    assert.equal(r.events.at(-2)?.type, "message-snapshot");
    const length = r.events.length;
    r.send("cancel");
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert.equal(r.events.length, length);
  } finally {
    r.child.kill();
  }
});

test("cancel before generate prevents model dispatch; malformed input fails closed", async () => {
  const r = runtime();
  try {
    r.send("cancel");
    r.start();
    r.send("hello", "handshake");
    await r.waitFor(() => r.events.some((e) => e.type === "ready"));
    assert.deepEqual(
      r.events.filter((e) => e.requestId === "request-1").map((e) => e.type),
      ["cancelled"],
    );
    r.child.stdin.write("x".repeat(100000));
    const [code] = await once(r.child, "exit");
    assert.equal(code, 1);
    assert.equal(r.stderr(), "");
  } finally {
    r.child.kill();
  }
});

test("cancelling a context transfer leaves another generation running", async () => {
  const r = runtime();
  try {
    r.start("fixture-fast", "other");
    r.send("begin", "upload");
    r.send("append", "upload", {
      data: Buffer.from("partial").toString("base64"),
    });
    r.send("cancel", "upload");
    await r.waitFor(() =>
      r.events.some((e) => e.requestId === "upload" && e.type === "cancelled"),
    );
    await r.waitFor(
      () =>
        r.events.filter((e) => e.requestId === "other" && e.type === "chunk")
          .length > 10,
    );
    assert.equal(r.events.filter((e) => e.requestId === "upload").length, 1);
    r.send("cancel", "other");
    await r.waitFor(() =>
      r.events.some((e) => e.requestId === "other" && e.type === "cancelled"),
    );
  } finally {
    r.child.kill();
  }
});

test("a local response limit aborts generation and reports the limit once", async () => {
  const r = runtime();
  try {
    r.start("fixture-large");
    await r.waitFor(() => r.events.some((e) => e.type === "failed"));
    const terminal = r.events.filter((e) =>
      ["completed", "failed", "cancelled"].includes(e.type),
    );
    assert.equal(terminal.length, 1);
    assert.equal(
      (terminal[0].payload as { code: string }).code,
      "response-limit",
    );
    const responseSnapshot = r.events.at(-2)?.payload as {
      message: { parts: { type: string; text?: string }[] };
    };
    assert.ok(
      Buffer.byteLength(JSON.stringify(responseSnapshot)) <= 16 * 1024 * 1024,
    );
    assert.ok(
      Buffer.byteLength(
        responseSnapshot.message.parts
          .filter((part) => part.type === "text")
          .map((part) => part.text ?? "")
          .join(""),
      ) <=
        2 * 1024 * 1024,
    );
    const count = r.events.length;
    await new Promise((resolve) => setTimeout(resolve, 100));
    assert.equal(r.events.length, count);
    assert.equal(r.stderr(), "");
  } finally {
    r.child.kill();
  }
});

test("MCP calls wait for native results, continue the model, and snapshot both steps", async () => {
  const r = runtime();
  const requestId = "mcp-request";
  try {
    r.start("fixture-mcp", requestId, { mcp: mcpConfig() });
    await r.waitFor(() =>
      r.events.some(
        (event) => event.requestId === requestId && event.type === "tool-call",
      ),
    );
    const call = r.events.find(
      (event) => event.requestId === requestId && event.type === "tool-call",
    )!.payload as { toolCallId: string; toolName: string; input: unknown };
    assert.deepEqual(call, {
      toolCallId: "fixture-tool-1",
      toolName: "lomi_workspace_list",
      input: {},
    });
    sendToolResult(
      r.send,
      requestId,
      call.toolCallId,
      fixtureWorkspaceResult(),
    );
    await r.waitFor(() =>
      r.events.some(
        (event) => event.requestId === requestId && event.type === "completed",
      ),
    );
    const snapshots = r.events.filter(
      (event) =>
        event.requestId === requestId && event.type === "message-snapshot",
    );
    const message = (snapshots.at(-1)!.payload as { message: { parts: any[] } })
      .message;
    assert.ok(message.parts.some((part) => part.type === "step-start"));
    const tool = message.parts.find((part) => part.type === "dynamic-tool");
    assert.equal(tool?.toolCallId, "fixture-tool-1");
    assert.equal(tool?.toolName, "lomi_workspace_list");
    assert.equal(tool?.state, "output-available");
    assert.deepEqual(tool?.input, {});
    assert.deepEqual(tool?.callProviderMetadata, {
      google: { thoughtSignature: "fixture-tool-signature" },
    });
    assert.ok(
      message.parts.some(
        (part) =>
          part.type === "text" &&
          part.text === "Workspace result: Fixture Workspace" &&
          part.providerMetadata?.google?.thoughtSignature ===
            "fixture-text-signature",
      ),
    );
    const textBlockIds = r.events
      .filter(
        (event) =>
          event.requestId === requestId &&
          event.type === "chunk" &&
          (event.payload as { type?: string }).type === "text-start",
      )
      .map((event) => (event.payload as { id: string }).id);
    assert.equal(textBlockIds.length, 2);
    assert.notEqual(textBlockIds[0], textBlockIds[1]);
    assert.ok(textBlockIds.every((id) => /^\d+:.+/.test(id)));
    assert.deepEqual(
      r.events
        .filter((event) => event.requestId === requestId)
        .map((event) => event.sequence),
      r.events
        .filter((event) => event.requestId === requestId)
        .map((_, index) => index + 1),
    );
  } finally {
    r.child.kill();
  }
});

test("cancelling queued parallel MCP calls settles them and leaves runtime usable", async () => {
  const r = runtime();
  try {
    r.start("fixture-mcp-parallel", "parallel", { mcp: mcpConfig() });
    await r.waitFor(() =>
      r.events.some(
        (event) => event.requestId === "parallel" && event.type === "tool-call",
      ),
    );
    assert.equal(
      r.events.filter(
        (event) => event.requestId === "parallel" && event.type === "tool-call",
      ).length,
      1,
    );
    r.send("cancel", "parallel");
    await r.waitFor(() =>
      r.events.some(
        (event) => event.requestId === "parallel" && event.type === "cancelled",
      ),
    );
    r.start("fixture", "after-cancel");
    await r.waitFor(() =>
      r.events.some(
        (event) =>
          event.requestId === "after-cancel" && event.type === "completed",
      ),
    );
    assert.equal(r.exited(), undefined);
  } finally {
    r.child.kill();
  }
});

test("parallel MCP calls emit serial requests and finish after both native results", async () => {
  const r = runtime();
  const requestId = "parallel-results";
  try {
    r.start("fixture-mcp-parallel", requestId, { mcp: mcpConfig() });
    await r.waitFor(
      () =>
        r.events.filter(
          (event) =>
            event.requestId === requestId && event.type === "tool-call",
        ).length === 1,
    );
    const first = r.events.find(
      (event) => event.requestId === requestId && event.type === "tool-call",
    )!.payload as { toolCallId: string };
    assert.equal(first.toolCallId, "fixture-tool-1");
    sendToolResult(
      r.send,
      requestId,
      first.toolCallId,
      fixtureWorkspaceResult(),
    );
    await r.waitFor(
      () =>
        r.events.filter(
          (event) =>
            event.requestId === requestId && event.type === "tool-call",
        ).length === 2,
    );
    const calls = r.events
      .filter(
        (event) => event.requestId === requestId && event.type === "tool-call",
      )
      .map((event) => (event.payload as { toolCallId: string }).toolCallId);
    assert.deepEqual(calls, ["fixture-tool-1", "fixture-tool-2"]);
    const second = r.events.find(
      (event) =>
        event.requestId === requestId &&
        event.type === "tool-call" &&
        (event.payload as { toolCallId: string }).toolCallId ===
          "fixture-tool-2",
    )!.payload as { toolCallId: string };
    sendToolResult(
      r.send,
      requestId,
      second.toolCallId,
      fixtureWorkspaceResult("Second Workspace"),
    );
    await r.waitFor(() =>
      r.events.some(
        (event) => event.requestId === requestId && event.type === "completed",
      ),
    );
    const snapshot = r.events
      .filter(
        (event) =>
          event.requestId === requestId && event.type === "message-snapshot",
      )
      .at(-1)!.payload as { message: { parts: any[] } };
    assert.equal(
      snapshot.message.parts.filter((part) => part.type === "dynamic-tool")
        .length,
      2,
    );
    assert.ok(
      snapshot.message.parts.some(
        (part) =>
          part.type === "text" &&
          part.text === "Workspace result: Fixture Workspace",
      ),
    );
  } finally {
    r.child.kill();
  }
});

test("duplicate result framing fails closed without leaking details", async () => {
  const r = runtime();
  try {
    r.start("fixture-mcp", "duplicate-frame", { mcp: mcpConfig() });
    await r.waitFor(() =>
      r.events.some(
        (event) =>
          event.requestId === "duplicate-frame" && event.type === "tool-call",
      ),
    );
    r.send("tool-result-begin", "duplicate-frame", {
      toolCallId: "fixture-tool-1",
    });
    r.send("tool-result-begin", "duplicate-frame", {
      toolCallId: "fixture-tool-1",
    });
    const [code] = await once(r.child, "exit");
    assert.equal(code, 1);
    assert.equal(r.stderr(), "");
  } finally {
    r.child.kill();
  }
});

test("cancel after a completed MCP result keeps its output in the final snapshot", async () => {
  const r = runtime();
  const requestId = "cancel-after-tool";
  try {
    r.start("fixture-mcp-cancel", requestId, { mcp: mcpConfig() });
    await r.waitFor(() =>
      r.events.some(
        (event) => event.requestId === requestId && event.type === "tool-call",
      ),
    );
    sendToolResult(
      r.send,
      requestId,
      "fixture-tool-1",
      fixtureWorkspaceResult(),
    );
    await r.waitFor(() =>
      r.events.some(
        (event) =>
          event.requestId === requestId &&
          event.type === "chunk" &&
          (event.payload as { delta?: string }).delta === "post-tool:",
      ),
    );
    r.send("cancel", requestId);
    await r.waitFor(() =>
      r.events.some(
        (event) => event.requestId === requestId && event.type === "cancelled",
      ),
    );
    const snapshot = r.events
      .filter(
        (event) =>
          event.requestId === requestId && event.type === "message-snapshot",
      )
      .at(-1)!.payload as { message: { parts: any[] } };
    const tool = snapshot.message.parts.find(
      (part) => part.type === "dynamic-tool",
    );
    assert.equal(tool?.state, "output-available");
    assert.equal(
      tool?.output.structuredContent.data.items[0].name,
      "Fixture Workspace",
    );
    assert.equal(r.exited(), undefined);
  } finally {
    r.child.kill();
  }
});

test("completed and pending dynamic-tool history is replayed without replaying pending calls", async () => {
  const r = runtime();
  try {
    r.start("fixture-history-pending", "history", {
      assistantId: "assistant-next",
      mcp: mcpConfig(),
      messages: [
        {
          id: "user-old",
          role: "user",
          parts: [{ type: "text", text: "Use the catalog." }],
        },
        {
          id: "assistant-old",
          role: "assistant",
          parts: [
            { type: "step-start" },
            {
              type: "dynamic-tool",
              toolName: "lomi_workspace_list",
              toolCallId: "completed-call",
              state: "output-available",
              input: {},
              output: fixtureWorkspaceResult(),
              callProviderMetadata: {
                google: { thoughtSignature: "fixture-tool-signature" },
              },
            },
            {
              type: "dynamic-tool",
              toolName: "lomi_workspace_list",
              toolCallId: "pending-call",
              state: "input-available",
              input: {},
            },
          ],
        },
        {
          id: "user-new",
          role: "user",
          parts: [{ type: "text", text: "Continue." }],
        },
      ],
    });
    await r.waitFor(() =>
      r.events.some(
        (event) => event.requestId === "history" && event.type === "completed",
      ),
    );
    const response = r.events
      .filter(
        (event) =>
          event.requestId === "history" && event.type === "message-snapshot",
      )
      .at(-1)!.payload as { message: { parts: any[] } };
    assert.ok(
      response.message.parts.some(
        (part) =>
          part.type === "text" &&
          part.text === "completed-call-replayed pending-call-ignored",
      ),
      JSON.stringify(response.message.parts),
    );
    assert.equal(
      r.events.filter(
        (event) => event.requestId === "history" && event.type === "tool-call",
      ).length,
      0,
    );
  } finally {
    r.child.kill();
  }
});

test("malformed input and unknown MCP tools become replayable errors", async () => {
  const r = runtime();
  try {
    for (const [requestId, model, callId] of [
      ["invalid-input", "fixture-mcp-invalid-input", "fixture-invalid-input"],
      ["unknown-tool", "fixture-mcp-unknown-tool", "fixture-unknown-tool"],
    ]) {
      r.start(model, requestId, { mcp: mcpConfig() });
      await r.waitFor(() =>
        r.events.some(
          (event) =>
            event.requestId === requestId &&
            ["completed", "failed"].includes(event.type),
        ),
      );
      const previous = r.events
        .filter(
          (event) =>
            event.requestId === requestId && event.type === "message-snapshot",
        )
        .at(-1)!.payload as { message: { id: string; parts: any[] } };
      const errored = previous.message.parts.find(
        (part) => part.type === "dynamic-tool",
      );
      assert.equal(errored?.toolCallId, callId);
      assert.equal(errored?.state, "output-error");

      const followup = `${requestId}-followup`;
      r.start("fixture-history", followup, {
        assistantId: `${followup}-assistant`,
        mcp: mcpConfig(),
        messages: [
          {
            id: `${requestId}-user`,
            role: "user",
            parts: [{ type: "text", text: "Use the tool." }],
          },
          previous.message,
          {
            id: `${requestId}-next-user`,
            role: "user",
            parts: [{ type: "text", text: "Try again." }],
          },
        ],
      });
      await r.waitFor(() =>
        r.events.some(
          (event) => event.requestId === followup && event.type === "completed",
        ),
      );
      assert.equal(r.exited(), undefined);
    }
  } finally {
    r.child.kill();
  }
});
