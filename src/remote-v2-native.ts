import { Channel, invoke } from "@tauri-apps/api/core";
import { ApiError } from "../../remote-web/src/api";
import protocolCodecWasmUrl from "@lomi/protocol-codec-wasm/wasm?url";
import type {
  ConsumerWindowChannel,
  ConsumerWindowChannelMessage,
  ConsumerWindowRegistration,
  ConsumerWindowSessionTuple,
  RemoteV2PlatformAdapter,
} from "../../remote-web/src/consumer-window-registration";
import {
  RemoteV2OutcomeUnknownError,
  RemoteV2PreSendError,
  RemoteV2RpcClient,
} from "../../remote-web/src/remote-v2-rpc-client";
import { REMOTE_V2_RPC_MAX_LOGICAL_BYTES } from "@lomi/remote-contracts/rpc-client";
import { initializeRemoteV2ContractBoundary } from "../../remote-web/src/remote-v2-contract-boundary";

let codecInitialization: Promise<void> | undefined;

function ensureDesktopProtocolCodec(): Promise<void> {
  if (!codecInitialization) {
    codecInitialization = initializeRemoteV2ContractBoundary(
      protocolCodecWasmUrl,
    ).catch((error) => {
      codecInitialization = undefined;
      throw error;
    });
  }
  return codecInitialization;
}

type NativeRegistration = {
  windowIdentityId: string;
  identityFingerprint: string;
  sessionTuple: ConsumerWindowSessionTuple;
};

type NativeChannelEvent =
  | { type: "message"; kind: number; id: string; bytes: number[] }
  | { type: "closed"; code: string };

type NativeChannelCommand = {
  channelHandleId: string;
  kind: number;
  bytes: number[];
};

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const ACCOUNT_ID = /^[0-9a-f]{32}$/;

export async function desktopRemoteV2AccountId(): Promise<string | undefined> {
  const accountId = await invoke<string | null>("remote_v2_account_id");
  if (accountId === null) return undefined;
  if (typeof accountId !== "string" || !ACCOUNT_ID.test(accountId)) {
    throw new Error("INVALID_NATIVE_ACCOUNT_ID");
  }
  return accountId;
}

export function openDesktopRemoteV2View(
  hostId: string,
  workspaceId: string,
): Promise<void> {
  if (!UUID.test(hostId) || !UUID.test(workspaceId)) {
    return Promise.reject(new Error("INVALID_REMOTE_VIEW_REFERENCE"));
  }
  return invoke("remote_v2_open_view", { hostId, workspaceId });
}

function accountMatches(left: string, right: string) {
  return (
    left.replaceAll("-", "").toLowerCase() ===
    right.replaceAll("-", "").toLowerCase()
  );
}

function remoteInvokeError(error: unknown): unknown {
  if (
    !error ||
    typeof error !== "object" ||
    !("status" in error) ||
    !("code" in error)
  )
    return error;
  const status = error.status;
  const code = error.code;
  if (typeof code !== "string" || !/^[A-Z0-9_]{1,80}$/.test(code)) return error;
  if (
    typeof status === "number" &&
    Number.isInteger(status) &&
    status >= 100 &&
    status <= 599
  ) {
    return new ApiError(status, code);
  }
  if (status === null) return new Error(code);
  return error;
}

