import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { api, macOS } from "../api";
import { useT } from "../i18n";
import { ArrowUp, ListPlus, Paperclip, Square } from "../icons";
import { IconButton } from "../ui";
import { PendingAttachments } from "./Attachments";
import { applyCommand, rankCommands, slashQuery } from "./commands";
import { applyMention, mentionQuery, rankFiles } from "./mentions";
import { threadCommands } from "./thread-runtime";
import type { Attachment, ThreadCommand } from "./types";

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
  steerable,
  commandThreadId,
  commandsKey,
}: {
  value: string;
  onChange: (value: string) => void;
  /** `queue` forces queueing instead of steering into the running reply. */
  onSend: (queue: boolean) => void;
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
  /** While running, plain send steers into the reply (Alt+Enter queues). */
  steerable?: boolean;
  /** CLI thread whose slash commands are listed besides the built-ins. */
  commandThreadId?: string | null;
  /** Changing this reloads the command list (e.g. after a completed turn). */
  commandsKey?: string;
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
  const mention = mentionThreadId ? mentionQuery(value, caret) : null;
  const slash = slashQuery(value, caret);
  const start = slash !== null ? 0 : mention?.start;
  const open = start !== undefined && dismissed !== start;
  // A dismissal ends with its token, so the next `/` or `@` opens the popup again.
  useEffect(() => {
    if (start === undefined) setDismissed(-1);
  }, [start]);
  const [commands, setCommands] = useState<{
    key: string;
    list: readonly ThreadCommand[];
  } | null>(null);
  const commandKey = `${commandThreadId}:${commandsKey}`;
  const builtins: ThreadCommand[] = [
    { name: "new", description: t("Start a new chat"), argumentHint: "" },
  ];
  const commandList = [
    ...builtins,
    ...(commands?.key === commandKey ? commands.list : []).filter(
      (command) => !builtins.some((item) => item.name === command.name),
    ),
  ];
  // Popup rows: label, muted hint and the insertion they perform.
  const matches: {
    key: string;
    label: string;
    hint: string;
    exact: boolean;
    run: () => void;
  }[] = !open
    ? []
    : slash !== null
      ? rankCommands(commandList, slash).map((command) => ({
          key: command.name,
          label: `/${command.name}`,
          hint: [command.argumentHint, command.description]
            .filter(Boolean)
            .join(" "),
          exact: command.name === slash,
          run: () => insertCommand(command.name),
        }))
      : files && mention
        ? rankFiles(files, mention.query).map((path) => {
            const slashAt = path.lastIndexOf("/") + 1;
            return {
              key: path,
              label: path.slice(slashAt),
              hint: path.slice(0, slashAt),
              exact: false,
              run: () => insert(path),
            };
          })
        : [];
  const active = Math.min(index, matches.length - 1);
  const mentionLoaded = open && slash === null && !!mentionThreadId;
  const commandsWanted = open && slash !== null && !!commandThreadId;
  useEffect(() => {
    if (!commandsWanted || !commandThreadId) return;
    let live = true;
    const done = (list: readonly ThreadCommand[]) =>
      live && setCommands({ key: commandKey, list });
    threadCommands(commandThreadId).then(done, () => done([]));
    return () => {
      live = false;
    };
  }, [commandsWanted, commandThreadId, commandKey]);
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
  function insertCommand(name: string) {
    const next = applyCommand(value, caret, name);
    pendingCaret.current = next.caret;
    setCaret(next.caret);
    onChange(next.text);
    field.current?.focus();
  }
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
        role="combobox"
        aria-autocomplete="list"
        aria-haspopup="listbox"
        aria-expanded={open}
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
              setDismissed(start);
              return;
            }
            if (matches.length && plain) {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                const step = event.key === "ArrowDown" ? 1 : -1;
                setIndex((active + step + matches.length) % matches.length);
                return;
              }
              // A fully typed command is sent, not completed again.
              const send = event.key === "Enter" && matches[active].exact;
              if ((event.key === "Enter" || event.key === "Tab") && !send) {
                event.preventDefault();
                matches[active].run();
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
          // Alt+Enter queues a message in both send modes.
          const queue = running && event.altKey;
          if (
            sendKey === "enter"
              ? mod || (event.altKey && !running)
              : !mod && !queue
          )
            return;
          event.preventDefault();
          if (canSend) onSend(queue);
        }}
      />
      {open && (
        <ul
          id={listId}
          role="listbox"
          aria-label={slash !== null ? t("Slash commands") : t("Project files")}
          className="thread-mentions"
        >
          {matches.length === 0 && (
            <li className="thread-mention-empty" role="presentation">
              {slash !== null
                ? t("No matching commands")
                : files
                  ? t("No matching files")
                  : t("Loading files…")}
            </li>
          )}
          {matches.map((match, i) => (
            <li
              key={match.key}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              className="thread-mention"
              onMouseDown={(event) => event.preventDefault()}
              onClick={match.run}
            >
              <span>{match.label}</span>
              <code data-slash={slash !== null || undefined}>{match.hint}</code>
            </li>
          ))}
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
          {running && steerable
            ? t("Sends to the running agent")
            : sendKey === "enter"
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
        {running && steerable && (
          <IconButton
            title={t("Add to queue ({shortcut})", { shortcut: "Alt+Enter" })}
            disabled={!canSend}
            onClick={() => onSend(true)}
          >
            <ListPlus size={16} />
          </IconButton>
        )}
        <IconButton
          title={
            running
              ? steerable
                ? t("Send now (steer)")
                : t("Add to queue")
              : t("Send")
          }
          className="thread-send"
          disabled={!canSend}
          onClick={() => onSend(false)}
        >
          <ArrowUp size={16} />
        </IconButton>
      </div>
    </div>
  );
}
