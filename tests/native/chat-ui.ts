import { ChatRuntime, main } from "../../src/chat/chat-runtime";
import type { Conversation } from "../../src/chat/types";
import { Channel, invoke } from "@tauri-apps/api/core";
import { Chat } from "@ai-sdk/react";
import type { UIMessageChunk } from "ai";
import { checkBrowserIsolation, runLive } from "./chat-live-ui";

type Packet = {
  type: string;
  sentAt?: number;
  epoch: number;
  sequence: number;
  chunk?: UIMessageChunk & { probeSentAt?: number };
  snapshot?: {
    message: {
      id: string;
      parts: { type: string; text?: string }[];
    };
    blocks: Record<
      string,
      { index: number; type: "text" | "reasoning"; open: boolean }
    >;
  };
  terminal?: unknown;
  status?: string;
};
const pause = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function snapshotText(snapshot: NonNullable<Packet["snapshot"]>["message"]) {
  return snapshot.parts
    .filter((part) => part.type === "text")
    .map((part) => part.text ?? "")
    .join("");
}

export async function run() {
  const mode = await invoke<{ live: boolean }>("chat_probe_backend", {
    action: "mode",
  });
  if (mode.live) return runLive();
  const latency: number[] = [];
  const presentation: number[] = [];
  let measuringPresentation = false;
  const preview = document.createElement("pre");
  preview.setAttribute("aria-label", "Native stream presentation probe");
  preview.style.cssText =
    "position:fixed;inset:20px;z-index:999999;background:var(--color-background);color:var(--color-surface-text);padding:16px;overflow:auto;pointer-events:none";
  document.body.append(preview);
  let channel!: Channel<Packet>;
  let startup: unknown;
  let resync = false;
  let terminal = "";
  let sends = 0;
  let finishes = 0;
  let cancels = 0;
  let snapshot: Packet | undefined;
  let disconnected = false;
  const open = (reconnect: boolean) =>
    new ReadableStream<UIMessageChunk>({
      start(controller) {
        channel = new Channel<Packet>();
        channel.onmessage = (packet) => {
          if (
            measuringPresentation &&
            packet.chunk?.probeSentAt &&
            presentation.length < 8192 &&
            document.visibilityState === "visible"
          ) {
            const sent = packet.chunk.probeSentAt;
            const delta = "delta" in packet.chunk ? packet.chunk.delta : "";
            requestAnimationFrame(() => {
              preview.textContent = `Native stream: ${delta}`;
              requestAnimationFrame(() => {
                if (
                  measuringPresentation &&
                  document.visibilityState === "visible"
                )
                  presentation.push(Date.now() - sent);
              });
            });
          }
          if (packet.sentAt && latency.length < 8192)
            latency.push(Date.now() - packet.sentAt);
          if (packet.type === "snapshot" && reconnect && packet.snapshot) {
            snapshot = packet;
            controller.enqueue({
              type: "start",
              messageId: packet.snapshot.message.id,
            });
            for (const [id, block] of Object.entries(packet.snapshot.blocks)) {
              const part = packet.snapshot.message.parts[block.index];
              if (
                !part ||
                part.type !== block.type ||
                typeof part.text !== "string"
              )
                throw Error("Snapshot block does not match its message part");
              controller.enqueue({
                type: block.type === "text" ? "text-start" : "reasoning-start",
                id,
              });
              controller.enqueue({
                type: block.type === "text" ? "text-delta" : "reasoning-delta",
                id,
                delta: part.text,
              });
              if (!block.open)
                controller.enqueue({
                  type: block.type === "text" ? "text-end" : "reasoning-end",
                  id,
                });
            }
          } else if (packet.type === "chunk" && packet.chunk) {
            controller.enqueue(packet.chunk);
            if (reconnect)
              void invoke("chat_probe_cancel", {
                action: "ack",
                channel,
                epoch: packet.epoch,
                sequence: packet.sequence,
              });
          } else if (packet.type === "resync") {
            resync = true;
            disconnected = true;
            controller.close();
          } else if (packet.type === "terminal") {
            terminal = packet.status ?? "";
            controller.enqueue({ type: "finish", finishReason: "stop" });
            controller.close();
          }
        };
        if (reconnect) {
          disconnected = false;
          void invoke("chat_probe_cancel", { action: "resync", channel }).catch(
            (error) => controller.error(error),
          );
        }
      },
    });
  const chat = new Chat({
    id: "conversation-1",
    transport: {
      async sendMessages({ abortSignal }) {
        sends++;
        abortSignal?.addEventListener("abort", () => {
          cancels++;
          void invoke("chat_probe_cancel", { action: "stop", channel });
        });
        const stream = open(false);
        startup = await invoke("chat_probe_start", { channel });
        return stream;
      },
      async reconnectToStream() {
        return open(true);
      },
    },
    onFinish: () => {
      finishes++;
    },
  });
  await chat.sendMessage({
    id: "user-1",
    role: "user",
    parts: [{ type: "text", text: "fixture-only" }],
  });
  if (!resync || !disconnected || chat.status !== "ready")
    throw Error(
      `SDK did not close its UI-only parser: ${chat.status}; resync=${resync}; ${String(chat.error)}`,
    );
  if (terminal) throw Error("UI EOF incorrectly ended provider request");
  await invoke("chat_probe_cancel", { action: "minimize", channel });
  await pause(900);
  // Rebuild the parser with an empty assistant; the subscription supplies the
  // authoritative prefix AND active block IDs, followed by post-watermark deltas.
  chat.messages = [
    chat.messages[0],
    { id: "assistant-1", role: "assistant", parts: [] },
  ];
  const resume = chat.resumeStream();
  for (let i = 0; i < 100 && !snapshot; i++) await pause(10);
  if (
    !snapshot ||
    snapshot.epoch !== 2 ||
    !snapshot.snapshot ||
    !snapshotText(snapshot.snapshot.message).includes("日本語")
  )
    throw Error("Snapshot lost Unicode prefix");
  // Measure active presentation separately from the minimized-window recovery.
  await Promise.race([
    new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    ),
    pause(1500),
  ]);
  measuringPresentation = true;
  await pause(500);
  measuringPresentation = false;
  const start = performance.now();
  await invoke("chat_probe_cancel", { action: "stop", channel });
  await resume;
  if (
    terminal !== "cancelled" ||
    sends !== 1 ||
    cancels !== 0 ||
    finishes !== 2 ||
    chat.messages.length !== 2
  )
    throw Error(
      `SDK resync mismatch: terminal=${terminal}, sends=${sends}, cancels=${cancels}, finishes=${finishes}, messages=${chat.messages.length}`,
    );
  const text =
    chat.messages
      .at(-1)
      ?.parts.filter((p) => p.type === "text")
      .map((p) => p.text)
      .join("") ?? "";
  if (!text.startsWith(snapshotText(snapshot.snapshot.message)))
    throw Error("SDK parser lost the snapshot prefix");
  const nativeStopMs = performance.now() - start;
  await invoke("chat_probe_backend");
  let deniedSettings = false;
  try {
    await invoke("chat_connection_action", {
      connectionId: "native-fixture",
      model: "fixture",
      operation: "test-connection",
    });
  } catch (error) {
    deniedSettings = String(error).includes("Only Settings");
  }
  if (!deniedSettings)
    throw Error("Main view could run a Settings-only paid test");
  const conversation = await main<Conversation>({
    action: "create",
    id: crypto.randomUUID(),
    origin: {
      projectId: "native",
      projectName: "Native fixture",
      workspaceId: "native",
      workspaceName: "Native fixture",
    },
  });
  const runtime = new ChatRuntime(conversation.id);
  await runtime.ready;
  runtime.setText("Native product transport 日本語");
  await runtime.send();
  runtime.setText("Retained next draft 👩🏽‍💻");
  await runtime.flush();
  await pause(120);
  await runtime.reconnect();
  await pause(120);
  const productStop = performance.now();
  await runtime.stop();
  for (let i = 0; i < 200 && runtime.snapshot.busy; i++) await pause(20);
  if (
    runtime.snapshot.busy ||
    runtime.snapshot.error ||
    !runtime.chat.messages.some(
      (m) =>
        m.role === "assistant" &&
        m.parts.some((p) => p.type === "text" && p.text.includes("日本語")),
    )
  )
    throw Error(`Production chat transport failed: ${runtime.snapshot.error}`);
  if (runtime.snapshot.text !== "Retained next draft 👩🏽‍💻")
    throw Error("Production draft was lost");
  const productStopMs = performance.now() - productStop;
  const deniedBrowser = await checkBrowserIsolation();
  await invoke("chat_close", { conversations: [conversation.id], all: false });
  const idleMemory = await invoke("chat_probe_backend", { action: "metrics" });
  const parallel = await Promise.all(
    Array.from({ length: 4 }, async () => {
      const conversation = await main<Conversation>({
        action: "create",
        id: crypto.randomUUID(),
        origin: {
          projectId: "native",
          projectName: "Native",
          workspaceId: "native",
          workspaceName: "Native",
        },
      });
      const runtime = new ChatRuntime(conversation.id);
      await runtime.ready;
      runtime.setText("Four stream memory fixture 日本語");
      await runtime.send();
      return runtime;
    }),
  );
  await pause(500);
  if (parallel.some((r) => !r.snapshot.busy || r.snapshot.error))
    throw Error("Four native streams did not remain active");
  const fourStreamMemory = await invoke("chat_probe_backend", {
    action: "metrics",
  });
  await Promise.all(parallel.map((r) => r.stop()));
  await invoke("chat_close", {
    conversations: parallel.map((r) => r.id),
    all: false,
  });
  latency.sort((a, b) => a - b);
  presentation.sort((a, b) => a - b);
  preview.remove();
  await invoke("chat_probe_result", {
    result: {
      passed: true,
      startup,
      stopMs: nativeStopMs,
      productStopMs,
      productTransport: true,
      deniedSettings,
      deniedBrowser,
      idleMemory,
      fourStreamMemory,
      rustToUiP95Ms: latency[Math.floor(latency.length * 0.95)],
      rustToUiSamples: latency.length,
      sidecarToPresentationP95Ms:
        presentation[Math.floor(presentation.length * 0.95)],
      presentationSamples: presentation.length,
      presentationUnavailable:
        presentation.length === 0
          ? "The OS did not expose visible animation frames during the probe."
          : undefined,
      resyncSequence: snapshot.sequence,
      sends,
      cancels,
      finishes,
      responseLength: text.length,
      checks: [
        "system keychain round trip and removal",
        "bundled Node to Rust to Tauri Channel",
        "Unicode",
        "SDK Chat parser resync with epoch and watermark",
        "UI-only EOF keeps provider active",
        "bounded delivery while minimized",
        "backend checkpoints",
        "native Stop",
      ],
    },
  });
}
