import assert from "node:assert/strict";
import { test } from "node:test";
import {
  chatTabs,
  newChatTab,
  newProject,
  newSession,
  restoreSession,
  splitPane,
  panes,
  layoutPanes,
  removeChatConversation,
  updateChat,
} from "../src/model.ts";
const info = {
  directory: "/project",
  home: "/home/test",
  platform: "linux",
  profiles: [],
};
test("session v3 retains chat identity across splits and legacy upgrades without owning a PTY", () => {
  const project = newProject("/project", "shell"),
    workspace = project.workspaces[0],
    terminal = workspace.tabs[0];
  assert.equal(terminal.type, "terminal");
  if (terminal.type !== "terminal") return;
  const chat = newChatTab("conversation");
  terminal.layout = splitPane(
    terminal.layout,
    terminal.activePaneId,
    "horizontal",
    chat,
  );
  terminal.activePaneId = chat.id;
  const second = { ...chat, id: "second-view", customTitle: "Local alias" };
  workspace.tabs.push(second);
  const session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  for (const version of [1, 2, 3, 4, 5]) {
    const saved = JSON.parse(JSON.stringify({ ...session, version }));
    delete saved.projects[0].workspaces[0].tabs[0].type;
    const restored = restoreSession(saved, info);
    assert.equal(restored.version, 5);
    assert.deepEqual(chatTabs(restored), [chat, second]);
    const tab = restored.projects[0].workspaces[0].tabs[0];
    assert.equal(tab.type, "terminal");
    if (tab.type === "terminal") {
      assert.equal(panes(tab.layout).length, 1);
      assert.equal(layoutPanes(tab.layout).length, 2);
    }
  }
  const renamed = updateChat(session, second.id, { title: "Shared title" });
  assert.equal(chatTabs(renamed)[1].customTitle, "Local alias");
  const removed = removeChatConversation(session, "conversation", "shell");
  assert.equal(chatTabs(removed).length, 0);
  const remaining = removed.projects[0].workspaces[0].tabs[0];
  assert.equal(remaining.type, "terminal");
  if (remaining.type === "terminal")
    assert.equal(panes(remaining.layout).length, 1);
});
test("unknown or corrupt sessions and malformed conversation descriptors fail without becoming terminals", () => {
  for (const value of [
    { version: 6, projects: [] },
    { version: 3, projects: null },
    {},
  ])
    assert.throws(() => restoreSession(value, info));
  const project = newProject("/project", "shell");
  project.workspaces[0].tabs = [
    { ...newChatTab("conversation"), conversationId: "../invalid" },
  ];
  assert.throws(() =>
    restoreSession({ ...newSession(), projects: [project] }, info),
  );
  assert.equal(restoreSession(null, info).version, 5);
});
