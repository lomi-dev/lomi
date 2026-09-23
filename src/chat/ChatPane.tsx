import { useChat } from "@ai-sdk/react";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { listen } from "@tauri-apps/api/event";
import type { ChatTab } from "../model";
import { api, errorMessage, native } from "../api";
import { DisclosureSummary, IconButton, Modal } from "../ui";
import Select from "../Select";
import {
  Copy,
  History,
  MessageSquare,
  ArrowUp,
  ChevronDown,
  Pencil,
  RotateCcw,
  Plus,
  Settings,
  Square,
  X,
} from "../icons";
import { replaceChat } from "./chat-service";
import { getChat, main } from "./chat-runtime";
import type { ChatRuntime } from "./chat-runtime";
import type { Attachment, Config } from "./types";
import ChatHistory from "./ChatHistory";
import { textOf } from "./types";
import capabilities from "./model-capabilities.json";
import { ModelSelect } from "./ModelSelect";
import { modelLabel, suggestedModel } from "./models";
import "./chat.css";
const copy = (value: string) =>
  native ? writeText(value) : navigator.clipboard.writeText(value);
export default function ChatPane({
  tab,
  onFocus,
  onClose,
}: {
  tab: ChatTab;
  onFocus: () => void;
  onClose: () => void;
}) {
  const runtime = getChat(tab.conversationId);
  const state = useSyncExternalStore(runtime.subscribe, runtime.getSnapshot);
  const { messages } = useChat({
    chat: runtime.chat,
    experimental_throttle: 50,
  });
  const root = useRef<HTMLElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const disclaimerId = useId();
  const scroll = useRef<HTMLDivElement>(null);
  const [awayFromBottom, setAwayFromBottom] = useState(false);
  useEffect(() => runtime.show(), [runtime]);
  const bottom = useRef(runtime.scroll.get(tab.id)?.bottom ?? true);
  const [recovering, setRecovering] = useState(false);
  const [history, setHistory] = useState(false);
  const historyId = useId();
  const historyTrigger = useRef<HTMLButtonElement | null>(null);
  const content = useRef<HTMLDivElement>(null);
  const closeHistory = (restoreFocus = true) => {
    setHistory(false);
    if (restoreFocus)
      requestAnimationFrame(() =>
        (historyTrigger.current ?? input.current)?.focus(),
      );
  };
  const [discarding, setDiscarding] = useState(false);
  const [options, setOptions] = useState(false);
  const [edit, setEdit] = useState<{ id: string; text: string } | null>(null);
  const [sensitive, setSensitive] = useState<
    { path: string } | { name: string; bytes: number[] } | null
  >(null);
  const attachmentQueue = useRef<Promise<void>>(Promise.resolve());
  const attachmentEpoch = useRef(0);
  const sensitiveDecision = useRef<(approved: boolean) => void>(undefined);
  const decideSensitive = (approved: boolean) => {
    sensitiveDecision.current?.(approved);
    sensitiveDecision.current = undefined;
    setSensitive(null);
  };
  useEffect(
    () => () => {
      attachmentEpoch.current++;
      sensitiveDecision.current?.(false);
    },
    [runtime],
  );
  const [preview, setPreview] = useState<{
    attachment: Attachment;
    url: string;
  } | null>(null);
  const loaded = state.loaded;
  const config = loaded?.conversation.config;
  const connection = state.preferences?.connections.find(
    (c) => c.id === config?.connectionId,
  );
  const run = (promise: Promise<unknown>) => {
    void promise.catch(runtime.report);
  };
  const attach = (source: NonNullable<typeof sensitive>) => {
    const epoch = attachmentEpoch.current;
    const next = attachmentQueue.current.then(async () => {
      if (epoch !== attachmentEpoch.current) return;
      try {
        await runtime.attach(source);
      } catch (error) {
        if (epoch !== attachmentEpoch.current) return;
        if (!errorMessage(error).startsWith("sensitive:")) throw error;
        const approved = await new Promise<boolean>((resolve) => {
          sensitiveDecision.current = resolve;
          setSensitive(source);
        });
        if (approved) await runtime.attach(source, true);
      }
    });
    attachmentQueue.current = next.catch(runtime.report);
    return next;
  };
  const files = async (files: File[]) => {
    if (files.length > 10) throw Error("Choose up to 10 attachments.");
    for (const file of files) {
      if (file.size > 10 * 1024 * 1024)
        throw Error(
          "Each image must be at most 10 MiB; text files at most 1 MiB.",
        );
      await attach({
        name: file.name,
        bytes: Array.from(new Uint8Array(await file.arrayBuffer())),
      });
    }
  };
  useEffect(() => {
    const element = root.current!;
    const action = (event: Event) => {
      const action = (event as CustomEvent<string>).detail;
      if (action === "chatFocusInput") input.current?.focus();
      if (action === "chatHistory") setHistory(true);
      if (action === "chatStop") run(runtime.stop());
    };
    const drop = (event: Event) => {
      const paths = (event as CustomEvent<string[]>).detail;
      if (paths.length > 10) {
        runtime.report("Choose up to 10 attachments.");
        return;
      }
      run(
        (async () => {
          for (const path of paths) await attach({ path });
        })(),
      );
    };
    element.addEventListener("chat-action", action);
    element.addEventListener("chat-files", drop);
    const stop = native
      ? listen("chat-preferences-changed", () => {
          run(runtime.refreshPreferences());
        })
      : undefined;
    return () => {
      element.removeEventListener("chat-action", action);
      element.removeEventListener("chat-files", drop);
      void stop?.then((fn) => fn());
    };
  }, [runtime]);
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const saved = runtime.scroll.get(tab.id);
    if (saved) element.scrollTop = saved.top;
    return () => {
      runtime.scroll.set(tab.id, {
        top: element.scrollTop,
        bottom: bottom.current,
      });
    };
  }, [runtime, tab.id, state.loading]);
  useLayoutEffect(() => {
    if (bottom.current && scroll.current)
      scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [messages]);
  useLayoutEffect(() => {
    const element = input.current;
    const pane = content.current;
    if (!element || !pane) return;
    const resize = () => {
      element.style.height = "0px";
      const maximum = parseFloat(getComputedStyle(element).maxHeight);
      element.style.height = `${Math.min(element.scrollHeight, maximum)}px`;
      if (bottom.current && scroll.current)
        scroll.current.scrollTop = scroll.current.scrollHeight;
    };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(pane);
    return () => observer.disconnect();
  }, [state.text, !!loaded]);
  const ready =
    !!connection?.enabled && !!connection.secretId && !!config?.model;
  const canSend =
    ready &&
    !state.storageFailed &&
    !state.unsentText &&
    (!!state.text.trim() || !!loaded?.draft.attachments.length);
  return (
    <section
      ref={root}
      className={`chat-pane${history ? " has-history" : ""}`}
      data-chat-pane-id={tab.id}
      onFocusCapture={onFocus}
      onPointerDownCapture={onFocus}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files")) e.preventDefault();
      }}
      onDrop={(e) => {
        if (e.dataTransfer.files.length) {
          e.preventDefault();
          run(files(Array.from(e.dataTransfer.files)));
        }
      }}
    >
      <div
        className="chat-history-drawer"
        inert={!history}
        aria-hidden={!history}
      >
        <ChatHistory
          id={historyId}
          open={history}
          tab={tab}
          runtime={runtime}
          onClose={closeHistory}
          onExport={(format) => exportChat(runtime, format)}
        />
      </div>
      <div ref={content} className="chat-conversation">
        <header className="chat-toolbar">
          <MessageSquare size={15} aria-hidden="true" />
          <span
            className="chat-title"
            title={loaded?.conversation.title ?? tab.title}
          >
            {loaded?.conversation.title ?? tab.title}
          </span>
          <IconButton
            title="New conversation"
            onClick={() => run(replaceChat(tab))}
          >
            <Plus size={15} />
          </IconButton>
          <IconButton
            title="Chat history"
            aria-expanded={history}
            aria-controls={historyId}
            onClick={(event) => {
              historyTrigger.current = event.currentTarget;
              setHistory((open) => !open);
            }}
          >
            <History size={15} />
          </IconButton>
          <IconButton
            title="Chat AI settings"
            onClick={() => run(api("open_settings", { page: "chat-ai" }))}
          >
            <Settings size={15} />
          </IconButton>
          <IconButton title="Close chat panel" onClick={onClose}>
            <X size={15} />
          </IconButton>
        </header>
        <div
          ref={scroll}
          className={`chat-messages${!messages.length ? " chat-messages-empty" : ""}`}
          aria-label="Conversation messages"
          onScroll={(e) => {
            const element = e.currentTarget;
            bottom.current =
              element.scrollHeight - element.scrollTop - element.clientHeight <
              48;
            setAwayFromBottom(!bottom.current);
            runtime.scroll.set(tab.id, {
              top: element.scrollTop,
              bottom: bottom.current,
            });
          }}
        >
          {state.loading ? (
            <p role="status">Loading conversation…</p>
          ) : !loaded ? (
            <div className="chat-empty">
              <h2>Conversation unavailable</h2>
              <p>The saved conversation could not be opened.</p>
              <p className="muted">Conversation ID: {tab.conversationId}</p>
              <button className="button" onClick={() => setHistory(true)}>
                Open history
              </button>
              {!state.error.startsWith("missing:") && (
                <>
                  <button
                    className="button"
                    onClick={() => run(runtime.recover(false))}
                  >
                    Retry opening history
                  </button>
                  <button
                    className="button"
                    onClick={() => setRecovering(true)}
                  >
                    Recover history…
                  </button>
                </>
              )}
            </div>
          ) : !messages.length ? (
            <div className="chat-empty">
              <MessageSquare size={28} aria-hidden="true" />
              <h2>What would you like to work on?</h2>
              <p>
                {ready
                  ? "Ask a question, explore an idea, or drop in a file."
                  : "Connect your preferred AI provider in Settings to get started."}
              </p>
              {!ready && (
                <button
                  className="button"
                  onClick={() => run(api("open_settings", { page: "chat-ai" }))}
                >
                  Set up Chat AI
                </button>
              )}
              <p className="muted">
                Only messages and files you choose are shared. History stays on
                this device.
              </p>
            </div>
          ) : (
            <>
              {loaded.hasOlder && (
                <button
                  className="button"
                  disabled={state.busy}
                  onClick={() => {
                    bottom.current = false;
                    run(runtime.older());
                  }}
                >
                  Load earlier messages
                </button>
              )}
              {messages.map((message) => {
                const saved = loaded.messages.find((m) => m.id === message.id);
                return (
                  <article
                    key={message.id}
                    className={`chat-message chat-message-${message.role}`}
                    aria-label={
                      message.role === "user"
                        ? "Your message"
                        : "Assistant message"
                    }
                  >
                    <div className="chat-message-content">
                      {message.parts.map((part, i) =>
                        part.type === "text" ? (
                          <div className="chat-markdown" key={i}>
                            <Markdown
                              text={part.text}
                              onError={runtime.report}
                            />
                          </div>
                        ) : part.type === "reasoning" ? (
                          <details key={i}>
                            <DisclosureSummary>Reasoning</DisclosureSummary>
                            <div className="chat-markdown">
                              <Markdown
                                text={part.text}
                                onError={runtime.report}
                              />
                            </div>
                          </details>
                        ) : (
                          <p key={i}>
                            Unsupported saved content. Export JSON to preserve
                            this message.
                          </p>
                        ),
                      )}
                      {!!saved?.attachments?.length && (
                        <div className="chat-attachments">
                          {saved.attachments.map((id) => (
                            <AttachmentChip
                              key={id}
                              id={id}
                              runtime={runtime}
                              onPreview={setPreview}
                            />
                          ))}
                        </div>
                      )}
                    </div>
                    {saved?.status &&
                      !["completed", "active"].includes(saved.status) && (
                        <span className="chat-message-status">
                          {saved.status}
                        </span>
                      )}
                    <div className="chat-message-actions">
                      <IconButton
                        title="Copy message"
                        onClick={() => run(copy(textOf(message)))}
                      >
                        <Copy size={15} />
                      </IconButton>
                      {message.role === "user" ? (
                        <IconButton
                          title="Edit as variant"
                          disabled={state.busy}
                          onClick={() =>
                            setEdit({ id: message.id, text: textOf(message) })
                          }
                        >
                          <Pencil size={15} />
                        </IconButton>
                      ) : (
                        <IconButton
                          title={
                            saved?.status === "completed"
                              ? "Regenerate"
                              : "Retry"
                          }
                          disabled={state.busy}
                          onClick={() => run(runtime.send("retry", message.id))}
                        >
                          <RotateCcw size={15} />
                        </IconButton>
                      )}
                      {saved?.previousVariant && (
                        <button
                          disabled={state.busy}
                          aria-label="Previous variant"
                          onClick={() => {
                            bottom.current = false;
                            run(runtime.variant(saved.previousVariant!));
                          }}
                        >
                          ← Variant
                        </button>
                      )}
                      {saved?.nextVariant && (
                        <button
                          disabled={state.busy}
                          aria-label="Next variant"
                          onClick={() => {
                            bottom.current = false;
                            run(runtime.variant(saved.nextVariant!));
                          }}
                        >
                          Variant →
                        </button>
                      )}
                      {message.role === "assistant" &&
                        !!saved?.metadata &&
                        Object.keys(saved.metadata).length > 0 && (
                          <details>
                            <DisclosureSummary>Details</DisclosureSummary>
                            <pre>{JSON.stringify(saved.metadata, null, 2)}</pre>
                          </details>
                        )}
                    </div>
                  </article>
                );
              })}
            </>
          )}
        </div>
        {awayFromBottom && (
          <button
            className="chat-jump"
            onClick={() => {
              bottom.current = true;
              setAwayFromBottom(false);
              scroll.current?.scrollTo({ top: scroll.current.scrollHeight });
            }}
          >
            Jump to latest message
          </button>
        )}
        {state.unsentText && (
          <div className="chat-error" role="alert">
            <span>
              The previous message was not sent. Your next draft is also
              retained.
            </span>
            <button onClick={() => runtime.resolveUnsent(true)}>
              Add unsent message to draft
            </button>
            <button onClick={() => run(copy(state.unsentText!))}>
              Copy unsent message
            </button>
            <button onClick={() => runtime.resolveUnsent(false)}>
              Discard unsent message
            </button>
          </div>
        )}
        {state.error && (
          <div className="chat-error" role="alert">
            <span>{state.error}</span>
            {state.storageFailed && (
              <button onClick={() => run(runtime.retrySave())}>
                Retry saving
              </button>
            )}
            {state.busy && (
              <button onClick={() => run(runtime.reconnect())}>
                Reconnect response
              </button>
            )}
            <button onClick={() => setDiscarding(true)}>
              Close without saving…
            </button>
            <button onClick={() => run(exportChat(runtime, "json", true))}>
              Export available data
            </button>
          </div>
        )}
        {loaded && (
          <div className="chat-composer">
            <div className="chat-composer-surface">
              {!!loaded.draft.attachments.length && (
                <div className="chat-attachments">
                  {loaded.draft.attachments.map((id) => (
                    <AttachmentChip
                      key={id}
                      id={id}
                      runtime={runtime}
                      onPreview={setPreview}
                      remove={() => run(runtime.removeAttachment(id))}
                    />
                  ))}
                </div>
              )}
              <textarea
                ref={input}
                data-chat-input
                aria-label="Message"
                aria-describedby={disclaimerId}
                rows={1}
                placeholder={
                  state.busy ? "Draft your next message…" : "Ask anything…"
                }
                value={state.text}
                onChange={(e) => runtime.setText(e.target.value)}
                onPaste={(e) => {
                  const images = Array.from(e.clipboardData.files).filter((f) =>
                    f.type.startsWith("image/"),
                  );
                  if (images.length) {
                    e.preventDefault();
                    run(files(images));
                  }
                }}
                onKeyDown={(e) => {
                  if (
                    e.nativeEvent.isComposing ||
                    e.nativeEvent.keyCode === 229 ||
                    e.repeat
                  )
                    return;
                  const send =
                    e.key === "Enter" &&
                    !e.shiftKey &&
                    !e.altKey &&
                    (state.preferences?.sendMode === "modifier-enter"
                      ? e.ctrlKey || e.metaKey
                      : !e.ctrlKey && !e.metaKey);
                  if (send && !state.busy) {
                    e.preventDefault();
                    if (canSend) {
                      bottom.current = true;
                      run(runtime.send());
                    }
                  }
                }}
              />
              <div className="chat-composer-actions">
                <IconButton
                  title="Attach UTF-8 text or image"
                  onClick={() =>
                    run(
                      (async () => {
                        const selected = await open({
                          multiple: true,
                          directory: false,
                          title: "Attach files to this conversation",
                        });
                        if (selected) {
                          const paths =
                            typeof selected === "string"
                              ? [selected]
                              : selected;
                          if (paths.length > 10)
                            throw Error("Choose up to 10 attachments.");
                          for (const path of paths) await attach({ path });
                        }
                      })(),
                    )
                  }
                >
                  <Plus size={20} />
                </IconButton>
                <button
                  className="chat-model-button"
                  title={
                    connection
                      ? `${connection.name} · ${config?.model || "Choose a model"}`
                      : "Set up Chat AI"
                  }
                  aria-label="Choose model"
                  disabled={state.busy}
                  onClick={() => {
                    if (
                      state.preferences?.connections.some(
                        (c) => c.enabled && c.secretId,
                      )
                    )
                      setOptions(true);
                    else run(api("open_settings", { page: "chat-ai" }));
                  }}
                >
                  <span>
                    {config?.model ? modelLabel(config.model) : "Choose model"}
                  </span>
                  <ChevronDown size={14} />
                </button>
                {state.busy ? (
                  <IconButton
                    className="chat-send"
                    title="Stop"
                    onClick={() => run(runtime.stop())}
                  >
                    <Square size={15} fill="currentColor" />
                  </IconButton>
                ) : (
                  <IconButton
                    className="chat-send"
                    title="Send"
                    disabled={!canSend}
                    onClick={() => {
                      bottom.current = true;
                      run(runtime.send());
                    }}
                  >
                    <ArrowUp size={19} />
                  </IconButton>
                )}
              </div>
            </div>
            <span className="chat-status" role="status">
              {state.status}
            </span>
            <p className="chat-disclaimer" id={disclaimerId}>
              Chat AI może popełniać błędy. Sprawdź ważne informacje.
            </p>
          </div>
        )}
      </div>
      {discarding && (
        <Modal
          protectTheme
          className="chat-dialog"
          title="Discard unsaved chat data?"
          tone="warning"
          onClose={() => setDiscarding(false)}
        >
          <div className="dialog-form">
            <p>
              Unsaved draft edits and response text may be lost. The last
              committed history remains available. Export available data first
              if you want to keep a copy.
            </p>
            <div className="dialog-actions">
              <button className="button" onClick={() => setDiscarding(false)}>
                Keep open
              </button>
              <button
                className="button button-danger"
                onClick={() =>
                  run(
                    runtime.discard().then(() => {
                      setDiscarding(false);
                      onClose();
                    }),
                  )
                }
              >
                Discard and close
              </button>
            </div>
          </div>
        </Modal>
      )}
      {options && config && (
        <ConfigDialog
          config={config}
          runtime={runtime}
          hasHistory={!!messages.length}
          onClose={() => setOptions(false)}
        />
      )}
      {recovering && (
        <Modal
          protectTheme
          className="chat-dialog"
          title="Recover chat history?"
          tone="warning"
          onClose={() => setRecovering(false)}
        >
          <div className="dialog-form">
            <p>
              The original database, WAL and attachments will be moved to a
              private recovery folder inside the Chat AI data directory. New
              history starts empty. Existing conversation tabs keep their IDs
              and will be unavailable until you create or open another
              conversation.
            </p>
            <button
              className="button"
              onClick={() =>
                run(
                  (async () => {
                    await api("chat_recover", {
                      target: "history",
                      reset: true,
                    });
                    setRecovering(false);
                    await replaceChat(tab);
                  })(),
                )
              }
            >
              Back up and start empty history
            </button>
          </div>
        </Modal>
      )}
      {edit && (
        <Modal
          className="chat-dialog"
          title="Edit message as a new variant"
          onClose={() => setEdit(null)}
        >
          <div className="dialog-form">
            <p>The original message and its responses remain in history.</p>
            <textarea
              aria-label="Edited message"
              rows={7}
              value={edit.text}
              onChange={(e) => setEdit({ ...edit, text: e.target.value })}
            />
            <button
              className="button"
              disabled={!edit.text.trim()}
              onClick={() => {
                run(runtime.send("edit", edit.id, edit.text));
                setEdit(null);
              }}
            >
              Send new variant
            </button>
          </div>
        </Modal>
      )}
      {sensitive && (
        <Modal
          protectTheme
          className="chat-dialog"
          title="Attach sensitive file?"
          tone="warning"
          onClose={() => decideSensitive(false)}
        >
          <div className="dialog-form">
            <p>
              <strong>
                {"path" in sensitive
                  ? sensitive.path.split(/[\\/]/).pop()
                  : sensitive.name}
              </strong>{" "}
              may contain credentials. Its contents will be sent to the selected
              provider when you send this message.
            </p>
            <div className="dialog-actions">
              <button className="button" onClick={() => decideSensitive(false)}>
                Cancel
              </button>
              <button className="button" onClick={() => decideSensitive(true)}>
                Attach this file
              </button>
            </div>
          </div>
        </Modal>
      )}
      {preview && (
        <Modal
          className="chat-dialog"
          title={preview.attachment.name}
          onClose={() => setPreview(null)}
        >
          <div className="dialog-form">
            {preview.attachment.mime.startsWith("image/") ? (
              <img
                className="chat-image-preview"
                src={preview.url}
                alt={preview.attachment.name}
              />
            ) : (
              <TextPreview url={preview.url} />
            )}
          </div>
        </Modal>
      )}
    </section>
  );
}
function Markdown({
  text,
  onError,
}: {
  text: string;
  onError: (error: unknown) => void;
}) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      skipHtml
      components={{
        img: ({ alt }) => (
          <span className="muted">[Remote image: {alt || "image"}]</span>
        ),
        a: ({ href, children }) =>
          /^https?:\/\//i.test(href ?? "") ? (
            <a
              href={href}
              onClick={(e) => {
                e.preventDefault();
                void openUrl(href!).catch(onError);
              }}
            >
              {children}
            </a>
          ) : (
            <span>{children}</span>
          ),
        pre: ({ children }) => (
          <div className="chat-code">
            <button
              aria-label="Copy code"
              onClick={(e) => {
                const value =
                  e.currentTarget.parentElement?.querySelector("pre")
                    ?.textContent ?? "";
                void copy(value).catch(onError);
              }}
            >
              Copy
            </button>
            <pre>{children}</pre>
          </div>
        ),
      }}
    >
      {text}
    </ReactMarkdown>
  );
}
function AttachmentChip({
  id,
  runtime,
  remove,
  onPreview,
}: {
  id: string;
  runtime: ChatRuntime;
  remove?: () => void;
  onPreview: (value: { attachment: Attachment; url: string }) => void;
}) {
  const [value, setValue] = useState<Attachment>();
  useEffect(() => {
    let active = true;
    void main<Attachment>({ action: "attachment-meta", attachment: id })
      .then((v) => {
        if (active) setValue(v);
      })
      .catch(runtime.report);
    return () => {
      active = false;
    };
  }, [id, runtime]);
  return (
    <span className="chat-attachment">
      <button
        disabled={!value}
        onClick={() => {
          void runtime.attachment(id).then(onPreview).catch(runtime.report);
        }}
      >
        {value?.name ?? "Loading attachment…"}
      </button>
      {remove && (
        <IconButton title="Remove attachment" onClick={remove}>
          <X size={12} />
        </IconButton>
      )}
    </span>
  );
}

