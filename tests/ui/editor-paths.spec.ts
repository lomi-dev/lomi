import { expect, test } from "@playwright/test";
import { newProject, newSession, openFileTab } from "../../src/model";
import { mockDesktop } from "./desktop";

test("moving a canonical parent preserves dirty editors under a nested restored alias", async ({
  page,
}) => {
  const logical = "/root/old/link";
  const canonical = "/root/old/real";
  const destination = "/root/new/real";
  const project = newProject("/root", "local:bash");
  let session = {
    ...newSession(),
    projects: [project, newProject(logical, "local:bash")],
    activeProjectId: project.id,
  };
  for (const root of [logical, canonical])
    session = openFileTab(session, project.workspaces[0].id, root, "file.txt");
  const workspace = session.projects[0].workspaces[0];
  workspace.tabs = workspace.tabs.filter((tab) => tab.type === "file");
  workspace.activeTabId = workspace.tabs[0].id;
  await mockDesktop(page, false, session, undefined, {
    [`${logical}/file.txt`]: {
      content: "original text",
      revision: "initial",
      encoding: "utf8",
      readOnly: false,
    },
  });
  await page.addInitScript(
    ({ logical, canonical, destination }) => {
      const bridge = (window as any).__TAURI_INTERNALS__;
      const native = (window as any).__nativeTest;
      const invoke = bridge.invoke;
      const files = native.editorFiles;
      files[`${canonical}/file.txt`] = files[`${logical}/file.txt`];
      let directory = "old";
      bridge.invoke = async (command: string, args: any, ...rest: any[]) => {
        if (command === "resolve_project_entry") {
          native.calls.push({ command, args });
          return `${args.root === logical ? canonical : args.root}${args.relative ? `/${args.relative}` : ""}`;
        }
        if (command === "list_directory") {
          native.calls.push({ command, args });
          return args.root === "/root" && !args.relative
            ? [
                {
                  name: directory,
                  relativePath: directory,
                  path: `/root/${directory}`,
                  isDirectory: true,
                  isSymlink: false,
                },
              ]
            : [];
        }
        if (command === "file_operation") {
          native.calls.push({ command, args });
          if (args.operation.kind !== "rename" || args.relative !== "old")
            throw Error("Unexpected mutation");
          files[`${destination}/file.txt`] = files[`${canonical}/file.txt`];
          delete files[`${logical}/file.txt`];
          delete files[`${canonical}/file.txt`];
          directory = "new";
          return { oldPath: "/root/old", newPath: "/root/new" };
        }
        if (command === "read_editor_file" || command === "save_editor_file") {
          const request = command === "read_editor_file" ? args : args.request;
          if (!files[`${request.root}/${request.relative}`])
            throw { kind: "io", message: "Stale logical alias" };
          const result = await invoke(command, args, ...rest);
          return command === "read_editor_file"
            ? {
                ...result,
                path: `${request.root === logical ? canonical : request.root}/${request.relative}`,
              }
            : result;
        }
        return invoke(command, args, ...rest);
      };
    },
    { logical, canonical, destination },
  );
  await page.goto("/");
  const content = page.locator(".cm-content:visible");
  await expect(content).toHaveText("original text");
  await content.fill("dirty nested alias żółć 🙂");
  await page
    .locator(".file-tree")
    .getByRole("button", { name: "old", exact: true })
    .click();
  await page.keyboard.press("F2");
  const rename = page.getByRole("textbox", {
    name: "Rename name",
    exact: true,
  });
  await rename.fill("new");
  await rename.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(() => {
        const state = JSON.parse(localStorage.getItem("test-session")!);
        if (!state) return null;
        return {
          project: state.projects[1].path,
          roots: state.projects[0].workspaces[0].tabs.map(
            (tab: any) => tab.root,
          ),
        };
      }),
    )
    .toEqual({ project: destination, roots: [destination, destination] });
  await expect(content).toHaveText("dirty nested alias żółć 🙂");
  await content.focus();
  await page.keyboard.press("Control+z");
  await expect(content).toHaveText("original text");
  await page.keyboard.press("Control+Shift+z");
  await expect(content).toHaveText("dirty nested alias żółć 🙂");
  await page.keyboard.press("Control+s");
  await expect
    .poll(() =>
      page.evaluate(
        ({ destination }) =>
          (window as any).__nativeTest.editorFiles[`${destination}/file.txt`]
            .content,
        { destination },
      ),
    )
    .toBe("dirty nested alias żółć 🙂");
  const shared = await page.evaluate(async () => {
    const runtimeUrl = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((url) => new URL(url).pathname === "/src/editor-runtime.ts")
      .at(-1);
    const runtime = await import(runtimeUrl ?? "/src/editor-runtime.ts");
    const tabs = JSON.parse(localStorage.getItem("test-session")!).projects[0]
      .workspaces[0].tabs;
    return {
      shared: runtime.findDocument(tabs[0]) === runtime.findDocument(tabs[1]),
      path: runtime.findDocument(tabs[0]).path,
      dirty: runtime.findDocument(tabs[0]).getSnapshot().dirty,
    };
  });
  expect(shared).toEqual({
    shared: true,
    path: `${destination}/file.txt`,
    dirty: false,
  });
});

