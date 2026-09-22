import assert from "node:assert/strict";
import { test } from "node:test";
import {
  active,
  addWorkspace,
  newBrowserTab,
  newPane,
  newProject,
  newSession,
  newTab,
  newFileTab,
  newWorkspace,
  openCommitTab,
  openDiffTab,
  canMergeTabs,
  openFileTab,
  fileTabs,
  panes,
  removePane,
  removeTabs,
  removeWorkspace,
  resizeSplit,
  restoreSession,
  splitPane,
  tabsToClose,
  tabTitle,
  updateBrowser,
  updateDirectories,
  updateWorkspace,
  updateTab,
  updateFile,
} from "../src/model.ts";
import type { AppInfo, Split, Tab, TabCloseAction } from "../src/model.ts";
import { inputChunks } from "../src/terminal-utils.ts";
import { applyFileChange } from "../src/explorer-model.ts";

const info: AppInfo = {
  directory: "/project",
  home: "/home/test",
  platform: "linux",
  profiles: [
    {
      id: "local:bash",
      name: "bash",
      kind: "bash",
      program: "/bin/bash",
      distro: null,
      home: "/home/test",
    },
  ],
};

function projectSession() {
  const project = newProject(info.directory, info.profiles[0].id);
  return { ...newSession(), projects: [project], activeProjectId: project.id };
}

test("untitled files keep distinct identities and become ordinary persisted file views", () => {
  const session = projectSession();
  const workspace = session.projects[0].workspaces[0];
  const first = newFileTab(session);
  workspace.tabs.push(first);
  const second = newFileTab(session);
  workspace.tabs.push(second);
  assert.notEqual(first.id, second.id);
  assert.deepEqual([first.title, second.title], ["Untitled-1", "Untitled-2"]);
  const restored = restoreSession(JSON.parse(JSON.stringify(session)), info);
  assert.deepEqual(fileTabs(restored), [first, second]);
  const saved = updateFile(restored, first.id, (file) => {
    const { untitled: _untitled, ...rest } = file;
    return {
      ...rest,
      root: "/elsewhere",
      relative: "note.txt",
      title: "note.txt",
    };
  });
  assert.deepEqual(fileTabs(restoreSession(saved, info)), [
    {
      type: "file",
      id: first.id,
      root: "/elsewhere",
      relative: "note.txt",
      title: "note.txt",
    },
    second,
  ]);
});

test("custom tab titles survive restoration and automatic title updates", () => {
  let session = projectSession();
  const workspace = active(session)!.workspace;
  session = openFileTab(session, workspace.id, "/project", "note.txt");
  session = openCommitTab(
    session,
    workspace.id,
    "/project",
    "a".repeat(40),
    "Commit",
  );
  const browser = newBrowserTab();
  active(session)!.workspace.tabs.push(browser);
  for (const tab of active(session)!.workspace.tabs) {
    assert.equal(tabTitle(tab), tab.title);
    session = updateTab(session, tab.id, (current) => ({
      ...current,
      customTitle: `Custom ${tab.type}`,
    }));
  }
  session = updateBrowser(session, browser.id, {
    title: "New page",
    url: "https://example.com/",
  });
  session = applyFileChange(
    session,
    { oldPath: "/project/note.txt", newPath: "/project/renamed.txt" },
    "local:bash",
  );
  const restored = restoreSession(JSON.parse(JSON.stringify(session)), info);
  assert.deepEqual(restored, session);
  assert.deepEqual(active(restored)!.workspace.tabs.map(tabTitle), [
    "Custom terminal",
    "Custom file",
    "Custom commit",
    "Custom browser",
  ]);
  assert.equal(fileTabs(restored)[0].relative, "renamed.txt");
  assert.equal(fileTabs(restored)[0].title, "renamed.txt");
  assert.equal(active(restored)!.workspace.tabs.at(-1)!.title, "New page");
  for (const invalid of [null, 42, {}, ""]) {
    const data = JSON.parse(JSON.stringify(session));
    for (const tab of data.projects[0].workspaces[0].tabs)
      tab.customTitle = invalid;
    for (const tab of active(restoreSession(data, info))!.workspace.tabs) {
      assert.equal(tab.customTitle, undefined);
      assert.equal(tabTitle(tab), tab.title);
    }
  }
});