function makeChannel(
  channelHandleId: string,
  events: Channel<NativeChannelEvent>,
  onClosed: (channelHandleId: string) => void,
): ConsumerWindowChannel {
  const messageListeners = new Set<
    (message: ConsumerWindowChannelMessage) => void
  >();
  const closeListeners = new Set<(reason: Error) => void>();
  let closed = false;

  const transport = {
    send: async (
      message: { kind: number; bytes: Uint8Array },
      sendGuard?: () => boolean,
      onStart?: () => boolean | void,
    ) => {
      if (closed) throw new RemoteV2PreSendError("CHANNEL_CLOSED");
      const command: NativeChannelCommand = {
        channelHandleId,
        kind: message.kind,
        bytes: Array.from(message.bytes),
      };
      try {
        if (closed || (sendGuard && !sendGuard())) {
          throw new RemoteV2PreSendError("SEND_CANCELLED");
        }
        // No asynchronous work separates the fresh guard from the bridge send.
        if (onStart?.() === false)
          throw new RemoteV2PreSendError("SEND_CANCELLED");
        try {
          await invoke("remote_v2_channel_send", command);
        } catch {
          throw new RemoteV2OutcomeUnknownError("NATIVE_SEND_FAILED");
        }
      } finally {
        command.bytes.fill(0);
      }
    },
    close: (reason?: Error) => {
      void close(reason).catch(() => {});
    },
  };

  const rpc = new RemoteV2RpcClient({
    transport,
    onStream: (message) => {
      try {
        for (const listener of messageListeners) {
          if (closed) break;
          const value: ConsumerWindowChannelMessage = {
            kind: message.kind,
            id: message.id,
            bytes: message.bytes.slice(),
          };
          try {
            listener(value);
          } catch {
            value.bytes.fill(0);
          }
        }
      } finally {
        message.bytes.fill(0);
      }
    },
  });

  const close = async (reason = new Error("CHANNEL_CLOSED")) => {
    if (closed) return;
    closed = true;
    const effective = reason;
    rpc.close(effective, false);
    onClosed(channelHandleId);
    for (const listener of closeListeners) {
      try {
        listener(effective);
      } catch {
        // A close notification must not prevent cleanup.
      }
    }
    closeListeners.clear();
    messageListeners.clear();
    await invoke("remote_v2_channel_close", { channelHandleId }).catch(
      () => {},
    );
  };

  events.onmessage = (event) => {
    if (closed) return;
    if (event.type === "closed") {
      const reason = new Error(event.code || "CHANNEL_CLOSED");
      closed = true;
      rpc.close(reason, false);
      onClosed(channelHandleId);
      for (const listener of closeListeners) {
        try {
          listener(reason);
        } catch {
          // A close listener is isolated from other view cleanup.
        }
      }
      closeListeners.clear();
      messageListeners.clear();
      void invoke("remote_v2_channel_close", { channelHandleId }).catch(
        () => {},
      );
      return;
    }
    if (
      event.type !== "message" ||
      ![2, 3, 4, 5].includes(event.kind) ||
      !Number.isSafeInteger(Number(event.id)) ||
      !Array.isArray(event.bytes) ||
      event.bytes.length > REMOTE_V2_RPC_MAX_LOGICAL_BYTES ||
      event.bytes.some(
        (byte) => !Number.isInteger(byte) || byte < 0 || byte > 255,
      )
    ) {
      void close(new Error("INVALID_NATIVE_CHANNEL_EVENT"));
      return;
    }
    const bytes = Uint8Array.from(event.bytes);
    try {
      rpc.receive({ kind: event.kind as 2 | 3 | 4 | 5, id: event.id, bytes });
    } finally {
      bytes.fill(0);
      event.bytes.fill(0);
    }
  };

  return {
    call: async (call, timeoutMs) => {
      if (closed) throw new Error("CHANNEL_CLOSED");
      if (
        [
          "terminal.claim",
          "terminal.renew",
          "terminal.release",
          "terminal.input",
          "terminal.resize",
        ].includes(call.method)
      ) {
        throw new Error("USE_NATIVE_TERMINAL_CONTROL_API");
      }
      return rpc.call(call, timeoutMs);
    },
    input: (bytes) => {
      if (closed) {
        bytes.fill(0);
        return Promise.reject(new Error("CHANNEL_CLOSED"));
      }
      bytes.fill(0);
      return Promise.reject(new Error("USE_NATIVE_TERMINAL_CONTROL_API"));
    },
    resize: () => {
      if (closed) return Promise.reject(new Error("CHANNEL_CLOSED"));
      return Promise.reject(new Error("USE_NATIVE_TERMINAL_CONTROL_API"));
    },
    nativeControl: {
      claim: (params) =>
        invoke("remote_v2_terminal_claim", { channelHandleId, params }),
      input: (bytes) => {
        if (closed) {
          bytes.fill(0);
          return Promise.reject(new Error("CHANNEL_CLOSED"));
        }
        const payload = Array.from(bytes);
        bytes.fill(0);
        return invoke("remote_v2_terminal_input", {
          channelHandleId,
          bytes: payload,
        });
      },
      resize: (columns, rows) =>
        invoke("remote_v2_terminal_resize", { channelHandleId, columns, rows }),
      release: () => invoke("remote_v2_terminal_release", { channelHandleId }),
      focus: (focused) =>
        invoke("remote_v2_terminal_focus", { channelHandleId, focused }),
    },
    sendFileWatchFrame: (frame) => rpc.sendFileWatchFrame(frame),
    sendWorkspaceFrame: (frame) => rpc.sendWorkspaceFrame(frame),
    subscribe: (listener) => {
      messageListeners.add(listener);
      return () => messageListeners.delete(listener);
    },
    onMessage: (listener) => {
      messageListeners.add(listener);
      return () => messageListeners.delete(listener);
    },
    onClose: (listener) => {
      if (closed) {
        listener(new Error("CHANNEL_CLOSED"));
        return () => {};
      }
      closeListeners.add(listener);
      return () => closeListeners.delete(listener);
    },
    close,
  };
}

