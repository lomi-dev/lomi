import {
  Children,
  createContext,
  isValidElement,
  memo,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ReactNode, RefObject } from "react";
import Markdown from "react-markdown";
import type { Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { emit } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { errorMessage } from "../api";
import { useT } from "../i18n";
import { Modal } from "../ui";
import {
  Check,
  ChevronRight,
  Circle,
  CircleAlert,
  Copy,
  FileDiff,
  FileText,
  Globe,
  Info,
  Layers,
  MessageSquare,
  Pencil,
  Puzzle,
  Search,
  Terminal,
  Undo2,
} from "../icons";
import { MessageAttachments } from "./Attachments";
import { highlightCode } from "./code-highlight";
import type { HighlightSpan } from "./code-highlight";
import { splitBlocks } from "./markdown-blocks";
import {
  INTERRUPTED_ERROR,
  turnDiff,
  undoTurn,
  undoTurnFiles,
  type ChatItemView,
} from "./thread-runtime";
import { parseUnifiedDiff } from "./unified-diff";
import {
  collapseGroup,
  compactNumber,
  formatCountdown,
  isPending,
  isPendingQuestion,
  interleave,
  lineDiff,
  relativeTitle,
} from "./activity";
import type {
  Activity,
  ActivityKind,
  ApprovalDecision,
  Attachment,
  FileChange,
  QuestionItem,
  TurnChanges,
  TurnUndoResult,
  Usage,
} from "./types";

const plugins = [remarkGfm];
/** True while the block being rendered may still grow. */
const StreamingBlock = createContext(false);

function CodeBlock({ children }: { children?: ReactNode }) {
  const { t } = useT();
  const [copied, setCopied] = useState(false);
  const code = Children.toArray(children).find(isValidElement) as
    { props: { className?: string; children?: ReactNode } } | undefined;
  const language = /language-([\w-]+)/.exec(code?.props.className ?? "")?.[1];
  const source = String(code?.props.children ?? "").replace(/\n$/, "");
  const streaming = useContext(StreamingBlock);
  const [spans, setSpans] = useState<{
    source: string;
    spans: HighlightSpan[];
  } | null>(null);
  useEffect(() => {
    let stale = false;
    // A growing block is highlighted only once its text stops changing.
    const timer = setTimeout(
      () =>
        void highlightCode(source, language).then((result) => {
          if (!stale && result) setSpans({ source, spans: result });
        }),
      streaming ? 150 : 0,
    );
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [source, language, streaming]);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <div className="thread-code">
      <div className="thread-code-header">
        <span>{language ?? "text"}</span>
        <button
          type="button"
          className="thread-code-copy"
          onClick={() =>
            void navigator.clipboard.writeText(source).then(
              () => setCopied(true),
              () => {},
            )
          }
        >
          {copied ? <Check size={13} /> : <Copy size={13} />}
          {copied ? t("Copied") : t("Copy")}
        </button>
      </div>
      <pre>
        {spans?.source === source ? (
          <code>
            {spans.spans.map((span, index) => (
              <span key={index} className={span.className}>
                {span.text}
              </span>
            ))}
          </code>
        ) : (
          children
        )}
      </pre>
    </div>
  );
}

const components: Components = {
  pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
  a: ({ href, title, children }) =>
    href && /^https?:\/\//i.test(href) ? (
      <a
        href={href}
        title={title}
        onClick={(event) => {
          event.preventDefault();
          void openUrl(href).catch(() => {});
        }}
      >
        {children}
      </a>
    ) : (
      <span>{children}</span>
    ),
  img: ({ alt }) => <span>{alt}</span>,
};

const Block = memo(function Block({ text }: { text: string }) {
  return (
    <Markdown remarkPlugins={plugins} skipHtml components={components}>
      {text}
    </Markdown>
  );
});

/** Splits at blank lines so only the growing last block re-parses per delta. */
const Markup = memo(function Markup({
  text,
  streaming = false,
}: {
  text: string;
  streaming?: boolean;
}) {
  const blocks = splitBlocks(text);
  return blocks.map((block, index) => (
    <StreamingBlock
      key={index}
      value={streaming && index === blocks.length - 1}
    >
      <Block text={block} />
    </StreamingBlock>
  ));
});

const kindIcons: Record<ActivityKind, typeof Terminal> = {
  reasoning: MessageSquare,
  read: FileText,
  edit: Pencil,
  search: Search,
  execute: Terminal,
  fetch: Globe,
  agent: Layers,
  notice: Info,
  question: MessageSquare,
  other: Puzzle,
};