test("Unix editor relocation preserves literal backslashes, dirty text and undo", async ({
  page,
}) => {
  const root = "/project\\original";
  const relative = "a\\b.txt";
  const project = newProject(root, "local:bash");
  const session = openFileTab(
    {
      ...newSession(),
      projects: [project],
      activeProjectId: project.id,
    },
    project.workspaces[0].id,
    root,
    relative,
  );
  await mockDesktop(page, false, session, undefined, {
    [`${root}/${relative}`]: {
      content: "original text",
      revision: "initial",
      encoding: "utf8",
      readOnly: false,
    },
  });
  await page.goto("/");
  const content = page.locator(".cm-content");
  await expect(content).toHaveText("original text");
  await content.fill("dirty Zażółć 🙂");
  const paths = await page.evaluate(
    async ({ root, relative }) => {
      const runtimeUrl = performance
        .getEntriesByType("resource")
        .map((entry) => entry.name)
        .filter((url) => new URL(url).pathname === "/src/editor-runtime.ts")
        .at(-1);
      const runtime = await import(runtimeUrl ?? "/src/editor-runtime.ts");
      const doc = runtime
        .documents()
        .find((item: any) => item.location.root === root)!;
      const before = {
        type: "file",
        id: "path-fixture",
        title: relative,
        root,
        relative,
      };
      const after = { ...before, root: "/project\\renamed" };
      runtime.relocateDocuments([before], [after]);
      return {
        path: doc.path,
        location: doc.location,
        dirty: doc.getSnapshot().dirty,
      };
    },
    { root, relative },
  );
  expect(paths).toEqual({
    path: "/project\\renamed/a\\b.txt",
    location: { root: "/project\\renamed", relative },
    dirty: true,
  });
  await expect(content).toHaveText("dirty Zażółć 🙂");
  await content.focus();
  await page.keyboard.press("Control+z");
  await expect(content).toHaveText("original text");
});