test("tab close actions use the clicked tab and preserve modified files", () => {
  const tabs: Tab[] = [
    newTab("/project", "local:bash"),
    {
      type: "file",
      id: "dirty",
      title: "Dirty",
      root: "/project",
      relative: "dirty.txt",
    },
    {
      type: "commit",
      id: "commit",
      title: "Commit",
      root: "/project",
      commit: "a".repeat(40),
    },
    {
      type: "file",
      id: "clean",
      title: "Clean",
      root: "/project",
      relative: "clean.txt",
    },
    newTab("/project", "local:bash"),
  ];
  const modified = new Set(["dirty"]);
  const cases: [TabCloseAction, number[]][] = [
    ["close", [2]],
    ["others", [0, 1, 3, 4]],
    ["left", [0, 1]],
    ["right", [3, 4]],
    ["clean", [0, 2, 3, 4]],
    ["all", [0, 1, 2, 3, 4]],
  ];
  for (const [action, indexes] of cases) {
    assert.deepEqual(
      tabsToClose(tabs, "commit", action, modified),
      indexes.map((index) => tabs[index]),
    );
    assert.deepEqual(tabsToClose(tabs, "missing", action, modified), []);
  }
  assert.deepEqual(tabsToClose(tabs, tabs[0].id, "left", modified), []);
  assert.deepEqual(tabsToClose(tabs, tabs[4].id, "right", modified), []);
  assert.deepEqual(tabsToClose([tabs[0]], tabs[0].id, "others", modified), []);
  assert.deepEqual(tabsToClose([tabs[1]], "dirty", "clean", modified), []);
});

test("bulk removal retains the active tab or selects its nearest survivor", () => {
  const workspace = newWorkspace("/project", "local:bash");
  const tabs = Array.from({ length: 6 }, () =>
    newTab("/project", "local:bash"),
  );
  workspace.tabs = tabs;
  workspace.activeTabId = tabs[2].id;
  const close = (...indexes: number[]) =>
    removeTabs(
      workspace,
      new Set(indexes.map((index) => tabs[index].id)),
      "/project",
      "local:bash",
    );
  assert.equal(close(0, 1, 3).activeTabId, tabs[2].id);
  assert.equal(close(0, 2, 3).activeTabId, tabs[4].id);
  assert.equal(close(2, 3, 4, 5).activeTabId, tabs[1].id);
  assert.deepEqual(close(0, 2, 3).tabs, [tabs[1], tabs[4], tabs[5]]);
  assert.equal(
    removeTabs(workspace, new Set(["missing"]), "/project", "local:bash"),
    workspace,
  );
  assert.equal(workspace.tabs, tabs);
  assert.equal(workspace.activeTabId, tabs[2].id);
});

test("closing all tabs creates exactly one fresh terminal with the chosen environment", () => {
  const workspace = newWorkspace("/project", "local:bash");
  workspace.tabs.push(newTab("/project/src", "wsl:Ubuntu"));
  const next = removeTabs(
    workspace,
    new Set(workspace.tabs.map((tab) => tab.id)),
    "/project",
    "wsl:Ubuntu",
  );
  assert.equal(next.tabs.length, 1);
  const tab = next.tabs[0];
  assert.equal(tab.type, "terminal");
  if (tab.type !== "terminal") throw new Error("Expected a terminal");
  assert.equal(tab.id, next.activeTabId);
  assert.equal(tab.profileId, "wsl:Ubuntu");
  assert.equal(panes(tab.layout)[0].cwd, "/project");
  assert.ok(!workspace.tabs.some((previous) => previous.id === tab.id));
});

