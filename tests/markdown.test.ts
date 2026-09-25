import assert from "node:assert/strict";
import { test } from "node:test";
import { isMarkdownFile, markdownTarget } from "../src/markdown.ts";
import {
  active,
  fileTabs,
  mergeTabs,
  newProject,
  newSession,
  openFileTab,
  restoreSession,
  updateFilePosition,
  updateFilePreviewRatio,
  updateFilePreviewView,
} from "../src/model.ts";

test("Markdown links resolve relative to the document and reject executable URLs", () => {
  assert.equal(isMarkdownFile("docs/README.Md"), true);
  assert.equal(isMarkdownFile("notes.markdown"), true);
  assert.equal(isMarkdownFile("README.md.txt"), false);
  assert.deepEqual(markdownTarget("../images/a%20b.svg", "docs/README.md"), {
    kind: "file",
    value: "images/a b.svg",
  });
  assert.deepEqual(markdownTarget("/README.md", "docs/start.md"), {
    kind: "file",
    value: "README.md",
  });
  assert.deepEqual(markdownTarget("#za%C5%BC%C3%B3%C5%82%C4%87", "README.md"), {
    kind: "fragment",
    value: "zażółć",
  });
  assert.deepEqual(markdownTarget("https://example.com/docs", "README.md"), {
    kind: "external",
    value: "https://example.com/docs",
  });
  for (const url of [
    "javascript:alert(1)",
    "data:text/html,test",
    "file:///etc/passwd",
    "//example.com/image.png",
    "\\\\example.com\\secret",
    "image%00.png",
    "%5csecret",
    "%not-encoded",
    "java\nscript:alert(1)",
  ])
    assert.equal(markdownTarget(url, "README.md"), null, url);
});

test("Markdown view modes retain file identities and positions in tabs and split layouts", () => {
  const project = newProject("/project", "bash");
  let state = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const workspace = project.workspaces[0];
  const terminal = workspace.tabs[0];
  state = openFileTab(state, workspace.id, project.path, "README.md");
  const file = fileTabs(state)[0];
  const position = { anchor: 12, head: 24, scrollTop: 140, scrollLeft: 10 };
  state = updateFilePosition(state, file.id, position);
  state = updateFilePreviewRatio(state, file.id, 0.72);
  state = updateFilePreviewView(state, file.id, "preview");
  assert.deepEqual(fileTabs(state)[0], {
    ...file,
    position,
    previewRatio: 0.72,
    previewView: "preview",
  });
  state.projects[0].workspaces[0] = mergeTabs(
    active(state)!.workspace,
    file.id,
    terminal.id,
    "right",
    { width: 1000, height: 700 },
  );
  state = updateFilePreviewView(state, file.id, "split");
  const info = {
    directory: "/project",
    home: "/home/test",
    platform: "linux",
    profiles: [],
  };
  const restored = restoreSession(JSON.parse(JSON.stringify(state)), info);
  assert.equal(fileTabs(restored)[0].previewView, "split");
  assert.equal(fileTabs(restored)[0].previewRatio, 0.72);
  assert.deepEqual(fileTabs(restored)[0].position, position);
  assert.equal(fileTabs(restored)[0].id, file.id);
  const legacy = JSON.parse(JSON.stringify(restored));
  const legacyFile = legacy.projects[0].workspaces[0].tabs[0].layout.second;
  legacyFile.markdownView = legacyFile.previewView;
  delete legacyFile.previewView;
  assert.equal(fileTabs(restoreSession(legacy, info))[0].previewView, "split");
  assert.equal(
    fileTabs(updateFilePreviewView(restored, file.id, "editor"))[0].previewView,
    undefined,
  );
  const invalid = JSON.parse(JSON.stringify(restored));
  invalid.projects[0].workspaces[0].tabs[0].layout.second.previewView =
    "unknown";
  assert.equal(
    fileTabs(restoreSession(invalid, info))[0].previewView,
    undefined,
  );
});

test("file preview ratios stay bounded and restore only finite values", () => {
  const project = newProject("/project", "bash");
  let state = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const workspace = project.workspaces[0];
  state = openFileTab(state, workspace.id, project.path, "README.md");
  const file = fileTabs(state)[0];

  state = updateFilePreviewRatio(state, file.id, 0.72);
  state = updateFilePreviewView(state, file.id, "split");
  const info = {
    directory: "/project",
    home: "/home/test",
    platform: "linux",
    profiles: [],
  };
  const restored = restoreSession(JSON.parse(JSON.stringify(state)), info);
  assert.equal(fileTabs(restored)[0].previewRatio, 0.72);
  assert.equal(
    fileTabs(updateFilePreviewRatio(state, file.id, -1))[0].previewRatio,
    0.1,
  );
  assert.equal(
    fileTabs(updateFilePreviewRatio(state, file.id, 2))[0].previewRatio,
    0.9,
  );
  assert.equal(
    fileTabs(updateFilePreviewRatio(state, file.id, Number.NaN))[0]
      .previewRatio,
    0.72,
  );

  const invalid = JSON.parse(JSON.stringify(state));
  invalid.projects[0].workspaces[0].tabs.find(
    (tab: { id: string }) => tab.id === file.id,
  ).previewRatio = "0.6";
  assert.equal(
    fileTabs(restoreSession(invalid, info))[0].previewRatio,
    undefined,
  );
  const outOfRange = JSON.parse(JSON.stringify(state));
  outOfRange.projects[0].workspaces[0].tabs.find(
    (tab: { id: string }) => tab.id === file.id,
  ).previewRatio = 4;
  assert.equal(fileTabs(restoreSession(outOfRange, info))[0].previewRatio, 0.9);
});