interface ApprovalContext {
  respond: (requestId: string, decision: ApprovalDecision) => Promise<void>;
  /** Answers a pending question by item id; null declines it. */
  answer: (
    requestId: string,
    answers: Record<string, string[]> | null,
  ) => Promise<void>;
  /** requestId of the first pending card; it may take focus on appearing. */
  focusId: string | null;
}
const Approvals = createContext<ApprovalContext>({
  respond: () => Promise.resolve(),
  answer: () => Promise.resolve(),
  focusId: null,
});

/** Hands focus back to the composer when a card that held it disappears. */
function useFocusBack(card: RefObject<HTMLElement | null>) {
  useLayoutEffect(() => {
    const element = card.current;
    return () => {
      if (!element?.contains(document.activeElement)) return;
      element
        .closest(".thread-pane")
        ?.querySelector<HTMLTextAreaElement>(".thread-composer textarea")
        ?.focus();
    };
  }, [card]);
}

function ApprovalCard({ activity, cwd }: { activity: Activity; cwd: string }) {
  const { t } = useT();
  const { respond, focusId } = useContext(Approvals);
  const approval = activity.approval!;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const allow = useRef<HTMLButtonElement>(null);
  const card = useRef<HTMLLIElement>(null);
  const Icon = kindIcons[activity.kind];
  const diff = activity.diff
    ? lineDiff(activity.diff.before, activity.diff.after)
    : [];
  useEffect(() => {
    if (focusId !== approval.requestId) return;
    const active = document.activeElement;
    // Never steal focus from the composer, other panels or this pane: a
    // pending Enter keypress must not approve a command.
    if (!active || active === document.body) allow.current?.focus();
  }, [focusId, approval.requestId]);
  useFocusBack(card);
  const answer = (decision: ApprovalDecision) => {
    setBusy(true);
    setError("");
    respond(approval.requestId, decision).then(
      () => setBusy(false),
      (cause) => {
        setBusy(false);
        setError(errorMessage(cause));
      },
    );
  };
  return (
    <li
      ref={card}
      className="thread-activity-item thread-approval"
      role="group"
      aria-label={t("Permission request")}
    >
      <div className="thread-approval-label">{t("Permission request")}</div>
      <div className="thread-approval-head">
        <Icon size={13} aria-hidden="true" />
        <span className="thread-activity-title" title={activity.title}>
          {relativeTitle(activity.title, cwd)}
        </span>
      </div>
      {activity.diff && (
        <div className="thread-diff">
          <div className="thread-diff-path">{activity.diff.path}</div>
          <pre>
            {diff.map((line, index) => (
              <div
                key={index}
                className="thread-diff-line"
                data-sign={line.sign}
              >
                {line.sign === "-" ? "\u2212" : "+"} {line.text}
              </div>
            ))}
          </pre>
        </div>
      )}
      {activity.detail && (
        <pre className="thread-activity-detail">{activity.detail}</pre>
      )}
      <div className="thread-approval-actions">
        <button
          ref={allow}
          type="button"
          className="button button-primary"
          disabled={busy}
          onClick={() => answer("allow")}
        >
          {t("Allow")}
        </button>
        {approval.canRemember && (
          <button
            type="button"
            className="button"
            disabled={busy}
            onClick={() => answer("allow_session")}
          >
            {t("Allow for this session")}
          </button>
        )}
        <button
          type="button"
          className="button"
          disabled={busy}
          onClick={() => answer("deny")}
        >
          {t("Deny")}
        </button>
      </div>
      {error && (
        <div className="thread-notice" role="alert">
          {error}
        </div>
      )}
    </li>
  );
}

// Secret answers (passwords) are masked and never echoed back.
const isSecret = (item: QuestionItem) => !!item.secret;

