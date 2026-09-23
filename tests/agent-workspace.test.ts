import assert from "node:assert/strict";
import { test } from "node:test";
import {
  closeAgentProject,
  closeAgentWorkspace,
  sameAgentSession,
} from "../src/agent-workspace.ts";
import {
  newBrowserTab,
  newFileTab,
  newProject,
  newSession,
  newWorkspace,
} from "../src/model.ts";

test("closure preserves concurrent unrelated changes and checks every target resource", () => {
  const project = newProject("/project", "shell");
  const workspace = project.workspaces[0];
  const file = newFileTab(newSession());
  const browser = newBrowserTab("https://example.test");
  browser.automation = { generation: "same-page", profileId: "private" };
  workspace.tabs.push(file, browser);
  const other = newWorkspace("/project", "lazy");
  project.workspaces.push(other);
  const before = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const current = structuredClone(before);
  current.sidebarWidth += 40;
  current.projects[0].activeWorkspaceId = other.id;
  current.projects[0].workspaces[1].name = "Concurrent user rename";
  current.projects[0].workspaces[0].tabs[0].title = "Late shell title";
  const selectedFile = current.projects[0].workspaces[0].tabs[1];
  assert.equal(selectedFile.type, "file");
  selectedFile.position = { anchor: 5, head: 5, scrollTop: 10, scrollLeft: 0 };
  const closed = closeAgentWorkspace(before, current, project.id, workspace.id);
  assert.equal(closed.sidebarWidth, current.sidebarWidth);
  assert.equal(closed.projects[0].workspaces[0].name, "Concurrent user rename");
  assert.strictEqual(
    closed.projects[0].workspaces[0],
    current.projects[0].workspaces[1],
  );
  assert.equal(closed.activeProjectId, project.id);
  assert.equal(
    closeAgentWorkspace(before, before, project.id, workspace.id)
      .activeProjectId,
    null,
  );
  assert.equal(current.projects[0].workspaces.length, 2);
  for (const mutate of [
    (s: typeof current) => {
      s.projects[0].path = "/different";
    },
    (s: typeof current) => {
      s.projects[0].workspaces[0].tabs.push(newFileTab(s));
    },
    (s: typeof current) => {
      s.projects[0].workspaces[0].tabs =
        s.projects[0].workspaces[0].tabs.slice(1);
    },
    (s: typeof current) => {
      const t = s.projects[0].workspaces[0].tabs[1];
      if (t.type === "file") t.relative = "different.txt";
    },
    (s: typeof current) => {
      const t = s.projects[0].workspaces[0].tabs[2];
      if (t.type === "browser") t.automation!.generation = "replacement";
    },
    (s: typeof current) => {
      s.projects[0].workspaces[0].name = "Changed approval target";
    },
  ]) {
    const changed = structuredClone(current);
    mutate(changed);
    assert.throws(
      () => closeAgentWorkspace(before, changed, project.id, workspace.id),
      /REVISION_CONFLICT/,
    );
  }
});

test("claim validation accepts only equal domain values despite new object references", () => {
  const before = {
    ...newSession(),
    projects: [newProject("/project", "shell")],
  };
  const cloned = structuredClone(before);
  assert.equal(sameAgentSession(before, cloned), true);
  cloned.projects[0].workspaces[0].tabs[0].title = "Changed";
  assert.equal(sameAgentSession(before, cloned), false);
  assert.equal(sameAgentSession(before, undefined), false);
});

test("editor viewport persistence does not change domain identity, including mixed layouts", () => {
  const project = newProject("/project", "shell");
  const before = { ...newSession(), projects: [project] };
  const tab = project.workspaces[0].tabs[0];
  assert.equal(tab.type, "terminal");
  const embedded = newFileTab(before);
  tab.layout = {
    type: "split",
    id: "mixed",
    axis: "horizontal",
    ratio: 0.5,
    first: tab.layout,
    second: embedded,
  };
  const standalone = newFileTab(before);
  project.workspaces[0].tabs.push(standalone);
  const after = structuredClone(before);
  const moved = after.projects[0].workspaces[0].tabs[0];
  assert.equal(moved.type, "terminal");
  assert.equal(moved.layout.type, "split");
  assert.equal(moved.layout.second.type, "file");
  moved.layout.second.position = {
    anchor: 20,
    head: 21,
    scrollTop: 800,
    scrollLeft: 5,
  };
  const file = after.projects[0].workspaces[0].tabs[1];
  assert.equal(file.type, "file");
  file.position = { anchor: 0, head: 0, scrollTop: 0, scrollLeft: 0 };
  assert.equal(sameAgentSession(before, after), true);
  file.relative = "changed-resource.txt";
  assert.equal(sameAgentSession(before, after), false);
  file.relative = standalone.relative;
  moved.activePaneId = embedded.id;
  assert.equal(sameAgentSession(before, after), false);
});

test("project closure retains other projects and refuses added or changed target workspaces", () => {
  const project = newProject("/project", "default");
  project.workspaces.push(newWorkspace("/project", "second"));
  const retained = newProject("/retained", "lazy");
  const before = {
    ...newSession(),
    projects: [project, retained],
    activeProjectId: project.id,
  };
  const current = structuredClone(before);
  current.projects[1].workspaces[0].name = "Concurrent unrelated change";
  const closed = closeAgentProject(before, current, project.id);
  assert.equal(closed.activeProjectId, null);
  assert.deepEqual(closed.projects, [current.projects[1]]);
  assert.strictEqual(closed.projects[0], current.projects[1]);
  current.activeProjectId = retained.id;
  assert.equal(
    closeAgentProject(before, current, project.id).activeProjectId,
    retained.id,
  );
  const added = structuredClone(current);
  added.projects[0].workspaces.push(newWorkspace("/project", "new"));
  assert.throws(
    () => closeAgentProject(before, added, project.id),
    /REVISION_CONFLICT/,
  );
  const changed = structuredClone(current);
  changed.projects[0].workspaces[1].tabs.push(newFileTab(changed));
  assert.throws(
    () => closeAgentProject(before, changed, project.id),
    /REVISION_CONFLICT/,
  );
  assert.equal(current.projects.length, 2);
});
