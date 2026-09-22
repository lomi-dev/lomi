import assert from "node:assert/strict";
import test from "node:test";
import {
  applyFileChange,
  containsPath,
  explorerGitStatuses,
  gitFilePath,
} from "../src/explorer-model.ts";
import {
  fileTabs,
  filesInTab,
  newProject,
  newSession,
  newWorkspace,
  openFileTab,
  openDiffTab,
  panes,
  splitPane,
} from "../src/model.ts";

test("Explorer matches repository paths and combines staged and working statuses", () => {
  const cases = [
    ["??", "?"],
    ["A ", "A"],
    ["AM", "A"],
    [" M", "M"],
    ["M ", "M"],
    ["AD", "D"],
    [" D", "D"],
    ["D ", "D"],
    ["R ", "R"],
    ["RM", "M"],
    ["C ", "C"],
    [" T", "T"],
    ["AA", "U"],
    ["DD", "U"],
    ["AU", "U"],
    ["UD", "U"],
    ["UA", "U"],
    ["DU", "U"],
    ["UU", "U"],
    ["  ", undefined],
    ["!!", undefined],
  ] as const;
  const statuses = explorerGitStatuses({
    root: "/repo/",
    branch: "main",
    changes: cases.map(([pair], i) => ({
      path: `project/src/file-${i}.ts`,
      originalPath: null,
      index: pair[0],
      worktree: pair[1],
    })),
  });
  cases.forEach(([, code], i) =>
    assert.equal(statuses.get(`/repo/project/src/file-${i}.ts`), code),
  );
  assert.equal(statuses.get("/repo/project-other/src/file-0.ts"), undefined);
  assert.equal(explorerGitStatuses(null).size, 0);
  assert.equal(
    gitFilePath("\\\\?\\C:\\repo\\src\\file.ts"),
    "C:/repo/src/file.ts",
  );
  assert.equal(
    gitFilePath("\\\\?\\UNC\\server\\repo\\file.ts"),
    "//server/repo/file.ts",
  );
});

test("Explorer propagates changes to ancestors with stable priority and respects repository boundaries", () => {
  const changes = [
    ["project/src/new.ts", "??", null],
    ["project/src/nested/changed.ts", " M", null],
    ["project/src/conflict.ts", "UU", null],
    ["project/new/deep/file.ts", "A ", null],
    ["project/deleted/file.ts", " D", null],
    ["project/moved/file.ts", "R ", "project/old/file.ts"],
    ["project/copied/file.ts", "C ", "project/unchanged/file.ts"],
    ["project/ignored/file.ts", "!!", null],
    ["project/type", " D", null],
    ["project/type/new.ts", "??", null],
  ].map(([path, pair, originalPath]) => ({
    path: path!,
    index: pair![0],
    worktree: pair![1],
    originalPath,
  }));
  for (const ordered of [changes, [...changes].reverse()]) {
    const statuses = explorerGitStatuses({
      root: "/repo",
      branch: "main",
      changes: ordered,
    });
    for (const [relative, code] of [
      ["", "U"],
      ["project", "U"],
      ["project/src", "U"],
      ["project/src/nested", "M"],
      ["project/new", "A"],
      ["project/new/deep", "A"],
      ["project/deleted", "M"],
      ["project/moved", "M"],
      ["project/old", "M"],
      ["project/copied", "M"],
      ["project/type", "A"],
      ["project/unchanged", undefined],
      ["project/ignored", undefined],
      ["project/sr", undefined],
    ])
      assert.equal(
        statuses.get(`/repo${relative ? `/${relative}` : ""}`),
        code,
      );
    assert.equal(statuses.has(""), false);
  }
  const modified = explorerGitStatuses({
    root: "/repo",
    branch: "main",
    changes: changes.filter((change) => change.worktree !== "U"),
  });
  assert.equal(modified.get("/repo/project/src"), "M");
  assert.equal(
    explorerGitStatuses({ root: "/repo", branch: "main", changes: [] }).size,
    0,
  );
});

