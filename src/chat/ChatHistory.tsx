import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { errorMessage, native } from "../api";
import ContextMenu from "../ContextMenu";
import { DisclosureSummary, IconButton, Modal } from "../ui";
import {
  Download,
  Ellipsis,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Search,
  Trash2,
  X,
} from "../icons";
import type { ChatTab } from "../model";
import { conversationTitle, replaceChat } from "./chat-service";
import { existing, main } from "./chat-runtime";
import type { ChatRuntime } from "./chat-runtime";
import type { Conversation } from "./types";

export default function ChatHistory({
  id,
  open,
  tab,
  runtime,
  onClose,
  onExport,
}: {
  id: string;
  open: boolean;
  tab: ChatTab;
  runtime: ChatRuntime;
  onClose: (restoreFocus?: boolean) => void;
  onExport: (format: string) => Promise<void>;
}) {
  const nameId = useId();
  const search = useRef<HTMLInputElement>(null);
  const menuTrigger = useRef<HTMLButtonElement | null>(null);
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<Conversation[]>([]);
  const [error, setError] = useState("");
  const [more, setMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const working = useRef(false);
  const [revision, setRevision] = useState(0);
  const [rename, setRename] = useState<Conversation | null>(null);
  const [deleting, setDeleting] = useState<Conversation | null>(null);
  const [menu, setMenu] = useState<{
    item: Conversation;
    x: number;
    y: number;
  }>();
  const requestKey = JSON.stringify([
    open,
    query,
    revision,
    tab.conversationId,
    runtime.snapshot.loaded?.conversation.title,
  ]);
  const latest = useRef(requestKey);
  const sequence = useRef(0);
  const previousQuery = useRef(query);
  latest.current = requestKey;
  const load = useCallback(
    async (offset = 0) => {
      setLoading(true);
      const request = ++sequence.current;
      const current = () =>
        latest.current === requestKey && sequence.current === request;
      try {
        const values = await main<Conversation[]>({
          action: "list",
          query,
          workspace: null,
          project: null,
          offset,
        });
        if (!current()) return;
        setItems((old) =>
          offset
            ? [
                ...old,
                ...values.filter(
                  (item) => !old.some((previous) => previous.id === item.id),
                ),
              ]
            : values,
        );
        setMore(values.length === 50);
      } catch (error) {
        if (current()) setError(errorMessage(error));
      } finally {
        if (current()) setLoading(false);
      }
    },
    [query, requestKey],
  );
  useLayoutEffect(() => {
    if (open) search.current?.focus({ preventScroll: true });
  }, [open]);
  useEffect(() => {
    if (!open) return;
    setLoading(true);
    setMore(false);
    if (previousQuery.current !== query) setItems([]);
    previousQuery.current = query;
    const timer = window.setTimeout(() => void load(), 150);
    return () => {
      window.clearTimeout(timer);
      sequence.current++;
    };
  }, [load, open, query]);
  useEffect(() => {
    if (!native || !open) return;
    const stop = listen("chat-history-changed", () =>
      setRevision((value) => value + 1),
    );
    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, [open]);
  const run = async (operation: () => Promise<unknown>) => {
    if (working.current) return;
    working.current = true;
    setBusy(true);
    setError("");
    try {
      await operation();
    } catch (error) {
      setError(errorMessage(error));
    } finally {
      working.current = false;
      setBusy(false);
    }
  };
  const showMenu = (
    item: Conversation,
    trigger: HTMLButtonElement,
    x?: number,
    y?: number,
  ) => {
    if (busy) return;
    menuTrigger.current = trigger;
    const bounds = trigger.getBoundingClientRect();
    setMenu({ item, x: x ?? bounds.left, y: y ?? bounds.bottom });
  };
  const closeMenu = () => {
    setMenu(undefined);
    menuTrigger.current?.focus({ preventScroll: true });
  };
  const group = (pinned: boolean, label: string) => {
    const conversations = items.filter((item) => !!item.pinned === pinned);
    if (!conversations.length) return null;
    return (
      <section className="chat-history-group" aria-label={label}>
        <h3>{label}</h3>
        <ul>
          {conversations.map((item) => (
            <li
              key={item.id}
              className={`chat-history-item${item.id === tab.conversationId ? " is-current" : ""}`}
            >
              <button
                className="chat-history-conversation"
                aria-current={
                  item.id === tab.conversationId ? "true" : undefined
                }
                title={`${item.title}\n${item.origin.projectName} / ${item.origin.workspaceName} · ${new Date(item.updatedAt).toLocaleDateString()}`}
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    if (item.id !== tab.conversationId)
                      await replaceChat(tab, item);
                    onClose(false);
                  })
                }
                onContextMenu={(event) => {
                  event.preventDefault();
                  showMenu(
                    item,
                    event.currentTarget,
                    event.clientX,
                    event.clientY,
                  );
                }}
                onKeyDown={(event) => {
                  if (
                    event.key === "ContextMenu" ||
                    (event.shiftKey && event.key === "F10")
                  ) {
                    event.preventDefault();
                    showMenu(item, event.currentTarget);
                  }
                }}
              >
                <span>{item.title}</span>
                {item.pinned && <Pin size={12} aria-hidden="true" />}
              </button>
              <IconButton
                title={`Actions for ${item.title}`}
                disabled={busy}
                aria-haspopup="menu"
                onClick={(event) => showMenu(item, event.currentTarget)}
              >
                <Ellipsis size={16} />
              </IconButton>
            </li>
          ))}
        </ul>
      </section>
    );
  };
  return (
    <>
      <aside
        id={id}
        className="chat-history-sidebar"
        aria-label="Chat history"
        onKeyDown={(event) => {
          if (event.key === "Escape" && !menu && !rename && !deleting) {
            event.preventDefault();
            event.stopPropagation();
            onClose();
          }
        }}
      >
        <header className="chat-history-heading">
          <h2>Chat AI</h2>
          <IconButton title="Close history" onClick={() => onClose()}>
            <X size={16} />
          </IconButton>
        </header>
        <button
          className="chat-history-new"
          disabled={busy}
          onClick={() =>
            void run(async () => {
              await replaceChat(tab);
              onClose(false);
            })
          }
        >
          <Plus size={17} aria-hidden="true" /> New conversation
        </button>
        <div className="chat-history-search">
          <Search size={14} aria-hidden="true" />
          <input
            ref={search}
            type="search"
            aria-label="Search conversations"
            placeholder="Search conversations…"
            value={query}
            maxLength={1024}
            onChange={(event) => {
              setError("");
              setQuery(event.target.value);
            }}
          />
        </div>
        <div className="chat-history-list" aria-busy={loading}>
          {group(true, "Pinned")}
          {group(false, "Recent")}
          {loading && (
            <p className="chat-history-empty" role="status">
              Loading conversations…
            </p>
          )}
          {!loading && !error && !items.length && (
            <p className="chat-history-empty">
              {query
                ? "No conversations match your search."
                : "No conversations yet."}
            </p>
          )}
          {more && (
            <button
              className="button chat-history-more"
              disabled={busy || loading}
              onClick={() => void load(items.length)}
            >
              Load more
            </button>
          )}
        </div>
        {error && !rename && !deleting && (
          <p className="chat-history-error" role="alert">
            {error}
          </p>
        )}
        {runtime.snapshot.loaded && (
          <details className="chat-history-export">
            <DisclosureSummary>Export conversation</DisclosureSummary>
            <div>
              <button
                className="button"
                disabled={busy}
                onClick={() => void run(() => onExport("markdown"))}
              >
                <Download size={13} /> Markdown
              </button>
              <button
                className="button"
                disabled={busy}
                onClick={() => void run(() => onExport("json"))}
              >
                JSON
              </button>
            </div>
            <p>Includes attachment descriptions, without binary files.</p>
          </details>
        )}
      </aside>
      {menu && (
        <ContextMenu
          label="Conversation actions"
          x={menu.x}
          y={menu.y}
          onClose={closeMenu}
          actions={[
            {
              label: "Zmień nazwę",
              icon: <Pencil size={14} />,
              run: () => {
                setError("");
                setRename(menu.item);
              },
            },
            {
              label: menu.item.pinned ? "Odepnij" : "Przypnij",
              icon: menu.item.pinned ? <PinOff size={14} /> : <Pin size={14} />,
              run: () =>
                void run(async () => {
                  await main({
                    action: "pin",
                    id: menu.item.id,
                    pinned: !menu.item.pinned,
                  });
                  setRevision((value) => value + 1);
                  search.current?.focus();
                }),
            },
            null,
            {
              label: "Usuń konwersację",
              icon: <Trash2 size={14} />,
              danger: true,
              run: () => {
                setError("");
                setDeleting(menu.item);
              },
            },
          ]}
        />
      )}
      {rename && (
        <Modal
          title="Rename conversation"
          className="chat-dialog"
          onClose={() => {
            if (!busy) {
              setRename(null);
              setError("");
            }
          }}
        >
          <form
            className="dialog-form"
            onSubmit={(event) => {
              event.preventDefault();
              void run(async () => {
                const active = existing(rename.id);
                if (active) await active.rename(rename.title);
                else {
                  await main({
                    action: "rename",
                    id: rename.id,
                    title: rename.title,
                  });
                  conversationTitle(rename.id, rename.title);
                }
                setRename(null);
                requestAnimationFrame(() => search.current?.focus());
                setRevision((value) => value + 1);
              });
            }}
          >
            <label htmlFor={nameId}>Conversation name</label>
            <input
              id={nameId}
              autoFocus
              required
              maxLength={256}
              disabled={busy}
              value={rename.title}
              onFocus={(event) => event.currentTarget.select()}
              onChange={(event) =>
                setRename({ ...rename, title: event.target.value })
              }
            />
            {error && <p role="alert">{error}</p>}
            <div className="dialog-actions">
              <button
                className="button"
                type="button"
                disabled={busy}
                onClick={() => {
                  setRename(null);
                  setError("");
                }}
              >
                Cancel
              </button>
              <button
                className="button chat-primary-button"
                disabled={busy || !rename.title.trim()}
              >
                Save name
              </button>
            </div>
          </form>
        </Modal>
      )}
      {deleting && (
        <Modal
          title="Delete conversation?"
          className="chat-dialog"
          onClose={() => {
            if (!busy) {
              setDeleting(null);
              setError("");
            }
          }}
        >
          <div className="dialog-form">
            <p>
              Permanently delete “{deleting.title}”, its variants and unused
              attachments? All views of this conversation will close.
            </p>
            {error && <p role="alert">{error}</p>}
            <div className="dialog-actions">
              <button
                className="button"
                disabled={busy}
                onClick={() => {
                  setDeleting(null);
                  setError("");
                }}
              >
                Cancel
              </button>
              <button
                className="button text-error"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await main({ action: "delete", id: deleting.id });
                    setDeleting(null);
                    requestAnimationFrame(() => search.current?.focus());
                    setRevision((value) => value + 1);
                  })
                }
              >
                Delete conversation
              </button>
            </div>
          </div>
        </Modal>
      )}
    </>
  );
}
