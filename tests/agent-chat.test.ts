import assert from "node:assert/strict";
import { test } from "node:test";
import { openAgentChat } from "../src/agent-chat.ts";
import { newChatTab, newProject, newSession } from "../src/model.ts";

test("chat open reuses the exact standalone descriptor and preserves other runtime identities", () => {
  const project = newProject("/project", "shell");
  const workspace = project.workspaces[0];
  const terminal = workspace.tabs[0];
  const chat = { ...newChatTab("conversation"), customTitle: "Human title" };
  workspace.tabs.push(chat);
  const session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const command = {
    type: "open_chat" as const,
    workspaceId: workspace.id,
    conversationId: chat.conversationId,
    panelId: chat.id,
    create: false,
    notAfterMillis: "0",
  };
  const existing = openAgentChat(session, project.id, command, "Changed title");
  assert.strictEqual(existing.panel, chat);
  assert.strictEqual(
    existing.session.projects[0].workspaces[0].tabs,
    workspace.tabs,
  );
  assert.equal(existing.session.projects[0].workspaces[0].activeTabId, chat.id);
  const created = openAgentChat(
    existing.session,
    project.id,
    {
      ...command,
      conversationId: "new-conversation",
      panelId: "new-panel",
      create: true,
    },
    "Chat AI",
  );
  const tabs = created.session.projects[0].workspaces[0].tabs;
  assert.strictEqual(tabs[0], terminal);
  assert.strictEqual(tabs[1], chat);
  assert.equal(tabs.length, 3);
  assert.equal(created.panel.id, "new-panel");
  assert.equal(workspace.tabs.length, 2);
});

test("mixed-layout reveal reuses the exact chat pane and preserves sibling identities", () => {
  const project = newProject("/project", "shell");
  const workspace = project.workspaces[0];
  const terminal = workspace.tabs[0];
  assert.equal(terminal.type, "terminal");
  const chat = newChatTab("conversation");
  terminal.layout = {
    type: "split",
    id: "split",
    axis: "horizontal",
    ratio: 0.5,
    first: terminal.layout,
    second: chat,
  };
  const session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const before = JSON.stringify(session);
  const command = {
    type: "open_chat" as const,
    workspaceId: workspace.id,
    conversationId: chat.conversationId,
    panelId: chat.id,
    create: false,
    notAfterMillis: "0",
  };
  const opened = openAgentChat(session, project.id, command, "Chat AI");
  const changed = opened.session.projects[0].workspaces[0];
  assert.equal(changed.activeTabId, terminal.id);
  assert.equal(changed.tabs[0].type, "terminal");
  assert.equal(changed.tabs[0].activePaneId, chat.id);
  assert.strictEqual(changed.tabs[0].layout, terminal.layout);
  assert.strictEqual(opened.panel, chat);
  assert.equal(JSON.stringify(session), before);
  assert.throws(
    () =>
      openAgentChat(
        session,
        project.id,
        { ...command, panelId: "different" },
        "Chat AI",
      ),
    /REVISION_CONFLICT/,
  );
});