test("Windows shared editor relocation retains its canonical watcher path", async ({
  page,
}) => {
  const root = "C:\\project";
  const project = newProject(root, "local:powershell");
  const session = openFileTab(
    {
      ...newSession(),
      projects: [project],
      activeProjectId: project.id,
    },
    project.workspaces[0].id,
    root,
    "old.txt",
  );
  await mockDesktop(
    page,
    false,
    session,
    undefined,
    {
      [`${root}/old.txt`]: {
        content: "original",
        revision: "initial",
        encoding: "utf8",
        readOnly: false,
      },
    },
    "windows",
  );
  await page.addInitScript(() => {
    const native = (window as any).__TAURI_INTERNALS__;
    const original = native.invoke;
    native.invoke = async (command: string, args: any, ...rest: any[]) => {
      const result = await original(command, args, ...rest);
      if (command === "read_editor_file")
        return {
          ...result,
          path: `\\\\?\\${args.root}\\${args.relative.replaceAll("/", "\\")}`,
        };
      return result;
    };
  });
  await page.goto("/");
  const content = page.locator(".cm-content");
  await expect(content).toHaveText("original");
  await content.fill("dirty shared text");
  const relocated = await page.evaluate(async () => {
    const runtimeUrl = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((url) => new URL(url).pathname === "/src/editor-runtime.ts")
      .at(-1);
    const runtime = await import(runtimeUrl ?? "/src/editor-runtime.ts");
    const before = ["first-view", "second-view"].map((id) => ({
      type: "file",
      id,
      title: "old.txt",
      root: "C:\\project",
      relative: "old.txt",
    }));
    const after = before.map((file) => ({
      ...file,
      title: "new.txt",
      relative: "new.txt",
    }));
    const native = (window as any).__nativeTest;
    native.editorFiles["C:\\project/new.txt"] =
      native.editorFiles["C:\\project/old.txt"];
    delete native.editorFiles["C:\\project/old.txt"];
    runtime.relocateDocuments(before, after, {
      oldPath: "\\\\?\\C:\\project\\old.txt",
      newPath: "\\\\?\\C:\\project\\new.txt",
    });
    const first = runtime.findDocument(after[0]);
    const second = runtime.findDocument(after[1]);
    await first.save();
    return {
      path: first.path,
      shared: first === second,
      count: runtime.documents().length,
      dirty: first.getSnapshot().dirty,
    };
  });
  expect(relocated).toEqual({
    path: "\\\\?\\C:\\project\\new.txt",
    shared: true,
    count: 1,
    dirty: false,
  });
  await page.evaluate(async () => {
    const native = (window as any).__nativeTest;
    native.editorFiles["C:\\project/new.txt"].content = "external renamed text";
    native.editorFiles["C:\\project/new.txt"].revision = "external";
    await native.emitEvent("editor-files-changed", [
      "\\\\?\\C:\\project\\new.txt",
    ]);
  });
  await expect(content).toHaveText("external renamed text");
});

