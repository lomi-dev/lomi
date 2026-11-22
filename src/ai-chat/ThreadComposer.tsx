import { useLayoutEffect, useRef } from "react";
import type { ReactNode } from "react";
import { macOS } from "../api";
import { useT } from "../i18n";
import { ArrowUp, Paperclip, Square } from "../icons";
import { IconButton } from "../ui";
import { PendingAttachments } from "./Attachments";
import type { Attachment } from "./types";

export default function ThreadComposer({
  value,
  onChange,
  onSend,
  onStop,
  running,
  disabled,
  sendKey,
  picker,
  effort,
  placeholder,
  focusKey,
  attachments,
  importing,
  attachmentError,
  onPick,
  onPasteImages,
  onRemoveAttachment,
}: {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  onStop: () => void;
  running: boolean;
  disabled: boolean;
  sendKey: "enter" | "mod-enter";
  picker: ReactNode;
  effort?: ReactNode;
  placeholder: string;
  /** Changing this refocuses the field (e.g. after Edit and resend). */
  focusKey?: unknown;
  attachments: readonly Attachment[];
  importing: number;
  attachmentError: string;
  onPick: () => void;
  onPasteImages: (files: File[]) => void;
  onRemoveAttachment: (id: string) => void;
}) {
  const { t } = useT();
  const field = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const element = field.current;
    if (!element) return;
    element.style.height = "auto";
    // A pane mounted while hidden measures 0; keep the CSS height instead of pinning 0px.
    if (element.scrollHeight > 0)
      element.style.height = `${Math.min(element.scrollHeight, 220)}px`;
  }, [value]);
  useLayoutEffect(() => {
    const element = field.current;
    if (!element || focusKey === undefined) return;
    element.focus();
    element.setSelectionRange(element.value.length, element.value.length);
  }, [focusKey]);
  const canSend =
    !disabled &&
    !running &&
    !importing &&
    (!!value.trim() || attachments.length > 0);
  const modifier = macOS ? "⌘" : "Ctrl";
  return (
    <div className="thread-composer">
      <PendingAttachments
        items={attachments}
        importing={importing}
        error={attachmentError}
        onRemove={onRemoveAttachment}
      />
      <textarea
        ref={field}
        rows={1}
        value={value}
        placeholder={placeholder}
        aria-label={t("Message")}
        onChange={(event) => onChange(event.target.value)}
        onPaste={(event) => {
          const images = Array.from(event.clipboardData.files).filter((file) =>
            file.type.startsWith("image/"),
          );
          if (!images.length) return;
          event.preventDefault();
          onPasteImages(images);
        }}
        onKeyDown={(event) => {
          if (
            event.key !== "Enter" ||
            event.shiftKey ||
            event.nativeEvent.isComposing ||
            event.keyCode === 229
          )
            return;
          const mod = macOS ? event.metaKey : event.ctrlKey;
          if (sendKey === "enter" ? event.altKey || mod : !mod) return;
          event.preventDefault();
          if (canSend) onSend();
        }}
      />
      <div className="thread-composer-bar">
        <IconButton
          title={t("Attach files")}
          disabled={disabled}
          onClick={onPick}
        >
          <Paperclip size={16} />
        </IconButton>
        {picker}
        {effort}
        <span className="thread-composer-hint">
          {sendKey === "enter"
            ? t("Enter to send")
            : t("{modifier} Enter to send", { modifier })}
        </span>
        {running ? (
          <IconButton
            title={t("Stop")}
            className="thread-send thread-stop"
            onClick={onStop}
          >
            <Square size={12} fill="currentColor" />
          </IconButton>
        ) : (
          <IconButton
            title={t("Send")}
            className="thread-send"
            disabled={!canSend}
            onClick={onSend}
          >
            <ArrowUp size={16} />
          </IconButton>
        )}
      </div>
    </div>
  );
}
