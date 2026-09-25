import type { DynamicToolUIPart, UIMessage, UIMessageChunk } from "ai";
import { id, MAX_RESPONSE, MAX_TEXT_RESPONSE } from "./protocol.ts";

// Retain one bounded snapshot. The SDK's eager stream reader can otherwise
// queue a full copy of the growing message for every provider token.
export class Snapshot {
  readonly message: UIMessage;
  readonly blocks = new Map<
    string,
    { index: number; type: "text" | "reasoning"; open: boolean }
  >();
  private readonly toolParts = new Map<string, number>();
  private readonly toolCallIds = new Set<string>();
  private bytes = 0;
  private textBytes = 0;
  private step = 0;

  constructor(id: string) {
    this.message = { id, role: "assistant", parts: [] };
  }

  accept(chunk: UIMessageChunk) {
    this.bytes += Buffer.byteLength(JSON.stringify(chunk));
    if (this.bytes > MAX_RESPONSE || this.blocks.size > 1024) {
      throw new Error("response-limit");
    }

    if (chunk.type === "start-step") {
      this.step++;
      this.message.parts.push({ type: "step-start" });
      return;
    }

    if (chunk.type === "text-start" || chunk.type === "reasoning-start") {
      const id = this.blockId(chunk.id);
      if (this.blocks.has(id)) throw new Error("protocol");
      const type = chunk.type === "text-start" ? "text" : "reasoning";
      this.blocks.set(id, {
        index: this.message.parts.length,
        type,
        open: true,
      });
      this.message.parts.push({
        type,
        text: "",
        state: "streaming",
        ...(chunk.providerMetadata != null
          ? { providerMetadata: chunk.providerMetadata }
          : {}),
      });
      return;
    }

    if (
      chunk.type === "text-delta" ||
      chunk.type === "reasoning-delta" ||
      chunk.type === "text-end" ||
      chunk.type === "reasoning-end"
    ) {
      const block = this.blocks.get(this.blockId(chunk.id));
      if (!block?.open) throw new Error("protocol");
      const part = this.message.parts[block.index];
      if (part.type !== "text" && part.type !== "reasoning")
        throw new Error("protocol");
      if ("delta" in chunk) {
        if (block.type === "text" || block.type === "reasoning") {
          this.textBytes += Buffer.byteLength(chunk.delta);
          if (this.textBytes > MAX_TEXT_RESPONSE)
            throw new Error("response-limit");
        }
        part.text += chunk.delta;
      } else {
        part.state = "done";
        block.open = false;
      }
      if (chunk.providerMetadata != null)
        part.providerMetadata = chunk.providerMetadata;
      return;
    }

    if (chunk.type === "tool-input-available") {
      if (
        chunk.dynamic !== true ||
        this.toolCallIds.has(chunk.toolCallId) ||
        !id.safeParse(chunk.toolCallId).success
      )
        throw new Error("protocol");
      this.toolCallIds.add(chunk.toolCallId);
      const part: DynamicToolUIPart = {
        type: "dynamic-tool",
        toolName: chunk.toolName,
        toolCallId: chunk.toolCallId,
        state: "input-available",
        input: chunk.input,
        ...(chunk.title != null ? { title: chunk.title } : {}),
        ...(chunk.toolMetadata != null
          ? { toolMetadata: chunk.toolMetadata }
          : {}),
        ...(chunk.providerExecuted != null
          ? { providerExecuted: chunk.providerExecuted }
          : {}),
        ...(chunk.providerMetadata != null
          ? { callProviderMetadata: chunk.providerMetadata }
          : {}),
      };
      this.toolParts.set(chunk.toolCallId, this.message.parts.length);
      this.message.parts.push(part);
      return;
    }

    if (chunk.type === "tool-input-error") {
      if (
        chunk.dynamic !== true ||
        this.toolCallIds.has(chunk.toolCallId) ||
        !id.safeParse(chunk.toolCallId).success
      )
        throw new Error("protocol");
      this.toolCallIds.add(chunk.toolCallId);
      const part: DynamicToolUIPart = {
        type: "dynamic-tool",
        toolName: chunk.toolName,
        toolCallId: chunk.toolCallId,
        state: "output-error",
        input: chunk.input,
        errorText: chunk.errorText,
        ...(chunk.title != null ? { title: chunk.title } : {}),
        ...(chunk.toolMetadata != null
          ? { toolMetadata: chunk.toolMetadata }
          : {}),
        ...(chunk.providerExecuted != null
          ? { providerExecuted: chunk.providerExecuted }
          : {}),
        ...(chunk.providerMetadata != null
          ? { callProviderMetadata: chunk.providerMetadata }
          : {}),
      };
      this.toolParts.set(chunk.toolCallId, this.message.parts.length);
      this.message.parts.push(part);
      return;
    }

    if (
      chunk.type === "tool-output-available" ||
      chunk.type === "tool-output-error"
    ) {
      const index = this.toolParts.get(chunk.toolCallId);
      const part = index === undefined ? undefined : this.message.parts[index];
      if (
        !part ||
        part.type !== "dynamic-tool" ||
        part.state !== "input-available" ||
        chunk.dynamic !== true
      )
        throw new Error("protocol");
      if (chunk.type === "tool-output-available") {
        Object.assign(part, {
          state: "output-available",
          output: chunk.output,
          ...(chunk.providerExecuted != null
            ? { providerExecuted: chunk.providerExecuted }
            : {}),
          ...(chunk.providerMetadata != null
            ? { resultProviderMetadata: chunk.providerMetadata }
            : {}),
          ...(chunk.toolMetadata != null
            ? { toolMetadata: chunk.toolMetadata }
            : {}),
          ...(chunk.preliminary != null
            ? { preliminary: chunk.preliminary }
            : {}),
        });
      } else {
        Object.assign(part, {
          state: "output-error",
          errorText: chunk.errorText,
          ...(chunk.providerExecuted != null
            ? { providerExecuted: chunk.providerExecuted }
            : {}),
          ...(chunk.providerMetadata != null
            ? { resultProviderMetadata: chunk.providerMetadata }
            : {}),
          ...(chunk.toolMetadata != null
            ? { toolMetadata: chunk.toolMetadata }
            : {}),
        });
      }
      return;
    }
  }

  value() {
    return { message: this.message, blocks: Object.fromEntries(this.blocks) };
  }

  namespace(id: string) {
    return `${this.step}:${id}`;
  }

  private blockId(id: string) {
    return this.namespace(id);
  }
}
