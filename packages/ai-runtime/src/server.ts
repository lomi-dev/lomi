import {
  frame,
  generation,
  VERSION,
  MAX_FRAME,
  MAX_CONTEXT,
  MAX_TOOL_CALLS,
  MAX_TOOL_FRAME_BYTES,
  MAX_TOOL_RESULT,
  MAX_TOOL_RESULTS,
  type Emit,
  type Generation,
} from "./protocol.ts";
import { catalog } from "./catalog.ts";
import { generate } from "./generate.ts";
import {
  assertMcpCallInput,
  parseMcpCallToolResult,
  type McpCall,
  type McpCallToolResult,
} from "./mcp.ts";
import type { LanguageModel } from "ai";
import type { Writable, Readable } from "node:stream";

interface ToolTransfer {
  bytes: number;
  chunks: Buffer[];
}

interface PendingToolCall {
  resolve: (result: McpCallToolResult) => void;
  reject: (error: Error) => void;
  transfer?: ToolTransfer;
  completed: boolean;
}

class ToolCallChannel {
  private readonly requestId: string;
  private readonly signal: AbortSignal;
  private readonly emit: Emit;
  private readonly toolNames: Set<string>;
  private readonly pending = new Map<string, PendingToolCall>();
  private readonly callIds = new Set<string>();
  private tail = Promise.resolve();
  private closed = false;
  private callCount = 0;
  private resultBytes = 0;

  constructor(
    requestId: string,
    signal: AbortSignal,
    emit: Emit,
    toolNames: Set<string>,
  ) {
    this.requestId = requestId;
    this.signal = signal;
    this.emit = emit;
    this.toolNames = toolNames;
    signal.addEventListener("abort", this.onAbort, { once: true });
    if (signal.aborted) this.abort();
  }

  readonly execute = (call: McpCall): Promise<McpCallToolResult> => {
    if (this.closed || this.signal.aborted)
      return Promise.reject(new DOMException("Cancelled", "AbortError"));
    if (!this.toolNames.has(call.toolName))
      return Promise.reject(new Error("unsupported-input"));
    try {
      assertMcpCallInput(call.input);
    } catch {
      return Promise.reject(new Error("unsupported-input"));
    }
    if (
      this.callCount >= MAX_TOOL_CALLS ||
      this.callIds.has(call.toolCallId) ||
      this.pending.has(call.toolCallId)
    )
      return Promise.reject(new Error("unsupported-input"));

    this.callCount++;
    this.callIds.add(call.toolCallId);
    let resolve!: (result: McpCallToolResult) => void;
    let reject!: (error: Error) => void;
    const response = new Promise<McpCallToolResult>((accept, fail) => {
      resolve = accept;
      reject = fail;
    });
    void response.catch(() => {});
    const pending: PendingToolCall = { resolve, reject, completed: false };
    this.pending.set(call.toolCallId, pending);

    const queued = this.tail.then(async () => {
      if (this.closed || this.signal.aborted)
        throw new DOMException("Cancelled", "AbortError");
      await this.emit({
        protocolVersion: VERSION,
        requestId: this.requestId,
        sequence: 0,
        type: "tool-call",
        payload: {
          toolCallId: call.toolCallId,
          toolName: call.toolName,
          input: call.input,
        },
      });
      return response;
    });
    this.tail = queued.then(
      () => undefined,
      () => undefined,
    );

    const abortSignal = call.abortSignal ?? this.signal;
    return this.abortable(
      queued.then((value) => value),
      abortSignal,
    ).finally(() => {
      if (this.pending.get(call.toolCallId) === pending)
        this.pending.delete(call.toolCallId);
    });
  };

  receive(
    type: "tool-result-begin" | "tool-result-append" | "tool-result-end",
    payload: { toolCallId?: string; data?: string } | null | undefined,
  ) {
    if (this.closed || this.signal.aborted) return;
    const toolCallId = payload?.toolCallId;
    if (!toolCallId) throw new Error("protocol");
    const pending = this.pending.get(toolCallId);
    if (!pending || pending.completed) throw new Error("protocol");

    if (type === "tool-result-begin") {
      if (pending.transfer) throw new Error("protocol");
      pending.transfer = { bytes: 0, chunks: [] };
      return;
    }

    const transfer = pending.transfer;
    if (!transfer) throw new Error("protocol");
    if (type === "tool-result-append") {
      if (payload.data === undefined) throw new Error("protocol");
      const chunk = Buffer.from(payload.data, "base64");
      const canonical = chunk.toString("base64");
      if (
        canonical.replace(/=+$/, "") !== payload.data.replace(/=+$/, "") ||
        chunk.byteLength > MAX_TOOL_FRAME_BYTES ||
        transfer.chunks.length >= MAX_TOOL_RESULT / MAX_TOOL_FRAME_BYTES ||
        (transfer.bytes += chunk.byteLength) > MAX_TOOL_RESULT
      )
        throw new Error("protocol");
      transfer.chunks.push(chunk);
      return;
    }

    const data = Buffer.concat(transfer.chunks, transfer.bytes);
    const totalBytes = this.resultBytes + data.byteLength;
    if (data.byteLength > MAX_TOOL_RESULT || totalBytes > MAX_TOOL_RESULTS)
      throw new Error("protocol");
    const result = parseMcpCallToolResult(
      JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(data)),
    );
    pending.completed = true;
    this.resultBytes = totalBytes;
    pending.resolve(result);
  }

  abort() {
    if (this.closed) return;
    this.closed = true;
    for (const pending of this.pending.values())
      pending.reject(new DOMException("Cancelled", "AbortError"));
    this.pending.clear();
  }

  dispose() {
    this.abort();
    this.signal.removeEventListener("abort", this.onAbort);
  }

  private readonly onAbort = () => this.abort();

  private abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
    if (signal.aborted)
      return Promise.reject(new DOMException("Cancelled", "AbortError"));
    return new Promise<T>((resolve, reject) => {
      const onAbort = () => {
        signal.removeEventListener("abort", onAbort);
        reject(new DOMException("Cancelled", "AbortError"));
      };
      signal.addEventListener("abort", onAbort, { once: true });
      promise.then(
        (value) => {
          signal.removeEventListener("abort", onAbort);
          resolve(value);
        },
        (error) => {
          signal.removeEventListener("abort", onAbort);
          reject(error);
        },
      );
    });
  }
}