test("file tabs deduplicate within each workspace and survive session restoration", () => {
  let state = projectSession();
  const workspace = active(state)!.workspace;
  state = openFileTab(state, workspace.id, "/project", "src/main.rs");
  const file = fileTabs(state)[0];
  assert.equal(active(state)!.tab.id, file.id);
  assert.equal(file.title, "main.rs");
  state = openFileTab(state, workspace.id, "/project", "src/main.rs");
  assert.equal(fileTabs(state).length, 1);
  const position = { anchor: 25, head: 31, scrollTop: 700, scrollLeft: 16 };
  state = updateTab(state, file.id, (tab) => ({ ...tab, position }));
  const second = newWorkspace("/project", "local:bash", "Review");
  state.projects[0].workspaces.push(second);
  state = openFileTab(state, second.id, "/project", "src/main.rs");
  assert.equal(fileTabs(state).length, 2);
  assert.notEqual(fileTabs(state)[0].id, fileTabs(state)[1].id);
  const restored = restoreSession(JSON.parse(JSON.stringify(state)), info);
  assert.deepEqual(restored, state);
  assert.deepEqual(fileTabs(restored)[0].position, position);
  const terminal = workspace.tabs[0];
  assert.equal(terminal.type, "terminal");
  if (terminal.type !== "terminal") throw new Error("Expected a terminal");
  const pane = panes(terminal.layout)[0];
  const updated = updateDirectories(restored, { [pane.id]: "/project/src" });
  assert.deepEqual(fileTabs(updated), fileTabs(restored));
  const changedTerminal = updated.projects[0].workspaces[0].tabs[0];
  if (changedTerminal.type !== "terminal")
    throw new Error("Expected a terminal");
  assert.equal(panes(changedTerminal.layout)[0].cwd, "/project/src");
});

test("a fresh session waits for an explicit project selection", () => {
  const state = newSession();
  assert.deepEqual(state.projects, []);
  assert.equal(state.activeProjectId, null);
  assert.equal(active(state), undefined);
  assert.deepEqual(restoreSession(null, info), state);
});

test("restores an empty session without selecting the startup directory", () => {
  const state = { ...newSession(), sidebarWidth: 320, sidebar: null };
  assert.deepEqual(
    restoreSession(JSON.parse(JSON.stringify(state)), info),
    state,
  );
});

test("retains recent projects when none is selected", () => {
  const state = { ...projectSession(), activeProjectId: null };
  const restored = restoreSession(JSON.parse(JSON.stringify(state)), info);
  assert.deepEqual(restored, state);
  assert.equal(active(restored), undefined);
});

test("new projects contain a workspace, a tab and one terminal", () => {
  const state = projectSession();
  const { project, workspace, tab } = active(state)!;
  assert.equal(project.path, "/project");
  assert.equal(workspace.tabs.length, 1);
  assert.equal(panes(tab.layout).length, 1);
  assert.equal(panes(tab.layout)[0].cwd, "/project");
});

test("split layouts keep their ratios and collapse only the closed branch", () => {
  const left = newPane("/left");
  const right = newPane("/right");
  const bottom = newPane("/bottom");
  let layout = splitPane(left, left.id, "horizontal", right);
  layout = splitPane(layout, right.id, "vertical", bottom);
  layout = resizeSplit(layout, layout.id, 0.7);
  assert.equal((layout as Split).ratio, 0.7);
  assert.deepEqual(
    panes(layout).map((pane) => pane.cwd),
    ["/left", "/right", "/bottom"],
  );
  const collapsed = removePane(layout, right.id)!;
  assert.deepEqual(
    panes(collapsed).map((pane) => pane.cwd),
    ["/left", "/bottom"],
  );
  assert.equal((collapsed as Split).ratio, 0.7);
  assert.equal(removePane(left, left.id), null);
});

test("restores projects, active workspaces, tabs, environments and directories", () => {
  let state = projectSession();
  const { project, workspace, tab } = active(state)!;
  const secondary = newWorkspace("/project", "wsl:Ubuntu", "Review");
  state.projects[0] = {
    ...project,
    activeWorkspaceId: secondary.id,
    workspaces: [workspace, secondary],
  };
  state = updateDirectories(state, {
    [panes(tab.layout)[0].id]: "/project/src",
  });
  const restored = restoreSession(JSON.parse(JSON.stringify(state)), info);
  assert.deepEqual(restored, state);
  assert.equal(active(restored)!.workspace.name, "Review");
  assert.equal(active(restored)!.tab.profileId, "wsl:Ubuntu");
});

