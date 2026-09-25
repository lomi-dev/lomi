import { z } from "zod";

export const VERSION = 1;
export const MAX_CONTEXT = 40 * 1024 * 1024;
export const MAX_FRAME = 96 * 1024;
export const MAX_RESPONSE = 16 * 1024 * 1024;
export const MAX_TEXT_RESPONSE = 2 * 1024 * 1024;
export const MAX_TOOL_RESULT = 8 * 1024 * 1024;
export const MAX_TOOL_RESULTS = 16 * 1024 * 1024;
export const MAX_TOOL_INPUT = 64 * 1024;
export const MAX_TOOL_CALLS = 128;
export const MAX_TOOL_STEPS = 32;
export const MAX_TOOL_FRAME_DATA = 48 * 1024;
export const MAX_TOOL_FRAME_BYTES = 32 * 1024;
export const id = z.string().regex(/^[a-zA-Z0-9_-]{1,100}$/);
const providerMetadata = z.record(z.string(), z.json()).optional();
const toolMetadata = z.record(z.string(), z.json()).optional();
const part = z.union([
  z
    .object({
      type: z.literal("text"),
      text: z.string().max(MAX_CONTEXT),
      state: z.enum(["streaming", "done"]).optional(),
      providerMetadata,
    })
    .strict(),
  z
    .object({
      type: z.literal("file"),
      mediaType: z.enum(["image/png", "image/jpeg", "image/webp"]),
      url: z
        .string()
        .max(14 * 1024 * 1024)
        .regex(/^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+/]+=*$/),
      filename: z.string().max(255).optional(),
      providerMetadata,
    })
    .strict(),
  z
    .object({
      type: z.literal("reasoning"),
      id: z.string().max(256).optional(),
      text: z.string().max(MAX_CONTEXT),
      state: z.enum(["streaming", "done"]).optional(),
      providerMetadata,
    })
    .strict(),
  z.object({ type: z.literal("step-start") }).strict(),
  z
    .object({
      type: z.literal("dynamic-tool"),
      toolName: z.string().min(1).max(100),
      toolCallId: id,
      state: z.literal("input-streaming"),
      input: z.json().optional(),
      title: z.string().max(1024).optional(),
      toolMetadata,
      providerExecuted: z.boolean().optional(),
      callProviderMetadata: providerMetadata,
    })
    .strict(),
  z
    .object({
      type: z.literal("dynamic-tool"),
      toolName: z.string().min(1).max(100),
      toolCallId: id,
      state: z.literal("input-available"),
      input: z.json(),
      title: z.string().max(1024).optional(),
      toolMetadata,
      providerExecuted: z.boolean().optional(),
      callProviderMetadata: providerMetadata,
    })
    .strict(),
  z
    .object({
      type: z.literal("dynamic-tool"),
      toolName: z.string().min(1).max(100),
      toolCallId: id,
      state: z.literal("output-available"),
      input: z.json(),
      output: z.json(),
      title: z.string().max(1024).optional(),
      toolMetadata,
      providerExecuted: z.boolean().optional(),
      callProviderMetadata: providerMetadata,
      resultProviderMetadata: providerMetadata,
      preliminary: z.boolean().optional(),
    })
    .strict(),
  z
    .object({
      type: z.literal("dynamic-tool"),
      toolName: z.string().min(1).max(100),
      toolCallId: id,
      state: z.literal("output-error"),
      input: z.json().optional(),
      rawInput: z.json().optional(),
      errorText: z.string().max(8192),
      title: z.string().max(1024).optional(),
      toolMetadata,
      providerExecuted: z.boolean().optional(),
      callProviderMetadata: providerMetadata,
      resultProviderMetadata: providerMetadata,
    })
    .strict(),
]);
const mcpTool = z
  .object({
    name: z.string().regex(/^[a-zA-Z0-9_-]{1,64}$/),
    description: z.string().max(16 * 1024),
    inputSchema: z.record(z.string(), z.json()),
  })
  .strict();