/** Pending question: one group per item with options and optional free text. */
function QuestionCard({ activity }: { activity: Activity }) {
  const { t } = useT();
  const { answer, focusId } = useContext(Approvals);
  const question = activity.question!;
  const name = useId();
  const [chosen, setChosen] = useState<Record<string, string[]>>({});
  const [other, setOther] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const card = useRef<HTMLLIElement>(null);
  useFocusBack(card);
  useEffect(() => {
    if (focusId !== question.requestId) return;
    const active = document.activeElement;
    // Same rule as approvals: never steal focus from the composer or other panels.
    if (!active || active === document.body)
      card.current?.querySelector<HTMLElement>("input")?.focus();
  }, [focusId, question.requestId]);
  const answers = Object.fromEntries(
    question.items.map((item) => [
      item.id,
      [...(chosen[item.id] ?? []), other[item.id]?.trim() ?? ""].filter(
        Boolean,
      ),
    ]),
  );
  const ready = question.items.every((item) => answers[item.id].length > 0);
  const submit = (value: Record<string, string[]> | null) => {
    setBusy(true);
    setError("");
    answer(question.requestId, value).then(
      () => setBusy(false),
      (cause) => {
        setBusy(false);
        setError(errorMessage(cause));
      },
    );
  };
  return (
    <li
      ref={card}
      className="thread-activity-item thread-approval"
      role="group"
      aria-label={t("Question from the agent")}
    >
      <div className="thread-approval-label">
        {t("Question from the agent")}
      </div>
      {question.items.map((item) => {
        const group = `${name}-${item.id}`;
        const type = item.multiSelect ? "checkbox" : "radio";
        return (
          <fieldset key={item.id} className="thread-question-item">
            <legend>
              {item.header && (
                <span className="thread-question-header">{item.header}</span>
              )}
              {item.question}
            </legend>
            {item.options.map((option) => (
              <label key={option.label} className="thread-question-option">
                <input
                  type={type}
                  name={group}
                  disabled={busy}
                  checked={(chosen[item.id] ?? []).includes(option.label)}
                  onChange={(event) => {
                    const on = event.target.checked;
                    setChosen((prev) => {
                      const had = prev[item.id] ?? [];
                      return {
                        ...prev,
                        [item.id]: item.multiSelect
                          ? on
                            ? [...had, option.label]
                            : had.filter((label) => label !== option.label)
                          : [option.label],
                      };
                    });
                    if (!item.multiSelect)
                      setOther((prev) => ({ ...prev, [item.id]: "" }));
                  }}
                />
                <span>
                  {option.label}
                  {option.description && <small>{option.description}</small>}
                </span>
              </label>
            ))}
            {(item.allowOther || !item.options.length) && (
              <input
                type={isSecret(item) ? "password" : "text"}
                autoComplete="off"
                className="thread-question-other"
                aria-label={t("Other answer")}
                placeholder={t("Other…")}
                disabled={busy}
                value={other[item.id] ?? ""}
                onChange={(event) => {
                  setOther((prev) => ({
                    ...prev,
                    [item.id]: event.target.value,
                  }));
                  if (!item.multiSelect)
                    setChosen((prev) => ({ ...prev, [item.id]: [] }));
                }}
              />
            )}
          </fieldset>
        );
      })}
      <div className="thread-approval-actions">
        <button
          type="button"
          className="button button-primary"
          disabled={busy || !ready}
          onClick={() => submit(answers)}
        >
          {t("Submit")}
        </button>
        <button
          type="button"
          className="button"
          disabled={busy}
          onClick={() => submit(null)}
        >
          {t("Decline")}
        </button>
      </div>
      {error && (
        <div className="thread-notice" role="alert">
          {error}
        </div>
      )}
    </li>
  );
}

/** Answered, declined or expired question, one compact line per item. */
function QuestionSummary({ activity }: { activity: Activity }) {
  const { t } = useT();
  const question = activity.question!;
  const state =
    question.state === "answered"
      ? t("Answered")
      : question.state === "declined"
        ? t("Declined")
        : t("Expired");
  return (
    <li className="thread-activity-item thread-info-item">
      {question.items.map((item) => (
        <div key={item.id} className="thread-info-row">
          <MessageSquare size={13} aria-hidden="true" />
          <span className="thread-question-asked">{item.question}</span>
          {isSecret(item)
            ? question.state === "answered" && <strong>••••</strong>
            : !!question.answers?.[item.id]?.length && (
                <strong>{question.answers[item.id].join(", ")}</strong>
              )}
        </div>
      ))}
      <div className="thread-info-row">
        <span
          className="thread-approval-state"
          data-state={question.state === "answered" ? undefined : "denied"}
        >
          {state}
        </span>
      </div>
    </li>
  );
}