test("does not impose an artificial tab count limit", () => {
  let state = projectSession();
  const workspace = active(state)!.workspace;
  const tabs = Array.from({ length: 1200 }, (_, index) =>
    newTab("/project", "local:bash", `Tab ${index}`),
  );
  state = updateWorkspace(state, workspace.id, (workspace) => ({
    ...workspace,
    tabs,
    activeTabId: tabs.at(-1)!.id,
  }));
  assert.equal(
    active(restoreSession(state, info))!.workspace.tabs.length,
    1200,
  );
  assert.equal(active(restoreSession(state, info))!.tab.title, "Tab 1199");
});

test("repairs missing active IDs and duplicate IDs in saved data", () => {
  const state = projectSession();
  const { workspace, tab } = active(state)!;
  workspace.tabs.push(structuredClone(tab));
  state.activeProjectId = "missing";
  workspace.activeTabId = "missing";
  tab.activePaneId = "missing";
  const restored = active(restoreSession(state, info))!;
  assert.notEqual(restored.workspace.tabs[0].id, restored.workspace.tabs[1].id);
  assert.notEqual(
    panes(restored.workspace.tabs[0].layout)[0].id,
    panes(restored.workspace.tabs[1].layout)[0].id,
  );
  assert.equal(restored.workspace.activeTabId, restored.workspace.tabs[0].id);
  assert.equal(restored.tab.activePaneId, panes(restored.tab.layout)[0].id);
});

test("unchanged directory observations preserve state identity", () => {
  const state = projectSession();
  const pane = panes(active(state)!.tab.layout)[0];
  assert.equal(updateDirectories(state, { [pane.id]: pane.cwd }), state);
  const next = updateDirectories(state, { [pane.id]: "/project/src" });
  assert.equal(panes(active(next)!.tab.layout)[0].cwd, "/project/src");
  assert.equal(pane.cwd, "/project");
});

test("large terminal input preserves emoji at transport boundaries", () => {
  const input = "a".repeat(16_383) + "🦀" + "Zażółć 🧪".repeat(10_000);
  const chunks = [...inputChunks(input)];
  assert.equal(chunks.join(""), input);
  assert.ok(chunks.every((chunk) => chunk.isWellFormed()));
  assert.ok(chunks.every((chunk) => chunk.length <= 16_384));
});

test("file diff tabs restore and deduplicate each comparison without becoming editors or terminals", () => {
  let state = projectSession();
  const { workspace, tab: terminal } = active(state)!;
  state = openFileTab(state, workspace.id, "/project", "src/file.ts");
  state = openDiffTab(state, workspace.id, "/project", "src/file.ts", false);
  const working = active(state)!.tab;
  assert.equal(working.type, "diff");
  assert.equal("layout" in working, false);
  assert.equal(fileTabs(state).length, 1);
  assert.equal(
    canMergeTabs(working, terminal, "right", { width: 2000, height: 1200 }),
    false,
  );
  state = openDiffTab(state, workspace.id, "/project", "src/file.ts", true);
  const staged = active(state)!.tab;
  assert.notEqual(staged.id, working.id);
  state = openDiffTab(state, workspace.id, "/project", "src/file.ts", false);
  assert.equal(active(state)!.tab.id, working.id);
  assert.equal(active(state)!.workspace.tabs.length, 4);
  const restored = restoreSession(JSON.parse(JSON.stringify(state)), info);
  assert.deepEqual(restored, state);
});

test("commit tabs persist their repository and revision without acquiring terminal panes", () => {
  let state = projectSession();
  const { workspace, tab } = active(state)!;
  const pane = panes(tab.layout)[0];
  const commit = "a".repeat(40);
  state = openCommitTab(
    state,
    workspace.id,
    "/repository",
    commit,
    "aaaaaaa · Initial commit",
  );
  const opened = active(state)!.tab;
  assert.equal(opened.type, "commit");
  assert.equal("layout" in opened, false);
  state = updateDirectories(state, { [pane.id]: "/project/src" });
  assert.equal(active(state)!.tab, opened);
  const restored = restoreSession(JSON.parse(JSON.stringify(state)), info);
  assert.deepEqual(restored, state);
  assert.equal(active(restored)!.tab.type, "commit");
  assert.equal(active(restored)!.workspace.tabs.length, 2);
  assert.equal(
    panes(active(restored)!.workspace.tabs[0].layout)[0].cwd,
    "/project/src",
  );
});

