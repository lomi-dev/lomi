import assert from "node:assert/strict";
import { test } from "node:test";
import { generate } from "../src/generate.ts";
import { modelFor } from "../src/providers.ts";
import { generation, type Generation, type Event } from "../src/protocol.ts";

type Provider = "openai" | "anthropic" | "google" | "deepseek";

function inputFor(provider: Provider): Generation {
  const model = {
    openai: "gpt-4.1",
    anthropic: "claude-opus-5",
    google: "gemini-2.5-pro",
    deepseek: "deepseek-flash",
  }[provider];
  return generation.parse({
    provider,
    apiKey: "fixture-key",
    model,
    assistantId: "assistant-next",
    messages: [
      {
        id: "user-old",
        role: "user",
        parts: [{ type: "text", text: "Call the workspace tool." }],
      },
      {
        id: "assistant-old",
        role: "assistant",
        parts: [
          { type: "step-start" },
          {
            type: "reasoning",
            text: "Previously preserved reasoning.",
            providerMetadata: {
              anthropic: { signature: "anthropic-reasoning-signature" },
              google: { thoughtSignature: "google-reasoning-signature" },
              deepseek: { preserved: true },
            },
          },
          {
            type: "dynamic-tool",
            toolName: "lomi_workspace_list",
            toolCallId: "prior-workspace-call",
            state: "output-available",
            input: { workspaceId: "workspace-old" },
            output: { status: "ok", name: "Prior workspace" },
            callProviderMetadata: {
              google: { thoughtSignature: "google-tool-signature" },
            },
            resultProviderMetadata: {
              anthropic: { signature: "anthropic-result-signature" },
            },
          },
        ],
      },
      {
        id: "user-new",
        role: "user",
        parts: [{ type: "text", text: "Now list the current workspace." }],
      },
    ],
    mcp: {
      instructions: "Use approved workspace tools.",
      tools: [
        {
          name: "lomi_workspace_list",
          description: "List approved workspaces.",
          inputSchema: {
            type: "object",
            properties: {
              workspaceId: {
                type: "string",
                description: "Optional workspace id.",
              },
            },
            required: ["workspaceId"],
            additionalProperties: false,
          },
        },
      ],
    },
  });
}

function sseData(value: unknown) {
  return `data: ${JSON.stringify(value)}\n\n`;
}

function responseStream(provider: Provider, model: string) {
  if (provider === "openai") {
    return [
      sseData({
        type: "response.output_item.added",
        output_index: 0,
        item: { type: "message", id: "message-1" },
      }),
      sseData({
        type: "response.output_text.delta",
        item_id: "message-1",
        output_index: 0,
        delta: "Done.",
      }),
      sseData({
        type: "response.output_item.done",
        output_index: 0,
        item: { type: "message", id: "message-1" },
      }),
      sseData({
        type: "response.completed",
        response: {
          usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
        },
      }),
      "data: [DONE]\n\n",
    ].join("");
  }

  if (provider === "anthropic") {
    const data = (type: string, value: unknown) =>
      `event: ${type}\ndata: ${JSON.stringify(value)}\n\n`;
    return [
      data("message_start", {
        type: "message_start",
        message: {
          id: "message-1",
          type: "message",
          role: "assistant",
          content: [],
          model,
          stop_reason: null,
          stop_sequence: null,
          usage: { input_tokens: 1, output_tokens: 0 },
        },
      }),
      data("content_block_start", {
        type: "content_block_start",
        index: 0,
        content_block: { type: "text", text: "" },
      }),
      data("content_block_delta", {
        type: "content_block_delta",
        index: 0,
        delta: { type: "text_delta", text: "Done." },
      }),
      data("content_block_stop", { type: "content_block_stop", index: 0 }),
      data("message_delta", {
        type: "message_delta",
        delta: { stop_reason: "end_turn", stop_sequence: null },
        usage: { output_tokens: 1 },
      }),
      data("message_stop", { type: "message_stop" }),
    ].join("");
  }

  if (provider === "google") {
    return sseData({
      responseId: "response-1",
      candidates: [
        {
          index: 0,
          content: { role: "model", parts: [{ text: "Done." }] },
          finishReason: "STOP",
        },
      ],
      usageMetadata: {
        promptTokenCount: 1,
        candidatesTokenCount: 1,
        totalTokenCount: 2,
      },
    });
  }

  return [
    sseData({
      id: "response-1",
      object: "chat.completion.chunk",
      model,
      created: 1,
      choices: [
        {
          index: 0,
          delta: { role: "assistant", content: "Done." },
          finish_reason: null,
        },
      ],
    }),
    sseData({
      id: "response-1",
      object: "chat.completion.chunk",
      model,
      created: 1,
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
    }),
    "data: [DONE]\n\n",
  ].join("");
}

function findValue(
  value: unknown,
  predicate: (value: Record<string, any>) => boolean,
): Record<string, any> | undefined {
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findValue(item, predicate);
      if (found) return found;
    }
    return undefined;
  }
  if (!value || typeof value !== "object") return undefined;
  const object = value as Record<string, any>;
  if (predicate(object)) return object;
  for (const child of Object.values(object)) {
    const found = findValue(child, predicate);
    if (found) return found;
  }
  return undefined;
}

for (const provider of ["openai", "anthropic", "google", "deepseek"] as const) {
  test(`${provider} adapter sends MCP tools and replays provider signatures`, async (t) => {
    const input = inputFor(provider);
    let requestBody: Record<string, any> | undefined;
    t.mock.method(
      globalThis,
      "fetch",
      async (_url: string, init: RequestInit) => {
        requestBody = JSON.parse(String(init.body));
        return new Response(responseStream(provider, input.model), {
          headers: { "content-type": "text/event-stream" },
        });
      },
    );

    const events: Event[] = [];
    await generate(
      input,
      "request",
      new AbortController(),
      async (event) => {
        events.push(event);
      },
      modelFor(input),
      async () => ({
        content: [],
        structuredContent: { ok: true },
        isError: false,
      }),
    );

    assert.equal(
      events.at(-1)?.type,
      "completed",
      JSON.stringify(events.at(-1)),
    );
    assert.ok(requestBody);
    assert.ok(Array.isArray(requestBody.tools), JSON.stringify(requestBody));
    const declaration = findValue(
      requestBody.tools,
      (value) => value.name === "lomi_workspace_list",
    );
    assert.ok(declaration, JSON.stringify(requestBody.tools));
    const schema =
      declaration.input_schema ??
      declaration.parameters ??
      declaration.parametersJsonSchema;
    assert.equal(schema?.type, "object");
    assert.equal(schema?.properties?.workspaceId?.type, "string");

    if (provider === "anthropic") {
      const thinking = findValue(
        requestBody.messages,
        (value) => value.type === "thinking",
      );
      assert.equal(thinking?.signature, "anthropic-reasoning-signature");
    } else if (provider === "google") {
      const call = findValue(
        requestBody.contents,
        (value) => value.functionCall?.name === "lomi_workspace_list",
      );
      assert.equal(call?.thoughtSignature, "google-tool-signature");
      assert.ok(
        requestBody.tools.some((tool: Record<string, any>) =>
          tool.functionDeclarations?.some(
            (item: Record<string, unknown>) =>
              item.name === "lomi_workspace_list",
          ),
        ),
      );
    } else if (provider === "deepseek") {
      assert.ok(
        requestBody.messages.some(
          (message: Record<string, unknown>) =>
            message.role === "assistant" &&
            message.reasoning_content === "Previously preserved reasoning.",
        ),
      );
    }
  });
}
