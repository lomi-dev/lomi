import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { api, macOS } from "../api";
import { useT } from "../i18n";
import { ArrowUp, Paperclip, Square } from "../icons";
import { IconButton } from "../ui";
import { PendingAttachments } from "./Attachments";
import { applyMention, mentionQuery, rankFiles } from "./mentions";
import type { Attachment } from "./types";

const FILE_CACHE_MS = 30_000;
const fileCache = new Map<
  string,
  { at: number; files: Promise<readonly string[]> }
>();

/** Per-thread file list, loaded on first use and reused for 30 s. */
function threadFiles(threadId: string): Promise<readonly string[]> {
  const hit = fileCache.get(threadId);
  if (hit && Date.now() - hit.at < FILE_CACHE_MS) return hit.files;
  const files = api<string[]>("ai_thread_files", { threadId }).catch(() => {
    // Do not cache failures.
    if (fileCache.get(threadId)?.files === files) fileCache.delete(threadId);
    return [] as string[];
  });
  fileCache.set(threadId, { at: Date.now(), files });
  return files;
}

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
  permissions,
  placeholder,
  focusKey,
  lastUserText,
  attachments,
  importing,
  attachmentError,
  onPick,
  onPasteImages,
  onRemoveAttachment,
  mentionThreadId,
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
  permissions?: ReactNode;
  placeholder: string;
  /** Changing this refocuses the field (e.g. after Edit and resend). */
  focusKey?: unknown;
  /** ArrowUp in an empty field recalls this text. */
  lastUserText?: string;
  attachments: readonly Attachment[];
  importing: number;
  attachmentError: string;
  onPick: () => void;
  onPasteImages: (files: File[]) => void;
  onRemoveAttachment: (id: string) => void;
  /** Enables `@file` mentions for this (CLI) thread. */
  mentionThreadId?: string | null;
}) {
  const { t } = useT();
  const field = useRef<HTMLTextAreaElement>(null);
  const listId = useId();
  const [caret, setCaret] = useState(0);
  const [loaded, setLoaded] = useState<{
    threadId: string;
    list: readonly string[];
  } | null>(null);
  // Null while the list of the current thread is loading.
  const files =
    loaded && loaded.threadId === mentionThreadId ? loaded.list : null;
  const [dismissed, setDismissed] = useState(-1);
  const [index, setIndex] = useState(0);
  const pendingCaret = useRef<number | null>(null);
  const token = mentionThreadId ? mentionQuery(value, caret) : null;
  const open = !!token && dismissed !== token.start;
  const matches = open && files ? rankFiles(files, token.query) : [];
  const active = Math.min(index, matches.length - 1);
  const mentionLoaded = open && !!mentionThreadId;
  useEffect(() => setDismissed(-1), [mentionThreadId]);
  useEffect(() => {
    if (!mentionLoaded || !mentionThreadId) return;
    let live = true;
    void threadFiles(mentionThreadId).then(
      (list) => live && setLoaded({ threadId: mentionThreadId, list }),
    );
    return () => {
      live = false;
    };
  }, [mentionLoaded, mentionThreadId]);
  useEffect(() => {
    if (active >= 0)
      document
        .getElementById(`${listId}-${active}`)
        ?.scrollIntoView({ block: "nearest" });
  }, [active, open, listId]);
  useLayoutEffect(() => {
    const at = pendingCaret.current;
    if (at === null) return;
    pendingCaret.current = null;
    field.current?.setSelectionRange(at, at);
  }, [value]);
  function insert(path: string) {
    const next = applyMention(value, caret, path);
    pendingCaret.current = next.caret;
    setCaret(next.caret);
    onChange(next.text);
    field.current?.focus();
  }
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
    !disabled && !importing && (!!value.trim() || attachments.length > 0);
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
        role={mentionThreadId ? "combobox" : undefined}
        aria-autocomplete={mentionThreadId ? "list" : undefined}
        aria-haspopup={mentionThreadId ? "listbox" : undefined}
        aria-expanded={mentionThreadId ? open : undefined}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={
          open && active >= 0 ? `${listId}-${active}` : undefined
        }
        onChange={(event) => {
          setCaret(event.target.selectionStart);
          setIndex(0);
          onChange(event.target.value);
        }}
        onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
        onPaste={(event) => {
          const images = Array.from(event.clipboardData.files).filter((file) =>
            file.type.startsWith("image/"),
          );
          if (!images.length) return;
          event.preventDefault();
          onPasteImages(images);
        }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing || event.keyCode === 229) return;
          if (open) {
            const plain = !(
              event.shiftKey ||
              event.altKey ||
              event.ctrlKey ||
              event.metaKey
            );
            if (event.key === "Escape") {
              event.preventDefault();
              setDismissed(token.start);
              return;
            }
            if (matches.length && plain) {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                const step = event.key === "ArrowDown" ? 1 : -1;
                setIndex((active + step + matches.length) % matches.length);
                return;
              }
              if (event.key === "Enter" || event.key === "Tab") {
                event.preventDefault();
                insert(matches[active]);
                return;
              }
            }
          }
          if (event.key === "Escape" && running) {
            event.preventDefault();
            onStop();
            return;
          }
          if (
            event.key === "ArrowUp" &&
            !value &&
            lastUserText &&
            !event.shiftKey &&
            !event.altKey &&
            !event.ctrlKey &&
            !event.metaKey
          ) {
            event.preventDefault();
            onChange(lastUserText);
            return;
          }
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
      {open && (
        <ul
          id={listId}
          role="listbox"
          aria-label={t("Project files")}
          className="thread-mentions"
        >
          {matches.length === 0 && (
            <li className="thread-mention-empty" role="presentation">
              {files ? t("No matching files") : t("Loading files…")}
            </li>
          )}
          {matches.map((path, i) => {
            const slash = path.lastIndexOf("/") + 1;
            return (
              <li
                key={path}
                id={`${listId}-${i}`}
                role="option"
                aria-selected={i === active}
                className="thread-mention"
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => insert(path)}
              >
                <span>{path.slice(slash)}</span>
                <code>{path.slice(0, slash)}</code>
              </li>
            );
          })}
        </ul>
      )}
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
        {permissions}
        <span className="thread-composer-hint">
          {sendKey === "enter"
            ? t("Enter to send")
            : t("{modifier} Enter to send", { modifier })}
        </span>
        {running && (
          <IconButton
            title={t("Stop")}
            className="thread-send thread-stop"
            onClick={onStop}
          >
            <Square size={12} fill="currentColor" />
          </IconButton>
        )}
        <IconButton
          title={running ? t("Add to queue") : t("Send")}
          className="thread-send"
          disabled={!canSend}
          onClick={onSend}
        >
          <ArrowUp size={16} />
        </IconButton>
      </div>
    </div>
  );
}
