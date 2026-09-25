import { Channel } from "@tauri-apps/api/core";
import type { ChatTransport, UIMessage, UIMessageChunk } from "ai";
import { api, errorMessage } from "../api";
import { snapshotToChunks } from "./chat-snapshot";
import type { ChatMessageSnapshot } from "./chat-snapshot";
import type { Accepted, Start } from "./types";
import type { ChatSent } from "../agent-chat";
export interface AgentGeneration {
  operationId: string;
  nonce: string;
  planHash: string;
  result?: ChatSent;
}
export interface Packet {
  type: "chunk" | "resync" | "snapshot" | "terminal" | "storage-error";
  epoch: number;
  sequence: number;
  chunk?: UIMessageChunk;
  status?: string;
  error?: string;
  result?: { code?: string };
  snapshot?: ChatMessageSnapshot;
  terminal?: Packet | null;
}
export class NativeTransport implements ChatTransport<UIMessage> {
  input?: Start;
  agent?: AgentGeneration;
  requestId = "";
  accepted: (value: Accepted) => void = () => {};
  failed: (error: string) => void = () => {};
  ended: (packet: Packet) => void = () => {};
  resync: () => void = () => {};
  disconnect?: () => void;
  sendMessages: ChatTransport<UIMessage>["sendMessages"] = async () => {
    if (!this.input) throw Error("No committed send intent.");
    const input = this.input;
    this.requestId = input.requestId;
    return this.open(false, input);
  };
  reconnectToStream: ChatTransport<UIMessage>["reconnectToStream"] =
    async () => (this.requestId ? this.open(true) : null);
  private open(reconnect: boolean, input?: Start) {
    const agent = reconnect ? undefined : this.agent;
    if (!reconnect) this.agent = undefined;
    const requestId = this.requestId;
    let closed = false;
    let epoch = reconnect ? -1 : 0;
    let sequence = 0;
    let lastAck: Packet | undefined;
    let wake: (() => void) | undefined;
    const queue: (Packet | { type: "failure"; error: string })[] = [];
    const channel = new Channel<Packet>();
    const signal = () => {
      wake?.();
      wake = undefined;
    };
    const close = () => {
      closed = true;
      signal();
    };
    this.disconnect = close;
    channel.onmessage = (packet) => {
      if (closed || this.requestId !== requestId) return;
      // Rust bounds the unacknowledged queue. ACK happens at parser consumption,
      // never merely when a native callback has put a delta into a JS queue.
      queue.push(packet);
      signal();
    };
    const fail = (error: unknown) => {
      if (!closed) {
        queue.push({ type: "failure", error: errorMessage(error) });
        signal();
      }
    };
    if (reconnect)
      void api<boolean>("chat_subscribe", { requestId, channel })
        .then((found) => {
          if (!found)
            fail(
              "The active stream is unavailable. Reload the saved conversation.",
            );
        })
        .catch(fail);
    else
      void (
        agent
          ? api<{ accepted: Accepted; result: ChatSent }>(
              "agent_control_chat_send",
              {
                operationId: agent.operationId,
                nonce: agent.nonce,
                planHash: agent.planHash,
                channel,
              },
            ).then((reply) => {
              agent.result = reply.result;
              return reply.accepted;
            })
          : api<Accepted>("chat_generate", { input, channel })
      )
        .then(this.accepted)
        .catch((error) => {
          this.failed(errorMessage(error));
          fail(error);
        });
    let ackTimer: ReturnType<typeof setTimeout> | undefined;
    let lastSent = 0;
    const acknowledge = (force = false) => {
      if (!force && performance.now() - lastSent < 25) {
        ackTimer ??= setTimeout(() => {
          ackTimer = undefined;
          acknowledge(true);
        }, 25);
        return;
      }
      clearTimeout(ackTimer);
      ackTimer = undefined;
      lastSent = performance.now();
      if (lastAck) {
        void api("chat_ack", {
          requestId,
          epoch: lastAck.epoch,
          sequence: lastAck.sequence,
        }).catch(() => {});
        lastAck = undefined;
      }
    };
    const transport = this;
    return new ReadableStream<UIMessageChunk>({
      async pull(controller) {
        acknowledge();
        while (!closed) {
          const packet = queue.shift();
          if (!packet) {
            await new Promise<void>((resolve) => {
              wake = resolve;
            });
            continue;
          }
          if (packet.type === "failure") {
            close();
            controller.error(Error(packet.error));
            return;
          }
          if (packet.type === "snapshot" && packet.snapshot) {
            epoch = packet.epoch;
            sequence = packet.sequence;
            for (const chunk of snapshotToChunks(packet.snapshot))
              controller.enqueue(chunk);
            if (packet.terminal) queue.unshift(packet.terminal);
            return;
          }
          if (packet.type === "chunk" && packet.chunk) {
            if (packet.epoch !== epoch || packet.sequence <= sequence) continue;
            sequence = packet.sequence;
            lastAck = packet;
            controller.enqueue(packet.chunk);
            return;
          }
          if (packet.type === "resync") {
            close();
            controller.close();
            transport.resync();
            return;
          }
          if (packet.type === "terminal" || packet.type === "storage-error") {
            close();
            transport.ended(packet);
            controller.enqueue({ type: "finish", finishReason: "stop" });
            controller.close();
            return;
          }
        }
        controller.close();
      },
      cancel() {
        close();
        acknowledge(true);
      },
    });
    // The SDK owns only its parser; native cancellation is an explicit runtime action.
  }
}