export const generation = z
  .object({
    operation: z
      .enum(["generate", "test-connection", "list-models"])
      .default("generate"),
    provider: z.enum([
      "openai",
      "anthropic",
      "google",
      "xai",
      "openrouter",
      "deepseek",
      "nvidia",
    ]),
    apiKey: z.string().min(1).max(8192),
    model: z.string().regex(/^[a-zA-Z0-9][a-zA-Z0-9._:/-]{0,199}$/),
    assistantId: id,
    messages: z
      .array(
        z
          .object({
            id,
            role: z.enum(["user", "assistant"]),
            parts: z.array(part).max(1024),
          })
          .strict(),
      )
      .min(1)
      .max(2000),
    system: z
      .string()
      .max(128 * 1024)
      .default(""),
    maxOutputTokens: z.number().int().min(1).max(32768).default(4096),
    temperature: z.number().min(0).max(2).optional(),
  })
  .extend({
    mcp: z
      .object({
        tools: z.array(mcpTool).max(MAX_TOOL_CALLS),
        instructions: z.string().max(128 * 1024),
      })
      .strict()
      .optional(),
  })
  .strict()
  .superRefine((value, context) => {
    if (value.operation !== "generate" && value.mcp !== undefined) {
      context.addIssue({
        code: "custom",
        path: ["mcp"],
        message: "MCP tools are available only for generation.",
      });
    }
    if (value.mcp) {
      const names = new Set<string>();
      value.mcp.tools.forEach((tool, index) => {
        if (names.has(tool.name)) {
          context.addIssue({
            code: "custom",
            path: ["mcp", "tools", index, "name"],
            message: "MCP tool names must be unique.",
          });
        }
        names.add(tool.name);
      });
    }
  });
export type Generation = z.infer<typeof generation>;
export const frame = z
  .object({
    protocolVersion: z.literal(VERSION),
    requestId: id,
    type: z.enum([
      "hello",
      "begin",
      "append",
      "generate",
      "cancel",
      "shutdown",
      "tool-result-begin",
      "tool-result-append",
      "tool-result-end",
    ]),
    payload: z
      .object({
        data: z
          .string()
          .max(MAX_TOOL_FRAME_DATA)
          .regex(/^[A-Za-z0-9+/]*={0,2}$/)
          .optional(),
        toolCallId: id.optional(),
      })
      .strict()
      .nullable()
      .optional(),
  })
  .strict()
  .superRefine((value, context) => {
    const payload = value.payload;
    if (
      value.type === "tool-result-begin" ||
      value.type === "tool-result-end"
    ) {
      if (!payload?.toolCallId || payload.data !== undefined) {
        context.addIssue({ code: "custom", path: ["payload"] });
      }
    } else if (value.type === "tool-result-append") {
      if (!payload?.toolCallId || payload.data === undefined) {
        context.addIssue({ code: "custom", path: ["payload"] });
      }
    } else if (payload?.toolCallId !== undefined) {
      context.addIssue({ code: "custom", path: ["payload", "toolCallId"] });
    }
  });
export interface Event {
  protocolVersion: 1;
  requestId: string;
  sequence: number;
  type:
    | "models"
    | "ready"
    | "chunk"
    | "tool-call"
    | "message-snapshot"
    | "completed"
    | "cancelled"
    | "failed";
  payload: unknown;
}
export type Emit = (event: Event) => Promise<void>;

export function errorCode(error: unknown): string {
  if (typeof error !== "object" || error === null) return "process";
  const value = error as {
    statusCode?: number;
    name?: string;
    data?: { error?: { code?: string } };
  };
  if (value.name === "AbortError") return "cancelled";
  if (value.name === "TimeoutError") return "timeout";
  if (value.data?.error?.code === "insufficient_quota") return "quota";
  if (value.data?.error?.code === "context_length_exceeded")
    return "context-limit";
  switch (value.statusCode) {
    case 401:
      return "auth";
    case 403:
      return "permission";
    case 404:
      return "model";
    case 429:
      return "rate-limit";
    case 400:
      return "unsupported-input";
    default:
      return "network";
  }
}
