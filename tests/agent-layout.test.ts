import assert from "node:assert/strict";
import { test } from "node:test";
import { moveAgentPanel, panelMoveIdentities } from "../src/agent-layout.ts";
import {
  active,
  layoutPanes,
  newProject,
  newSession,
  newFileTab,
  newTab,
  newWorkspace,
} from "../src/model.ts";

test("panel docking preserves descriptors, explicit source profile and runtime identities", () => {
  const project = newProject("/project", "default");
  const workspace = project.workspaces[0];
  const target = workspace.tabs[0];
  assert.equal(target.type, "terminal");
  const source = newTab("/project", "alternate");
  const file = newFileTab(newSession());
  workspace.tabs.push(source, file);
  let session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const sessions = new Map([
    [source.activePaneId, "retained-source"],
    [target.activePaneId, "retained-target"],
  ]);
  const identify = (id: string) => sessions.get(id) ?? null;
  const before = panelMoveIdentities(workspace, identify);
  session = moveAgentPanel(
    session,
    project.id,
    workspace.id,
    {
      type: "dock_tab",
      tabId: source.id,
      targetTabId: target.id,
      side: "right",
    },
    { width: 1000, height: 700 },
  );
  const selected = active(session)!;
  assert.equal(selected.tab.type, "terminal");
  const panes = layoutPanes(selected.tab.layout);
  assert.equal(
    panes.find((p) => p.id === source.activePaneId)?.profileId,
    "alternate",
  );
  assert.strictEqual(
    panes.find((p) => p.id === target.activePaneId),
    target.layout,
  );
  assert.deepEqual(
    panelMoveIdentities(selected.workspace, identify),
    before.map((p) => ({
      ...p,
      tabId: p.tabId === source.id ? target.id : p.tabId,
    })),
  );
  session = moveAgentPanel(
    session,
    project.id,
    workspace.id,
    {
      type: "dock_tab",
      tabId: file.id,
      targetTabId: target.id,
      side: "bottom",
    },
    { width: 1000, height: 700 },
  );
  const tab = active(session)!.tab;
  assert.equal(tab.type, "terminal");
  assert.strictEqual(
    layoutPanes(tab.layout).find((p) => p.id === file.id),
    file,
  );
  const moved = moveAgentPanel(
    session,
    project.id,
    workspace.id,
    {
      type: "move_pane",
      panelId: file.id,
      targetPanelId: source.activePaneId,
      side: "top",
    },
    { width: 1000, height: 700 },
  );
  assert.deepEqual(
    panelMoveIdentities(active(moved)!.workspace, identify),
    panelMoveIdentities(active(session)!.workspace, identify),
  );
  const movedTab = active(moved)!.tab;
  assert.equal(movedTab.type, "terminal");
  assert.equal(movedTab.activePaneId, file.id);
  assert.strictEqual(
    layoutPanes(movedTab.layout).find((p) => p.id === file.id),
    file,
  );
});

test("workspace transfer preserves the whole mixed descriptor and cannot activate a replacement shell", () => {
  const project = newProject("/project", "source-profile");
  const source = project.workspaces[0];
  const moved = source.tabs[0];
  assert.equal(moved.type, "terminal");
  const file = newFileTab(newSession());
  const lazy = newTab("/project", "lazy-profile");
  source.tabs.push(lazy);
  moved.layout = {
    type: "split",
    id: "fixture-split",
    axis: "horizontal",
    ratio: 0.5,
    first: moved.layout,
    second: file,
  };
  const destination = newWorkspace("/project", "target-profile");
  project.workspaces.push(destination);
  const before = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const after = moveAgentPanel(
    before,
    project.id,
    source.id,
    {
      type: "transfer_tab",
      tabId: moved.id,
      targetWorkspaceId: destination.id,
      beforeTabId: null,
    },
    null,
  );
  assert.equal(after.activeProjectId, null);
  const updated = after.projects[0];
  assert.strictEqual(updated.workspaces[0].tabs[0], lazy);
  assert.strictEqual(updated.workspaces[1].tabs.at(-1), moved);
  assert.strictEqual(
    layoutPanes(
      (updated.workspaces[1].tabs.at(-1) as typeof moved).layout,
    ).find((p) => p.id === file.id),
    file,
  );
  assert.equal(source.tabs.length, 2, "Original session is immutable");
  assert.throws(
    () =>
      moveAgentPanel(
        before,
        project.id,
        source.id,
        {
          type: "transfer_tab",
          tabId: moved.id,
          targetWorkspaceId: "foreign",
          beforeTabId: null,
        },
        null,
      ),
    /TARGET_NOT_FOUND/,
  );
  const hidden = {
    ...before,
    projects: [{ ...project, activeWorkspaceId: destination.id }],
  };
  const kept = moveAgentPanel(
    hidden,
    project.id,
    source.id,
    {
      type: "transfer_tab",
      tabId: moved.id,
      targetWorkspaceId: destination.id,
      beforeTabId: destination.tabs[0].id,
    },
    null,
  );
  assert.equal(
    active(kept)!.tab.id,
    destination.tabs[0].id,
    "Destination focus is preserved",
  );
  const empty = {
    ...hidden,
    projects: [
      {
        ...hidden.projects[0],
        workspaces: [source, { ...destination, tabs: [], activeTabId: "" }],
      },
    ],
  };
  const neutral = moveAgentPanel(
    empty,
    project.id,
    source.id,
    {
      type: "transfer_tab",
      tabId: moved.id,
      targetWorkspaceId: destination.id,
      beforeTabId: null,
    },
    null,
  );
  assert.equal(
    neutral.activeProjectId,
    null,
    "An empty selected destination does not start transferred lazy resources",
  );
});
test("reordering preserves selection and lazy descriptors; invalid or cramped docking has no effect", () => {
  const project = newProject("/project", "default");
  const workspace = project.workspaces[0];
  const source = newTab("/project", "other");
  workspace.tabs.push(source);
  const session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const reordered = moveAgentPanel(
    session,
    project.id,
    workspace.id,
    {
      type: "reorder_tab",
      tabId: source.id,
      beforeTabId: workspace.tabs[0].id,
    },
    null,
  );
  assert.equal(active(reordered)!.tab.id, workspace.activeTabId);
  assert.strictEqual(active(reordered)!.workspace.tabs[0], source);
  for (const size of [null, { width: 100, height: 100 }])
    assert.throws(
      () =>
        moveAgentPanel(
          session,
          project.id,
          workspace.id,
          {
            type: "dock_tab",
            tabId: source.id,
            targetTabId: workspace.activeTabId,
            side: "right",
          },
          size,
        ),
      /PANEL_NOT_RENDERABLE/,
    );
  assert.throws(
    () =>
      moveAgentPanel(
        session,
        project.id,
        "foreign",
        { type: "reorder_tab", tabId: source.id, beforeTabId: null },
        null,
      ),
    /TARGET_NOT_FOUND/,
  );
  assert.equal(workspace.tabs.length, 2);
});
