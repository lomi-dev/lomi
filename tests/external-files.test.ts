import assert from "node:assert/strict";
import { test } from "node:test";
import {
  active,
  addWorkspace,
  fileTabs,
  newProject,
  newSession,
  openFileTab,
} from "../src/model.ts";
import { openExternalFile } from "../src/external-files.ts";

function sessionFor(path: string) {
  const project = newProject(path, "local:bash");
  return {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
}

test("external files create a file-only project when their root is new", () => {
  const initial = newSession();
  const next = openExternalFile(initial, {
    root: "/work/new-folder",
    relative: "notes/launch",
  });

  assert.equal(next.projects.length, 1);
  assert.equal(next.projects[0].path, "/work/new-folder");
  assert.equal(next.projects[0].workspaces.length, 1);
  assert.equal(next.projects[0].workspaces[0].tabs.length, 1);
  assert.equal(next.projects[0].workspaces[0].tabs[0].type, "file");
  assert.equal(active(next)?.tab.type, "file");
  assert.deepEqual(
    fileTabs(next).map(({ root, relative }) => [root, relative]),
    [["/work/new-folder", "notes/launch"]],
  );
});

test("external files focus the file in the native-selected project root", () => {
  const outer = newProject("/work", "local:bash");
  const nested = newProject("/work/nested", "local:bash");
  const initial = {
    ...newSession(),
    projects: [outer, nested],
    activeProjectId: outer.id,
  };

  const next = openExternalFile(initial, {
    root: "/work/nested",
    relative: "README.md",
  });

  assert.equal(next.activeProjectId, nested.id);
  assert.equal(active(next)?.project.path, "/work/nested");
  assert.equal(fileTabs(next).length, 1);
  assert.equal(fileTabs(next)[0].root, "/work/nested");
});

test("external files reuse their descriptor and retain editor metadata", () => {
  const initial = sessionFor("/project");
  const workspace = initial.projects[0].workspaces[0];
  let withFile = openFileTab(initial, workspace.id, "/project", "draft.txt");
  const opened = fileTabs(withFile)[0];
  withFile = {
    ...withFile,
    projects: withFile.projects.map((project) => ({
      ...project,
      workspaces: project.workspaces.map((candidate) => ({
        ...candidate,
        tabs: candidate.tabs.map((tab) =>
          tab.id === opened.id && tab.type === "file"
            ? {
                ...tab,
                customTitle: "Working draft",
                position: { anchor: 8, head: 8, scrollTop: 31, scrollLeft: 4 },
                previewView: "editor",
              }
            : tab,
        ),
      })),
    })),
  };
  const before = fileTabs(withFile)[0];

  const next = openExternalFile(withFile, {
    root: "/project",
    relative: "draft.txt",
  });

  assert.equal(fileTabs(next).length, 1);
  assert.deepEqual(fileTabs(next)[0], before);
  assert.equal(active(next)?.tab.id, opened.id);
});

test("Windows file identity normalizes separators without changing case", () => {
  const initial = sessionFor("C:\\Project");
  const workspace = initial.projects[0].workspaces[0];
  const withFile = openFileTab(
    initial,
    workspace.id,
    "C:\\Project",
    "src\\Main.ts",
  );
  const existing = fileTabs(withFile)[0];

  const next = openExternalFile(withFile, {
    root: "C:\\Project",
    relative: "src/Main.ts",
  });

  assert.equal(fileTabs(next).length, 1);
  assert.deepEqual(fileTabs(next)[0], existing);
  assert.equal(active(next)?.tab.id, existing.id);
});

test("Windows case-sensitive paths remain distinct", () => {
  const initial = sessionFor("C:\\Project");
  const workspace = initial.projects[0].workspaces[0];
  const withFile = openFileTab(
    initial,
    workspace.id,
    "C:\\Project",
    "src\\Main.ts",
  );

  const next = openExternalFile(withFile, {
    root: "C:\\Project",
    relative: "src/main.ts",
  });

  assert.equal(fileTabs(next).length, 2);
  assert.equal(active(next)?.tab.type, "file");
  if (active(next)?.tab.type !== "file") throw Error("Expected a file tab");
  assert.equal(active(next).tab.relative, "src/main.ts");
});

test("external files reuse tabs opened under a nested Git root", () => {
  const initial = sessionFor("/project");
  const workspace = initial.projects[0].workspaces[0];
  const withFile = openFileTab(
    initial,
    workspace.id,
    "/project/packages/service",
    "src/main.ts",
  );
  const existing = fileTabs(withFile)[0];

  const next = openExternalFile(withFile, {
    root: "/project",
    relative: "packages/service/src/main.ts",
  });

  assert.equal(fileTabs(next).length, 1);
  assert.deepEqual(fileTabs(next)[0], existing);
  assert.equal(active(next)?.tab.id, existing.id);
});

test("external files prefer an existing tab in another workspace", () => {
  let initial = sessionFor("/project");
  const project = initial.projects[0];
  const firstWorkspace = project.workspaces[0];
  initial = addWorkspace(initial, "/project", "local:bash", "Second");
  const updatedProject = initial.projects[0];
  const secondWorkspace = updatedProject.workspaces.find(
    (workspace) => workspace.id !== firstWorkspace.id,
  )!;
  initial = openFileTab(initial, secondWorkspace.id, "/project", "src/main.ts");
  initial = {
    ...initial,
    projects: initial.projects.map((candidate) =>
      candidate.id === project.id
        ? { ...candidate, activeWorkspaceId: firstWorkspace.id }
        : candidate,
    ),
  };
  const existing = fileTabs(initial)[0];

  const next = openExternalFile(initial, {
    root: "/project",
    relative: "src/main.ts",
  });

  const selected = active(next)!;
  assert.equal(selected.workspace.id, secondWorkspace.id);
  assert.equal(selected.tab.id, existing.id);
  assert.equal(fileTabs(next).length, 1);
});
