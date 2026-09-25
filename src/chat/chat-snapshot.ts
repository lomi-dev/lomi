import type { UIMessage, UIMessageChunk } from "ai";

export interface ChatMessageSnapshot {
  message: UIMessage;
  blocks: Record<
    string,
    { index: number; type: "text" | "reasoning"; open: boolean }
  >;
}

/** Rebuild the SDK stream from the authoritative, ordered message parts. */
export function snapshotToChunks(
  snapshot: ChatMessageSnapshot,
): UIMessageChunk[] {
  const chunks: UIMessageChunk[] = [
    { type: "start", messageId: snapshot.message.id },
  ];
  const blockIds = new Map<number, string>();
  for (const [id, block] of Object.entries(snapshot.blocks))
    blockIds.set(block.index, id);

  snapshot.message.parts.forEach((part, index) => {
    if (part.type === "text" || part.type === "reasoning") {
      const kind = part.type;
      const id =
        blockIds.get(index) ??
        ("id" in part ? part.id : undefined) ??
        `${kind}-${index}`;
      const open = snapshot.blocks[id]?.open ?? part.state === "streaming";
      const providerMetadata = part.providerMetadata
        ? { providerMetadata: part.providerMetadata }
        : {};
      chunks.push({ type: `${kind}-start`, id, ...providerMetadata });
      chunks.push({
        type: `${kind}-delta`,
        id,
        delta: part.text,
        ...providerMetadata,
      });
      if (!open) chunks.push({ type: `${kind}-end`, id, ...providerMetadata });
      return;
    }

    if (part.type === "step-start") {
      chunks.push({ type: "start-step" });
      return;
    }

    if (part.type !== "dynamic-tool") return;

    if (part.state === "input-streaming") {
      chunks.push({
        type: "tool-input-start",
        toolCallId: part.toolCallId,
        toolName: part.toolName,
        dynamic: true,
        ...(part.providerExecuted === undefined
          ? {}
          : { providerExecuted: part.providerExecuted }),
        ...(part.callProviderMetadata
          ? { providerMetadata: part.callProviderMetadata }
          : {}),
        ...(part.toolMetadata ? { toolMetadata: part.toolMetadata } : {}),
        ...(part.title ? { title: part.title } : {}),
      });
      return;
    }

    const common = {
      toolCallId: part.toolCallId,
      toolName: part.toolName,
      input: part.input,
      dynamic: true as const,
      ...(part.providerExecuted === undefined
        ? {}
        : { providerExecuted: part.providerExecuted }),
      ...(part.callProviderMetadata
        ? { providerMetadata: part.callProviderMetadata }
        : {}),
      ...(part.toolMetadata ? { toolMetadata: part.toolMetadata } : {}),
      ...(part.title ? { title: part.title } : {}),
    };
    chunks.push({ type: "tool-input-available", ...common });
    if (part.state === "output-available")
      chunks.push({
        type: "tool-output-available",
        toolCallId: part.toolCallId,
        output: part.output,
        dynamic: true,
        ...(part.providerExecuted === undefined
          ? {}
          : { providerExecuted: part.providerExecuted }),
        ...(part.resultProviderMetadata
          ? { providerMetadata: part.resultProviderMetadata }
          : {}),
        ...(part.toolMetadata ? { toolMetadata: part.toolMetadata } : {}),
        ...(part.preliminary === undefined
          ? {}
          : { preliminary: part.preliminary }),
      });
    else if (part.state === "output-error")
      chunks.push({
        type: "tool-output-error",
        toolCallId: part.toolCallId,
        errorText: part.errorText,
        dynamic: true,
        ...(part.providerExecuted === undefined
          ? {}
          : { providerExecuted: part.providerExecuted }),
        ...(part.resultProviderMetadata
          ? { providerMetadata: part.resultProviderMetadata }
          : {}),
        ...(part.toolMetadata ? { toolMetadata: part.toolMetadata } : {}),
      });
  });

  return chunks;
}
