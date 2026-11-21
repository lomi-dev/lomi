import {
  Children,
  isValidElement,
  memo,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import Markdown from "react-markdown";
import type { Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, Circle, CircleAlert, Copy, Terminal } from "../icons";
import { MessageAttachments } from "./Attachments";
import type { ChatItem, ToolRow } from "./types";

const plugins = [remarkGfm];

function CodeBlock({ children }: { children?: ReactNode }) {
  const [copied, setCopied] = useState(false);
  const code = Children.toArray(children).find(isValidElement) as
    { props: { className?: string; children?: ReactNode } } | undefined;
  const language = /language-([\w-]+)/.exec(code?.props.className ?? "")?.[1];
  const source = String(code?.props.children ?? "").replace(/\n$/, "");
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
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre>{children}</pre>
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

/** Re-renders only when the text changes, so settled messages stay inert. */
const Markup = memo(function Markup({ text }: { text: string }) {
  return (
    <Markdown remarkPlugins={plugins} skipHtml components={components}>
      {text}
    </Markdown>
  );
});

function ToolLine({ tool }: { tool: ToolRow }) {
  const Icon =
    tool.state === "failed"
      ? CircleAlert
      : tool.state === "completed"
        ? Check
        : Circle;
  return (
    <li className="thread-tool" data-state={tool.state}>
      <Terminal size={13} />
      <span className="thread-tool-label">{tool.label}</span>
      <Icon size={12} aria-hidden="true" />
      <span className="thread-tool-state">{tool.state}</span>
    </li>
  );
}

const Message = memo(function Message({
  item,
  onEdit,
}: {
  item: ChatItem;
  onEdit: (text: string) => void;
}) {
  if (item.role === "user")
    return (
      <div className="thread-msg thread-msg-user">
        <MessageAttachments attachments={item.attachments} />
        {item.text && <div className="thread-bubble">{item.text}</div>}
      </div>
    );
  return (
    <div
      className="thread-msg thread-msg-assistant"
      data-streaming={item.streaming || undefined}
    >
      {item.tools.length > 0 && (
        <ul className="thread-tools">
          {item.tools.map((tool) => (
            <ToolLine key={tool.id} tool={tool} />
          ))}
        </ul>
      )}
      {(item.text || item.streaming) && (
        <div className="thread-prose">
          <Markup text={item.text} />
        </div>
      )}
      {item.stopped && <p className="thread-note">Stopped.</p>}
      {item.error && (
        <div className="thread-notice" role="alert">
          <CircleAlert size={14} aria-hidden="true" />
          <span>{item.error}</span>
          {item.resend && (
            <button
              type="button"
              className="thread-link"
              onClick={() => onEdit(item.resend!)}
            >
              Edit and resend
            </button>
          )}
        </div>
      )}
    </div>
  );
});

export default function ThreadMessages({
  items,
  onEdit,
}: {
  items: readonly ChatItem[];
  onEdit: (text: string) => void;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const tail = items.at(-1)?.text.length;
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
      }}
    >
      <div className="thread-column" role="log" aria-live="polite">
        {items.map((item) => (
          <Message key={item.id} item={item} onEdit={onEdit} />
        ))}
      </div>
    </div>
  );
}