/** System note (stable machine title) shown as a subtle translated info row. */
function NoticeRow({ activity }: { activity: Activity }) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  // A message sent into the running turn reads like a compact user message.
  if (activity.title === "steer")
    return (
      <li className="thread-activity-item thread-steer">
        <span className="thread-steer-label">{t("Sent to the agent")}</span>
        <span className="thread-steer-text">{activity.detail}</span>
      </li>
    );
  const text =
    activity.title === "context_reset"
      ? t(
          "The previous session could not be resumed, so the agent started fresh without earlier context.",
        )
      : activity.title === "resume_failed"
        ? t(
            "The session could not be resumed; your next message starts a new one.",
          )
        : activity.title === "checkpoint_unavailable"
          ? t("Undo is unavailable for this reply.")
          : activity.title === "context_compacted"
            ? t("The conversation context was compacted.")
            : t("Notice");
  return (
    <li className="thread-activity-item thread-info-item">
      <div className="thread-info-row">
        <Info size={13} aria-hidden="true" />
        <span>{text}</span>
        {activity.detail && (
          <button
            type="button"
            className="thread-link"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
          >
            {t("Details")}
          </button>
        )}
      </div>
      {open && activity.detail && (
        <pre className="thread-activity-detail">{activity.detail}</pre>
      )}
    </li>
  );
}

function ActivityRow({ activity, cwd }: { activity: Activity; cwd: string }) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  if (isPending(activity))
    return <ApprovalCard activity={activity} cwd={cwd} />;
  if (activity.kind === "notice") return <NoticeRow activity={activity} />;
  if (activity.question)
    return isPendingQuestion(activity) ? (
      <QuestionCard activity={activity} />
    ) : (
      <QuestionSummary activity={activity} />
    );
  const Icon = kindIcons[activity.kind];
  const running = activity.status === "running";
  const StatusIcon =
    activity.status === "failed"
      ? CircleAlert
      : activity.status === "completed"
        ? null
        : Circle;
  const status = running
    ? t("Running")
    : activity.status === "failed"
      ? t("Failed")
      : t("Completed");
  // Backends send English reasoning titles; the label is always localized.
  const reasoning = activity.kind === "reasoning";
  const label = reasoning
    ? running
      ? t("Thinking")
      : t("Thought")
    : relativeTitle(activity.title, cwd);
  const expandable = !!(activity.detail || activity.diff);
  const diff =
    open && activity.diff
      ? lineDiff(activity.diff.before, activity.diff.after)
      : [];
  const state = activity.approval?.state;
  const settled =
    state === "allowed"
      ? { state, label: t("Allowed") }
      : state === "denied"
        ? { state, label: t("Denied") }
        : state === "expired"
          ? { state, label: t("Expired") }
          : null;
  const content = (
    <>
      <Icon size={13} aria-hidden="true" />
      <span
        className="thread-activity-title"
        title={reasoning ? undefined : activity.title}
      >
        {label}
      </span>
      <span
        className="thread-activity-status"
        data-running={running || undefined}
      >
        {StatusIcon && <StatusIcon size={12} aria-hidden="true" />}
        <span className="thread-visually-hidden">{status}</span>
      </span>
      {settled && (
        <span className="thread-approval-state" data-state={settled.state}>
          {settled.label}
        </span>
      )}
      {expandable && (
        <ChevronRight
          size={12}
          className="thread-activity-chevron"
          aria-hidden="true"
        />
      )}
    </>
  );
  return (
    <li className="thread-activity-item" data-status={activity.status}>
      {expandable ? (
        <button
          type="button"
          className="thread-activity-row"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          {content}
        </button>
      ) : (
        <div className="thread-activity-row">{content}</div>
      )}
      {open && (
        <div className="thread-activity-panel">
          {activity.diff && (
            <div className="thread-diff">
              <div className="thread-diff-path">{activity.diff.path}</div>
              <pre>
                {diff.map((line, index) => (
                  <div
                    key={index}
                    className="thread-diff-line"
                    data-sign={line.sign}
                  >
                    {line.sign === "-" ? "\u2212" : "+"} {line.text}
                  </div>
                ))}
              </pre>
            </div>
          )}
          {activity.detail && (
            <pre className="thread-activity-detail">{activity.detail}</pre>
          )}
        </div>
      )}
    </li>
  );
}

function ActivityGroup({
  group,
  cwd,
}: {
  group: readonly Activity[];
  cwd: string;
}) {
  const { tp } = useT();
  const [open, setOpen] = useState(false);
  const { collapsed, visible } = collapseGroup(group);
  return (
    <ul className="thread-activity">
      {collapsed.length > 0 && (
        <li className="thread-activity-item">
          <button
            type="button"
            className="thread-activity-row"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
          >
            <ChevronRight
              size={12}
              className="thread-activity-chevron"
              aria-hidden="true"
            />
            <span className="thread-activity-title">
              {tp(collapsed.length, "{count} step", "{count} steps")}
            </span>
          </button>
          {open && (
            <ul className="thread-activity thread-activity-nested">
              {collapsed.map((activity) => (
                <ActivityRow key={activity.id} activity={activity} cwd={cwd} />
              ))}
            </ul>
          )}
        </li>
      )}
      {visible.map((activity) => (
        <ActivityRow key={activity.id} activity={activity} cwd={cwd} />
      ))}
    </ul>
  );
}

