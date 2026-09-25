import assert from "node:assert/strict";
import { test } from "node:test";
import { mcpResultToModelOutput, normalizeInputSchema } from "../src/mcp.ts";
import { frame, generation, MAX_TOOL_FRAME_DATA } from "../src/protocol.ts";
import { Snapshot } from "../src/snapshot.ts";

test("normalizes catalog JSON Schema 2020-12 references for SDK providers", () => {
  const schema = normalizeInputSchema({
    $schema: "https://json-schema.org/draft/2020-12/schema",
    type: "object",
    properties: {
      query: { $ref: "#/$defs/query" },
    },
    $defs: {
      query: {
        type: "object",
        properties: { text: { type: "string" } },
        required: ["text"],
      },
    },
  }) as Record<string, any>;

  assert.equal(schema.$schema, undefined);
  assert.equal(schema.properties.query.$ref, "#/definitions/query");
  assert.deepEqual(schema.definitions.query.required, ["text"]);
  assert.equal(schema.$defs, undefined);
});

test("maps MCP text, image, embedded resource, structured output, and errors", () => {
  const output = mcpResultToModelOutput({
    content: [
      { type: "text", text: "A caption" },
      { type: "image", mimeType: "image/png", data: "aGVsbG8=" },
      {
        type: "resource",
        resource: {
          uri: "file:///fixture.txt",
          mimeType: "text/plain",
          text: "A resource",
        },
      },
      {
        type: "resource",
        resource: {
          uri: "file:///fixture.png",
          mimeType: "image/png",
          blob: "aGVsbG8=",
        },
      },
    ],
    structuredContent: { answer: 42 },
    isError: false,
  }) as any;

  assert.equal(output.type, "content");
  assert.deepEqual(output.value, [
    { type: "text", text: "A caption" },
    {
      type: "file",
      mediaType: "image/png",
      data: { type: "data", data: "aGVsbG8=" },
    },
    { type: "text", text: "A resource" },
    {
      type: "file",
      mediaType: "image/png",
      data: { type: "data", data: "aGVsbG8=" },
      filename: "file:///fixture.png",
    },
    { type: "text", text: 'Structured content: {"answer":42}' },
  ]);

  assert.deepEqual(
    mcpResultToModelOutput({
      content: [{ type: "text", text: "Access denied" }],
      structuredContent: { code: "denied" },
      isError: true,
    }),
    {
      type: "error-text",
      value: 'Access denied\n{"code":"denied"}',
    },
  );
  assert.deepEqual(mcpResultToModelOutput({ isError: true }), {
    type: "error-text",
    value: "The MCP tool returned an error.",
  });
});

test("rejects MCP configuration outside generate and malformed tool-result frames", () => {
  const base = {
    provider: "openai",
    apiKey: "fixture-key",
    model: "gpt-4.1",
    assistantId: "assistant",
    messages: [
      { id: "user", role: "user", parts: [{ type: "text", text: "Hi" }] },
    ],
    mcp: { instructions: "Use tools", tools: [] },
  };
  assert.equal(
    generation.safeParse({ ...base, operation: "test-connection" }).success,
    false,
  );
  assert.equal(
    frame.safeParse({
      protocolVersion: 1,
      requestId: "request",
      type: "tool-result-append",
      payload: {
        toolCallId: "call",
        data: "*".repeat(MAX_TOOL_FRAME_DATA + 1),
      },
    }).success,
    false,
  );
});

test("snapshots preserve dynamic tool metadata, errors, and step-scoped IDs", () => {
  const snapshot = new Snapshot("assistant");
  snapshot.accept({ type: "start-step" });
  snapshot.accept({
    type: "text-start",
    id: snapshot.namespace("block"),
    providerMetadata: { google: { thoughtSignature: "text-signature" } },
  });
  snapshot.accept({
    type: "text-end",
    id: snapshot.namespace("block"),
  });
  snapshot.accept({
    type: "tool-input-available",
    dynamic: true,
    toolName: "lomi_workspace_list",
    toolCallId: "fixture-call",
    input: {},
    providerMetadata: { google: { thoughtSignature: "call-signature" } },
  });
  snapshot.accept({
    type: "tool-output-available",
    dynamic: true,
    toolCallId: "fixture-call",
    output: { ok: true },
    providerMetadata: { anthropic: { signature: "result-signature" } },
  });
  snapshot.accept({ type: "start-step" });
  snapshot.accept({
    type: "text-start",
    id: snapshot.namespace("block"),
  });

  const message = snapshot.value().message as any;
  assert.deepEqual(message.parts[0], { type: "step-start" });
  assert.equal(
    message.parts[1].providerMetadata.google.thoughtSignature,
    "text-signature",
  );
  assert.equal(
    message.parts[2].callProviderMetadata.google.thoughtSignature,
    "call-signature",
  );
  assert.equal(
    message.parts[2].resultProviderMetadata.anthropic.signature,
    "result-signature",
  );
  assert.deepEqual(message.parts[3], { type: "step-start" });
  assert.notEqual(snapshot.namespace("block"), "1:block");

  const errorSnapshot = new Snapshot("assistant-error");
  errorSnapshot.accept({
    type: "tool-input-error",
    dynamic: true,
    toolName: "lomi_workspace_list",
    toolCallId: "bad-call",
    input: "[]",
    errorText: "Input must be an object.",
    providerMetadata: {
      google: { thoughtSignature: "invalid-call-signature" },
    },
  });
  assert.equal((errorSnapshot.message.parts[0] as any).state, "output-error");
  assert.equal(
    (errorSnapshot.message.parts[0] as any).callProviderMetadata.google
      .thoughtSignature,
    "invalid-call-signature",
  );
  assert.throws(
    () =>
      errorSnapshot.accept({
        type: "tool-input-available",
        dynamic: true,
        toolName: "lomi_workspace_list",
        toolCallId: "bad-call",
        input: {},
      }),
    /protocol/,
  );
});