export async function serve(
  input: Readable,
  output: Writable,
  modelFor: (input: Generation) => LanguageModel,
) {
  const transfers = new Map<string, { bytes: number; chunks: Buffer[] }>();
  const active = new Map<
    string,
    { controller: AbortController; tools: ToolCallChannel }
  >();
  const retired = new Set<string>();
  let writes = Promise.resolve();
  let stopping = false;
  const eventSequences = new Map<string, number>();
  const emit: Emit = (event) => {
    const line = JSON.stringify(event) + "\n";
    // At most one awaiting write per active request plus bounded control replies.
    const next = writes.then(
      () =>
        new Promise<void>((resolve, reject) =>
          output.write(line, (error) => (error ? reject(error) : resolve())),
        ),
    );
    writes = next.catch(() => {});
    return next;
  };
  const retire = (id: string) => {
    retired.add(id);
    if (retired.size > 256) retired.delete(retired.values().next().value!);
  };
  let pending = Buffer.alloc(0);
  try {
    for await (const data of input) {
      const buffer = Buffer.concat([pending, data]);
      let start = 0;
      for (;;) {
        const end = buffer.indexOf(10, start);
        if (end < 0) break;
        if (end - start > MAX_FRAME) throw new Error("protocol");
        // Decode only complete frames, preserving UTF-8 across pipe reads.
        const parsed = frame.safeParse(
          JSON.parse(
            new TextDecoder("utf-8", { fatal: true }).decode(
              buffer.subarray(start, end),
            ),
          ),
        );
        if (!parsed.success) throw new Error("protocol");
        const { type, requestId, payload } = parsed.data;
        start = end + 1;
        if (type === "shutdown") {
          stopping = true;
          break;
        }
        if (type === "cancel") {
          if (retired.has(requestId)) continue;
          const request = active.get(requestId);
          if (request) request.controller.abort();
          else {
            transfers.delete(requestId);
            retire(requestId);
            await emit({
              protocolVersion: VERSION,
              requestId,
              sequence: 1,
              type: "cancelled",
              payload: {},
            });
          }
          continue;
        }
        if (
          type === "tool-result-begin" ||
          type === "tool-result-append" ||
          type === "tool-result-end"
        ) {
          if (retired.has(requestId)) continue;
          const request = active.get(requestId);
          if (!request) throw new Error("protocol");
          request.tools.receive(type, payload);
          continue;
        }
        if (type === "hello") {
          await emit({
            protocolVersion: VERSION,
            requestId,
            sequence: 0,
            type: "ready",
            payload: { node: process.versions.node },
          });
          continue;
        }
        if (retired.has(requestId)) continue;
        if (type === "begin") {
          if (
            transfers.has(requestId) ||
            active.has(requestId) ||
            transfers.size + active.size >= 4
          )
            throw new Error("protocol");
          transfers.set(requestId, { bytes: 0, chunks: [] });
        } else if (type === "append") {
          const transfer = transfers.get(requestId);
          if (!transfer || !payload?.data) throw new Error("protocol");
          const chunk = Buffer.from(payload.data, "base64");
          if ((transfer.bytes += chunk.length) > MAX_CONTEXT)
            throw new Error("protocol");
          if (transfer.chunks.length >= 2048) throw new Error("protocol");
          transfer.chunks.push(chunk);
        } else if (type === "generate") {
          const transfer = transfers.get(requestId);
          if (!transfer) throw new Error("protocol");
          transfers.delete(requestId);
          const parsed = generation.safeParse(
            JSON.parse(
              new TextDecoder("utf-8", { fatal: true }).decode(
                Buffer.concat(transfer.chunks),
              ),
            ),
          );
          if (!parsed.success) throw new Error("protocol");
          const controller = new AbortController();
          const requestEmit: Emit = (event) => {
            const sequence = (eventSequences.get(requestId) ?? 0) + 1;
            eventSequences.set(requestId, sequence);
            return emit({ ...event, sequence });
          };
          const toolNames = new Set(
            parsed.data.mcp?.tools.map(({ name }) => name),
          );
          const tools = new ToolCallChannel(
            requestId,
            controller.signal,
            requestEmit,
            toolNames,
          );
          active.set(requestId, { controller, tools });
          void (
            parsed.data.operation === "list-models"
              ? catalog(parsed.data, requestId, controller, emit)
              : generate(
                  parsed.data,
                  requestId,
                  controller,
                  requestEmit,
                  modelFor(parsed.data),
                  tools.execute,
                )
          )
            .catch(() => {
              stopping = true;
              input.destroy();
            })
            .finally(() => {
              tools.dispose();
              active.delete(requestId);
              retire(requestId);
              eventSequences.delete(requestId);
            });
        }
      }
      if (stopping) break;
      pending = Buffer.from(buffer.subarray(start));
      if (pending.length > MAX_FRAME) throw new Error("protocol");
    }
  } finally {
    for (const request of active.values()) request.controller.abort();
    transfers.clear();
  }
}