function UsageLine({ usage }: { usage: Usage }) {
  const { t, locale } = useT();
  const input = compactNumber(usage.inputTokens, locale);
  const output = compactNumber(usage.outputTokens, locale);
  return (
    <p
      className="thread-note thread-usage"
      aria-label={t("Tokens: input {input}, output {output}", {
        input,
        output,
      })}
    >
      {t("{input} in, {output} out", { input, output })}
      {usage.costUsd !== null && ` \u00b7 $${usage.costUsd.toFixed(4)}`}
    </p>
  );
}

const statusLetter = { added: "A", modified: "M", deleted: "D", renamed: "R" };

/** Cached per message and path; the diff of a finished turn does not change. */
const diffCache = new Map<string, string>();
const maxCachedDiffs = 200;

function FileDiffView({
  threadId,
  messageId,
  file,
}: {
  threadId: string;
  messageId: string;
  file: FileChange;
}) {
  const { t } = useT();
  const key = `${threadId}\0${messageId}\0${file.path}`;
  const [text, setText] = useState(diffCache.get(key) ?? null);
  const [error, setError] = useState("");
  useEffect(() => {
    if (text !== null) return;
    let live = true;
    turnDiff(threadId, messageId, file.path)
      .then((value) => {
        if (diffCache.size >= maxCachedDiffs) diffCache.clear();
        diffCache.set(key, value);
        if (live) setText(value);
      })
      .catch((cause) => live && setError(errorMessage(cause)));
    return () => {
      live = false;
    };
  }, [key, text, threadId, messageId, file.path]);
  if (error)
    return (
      <p className="thread-notice" role="alert">
        {error}
      </p>
    );
  if (text === null)
    return (
      <p className="thread-note" role="status">
        {t("Loading diff…")}
      </p>
    );
  const parsed = parseUnifiedDiff(text);
  // Git header lines before the first hunk carry no information here.
  const firstHunk = parsed.findIndex((row) => row.kind === "hunk");
  return (
    <div className="thread-diff thread-turn-diff">
      <pre>
        {parsed.slice(Math.max(firstHunk, 0)).map((row, index) => (
          <div key={index} className="thread-diff-line" data-kind={row.kind}>
            <span className="thread-diff-num" aria-hidden="true">
              {row.oldLine ?? ""}
            </span>
            <span className="thread-diff-num" aria-hidden="true">
              {row.newLine ?? ""}
            </span>
            <span>
              {row.kind === "add" ? "+" : row.kind === "del" ? "\u2212" : " "}
              {row.text}
            </span>
          </div>
        ))}
      </pre>
    </div>
  );
}

