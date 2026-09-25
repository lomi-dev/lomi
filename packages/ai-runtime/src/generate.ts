import {
  convertToModelMessages,
  stepCountIs,
  streamText,
  toUIMessageStream,
  validateUIMessages,
  type LanguageModel,
  type UIMessageChunk,
} from "ai";
import {
  type Generation,
  type Emit,
  errorCode,
  MAX_RESPONSE,
  MAX_TOOL_STEPS,
  VERSION,
} from "./protocol.ts";
import { createMcpTools, type McpExecutor } from "./mcp.ts";
import { Snapshot } from "./snapshot.ts";

globalThis.AI_SDK_LOG_WARNINGS = false;

export async function generate(
  input: Generation,
  requestId: string,
  controller: AbortController,
  emit: Emit,
  model: LanguageModel,
  executeMcpTool?: McpExecutor,
) {
  if (input.operation === "test-connection")
    input = {
      ...input,
      messages: [
        {
          id: "connection-test",
          role: "user",
          parts: [{ type: "text", text: "Reply with OK." }],
        },
      ],
      system: "",
      maxOutputTokens: 32,
      temperature: undefined,
    };
  const timeout = input.operation === "test-connection" ? 30_000 : 600_000;
  let sequence = 0;
  const output = async (type: Parameters<Emit>[0]["type"], payload: unknown) =>
    emit({
      protocolVersion: VERSION,
      requestId,
      sequence: ++sequence,
      type,
      payload,
    });
  const snapshot = new Snapshot(input.assistantId);
  const tools =
    input.operation === "generate"
      ? createMcpTools(
          input.mcp,
          executeMcpTool ??
            (async () => {
              throw new Error("unsupported-input");
            }),
          (error) => {
            if (
              error instanceof Error &&
              ["unsupported-input", "response-limit"].includes(error.message)
            )
              failure = error.message;
            else failure ??= errorCode(error);
          },
        )
      : undefined;
  let failure: string | undefined;
  let usage: unknown;
  let rawBytes = 0;
  let count = 0;
  let lastSnapshot = Date.now();
  let snapshotBytes = 0;
  const deadline = setTimeout(() => {
    failure = "timeout";
    controller.abort();
  }, timeout);
  try {
    if (controller.signal.aborted)
      throw new DOMException("Cancelled", "AbortError");
    const messages = await validateUIMessages({
      messages: input.messages,
      tools,
    });
    if (messages.at(-1)?.role !== "user") throw new Error("protocol");
    const result = streamText({
      model,
      messages: await convertToModelMessages(messages, {
        tools,
        ignoreIncompleteToolCalls: true,
      }),
      system:
        [input.system, input.mcp?.instructions].filter(Boolean).join("\n\n") ||
        undefined,
      ...(tools ? { tools } : {}),
      stopWhen: stepCountIs(MAX_TOOL_STEPS),
      maxOutputTokens: input.maxOutputTokens,
      ...(input.temperature === undefined
        ? {}
        : { temperature: input.temperature }),
      providerOptions:
        input.provider === "openai" ? { openai: { store: false } } : undefined,
      maxRetries: 0,
      abortSignal: controller.signal,
      timeout: { firstChunkMs: 120_000, chunkMs: 120_000, totalMs: timeout },
      // Never download a model-supplied URL or log SDK error bodies.
      experimental_download: async (urls) => {
        if (urls.length) throw new Error("unsupported-input");
        return [];
      },
      onError: ({ error }) => {
        failure ??= errorCode(error);
      },
      onEnd: (event) => {
        usage = {
          inputTokens: event.totalUsage.inputTokens,
          outputTokens: event.totalUsage.outputTokens,
          finishReason: event.finishReason,
        };
      },
      experimental_transform: () =>
        new TransformStream({
          transform(part, sink) {
            rawBytes += Buffer.byteLength(JSON.stringify(part));
            // streamText internally retains a tee. Bound bytes AND tiny event count
            // before that tee rather than assuming its unused branch is drained.
            if (++count > 32768 || rawBytes > MAX_RESPONSE) {
              failure = "response-limit";
              controller.abort();
              throw new Error("response-limit");
            }
            sink.enqueue(part);
          },
        }),
    });
    const stream = toUIMessageStream({
      stream: result.stream,
      originalMessages: messages,
      generateMessageId: () => input.assistantId,
      onError: () => failure ?? "network",
    });
    for await (const raw of stream) {
      let chunk: UIMessageChunk;
      switch (raw.type) {
        case "start":
          chunk = { type: "start", messageId: input.assistantId };
          break;
        case "text-start":
        case "reasoning-start":
        case "text-end":
        case "reasoning-end":
          chunk = {
            type: raw.type,
            id: snapshot.namespace(raw.id),
            ...(raw.providerMetadata != null
              ? { providerMetadata: raw.providerMetadata }
              : {}),
          };
          break;
        case "text-delta":
        case "reasoning-delta":
          chunk = {
            type: raw.type,
            id: snapshot.namespace(raw.id),
            delta: raw.delta,
            ...(raw.providerMetadata != null
              ? { providerMetadata: raw.providerMetadata }
              : {}),
          };
          break;
        case "tool-input-available":
        case "tool-input-error":
        case "tool-output-available":
        case "tool-output-error":
          chunk = raw;
          break;
        case "start-step":
          chunk = raw;
          break;
        case "error":
          failure ??= "network";
          continue;
        case "finish":
        case "abort":
        case "tool-input-start":
        case "tool-input-delta":
        case "finish-step":
          continue;
        default:
          failure = "unsupported-input";
          controller.abort();
          continue;
      }
      snapshot.accept(chunk);
      await output("chunk", chunk);
      snapshotBytes += Buffer.byteLength(JSON.stringify(chunk));
      if (Date.now() - lastSnapshot >= 500 || snapshotBytes >= 32768) {
        await output("message-snapshot", snapshot.value());
        lastSnapshot = Date.now();
        snapshotBytes = 0;
      }
    }
  } catch (error) {
    failure ??=
      error instanceof Error && error.message === "response-limit"
        ? "response-limit"
        : controller.signal.aborted
          ? "cancelled"
          : errorCode(error);
    // A local decoder/size failure must stop the provider even if the SDK's
    // other tee branch is still retained.
    controller.abort();
  } finally {
    clearTimeout(deadline);
  }
  const status =
    failure && failure !== "cancelled"
      ? "failed"
      : controller.signal.aborted
        ? "cancelled"
        : "completed";
  await output("message-snapshot", snapshot.value());
  await output(status, { code: failure, usage });
}