export function createDesktopRemoteV2PlatformAdapter(): RemoteV2PlatformAdapter {
  const registrations = new Set<ConsumerWindowRegistration>();
  return {
    request: async <T>(
      path: string,
      options: {
        method: "GET" | "POST";
        body?: unknown;
        signal?: AbortSignal;
      },
    ): Promise<T> => {
      if (options.signal?.aborted)
        throw new DOMException("Aborted", "AbortError");
      let value: T;
      try {
        value = await invoke<T>("remote_v2_request", {
          method: options.method,
          path,
          body: options.body,
        });
      } catch (error) {
        throw remoteInvokeError(error);
      }
      if (options.signal?.aborted)
        throw new DOMException("Aborted", "AbortError");
      return value;
    },
    authorizeConsumerAccess: async (signal) => {
      if (signal?.aborted) throw new DOMException("Aborted", "AbortError");
      try {
        await invoke("remote_v2_authorize_consumer");
      } catch (error) {
        throw remoteInvokeError(error);
      }
      if (signal?.aborted) throw new DOMException("Aborted", "AbortError");
    },
    existingConsumerWindowId: async (accountId, tuple) => {
      if (!accountMatches(accountId, tuple.accountId)) return undefined;
      return (
        (await invoke<string | null>("remote_v2_existing_window_identity", {
          tuple,
        })) ?? undefined
      );
    },
    registerConsumerWindow: async (accountId) => {
      let saved: NativeRegistration;
      try {
        saved = await invoke<NativeRegistration>(
          "remote_v2_register_consumer_window",
          { accountId },
        );
      } catch (error) {
        throw remoteInvokeError(error);
      }
      if (
        !UUID.test(saved.windowIdentityId) ||
        !/^[0-9a-f]{64}$/.test(saved.identityFingerprint) ||
        !accountMatches(accountId, saved.sessionTuple.accountId)
      ) {
        throw new Error("INVALID_NATIVE_CONSUMER_IDENTITY");
      }
      const openChannels = new Map<string, ConsumerWindowChannel>();
      let closed = false;
      const registration: ConsumerWindowRegistration = {
        sessionId: crypto.randomUUID(),
        windowIdentityId: saved.windowIdentityId,
        identityFingerprint: saved.identityFingerprint,
        sessionTuple: saved.sessionTuple,
        openChannel: async (options) => {
          if (closed) throw new Error("IDENTITY_REQUIRED");
          if (options.signal?.aborted)
            throw new DOMException("Aborted", "AbortError");
          await ensureDesktopProtocolCodec();
          if (closed || options.signal?.aborted)
            throw new DOMException("Aborted", "AbortError");
          const events = new Channel<NativeChannelEvent>();
          const channelHandleId = await invoke<string>(
            "remote_v2_channel_open",
            {
              windowIdentityId: saved.windowIdentityId,
              host: options.host,
              pairing: options.pairing,
              runtime: options.runtime,
              target: options.target,
              events,
            },
          );
          if (!UUID.test(channelHandleId)) {
            await invoke("remote_v2_channel_close", { channelHandleId }).catch(
              () => {},
            );
            throw new Error("INVALID_NATIVE_CHANNEL_HANDLE");
          }
          let channel: ConsumerWindowChannel;
          try {
            channel = makeChannel(channelHandleId, events, (id) =>
              openChannels.delete(id),
            );
          } catch (error) {
            await invoke("remote_v2_channel_close", { channelHandleId }).catch(
              () => {},
            );
            throw error;
          }
          openChannels.set(channelHandleId, channel);
          if (closed || options.signal?.aborted) {
            await channel.close();
            throw new DOMException("Aborted", "AbortError");
          }
          return channel;
        },
        close: async () => {
          if (closed) return;
          closed = true;
          await Promise.allSettled(
            [...openChannels.values()].map((channel) => channel.close()),
          );
          registrations.delete(registration);
        },
      };
      registrations.add(registration);
      return registration;
    },
    clearRevokedConsumerWindow: async (accountId, registration) => {
      await registration.close();
      await invoke("remote_v2_clear_revoked_consumer_window", {
        accountId,
        windowIdentityId: registration.windowIdentityId,
      });
      registrations.delete(registration);
    },
  };
}