function TurnChangesCard({
  threadId,
  messageId,
  changes,
}: {
  threadId: string;
  messageId: string;
  changes: TurnChanges;
}) {
  const { t, tp } = useT();
  const [open, setOpen] = useState(false);
  const [shown, setShown] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<TurnUndoResult | null>(null);
  const [restored, setRestored] = useState<ReadonlySet<string>>(new Set());
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [error, setError] = useState("");
  if (!changes.files.length) return null;
  const isUndone = (file: FileChange) => file.undone || restored.has(file.path);
  const remaining = changes.files.filter((file) => !isUndone(file));
  const additions = changes.files.reduce((sum, f) => sum + f.additions, 0);
  const deletions = changes.files.reduce((sum, f) => sum + f.deletions, 0);
  const openConfirm = () => {
    // Files outside the agent's edit tools may hold the user's own work.
    setPicked(
      new Set(remaining.filter((file) => file.agent).map((file) => file.path)),
    );
    setConfirm(true);
  };
  const undo = async () => {
    setConfirm(false);
    setBusy(true);
    setError("");
    try {
      const all = remaining.every((file) => picked.has(file.path));
      const next = all
        ? await undoTurn(threadId, messageId)
        : await undoTurnFiles(threadId, messageId, [...picked]);
      setResult(next);
      setRestored((known) => new Set([...known, ...next.restored]));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section
      className="thread-turn-changes"
      aria-label={t("Changes from this reply")}
    >
      <div className="thread-turn-head">
        <button
          type="button"
          className="thread-turn-toggle"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          <ChevronRight
            size={14}
            className="thread-turn-chevron"
            aria-hidden="true"
          />
          <FileDiff size={14} aria-hidden="true" />
          <span>
            {tp(
              changes.files.length,
              "{count} file changed",
              "{count} files changed",
            )}
          </span>
          {additions > 0 && (
            <span className="thread-turn-add">+{additions}</span>
          )}
          {deletions > 0 && (
            <span className="thread-turn-del">
              {"\u2212"}
              {deletions}
            </span>
          )}
        </button>
        {changes.undone || !remaining.length ? (
          <span className="thread-note">{t("Changes undone")}</span>
        ) : (
          <button
            type="button"
            className="thread-link"
            disabled={busy}
            onClick={openConfirm}
          >
            <Undo2 size={12} aria-hidden="true" />
            {t("Undo")}
          </button>
        )}
      </div>
      {error && (
        <p className="thread-notice" role="alert">
          {error}
        </p>
      )}
      {result && (
        <div className="thread-note thread-undo-result" role="status">
          <p>
            <Check size={12} aria-hidden="true" />
            {tp(
              result.restored.length,
              "Restored {count} file",
              "Restored {count} files",
            )}
          </p>
          {result.skipped.length > 0 && (
            <>
              <p>
                {tp(
                  result.skipped.length,
                  "Skipped {count} file changed since this reply:",
                  "Skipped {count} files changed since this reply:",
                )}
              </p>
              <ul className="thread-undo-skipped">
                {result.skipped.map((name) => (
                  <li key={name}>{name}</li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
      {open && (
        <ul className="thread-turn-files">
          {changes.files.map((file) => (
            <li key={file.path} data-undone={isUndone(file) || undefined}>
              <button
                type="button"
                className="thread-turn-file"
                aria-expanded={shown === file.path}
                disabled={busy}
                onClick={() => setShown(shown === file.path ? null : file.path)}
              >
                <span className="thread-turn-status" data-status={file.status}>
                  {statusLetter[file.status]}
                </span>
                <span className="thread-turn-path" title={file.path}>
                  <span className="thread-turn-dir">
                    {file.path.slice(0, file.path.lastIndexOf("/") + 1)}
                  </span>
                  <span className="thread-turn-name">
                    {file.path.slice(file.path.lastIndexOf("/") + 1)}
                  </span>
                </span>
                {isUndone(file) && (
                  <span className="thread-note">{t("Undone")}</span>
                )}
                {file.binary ? (
                  <span className="thread-note">{t("binary")}</span>
                ) : (
                  <>
                    {file.additions > 0 && (
                      <span className="thread-turn-add">+{file.additions}</span>
                    )}
                    {file.deletions > 0 && (
                      <span className="thread-turn-del">
                        {"\u2212"}
                        {file.deletions}
                      </span>
                    )}
                  </>
                )}
              </button>
              {shown === file.path && (
                <FileDiffView
                  threadId={threadId}
                  messageId={messageId}
                  file={file}
                />
              )}
            </li>
          ))}
        </ul>
      )}
      {confirm && (
        <Modal title={t("Undo changes?")} onClose={() => setConfirm(false)}>
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p>
                {t(
                  "Choose the files to restore to how they were before this reply. Files edited since then are left as they are.",
                )}
              </p>
              <ul className="thread-undo-pick">
                {changes.files.map((file) => (
                  <li key={file.path} data-undone={isUndone(file) || undefined}>
                    <label>
                      <input
                        type="checkbox"
                        disabled={isUndone(file)}
                        checked={picked.has(file.path)}
                        onChange={(event) =>
                          setPicked((known) => {
                            const next = new Set(known);
                            if (event.target.checked) next.add(file.path);
                            else next.delete(file.path);
                            return next;
                          })
                        }
                      />
                      <span className="thread-turn-path" title={file.path}>
                        {file.path}
                      </span>
                      {isUndone(file) && (
                        <span className="thread-note">{t("Undone")}</span>
                      )}
                    </label>
                    {!file.agent && !isUndone(file) && (
                      <p className="thread-note">
                        {t(
                          "Changed outside the agent's edit tools (by a command or by you)",
                        )}
                      </p>
                    )}
                  </li>
                ))}
              </ul>
            </div>
            <div className="dialog-actions">
              <button
                type="button"
                className="button"
                autoFocus
                onClick={() => setConfirm(false)}
              >
                {t("Cancel")}
              </button>
              <button
                type="button"
                className="button button-primary"
                disabled={!picked.size}
                onClick={() => void undo()}
              >
                {t("Undo changes")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </section>
  );
}

/** Opens Settings → Chat AI → Providers in the main view. */
const openProviders = () =>
  void emit("settings-page-changed", "chat-providers");

const noFiles: Attachment[] = [];

/** Starts a resend; false while the thread is busy. */
type ResendHandler = (
  text: string,
  files: readonly { id: string }[],
) => boolean;

/** Failure notice with an action tailored to the error class; the raw error stays in details. */
function ErrorNotice({
  item,
  provider,
  retryable,
  onEdit,
  onResend,
}: {
  item: ChatItemView;
  provider: string;
  /** Only the last reply of the thread can be retried. */
  retryable: boolean;
  onEdit: (text: string) => void;
  onResend: ResendHandler;
}) {
  const { t, locale } = useT();
  const [auto, setAuto] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const retryAt = item.retryAtMs ?? null;
  const edit = item.resend;
  // Attachments bind to one message only, so a reply with files can't be resent as is.
  const files = noFiles;
  const resend =
    retryable && !item.resendAttachments?.length ? item.resend : null;
  const left = retryAt === null ? 0 : retryAt - now;
  useEffect(() => {
    if (!auto) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [auto]);
  useEffect(() => {
    if (!auto || retryAt === null || !resend || retryAt > now) return;
    // A busy thread refuses; stay armed and try again on the next tick.
    if (onResend(resend, files)) setAuto(false);
  }, [auto, retryAt, now, resend, files, onResend]);
  const raw =
    item.error === INTERRUPTED_ERROR
      ? t("The response was interrupted; the partial reply was kept.")
      : item.error!;
  const name = provider || t("the provider");
  let message = raw;
  let action: ReactNode = null;
  switch (item.errorKind) {
    case "auth":
      message = t("Sign in to {provider} to continue.", { provider: name });
      action = (
        <button type="button" className="thread-link" onClick={openProviders}>
          {t("Open provider settings")}
        </button>
      );
      break;
    case "missing_cli":
      message = t("The {cli} CLI was not found.", { cli: name });
      action = (
        <button type="button" className="thread-link" onClick={openProviders}>
          {t("Open provider settings")}
        </button>
      );
      break;
    case "rate_limit":
      message =
        t("Usage limit reached.") +
        (retryAt === null
          ? ""
          : " " +
            t("Resets at {time}", {
              time: new Date(retryAt).toLocaleTimeString(locale, {
                timeStyle: "short",
              }),
            }));
      if (resend && retryAt !== null)
        action = auto ? (
          <>
            <span aria-live="off">
              {t("Resuming in {time}", { time: formatCountdown(left) })}
            </span>
            <button
              type="button"
              className="thread-link"
              onClick={() => setAuto(false)}
            >
              {t("Cancel")}
            </button>
          </>
        ) : (
          <button
            type="button"
            className="thread-link"
            onClick={() => {
              setNow(Date.now());
              setAuto(true);
            }}
          >
            {t("Resume automatically")}
          </button>
        );
      break;
    case "network":
      message = t("Network error.");
      if (resend)
        action = (
          <button
            type="button"
            className="thread-link"
            onClick={() => onResend(resend, files)}
          >
            {t("Retry")}
          </button>
        );
      break;
    case "context_length":
      message = t(
        "The conversation is too long for this model. Start a new chat or compact it.",
      );
      break;
  }
  const typed = message !== raw;
  return (
    <div className="thread-notice" role="alert">
      <CircleAlert size={14} aria-hidden="true" />
      <span title={raw}>{message}</span>
      {action}
      {edit && (
        <button
          type="button"
          className="thread-link"
          onClick={() => onEdit(edit)}
        >
          {t("Edit and resend")}
        </button>
      )}
      {typed && (
        <details className="thread-notice-details">
          <summary>{t("Error details")}</summary>
          <pre>{raw}</pre>
        </details>
      )}
    </div>
  );
}

const Message = memo(function Message({
  item,
  threadId,
  cwd,
  provider,
  retryable,
  onEdit,
  onResend,
}: {
  item: ChatItemView;
  threadId: string | null;
  cwd: string;
  provider: string;
  retryable: boolean;
  onEdit: (text: string) => void;
  onResend: ResendHandler;
}) {
  const { t } = useT();
  if (item.role === "user")
    return (
      <div className="thread-msg thread-msg-user">
        <MessageAttachments attachments={item.attachments} />
        {item.text && <div className="thread-bubble">{item.text}</div>}
      </div>
    );
  const parts = interleave(item.text, item.activity);
  return (
    <div
      className="thread-msg thread-msg-assistant"
      data-streaming={item.streaming || undefined}
    >
      {parts.map((part, index) => {
        if (part.type === "group")
          return (
            <ActivityGroup key={index} group={part.activities} cwd={cwd} />
          );
        const last = index === parts.length - 1;
        if (!part.text && !(last && item.streaming)) return null;
        return (
          <div key={index} className="thread-prose">
            <Markup text={part.text} streaming={item.streaming && last} />
          </div>
        );
      })}
      {item.stopped && <p className="thread-note">{t("Stopped.")}</p>}
      {item.error && (
        <ErrorNotice
          item={item}
          provider={provider}
          retryable={retryable}
          onEdit={onEdit}
          onResend={onResend}
        />
      )}
      {!item.streaming && item.changes && threadId && (
        <TurnChangesCard
          threadId={threadId}
          messageId={item.id}
          changes={item.changes}
        />
      )}
      {!item.streaming && item.usage && <UsageLine usage={item.usage} />}
    </div>
  );
});

export default function ThreadMessages({
  items,
  threadId,
  cwd,
  provider,
  hasMore,
  loadingEarlier,
  onLoadEarlier,
  onEdit,
  onResend,
  onRespond,
  onAnswer,
}: {
  items: readonly ChatItemView[];
  threadId: string | null;
  cwd: string;
  /** Label of the thread's provider, used in error messages. */
  provider: string;
  hasMore: boolean;
  loadingEarlier: boolean;
  onLoadEarlier: () => void;
  onEdit: (text: string) => void;
  onResend: ResendHandler;
  onRespond: ApprovalContext["respond"];
  onAnswer: ApprovalContext["answer"];
}) {
  const { t } = useT();
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  /** Scroll geometry captured before older messages are prepended. */
  const anchor = useRef<{ height: number; top: number } | null>(null);
  const last = items.at(-1);
  const lastReply = items.filter((item) => item.role === "assistant").at(-1);
  const firstId = items[0]?.id;
  useLayoutEffect(() => {
    pinned.current = true;
    anchor.current = null;
  }, [threadId]);
  const loadEarlier = () => {
    const element = scroller.current;
    if (!element || loadingEarlier) return;
    anchor.current = { height: element.scrollHeight, top: element.scrollTop };
    onLoadEarlier();
  };
  // Keep the viewport on the same message after a page is prepended.
  useLayoutEffect(() => {
    const element = scroller.current;
    const saved = anchor.current;
    if (!element || !saved) return;
    anchor.current = null;
    element.scrollTop = saved.top + element.scrollHeight - saved.height;
  }, [firstId]);
  const first = items
    .flatMap((item) => item.activity)
    .find((item) => isPending(item) || isPendingQuestion(item));
  const focusId =
    first?.approval?.requestId ?? first?.question?.requestId ?? null;
  const approvals = useMemo(
    () => ({ respond: onRespond, answer: onAnswer, focusId }),
    [onRespond, onAnswer, focusId],
  );
  const tail = `${last?.text.length}:${last?.activity
    .map(
      (item) =>
        `${item.status}${item.approval?.state ?? ""}${item.question?.state ?? ""}${item.detail?.length ?? 0}`,
    )
    .join()}`;
  useLayoutEffect(() => {
    const element = scroller.current;
    if (element && pinned.current) element.scrollTop = element.scrollHeight;
  }, [items.length, tail]);
  return (
    <div
      ref={scroller}
      className="thread-scroll"
      onScroll={(event) => {
        const el = event.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
        if (hasMore && el.scrollTop < 80) loadEarlier();
      }}
    >
      <div className="thread-column">
        {hasMore && (
          <button
            type="button"
            className="button thread-earlier"
            disabled={loadingEarlier}
            onClick={loadEarlier}
          >
            {loadingEarlier ? t("Loading…") : t("Load earlier messages")}
          </button>
        )}
        <Approvals.Provider value={approvals}>
          {items.map((item) => (
            <Message
              key={item.id}
              item={item}
              threadId={threadId}
              cwd={cwd}
              provider={provider}
              retryable={item.id === lastReply?.id}
              onEdit={onEdit}
              onResend={onResend}
            />
          ))}
        </Approvals.Provider>
      </div>
    </div>
  );
}