function TextPreview({ url }: { url: string }) {
  const [text, setText] = useState("");
  useEffect(() => {
    let active = true;
    void fetch(url)
      .then((r) => r.text())
      .then((t) => {
        if (active) setText(t);
      })
      .catch(() => {
        if (active) setText("This attachment preview is unavailable.");
      });
    return () => {
      active = false;
    };
  }, [url]);
  return <pre className="chat-text-preview">{text}</pre>;
}
function ConfigDialog({
  config,
  runtime,
  hasHistory,
  onClose,
}: {
  config: Config;
  runtime: ChatRuntime;
  hasHistory: boolean;
  onClose: () => void;
}) {
  const connectionId = useId();
  const [next, setNext] = useState(config);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const connections = runtime.snapshot.preferences?.connections ?? [];
  const selected = connections.find((c) => c.id === next.connectionId);
  const capability = capabilities.models.find(
    (m) => m.provider === selected?.provider && m.id === next.model,
  );
  const visibleBytes = new TextEncoder().encode(
    next.system +
      runtime.snapshot.text +
      runtime.chat.messages.map(textOf).join("\n"),
  ).length;
  return (
    <Modal
      className="chat-dialog"
      title="Conversation settings"
      onClose={onClose}
    >
      <form
        className="dialog-form chat-config"
        onSubmit={(e) => {
          e.preventDefault();
          setBusy(true);
          void runtime
            .configure({ ...next, configured: true })
            .then(onClose)
            .catch((e) => setError(errorMessage(e)))
            .finally(() => setBusy(false));
        }}
      >
        <div className="chat-select-field">
          <label htmlFor={connectionId}>Connection</label>
          <Select
            id={connectionId}
            value={next.connectionId ?? ""}
            onChange={(value) =>
              setNext({
                ...next,
                connectionId: value || null,
                model: suggestedModel(connections.find((c) => c.id === value)),
                temperature: null,
              })
            }
            options={[
              { value: "", label: "Choose connection" },
              ...connections
                .filter((c) => c.enabled)
                .map((c) => ({
                  value: c.id,
                  label: `${c.name} · ${c.provider}`,
                })),
            ]}
          />
        </div>
        <ModelSelect
          key={next.connectionId}
          connection={selected}
          value={next.model}
          onChange={(model) => setNext({ ...next, model, temperature: null })}
        />
        <details className="chat-advanced">
          <DisclosureSummary>Advanced options</DisclosureSummary>
          {!capability && (
            <small>
              Manual model IDs support basic text. Image and temperature support
              must be verified for this model.
            </small>
          )}
          <label>
            System instructions
            <textarea
              rows={4}
              value={next.system}
              onChange={(e) => setNext({ ...next, system: e.target.value })}
            />
          </label>
          <label>
            Maximum output tokens
            <input
              type="number"
              min={1}
              max={32768}
              value={next.maxOutputTokens}
              onChange={(e) =>
                setNext({ ...next, maxOutputTokens: Number(e.target.value) })
              }
            />
          </label>
          {capability?.temperature && (
            <label>
              Temperature (optional)
              <input
                type="number"
                min={0}
                max={2}
                step={0.1}
                value={next.temperature ?? ""}
                onChange={(e) =>
                  setNext({
                    ...next,
                    temperature:
                      e.target.value === "" ? null : Number(e.target.value),
                  })
                }
              />
            </label>
          )}
          <small>
            Visible text estimate: roughly{" "}
            {Math.ceil(visibleBytes / 4).toLocaleString()}–
            {visibleBytes.toLocaleString()} tokens. Files, earlier pages and
            provider overhead are excluded; the model can reject a larger
            context.
          </small>
        </details>
        {hasHistory && config.connectionId !== next.connectionId && (
          <p role="status">
            On the next Send or Retry, the active conversation history and its
            attachments will be sent to{" "}
            {selected?.name ?? "the selected connection"}.
          </p>
        )}
        {error && <p role="alert">{error}</p>}
        <div className="dialog-actions">
          <button
            type="button"
            className="button"
            onClick={() =>
              void api("open_settings", { page: "chat-ai" }).catch(
                runtime.report,
              )
            }
          >
            Manage connections
          </button>
          <button
            type="button"
            className="button"
            disabled={!runtime.snapshot.preferences}
            onClick={() => {
              const defaults = runtime.snapshot.preferences?.defaults;
              if (defaults) setNext({ ...defaults });
            }}
          >
            Use defaults
          </button>
          <button
            className="button"
            disabled={busy || !next.connectionId || !next.model}
          >
            Apply
          </button>
        </div>
      </form>
    </Modal>
  );
}
async function exportChat(runtime: ChatRuntime, format: string, ram = false) {
  if (!ram) await runtime.flush();
  await api("chat_export", {
    conversation: runtime.id,
    format,
    ram: ram
      ? { messages: runtime.chat.messages, draft: runtime.snapshot.text }
      : null,
  });
}
