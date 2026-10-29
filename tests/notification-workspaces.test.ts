import assert from "node:assert/strict";
import { test } from "node:test";
import {
  unreadTerminalPaneIds,
  type NotificationSnapshot,
} from "../src/notifications.ts";

test("only unread source-less notifications with pane IDs can mark terminal leaves", () => {
  const snapshot: NotificationSnapshot = {
    revision: 4,
    items: [
      {
        id: "terminal-unread",
        kind: "finished",
        title: "Finished",
        body: "",
        createdAt: 1,
        read: false,
        paneId: "terminal-pane",
      },
      {
        id: "terminal-read",
        kind: "attention",
        title: "Read terminal event",
        body: "",
        createdAt: 2,
        read: true,
        paneId: "read-pane",
      },
      {
        id: "application",
        kind: "attention",
        title: "Application needs attention",
        body: "",
        createdAt: 3,
        read: false,
        source: "app",
        paneId: "application-pane",
      },
      {
        id: "legacy",
        kind: "finished",
        title: "Legacy notification",
        body: "",
        createdAt: 4,
        read: false,
        agent: "claude",
      },
    ],
  };

  assert.deepEqual([...unreadTerminalPaneIds(snapshot)], ["terminal-pane"]);
  assert.deepEqual([...unreadTerminalPaneIds(null)], []);
});
