import assert from "node:assert/strict";
import { test } from "node:test";
import {
  layoutPanes,
  movePaneToTab,
  newPane,
  newProject,
  newSession,
  newTab,
  restoreSession,
  splitPane,
} from "../src/model.ts";
import type { LayoutPane, Tab, TerminalTab, Workspace } from "../src/model.ts";

const size = { width: 1200, height: 900 };
const descriptors: LayoutPane[] = [
  { type: "terminal", id: "pane", cwd: "/project/nested" },
  {
    type: "file",
    id: "file",
    title: "File",
    customTitle: "Draft",
    root: "/project",
    relative: "file.md",
    position: { anchor: 2, head: 4, scrollTop: 10, scrollLeft: 3 },
    previewView: "split",
    previewRatio: 0.4,
  },
  {
    type: "browser",
    id: "browser",
    title: "Browser",
    url: "https://example.com/",
    automation: { generation: "generation", profileId: "browser-profile" },
  },
  {
    type: "android",
    id: "android",
    title: "Android",
    deviceId: "device",
    startMode: "manual",
  },
  { type: "chat", id: "chat", title: "Chat", conversationId: "conversation" },
  { type: "agent-task", id: "agent", title: "Agent", taskId: "task" },
  {
    type: "plugin",
    id: "plugin",
    title: "Plugin",
    customTitle: "Tools",
    owner: "example.tools",
    viewType: "example.tools.view",
    stateVersion: 2,
    state: { expanded: true, items: [1, 2] },
  },
];

function fixture(pane: LayoutPane, active = true) {
  const source = newTab("/project/source", "zsh", "Source");
  source.customTitle = "Source title";
  const retained = source.layout;
  source.layout = splitPane(retained, retained.id, "vertical", pane);
  if (active) source.activePaneId = pane.id;
  const target = newTab("/project/target", "bash", "Target");
  target.customTitle = "Target title";
  const workspace: Workspace = {
    id: "workspace",
    name: "Workspace",
    tabs: [source, target],
    activeTabId: source.id,
  };
  return { source, target, retained, workspace };
}

function terminal(tab: Tab | undefined): TerminalTab {
  assert.equal(tab?.type, "terminal");
  return tab as TerminalTab;
}

for (const pane of descriptors) {
  test(`extracting ${pane.type} preserves its descriptor and source selection`, () => {
    const { source, target, retained, workspace } = fixture(pane);
    const before = structuredClone(workspace);
    const next = movePaneToTab(
      workspace,
      source.id,
      pane.id,
      { type: "new-tab", beforeId: target.id },
      size,
    );
    assert.equal(next.tabs.length, 3);
    assert.equal(terminal(next.tabs[0]).layout, retained);
    assert.equal(terminal(next.tabs[0]).activePaneId, retained.id);
    const extracted = next.tabs[1];
    assert.equal(next.activeTabId, extracted.id);
    assert.equal(next.tabs[2], target);
    if (pane.type === "terminal") {
      const tab = terminal(extracted);
      assert.notEqual(tab.id, pane.id);
      assert.equal(tab.layout, pane);
      assert.equal(tab.profileId, "zsh");
      assert.equal(tab.title, source.title);
      assert.equal(tab.customTitle, source.customTitle);
    } else {
      assert.equal(extracted, pane);
    }
    assert.deepEqual(workspace, before);
  });

  test(`merging ${pane.type} keeps the target wrapper and the whole target layout`, () => {
    const { source, target, retained, workspace } = fixture(pane, false);
    const extra = newPane("/target/extra");
    target.layout = splitPane(
      target.layout,
      target.activePaneId,
      "vertical",
      extra,
    );
    const before = structuredClone(workspace);
    const next = movePaneToTab(
      workspace,
      source.id,
      pane.id,
      { type: "tab", tabId: target.id },
      size,
    );
    const merged = terminal(next.tabs[1]);
    assert.equal(merged.id, target.id);
    assert.equal(merged.title, target.title);
    assert.equal(merged.customTitle, target.customTitle);
    assert.equal(merged.profileId, "bash");
    assert.equal(merged.activePaneId, pane.id);
    assert.equal(next.activeTabId, merged.id);
    assert.equal(terminal(next.tabs[0]).activePaneId, retained.id);
    assert.equal(merged.layout.type, "split");
    if (merged.layout.type !== "split") throw new Error("Expected split");
    assert.equal(merged.layout.first, target.layout);
    assert.equal(merged.layout.axis, "horizontal");
    if (pane.type === "terminal") {
      assert.deepEqual(merged.layout.second, { ...pane, profileId: "zsh" });
    } else {
      assert.equal(merged.layout.second, pane);
    }
    assert.deepEqual(workspace, before);
  });
}

