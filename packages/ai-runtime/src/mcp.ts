import { dynamicTool, jsonSchema, type ToolSet } from "ai";
import { MAX_TOOL_CALLS, MAX_TOOL_INPUT, type Generation } from "./protocol.ts";

type JsonSchemaInput = Exclude<
  Parameters<typeof jsonSchema>[0],
  PromiseLike<unknown> | (() => unknown)
>;
type ModelOutput = Awaited<
  ReturnType<NonNullable<ToolSet[string]["toModelOutput"]>>
>;
type ModelContentPart =
  | { type: "text"; text: string }
  | {
      type: "file";
      mediaType: string;
      data: { type: "data"; data: string };
      filename?: string;
    };

export interface McpCall {
  toolCallId: string;
  toolName: string;
  input: unknown;
  abortSignal?: AbortSignal;
}

export type McpExecutor = (call: McpCall) => Promise<McpCallToolResult>;

export interface McpCallToolResult {
  content?: unknown[];
  structuredContent?: Record<string, unknown>;
  isError?: boolean;
  [key: string]: unknown;
}

export function normalizeInputSchema(
  schema: Record<string, unknown>,
): JsonSchemaInput {
  const normalize = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(normalize);
    if (value === null || typeof value !== "object") return value;
    const object = value as Record<string, unknown>;
    const output: Record<string, unknown> = {};
    for (const [key, child] of Object.entries(object)) {
      if (key === "$schema") continue;
      const normalizedKey = key === "$defs" ? "definitions" : key;
      let normalized = normalize(child);
      if (
        key === "$ref" &&
        typeof child === "string" &&
        child.startsWith("#/$defs/")
      ) {
        normalized = child.replace("#/$defs/", "#/definitions/");
      }
      output[normalizedKey] = normalized;
    }
    return output;
  };
  return normalize(schema) as JsonSchemaInput;
}

export function createMcpTools(
  mcp: Generation["mcp"],
  execute: McpExecutor,
  onFailure: (error: unknown) => void,
): ToolSet | undefined {
  if (!mcp?.tools.length) return undefined;
  if (mcp.tools.length > MAX_TOOL_CALLS) throw new Error("protocol");

  const tools: ToolSet = {};
  for (const entry of mcp.tools) {
    tools[entry.name] = dynamicTool({
      description: entry.description,
      inputSchema: jsonSchema(normalizeInputSchema(entry.inputSchema)),
      execute: async (input, options) => {
        try {
          return await execute({
            toolCallId: options.toolCallId,
            toolName: entry.name,
            input,
            abortSignal: options.abortSignal,
          });
        } catch (error) {
          onFailure(error);
          throw error;
        }
      },
      toModelOutput: ({ output }) => mcpResultToModelOutput(output),
    });
  }
  return tools;
}

export function parseMcpCallToolResult(value: unknown): McpCallToolResult {
  if (value === null || typeof value !== "object" || Array.isArray(value))
    throw new Error("protocol");
  const result = value as Record<string, unknown>;
  if (
    (result.content !== undefined && !Array.isArray(result.content)) ||
    (result.structuredContent !== undefined &&
      (result.structuredContent === null ||
        typeof result.structuredContent !== "object" ||
        Array.isArray(result.structuredContent))) ||
    (result.isError !== undefined && typeof result.isError !== "boolean")
  )
    throw new Error("protocol");
  return result as McpCallToolResult;
}

export function assertMcpCallInput(
  input: unknown,
): asserts input is Record<string, unknown> {
  if (
    input === null ||
    typeof input !== "object" ||
    Array.isArray(input) ||
    Buffer.byteLength(JSON.stringify(input)) > MAX_TOOL_INPUT
  )
    throw new Error("tool-input-limit");
}

export function mcpResultToModelOutput(value: unknown): ModelOutput {
  const result = parseMcpCallToolResult(value);
  const textParts: string[] = [];
  const content: ModelContentPart[] = [];

  const addResource = (resource: unknown) => {
    if (resource === null || typeof resource !== "object") return;
    const item = resource as Record<string, unknown>;
    if (typeof item.text === "string") {
      textParts.push(item.text);
      content.push({ type: "text", text: item.text });
    } else if (
      typeof item.blob === "string" &&
      typeof item.mimeType === "string"
    ) {
      content.push({
        type: "file",
        mediaType: item.mimeType,
        data: { type: "data", data: item.blob },
        ...(typeof item.uri === "string" ? { filename: item.uri } : {}),
      });
    }
  };

  for (const entry of result.content ?? []) {
    if (entry === null || typeof entry !== "object" || Array.isArray(entry))
      continue;
    const part = entry as Record<string, unknown>;
    if (part.type === "text" && typeof part.text === "string") {
      textParts.push(part.text);
      content.push({ type: "text", text: part.text });
    } else if (
      part.type === "image" &&
      typeof part.data === "string" &&
      typeof part.mimeType === "string"
    ) {
      content.push({
        type: "file",
        mediaType: part.mimeType,
        data: { type: "data", data: part.data },
      });
    } else if (
      part.type === "audio" &&
      typeof part.data === "string" &&
      typeof part.mimeType === "string"
    ) {
      content.push({
        type: "file",
        mediaType: part.mimeType,
        data: { type: "data", data: part.data },
      });
    } else if (part.type === "resource") {
      addResource(part.resource);
    } else if (part.type === "resource_link" && typeof part.uri === "string") {
      textParts.push(
        typeof part.name === "string"
          ? `Resource ${part.name}: ${part.uri}`
          : `Resource: ${part.uri}`,
      );
      content.push({ type: "text", text: textParts.at(-1)! });
    }
  }

  if (result.isError) {
    const description = [
      ...textParts,
      ...(result.structuredContent
        ? [JSON.stringify(result.structuredContent)]
        : []),
    ].filter(Boolean);
    return {
      type: "error-text",
      value: description.join("\n") || "The MCP tool returned an error.",
    };
  }

  if (result.structuredContent !== undefined) {
    if (textParts.length || content.length) {
      const text = `Structured content: ${JSON.stringify(result.structuredContent)}`;
      textParts.push(text);
      content.push({ type: "text", text });
    } else {
      return { type: "json", value: result.structuredContent as never };
    }
  }

  if (content.length) return { type: "content", value: content };
  return { type: "json", value: result as never };
}