test("opening a commit again selects its existing tab within the same workspace", () => {
  let state = projectSession();
  const { project, workspace } = active(state)!;
  const commit = "b".repeat(40);
  state = openCommitTab(
    state,
    workspace.id,
    "/project",
    commit,
    "bbbbbbb · Commit",
  );
  const first = active(state)!.tab;
  state = openCommitTab(
    state,
    workspace.id,
    "/project",
    "c".repeat(40),
    "Another commit",
  );
  state = openCommitTab(
    state,
    workspace.id,
    "/project",
    commit,
    "A different label",
  );
  assert.equal(active(state)!.tab.id, first.id);
  assert.equal(active(state)!.workspace.tabs.length, 3);
  const review = newWorkspace("/project", "local:bash", "Review");
  state.projects[0] = {
    ...state.projects[0],
    workspaces: [...state.projects[0].workspaces, review],
    activeWorkspaceId: review.id,
  };
  state = openCommitTab(state, review.id, "/project", commit, "Review commit");
  assert.notEqual(active(state)!.tab.id, first.id);
  assert.equal(active(state)!.workspace.tabs.length, 2);
  assert.equal(active(state)!.project.id, project.id);
});

test("restores terminal sessions saved before tab types were introduced", () => {
  const saved = JSON.parse(JSON.stringify(projectSession()));
  const terminal = saved.projects[0].workspaces[0].tabs[0];
  delete terminal.type;
  const restored = active(restoreSession(saved, info))!.tab;
  assert.equal(restored.type, "terminal");
  assert.deepEqual(restored.layout, terminal.layout);
  assert.equal(restored.activePaneId, terminal.activePaneId);
});

test("new workspaces share a folder while retaining independent tabs and restore together", () => {
  const first = addWorkspace(
    newSession(),
    "/work/simplevoice",
    "local:bash",
    "Voice 1",
  );
  const original = active(first)!;
  const second = addWorkspace(
    first,
    "/work/simplevoice",
    "local:bash",
    "Voice 2",
  );
  const third = addWorkspace(second, "/work/lomi", "local:bash", "Bench");
  assert.equal(first.projects[0].workspaces.length, 1);
  assert.equal(third.projects.length, 2);
  assert.equal(third.projects[0].workspaces.length, 2);
  assert.equal(third.projects[0].workspaces[0], original.workspace);
  assert.notEqual(active(second)!.tab.id, original.tab.id);
  assert.equal(active(third)!.workspace.name, "Bench");
  const terminals = third.projects.flatMap((project) =>
    project.workspaces.flatMap((workspace) => workspace.tabs),
  );
  const paneIds = terminals.flatMap((tab) =>
    tab.type === "terminal" ? panes(tab.layout).map((pane) => pane.id) : [],
  );
  assert.equal(new Set(paneIds).size, 3);
  assert.deepEqual(
    restoreSession(JSON.parse(JSON.stringify(third)), info),
    third,
  );
});

test("removing a workspace preserves other selections and permits an empty restored session", () => {
  let state = addWorkspace(newSession(), "/a", "bash", "One");
  const first = active(state)!.workspace;
  state = addWorkspace(state, "/a", "bash", "Two");
  const second = active(state)!.workspace;
  state = addWorkspace(state, "/b", "bash", "Other");
  const other = active(state)!;
  assert.equal(removeWorkspace(state, "missing"), state);
  const withoutInactive = removeWorkspace(state, first.id);
  assert.equal(active(withoutInactive)!.workspace, other.workspace);
  assert.equal(withoutInactive.projects[0].activeWorkspaceId, second.id);
  assert.equal(state.projects[0].workspaces.length, 2);
  const withoutOther = removeWorkspace(withoutInactive, other.workspace.id);
  assert.equal(withoutOther.projects.length, 1);
  assert.equal(active(withoutOther)!.workspace, second);
  const withoutSelected = removeWorkspace(
    { ...state, activeProjectId: state.projects[0].id },
    second.id,
  );
  assert.equal(active(withoutSelected)!.workspace, first);
  const empty = removeWorkspace(withoutOther, second.id);
  assert.deepEqual(empty.projects, []);
  assert.equal(empty.activeProjectId, null);
  assert.equal(active(empty), undefined);
  assert.deepEqual(
    restoreSession(JSON.parse(JSON.stringify(empty)), info),
    empty,
  );
});
