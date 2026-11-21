import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { errorMessage } from "../api";
import { useT } from "../i18n";
import { History, Plus } from "../icons";
import { newId, type ThreadTab } from "../model";
import { IconButton, Modal } from "../ui";
import { useAttachments } from "./attachment-store";
import ProviderModelPicker, { type Choice } from "./ProviderModelPicker";
import ThreadComposer from "./ThreadComposer";
import ThreadHistory from "./ThreadHistory";
import ThreadMessages from "./ThreadMessages";
import {
  apiMessages,
  createThread,
  getDraft,
  getThreadView,
  renameThread,
  setDraft,
  threadTitle,
  useThreadView,
} from "./thread-runtime";
import { providerDefault, useChatProviders } from "./providers";
import type { ThreadSummary } from "./types";
import "./thread.css";

type TabChange = Partial<Pick<ThreadTab, "threadId" | "title">>;

export default function ThreadPane({
  tab,
  projectPath,
  workspaceId,
  onFocus,
  onChange,
}: {
  tab: ThreadTab;
  projectPath: string;
  workspaceId: string;
  onFocus: () => void;
  onClose: () => void;
  onChange: (change: TabChange) => void;
}) {
  const { t } = useT();
  const { entries, settings } = useChatProviders();
  const { view, state } = useThreadView(tab.threadId);
  const thread = state.snapshot?.thread;
  const [draft, setDraftState] = useState(() => getDraft(tab.id));
  const [choice, setChoice] = useState<Choice>();
  const [error, setError] = useState("");
  const [sending, setSending] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [renaming, setRenaming] = useState<string>();
  const [focusKey, setFocusKey] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [confirmNew, setConfirmNew] = useState(false);
  const pane = useRef<HTMLElement>(null);
  const attach = useAttachments(tab.id);
  const { importPaths } = attach;
  const op = useRef<{ key: string; id: string } | undefined>(undefined);
  const operation = (key: string) =>
    (op.current = op.current?.key === key ? op.current : { key, id: newId() })
      .id;

  useEffect(() => setChoice(undefined), [tab.threadId]);
  // Workbench routes native file drops over this pane here (see Workbench.tsx).
  useEffect(() => {
    const element = pane.current;
    if (!element) return;
    const listener = (event: Event) => {
      const { type, paths } = (
        event as CustomEvent<{ type: string; paths: string[] }>
      ).detail;
      setDragging(type === "over");
      if (type === "drop") void importPaths(paths);
    };
    element.addEventListener("thread-file-drag", listener);
    return () => element.removeEventListener("thread-file-drag", listener);
  }, [importPaths]);
  useEffect(() => {
    if (
      thread &&
      tab.threadId === thread.threadId &&
      thread.title !== tab.title
    )
      onChange({ title: thread.title });
  }, [thread, tab.threadId, tab.title, onChange]);

  const fallback = useMemo<Choice>(() => {
    if (thread) return { instanceId: thread.instanceId, model: thread.model };
    const entry = providerDefault(entries, settings);
    const defaults = settings?.defaults;
    const fb = defaults?.fallback;
    const saved =
      defaults?.instanceId === entry?.instanceId
        ? defaults?.model
        : fb?.instanceId === entry?.instanceId
          ? fb?.model
          : null;
    return {
      instanceId: entry?.instanceId ?? "",
      model: saved || entry?.models[0] || "",
    };
  }, [thread, entries, settings]);
  const value = choice ?? fallback;
  const entry = entries.find((item) => item.instanceId === value.instanceId);
  const items = useMemo(
    () => apiMessages(state.snapshot?.messages ?? [], state.live),
    [state.snapshot, state.live],
  );
  const running =
    !!state.snapshot?.messages.some(
      (message) =>
        message.state === "streaming" || message.state === "dispatching",
    ) || state.busy;
  const blockReason =
    entry && !entry.available
      ? (entry.unavailableReason ?? t("This provider is unavailable."))
      : !entry
        ? t("No provider is available. Add one in Settings → Chat AI.")
        : null;

  const edit = useCallback(
    (text: string) => {
      setDraftState(text);
      setDraft(tab.id, text);
      setFocusKey((key) => key + 1);
    },
    [tab.id],
  );
  const changeDraft = (text: string) => {
    setDraftState(text);
    setDraft(tab.id, text);
  };
  const clearDraft = (text: string) => {
    op.current = undefined;
    if (getDraft(tab.id).trim() === text) changeDraft("");
  };

  // A CLI session is bound to its CLI; an API thread cannot move to a CLI.
  const threadKind = thread
    ? entries.find((item) => item.instanceId === thread.instanceId)?.kind
    : undefined;
  const pickerEntries = !thread
    ? entries
    : threadKind === "cli"
      ? entries.filter((item) => item.instanceId === thread.instanceId)
      : entries.filter((item) => item.kind !== "cli");

  async function send() {
    const text = draft.trim();
    const ids = attach.items.map((item) => item.id);
    if (
      (!text && !ids.length) ||
      attach.importing ||
      sending ||
      !entry?.available
    )
      return;
    setError("");
    setSending(true);
    try {
      let threadId = tab.threadId;
      let summary = thread;
      if (!threadId) {
        summary = await createThread({
          operationId: operation(
            `create:${entry.instanceId}:${value.model}:${text}:${ids}`,
          ),
          project: projectPath,
          workspaceId,
          instanceId: entry.instanceId,
          model: value.model,
          title: threadTitle(text || attach.items[0].name),
          cwd: entry.kind === "cli" ? projectPath : undefined,
          effort:
            entry.instanceId === settings?.defaults.instanceId &&
            value.model === settings.defaults.model
              ? settings.defaults.effort
              : null,
        });
        threadId = summary.threadId;
        getThreadView(threadId).seed(summary);
        onChange({ threadId, title: summary.title });
      }
      if (!summary) throw new Error(t("The chat is still loading."));
      const sendView = getThreadView(threadId);
      const revision =
        sendView.state.snapshot?.thread.revision ?? summary.revision;
      await sendView.send({
        operationId: operation(
          `send:${threadId}:${entry.instanceId}:${value.model}:${text}:${ids}`,
        ),
        expectedRevision: revision,
        instanceId: entry.instanceId,
        model: value.model,
        text,
        attachments: ids,
      });
      attach.consume(ids);
      clearDraft(text);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSending(false);
    }
  }

  const stop = () => void view?.stop();

  async function commitRename() {
    const title = renaming?.trim();
    setRenaming(undefined);
    if (!title || title === (thread?.title ?? tab.title)) return;
    try {
      if (thread) {
        const next = await renameThread(thread, title);
        void view?.refresh();
        onChange({ title: next.title });
      } else onChange({ title });
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  const openThread = (next: ThreadSummary) =>
    onChange({ threadId: next.threadId, title: next.title });
  const startNewChat = () => {
    setConfirmNew(false);
    setError("");
    onChange({ threadId: null, title: "New chat" });
  };
  // Leaving a chat that has messages asks first; it stays in this workspace's history.
  const newChat = () =>
    tab.threadId && (state.snapshot?.messages.length ?? 0) > 0
      ? setConfirmNew(true)
      : startNewChat();
  const shownError = error || state.error;
  const title = thread?.title ?? tab.title;
  const fresh = !tab.threadId;

  return (
    <section
      ref={pane}
      className="thread-pane"
      aria-label={title}
      data-thread-tab-id={tab.id}
      onPointerDown={onFocus}
    >
      <header className="thread-header" data-pane-drag-handle>
        {renaming !== undefined ? (
          <input
            className="thread-title-input"
            autoFocus
            value={renaming}
            maxLength={200}
            aria-label={t("Chat title")}
            onChange={(event) => setRenaming(event.target.value)}
            onBlur={() => void commitRename()}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing) return;
              if (event.key === "Enter") void commitRename();
              if (event.key === "Escape") setRenaming(undefined);
            }}
          />
        ) : (
          <strong
            className="thread-title"
            title={t("Double-click to rename")}
            onDoubleClick={() => setRenaming(title)}
          >
            {title}
          </strong>
        )}
        <IconButton
          title={t("History")}
          aria-pressed={historyOpen}
          onClick={() => setHistoryOpen((open) => !open)}
        >
          <History size={15} />
        </IconButton>
        <IconButton title={t("New chat")} onClick={newChat}>
          <Plus size={16} />
        </IconButton>
      </header>
      <div className="thread-body">
        <div className="thread-main">
          {tab.threadId && !state.loaded && (
            <p className="thread-empty" role="status">
              {t("Loading chat…")}
            </p>
          )}
          {state.missing && (
            <div className="thread-empty" role="status">
              <p>{t("This thread was deleted.")}</p>
              <button type="button" className="button" onClick={newChat}>
                {t("New chat")}
              </button>
            </div>
          )}
          {!state.missing && fresh && !items.length && (
            <div className="thread-empty">
              <p className="thread-empty-title">{t("New chat")}</p>
              <p className="thread-note">
                {t("Pick a provider and model below, then write a message.")}
              </p>
            </div>
          )}
          {!state.missing && (!!items.length || !fresh) && (
            <ThreadMessages items={items} onEdit={edit} />
          )}
          <div className="thread-dock">
            {blockReason && !state.missing && (
              <div className="thread-notice" role="status">
                {blockReason}
              </div>
            )}
            {shownError && (
              <div className="thread-notice" role="alert">
                {shownError}
              </div>
            )}
            {!state.missing && (
              <ThreadComposer
                value={draft}
                onChange={changeDraft}
                onSend={() => void send()}
                onStop={stop}
                running={running}
                disabled={
                  sending ||
                  !entry?.available ||
                  !value.model.trim() ||
                  (tab.threadId !== null && !state.loaded)
                }
                sendKey={settings?.behavior.sendKey ?? "enter"}
                placeholder={t("Message")}
                focusKey={focusKey || undefined}
                attachments={attach.items}
                importing={attach.importing}
                attachmentError={attach.error}
                onPick={() => void attach.pick()}
                onPasteImages={(files) => void attach.importImages(files)}
                onRemoveAttachment={attach.remove}
                picker={
                  <ProviderModelPicker
                    entries={pickerEntries}
                    value={value}
                    onChange={setChoice}
                    disabled={running || sending}
                  />
                }
              />
            )}
          </div>
        </div>
        {historyOpen && (
          <ThreadHistory
            project={projectPath}
            workspaceId={workspaceId}
            activeId={tab.threadId}
            onOpen={openThread}
            onRenamed={(next) => {
              if (next.threadId === tab.threadId)
                onChange({ title: next.title });
            }}
            onDeleted={(id) => {
              if (id === tab.threadId) startNewChat();
            }}
          />
        )}
      </div>
      {dragging && (
        <div className="thread-drop" aria-hidden="true">
          {t("Drop files to attach")}
        </div>
      )}
      {confirmNew && (
        <Modal
          title={t("Start a new chat?")}
          onClose={() => setConfirmNew(false)}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p>{t("The current chat stays in this workspace's history.")}</p>
            </div>
            <div className="dialog-actions">
              <button
                type="button"
                className="button"
                onClick={() => setConfirmNew(false)}
              >
                {t("Cancel")}
              </button>
              <button
                type="button"
                className="button button-primary"
                autoFocus
                onClick={startNewChat}
              >
                {t("New chat")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </section>
  );
}
