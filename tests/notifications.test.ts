import assert from "node:assert/strict";
import { test } from "node:test";
import {
  newestNotificationSnapshot,
  notificationBadge,
  notificationAgent,
  notificationSource,
  notificationKindLabel,
  notificationRelativeTime,
  unreadNotificationCount,
  reportNotificationError,
  subscribeNotificationErrors,
  clearNotificationError,
  acceptNotificationSnapshot,
  getNotificationSnapshot,
  subscribeNotificationSnapshot,
  unreadTerminalPaneIds,
} from "../src/notifications.ts";
import type { NotificationSnapshot } from "../src/notifications.ts";

test("canonical inbox ignores stale and equal initial loads or mutation responses", () => {
  const current: NotificationSnapshot = { revision: 8, items: [] };
  assert.equal(
    newestNotificationSnapshot(current, { revision: 7, items: [] }),
    current,
  );
  assert.equal(
    newestNotificationSnapshot(current, { revision: 8, items: [] }),
    current,
  );
  const next = { revision: 9, items: [] };
  assert.equal(newestNotificationSnapshot(current, next), next);
  assert.equal(newestNotificationSnapshot(null, current), current);
});

test("inbox observers share unread targets and reject stale responses after reading", () => {
  const first: NotificationSnapshot = {
    revision: 20,
    items: [
      {
        id: "notification-20",
        kind: "attention",
        title: "Needs input",
        body: "Workspace",
        createdAt: 1,
        read: false,
        paneId: "pane-a",
      },
    ],
  };
  const observed: Array<NotificationSnapshot | null> = [];
  const stop = subscribeNotificationSnapshot(() =>
    observed.push(getNotificationSnapshot()),
  );
  acceptNotificationSnapshot(first);
  const read = {
    revision: 21,
    items: first.items.map((item) => ({ ...item, read: true })),
  };
  acceptNotificationSnapshot(read);
  acceptNotificationSnapshot(first);
  acceptNotificationSnapshot({ ...first, revision: 21 });
  assert.deepEqual(observed, [first, read]);
  assert.equal(getNotificationSnapshot(), read);
  assert.deepEqual([...unreadTerminalPaneIds(getNotificationSnapshot())], []);
  stop();
  acceptNotificationSnapshot({ revision: 22, items: [] });
  assert.equal(observed.length, 2);
});

test("unread counts use explicit read state and displayed badges cap at nine", () => {
  assert.equal(unreadNotificationCount(null), 0);
  const items = Array.from({ length: 12 }, (_, i) => ({
    id: String(i),
    kind: "finished" as const,
    title: "Done",
    body: "Context",
    createdAt: i,
    read: i < 2,
  }));
  assert.equal(unreadNotificationCount({ revision: 1, items }), 10);
  assert.equal(notificationBadge(1), "1");
  assert.equal(notificationBadge(9), "9");
  assert.equal(notificationBadge(10), "9+");
});

test("save errors reach an existing or subsequently mounted inbox only", () => {
  clearNotificationError();
  const errors: string[] = [];
  const unsubscribe = subscribeNotificationErrors((message) =>
    errors.push(message),
  );
  reportNotificationError("Disk is full");
  unsubscribe();
  reportNotificationError("Storage unavailable");
  assert.deepEqual(errors, ["Disk is full"]);
  const stop = subscribeNotificationErrors((message) => errors.push(message));
  assert.deepEqual(errors, ["Disk is full", "Storage unavailable"]);
  stop();
  clearNotificationError();
});

test("notification sources use saved CLI identity and only exact legacy Claude titles", () => {
  const item = {
    id: "1",
    kind: "finished" as const,
    title: "Agent finished",
    body: "Workspace",
    createdAt: 0,
    read: false,
  };
  assert.equal(
    notificationSource({ ...item, agent: "codex" }),
    "Notification from Codex",
  );
  assert.equal(
    notificationSource({ ...item, agent: "claude" }),
    "Notification from Claude Code",
  );
  assert.equal(notificationSource(item), "Notification from terminal");
  assert.equal(
    notificationAgent({ ...item, title: "Claude Code needs your input" }),
    "claude",
  );
  assert.equal(
    notificationAgent({
      ...item,
      title: "Claude Code finished responding",
      agent: null,
    }),
    "claude",
  );
  assert.equal(
    notificationAgent({
      ...item,
      title: "Claude Code finished responding",
      agent: "codex",
    }),
    "codex",
  );
  assert.equal(
    notificationAgent({
      ...item,
      title: "Claude Code finished responding elsewhere",
    }),
    null,
  );
  assert.equal(notificationAgent({ ...item, agent: "unknown" as never }), null);
  assert.equal(
    notificationSource({ ...item, agent: "toString" as never }),
    "Notification from terminal",
  );
});

test("relative notification times cross minute, hour and day boundaries and tolerate clock skew", () => {
  const now = 200_000_000;
  for (const [age, expected] of [
    [-1, "Just now"],
    [0, "Just now"],
    [59_999, "Just now"],
    [60_000, "1m ago"],
    [3_599_999, "59m ago"],
    [3_600_000, "1h ago"],
    [86_399_999, "23h ago"],
    [86_400_000, "1d ago"],
    [259_200_000, "3d ago"],
  ] as const)
    assert.equal(notificationRelativeTime(now - age, now), expected);
  assert.equal(notificationRelativeTime(NaN, now), "Unknown time");
  assert.equal(notificationRelativeTime(0, Infinity), "Unknown time");
});

test("application sources override legacy title inference and describe informational events", () => {
  const item = {
    id: "notification-1",
    kind: "info" as const,
    title: "Claude Code needs your input",
    body: "Application message",
    createdAt: 0,
    read: false,
  };
  for (const [source, label] of [
    ["app", "Lomi"],
    ["android", "Android"],
    ["mcp", "MCP"],
  ] as const) {
    assert.equal(
      notificationSource({ ...item, source }),
      `Notification from ${label}`,
    );
    assert.equal(notificationAgent({ ...item, source }), null);
  }
  assert.equal(notificationKindLabel(item), "Information");
  assert.equal(
    notificationKindLabel({ ...item, kind: "attention" }),
    "Needs attention",
  );
  assert.equal(
    notificationKindLabel({ ...item, kind: "finished" }),
    "Finished",
  );
});