test("restored root aliases rename, save and watch shared dirty editors and protect deletion", async ({
  page,
}) => {
  const root = "/tmp/project";
  const canonical = "/private/tmp/project";
  const other = "/tmp/project-neighbor";
  const project = newProject(root, "local:bash");
  let session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  for (const [fileRoot, relative] of [
    [root, "old.txt"],
    [canonical, "old.txt"],
    [other, "unrelated.txt"],
  ]) {
    session = openFileTab(
      session,
      project.workspaces[0].id,
      fileRoot,
      relative,
    );
  }
  const workspace = session.projects[0].workspaces[0];
  workspace.tabs = workspace.tabs.filter((tab) => tab.type === "file");
  for (const [index, tab] of workspace.tabs.entries())
    tab.customTitle = ["Logical view", "Canonical view", "Unrelated view"][
      index
    ];
  workspace.activeTabId = workspace.tabs[0].id;
  const missing = newProject("/unavailable-project", "local:bash");
  session.projects.push(missing);
  await mockDesktop(page, false, session, undefined, {
    [`${root}/old.txt`]: {
      content: "original text",
      revision: "initial",
      encoding: "utf8",
      readOnly: false,
    },
    [`${other}/unrelated.txt`]: {
      content: "other text",
      revision: "initial",
      encoding: "utf8",
      readOnly: false,
    },
  });
  await page.addInitScript(
    ({ root, canonical, other }) => {
      const bridge = (window as any).__TAURI_INTERNALS__;
      const native = (window as any).__nativeTest;
      const invoke = bridge.invoke;
      const canonicalRoot = (value: string) =>
        value === root
          ? canonical
          : value === other
            ? "/private/tmp/project-neighbor"
            : value;
      const files = native.editorFiles;
      files[`${canonical}/old.txt`] = files[`${root}/old.txt`];
      let filename = "old.txt";
      bridge.invoke = async (command: string, args: any, ...rest: any[]) => {
        if (command === "resolve_project_entry") {
          native.calls.push({ command, args });
          if (args.root === "/unavailable-project")
            throw Error("Unrelated project is offline");
          return `${canonicalRoot(args.root)}${args.relative ? `/${args.relative}` : ""}`;
        }
        if (command === "list_directory") {
          native.calls.push({ command, args });
          return [
            {
              name: filename,
              relativePath: filename,
              path: `${canonical}/${filename}`,
              isDirectory: false,
              isSymlink: false,
            },
          ];
        }
        if (command === "file_operation") {
          native.calls.push({ command, args });
          if (args.operation.kind !== "rename")
            throw Error("Deletion must remain cancelled");
          const oldPath = `${canonical}/${filename}`;
          const file = files[`${root}/${filename}`];
          delete files[`${root}/${filename}`];
          delete files[`${canonical}/${filename}`];
          filename = args.operation.name;
          files[`${root}/${filename}`] = file;
          files[`${canonical}/${filename}`] = file;
          return { oldPath, newPath: `${canonical}/${filename}` };
        }
        if (command === "read_editor_file" || command === "save_editor_file") {
          const request = command === "read_editor_file" ? args : args.request;
          if (!files[`${request.root}/${request.relative}`])
            throw {
              kind: "io",
              message: "Missing file: stale logical location",
            };
          const result = await invoke(command, args, ...rest);
          if (command === "read_editor_file")
            return {
              ...result,
              path: `${canonicalRoot(request.root)}/${request.relative}`,
            };
          await native.emitEvent("editor-files-changed", [
            `${canonicalRoot(request.root)}/${request.relative}`,
          ]);
          return result;
        }
        return invoke(command, args, ...rest);
      };
    },
    { root, canonical, other },
  );
  await page.goto("/");
  const content = page.locator(".cm-content:visible");
  await expect(content).toHaveText("original text");
  await page.getByRole("tab", { name: "Unrelated view", exact: true }).click();
  await expect(content).toHaveText("other text");
  await content.fill("unrelated draft");
  await page
    .getByRole("tab", { name: /^Logical view(?: Unsaved changes)?$/ })
    .click();
  await content.fill("dirty shared żółć 🙂");
  await page
    .getByRole("tab", { name: /^Canonical view(?: Unsaved changes)?$/ })
    .click();
  await expect(content).toHaveText("dirty shared żółć 🙂");
  const entry = page
    .locator(".file-tree")
    .getByRole("button", { name: "old.txt", exact: true });
  await entry.click();
  await page.keyboard.press("F2");
  const rename = page.getByRole("textbox", {
    name: "Rename name",
    exact: true,
  });
  await rename.fill("renamed.txt");
  await rename.press("Enter");
  await expect(
    page
      .locator(".file-tree")
      .getByRole("button", { name: "renamed.txt", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const state = JSON.parse(localStorage.getItem("test-session")!);
        return state?.projects[0].workspaces[0].tabs
          .slice(0, 2)
          .map((tab: any) => ({ root: tab.root, relative: tab.relative }));
      }),
    )
    .toEqual([
      { root, relative: "renamed.txt" },
      { root: canonical, relative: "renamed.txt" },
    ]);
  await page
    .getByRole("tab", { name: /^Logical view(?: Unsaved changes)?$/ })
    .click();
  await expect(content).toHaveText("dirty shared żółć 🙂");
  await content.focus();
  await page.keyboard.press("Control+z");
  await expect(content).toHaveText("original text");
  await page.keyboard.press("Control+Shift+z");
  await expect(content).toHaveText("dirty shared żółć 🙂");
  await page.keyboard.press("Control+s");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "save_editor_file")
            .at(-1)?.args.request.relative,
      ),
    )
    .toBe("renamed.txt");
  await expect
    .poll(() =>
      page.evaluate(
        ({ root }) =>
          (window as any).__nativeTest.editorFiles[`${root}/renamed.txt`]
            .content,
        { root },
      ),
    )
    .toBe("dirty shared żółć 🙂");
  const shared = await page.evaluate(async () => {
    const url = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((url) => new URL(url).pathname === "/src/editor-runtime.ts")
      .at(-1);
    const runtime = await import(url ?? "/src/editor-runtime.ts");
    const tabs = JSON.parse(localStorage.getItem("test-session")!).projects[0]
      .workspaces[0].tabs;
    return {
      shared: runtime.findDocument(tabs[0]) === runtime.findDocument(tabs[1]),
      path: runtime.findDocument(tabs[0]).path,
    };
  });
  expect(shared).toEqual({ shared: true, path: `${canonical}/renamed.txt` });
  await page.evaluate(
    async ({ root, canonical }) => {
      const native = (window as any).__nativeTest;
      native.editorFiles[`${root}/renamed.txt`].content =
        "external canonical watch";
      native.editorFiles[`${root}/renamed.txt`].revision = "external";
      await native.emitEvent("editor-files-changed", [
        `${canonical}/renamed.txt`,
      ]);
    },
    { root, canonical },
  );
  await expect(content).toHaveText("external canonical watch");
  await content.fill("keep dirty before delete");
  await page
    .locator(".file-tree")
    .getByRole("button", { name: "renamed.txt", exact: true })
    .click();
  await page.keyboard.press("Control+Delete");
  const deletion = page.getByRole("dialog", {
    name: "Delete Permanently",
    exact: true,
  });
  await deletion
    .getByRole("button", { name: "Delete Permanently", exact: true })
    .click();
  const guard = page.getByRole("dialog", {
    name: "Save changes before closing?",
    exact: true,
  });
  await expect(guard).toContainText("renamed.txt");
  await expect(guard).not.toContainText("unrelated.txt");
  await guard.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(guard).toHaveCount(0);
  if (await deletion.isVisible())
    await deletion.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls
        .filter((call: any) => call.command === "file_operation")
        .map((call: any) => call.args.operation.kind),
    ),
  ).toEqual(["rename"]);
  await page
    .getByRole("tab", { name: /^Logical view(?: Unsaved changes)?$/ })
    .click();
  await expect(content).toHaveText("keep dirty before delete");
});

