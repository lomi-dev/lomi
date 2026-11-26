import assert from "node:assert/strict";
import test from "node:test";
import {
  pruneChatUsage,
  registerChatUsage,
  subscribeChatUsage,
  withChatTargets,
} from "./chat-usage.ts";

test("chat targets fill CLIs without a terminal and skip unknown ones", () => {
  const terminal = [{ id: "t1", process: { cli: "claude" as const, pid: 5 } }];
  const chats = new Map([
    ["a", "claude"],
    ["b", "codex"],
    ["c", "codex"],
    ["d", "nope"],
  ]);
  assert.deepEqual(withChatTargets(terminal, chats), [
    { id: "chat:b", process: { cli: "codex", pid: 0 } },
    ...terminal,
  ]);
});

test("chat usage survives remounts and leaves with its tab", () => {
  let notified = 0;
  const stop = subscribeChatUsage(() => notified++);
  registerChatUsage("tab", "claude");
  registerChatUsage("tab", "claude");
  assert.equal(notified, 1);
  pruneChatUsage(new Set(["tab"]));
  assert.deepEqual(withChatTargets([]), [
    { id: "chat:tab", process: { cli: "claude", pid: 0 } },
  ]);
  pruneChatUsage(new Set());
  assert.deepEqual(withChatTargets([]), []);
  registerChatUsage("api", "claude");
  registerChatUsage("api", null);
  assert.deepEqual(withChatTargets([]), []);
  assert.equal(notified, 4);
  stop();
});
