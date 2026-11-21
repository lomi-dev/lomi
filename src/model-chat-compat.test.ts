import assert from "node:assert/strict";
import test from "node:test";
import { layoutPanes, restoreSession, type AppInfo } from "./model.ts";

const info = { profiles: [{ id: "sh" }] } as unknown as AppInfo;
const term = (id: string) => ({ type: "terminal", id, cwd: "/p" });
const chat = (id: string) => ({
  type: "chat",
  id,
  title: "Chat AI",
  conversationId: "c1",
});
const agent = (id: string) => ({
  type: "agent-task",
  id,
  title: "A",
  taskId: "x1",
});
const legacy = (id: string) => ({ type: "cli-agent", id, runId: "r1" });
const session = (tabs: unknown[]) => ({
  version: 6,
  projects: [
    {
      id: "p",
      path: "/p",
      workspaces: [{ id: "w", name: "W", tabs, activeTabId: "t2" }],
    },
  ],
});
const tabsOf = (value: unknown) =>
  restoreSession(value, info).projects[0].workspaces[0].tabs;

test("legacy chat tabs and panes are dropped silently", () => {
  const tabs = tabsOf(
    session([
      chat("t1"),
      {
        type: "terminal",
        id: "t2",
        activePaneId: "a",
        layout: {
          type: "split",
          id: "s",
          axis: "horizontal",
          ratio: 0.5,
          first: chat("x"),
          second: {
            type: "split",
            id: "s2",
            axis: "vertical",
            ratio: 0.5,
            first: term("a"),
            second: chat("y"),
          },
        },
      },
      { type: "terminal", id: "t3", layout: chat("z") },
    ]),
  );
  assert.deepEqual(
    tabs.map((t) => t.id),
    ["t2"],
  );
  const tab = tabs[0];
  assert.ok(tab.type === "terminal");
  assert.deepEqual(
    layoutPanes(tab.layout).map((p) => p.id),
    ["a"],
  );
  assert.equal(tab.layout.type, "terminal");
});

test("a workspace with only chat tabs gets a fresh terminal tab", () => {
  const tabs = tabsOf(session([chat("t1")]));
  assert.equal(tabs.length, 1);
  assert.equal(tabs[0].type, "terminal");
});

test("agent-task and cli-agent tabs and panes are dropped silently", () => {
  const tabs = tabsOf(
    session([
      agent("t1"),
      legacy("t4"),
      {
        type: "terminal",
        id: "t2",
        activePaneId: "a",
        layout: {
          type: "split",
          id: "s",
          axis: "horizontal",
          ratio: 0.5,
          first: agent("x"),
          second: {
            type: "split",
            id: "s2",
            axis: "vertical",
            ratio: 0.5,
            first: term("a"),
            second: legacy("y"),
          },
        },
      },
    ]),
  );
  assert.deepEqual(
    tabs.map((t) => t.id),
    ["t2"],
  );
  assert.equal(tabsOf(session([agent("t1")]))[0].type, "terminal");
});