test("renaming a directory symlink preserves shared canonical editor identity and watcher events", async ({
  page,
}) => {
  const project = newProject("/a", "local:bash");
  let session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  session = openFileTab(session, project.activeWorkspaceId, "/a", "b/file.txt");
  session = openFileTab(
    session,
    project.activeWorkspaceId,
    "/realB",
    "file.txt",
  );
  const workspace = session.projects[0].workspaces[0];
  workspace.tabs = workspace.tabs.filter((tab) => tab.type === "file");
  workspace.activeTabId = workspace.tabs[0].id;
  const logicalTab = page.locator(
    `[data-tab-id="${workspace.tabs[0].id}"] [role="tab"]`,
  );
  const canonicalTab = page.locator(
    `[data-tab-id="${workspace.tabs[1].id}"] [role="tab"]`,
  );
  await mockDesktop(page, false, session, undefined, {
    "/a/b/file.txt": {
      content: "original symlink target",
      revision: "initial",
      encoding: "utf8",
      readOnly: false,
    },
  });
  await page.addInitScript(() => {
    const bridge = (window as any).__TAURI_INTERNALS__;
    const native = (window as any).__nativeTest;
    const invoke = bridge.invoke;
    const files = native.editorFiles;
    files["/realB/file.txt"] = files["/a/b/file.txt"];
    let directory = "b";
    bridge.invoke = async (command: string, args: any, ...rest: any[]) => {
      if (command === "resolve_project_entry") {
        native.calls.push({ command, args });
        return args.root === "/a"
          ? `/realA${args.relative ? `/${args.relative}` : ""}`
          : args.root;
      }
      if (command === "list_directory") {
        native.calls.push({ command, args });
        return args.relative
          ? [
              {
                name: "file.txt",
                relativePath: `${directory}/file.txt`,
                path: "/realB/file.txt",
                isDirectory: false,
                isSymlink: false,
              },
            ]
          : [
              {
                name: directory,
                relativePath: directory,
                path: `/realA/${directory}`,
                isDirectory: true,
                isSymlink: true,
              },
            ];
      }
      if (command === "file_operation") {
        native.calls.push({ command, args });
        const oldPath = `/realA/${directory}`;
        const file = files[`/a/${directory}/file.txt`];
        delete files[`/a/${directory}/file.txt`];
        directory = args.operation.name;
        files[`/a/${directory}/file.txt`] = file;
        return { oldPath, newPath: `/realA/${directory}` };
      }
      if (command === "read_editor_file" || command === "save_editor_file") {
        const request = command === "read_editor_file" ? args : args.request;
        if (!files[`${request.root}/${request.relative}`])
          throw { kind: "io", message: "The old symlink entry is gone" };
        const result = await invoke(command, args, ...rest);
        if (command === "read_editor_file")
          return { ...result, path: "/realB/file.txt" };
        await native.emitEvent("editor-files-changed", ["/realB/file.txt"]);
        return result;
      }
      return invoke(command, args, ...rest);
    };
  });
  await page.goto("/");
  const content = page.locator(".cm-content:visible");
  await expect(content).toHaveText("original symlink target");
  await content.fill("dirty symlink target");
  await canonicalTab.click();
  await expect(content).toHaveText("dirty symlink target");
  await page.locator('.file-tree .tree-entry[title="/realA/b"]').click();
  await page.keyboard.press("F2");
  const rename = page.getByRole("textbox", {
    name: "Rename name",
    exact: true,
  });
  await rename.fill("c");
  await rename.press("Enter");
  await expect(
    page.locator('.file-tree .tree-entry[title="/realA/c"]'),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")
            ?.projects[0].workspaces[0].tabs[0].relative,
      ),
    )
    .toBe("c/file.txt");
  await logicalTab.click();
  await expect(content).toHaveText("dirty symlink target");
  await content.focus();
  await page.keyboard.press("Control+s");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "save_editor_file")
            .at(-1)?.args.request,
      ),
    )
    .toMatchObject({
      root: "/a",
      relative: "c/file.txt",
      content: "dirty symlink target",
    });
  const identity = await page.evaluate(async () => {
    const url = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((url) => new URL(url).pathname === "/src/editor-runtime.ts")
      .at(-1);
    const runtime = await import(url ?? "/src/editor-runtime.ts");
    const tabs = JSON.parse(localStorage.getItem("test-session")!).projects[0]
      .workspaces[0].tabs;
    const first = runtime.findDocument(tabs[0]);
    return {
      shared: first === runtime.findDocument(tabs[1]),
      path: first.path,
      documents: runtime.documents().length,
    };
  });
  expect(identity).toEqual({
    shared: true,
    path: "/realB/file.txt",
    documents: 1,
  });
  await page.evaluate(async () => {
    const native = (window as any).__nativeTest;
    native.editorFiles["/realB/file.txt"].content =
      "unchanged canonical watcher target";
    native.editorFiles["/realB/file.txt"].revision = "external";
    await native.emitEvent("editor-files-changed", ["/realB/file.txt"]);
  });
  await expect(content).toHaveText("unchanged canonical watcher target");
});
