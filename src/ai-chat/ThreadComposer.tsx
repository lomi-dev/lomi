import { Document } from "@tiptap/extension-document";
import { HardBreak } from "@tiptap/extension-hard-break";
import { Paragraph } from "@tiptap/extension-paragraph";
import { Text } from "@tiptap/extension-text";
import { Placeholder, UndoRedo } from "@tiptap/extensions";
import type { Transaction } from "@tiptap/pm/state";
import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { api, macOS } from "../api";
import { useT } from "../i18n";
import { ArrowUp, ListPlus, Paperclip, Square } from "../icons";
import { IconButton } from "../ui";
import { PendingAttachments } from "./Attachments";
import {
  commandDescription,
  groupCommands,
  rankCommands,
  slashQuery,
  splitCommandName,
} from "./commands";
import ComposerQuestion from "./ComposerQuestion";
import { docToText, textToDoc } from "./composer-doc";
import { FileMention, SlashCommand } from "./composer-nodes";
import { mentionQuery, rankFiles } from "./mentions";
import { spaceForInsert } from "../stt/stt-insert";
import { threadCommands, type CommandSource } from "./thread-runtime";
import type { Attachment, Question, ThreadCommand } from "./types";

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
  commandSource,
  commandsKey,
  question,
  onAnswer,
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
  /** CLI agent whose slash commands are listed besides the built-ins. */
  commandSource?: CommandSource | null;
  /** Changing this reloads the command list (e.g. after a completed turn). */
  commandsKey?: string;
  /** Pending agent question; the composer shows it in place of the editor. */
  question?: Question | null;
  onAnswer: (
    requestId: string,
    answers: Record<string, string[]> | null,
  ) => Promise<void>;
}) {
  const { t } = useT();
  const listId = useId();
  // Serialized draft up to the selection start, mapped from the editor document.
  const [{ text, caret }, setCtx] = useState({
    text: value,
    caret: value.length,
  });
  const [loaded, setLoaded] = useState<{
    threadId: string;
    list: readonly string[];
  } | null>(null);
  // Null while the list of the current thread is loading.
  const files =
    loaded && loaded.threadId === mentionThreadId ? loaded.list : null;
  const [dismissed, setDismissed] = useState(-1);
  const [index, setIndex] = useState(0);
  const mention = mentionThreadId ? mentionQuery(text, caret) : null;
  const slash = slashQuery(text, caret);
  const start = slash !== null ? 0 : mention?.start;
  const open = start !== undefined && dismissed !== start && !question;
  // A dismissal ends with its token, so the next `/` or `@` opens the popup again.
  useEffect(() => {
    if (start === undefined) setDismissed(-1);
  }, [start]);
  const [commands, setCommands] = useState<{
    key: string;
    list: readonly ThreadCommand[];
  } | null>(null);
  const sourceKey = commandSource
    ? `${commandSource.threadId}:${commandSource.instanceId}:${commandSource.project}`
    : "";
  const commandKey = `${sourceKey}:${commandsKey}`;
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
  interface Match {
    key: string;
    label: string;
    /** Skill plugin, shown as a badge. */
    badge?: string;
    /** Muted monospace suffix after the label. */
    args?: string;
    hint: string;
    exact: boolean;
    run: () => void;
    /** Title of the section this row opens. */
    section?: string;
  }
  const sectionTitles = {
    lomi: t("Lomi"),
    commands: t("Commands"),
    skills: t("Skills"),
  };
  const matches: Match[] = !open
    ? []
    : slash !== null
      ? groupCommands(rankCommands(commandList, slash)).flatMap((group) =>
          group.items.map((command, i) => {
            const { plugin, base } = splitCommandName(command.name);
            return {
              key: command.name,
              label: `/${base}`,
              badge: plugin,
              args: command.argumentHint,
              hint: commandDescription(command.name, command.description),
              exact: command.name === slash,
              run: () =>
                insertPill(`/${slash}`, "slashCommand", {
                  name: command.name,
                }),
              section: i === 0 ? sectionTitles[group.id] : undefined,
            };
          }),
        )
      : files && mention
        ? rankFiles(files, mention.query).map((path) => {
            const slashAt = path.lastIndexOf("/") + 1;
            return {
              key: path,
              label: path.slice(slashAt),
              hint: path.slice(0, slashAt),
              exact: false,
              run: () =>
                insertPill(`@${mention.query}`, "fileMention", { path }),
            };
          })
        : [];
  const active = Math.min(index, matches.length - 1);
  // Consecutive rows from one section (files form a single untitled one).
  const sections = matches.reduce<{ match: Match; i: number }[][]>(
    (acc, match, i) => {
      if (match.section || !acc.length) acc.push([]);
      acc[acc.length - 1].push({ match, i });
      return acc;
    },
    [],
  );
  const mentionLoaded = open && slash === null && !!mentionThreadId;
  const commandsWanted = open && slash !== null && !!commandSource;
  useEffect(() => {
    if (!commandsWanted || !commandSource) return;
    let live = true;
    const done = (list: readonly ThreadCommand[]) =>
      live && setCommands({ key: commandKey, list });
    threadCommands(commandSource).then(done, () => done([]));
    return () => {
      live = false;
    };
    // `commandKey` covers `commandSource`'s fields.
  }, [commandsWanted, commandKey]);
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
  function refresh(source: Editor) {
    const { doc, selection } = source.state;
    const next = {
      text: docToText(doc.toJSON()),
      // Serializing the prefix yields a prefix of the full text.
      caret: docToText(doc.cut(0, selection.from).toJSON()).length,
    };
    setCtx((prev) =>
      prev.text === next.text && prev.caret === next.caret ? prev : next,
    );
  }
  /** Replaces the typed `token` before the caret with a pill and a space. */
  function insertPill(
    token: string,
    type: string,
    attrs: Record<string, string>,
  ) {
    if (!editor) return;
    const to = editor.state.selection.from;
    const typed = editor.state.doc.textBetween(to - token.length, to) === token;
    editor
      .chain()
      .focus()
      .insertContentAt({ from: typed ? to - token.length : to, to }, [
        { type, attrs },
        { type: "text", text: " " },
      ])
      .run();
  }
  function onPaste(event: ClipboardEvent) {
    const data = event.clipboardData;
    if (!data) return false;
    const images = Array.from(data.files).filter((file) =>
      file.type.startsWith("image/"),
    );
    if (images.length) {
      event.preventDefault();
      onPasteImages(images);
      return true;
    }
    const plain = data.getData("text/plain").replace(/\r\n?/g, "\n");
    if (!plain) return false;
    // Plain text only: drop any HTML formatting from the clipboard.
    event.preventDefault();
    editor?.commands.insertContent(
      plain
        .split("\n")
        .flatMap((line, i) => [
          ...(i ? [{ type: "hardBreak" }] : []),
          ...(line ? [{ type: "text", text: line }] : []),
        ]),
    );
    return true;
  }
  function onKey(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return false;
    const plainKey = !(
      event.shiftKey ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey
    );
    if (open) {
      if (event.key === "Escape") {
        event.preventDefault();
        setDismissed(start);
        return true;
      }
      if (matches.length && plainKey) {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
          event.preventDefault();
          const step = event.key === "ArrowDown" ? 1 : -1;
          setIndex((active + step + matches.length) % matches.length);
          return true;
        }
        // A fully typed command is sent, not completed again.
        const send = event.key === "Enter" && matches[active].exact;
        if ((event.key === "Enter" || event.key === "Tab") && !send) {
          event.preventDefault();
          matches[active].run();
          return true;
        }
      }
    }
    if (event.key === "Escape" && running) {
      event.preventDefault();
      onStop();
      return true;
    }
    if (event.key === "Backspace" && editor?.state.selection.empty) {
      // Pills are atoms: delete one as a unit.
      const { $from } = editor.state.selection;
      if ($from.nodeBefore?.type.spec.atom) {
        editor.view.dispatch(
          editor.state.tr.delete(
            $from.pos - $from.nodeBefore.nodeSize,
            $from.pos,
          ),
        );
        return true;
      }
    }
    if (event.key === "ArrowUp" && !text && lastUserText && plainKey) {
      event.preventDefault();
      onChange(lastUserText);
      return true;
    }
    if (event.key !== "Enter" || event.shiftKey) return false;
    const mod = macOS ? event.metaKey : event.ctrlKey;
    // Alt+Enter queues a message in both send modes.
    const queue = running && event.altKey;
    if (
      sendKey === "enter" ? mod || (event.altKey && !running) : !mod && !queue
    )
      return false;
    event.preventDefault();
    if (canSend) onSend(queue);
    return true;
  }
  // The editor reads its handlers through this ref, always from the last render.
  const live = useRef({ onKey, onPaste, onChange, placeholder });
  live.current = { onKey, onPaste, onChange, placeholder };
  const editor = useEditor({
    immediatelyRender: true,
    shouldRerenderOnTransaction: false,
    extensions: [
      Document,
      Paragraph,
      Text,
      HardBreak,
      UndoRedo,
      FileMention,
      SlashCommand,
      Placeholder.configure({ placeholder: () => live.current.placeholder }),
    ],
    content: textToDoc(value),
    editorProps: {
      attributes: {
        role: "combobox",
        "aria-label": t("Message"),
        "aria-autocomplete": "list",
        "aria-haspopup": "listbox",
        "aria-expanded": String(open),
        ...(open && { "aria-controls": listId }),
        ...(open &&
          active >= 0 && { "aria-activedescendant": `${listId}-${active}` }),
      },
      handleKeyDown: (_view, event) => live.current.onKey(event),
      handlePaste: (_view, event) => live.current.onPaste(event),
    },
    onUpdate: ({ editor: source }) => {
      setIndex(0);
      refresh(source);
      live.current.onChange(docToText(source.getJSON()));
    },
    onSelectionUpdate: ({ editor: source }) => refresh(source),
  });
  // External changes (send, history recall, queued edit) replace the content.
  useEffect(() => {
    if (!editor || value === docToText(editor.getJSON())) return;
    editor.commands.setContent(textToDoc(value), { emitUpdate: false });
    editor.commands.setTextSelection(editor.state.doc.content.size - 1);
    refresh(editor);
  }, [editor, value]);
  useLayoutEffect(() => {
    if (focusKey !== undefined) editor?.commands.focus("end");
  }, [focusKey]);
  const root = useRef<HTMLDivElement>(null);
  // Dictation results (src/stt/stt-runtime.ts) arrive as sanitized plain text.
  useEffect(() => {
    // Not editor.view: Tiptap throws when it is read before the view mounts.
    const host = root.current?.closest("[data-thread-tab-id]");
    if (!editor || !host) return;
    // Where the cursor was when dictation started, mapped through every edit
    // made while recording, so typing on does not move the insertion point.
    let anchor: { from: number; to: number } | undefined;
    const track = ({ transaction }: { transaction: Transaction }) => {
      if (anchor)
        anchor = {
          from: transaction.mapping.map(anchor.from, -1),
          to: transaction.mapping.map(anchor.to, 1),
        };
    };
    const start = () => {
      const { from, to } = editor.state.selection;
      anchor = { from, to };
    };
    const listener = (event: Event) => {
      const text = (event as CustomEvent<string>).detail;
      const doc = editor.state.doc;
      // A replaced draft (sent message, history recall) can shrink the doc.
      const end = doc.content.size;
      const at = anchor ?? editor.state.selection;
      anchor = undefined;
      const from = Math.min(at.from, end);
      const to = Math.min(Math.max(at.to, from), end);
      const before = doc.textBetween(Math.max(0, from - 1), from);
      const after = doc.textBetween(to, Math.min(doc.content.size, to + 1));
      // A text node, never an HTML string: dictated "<b>" stays literal.
      editor
        .chain()
        .focus()
        .insertContentAt(
          { from, to },
          { type: "text", text: spaceForInsert(before, text, after) },
        )
        .run();
    };
    editor.on("transaction", track);
    host.addEventListener("thread-dictation-start", start);
    host.addEventListener("thread-dictation", listener);
    return () => {
      editor.off("transaction", track);
      host.removeEventListener("thread-dictation-start", start);
      host.removeEventListener("thread-dictation", listener);
    };
  }, [editor]);
  const canSend =
    !disabled && !importing && (!!value.trim() || attachments.length > 0);
  const modifier = macOS ? "⌘" : "Ctrl";
  const lead = (
    <>
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
    </>
  );
  // Back to the editor once the question is gone (its panel held the focus).
  const asked = useRef(false);
  useEffect(() => {
    const active = document.activeElement;
    if (asked.current && !question && (!active || active === document.body))
      editor?.commands.focus("end");
    asked.current = !!question;
  }, [question, editor]);
  return (
    <div className="thread-composer" ref={root}>
      {question && (
        <ComposerQuestion
          key={question.requestId}
          question={question}
          onAnswer={onAnswer}
        />
      )}
      {/* Kept mounted while asking so the draft survives. */}
      <div className="thread-composer-editor" hidden={!!question}>
        <PendingAttachments
          items={attachments}
          importing={importing}
          error={attachmentError}
          onRemove={onRemoveAttachment}
        />
        <EditorContent editor={editor} />
      </div>
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
          {sections.map((rows) => (
            <li key={rows[0].match.key} role="presentation">
              <ul
                role="group"
                aria-label={rows[0].match.section}
                className="thread-mention-group"
              >
                {rows[0].match.section && (
                  <li
                    className="thread-mention-section"
                    role="presentation"
                    aria-hidden="true"
                  >
                    {rows[0].match.section}
                  </li>
                )}
                {rows.map(({ match, i }) => (
                  <li
                    key={match.key}
                    id={`${listId}-${i}`}
                    role="option"
                    aria-selected={i === active}
                    className="thread-mention"
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={match.run}
                  >
                    <span className="thread-mention-name">{match.label}</span>
                    {match.badge && (
                      <small className="thread-mention-badge">
                        {match.badge}
                      </small>
                    )}
                    {match.args && <code>{match.args}</code>}
                    <span
                      className="thread-mention-hint"
                      data-path={slash === null || undefined}
                    >
                      {match.hint}
                    </span>
                  </li>
                ))}
              </ul>
            </li>
          ))}
          {slash !== null && (
            <li className="thread-mention-footer" role="presentation">
              {t("Type to filter · ↑↓ to move · Enter to insert")}
            </li>
          )}
        </ul>
      )}
      {!question && (
        <div className="thread-composer-bar">
          {lead}
          <span className="thread-composer-spacer" />
          {running ? (
            <>
              <IconButton
                title={t("Add to queue ({shortcut})", {
                  shortcut: "Alt+Enter",
                })}
                disabled={!canSend}
                onClick={() => onSend(true)}
              >
                <ListPlus size={16} />
              </IconButton>
              <IconButton
                title={t("Stop ({shortcut})", { shortcut: "Esc" })}
                className="thread-send thread-stop"
                onClick={onStop}
              >
                <Square size={12} fill="currentColor" />
              </IconButton>
            </>
          ) : (
            <IconButton
              title={t("Send ({shortcut})", {
                shortcut: sendKey === "enter" ? "Enter" : `${modifier} Enter`,
              })}
              className="thread-send"
              disabled={!canSend}
              onClick={() => onSend(false)}
            >
              <ArrowUp size={16} />
            </IconButton>
          )}
        </div>
      )}
    </div>
  );
}