for (const target of descriptors.filter((pane) => pane.type !== "terminal")) {
  test(`wrapping a standalone ${target.type} preserves its descriptor and title`, () => {
    const source = newTab("/project", "fish");
    const workspace: Workspace = {
      id: "workspace",
      name: "Workspace",
      tabs: [source, target],
      activeTabId: source.id,
    };
    const next = movePaneToTab(
      workspace,
      source.id,
      source.activePaneId,
      { type: "tab", tabId: target.id },
      { width: 240, height: 243 },
    );
    assert.equal(next.tabs.length, 1);
    const merged = terminal(next.tabs[0]);
    assert.notEqual(merged.id, target.id);
    assert.equal(merged.title, target.title);
    assert.equal(merged.customTitle, target.customTitle);
    assert.equal(merged.profileId, source.profileId);
    assert.equal(next.activeTabId, merged.id);
    assert.equal(merged.layout.type, "split");
    if (merged.layout.type !== "split") throw new Error("Expected split");
    assert.equal(merged.layout.axis, "vertical");
    assert.equal(merged.layout.first, target);
    assert.equal(merged.layout.second, source.layout);
    const ids = [
      merged.id,
      ...layoutPanes(merged.layout).map((pane) => pane.id),
    ];
    assert.equal(new Set(ids).size, ids.length);
  });
}

test("terminal profiles and directories survive transfers and session restoration", () => {
  for (const profileId of [undefined, "fish"]) {
    for (const destinationType of ["tab", "new-tab"] as const) {
      const pane = {
        ...newPane("/project/working"),
        ...(profileId ? { profileId } : {}),
      };
      const { source, target, workspace } = fixture(pane);
      const next = movePaneToTab(
        workspace,
        source.id,
        pane.id,
        destinationType === "tab"
          ? { type: "tab", tabId: target.id }
          : { type: "new-tab", beforeId: null },
        size,
      );
      const project = newProject("/project", "bash");
      project.workspaces = [next];
      project.activeWorkspaceId = next.id;
      const state = {
        ...newSession(),
        activeProjectId: project.id,
        projects: [project],
      };
      const restored = restoreSession(JSON.parse(JSON.stringify(state)), {
        directory: "/project",
        home: "/home/user",
        platform: "linux",
        profiles: [],
      });
      const restoredWorkspace = restored.projects[0].workspaces[0];
      const restoredTab = terminal(
        restoredWorkspace.tabs.find((tab) => tab.id === next.activeTabId),
      );
      const restoredPane = layoutPanes(restoredTab.layout).find(
        (item) => item.id === pane.id,
      );
      assert.equal(restoredPane?.type, "terminal");
      if (restoredPane?.type !== "terminal")
        throw new Error("Expected terminal pane");
      assert.equal(restoredPane.cwd, pane.cwd);
      assert.equal(
        restoredPane.profileId ?? restoredTab.profileId,
        profileId ?? "zsh",
      );
      assert.equal(
        restoredTab.customTitle,
        destinationType === "tab" ? target.customTitle : source.customTitle,
      );
    }
  }
});