test("folder renames preserve file IDs and positions in every workspace and split", () => {
  const project = newProject("/project", "bash");
  const second = newWorkspace("Other", "/project", "bash");
  project.workspaces.push(second);
  let session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const first = project.workspaces[0];
  session = openFileTab(session, first.id, "/project", "src/main.ts");
  session = openFileTab(session, second.id, "/project", "src/main.ts");
  const workspace = session.projects[0].workspaces[0];
  const file = filesInTab(workspace.tabs[1])[0];
  file.position = { head: 8, anchor: 3, scrollTop: 44, scrollLeft: 0 };
  const terminal = workspace.tabs[0];
  assert.equal(terminal.type, "terminal");
  if (terminal.type !== "terminal") return;
  const terminalId = panes(terminal.layout)[0].id;
  terminal.layout = splitPane(terminal.layout, terminalId, "horizontal", file);
  terminal.activePaneId = file.id;
  workspace.tabs.pop();
  const next = applyFileChange(
    session,
    { oldPath: "/project/src", newPath: "/project/code" },
    "bash",
  );
  assert.deepEqual(
    fileTabs(next).map((file) => [file.id, file.relative]),
    fileTabs(session).map((file) => [file.id, "code/main.ts"]),
  );
  assert.deepEqual(fileTabs(next)[0].position, file.position);
  assert.equal(next.projects[0].workspaces[0].tabs[0].type, "terminal");
  const after = next.projects[0].workspaces[0].tabs[0];
  if (after.type === "terminal")
    assert.equal(panes(after.layout)[0].id, terminalId);
  const deleted = applyFileChange(
    next,
    { oldPath: "/project/code", newPath: null },
    "bash",
  );
  assert.equal(fileTabs(deleted).length, 0);
  const kept = deleted.projects[0].workspaces[0].tabs[0];
  if (kept.type === "terminal") assert.equal(kept.activePaneId, terminalId);
});

test("project renames and cross-project moves retarget open files, while root deletion removes its project", () => {
  const a = newProject("/a", "bash");
  const b = newProject("/b", "bash");
  let session = { ...newSession(), projects: [a, b], activeProjectId: a.id };
  session = openFileTab(session, a.workspaces[0].id, "/a", "file.txt");
  const renamed = applyFileChange(
    session,
    { oldPath: "/a", newPath: "/renamed" },
    "bash",
  );
  assert.equal(renamed.projects[0].path, "/renamed");
  assert.equal(fileTabs(renamed)[0].root, "/renamed");
  const moved = applyFileChange(
    renamed,
    { oldPath: "/renamed/file.txt", newPath: "/b/moved.txt" },
    "bash",
  );
  assert.equal(fileTabs(moved)[0].root, "/b");
  assert.equal(fileTabs(moved)[0].relative, "moved.txt");
  const removed = applyFileChange(
    moved,
    { oldPath: "/renamed", newPath: null },
    "bash",
  );
  assert.equal(removed.projects.length, 1);
  assert.equal(removed.activeProjectId, b.id);
  assert.equal(containsPath("/src", "/src-other/file"), false);
  assert.equal(
    containsPath("C:\\project\\src", "C:\\project\\src\\file"),
    true,
  );
});

test("file diff tabs follow renames and retain deleted files for comparison", () => {
  const project = newProject("/project", "bash");
  let state = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  state = openDiffTab(
    state,
    project.workspaces[0].id,
    "/project",
    "src/file.ts",
    false,
  );
  state = applyFileChange(
    state,
    { oldPath: "/project/src", newPath: "/project/code" },
    "bash",
  );
  const tab = state.projects[0].workspaces[0].tabs[1];
  assert.equal(tab.type, "diff");
  if (tab.type !== "diff") throw new Error("Expected a file diff tab");
  assert.equal(tab.relative, "code/file.ts");
  state = applyFileChange(
    state,
    { oldPath: "/project/code/file.ts", newPath: null },
    "bash",
  );
  assert.equal(state.projects[0].workspaces[0].tabs[1].id, tab.id);
});

test("Explorer combines sibling repositories and prefers nested repository statuses", () => {
  const repositories = [
    {
      root: "/project",
      branch: "main",
      changes: [
        {
          path: "first/file.txt",
          originalPath: null,
          index: "?",
          worktree: "?",
        },
      ],
    },
    {
      root: "/project/first",
      branch: "main",
      changes: [
        { path: "file.txt", originalPath: null, index: " ", worktree: "M" },
      ],
    },
    {
      root: "/project/group/second",
      branch: "main",
      changes: [
        { path: "other.txt", originalPath: null, index: "U", worktree: "U" },
      ],
    },
  ];
  const statuses = explorerGitStatuses(repositories, "/project");
  assert.equal(statuses.get("/project/first/file.txt"), "M");
  assert.equal(statuses.get("/project/first"), "M");
  assert.equal(statuses.get("/project/group"), "U");
  assert.equal(statuses.get("/project"), "U");
});
