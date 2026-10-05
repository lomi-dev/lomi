import assert from "node:assert/strict";
import { test } from "node:test";
import { catalog } from "../src/catalog.ts";
import { generate } from "../src/generate.ts";
import { modelFor } from "../src/providers.ts";
import { generation, type Event } from "../src/protocol.ts";
import {
  providerIds,
  providerPresets,
} from "../../../src/chat/provider-presets.ts";

for (const provider of ["xai", "openrouter", "deepseek", "nvidia"] as const) {
  test(`${provider} streams text and reasoning through its fixed endpoint`, async (t) => {
    const input = generation.parse({
      provider,
      apiKey: "fixture-key",
      model: providerPresets[provider].models[0],
      assistantId: "assistant",
      messages: [
        {
          id: "user",
          role: "user",
          parts: [{ type: "text", text: "Hello 日本語" }],
        },
      ],
    });
    const requests: { url: string; init?: RequestInit }[] = [];
    t.mock.method(
      globalThis,
      "fetch",
      async (url: string, init?: RequestInit) => {
        requests.push({ url: String(url), init });
        const chunks = [
          {
            delta: { role: "assistant", reasoning_content: "Thinking" },
            finish_reason: null,
          },
          { delta: { content: "Hello 日本語" }, finish_reason: null },
          { delta: {}, finish_reason: "stop" },
        ]
          .map(
            (choice) =>
              `data: ${JSON.stringify({ id: "response", object: "chat.completion.chunk", model: input.model, created: 1, choices: [{ index: 0, ...choice }] })}\n\n`,
          )
          .join("");
        return new Response(chunks + "data: [DONE]\n\n", {
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
    );
    assert.equal(requests.length, 1);
    assert.equal(
      requests[0].url,
      `${providerPresets[provider].baseURL}/chat/completions`,
    );
    assert.equal(
      new Headers(requests[0].init?.headers).get("authorization"),
      "Bearer fixture-key",
    );
    const body = JSON.parse(String(requests[0].init?.body));
    assert.equal(body.model, input.model);
    assert.equal(body.stream, true);
    assert.equal(body.max_tokens, 4096);
    assert.equal(body.tools, undefined);
    assert.equal(events.at(-1)?.type, "completed");
    assert.ok(
      events.some(
        (event) =>
          event.type === "chunk" &&
          (event.payload as any).type === "reasoning-delta",
      ),
    );
    assert.ok(
      events.some(
        (event) =>
          event.type === "chunk" &&
          (event.payload as any).delta === "Hello 日本語",
      ),
    );
  });
}

for (const provider of providerIds.filter((id) => id !== "custom")) {
  test(`${provider} fetches its catalog using the correct authentication`, async (t) => {
    const input = generation.parse({
      provider,
      apiKey: "fixture-key",
      model: "catalog",
      assistantId: "assistant",
      messages: [
        { id: "user", role: "user", parts: [{ type: "text", text: "unused" }] },
      ],
    });
    t.mock.method(
      globalThis,
      "fetch",
      async (url: string, init: RequestInit) => {
        assert.equal(
          new URL(url).origin,
          new URL(providerPresets[provider].baseURL).origin,
        );
        assert.equal(
          new URL(url).pathname,
          new URL(`${providerPresets[provider].baseURL}/models`).pathname,
        );
        const headers = new Headers(init.headers);
        assert.equal(
          headers.get(
            provider === "anthropic"
              ? "x-api-key"
              : provider === "google"
                ? "x-goog-api-key"
                : "authorization",
          ),
          ["anthropic", "google"].includes(provider)
            ? "fixture-key"
            : "Bearer fixture-key",
        );
        assert.equal(init.redirect, "error");
        return Response.json(
          provider === "google"
            ? {
                models: [
                  {
                    name: "models/chat-model",
                    supportedGenerationMethods: ["generateContent"],
                  },
                  {
                    name: "models/embed-model",
                    supportedGenerationMethods: ["embedContent"],
                  },
                ],
              }
            : {
                data: [
                  {
                    id: "chat-model",
                    architecture: { output_modalities: ["text"] },
                  },
                  { id: "invalid model" },
                  ...(provider === "openrouter"
                    ? [
                        {
                          id: "image-model",
                          architecture: { output_modalities: ["image"] },
                        },
                      ]
                    : []),
                ],
              },
        );
      },
    );
    const events: Event[] = [];
    await catalog(input, "catalog", new AbortController(), async (event) => {
      events.push(event);
    });
    assert.deepEqual((events[0].payload as any).models, ["chat-model"]);
    assert.equal(events.at(-1)?.type, "completed");
  });
}

test("compatible provider failures are sanitized and never retried", async (t) => {
  let calls = 0;
  t.mock.method(globalThis, "fetch", async () => {
    calls++;
    return Response.json(
      {
        error: {
          message: "PRIVATE_PROVIDER_ERROR",
          type: "authentication_error",
        },
      },
      { status: 401 },
    );
  });
  const input = generation.parse({
    provider: "deepseek",
    apiKey: "PRIVATE_KEY",
    model: "deepseek-flash",
    assistantId: "assistant",
    messages: [
      {
        id: "user",
        role: "user",
        parts: [{ type: "text", text: "PRIVATE_MESSAGE" }],
      },
    ],
  });
  const events: Event[] = [];
  await generate(
    input,
    "request",
    new AbortController(),
    async (event) => {
      events.push(event);
    },
    modelFor(input),
  );
  assert.equal(calls, 1);
  assert.equal(events.at(-1)?.type, "failed");
  assert.equal((events.at(-1)?.payload as any).code, "auth");
  assert.ok(!JSON.stringify(events).includes("PRIVATE_"));
  assert.equal(
    generation.safeParse({ ...input, provider: "unknown" }).success,
    false,
  );
  assert.equal(
    generation.safeParse({ ...input, baseURL: "https://untrusted.invalid" })
      .success,
    false,
  );
});

const customInput = {
  provider: "custom",
  apiKey: "",
  baseUrl: "http://localhost:1234/v1",
  apiFormat: "chat-completions",
  model: "local/model",
  assistantId: "assistant",
  messages: [
    { id: "user", role: "user", parts: [{ type: "text", text: "Hello" }] },
  ],
};

test("custom endpoints are validated without changing preset destinations", () => {
  for (const baseUrl of [
    "https://inference.example/api/v1",
    "http://localhost:1234/v1",
    "http://127.0.0.1:11434/v1",
    "http://[::1]:1234/v1",
  ])
    assert.ok(
      generation.safeParse({ ...customInput, baseUrl }).success,
      baseUrl,
    );
  for (const baseUrl of [
    "",
    "http://remote.example/v1",
    "file:///tmp/api",
    "https://key:secret@example.com",
    "https://example.com/v1?key=secret",
    "https://example.com/#hash",
    "https://example.com/\npath",
  ])
    assert.equal(
      generation.safeParse({ ...customInput, baseUrl }).success,
      false,
      baseUrl,
    );
  for (const override of [
    { baseUrl: undefined },
    { apiFormat: undefined },
    { apiFormat: "unknown" },
    { apiKey: "key\nheader" },
    { provider: "openai", apiKey: "key" },
  ])
    assert.equal(
      generation.safeParse({ ...customInput, ...override }).success,
      false,
    );
});

for (const apiFormat of [
  "chat-completions",
  "responses",
  "anthropic-messages",
] as const) {
  test(`custom ${apiFormat} lists models with optional authentication`, async (t) => {
    for (const apiKey of ["", "fixture-key"]) {
      const input = generation.parse({
        ...customInput,
        operation: "list-models",
        apiFormat,
        apiKey,
      });
      t.mock.method(
        globalThis,
        "fetch",
        async (url: string, init: RequestInit) => {
          assert.equal(
            String(url),
            `http://localhost:1234/v1/models${apiFormat === "anthropic-messages" ? "?limit=1000" : ""}`,
          );
          const headers = new Headers(init.headers);
          assert.equal(
            headers.get(
              apiFormat === "anthropic-messages"
                ? "x-api-key"
                : "authorization",
            ),
            apiKey
              ? apiFormat === "anthropic-messages"
                ? apiKey
                : `Bearer ${apiKey}`
              : null,
          );
          return Response.json({
            data: [{ id: "local/model" }, { id: "models/custom-model" }],
          });
        },
      );
      const events: Event[] = [];
      await catalog(input, "catalog", new AbortController(), async (event) => {
        events.push(event);
      });
      assert.equal(events.at(-1)?.type, "completed");
      assert.deepEqual((events[0].payload as any).models, [
        "local/model",
        "models/custom-model",
      ]);
      t.mock.restoreAll();
    }
  });
}
