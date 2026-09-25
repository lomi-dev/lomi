import { MockLanguageModelV4 } from "ai/test";
import { Writable } from "node:stream";
import { serve } from "./server.ts";

const usage = {
  inputTokens: { total: 1, noCache: 1, cacheRead: 0, cacheWrite: 0 },
  outputTokens: { total: 1, text: 1, reasoning: 0 },
};
function findWorkspaceName(value: unknown): string | undefined {
  if (typeof value === "string") {
    const text = value.startsWith("Structured content: ")
      ? value.slice("Structured content: ".length)
      : value;
    try {
      return findWorkspaceName(JSON.parse(text));
    } catch {
      return undefined;
    }
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findWorkspaceName(item);
      if (found) return found;
    }
    return undefined;
  }
  if (!value || typeof value !== "object") return undefined;
  const object = value as Record<string, unknown>;
  if (object.kind === "workspaces" && Array.isArray(object.items)) {
    const first = object.items[0];
    if (
      first &&
      typeof first === "object" &&
      typeof (first as { name?: unknown }).name === "string"
    )
      return (first as { name: string }).name;
  }
  for (const child of Object.values(object)) {
    const found = findWorkspaceName(child);
    if (found) return found;
  }
  return undefined;
}
function hasFixtureToolResult(prompt: unknown) {
  return JSON.stringify(prompt).includes('"fixture-tool-1"');
}
function hasFixtureParallelResults(prompt: unknown) {
  const content = JSON.stringify(prompt);
  return (
    content.includes('"fixture-tool-1"') && content.includes('"fixture-tool-2"')
  );
}
globalThis.fetch = async (_url, init) => {
  const headers = new Headers(init?.headers);
  if (
    [...headers.values()].some((value) =>
      value.includes("fixture-catalog-denied"),
    )
  )
    return Response.json({ error: "FIXTURE_PRIVATE_ERROR" }, { status: 401 });
  return Response.json({
    data: [
      {
        id: "fixture-catalog-model",
        supportedGenerationMethods: ["generateContent"],
      },
    ],
    models: [
      {
        name: "models/fixture-catalog-model",
        supportedGenerationMethods: ["generateContent"],
      },
    ],
  });
};
const measuredOutput = new Writable({
  write(bytes, _encoding, complete) {
    const event = JSON.parse(bytes.toString());
    if (event.type === "chunk") event.payload.probeSentAt = Date.now();
    process.stdout.write(JSON.stringify(event) + "\n", complete);
  },
});
void serve(
  process.stdin,
  measuredOutput,
  (input) =>
    new MockLanguageModelV4({
      modelId: input.model,
      doStream: async ({ abortSignal, prompt }) => ({
        stream: new ReadableStream({
          async start(controller) {
            controller.enqueue({
              type: "stream-start",
              warnings: [{ type: "other", message: "FIXTURE_PRIVATE_WARNING" }],
            });
            if (
              [
                "fixture-mcp",
                "fixture-mcp-parallel",
                "fixture-mcp-cancel",
                "fixture-mcp-invalid-input",
                "fixture-mcp-unknown-tool",
              ].includes(input.model)
            ) {
              if (
                (["fixture-mcp", "fixture-mcp-cancel"].includes(input.model) &&
                  hasFixtureToolResult(prompt)) ||
                (input.model === "fixture-mcp-parallel" &&
                  hasFixtureParallelResults(prompt))
              ) {
                const name = findWorkspaceName(prompt);
                controller.enqueue({
                  type: "text-start",
                  id: "block-1",
                  providerMetadata: {
                    google: { thoughtSignature: "fixture-text-signature" },
                  },
                });
                controller.enqueue({
                  type: "text-delta",
                  id: "block-1",
                  delta:
                    input.model === "fixture-mcp-cancel"
                      ? "post-tool:"
                      : name
                        ? `Workspace result: ${name}`
                        : "Workspace result missing",
                  providerMetadata: {
                    google: { thoughtSignature: "fixture-text-signature" },
                  },
                });
                controller.enqueue({
                  type: "text-end",
                  id: "block-1",
                  providerMetadata: {
                    google: { thoughtSignature: "fixture-text-signature" },
                  },
                });
                if (input.model === "fixture-mcp-cancel") {
                  await new Promise<void>((resolve) => {
                    const timer = setTimeout(resolve, 15_000);
                    abortSignal?.addEventListener(
                      "abort",
                      () => {
                        clearTimeout(timer);
                        resolve();
                      },
                      { once: true },
                    );
                  });
                }
                controller.enqueue({
                  type: "finish",
                  finishReason: { unified: "stop", raw: "stop" },
                  usage,
                });
                controller.close();
                return;
              }
              if (input.model === "fixture-mcp") {
                controller.enqueue({
                  type: "text-start",
                  id: "block-1",
                  providerMetadata: {
                    google: {
                      thoughtSignature: "fixture-first-step-signature",
                    },
                  },
                });
                controller.enqueue({
                  type: "text-delta",
                  id: "block-1",
                  delta: "Checking workspaces.",
                });
                controller.enqueue({ type: "text-end", id: "block-1" });
              }
              const parallel = input.model === "fixture-mcp-parallel";
              if (
                input.model === "fixture-mcp-invalid-input" ||
                input.model === "fixture-mcp-unknown-tool"
              ) {
                controller.enqueue({
                  type: "tool-call",
                  toolCallId:
                    input.model === "fixture-mcp-invalid-input"
                      ? "fixture-invalid-input"
                      : "fixture-unknown-tool",
                  toolName:
                    input.model === "fixture-mcp-invalid-input"
                      ? "lomi_workspace_list"
                      : "lomi_unavailable",
                  input:
                    input.model === "fixture-mcp-invalid-input" ? "[]" : "{}",
                });
                controller.enqueue({
                  type: "finish",
                  finishReason: { unified: "tool-calls", raw: "tool_calls" },
                  usage,
                });
                controller.close();
                return;
              }
              const callCount = parallel ? 2 : 1;
              for (let index = 0; index < callCount; index++) {
                controller.enqueue({
                  type: "tool-call",
                  toolCallId:
                    parallel && index === 1
                      ? "fixture-tool-2"
                      : "fixture-tool-1",
                  toolName: "lomi_workspace_list",
                  input: "{}",
                  providerMetadata: {
                    google: {
                      thoughtSignature: "fixture-tool-signature",
                    },
                  },
                });
              }
              controller.enqueue({
                type: "finish",
                finishReason: { unified: "tool-calls", raw: "tool_calls" },
                usage,
              });
              controller.close();
              return;
            }
            if (
              input.model === "fixture-history" ||
              input.model === "fixture-history-pending"
            ) {
              const content = JSON.stringify(prompt);
              controller.enqueue({ type: "text-start", id: "block-1" });
              controller.enqueue({
                type: "text-delta",
                id: "block-1",
                delta:
                  input.model === "fixture-history-pending"
                    ? [
                        content.includes("completed-call")
                          ? "completed-call-replayed"
                          : "completed-call-missing",
                        content.includes("pending-call")
                          ? "pending-call-replayed"
                          : "pending-call-ignored",
                      ].join(" ")
                    : content.includes("fixture-tool-signature")
                      ? "metadata-roundtrip-ok"
                      : "metadata-roundtrip-missing",
              });
              controller.enqueue({ type: "text-end", id: "block-1" });
              controller.enqueue({
                type: "finish",
                finishReason: { unified: "stop", raw: "stop" },
                usage,
              });
              controller.close();
              return;
            }
            controller.enqueue({ type: "text-start", id: "block-1" });
            const count =
              input.model === "fixture-fast"
                ? 10000
                : input.model === "fixture-large"
                  ? 100
                  : 40;
            for (let i = 0; i < count && !abortSignal?.aborted; i++) {
              controller.enqueue({
                type: "text-delta",
                id: "block-1",
                delta:
                  input.model === "fixture-large"
                    ? "x".repeat(32768)
                    : "Zażółć 日本語 👩🏽‍💻\n",
              });
              await new Promise((resolve) =>
                setTimeout(resolve, input.model === "fixture-fast" ? 0 : 10),
              );
            }
            controller.enqueue({ type: "text-end", id: "block-1" });
            if (input.model === "fixture-error")
              controller.enqueue({
                type: "error",
                error: new Error("FIXTURE_PRIVATE_ERROR"),
              });
            controller.enqueue({
              type: "finish",
              finishReason: { unified: "stop", raw: "stop" },
              usage,
            });
            controller.close();
          },
        }),
      }),
    }),
).then(
  () => process.exit(0),
  () => process.exit(1),
);