test("extracting the last descriptor removes its wrapper and inserts at the removed anchor", () => {
  const pane = descriptors[1];
  const source = {
    ...newTab("/project", "bash"),
    layout: pane,
    activePaneId: pane.id,
  };
  const target = newTab("/project", "bash");
  const workspace: Workspace = {
    id: "workspace",
    name: "Workspace",
    tabs: [source, target],
    activeTabId: target.id,
  };
  const next = movePaneToTab(
    workspace,
    source.id,
    pane.id,
    { type: "new-tab", beforeId: source.id },
    size,
  );
  assert.deepEqual(next.tabs, [pane, target]);
  assert.equal(next.activeTabId, pane.id);
});

test("extracting a sole terminal reorders and activates its existing wrapper", () => {
  const source = newTab("/project", "bash");
  const target = newTab("/project", "bash");
  const workspace: Workspace = {
    id: "workspace",
    name: "Workspace",
    tabs: [source, target],
    activeTabId: target.id,
  };
  const next = movePaneToTab(
    workspace,
    source.id,
    source.activePaneId,
    { type: "new-tab", beforeId: null },
    size,
  );
  assert.deepEqual(next.tabs, [target, source]);
  assert.equal(next.tabs[1], source);
  assert.equal(next.activeTabId, source.id);
});

test("stale, self, incompatible, duplicate and undersized transfers return the original workspace", () => {
  const pane = descriptors[0];
  const { source, target, workspace } = fixture(pane);
  for (const [sourceId, paneId, destination, bounds] of [
    ["missing", pane.id, { type: "tab", tabId: target.id }, size],
    [target.id, pane.id, { type: "tab", tabId: source.id }, size],
    [source.id, "missing", { type: "new-tab", beforeId: null }, size],
    [source.id, pane.id, { type: "tab", tabId: source.id }, size],
    [source.id, pane.id, { type: "tab", tabId: "missing" }, size],
    [source.id, pane.id, { type: "new-tab", beforeId: "missing" }, size],
    [
      source.id,
      pane.id,
      { type: "new-tab", beforeId: null },
      { width: 239, height: 900 },
    ],
    [
      source.id,
      pane.id,
      { type: "new-tab", beforeId: null },
      { width: 900, height: 119 },
    ],
    [
      source.id,
      pane.id,
      { type: "tab", tabId: target.id },
      { width: 482, height: 242 },
    ],
  ] as const) {
    assert.equal(
      movePaneToTab(workspace, sourceId, paneId, destination, bounds),
      workspace,
    );
  }
  for (const incompatible of [
    {
      type: "diff" as const,
      id: "diff",
      title: "Diff",
      root: "/project",
      relative: "file",
      staged: false,
    },
    {
      type: "commit" as const,
      id: "commit",
      title: "Commit",
      root: "/project",
      commit: "a".repeat(40),
    },
  ]) {
    const invalid = { ...workspace, tabs: [...workspace.tabs, incompatible] };
    assert.equal(
      movePaneToTab(
        invalid,
        source.id,
        pane.id,
        { type: "tab", tabId: incompatible.id },
        size,
      ),
      invalid,
    );
    assert.equal(
      movePaneToTab(
        invalid,
        incompatible.id,
        pane.id,
        { type: "tab", tabId: target.id },
        size,
      ),
      invalid,
    );
  }
  for (const tabs of [
    [source, { ...target, layout: pane }],
    [source, { ...target, id: pane.id }],
    [source, source, target],
  ]) {
    const invalid = { ...workspace, tabs };
    assert.equal(
      movePaneToTab(
        invalid,
        source.id,
        pane.id,
        { type: "tab", tabId: target.id },
        size,
      ),
      invalid,
    );
    assert.equal(
      movePaneToTab(
        invalid,
        source.id,
        pane.id,
        { type: "new-tab", beforeId: null },
        size,
      ),
      invalid,
    );
  }
});
