import assert from "node:assert/strict";
import { test } from "node:test";
import type { UIMessage } from "ai";
import { chatMessageView } from "../src/chat/chat-messages.ts";

function source() {
  const messageListeners = new Set<() => void>();
  const statusListeners = new Set<() => void>();
  const chat = {
    messages: [] as UIMessage[],
    status: "streaming" as "streaming" | "ready" | "error",
    "~registerMessagesCallback": (listener: () => void) => {
      messageListeners.add(listener);
      return () => {
        messageListeners.delete(listener);
      };
    },
    "~registerStatusCallback": (listener: () => void) => {
      statusListeners.add(listener);
      return () => {
        statusListeners.delete(listener);
      };
    },
    append(text: string) {
      chat.messages = [
        { id: "answer", role: "assistant", parts: [{ type: "text", text }] },
      ];
      for (const listener of messageListeners) listener();
    },
    finish(status: "ready" | "error") {
      chat.status = status;
      for (const listener of statusListeners) listener();
    },
  };
  return { chat, messageListeners, statusListeners };
}

test("message views publish a token burst in one trailing task and keep the parser snapshot intact", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"] });
  const { chat } = source();
  const view = chatMessageView(chat);
  let renders = 0;
  const stop = view.subscribe(() => renders++);
  const initial = view.getSnapshot();
  for (let token = 1; token <= 1000; token++) {
    // Rendering can consume wall time without permitting a timer task to run.
    t.mock.timers.setTime(token * 100);
    chat.append("🙂".repeat(token));
  }
  assert.equal(renders, 0);
  assert.strictEqual(view.getSnapshot(), initial);
  assert.equal(chat.messages[0].parts[0].type, "text");
  t.mock.timers.tick(50);
  assert.equal(renders, 1);
  assert.strictEqual(view.getSnapshot(), chat.messages);
  stop();
});

test("terminal status publishes final text immediately and cancels the delayed render", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  for (const status of ["ready", "error"] as const) {
    const { chat } = source();
    const view = chatMessageView(chat);
    let renders = 0;
    const stop = view.subscribe(() => renders++);
    chat.append("Saved partial or completed answer 日本語");
    chat.finish(status);
    assert.equal(renders, 1);
    assert.strictEqual(view.getSnapshot(), chat.messages);
    t.mock.timers.tick(100);
    assert.equal(renders, 1);
    stop();
  }
});

test("view removal unsubscribes without stopping the retained stream and catches up on remount", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { chat, messageListeners, statusListeners } = source();
  const view = chatMessageView(chat);
  chat.append("Changed before subscription");
  let renders = 0;
  const stop = view.subscribe(() => renders++);
  assert.strictEqual(view.getSnapshot(), chat.messages);
  chat.append("Changed while mounted");
  stop();
  assert.equal(messageListeners.size, 0);
  assert.equal(statusListeners.size, 0);
  t.mock.timers.tick(100);
  assert.equal(renders, 0);
  chat.append("Hidden stream retained");
  assert.equal(chat.status, "streaming");
  const stopAgain = view.subscribe(() => renders++);
  assert.strictEqual(view.getSnapshot(), chat.messages);
  stopAgain();
});
