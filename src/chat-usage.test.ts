import assert from "node:assert/strict";
import test from "node:test";
import { withChatTargets } from "./chat-usage.ts";

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
