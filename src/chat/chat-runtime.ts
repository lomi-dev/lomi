import type { UIMessage } from "ai";
import { Chat } from "@ai-sdk/react";
import { api, errorMessage } from "../api";
import { newId } from "../model";
import {
  conversationTitle,
  chatActivity,
  retainedChatIds,
  registerChatRuntime,
} from "./chat-service";
import { NativeTransport } from "./chat-transport";
import type { AgentGeneration, Packet } from "./chat-transport";
import type {
  Accepted,
  Attachment,
  Config,
  Draft,
  Loaded,
  Preferences,
  Start,
} from "./types";
const entries = new Map<string, ChatRuntime>();
export const existing = (id: string) => entries.get(id);
export const getChat = (id: string) => {
  let entry = entries.get(id);
  if (!entry) {
    entry = new ChatRuntime(id);
    entries.set(id, entry);
  }
  return entry;
};
export function retain(ids: Set<string>) {
  for (const [id, entry] of entries)
    if (
      !ids.has(id) &&
      !entry.snapshot.busy &&
      !entry.dirty &&
      !entry.snapshot.storageFailed
    ) {
      entry.dispose();
      entries.delete(id);
    }
}
export const main = <T>(input: object) => api<T>("chat_main", { input });
export const readPreferences = () => api<Preferences>("chat_preferences");
const failures: Record<string, string> = {
  auth: "The provider rejected the API key. Update it in Settings → Chat AI.",
  quota: "The provider reports insufficient quota or credit.",
  "rate-limit": "The provider rate limit was reached. Wait before retrying.",
  model: "This model is unavailable for the connection. Choose another model.",
  "context-limit":
    "This conversation exceeds the provider's context limit. Start a new conversation.",
  timeout: "The provider timed out. The partial response was saved.",
  network:
    "The provider could not be reached. Check your connection and retry.",
  permission:
    "The provider denied access to this model. Check the connection permissions.",
  "unsupported-input":
    "The provider rejected a message or parameter. Review the model and attachments.",
  process:
    "The AI runtime stopped. The partial response was saved; Retry starts a new request.",
  "response-limit":
    "The response reached the local size limit. Its saved portion is available.",
};
interface Snapshot {
  storageFailed: boolean;
  unsentText?: string;
  loaded?: Loaded;
  text: string;
  busy: boolean;
  loading: boolean;
  error: string;
  status: string;
  preferences?: Preferences;
}
export class ChatRuntime {
  readonly transport = new NativeTransport();
  readonly chat: Chat<UIMessage>;
  snapshot: Snapshot = {
    storageFailed: false,
    text: "",
    busy: false,
    loading: true,
    error: "",
    status: "",
  };
  private listeners = new Set<() => void>();
  private serial: Promise<unknown> = Promise.resolve();
  private timer?: ReturnType<typeof setTimeout>;
  private localRevision = 0;
  private savedRevision = 0;
  private resuming = false;
  private ended = false;
  private terminalPacket?: Packet;
  private intent?: Start;
  private disposed = false;
  private views = 0;
  private unread = false;
  show() {
    this.views++;
    this.unread = false;
    this.publishActivity();
    return () => {
      this.views--;
    };
  }
  private publishActivity() {
    chatActivity(
      this.id,
      this.snapshot.error
        ? "Chat error"
        : this.snapshot.busy
          ? "Generating"
          : this.unread
            ? "New response"
            : "",
    );
  }
  readonly scroll = new Map<string, { top: number; bottom: boolean }>();
  readonly attachments = new Map<
    string,
    { attachment: Attachment; url: string }
  >();
  readonly ready: Promise<void>;
  get dirty() {
    return (
      this.snapshot.storageFailed ||
      !!this.snapshot.unsentText ||
      this.localRevision !== this.savedRevision
    );
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  getSnapshot = () => this.snapshot;
  private update(value: Partial<Snapshot>) {
    if (this.disposed) return;
    this.snapshot = { ...this.snapshot, ...value };
    this.publishActivity();
    this.listeners.forEach((listener) => listener());
  }
  report = (error: unknown) => this.update({ error: errorMessage(error) });
  constructor(readonly id: string) {
    this.chat = new Chat({
      id,
      transport: this.transport,
      onError: this.report,
      onFinish: () => {
        if (this.resuming)
          setTimeout(() => {
            void this.resume().catch(this.report);
          }, 0);
        else if (this.ended) {
          this.ended = false;
          void this.enqueue(async () => {
            this.update({ busy: false });
            if (this.terminalPacket?.type !== "storage-error")
              await this.reload();
            retain(retainedChatIds());
          }).catch(this.report);
        }
      },
    });
    this.transport.resync = () => {
      this.resuming = true;
    };
    this.transport.ended = (packet) => this.terminal(packet);
    this.ready = this.load().catch((error) =>
      this.update({ loading: false, error: errorMessage(error) }),
    );
  }
  private async load() {
    const loaded = await main<Loaded>({ action: "load", id: this.id });
    this.chat.messages = loaded.messages;
    this.update({
      loaded,
      text: loaded.draft.text,
      loading: false,
      busy: loaded.request?.status === "active",
    });
    conversationTitle(this.id, loaded.conversation.title);
    void this.refreshPreferences();
    if (loaded.request?.status === "active") {
      this.transport.requestId = loaded.request.id;
      void this.resume().catch(this.report);
    }
  }
  async recover(reset: boolean) {
    await api("chat_recover", { target: "history", reset });
    this.update({ error: "", loading: true });
    await this.load();
  }
  async refreshPreferences() {
    try {
      const preferences = await readPreferences();
      this.update({ preferences });
      const loaded = this.snapshot.loaded;
      if (
        loaded &&
        !loaded.conversation.config.configured &&
        preferences.defaults.connectionId &&
        preferences.defaults.model &&
        !this.snapshot.busy
      ) {
        await this.configure({ ...preferences.defaults, configured: true });
      }
    } catch (error) {
      this.report(error);
    }
  }
  private enqueue<T>(action: () => Promise<T>): Promise<T> {
    const next = this.serial.then(action);
    this.serial = next.catch(() => {});
    return next;
  }
  private async reload() {
    const loaded = await main<Loaded>({ action: "load", id: this.id });
    this.update({ loaded });
    conversationTitle(this.id, loaded.conversation.title);
    if (!this.snapshot.busy || this.ended) this.chat.messages = loaded.messages;
  }
  setText(text: string) {
    this.localRevision++;
    this.update({ text });
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      void this.flush().catch(this.report);
    }, 300);
  }
  private async saveDraft() {
    if (this.snapshot.unsentText)
      throw Error(
        "Resolve the unsent message before saving or closing this conversation.",
      );
    if (this.localRevision === this.savedRevision || !this.snapshot.loaded)
      return;
    const revision = this.localRevision;
    let draft: Draft;
    try {
      draft = await main<Draft>({
        action: "draft",
        id: this.id,
        text: this.snapshot.text,
        expected: this.snapshot.loaded.draft.revision,
      });
    } catch (error) {
      this.update({ storageFailed: true, status: "Draft not saved" });
      void this.stop().catch(this.report);
      throw error;
    }
    this.savedRevision = revision;
    this.update({ loaded: { ...this.snapshot.loaded, draft } });
  }
  flush() {
    clearTimeout(this.timer);
    return this.enqueue(async () => {
      await this.ready;
      await this.saveDraft();
    });
  }
  async applyAgentDraft(
    expectedRevision: string,
    expectedConversationRevision: string,
    text: string,
    commit: () => Promise<Draft>,
  ): Promise<{ draft: Draft; humanTextRetained: boolean }> {
    return this.enqueue(async () => {
      await this.ready;
      const loaded = this.snapshot.loaded;
      if (!loaded || this.disposed) throw Error("TARGET_NOT_FOUND");
      if (
        this.dirty ||
        String(loaded.draft.revision) !== expectedRevision ||
        String(loaded.conversation.revision) !== expectedConversationRevision
      )
        throw Error("REVISION_CONFLICT");
      const local = this.localRevision;
      const draft = await commit();
      if (
        draft.text !== text ||
        !Number.isSafeInteger(draft.revision) ||
        draft.revision <= loaded.draft.revision
      )
        throw Error("OUTCOME_UNKNOWN");
      if (this.disposed || !this.snapshot.loaded)
        throw Error("TARGET_NOT_FOUND");
      const humanTextRetained = this.localRevision !== local;
      if (!humanTextRetained) {
        this.localRevision++;
        this.savedRevision = this.localRevision;
      }
      // A keystroke during native persistence remains the live draft; its next
      // autosave uses the newly committed native revision instead of old CAS.
      this.update({
        loaded: { ...this.snapshot.loaded, draft },
        ...(humanTextRetained ? {} : { text }),
      });
      return { draft, humanTextRetained };
    });
  }
  async configure(config: Config) {
    return this.enqueue(async () => {
      const loaded = this.snapshot.loaded;
      if (!loaded) return;
      const conversation = await main<Loaded["conversation"]>({
        action: "configure",
        id: this.id,
        config,
        expected: loaded.conversation.revision,
      });
      this.update({ loaded: { ...loaded, conversation } });
    });
  }
  async send(
    action: Start["action"] = "send",
    targetId: string | null = null,
    text?: string,
    agent?: { input: Start; authorization: AgentGeneration },
  ) {
    if (
      this.snapshot.busy ||
      this.snapshot.unsentText ||
      this.snapshot.storageFailed
    ) {
      if (agent) throw Error("TARGET_BUSY");
      return;
    }
    this.terminalPacket = undefined;
    this.update({ busy: true, error: "", status: "Preparing request…" });
    try {
      await this.enqueue(async () => {
        await this.ready;
        if (agent) {
          if (this.dirty || this.snapshot.text !== agent.input.text)
            throw Error("REVISION_CONFLICT");
        } else await this.saveDraft();
        const loaded = this.snapshot.loaded;
        if (!loaded) throw Error("This conversation is unavailable.");
        if (
          agent &&
          (agent.input.conversationId !== this.id ||
            agent.input.expectedRevision !== loaded.conversation.revision ||
            agent.input.draftRevision !== loaded.draft.revision)
        )
          throw Error("REVISION_CONFLICT");
        const input: Start = agent?.input ?? {
          requestId: newId(),
          assistantId: newId(),
          userId: newId(),
          conversationId: this.id,
          expectedRevision: loaded.conversation.revision,
          draftRevision: loaded.draft.revision,
          action,
          targetId,
          text: text ?? this.snapshot.text,
        };
        const accepted = new Promise<Accepted>((resolve, reject) => {
          this.transport.accepted = resolve;
          this.transport.failed = reject;
        });
        this.intent = input;
        this.transport.input = input;
        this.transport.agent = agent?.authorization;
        const revision = this.localRevision;
        if (action === "send") {
          this.update({ text: "" });
          this.localRevision++;
        }
        const prefix =
          action === "send"
            ? this.chat.messages
            : loaded.messages.slice(
                0,
                Math.max(
                  0,
                  loaded.messages.findIndex((m) => m.id === targetId),
                ) +
                  (action === "retry" &&
                  loaded.messages.find((m) => m.id === targetId)?.role ===
                    "user"
                    ? 1
                    : 0),
              );
        this.chat.messages = prefix;
        // The native input is authoritative. This message only initializes the SDK parser.
        void this.chat
          .sendMessage({
            id: input.userId,
            role: "user",
            parts: [{ type: "text", text: input.text }],
          })
          .catch(this.report);
        try {
          const ack = await accepted;
          this.intent = undefined;
          const fresh = await main<Loaded>({ action: "load", id: this.id });
          this.update({
            loaded: {
              ...fresh,
              draft: { ...fresh.draft, revision: ack.draftRevision },
            },
            status: this.terminalPacket ? this.snapshot.status : "Generating…",
          });
          conversationTitle(this.id, fresh.conversation.title);
          if (action === "send" && this.localRevision === revision + 1)
            this.savedRevision = this.localRevision;
          // Retry's user ID is retained by Rust, and may differ from the reserved ID.
          this.chat.messages = [
            ...fresh.messages.filter((m) => m.id !== input.assistantId),
            ...this.chat.messages.filter((m) => m.id === input.assistantId),
          ];
          await this.saveDraft();
          if (ack.repeated) {
            this.resuming = true;
            this.transport.disconnect?.();
          }
        } catch (error) {
          // Resolve an uncertain IPC acknowledgement from persisted identity before
          // allowing another paid generation or restoring the consumed draft.
          const fresh = await main<Loaded>({ action: "load", id: this.id });
          if (fresh.request?.id === input.requestId) {
            this.intent = undefined;
            this.update({
              loaded: fresh,
              busy: fresh.request.status === "active",
            });
            if (action === "send" && this.localRevision === revision + 1)
              this.savedRevision = this.localRevision;
            await this.saveDraft();
            this.transport.requestId = input.requestId;
            this.chat.messages = fresh.messages;
            if (fresh.request.status === "active")
              setTimeout(() => {
                void this.resume().catch(this.report);
              }, 0);
            return;
          }
          this.intent = undefined;
          this.chat.messages = fresh.messages;
          if (action === "send" && this.localRevision === revision + 1) {
            this.update({ text: input.text });
            this.localRevision++;
          } else if (action === "send") {
            this.update({ unsentText: input.text });
          }
          throw error;
        }
      });
    } catch (error) {
      this.update({
        busy:
          !!this.intent || this.snapshot.loaded?.request?.status === "active",
        error: errorMessage(error),
        status: this.intent
          ? "Request acknowledgement unavailable"
          : "Request not started",
      });
      if (agent) throw error;
    }
  }
  async sendAgent(input: Start, authorization: AgentGeneration) {
    if (input.action !== "send" || input.targetId !== null)
      throw Error("SCOPE_DENIED");
    await this.send("send", null, undefined, { input, authorization });
    if (!authorization.result) throw Error("OUTCOME_UNKNOWN");
    return authorization.result;
  }
  async reconnect() {
    const loaded = await main<Loaded>({ action: "load", id: this.id });
    if (!loaded.request)
      throw Error("No native request is available to reconnect.");
    this.intent = undefined;
    this.update({
      loaded,
      busy: loaded.request.status === "active",
      error: "",
    });
    this.transport.requestId = loaded.request.id;
    if (this.chat.status === "streaming" || this.chat.status === "submitted") {
      this.resuming = true;
      this.transport.disconnect?.();
    } else {
      await this.resume();
    }
  }
  async stop() {
    if (!this.snapshot.busy) return;
    const requestId =
      this.intent?.requestId ||
      this.transport.requestId ||
      this.snapshot.loaded?.request?.id;
    if (requestId) {
      this.update({ status: "Stopping…" });
      await api("chat_cancel", { requestId });
    }
  }
  private terminal(packet: Packet) {
    if (!this.views) this.unread = true;
    this.ended = true;
    this.terminalPacket = packet;
    const error =
      packet.type === "storage-error"
        ? packet.error ||
          "The response could not be saved. Keep this view open and retry saving."
        : packet.status === "failed" || packet.status === "interrupted"
          ? (failures[packet.result?.code ?? ""] ??
            "The provider could not complete this response. Retry starts a new request.")
          : this.snapshot.storageFailed
            ? this.snapshot.error
            : "";
    this.update({
      error,
      storageFailed:
        this.snapshot.storageFailed || packet.type === "storage-error",
      status:
        this.snapshot.storageFailed || packet.type === "storage-error"
          ? "Not saved"
          : packet.status === "cancelled"
            ? "Stopped · partial response saved"
            : packet.status === "completed"
              ? "Response saved"
              : "Response interrupted",
    });
  }
  private async resume() {
    this.resuming = false;
    const assistantId =
      this.snapshot.loaded?.request?.assistantId ??
      this.transport.input?.assistantId;
    if (!assistantId) return;
    this.chat.messages = [
      ...this.chat.messages.filter((m) => m.id !== assistantId),
      { id: assistantId, role: "assistant", parts: [] },
    ];
    await this.chat.resumeStream();
  }
  async variant(target: string) {
    return this.enqueue(async () => {
      const loaded = this.snapshot.loaded;
      if (!loaded || this.snapshot.busy) return;
      const next = await main<Loaded>({
        action: "variant",
        id: this.id,
        target,
        expected: loaded.conversation.revision,
      });
      this.chat.messages = next.messages;
      this.update({ loaded: next });
    });
  }
  async older() {
    const loaded = this.snapshot.loaded;
    if (!loaded || !loaded.hasOlder || this.snapshot.busy) return;
    const next = await main<Loaded>({
      action: "load",
      id: this.id,
      offset: loaded.messages.length,
    });
    const messages = [...next.messages, ...loaded.messages];
    this.chat.messages = messages;
    this.update({ loaded: { ...loaded, messages, hasOlder: next.hasOlder } });
  }
  async attach(
    source: { path: string } | { name: string; bytes: number[] },
    approved = false,
  ) {
    return this.enqueue(async () => {
      await this.saveDraft();
      const loaded = this.snapshot.loaded;
      if (!loaded) return;
      const result = await main<{ draft: Draft }>({
        action: "import",
        id: this.id,
        attachment: newId(),
        expected: loaded.draft.revision,
        approved,
        ...source,
      });
      this.update({
        loaded: { ...this.snapshot.loaded!, draft: result.draft },
      });
    });
  }
  async removeAttachment(attachment: string) {
    return this.enqueue(async () => {
      await this.saveDraft();
      const loaded = this.snapshot.loaded;
      if (!loaded) return;
      const draft = await main<Draft>({
        action: "remove-attachment",
        id: this.id,
        attachment,
        expected: loaded.draft.revision,
      });
      this.update({ loaded: { ...loaded, draft } });
    });
  }
  async attachment(id: string) {
    const cached = this.attachments.get(id);
    if (cached) return cached;
    const value = await main<{ attachment: Attachment; data: string }>({
      action: "attachment",
      attachment: id,
    });
    if (this.disposed) throw Error("The conversation view has closed.");
    const concurrent = this.attachments.get(id);
    if (concurrent) return concurrent;
    const bytes = Uint8Array.from(atob(value.data), (c) => c.charCodeAt(0));
    const result = {
      attachment: value.attachment,
      url: URL.createObjectURL(
        new Blob([bytes], { type: value.attachment.mime }),
      ),
    };
    while (this.attachments.size >= 2) {
      const oldest = this.attachments.keys().next().value!;
      URL.revokeObjectURL(this.attachments.get(oldest)!.url);
      this.attachments.delete(oldest);
    }
    this.attachments.set(id, result);
    return result;
  }
  async rename(title: string) {
    const conversation = await main<Loaded["conversation"]>({
      action: "rename",
      id: this.id,
      title,
    });
    if (this.snapshot.loaded)
      this.update({ loaded: { ...this.snapshot.loaded, conversation } });
    conversationTitle(this.id, title);
  }
  resolveUnsent(keep: boolean) {
    const text = this.snapshot.unsentText;
    this.update({ unsentText: undefined, error: "" });
    if (keep && text) this.setText(`${text}\n\n${this.snapshot.text}`);
    else void this.flush().catch(this.report);
  }
  async discard() {
    clearTimeout(this.timer);
    await api("chat_discard", { conversation: this.id });
    this.savedRevision = this.localRevision;
    this.update({
      busy: false,
      error: "",
      status: "Unsaved changes discarded",
      unsentText: undefined,
      storageFailed: false,
    });
  }
  async retrySave() {
    await this.flush();
    await api("chat_flush", { conversation: this.id });
    await this.reload();
    this.update({ error: "", status: "Saved", storageFailed: false });
  }
  dispose() {
    this.disposed = true;
    chatActivity(this.id, "");
    clearTimeout(this.timer);
    this.transport.disconnect?.();
    this.attachments.forEach((a) => URL.revokeObjectURL(a.url));
    this.listeners.clear();
  }
}

registerChatRuntime({ existing, retain });
