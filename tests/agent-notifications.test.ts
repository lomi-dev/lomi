import assert from "node:assert/strict";
import { test } from "node:test";
import {
  createAgentNotificationGate,
  notificationContext,
  parseAgentSignal,
} from "../src/agent-notifications.ts";
import { newProject, newSession, newWorkspace } from "../src/model.ts";

test("only explicit Lomi Claude signals can trigger agent notifications", () => {
  for (const signal of ["working", "attention", "finished"])
    assert.equal(parseAgentSignal(`notify;Lomi;claude;${signal}`), signal);
  for (const invalid of [
    "",
    "finished",
    "notify;Other;claude;finished",
    "notify;Lomi;other;finished",
    "notify;Lomi;claude;finished;extra",
    "notify;Lomi;claude;finished\n",
    "notify;Lomi;claude;" + "x".repeat(10000),
  ])
    assert.equal(parseAgentSignal(invalid), null);
});

test("notification cooldown keeps terminals and event kinds independent and does not reset on working", () => {
  const first = createAgentNotificationGate();
  const second = createAgentNotificationGate();
  assert.equal(first("working", 0), false);
  assert.equal(first("finished", 1), true);
  assert.equal(first("finished", 2), false);
  assert.equal(second("finished", 2), true);
  assert.equal(first("attention", 3), true);
  assert.equal(first("working", 4), false);
  assert.equal(first("finished", 5), false);
  assert.equal(first("finished", 2001), true);
  assert.equal(first("finished", 0), true);
});

test("notification context follows panes moved between workspaces and disappears when closed", () => {
  const project = newProject("/projects/example", "local:zsh");
  const workspace = project.workspaces[0];
  const tab = workspace.tabs[0];
  assert.equal(tab.type, "terminal");
  if (tab.type !== "terminal") return;
  const id = tab.activePaneId;
  const session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  workspace.name = "First";
  tab.customTitle = "Task";
  assert.equal(notificationContext(session, id), "example · First · Task");
  const next = newWorkspace("/projects/example", "local:zsh", "Second");
  next.name = "Second";
  project.workspaces.push(next);
  workspace.tabs = [];
  next.tabs.push(tab);
  assert.equal(notificationContext(session, id), "example · Second · Task");
  tab.customTitle = "x\n".repeat(500);
  assert.equal(notificationContext(session, id)?.length, 300);
  assert.equal(notificationContext(session, id)?.includes("\n"), false);
  next.tabs = [];
  assert.equal(notificationContext(session, id), null);
  assert.equal(notificationContext(undefined, id), null);
});
