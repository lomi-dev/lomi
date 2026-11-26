import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { api, errorMessage } from "../api";
import { useT } from "../i18n";
import { History, Plus } from "../icons";
import { registerChatUsage } from "../chat-usage";
import { newId, type ThreadTab } from "../model";
import { IconButton, Modal } from "../ui";
import { useAttachments } from "./attachment-store";
import EffortSelect, { loadEfforts, useEfforts } from "./EffortSelect";
import { hasPendingApproval, isPendingQuestion } from "./activity";
import { isNewCommand } from "./commands";
import { effortFor, initialEffort } from "./effort";
import ProviderModelPicker, { type Choice } from "./ProviderModelPicker";
import { decide, editable } from "./send-queue";
import ThreadComposer from "./ThreadComposer";
import ThreadHistory from "./ThreadHistory";
import ThreadMessages from "./ThreadMessages";
import ThreadPermissions from "./ThreadPermissions";
import {
  apiMessages,
  createThread,
  getDraft,
  getThreadView,
  answerQuestion,
  respondToApproval,
  copyAttachments,
  steerThread,
  steerUnknown,
  steerUnsupported,
  renameThread,
  updateThread,
  setDraft,
  threadTitle,
  useThreadView,
} from "./thread-runtime";
import { providerDefault, useChatProviders } from "./providers";
import type { ApprovalDecision, Permissions, ThreadSummary } from "./types";
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
  const { t, tp } = useT();
  const queueId = useId();
  const { entries, settings } = useChatProviders();
  const { view, state } = useThreadView(tab.threadId);
  const thread = state.snapshot?.thread;
  const [draft, setDraftState] = useState(() => getDraft(tab.id));
  const [choice, setChoice] = useState<Choice>();
  // New chats only; undefined keeps the prefill from the defaults.
  const [effortChoice, setEffortChoice] = useState<string | null>();
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
  const steering = useRef(false);
  const op = useRef<{ key: string; id: string } | undefined>(undefined);
  const operation = (key: string) =>
    (op.current = op.current?.key === key ? op.current : { key, id: newId() })
      .id;

  useEffect(() => {
    setChoice(undefined);
    setEffortChoice(undefined);
  }, [tab.threadId]);
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
  // Workbench shortcuts (focus message field, open model picker) arrive here.
  useEffect(() => {
    const element = pane.current;
    if (!element) return;
    const listener = (event: Event) => {
      const command = (event as CustomEvent<string>).detail;
      if (command === "focusChat")
        element
          .querySelector<HTMLElement>(".thread-composer .ProseMirror")
          ?.focus();
      else
        element
          .querySelector<HTMLButtonElement>(
            ".thread-picker-trigger:not(:disabled)",
          )
          ?.click();
    };
    element.addEventListener("thread-command", listener);
    return () => element.removeEventListener("thread-command", listener);
  }, []);
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
  const usageCli = entry?.kind === "cli" ? entry.cli : null;
  useEffect(() => registerChatUsage(tab.id, usageCli), [tab.id, usageCli]);
  const efforts = useEfforts(entry, value.model);
  const effort = effortFor(
    thread
      ? thread.effort
      : effortChoice === undefined
        ? initialEffort(settings?.defaults, value)
        : effortChoice,
    efforts,
  );
  const items = useMemo(
    () =>
      apiMessages(
        state.snapshot?.messages ?? [],
        state.live,
        state.liveActivity,
      ),
    [state.snapshot, state.live, state.liveActivity],
  );
  const running =
    !!state.snapshot?.messages.some(
      (message) =>
        message.state === "streaming" || message.state === "dispatching",
    ) || state.busy;
  const lastState = state.snapshot?.messages
    .filter((message) => message.role === "assistant")
    .at(-1)?.state;
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

  // Steering is offered for the CLIs that accept messages mid-turn.
  const steerable = entry?.cli === "claude" || entry?.cli === "codex";

  // A CLI session is bound to its CLI; an API thread cannot move to a CLI.
  const threadKind = thread
    ? entries.find((item) => item.instanceId === thread.instanceId)?.kind
    : undefined;
  const pickerEntries = !thread
    ? entries
    : threadKind === "cli"
      ? entries.filter((item) => item.instanceId === thread.instanceId)
      : entries.filter((item) => item.kind !== "cli");

  async function send(forceQueue = false) {
    const text = draft.trim();
    const ids = attach.items.map((item) => item.id);
    if (isNewCommand(text) && !ids.length) {
      op.current = undefined;
      changeDraft("");
      return newChat();
    }
    if (
      (!text && !ids.length) ||
      attach.importing ||
      sending ||
      !entry?.available
    )
      return;
    if (running && view && thread) {
      if (steerable && !forceQueue && text && !ids.length) {
        if (steering.current) return;
        steering.current = true;
        // Clear first so a second Enter cannot resend it; text typed meanwhile stays.
        op.current = undefined;
        changeDraft("");
        try {
          await steerThread(thread.threadId, text);
          return;
        } catch (cause) {
          const code = errorMessage(cause);
          if (code === steerUnknown)
            return setError(t("The message may not have reached the agent."));
          if (code !== steerUnsupported) {
            // Definite failure: give the text back without wiping new typing.
            const typed = getDraft(tab.id);
            changeDraft(typed.trim() ? `${text}\n${typed}` : text);
            return setError(code);
          }
          // Unsupported: queue it like any follow-up (the draft is already clear).
          view.enqueue({ id: newId(), text, attachments: [] });
          return;
        } finally {
          steering.current = false;
        }
      }
      // The backend takes one reply at a time; hold this one until it ends.
      view.enqueue({
        id: newId(),
        text,
        attachments: attach.items.filter((item) => ids.includes(item.id)),
      });
      attach.consume(ids);
      op.current = undefined;
      changeDraft("");
      return;
    }
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
          effort,
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

  // Sends the text of a failed turn again; false while busy so the caller can wait.
  function resend(text: string, files: readonly { id: string }[]) {
    if (!view || !thread || running || sending || !entry?.available)
      return false;
    setError("");
    setSending(true);
    // Sent files are bound to their message; resend fresh copies.
    const copies = files.length
      ? copyAttachments(
          thread.threadId,
          files.map((file) => file.id),
        )
      : Promise.resolve([]);
    copies
      .then((fresh) =>
        view.send({
          operationId: newId(),
          expectedRevision:
            view.state.snapshot?.thread.revision ?? thread.revision,
          instanceId: entry.instanceId,
          model: value.model,
          text,
          attachments: fresh.map((file) => file.id),
        }),
      )
      .catch((cause) => setError(errorMessage(cause)))
      .finally(() => setSending(false));
    return true;
  }

  // Stable callbacks keep memoized messages from re-rendering on every token.
  const latest = useRef({ resend, view });
  latest.current = { resend, view };
  const onResend = useCallback(
    (text: string, files: readonly { id: string }[]) =>
      latest.current.resend(text, files),
    [],
  );
  const loadEarlier = useCallback(
    () => void latest.current.view?.loadEarlier(),
    [],
  );

  const draining = useRef(false);
  async function sendQueued() {
    const item = state.queued[0];
    if (!view || !thread || !item || draining.current) return;
    if (!entry?.available) return view.setPaused(true);
    draining.current = true;
    setError("");
    setSending(true);
    try {
      await view.send({
        operationId: item.id,
        expectedRevision:
          view.state.snapshot?.thread.revision ?? thread.revision,
        instanceId: entry.instanceId,
        model: value.model,
        text: item.text,
        attachments: item.attachments.map((file) => file.id),
      });
      view.dequeue(item.id);
    } catch (cause) {
      // Keep the entry at the head and wait for "Send next".
      setError(errorMessage(cause));
      view.setPaused(true);
    } finally {
      draining.current = false;
      setSending(false);
    }
  }
  // Runs on every state change; `decide` makes it idempotent.
  useEffect(() => {
    if (!view || !state.loaded || sending) return;
    const action = decide({
      wasRunning: view.wasRunning,
      running,
      lastState,
      queued: state.queued.length,
      paused: state.paused,
    });
    view.wasRunning = running;
    if (action === "pause") view.setPaused(true);
    else if (action === "send") void sendQueued();
  });
  const editQueued = (id: string) => {
    const item = editable(
      state.queued,
      id,
      !draft.trim() && !attach.items.length,
    );
    if (!item || !view) return;
    view.dequeue(id);
    attach.restore(item.attachments);
    edit(item.text);
  };
  const removeQueued = (id: string) => {
    for (const file of state.queued.find((item) => item.id === id)
      ?.attachments ?? [])
      void api("ai_attachment_discard", { id: file.id }).catch(() => {});
    view?.dequeue(id);
  };

  async function changePermissions(next: Permissions) {
    if (!thread) return;
    try {
      await updateThread(thread, { permissions: next });
      await view?.refresh();
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  async function changeEffort(next: string | null) {
    if (!thread) return setEffortChoice(next);
    try {
      await updateThread(thread, { effort: next });
      await view?.refresh();
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }
  // A new model keeps the effort only when it supports it; a new chat re-prefills from the defaults.
  async function changeModel(next: Choice) {
    setChoice(next);
    if (!thread) return setEffortChoice(undefined);
    const nextEntry = entries.find(
      (item) => item.instanceId === next.instanceId,
    );
    if (!thread.effort || !nextEntry?.cli) return;
    const supported = await loadEfforts(nextEntry.cli, next.model);
    if (effortFor(thread.effort, supported) === null) await changeEffort(null);
  }

  const stop = () => void view?.stop();
  const threadId = tab.threadId;
  const respond = useCallback(
    (requestId: string, decision: ApprovalDecision) =>
      threadId
        ? respondToApproval(threadId, requestId, decision)
        : Promise.resolve(),
    [threadId],
  );
  const answer = useCallback(
    (requestId: string, answers: Record<string, string[]> | null) =>
      threadId
        ? answerQuestion(threadId, requestId, answers)
        : Promise.resolve(),
    [threadId],
  );
  const waiting = items.some(
    (item) => item.streaming && hasPendingApproval(item.activity),
  );
  const pendingQuestion =
    items
      .filter((item) => item.streaming)
      .flatMap((item) => item.activity)
      .find(isPendingQuestion)?.question ?? null;

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
            <ThreadMessages
              items={items}
              threadId={tab.threadId}
              cwd={state.snapshot?.thread.cwd ?? projectPath}
              provider={
                entries.find((item) => item.instanceId === thread?.instanceId)
                  ?.label ?? ""
              }
              hasMore={!!state.snapshot?.hasMore}
              loadingEarlier={state.loadingEarlier}
              onLoadEarlier={loadEarlier}
              onEdit={edit}
              onResend={onResend}
              onRespond={respond}
            />
          )}
          <div className="thread-dock">
            {blockReason && !state.missing && (
              <div className="thread-notice" role="status">
                {blockReason}
              </div>
            )}
            {waiting && (
              <div className="thread-notice" role="status">
                {t("Waiting for your approval")}
              </div>
            )}
            {shownError && (
              <div className="thread-notice" role="alert">
                {shownError}
              </div>
            )}
            {state.queued.length > 0 && (
              <div
                className="thread-queue"
                role="region"
                aria-label={t("Queued messages")}
              >
                {state.paused && (
                  <div className="thread-queue-paused" role="status">
                    <span>{t("Queue paused")}</span>
                    <button
                      type="button"
                      className="button"
                      disabled={running || sending}
                      onClick={() => view?.setPaused(false)}
                    >
                      {t("Send next")}
                    </button>
                  </div>
                )}
                <ul>
                  {state.queued.map((item) => {
                    const files = item.attachments.length;
                    const composerEmpty = !draft.trim() && !attach.items.length;
                    return (
                      <li key={item.id} className="thread-queue-item">
                        <span
                          id={`${queueId}-${item.id}`}
                          className="thread-queue-text"
                        >
                          {item.text || item.attachments[0]?.name}
                        </span>
                        {files > 0 && (
                          <span className="thread-queue-files">
                            {tp(files, "{count} file", "{count} files")}
                          </span>
                        )}
                        <button
                          type="button"
                          className="button"
                          aria-describedby={`${queueId}-${item.id}`}
                          disabled={!composerEmpty}
                          title={
                            !composerEmpty
                              ? t(
                                  "Clear the message field to edit this message.",
                                )
                              : undefined
                          }
                          onClick={() => editQueued(item.id)}
                        >
                          {t("Edit")}
                        </button>
                        <button
                          type="button"
                          className="button"
                          aria-describedby={`${queueId}-${item.id}`}
                          onClick={() => removeQueued(item.id)}
                        >
                          {t("Remove")}
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </div>
            )}
            {!state.missing && (
              <ThreadComposer
                value={draft}
                onChange={changeDraft}
                onSend={(queue) => void send(queue)}
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
                lastUserText={
                  items
                    .filter((item) => item.role === "user" && item.text)
                    .at(-1)?.text
                }
                attachments={attach.items}
                importing={attach.importing}
                attachmentError={attach.error}
                onPick={() => void attach.pick()}
                onPasteImages={(files) => void attach.importImages(files)}
                onRemoveAttachment={attach.remove}
                mentionThreadId={threadKind === "cli" ? tab.threadId : null}
                commandSource={
                  (thread ? threadKind : entry?.kind) === "cli"
                    ? {
                        threadId: tab.threadId,
                        instanceId: thread?.instanceId ?? value.instanceId,
                        project: projectPath,
                      }
                    : null
                }
                commandsKey={`${items.length}:${running}`}
                question={pendingQuestion}
                onAnswer={answer}
                picker={
                  <ProviderModelPicker
                    entries={pickerEntries}
                    value={value}
                    onChange={(next) => void changeModel(next)}
                    disabled={running || sending}
                  />
                }
                permissions={
                  thread?.kind === "cli" ? (
                    <ThreadPermissions
                      value={thread.permissions}
                      disabled={running || sending}
                      onChange={(next) => void changePermissions(next)}
                    />
                  ) : undefined
                }
                effort={
                  <EffortSelect
                    className="thread-effort"
                    efforts={efforts}
                    value={effort}
                    disabled={running || sending}
                    onChange={(next) => void changeEffort(next)}
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
