import assert from "node:assert/strict";
import test from "node:test";
import {
  canMergeTabs,
  layoutPanes,
  mergeTabs,
  minimumLayoutSize,
  newPane,
  newTab,
  newThreadTab,
  restoreSession,
  updateThread,
  type AppInfo,
  type Layout,
  type Tab,
  type ThreadTab,
  type Workspace,
} from "./model.ts";

const info = { profiles: [{ id: "sh" }] } as unknown as AppInfo;
const T = "a".repeat(32);
const thread = (id: string, extra: object = {}) => ({
  type: "thread",
  id,
  title: "New chat",
  threadId: T,
  ...extra,
});
const session = (version: number, tabs: unknown[]) => ({
  version,
  projects: [
    {
      id: "p",
      path: "/p",
      workspaces: [{ id: "w", name: "W", tabs, activeTabId: "t1" }],
    },
  ],
});
const restore = (v: unknown) => restoreSession(v, info);
const nested = {
  type: "terminal",
  id: "t2",
  activePaneId: "a",
  layout: {
    type: "split",
    id: "s",
    axis: "horizontal",
    ratio: 0.5,
    first: { type: "terminal", id: "a", cwd: "/p" },
    second: thread("n"),
  },
};

test("invalid thread ids are rejected", () => {
  for (const bad of [{ threadId: "xyz" }, { threadId: undefined }])
    assert.throws(
      () => restore(session(6, [thread("t1", bad)])),
      /Invalid saved chat descriptor/,
    );
});

test("thread taskId is accepted on read and dropped", () => {
  const tab = restore(session(6, [thread("t1", { taskId: "k" })])).projects[0]
    .workspaces[0].tabs[0];
  assert.deepEqual(tab, thread("t1"));
  assert.doesNotThrow(() =>
    restore(session(6, [thread("t1", { taskId: "bad id" })])),
  );
});

test("thread tabs round-trip in tabs and nested layouts", () => {
  const restored = restore(
    session(6, [thread("t1", { customTitle: "X" }), nested]),
  );
  assert.equal(restored.version, 6);
  const [tab, term] = restored.projects[0].workspaces[0].tabs;
  assert.deepEqual(tab, thread("t1", { customTitle: "X" }));
  assert.ok(term.type === "terminal");
  assert.deepEqual(layoutPanes(term.layout)[1], nested.layout.second);
  assert.deepEqual(restore(restored), restored);
});

test("updateThread patches tabs and panes", () => {
  const s = restore(session(6, [thread("t1"), nested]));
  const a = updateThread(s, "t1", { title: "Z" });
  const tab = a.projects[0].workspaces[0].tabs[0];
  assert.ok(tab.type === "thread" && tab.title === "Z");
  const b = updateThread(s, "n", { threadId: null });
  const term = b.projects[0].workspaces[0].tabs[1];
  assert.ok(term.type === "terminal");
  assert.equal(
    (layoutPanes(term.layout)[1] as { threadId: null }).threadId,
    null,
  );
  assert.equal(newThreadTab().threadId, null);
});

const ws = (tabs: Tab[]): Workspace => ({
  id: "w",
  name: "W",
  tabs,
  activeTabId: tabs[0].id,
});
const chat = (id: string, threadId: string | null = T): ThreadTab => ({
  type: "thread",
  id,
  title: "Chat",
  threadId,
});

test("minimumLayoutSize uses per-pane minimums", () => {
  const split = (axis: "horizontal" | "vertical"): Layout => ({
    type: "split",
    id: "s",
    axis,
    ratio: 0.5,
    first: chat("c"),
    second: newPane("/p"),
  });
  assert.deepEqual(minimumLayoutSize(chat("c")), { width: 360, height: 280 });
  assert.deepEqual(minimumLayoutSize(split("horizontal")), {
    width: 360 + 3 + 240,
    height: 280,
  });
  assert.deepEqual(minimumLayoutSize(split("vertical")), {
    width: 360,
    height: 280 + 3 + 120,
  });
});

test("a thread tab accepts merges as a layout container", () => {
  const target = chat("c1");
  const source = chat("c2", "b".repeat(32));
  const out = mergeTabs(
    ws([target, source]),
    "c2",
    "c1",
    "right",
    { width: 800, height: 400 },
    "sh",
  );
  assert.equal(out.tabs.length, 1);
  const tab = out.tabs[0];
  assert.equal(tab.type, "terminal");
  if (tab.type !== "terminal") return;
  assert.equal(tab.id, "c1");
  assert.equal(tab.profileId, "sh");
  assert.equal(tab.activePaneId, "c2");
  assert.equal(tab.layout.type, "split");
  assert.deepEqual(
    layoutPanes(tab.layout).map((pane) => [
      pane.id,
      (pane as ThreadTab).threadId,
    ]),
    [
      ["c1", T],
      ["c2", "b".repeat(32)],
    ],
  );
});

test("thread merges are refused below chat minimums", () => {
  const tabs = [chat("c1"), chat("c2")];
  const workspace = ws(tabs);
  assert.equal(
    mergeTabs(workspace, "c2", "c1", "right", { width: 700, height: 400 }),
    workspace,
  );
  assert.equal(
    canMergeTabs(tabs[1], tabs[0], "right", { width: 723, height: 280 }),
    true,
  );
  assert.equal(
    canMergeTabs(tabs[1], tabs[0], "right", { width: 722, height: 280 }),
    false,
  );
  assert.equal(
    canMergeTabs(tabs[1], tabs[0], "bottom", { width: 360, height: 400 }),
    false,
  );
  assert.equal(
    canMergeTabs(newTab("/p", "sh"), tabs[0], "right", {
      width: 602,
      height: 280,
    }),
    false,
  );
  assert.equal(
    canMergeTabs(newTab("/p", "sh"), tabs[0], "right", {
      width: 603,
      height: 280,
    }),
    true,
  );
});
