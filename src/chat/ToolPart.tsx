import { useState } from "react";
import type { DynamicToolUIPart } from "ai";

const MAX_TEXT_PREVIEW = 20_000;
const MAX_INLINE_IMAGE_BASE64 = 4 * 1024 * 1024;
const MAX_INLINE_IMAGES = 4;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isBase64(value: string) {
  return (
    value.length > 0 &&
    value.length % 4 === 0 &&
    /^[A-Za-z0-9+/]*={0,2}$/.test(value)
  );
}

function safeValue(value: unknown, depth = 0): unknown {
  if (depth > 6) return "[nested content omitted]";
  if (typeof value === "string") {
    if (/^data:image\/(png|jpeg|webp);base64,/i.test(value))
      return "[inline image omitted]";
    if (
      value.length > 1024 &&
      value.length % 4 === 0 &&
      /^[A-Za-z0-9+/]*={0,2}$/.test(value)
    )
      return "[binary content omitted]";
    return value.length > MAX_TEXT_PREVIEW
      ? `${value.slice(0, MAX_TEXT_PREVIEW)}… [preview truncated]`
      : value;
  }
  if (Array.isArray(value))
    return [
      ...value.slice(0, 100).map((item) => safeValue(item, depth + 1)),
      ...(value.length > 100 ? [`[${value.length - 100} items omitted]`] : []),
    ];
  if (isRecord(value))
    return Object.fromEntries(
      Object.entries(value)
        .slice(0, 100)
        .map(([key, item]) => [key, safeValue(item, depth + 1)]),
    );
  return value;
}

function preview(value: unknown) {
  let text: string;
  try {
    text =
      typeof value === "string"
        ? (safeValue(value) as string)
        : (JSON.stringify(safeValue(value), null, 2) ?? "[no content]");
  } catch {
    text = "[content unavailable]";
  }
  return text.length > MAX_TEXT_PREVIEW
    ? `${text.slice(0, MAX_TEXT_PREVIEW)}\n… [preview truncated]`
    : text;
}

function InlineImage({ value }: { value: Record<string, unknown> }) {
  const mimeType =
    typeof value.mimeType === "string" ? value.mimeType.toLowerCase() : "";
  const data = typeof value.data === "string" ? value.data : "";
  const safeMime = ["image/png", "image/jpeg", "image/webp"].includes(mimeType);
  if (!safeMime || data.length > MAX_INLINE_IMAGE_BASE64 || !isBase64(data))
    return (
      <p className="chat-tool-note">
        Image omitted (unsupported or too large).
      </p>
    );
  return (
    <img
      className="chat-tool-image"
      src={`data:${mimeType};base64,${data}`}
      alt={
        typeof value.altText === "string"
          ? value.altText.slice(0, 200)
          : "Image returned by the MCP tool"
      }
      loading="lazy"
      decoding="async"
    />
  );
}

function ToolOutput({ output }: { output: unknown }) {
  if (typeof output === "string") return <pre>{preview(output)}</pre>;
  if (!isRecord(output)) return <pre>{preview(output)}</pre>;

  const content = Array.isArray(output.content) ? output.content : [];
  const hasStructuredContent = output.structuredContent !== undefined;
  const renderedImages = content.filter(
    (item) => isRecord(item) && item.type === "image",
  );
  let imageCount = 0;

  return (
    <div className="chat-tool-output-content">
      {content.map((item, index) => {
        if (!isRecord(item)) return null;
        if (item.type === "text" && typeof item.text === "string")
          return <pre key={index}>{preview(item.text)}</pre>;
        if (item.type === "image") {
          if (imageCount++ >= MAX_INLINE_IMAGES) return null;
          return <InlineImage key={index} value={item} />;
        }
        return null;
      })}
      {renderedImages.length > MAX_INLINE_IMAGES && (
        <p className="chat-tool-note">
          {renderedImages.length - MAX_INLINE_IMAGES} additional images omitted.
        </p>
      )}
      {hasStructuredContent && (
        <>
          <span className="chat-tool-output-label">Structured content</span>
          <pre>{preview(output.structuredContent)}</pre>
        </>
      )}
      {!content.length && !hasStructuredContent && <pre>{preview(output)}</pre>}
    </div>
  );
}

function toolStatus(
  part: DynamicToolUIPart,
  messageStatus: string | undefined,
  busy: boolean,
) {
  if (part.state === "output-error") return "Failed";
  if (
    part.state === "output-available" &&
    isRecord(part.output) &&
    part.output.isError === true
  )
    return "Failed";
  if (part.state === "output-available") return "Completed";
  if (messageStatus === "cancelled") return "Stopped · no result recorded";
  if (messageStatus === "interrupted")
    return "Interrupted · no result recorded";
  if (messageStatus === "failed") return "Failed · no result recorded";
  if (busy && (!messageStatus || messageStatus === "active"))
    return "In progress";
  return "No result recorded";
}

export function ToolPart({
  part,
  messageStatus,
  busy,
}: {
  part: DynamicToolUIPart;
  messageStatus?: string;
  busy: boolean;
}) {
  const [inputOpen, setInputOpen] = useState(false);
  const [resultOpen, setResultOpen] = useState(false);
  const status = toolStatus(part, messageStatus, busy);
  const hasResult =
    part.state === "output-available" || part.state === "output-error";
  const resultIsError =
    part.state === "output-error" ||
    (part.state === "output-available" &&
      isRecord(part.output) &&
      part.output.isError === true);

  return (
    <section className={`chat-tool${resultIsError ? " chat-tool-error" : ""}`}>
      <div className="chat-tool-heading">
        <strong>{part.toolName}</strong>
        <span role="status">{status}</span>
      </div>
      <details onToggle={(event) => setInputOpen(event.currentTarget.open)}>
        <summary>Input</summary>
        {inputOpen && <pre>{preview(part.input)}</pre>}
      </details>
      <details onToggle={(event) => setResultOpen(event.currentTarget.open)}>
        <summary>{resultIsError ? "Error details" : "Result details"}</summary>
        {resultOpen &&
          (part.state === "output-error" ? (
            <pre>{preview(part.errorText)}</pre>
          ) : part.state === "output-available" ? (
            <ToolOutput output={part.output} />
          ) : hasResult ? null : (
            <p className="chat-tool-note">No result has been recorded.</p>
          ))}
      </details>
    </section>
  );
}
