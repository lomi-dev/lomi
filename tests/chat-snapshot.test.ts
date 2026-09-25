import assert from "node:assert/strict";
import { test } from "node:test";
import { snapshotToChunks } from "../src/chat/chat-snapshot.ts";

test("snapshot replay follows part order and restores dynamic tool results", () => {
  const snapshot = {
    message: {
      id: "assistant-1",
      role: "assistant",
      parts: [
        { type: "text", text: "Before." },
        { type: "step-start" },
        {
          type: "dynamic-tool",
          toolName: "lomi_workspace_list",
          toolCallId: "call-1",
          state: "output-available",
          input: { workspaceId: "workspace-1" },
          output: { structuredContent: { items: ["one"] } },
          callProviderMetadata: { openai: { call: "kept" } },
          resultProviderMetadata: { openai: { result: "kept" } },
        },
        { type: "reasoning", id: "think", text: "After tool." },
      ],
    },
    blocks: {
      before: { index: 0, type: "text", open: false },
      reasoning: { index: 3, type: "reasoning", open: false },
    },
  } as any;

  const chunks = snapshotToChunks(snapshot);
  assert.deepEqual(
    chunks.map((chunk) => chunk.type),
    [
      "start",
      "text-start",
      "text-delta",
      "text-end",
      "start-step",
      "tool-input-available",
      "tool-output-available",
      "reasoning-start",
      "reasoning-delta",
      "reasoning-end",
    ],
  );
  assert.equal(chunks[1].type === "text-start" && chunks[1].id, "before");
  assert.equal(
    chunks[5].type === "tool-input-available" && chunks[5].dynamic,
    true,
  );
  assert.deepEqual(
    chunks[5].type === "tool-input-available" && chunks[5].providerMetadata,
    { openai: { call: "kept" } },
  );
  assert.deepEqual(
    chunks[6].type === "tool-output-available" && chunks[6].providerMetadata,
    { openai: { result: "kept" } },
  );
});

test("snapshot replay restores tool errors and keeps open text blocks open", () => {
  const chunks = snapshotToChunks({
    message: {
      id: "assistant-2",
      role: "assistant",
      parts: [
        {
          type: "dynamic-tool",
          toolName: "lomi_read",
          toolCallId: "call-2",
          state: "output-error",
          input: {},
          errorText: "Denied",
        },
        { type: "text", text: "Partial", state: "streaming" },
      ],
    },
    blocks: { live: { index: 1, type: "text", open: true } },
  } as any);
  assert.deepEqual(
    chunks.map((chunk) => chunk.type),
    [
      "start",
      "tool-input-available",
      "tool-output-error",
      "text-start",
      "text-delta",
    ],
  );
  assert.equal(
    chunks[2].type === "tool-output-error" && chunks[2].errorText,
    "Denied",
  );
  assert.equal(chunks[4].type === "text-delta" && chunks[4].delta, "Partial");
});
