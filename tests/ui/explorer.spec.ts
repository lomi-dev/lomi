import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { newProject, newSession } from "../../src/model";
import { mockDesktop } from "./desktop";

async function setup(
  page: Page,
  git = true,
  initialEntries = [
    { relative: "src", directory: true },
    { relative: "src/main.ts", directory: false },
    { relative: "README.md", directory: false },
  ],
  explorerOpenOnSingleClick = false,
  platform: "linux" | "macos" | "windows" = "linux",
  saved?: unknown,
) {
  await mockDesktop(
    page,
    git,
    saved,
    undefined,
    {
      "/project/src/main.ts": {
        content: "first\n🦀 needle here\nneedle again\n",
        revision: "initial",
        encoding: "utf8",
        readOnly: false,
      },
      "/project/README.md": {
        content: "# Project\nneedle in readme\n",
        revision: "initial",
        encoding: "utf8",
        readOnly: false,
      },
    },
    platform,
  );
  await page.addInitScript((enabled) => {
    if (enabled)
      localStorage.setItem(
        "test-keybindings",
        JSON.stringify({
          version: 1,
          bindings: {},
          explorerOpenOnSingleClick: true,
        }),
      );
  }, explorerOpenOnSingleClick);
  await page.addInitScript((initialEntries) => {
    const native = (window as any).__nativeTest;
    native.operationError = "";
    native.externalEntries = [];
    native.operationDelay = 0;
    native.searchDelays = {};
    native.directoryDelay = 0;
    native.directoryReads = 0;
    native.maxDirectoryReads = 0;
    native.explorerWatchers = new Map();
    let watcherId = 1000;
    let entries = initialEntries;
    native.setExplorerEntries = (next: typeof initialEntries) => {
      entries = next;
    };
    native.changeExplorerDirectories = (relatives: string[]) => {
      for (const watcher of native.explorerWatchers.values())
        watcher.onChange.onmessage(relatives);
    };
    const bridge = (window as any).__TAURI_INTERNALS__;
    const invoke = bridge.invoke;
    bridge.invoke = async (command: string, args: any = {}) => {
      if (command === "watch_explorer_directories") {
        native.calls.push({ command, args });
        const id = watcherId++;
        native.explorerWatchers.set(id, args);
        return id;
      }
      if (
        command === "plugin:resources|close" &&
        native.explorerWatchers.delete(args.rid)
      )
        return;
      if (
        ![
          "list_directory",
          "resolve_project_entry",
          "file_operation",
          "search_project",
          "cancel_project_search",
          "open_project_item",
          "ignore_project_item",
        ].includes(command)
      )
        return invoke(command, args);
      native.calls.push({ command, args });
      if (command === "resolve_project_entry")
        return `${args.root}/${args.relative}`.replace(/\/$/, "");
      if (command === "list_directory") {
        native.maxDirectoryReads = Math.max(
          native.maxDirectoryReads,
          ++native.directoryReads,
        );
        const result = entries
          .filter((entry) => {
            const relative = navigator.platform.startsWith("Win")
              ? entry.relative.replaceAll("\\", "/")
              : entry.relative;
            return (
              relative.split("/").slice(0, -1).join("/") ===
              (navigator.platform.startsWith("Win")
                ? args.relative.replaceAll("\\", "/")
                : args.relative)
            );
          })
          .map((entry) => {
            const relative = navigator.platform.startsWith("Win")
              ? entry.relative.replaceAll("\\", "/")
              : entry.relative;
            return {
              name: relative.split("/").at(-1),
              relativePath: entry.relative,
              path: `${args.root}/${entry.relative}`,
              isDirectory: entry.directory,
              isSymlink: false,
            };
          });
        if (native.directoryDelay)
          await new Promise((resolve) =>
            setTimeout(resolve, native.directoryDelay),
          );
        native.directoryReads--;
        return result;
      }
      if (command === "search_project") {
        const result = { matches: [] as any[], limited: false, skipped: 0 };
        for (const [path, file] of Object.entries(native.editorFiles) as [
          string,
          any,
        ][]) {
          const relative = path.slice(args.root.length + 1);
          if (args.relative && !relative.startsWith(args.relative + "/"))
            continue;
          file.content.split("\n").forEach((line: string, index: number) => {
            const options = args.options;
            const expression = options.regex
              ? args.query
              : args.query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
            const pattern = new RegExp(
              options.wholeWord ? `\\b(?:${expression})\\b` : expression,
              options.caseSensitive ? "g" : "gi",
            );
            for (const match of line.matchAll(pattern))
              result.matches.push({
                relative,
                line: index + 1,
                column: match.index + 1,
                length: match[0].length,
                preview: line,
                previewStart: 0,
              });
          });
        }
        if (native.searchDelays[args.query])
          await new Promise((resolve) =>
            setTimeout(resolve, native.searchDelays[args.query]),
          );
        return result;
      }
      if (command !== "file_operation") return;
      if (native.operationDelay)
        await new Promise((resolve) =>
          setTimeout(resolve, native.operationDelay),
        );
      if (native.operationError) throw new Error(native.operationError);
      const { operation, relative, root } = args;
      const source = operation.source ?? relative;
      const oldPath =
        `${(operation.sourceRoot ?? root).replace(/\/$/, "")}/${source}`.replace(
          /\/$/,
          "",
        );
      if (operation.kind === "rename" && !relative) {
        const newPath = `${root.slice(0, root.lastIndexOf("/"))}/${operation.name}`;
        for (const [path, content] of Object.entries(native.editorFiles))
          if (path.startsWith(oldPath + "/")) {
            native.editorFiles[newPath + path.slice(oldPath.length)] = content;
            delete native.editorFiles[path];
          }
        return { oldPath, newPath };
      }
      if (["delete", "trash"].includes(operation.kind)) {
        entries = entries.filter(
          (entry) =>
            entry.relative !== source &&
            !entry.relative.startsWith(source + "/"),
        );
        for (const key of Object.keys(native.editorFiles))
          if (key === oldPath || key.startsWith(oldPath + "/"))
            delete native.editorFiles[key];
        return { oldPath, newPath: null };
      }
      const target =
        operation.kind === "rename"
          ? [...source.split("/").slice(0, -1), operation.name].join("/")
          : operation.kind === "duplicate"
            ? source.replace(/(\.[^/.]+)?$/, " copy$1")
            : [relative, operation.name ?? source.split("/").at(-1)]
                .filter(Boolean)
                .join("/");
      if (entries.some((entry) => entry.relative === target))
        throw new Error("A file or folder with that name already exists.");
      const moving = ["rename", "move"].includes(operation.kind);
      if (operation.kind.startsWith("new"))
        entries.push({
          relative: target,
          directory: operation.kind === "newFolder",
        });
      else {
        const sourceEntries =
          operation.sourceRoot &&
          operation.sourceRoot.replace(/\/$/, "") !== root &&
          native.externalEntries.some(
            (entry: { relative: string }) =>
              entry.relative === source ||
              entry.relative.startsWith(source + "/"),
          )
            ? native.externalEntries
            : entries;
        const copied = sourceEntries
          .filter(
            (entry) =>
              entry.relative === source ||
              entry.relative.startsWith(source + "/"),
          )
          .map((entry) => ({
            ...entry,
            relative: target + entry.relative.slice(source.length),
          }));
        if (moving)
          entries = entries.filter(
            (entry) =>
              entry.relative !== source &&
              !entry.relative.startsWith(source + "/"),
          );
        entries.push(...copied);
        for (const [path, content] of Object.entries(native.editorFiles))
          if (path === oldPath || path.startsWith(oldPath + "/")) {
            native.editorFiles[
              `${root}/${target}${path.slice(oldPath.length)}`
            ] = structuredClone(content);
            if (moving) delete native.editorFiles[path];
          }
      }
      return {
        oldPath: moving ? oldPath : null,
        newPath: `${root}/${target}`,
      };
    };
  }, initialEntries);
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Project folder project", exact: true }),
  ).toBeVisible();
  if (explorerOpenOnSingleClick)
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as any).__nativeTest.calls.some(
            (call: any) => call.command === "load_keybindings",
          ),
        ),
      )
      .toBe(true);
}
async function menu(page: Page, name: string, item: string) {
  await page
    .getByRole("button", { name, exact: true })
    .click({ button: "right" });
  await page.getByRole("menuitem", { name: item, exact: true }).click();
}

async function dispatchClipboardEvent(
  target: import("@playwright/test").Locator,
  type: "copy" | "cut" | "paste",
) {
  return target.evaluate((element, type) => {
    const event = new ClipboardEvent(type, {
      clipboardData: new DataTransfer(),
      bubbles: true,
      cancelable: true,
    });
    element.dispatchEvent(event);
    return event.defaultPrevented;
  }, type);
}

test("Explorer clicks select entries while double clicks and chevrons activate them", async ({
  page,
}) => {
  await setup(page);
  const tree = page.locator(".file-tree");
  const folder = tree.locator('.tree-entry[title="/project/src"]');
  const folderRow = folder.locator("..");
  const file = tree.locator('.tree-entry[title="/project/README.md"]');
  const fileRow = file.locator("..");

  await folder.click();
  await expect(folderRow).toHaveAttribute("data-selected", "true");
  await expect(folder).toHaveAttribute("aria-expanded", "false");
  await page.mouse.move(1400, 880);
  const selectedBackground = await folderRow.evaluate(
    (row) => getComputedStyle(row).backgroundColor,
  );
  expect(selectedBackground).not.toBe("rgba(0, 0, 0, 0)");

  await file.click();
  await expect(fileRow).toHaveAttribute("data-selected", "true");
  await expect(folderRow).not.toHaveAttribute("data-selected", "true");
  await expect(tree.locator('.tree-row[data-selected="true"]')).toHaveCount(1);
  await expect(page.locator(".cm-content")).toHaveCount(0);
  await page.mouse.move(1400, 880);
  await page.locator(".explorer-panel").screenshot({
    path: test.info().outputPath("explorer-selected-entry.png"),
  });

  await page.getByRole("button", { name: "Expand src", exact: true }).click();
  await expect(folder).toHaveAttribute("aria-expanded", "true");
  await expect(folderRow).toHaveAttribute("data-selected", "true");
  await page.keyboard.press("F2");
  const renameFolder = tree.getByRole("textbox", { name: "Rename name" });
  await expect(renameFolder).toHaveValue("src");
  await renameFolder.press("Escape");
  await folder.click();
  await expect(folder).toHaveAttribute("aria-expanded", "true");
  await folder.dblclick();
  await expect(folder).toHaveAttribute("aria-expanded", "false");

  await page.getByRole("button", { name: "Expand src", exact: true }).click();
  const sourceFile = tree.locator('.tree-entry[title="/project/src/main.ts"]');
  await sourceFile.click();
  await expect(page.locator(".cm-content")).toHaveCount(0);
  await sourceFile.dblclick();
  await expect(page.locator(".editor-path")).toContainText("src/main.ts");
});

test("keyboard activation and commands act on the clicked Explorer entry", async ({
  page,
}) => {
  await setup(page);
  const tree = page.locator(".file-tree");
  const folder = tree.locator('.tree-entry[title="/project/src"]');
  await folder.focus();
  await page.keyboard.press("Enter");
  await expect(folder).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("Space");
  await expect(folder).toHaveAttribute("aria-expanded", "false");

  const readme = tree.locator('.tree-entry[title="/project/README.md"]');
  await readme.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator(".editor-path")).toContainText("README.md");
  await page
    .getByRole("button", { name: "Close README.md", exact: true })
    .click();
  await readme.focus();
  await page.keyboard.press("Space");
  await expect(page.locator(".editor-path")).toContainText("README.md");
  await page
    .getByRole("button", { name: "Close README.md", exact: true })
    .click();

  await readme.click();
  await page.keyboard.press("F2");
  const rename = tree.getByRole("textbox", { name: "Rename name" });
  await expect(rename).toHaveValue("README.md");
  await rename.press("Escape");

  await readme.click();
  await page.keyboard.press("Delete");
  const trash = page.getByRole("dialog", { name: "Move to Trash" });
  await expect(trash.locator("strong")).toHaveText("README.md");
  await trash.getByRole("button", { name: "Cancel", exact: true }).click();

  await readme.click();
  await page.keyboard.press("Control+Delete");
  const permanentlyDelete = page.getByRole("dialog", {
    name: "Delete Permanently",
  });
  await expect(permanentlyDelete.locator("strong")).toHaveText("README.md");
  await permanentlyDelete
    .getByRole("button", { name: "Cancel", exact: true })
    .click();

  await readme.click();
  await page.keyboard.press("Control+c");
  await menu(page, "src", "Paste");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "file_operation")
            .at(-1)?.args,
      ),
    )
    .toMatchObject({
      root: "/project",
      relative: "src",
      operation: { kind: "copy", source: "README.md" },
    });
});

for (const platform of ["linux", "windows"] as const) {
  test(`${platform} Explorer handles native clipboard events without keydown`, async ({
    page,
  }) => {
    await setup(page, true, undefined, false, platform);
    const tree = page.locator(".file-tree");
    const readme = tree.locator('.tree-entry[title="/project/README.md"]');
    const folder = tree.locator('.tree-entry[title="/project/src"]');

    await page.getByRole("button", { name: "Expand src", exact: true }).click();

    await readme.focus();
    expect(await dispatchClipboardEvent(readme, "copy")).toBe(true);
    await folder.focus();
    expect(await dispatchClipboardEvent(folder, "paste")).toBe(true);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls
              .filter((call: any) => call.command === "file_operation")
              .at(-1)?.args,
        ),
      )
      .toMatchObject({
        root: "/project",
        relative: "src",
        operation: {
          kind: "copy",
          sourceRoot: "/project",
          source: "README.md",
        },
      });

    const source = tree.locator('.tree-entry[title="/project/src/main.ts"]');
    await source.focus();
    expect(await dispatchClipboardEvent(source, "cut")).toBe(true);
    await expect(source.locator("..")).toHaveAttribute("data-cut", "true");
    await readme.focus();
    expect(await dispatchClipboardEvent(readme, "paste")).toBe(true);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls
              .filter((call: any) => call.command === "file_operation")
              .at(-1)?.args,
        ),
      )
      .toMatchObject({
        root: "/project",
        relative: "",
        operation: {
          kind: "move",
          sourceRoot: "/project",
          source: "src/main.ts",
        },
      });
    await expect(source).toHaveCount(0);
  });

  test(`${platform} Explorer duplicates with Ctrl+D and renames with F2`, async ({
    page,
  }) => {
    await setup(page, true, undefined, false, platform);
    const tree = page.locator(".file-tree");
    const readme = tree.locator('.tree-entry[title="/project/README.md"]');

    await readme.focus();
    await page.keyboard.press("Control+d");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls
              .filter((call: any) => call.command === "file_operation")
              .at(-1)?.args,
        ),
      )
      .toMatchObject({
        root: "/project",
        relative: "README.md",
        operation: { kind: "duplicate" },
      });
    await expect(
      tree.locator('.tree-entry[title="/project/README copy.md"]'),
    ).toBeVisible();

    await readme.focus();
    await page.keyboard.press("F2");
    const rename = tree.getByRole("textbox", { name: "Rename name" });
    await expect(rename).toHaveValue("README.md");
    await rename.press("Escape");
  });

  test(`${platform} Explorer supports Ctrl+C, Ctrl+X, and Ctrl+V for files`, async ({
    page,
  }) => {
    await setup(page, true, undefined, false, platform);
    const tree = page.locator(".file-tree");
    const folder = tree.locator('.tree-entry[title="/project/src"]');
    const folderRow = folder.locator("..");
    const readme = tree.locator('.tree-entry[title="/project/README.md"]');
    const readmeRow = readme.locator("..");

    await page.getByRole("button", { name: "Expand src", exact: true }).click();

    // Meta is not the file shortcut on Linux or Windows.
    await readme.click();
    await page.keyboard.press("Meta+x");
    await expect(readmeRow).not.toHaveAttribute("data-cut", "true");

    // Folders can be cut too, and copying replaces the cut state immediately.
    await folder.click();
    await page.keyboard.press("Control+x");
    await expect(folderRow).toHaveAttribute("data-cut", "true");
    await expect(folderRow.locator(".tree-cut-mark")).toBeVisible();
    await page.keyboard.press("Control+c");
    await expect(folderRow).not.toHaveAttribute("data-cut", "true");
    await expect(folderRow.locator(".tree-cut-mark")).toHaveCount(0);

    // Copy README into src with Ctrl+V.
    await readme.click();
    await page.keyboard.press("Control+c");
    await folder.focus();
    await page.keyboard.press("Control+v");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls
              .filter((call: any) => call.command === "file_operation")
              .at(-1)?.args,
        ),
      )
      .toMatchObject({
        root: "/project",
        relative: "src",
        operation: {
          kind: "copy",
          sourceRoot: "/project",
          source: "README.md",
        },
      });
    await expect(
      tree.locator('.tree-entry[title="/project/src/README.md"]'),
    ).toBeVisible();

    // Cut a file, confirm the muted scissors state, then move it across folders.
    const source = tree.locator('.tree-entry[title="/project/src/main.ts"]');
    const sourceRow = source.locator("..");
    await source.click();
    await page.keyboard.press("Control+x");
    await expect(sourceRow).toHaveAttribute("data-cut", "true");
    await expect(sourceRow.locator(".tree-cut-mark")).toBeVisible();
    await expect
      .poll(() =>
        source
          .locator("span")
          .last()
          .evaluate((name) => {
            const style = getComputedStyle(name);
            return { filter: style.filter, opacity: style.opacity };
          }),
      )
      .toEqual({ filter: "grayscale(1)", opacity: "0.55" });
    await expect
      .poll(() =>
        source
          .locator("svg")
          .nth(1)
          .evaluate((icon) => {
            const style = getComputedStyle(icon);
            return { filter: style.filter, opacity: style.opacity };
          }),
      )
      .toEqual({ filter: "grayscale(1)", opacity: "0.55" });
    if (platform === "linux")
      await page.locator(".explorer-panel").screenshot({
        path: test.info().outputPath("explorer-cut-item.png"),
      });

    await page.keyboard.press("Control+c");
    await expect(sourceRow).not.toHaveAttribute("data-cut", "true");
    await expect(sourceRow.locator(".tree-cut-mark")).toHaveCount(0);
    await page.keyboard.press("Control+x");
    await expect(sourceRow).toHaveAttribute("data-cut", "true");
    await readme.focus();
    await page.keyboard.press("Control+v");

    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls
              .filter((call: any) => call.command === "file_operation")
              .at(-1)?.args,
        ),
      )
      .toMatchObject({
        root: "/project",
        relative: "",
        operation: {
          kind: "move",
          sourceRoot: "/project",
          source: "src/main.ts",
        },
      });
    await expect(
      tree.locator('.tree-entry[title="/project/main.ts"]'),
    ).toBeVisible();
    await expect(
      tree.locator('.tree-entry[title="/project/src/main.ts"]'),
    ).toHaveCount(0);

    // A successful cut paste consumes the in-app clipboard.
    await tree
      .locator('.tree-entry[title="/project/main.ts"]')
      .click({ button: "right" });
    await expect(page.getByRole("menuitem", { name: "Paste" })).toBeDisabled();
    await page.keyboard.press("Escape");
  });
}

test("Windows-style Explorer paths paste a copied item into its current folder", async ({
  page,
}) => {
  await setup(
    page,
    true,
    [
      { relative: "src", directory: true },
      { relative: "src\\main.ts", directory: false },
      { relative: "README.md", directory: false },
    ],
    false,
    "windows",
  );
  const tree = page.locator(".file-tree");
  const folder = tree.locator('.tree-entry[title="/project/src"]');
  await page.getByRole("button", { name: "Expand src", exact: true }).click();
  const source = tree.getByRole("button", { name: "main.ts", exact: true });

  await source.click();
  await page.keyboard.press("Control+c");
  await folder.focus();
  await page.keyboard.press("Control+v");

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "file_operation")
            .at(-1)?.args,
      ),
    )
    .toMatchObject({
      root: "/project",
      relative: "src\\main.ts",
      operation: { kind: "duplicate" },
    });
  await expect(
    tree.getByRole("button", { name: "main copy.ts", exact: true }),
  ).toBeVisible();
});

test("the in-app clipboard survives switching Explorer projects", async ({
  page,
}) => {
  const project = newProject("/project", "local:bash");
  const otherProject = newProject("/other", "local:bash");
  const saved = {
    ...newSession(),
    projects: [project, otherProject],
    activeProjectId: project.id,
  };
  await setup(page, true, undefined, false, "linux", saved);

  const readme = page.locator(
    '.file-tree .tree-entry[title="/project/README.md"]',
  );
  await readme.click();
  await page.keyboard.press("Control+c");

  await page.locator(".project-switcher").click();
  await page.getByRole("menuitem", { name: "other", exact: true }).click();
  await expect(page.locator(".project-switcher")).toContainText("other");

  const destination = page.locator(
    '.file-tree .tree-entry[title="/other/src"]',
  );
  await destination.focus();
  await page.keyboard.press("Control+v");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "file_operation")
            .at(-1)?.args,
      ),
    )
    .toMatchObject({
      root: "/other",
      relative: "src",
      operation: {
        kind: "copy",
        sourceRoot: "/project",
        source: "README.md",
      },
    });
});

test("single-click mode delays activation without selecting and leaves chevrons immediate", async ({
  page,
}) => {
  await setup(page, true, undefined, true);
  const tree = page.locator(".file-tree");
  const file = tree.locator('.tree-entry[title="/project/README.md"]');
  const folder = tree.locator('.tree-entry[title="/project/src"]');

  await file.click();
  await page.waitForTimeout(150);
  await expect(file).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator(".editor-path")).toHaveCount(0);
  await expect(
    page.getByRole("tab", { name: "README.md", exact: true }),
  ).toHaveCount(0);
  await page.waitForTimeout(400);
  await expect(page.locator(".editor-path")).toContainText("README.md", {
    timeout: 1000,
  });
  await expect(
    page.getByRole("tab", { name: "README.md", exact: true }),
  ).toBeVisible();
  await expect(file).toHaveAttribute("aria-pressed", "false");
  await expect(tree.locator('.tree-row[data-selected="true"]')).toHaveCount(0);

  await page
    .getByRole("button", { name: "Close README.md", exact: true })
    .click();

  await folder.click();
  await page.waitForTimeout(150);
  await expect(folder).toHaveAttribute("aria-pressed", "false");
  await expect(folder).toHaveAttribute("aria-expanded", "false");
  await expect(
    tree.getByRole("button", { name: "main.ts", exact: true }),
  ).toHaveCount(0);
  await expect(folder).toHaveAttribute("aria-expanded", "true", {
    timeout: 1000,
  });
  await expect(folder).toHaveAttribute("aria-pressed", "false");

  await tree.getByRole("button", { name: "Collapse src", exact: true }).click();
  await expect(folder).toHaveAttribute("aria-expanded", "false", {
    timeout: 250,
  });
  await tree.getByRole("button", { name: "Expand src", exact: true }).click();
  await expect(folder).toHaveAttribute("aria-expanded", "true", {
    timeout: 250,
  });
});

test("double-click mode selects only and keyboard activation and shortcuts remain immediate", async ({
  page,
}) => {
  await setup(page, true, undefined, true);
  const tree = page.locator(".file-tree");
  const file = tree.locator('.tree-entry[title="/project/README.md"]');
  const fileRow = file.locator("..");
  const folder = tree.locator('.tree-entry[title="/project/src"]');
  const folderRow = folder.locator("..");

  await file.dblclick();
  await expect(file).toHaveAttribute("aria-pressed", "true");
  await expect(fileRow).toHaveAttribute("data-selected", "true");
  await page.waitForTimeout(600);
  await expect(page.locator(".editor-path")).toHaveCount(0);
  await expect(
    page.getByRole("tab", { name: "README.md", exact: true }),
  ).toHaveCount(0);

  await folder.dblclick();
  await expect(folder).toHaveAttribute("aria-pressed", "true");
  await expect(folderRow).toHaveAttribute("data-selected", "true");
  await expect(file).toHaveAttribute("aria-pressed", "false");
  await expect(folder).toHaveAttribute("aria-expanded", "false");
  await page.waitForTimeout(600);
  await expect(page.locator(".editor-path")).toHaveCount(0);
  await expect(
    page.getByRole("tab", { name: "README.md", exact: true }),
  ).toHaveCount(0);
  await expect(folder).toHaveAttribute("aria-expanded", "false");

  await folder.focus();
  await page.keyboard.press("Enter");
  await expect(folder).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("Space");
  await expect(folder).toHaveAttribute("aria-expanded", "false");

  await file.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator(".editor-path")).toContainText("README.md");
  await page
    .getByRole("button", { name: "Close README.md", exact: true })
    .click();
  await file.focus();
  await page.keyboard.press("Space");
  await expect(page.locator(".editor-path")).toContainText("README.md");
  await page
    .getByRole("button", { name: "Close README.md", exact: true })
    .click();

  await file.focus();
  await page.keyboard.press("F2");
  const rename = tree.getByRole("textbox", { name: "Rename name" });
  await expect(rename).toHaveValue("README.md");
  await rename.press("Escape");
});

test("dragging out of an Explorer entry clears pointer focus suppression", async ({
  page,
}) => {
  await setup(page, true, undefined, true);
  const tree = page.locator(".file-tree");
  const file = tree.locator('.tree-entry[title="/project/README.md"]');
  const fileRow = file.locator("..");
  const project = page.getByRole("button", {
    name: "Project folder project",
    exact: true,
  });
  const bounds = (await file.boundingBox())!;

  await page.mouse.move(
    bounds.x + bounds.width / 2,
    bounds.y + bounds.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(1000, 400, { steps: 4 });
  await page.mouse.up();
  await expect(file).toHaveAttribute("aria-pressed", "false");

  await project.focus();
  await expect(project).toHaveAttribute("aria-pressed", "true");
  await file.focus();
  await expect(file).toHaveAttribute("aria-pressed", "true");
  await expect(fileRow).toHaveAttribute("data-selected", "true");

  await page.keyboard.press("F2");
  const rename = tree.getByRole("textbox", { name: "Rename name" });
  await expect(rename).toHaveValue("README.md");
  await rename.press("Escape");
});

test("automatically refreshes visible directories without Git and releases collapsed watches", async ({
  page,
}, testInfo) => {
  await setup(page, false);
  const tree = page.locator(".file-tree");
  await tree.getByRole("button", { name: "Expand src", exact: true }).click();
  await expect(
    tree.getByRole("button", { name: "main.ts", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const native = (window as any).__nativeTest;
        return [...native.explorerWatchers.values()].map(
          (watcher: any) => watcher.relatives,
        );
      }),
    )
    .toEqual([["", "src"]]);
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__nativeTest.directoryReads),
    )
    .toBe(0);
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.calls.length = 0;
    native.setExplorerEntries([
      { relative: "src", directory: true },
      { relative: "src/main.ts", directory: false },
      { relative: "src/ignored.log", directory: false },
      { relative: "README.md", directory: false },
      { relative: "new folder", directory: true },
    ]);
    native.changeExplorerDirectories(["src"]);
  });
  await expect(
    tree.getByRole("button", { name: "ignored.log", exact: true }),
  ).toBeVisible();
  await expect(
    tree.getByRole("button", { name: "new folder", exact: true }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls
        .filter((call: any) => call.command === "list_directory")
        .map((call: any) => call.args.relative),
    ),
  ).toEqual(["src"]);
  await page.evaluate(() =>
    (window as any).__nativeTest.changeExplorerDirectories([""]),
  );
  await expect(
    tree.getByRole("button", { name: "new folder", exact: true }),
  ).toBeVisible();
  await expect(
    tree.getByRole("button", { name: "src", exact: true }),
  ).toHaveAttribute("aria-expanded", "true");
  await page.screenshot({
    path: testInfo.outputPath("explorer-auto-refresh.png"),
  });
  await tree.getByRole("button", { name: "Collapse src", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...(window as any).__nativeTest.explorerWatchers.values()].map(
          (watcher: any) => watcher.relatives,
        ),
      ),
    )
    .toEqual([[""]]);
  await page
    .getByRole("button", { name: "Search in project", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__nativeTest.explorerWatchers.size),
    )
    .toBe(0);
  await page
    .getByRole("button", { name: "Back to Explorer", exact: true })
    .click();
  await expect(
    tree.getByRole("button", { name: "new folder", exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.setExplorerEntries([{ relative: "renamed.txt", directory: false }]);
    native.changeExplorerDirectories([""]);
  });
  await expect(
    tree.getByRole("button", { name: "renamed.txt", exact: true }),
  ).toBeVisible();
  await expect(
    tree.getByRole("button", { name: "new folder", exact: true }),
  ).toHaveCount(0);
});

test("coalesces updates during a slow directory read without losing the final change", async ({
  page,
}) => {
  await setup(page, false);
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__nativeTest.directoryReads),
    )
    .toBe(0);
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.maxDirectoryReads = 0;
    native.directoryDelay = 300;
    native.calls.length = 0;
    native.changeExplorerDirectories([""]);
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__nativeTest.directoryReads),
    )
    .toBe(1);
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.setExplorerEntries([{ relative: "last.txt", directory: false }]);
    for (let i = 0; i < 100; i++) native.changeExplorerDirectories([""]);
  });
  await expect(
    page
      .locator(".file-tree")
      .getByRole("button", { name: "last.txt", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => (window as any).__nativeTest.maxDirectoryReads),
  ).toBe(1);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "list_directory",
        ).length,
    ),
  ).toBe(2);
});

test("Explorer colors files and ancestor folders and refreshes new files and clean states", async ({
  page,
}, testInfo) => {
  await setup(page, true, [
    { relative: "src", directory: true },
    { relative: "src/nested", directory: true },
    { relative: "src/nested/deep", directory: true },
    { relative: ".new", directory: true },
    { relative: "src-other", directory: true },
    ...[
      "README.md",
      ".env",
      "clean.txt",
      "src/main.ts",
      "src/new.ts",
      "src/ignored.log",
      "src/conflict.ts",
      "src/nested/deep/changed.ts",
      ".new/file.ts",
    ].map((relative) => ({ relative, directory: false })),
  ]);
  await page.evaluate(() => {
    const desktop = window as any;
    desktop.__explorerGitStatus = {
      root: "/",
      branch: "main",
      changes: [
        { path: "project/README.md", index: "M", worktree: " " },
        { path: "project/.env", index: " ", worktree: "M" },
        { path: "project/src/main.ts", index: "A", worktree: "M" },
        { path: "project/src/new.ts", index: "?", worktree: "?" },
        { path: "project/src/conflict.ts", index: "A", worktree: "A" },
        {
          path: "project/src/nested/deep/changed.ts",
          index: " ",
          worktree: "M",
        },
        { path: "project/.new/file.ts", index: "?", worktree: "?" },
        { path: "other/clean.txt", index: " ", worktree: "M" },
      ].map((change) => ({ ...change, originalPath: null })),
    };
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = (command: string, args: unknown) =>
      command === "git_status"
        ? Promise.resolve(structuredClone(desktop.__explorerGitStatus))
        : command === "git_repositories"
          ? Promise.resolve({
              repositories: desktop.__explorerGitStatus
                ? [structuredClone(desktop.__explorerGitStatus)]
                : [],
              errors: [],
              limited: false,
            })
          : invoke(command, args);
    window.dispatchEvent(new Event("focus"));
  });
  const tree = page.locator(".file-tree");
  const source = tree.getByRole("button", { name: "src", exact: true });
  await expect(source).toHaveAttribute("data-git-status", "U");
  await expect(source).toHaveAttribute("aria-expanded", "false");
  await tree.getByRole("button", { name: "Expand src", exact: true }).click();
  await tree
    .getByRole("button", { name: "Expand nested", exact: true })
    .click();
  for (const mode of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: mode });
    await expect(page.locator("html")).toHaveAttribute("data-appearance", mode);
    const colors = await page.evaluate(() => {
      const probe = document.createElement("span");
      document.body.append(probe);
      const resolve = (token: string) => {
        probe.style.color = `var(${token})`;
        return getComputedStyle(probe).color;
      };
      const colors = {
        added: resolve("--color-info"),
        modified: resolve("--color-warning"),
        conflict: resolve("--color-error"),
      };
      probe.remove();
      return colors;
    });
    for (const [name, color] of [
      ["README.md", colors.modified],
      [".env", colors.modified],
      ["main.ts", colors.added],
      ["new.ts", colors.added],
      ["conflict.ts", colors.conflict],
      ["src", colors.conflict],
      ["nested", colors.modified],
      ["deep", colors.modified],
      [".new", colors.added],
    ]) {
      const file = tree.getByRole("button", { name, exact: true });
      await expect(file.locator("span").last()).toHaveCSS("color", color);
      await expect(file.locator("svg").last()).toHaveCSS("color", color);
    }
    const neutral = await tree
      .getByRole("button", { name: "clean.txt", exact: true })
      .evaluate((element) => getComputedStyle(element).color);
    await expect(
      tree.getByRole("button", { name: "ignored.log", exact: true }),
    ).toHaveCSS("color", neutral);
    await expect(
      tree.getByRole("button", { name: "src-other", exact: true }),
    ).toHaveCSS("color", neutral);
    await expect(page.locator(".project-tree-heading")).toHaveCSS(
      "color",
      colors.conflict,
    );
    await page
      .locator(".explorer-panel")
      .screenshot({ path: testInfo.outputPath(`explorer-git-${mode}.png`) });
  }
  await tree.getByRole("button", { name: "README.md", exact: true }).focus();
  await page.evaluate(async () => {
    const desktop = window as any;
    await desktop.__TAURI_INTERNALS__.invoke("file_operation", {
      root: "/project",
      relative: "src",
      operation: { kind: "newFile", name: "created.ts" },
    });
    desktop.__explorerGitStatus.changes.push({
      path: "project/src/created.ts",
      originalPath: null,
      index: "?",
      worktree: "?",
    });
    desktop.__explorerGitStatus.changes =
      desktop.__explorerGitStatus.changes.filter(
        (change: { path: string }) => change.path !== "project/src/conflict.ts",
      );
  });
  const addedColor = await tree
    .getByRole("button", { name: "new.ts", exact: true })
    .evaluate((element) => getComputedStyle(element).color);
  const modifiedColor = await tree
    .getByRole("button", { name: "README.md", exact: true })
    .evaluate((element) => getComputedStyle(element).color);
  await expect(
    tree.getByRole("button", { name: "created.ts", exact: true }),
  ).toHaveCSS("color", addedColor, { timeout: 10000 });
  await expect(source).toHaveCSS("color", modifiedColor);
  await expect(
    tree.getByRole("button", { name: "README.md", exact: true }),
  ).toBeFocused();
  await page.evaluate(() => {
    (window as any).__explorerGitStatus.changes = [];
  });
  await page
    .getByRole("button", { name: "Refresh explorer", exact: true })
    .click();
  await expect(tree.locator("[data-git-status]")).toHaveCount(0);
  await expect(
    page.locator(".project-tree-heading[data-git-status]"),
  ).toHaveCount(0);
  await page.evaluate(() => {
    (window as any).__explorerGitStatus = null;
    window.dispatchEvent(new Event("focus"));
  });
  await expect(
    page.getByRole("button", { name: /Toggle source control/ }),
  ).toHaveCount(0);
  await expect(tree.locator("[data-git-status]")).toHaveCount(0);
});

test("searches the project or a folder and opens the matching editor selection", async ({
  page,
}) => {
  await setup(page);
  await menu(page, "src", "Search in Folder…");
  await page.getByRole("textbox", { name: "Search in files" }).fill("needle");
  await expect(page.locator(".search-match")).toHaveCount(2);
  await page
    .getByRole("button", { name: "src/main.ts, line 2, column 4", exact: true })
    .click();
  await expect(page.locator(".cm-content")).toContainText("🦀 needle here");
  await expect(page.getByRole("contentinfo")).toContainText("Ln 2, Col 10");
  await page.keyboard.insertText("REPLACED");
  await expect(page.locator(".cm-content")).toContainText("🦀 REPLACED here");
  await page.getByRole("button", { name: "Search entire project" }).click();
  await expect(page.locator(".search-match")).toHaveCount(3);
  await page.screenshot({ path: test.info().outputPath("project-search.png") });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.locator(".project-search").screenshot({
    path: test.info().outputPath("project-search-dark.png"),
  });
  await page.getByRole("button", { name: "Back to Explorer" }).click();
  await menu(page, "Project folder project", "Search in Folder…");
  await expect(
    page.getByRole("textbox", { name: "Search in files" }),
  ).toHaveValue("needle");
  await expect(
    page.getByRole("textbox", { name: "Search in files" }),
  ).toBeFocused();
  await expect(page.locator(".search-match")).toHaveCount(3);
});

test("renames a folder without losing dirty editor text or undo history", async ({
  page,
}) => {
  await setup(page);
  await page.getByRole("button", { name: "Expand src", exact: true }).click();
  await page.getByRole("button", { name: "main.ts", exact: true }).dblclick();
  await expect(page.locator(".cm-content")).toBeVisible();
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+End");
  await page.keyboard.insertText("unsaved");
  await menu(page, "src", "Rename…");
  const input = page.locator(".file-tree").getByRole("textbox", {
    name: "Rename name",
    exact: true,
  });
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("src");
  await expect(
    page.getByRole("button", { name: "main.ts", exact: true }),
  ).toBeVisible();
  await input.fill("code");
  await input.press("Enter");
  await expect(input).toHaveCount(0);
  await expect(page.locator(".editor-path")).toContainText("code/main.ts");
  await expect(page.locator(".cm-content")).toContainText("unsaved");
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+z");
  await expect(page.locator(".cm-content")).not.toContainText("unsaved");
  await page.keyboard.press("Control+Shift+z");
  await page.keyboard.press("Control+s");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.editorFiles["/project/code/main.ts"]
            .content,
      ),
    )
    .toContain("unsaved");
  await expect(
    page.getByRole("tab", { name: "Terminal", exact: true }),
  ).toBeVisible();
});

test("deleting a dirty folder supports cancel and retains edits after a failed save", async ({
  page,
}) => {
  await setup(page);
  await page.getByRole("button", { name: "Expand src", exact: true }).click();
  await page.getByRole("button", { name: "main.ts", exact: true }).dblclick();
  await page.locator(".cm-content").fill("dirty");
  await menu(page, "src", "Delete Permanently…");
  await page
    .getByRole("button", { name: "Delete Permanently", exact: true })
    .click();
  const guard = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await guard.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.locator(".cm-content")).toContainText("dirty");
  await page
    .getByRole("button", { name: "Delete Permanently", exact: true })
    .click();
  await page.evaluate(() => {
    (window as any).__nativeTest.failFileSave = true;
  });
  await guard.getByRole("button", { name: /Save/ }).click();
  await expect(guard).toContainText("Disk is full");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "file_operation",
          ).length,
      ),
    )
    .toBe(0);
  await page.evaluate(() => {
    (window as any).__nativeTest.failFileSave = false;
  });
  await guard.getByRole("button", { name: /Save/ }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("tab", { name: /main.ts/ })).toHaveCount(0);
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "src", exact: true }),
  ).toHaveCount(0);
});

for (const empty of [false, true]) {
  test(`creates files and folders from blank Explorer space in an ${empty ? "empty" : "existing"} project`, async ({
    page,
  }) => {
    await setup(page, false, empty ? [] : undefined);
    await page.emulateMedia({ colorScheme: empty ? "light" : "dark" });
    const tree = page.locator(".file-tree");
    const entryActions = ["Rename…", "Move to Trash…", "Delete Permanently…"];
    if (empty) await expect(tree).toContainText("Empty folder");
    else
      await expect(
        tree.getByRole("button", { name: "README.md", exact: true }),
      ).toBeVisible();

    for (const [label, kind, name] of [
      ["New Folder", "newFolder", "docs"],
      ["New File", "newFile", "notes.txt"],
    ]) {
      const bounds = (await tree.boundingBox())!;
      await tree.click({
        button: "right",
        position: { x: bounds.width / 2, y: bounds.height - 12 },
      });
      await expect(
        page.getByRole("menu", { name: "project actions", exact: true }),
      ).toBeVisible();
      for (const name of entryActions)
        await expect(
          page.getByRole("menuitem", { name, exact: true }),
        ).toHaveCount(0);
      if (kind === "newFolder")
        await page.screenshot({
          path: test.info().outputPath("explorer-blank-menu.png"),
        });
      await page.getByRole("menuitem", { name: label, exact: true }).click();
      const input = tree.getByRole("textbox", {
        name: `${label} name`,
        exact: true,
      });
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(input).toBeFocused();
      await expect(input).toHaveValue("");
      await page.locator(".explorer-panel").screenshot({
        path: test.info().outputPath(`explorer-inline-${kind}.png`),
      });
      await input.fill(name);
      await input.press("Enter");
      await expect(input).toHaveCount(0);
      await expect(
        tree.getByRole("button", { name, exact: true }),
      ).toBeVisible();
      expect(
        await page.evaluate(() =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "file_operation")
            .at(-1),
        ),
      ).toMatchObject({
        args: { root: "/project", relative: "", operation: { kind, name } },
      });
    }
    await expect(page.locator(".editor-path")).toContainText("notes.txt");
    for (const name of ["Project folder project", "docs", "notes.txt"]) {
      await page
        .getByRole("button", { name, exact: true })
        .click({ button: "right" });
      for (const action of entryActions)
        await expect(
          page.getByRole("menuitem", { name: action, exact: true }),
        ).toBeEnabled();
      await page.keyboard.press("Escape");
    }
  });
}

test("cancels unnamed or unconfirmed Explorer items without creating anything", async ({
  page,
}) => {
  await setup(page, false);
  for (const label of ["New File", "New Folder"]) {
    for (const [name, finish] of [
      ["", "Enter"],
      ["   ", "Enter"],
      ["", "blur"],
      ["cancelled", "Escape"],
      ["cancelled", "blur"],
    ]) {
      await menu(page, "Project folder project", label);
      const input = page.getByRole("textbox", {
        name: `${label} name`,
        exact: true,
      });
      await expect(input).toBeFocused();
      await input.fill(name);
      if (finish === "blur")
        await page
          .getByRole("button", { name: "Project folder project", exact: true })
          .click();
      else await input.press(finish);
      await expect(input).toHaveCount(0);
    }
  }
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "file_operation",
      ),
    ),
  ).toHaveLength(0);
});

test("creates inline in nested folders and retains the name after errors and IME confirmation", async ({
  page,
}) => {
  await setup(page);
  const src = page.getByRole("button", { name: "src", exact: true });
  await expect(src).toHaveAttribute("aria-expanded", "false");
  await menu(page, "src", "New Folder");
  await expect(src).toHaveAttribute("aria-expanded", "true");
  const folder = page.getByRole("textbox", {
    name: "New Folder name",
    exact: true,
  });
  await expect(folder).toBeFocused();
  await folder.fill("main.ts");
  await folder.press("Enter");
  await expect(page.getByRole("alert").filter({ hasText: /\S/ })).toContainText(
    "already exists",
  );
  await expect(folder).toHaveValue("main.ts");
  await expect(folder).toBeFocused();
  await folder.fill("docs");
  await folder.press("Enter");
  await expect(folder).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "docs", exact: true }),
  ).toHaveAttribute("title", "/project/src/docs");

  await menu(page, "docs", "New File");
  const file = page.getByRole("textbox", {
    name: "New File name",
    exact: true,
  });
  await expect(file).toBeFocused();
  await file.fill("草稿.txt");
  for (const key of ["Enter", "Escape"])
    expect(
      await file.evaluate(
        (element, key) =>
          element.dispatchEvent(
            new KeyboardEvent("keydown", {
              key,
              isComposing: true,
              bubbles: true,
              cancelable: true,
            }),
          ),
        key,
      ),
    ).toBe(key !== "Enter");
  await expect(file).toHaveValue("草稿.txt");
  await expect(file).toBeFocused();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "file_operation",
      ),
    ),
  ).toHaveLength(2);
  await file.press("Enter");
  await expect(file).toHaveCount(0);
  await expect(page.locator(".editor-path")).toContainText("src/docs/草稿.txt");

  await menu(page, "main.ts", "New File");
  await file.fill("sibling.ts");
  await file.press("Enter");
  await expect(page.locator(".editor-path")).toContainText("src/sibling.ts");
});

test("creates and copies items and exposes scoped Git history", async ({
  page,
}) => {
  await setup(page);
  await menu(page, "README.md", "Copy");
  await menu(page, "src", "Paste");
  await page.getByRole("button", { name: "Expand src", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "README.md", exact: true }),
  ).toHaveCount(2);
  await menu(page, "Project folder project", "New Folder");
  const input = page.getByRole("textbox", {
    name: "New Folder name",
    exact: true,
  });
  await input.fill("docs");
  await input.press("Enter");
  await expect(
    page.getByRole("button", { name: "docs", exact: true }),
  ).toBeVisible();
  await menu(page, "src", "View History");
  await expect(
    page.getByRole("dialog", { name: "Git History · src" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "git_history")
            .at(-1)?.args.path,
      ),
    )
    .toBe("src");
});

test("context menus fit small windows and inline renames retain errors and support retry", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await setup(page, false);
  await page
    .getByRole("button", { name: "README.md", exact: true })
    .click({ button: "right" });
  await expect(
    page.getByRole("menuitem", { name: "View History", exact: true }),
  ).toBeDisabled();
  const bounds = await page.getByRole("menu").boundingBox();
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(420);
  await page.screenshot({
    path: test.info().outputPath("explorer-menu-small.png"),
  });
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "README.md", exact: true }).focus();
  await page.keyboard.press("F2");
  const input = page.locator(".file-tree").getByRole("textbox", {
    name: "Rename name",
    exact: true,
  });
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("README.md");
  expect(
    await input.evaluate((element: HTMLInputElement) => [
      element.selectionStart,
      element.selectionEnd,
    ]),
  ).toEqual([0, "README.md".length]);
  await input.fill("src");
  await input.press("Enter");
  await expect(page.getByRole("alert").filter({ hasText: /\S/ })).toContainText(
    "already exists",
  );
  await expect(input).toHaveValue("src");
  await expect(input).toBeFocused();
  await input.fill("草稿.md");
  for (const key of ["Enter", "Escape"])
    await input.dispatchEvent("keydown", { key, isComposing: true });
  await expect(input).toHaveValue("草稿.md");
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "file_operation",
      ),
    ),
  ).toHaveLength(1);
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme });
    await page.locator(".explorer-panel").screenshot({
      path: test.info().outputPath(`rename-file-${colorScheme}.png`),
    });
  }
  await input.press("Enter");
  await expect(input).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "草稿.md", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls
        .filter((call: any) => call.command === "file_operation")
        .at(-1),
    ),
  ).toMatchObject({
    args: {
      root: "/project",
      relative: "README.md",
      operation: { kind: "rename", name: "草稿.md" },
    },
  });
});

test("cancels inline file and folder renames without changing disk contents", async ({
  page,
}) => {
  await setup(page, false);
  for (const [entry, original] of [
    ["README.md", "README.md"],
    ["src", "src"],
    ["Project folder project", "project"],
  ]) {
    const row = page.getByRole("button", { name: entry, exact: true });
    for (const [name, finish] of [
      ["", "Enter"],
      ["   ", "Enter"],
      [original, "Enter"],
      ["cancelled", "Escape"],
      ["cancelled", "blur"],
    ]) {
      await menu(page, entry, "Rename…");
      const input = page.getByRole("textbox", {
        name: "Rename name",
        exact: true,
      });
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(input).toBeFocused();
      await expect(input).toHaveValue(original);
      await input.fill(name);
      if (finish === "blur")
        await page
          .getByRole("button", { name: "Refresh explorer", exact: true })
          .click();
      else await input.press(finish);
      await expect(input).toHaveCount(0);
      await expect(row).toBeVisible();
      if (finish !== "blur") await expect(row).toBeFocused();
    }
  }
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "file_operation",
      ),
    ),
  ).toHaveLength(0);
});

test("renames nested files and the project root inline while keeping the editor open", async ({
  page,
}) => {
  await setup(page, false);
  await page.getByRole("button", { name: "Expand src", exact: true }).click();
  await page.getByRole("button", { name: "main.ts", exact: true }).dblclick();
  await page.locator(".cm-content").fill("unsaved rename");
  for (const [entry, name, relative] of [
    ["main.ts", "renamed.ts", "src/main.ts"],
    ["Project folder project", "renamed-project", ""],
  ]) {
    await menu(page, entry, "Rename…");
    const input = page.getByRole("textbox", {
      name: "Rename name",
      exact: true,
    });
    await expect(input).toBeFocused();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await input.fill(name);
    await input.press("Enter");
    await expect(input).toHaveCount(0);
    await expect(page.locator(".cm-content")).toContainText("unsaved rename");
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) => call.command === "file_operation")
          .at(-1),
      ),
    ).toMatchObject({
      args: { root: "/project", relative, operation: { kind: "rename", name } },
    });
  }
  await expect(
    page.getByRole("button", {
      name: "Project folder renamed-project",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator(".editor-path")).toContainText("src/renamed.ts");
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+s");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.editorFiles[
            "/renamed-project/src/renamed.ts"
          ].content,
      ),
    )
    .toBe("unsaved rename");
});

test("a newer search cannot be replaced by a slower previous response", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.searchDelays.needle = 1000;
  });
  await page
    .getByRole("button", { name: "Search in project", exact: true })
    .click();
  const input = page.getByRole("textbox", { name: "Search in files" });
  await input.fill("needle");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls.some(
          (call: any) =>
            call.command === "search_project" && call.args.query === "needle",
        ),
      ),
    )
    .toBe(true);
  await input.fill("first");
  await expect(page.locator(".search-match")).toHaveCount(1);
  await page.waitForTimeout(1100);
  await expect(page.locator(".search-match")).toHaveCount(1);
  await expect(
    page.getByRole("button", {
      name: "src/main.ts, line 1, column 1",
      exact: true,
    }),
  ).toBeVisible();
});

test("moving a dirty file updates its buffer location and keeps failed operations reviewable", async ({
  page,
}) => {
  await setup(page);
  await page.getByRole("button", { name: "README.md", exact: true }).dblclick();
  await page.locator(".cm-content").fill("unsaved move");
  await menu(page, "README.md", "Cut");
  await menu(page, "src", "Paste");
  await expect(page.locator(".editor-path")).toContainText("src/README.md");
  await expect(page.locator(".cm-content")).toContainText("unsaved move");
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+s");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.editorFiles["/project/src/README.md"]
            .content,
      ),
    )
    .toBe("unsaved move");
  await page.emulateMedia({ colorScheme: "dark" });
  await menu(page, "src", "Rename…");
  await page.screenshot({
    path: test.info().outputPath("rename-folder-dark.png"),
  });
});

test("navigates and collapses results, applies search options and recovers from invalid regex", async ({
  page,
}) => {
  await setup(page);
  await page
    .getByRole("button", { name: "Search in project", exact: true })
    .click();
  const input = page.getByRole("textbox", { name: "Search in files" });
  await input.fill("needle");
  await expect(page.locator(".search-match")).toHaveCount(3);
  await input.press("ArrowDown");
  const readme = page.getByRole("button", {
    name: "README.md, 1 result",
    exact: true,
  });
  await expect(readme).toBeFocused();
  await readme.press("ArrowLeft");
  await expect(readme).toHaveAttribute("aria-expanded", "false");
  await readme.press("ArrowRight");
  await readme.press("ArrowDown");
  const match = page.getByRole("button", {
    name: "README.md, line 2, column 1",
    exact: true,
  });
  await expect(match).toBeFocused();
  await match.press("Enter");
  await expect(page.locator(".cm-content")).toContainText("needle in readme");
  await expect(match).toHaveAttribute("aria-current", "true");
  await page
    .getByRole("button", { name: "Collapse all results", exact: true })
    .click();
  await expect(page.locator(".search-match")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Expand all results", exact: true })
    .click();
  await expect(page.locator(".search-match")).toHaveCount(3);
  await page.getByRole("button", { name: "Match case", exact: true }).click();
  await input.fill("NEEDLE");
  await expect(page.getByRole("status")).toContainText("No results");
  await page.getByRole("button", { name: "Match case", exact: true }).click();
  await expect(page.locator(".search-match")).toHaveCount(3);
  await page
    .getByRole("button", { name: "Match whole word", exact: true })
    .click();
  await input.fill("need");
  await expect(page.getByRole("status")).toContainText("No results");
  await page
    .getByRole("button", { name: "Use regular expression", exact: true })
    .click();
  await input.fill("ne{2}dle");
  await expect(page.locator(".search-match")).toHaveCount(3);
  await input.fill("[");
  await expect(page.getByRole("alert").filter({ hasText: /\S/ })).toContainText(
    "Invalid regular expression",
  );
  await expect(page.locator(".search-match")).toHaveCount(0);
  await input.fill("needle");
  await expect(page.locator(".search-match")).toHaveCount(3);
  await page
    .getByRole("button", { name: "Toggle search details", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Files to include", exact: true })
    .fill("*.ts, src/**");
  await page
    .getByRole("textbox", { name: "Files to exclude", exact: true })
    .fill("dist, **/*.test.ts");
  await page
    .getByRole("checkbox", { name: "Include ignored files", exact: true })
    .check();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "search_project")
            .at(-1)?.args.options,
      ),
    )
    .toEqual({
      caseSensitive: false,
      wholeWord: true,
      regex: true,
      include: "*.ts, src/**",
      exclude: "dist, **/*.test.ts",
      includeIgnored: true,
    });
  await page
    .getByRole("button", { name: "Toggle search details", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Back to Explorer", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Search in project", exact: true })
    .click();
  await expect(input).toHaveValue("needle");
  await expect(
    page.getByRole("button", { name: "Match whole word", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await page
    .getByRole("button", { name: "Toggle search details", exact: true })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Files to include", exact: true }),
  ).toHaveValue("*.ts, src/**");
  await page
    .getByRole("button", { name: "Clear search results", exact: true })
    .click();
  await expect(input).toHaveValue("");
  await expect(input).toBeFocused();
  await expect(page.locator(".search-match")).toHaveCount(0);
});

test("fits narrow sidebars in both appearances and keeps composition out of searches", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await setup(page);
  await page
    .getByRole("button", { name: "Search in project", exact: true })
    .click();
  const input = page.getByRole("textbox", { name: "Search in files" });
  await input.dispatchEvent("compositionstart");
  await input.fill("need");
  await page.waitForTimeout(350);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "search_project",
      ),
    ),
  ).toHaveLength(0);
  await input.fill("needle");
  await input.dispatchEvent("compositionend");
  await expect(page.locator(".search-match")).toHaveCount(3);
  await page
    .getByRole("button", { name: "Toggle search details", exact: true })
    .click();
  const divider = page.getByRole("separator", {
    name: "Resize sidebar",
    exact: true,
  });
  for (let step = 0; step < 6; step++) await divider.press("ArrowLeft");
  const panel = page.locator(".project-search");
  for (const appearance of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: appearance });
    await expect(page.locator(".search-match")).toHaveCount(3);
    const bounds = await panel.boundingBox();
    for (const control of await panel.locator("input, button").all()) {
      if (!(await control.isVisible())) continue;
      const box = (await control.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(bounds!.x);
      expect(box.x + box.width).toBeLessThanOrEqual(
        bounds!.x + bounds!.width + 1,
      );
    }
    await page.screenshot({
      path: test.info().outputPath(`search-narrow-${appearance}.png`),
    });
  }
});

test("nested repositories supply file decorations, history and ignore targets", async ({
  page,
}) => {
  await setup(page, false, [
    { relative: "first", directory: true },
    { relative: "first/file.txt", directory: false },
    { relative: "second", directory: true },
  ]);
  await page.evaluate(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = (command: string, args: any) => {
      if (command === "git_repositories")
        return Promise.resolve({
          repositories: ["first", "second"].map((name) => ({
            root: `/project/${name}`,
            branch: "main",
            changes: [
              {
                path: "file.txt",
                index: " ",
                worktree: "M",
                originalPath: null,
              },
            ],
          })),
          errors: [],
          limited: false,
        });
      return invoke(command, args);
    };
    window.dispatchEvent(new Event("focus"));
  });
  const first = page.getByRole("button", { name: "first", exact: true });
  await expect(first).toHaveAttribute("data-git-status", "M");
  await page.getByRole("button", { name: "Expand first", exact: true }).click();
  const file = page.getByRole("button", { name: "file.txt", exact: true });
  await expect(file).toHaveAttribute("data-git-status", "M");
  await menu(page, "file.txt", "Add to .gitignore");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "ignore_project_item")
            .at(-1)?.args,
      ),
    )
    .toEqual({ root: "/project/first", relative: "file.txt", local: false });
  await menu(page, "file.txt", "View History");
  await expect(
    page.getByRole("dialog", { name: "Git History · file.txt" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const args = (window as any).__nativeTest.calls
          .filter((call: any) => call.command === "git_history")
          .at(-1)?.args;
        return args && { root: args.root, path: args.path };
      }),
    )
    .toEqual({ root: "/project/first", path: "file.txt" });
});

async function nativeExplorerDrag(
  page: Page,
  type: "enter" | "over" | "drop" | "leave",
  target?: import("@playwright/test").Locator,
  paths: string[] = [],
  offset = { x: 0, y: 0 },
) {
  const box = target ? await target.boundingBox() : null;
  await page.evaluate(
    async ({ type, box, paths, offset }) => {
      const position = box
        ? {
            x: (box.x + box.width / 2 + offset.x) * devicePixelRatio,
            y: (box.y + box.height / 2 + offset.y) * devicePixelRatio,
          }
        : undefined;
      await (window as any).__nativeTest.emitEvent(`tauri://drag-${type}`, {
        paths,
        position,
      });
    },
    { type, box, paths, offset },
  );
}

async function importCalls(page: Page) {
  return page.evaluate(() =>
    (window as any).__nativeTest.calls.filter(
      (call: any) =>
        call.command === "file_operation" &&
        call.args.operation.kind === "copy",
    ),
  );
}

async function seedImports(page: Page) {
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.externalEntries = [
      { relative: "notes with spaces.txt", directory: false },
      { relative: "second.txt", directory: false },
      { relative: "README.md", directory: false },
    ];
    native.editorFiles["/outside folder/notes with spaces.txt"] = {
      content: "external source stays intact",
    };
  });
}

test("native Explorer drops copy multiple paths into folders and retain sources", async ({
  page,
}, testInfo) => {
  await setup(page);
  await seedImports(page);
  const folder = page.locator('[data-explorer-folder="src"]');
  await nativeExplorerDrag(page, "enter", folder);
  await expect(folder).toHaveAttribute("data-explorer-drop-target", "true");
  await page.screenshot({
    path: testInfo.outputPath("explorer-drop-folder.png"),
  });
  await nativeExplorerDrag(page, "drop", folder, [
    "/outside folder/notes with spaces.txt",
    "/outside folder/second.txt",
  ]);
  await expect(
    page.getByRole("button", { name: "notes with spaces.txt", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "second.txt", exact: true }),
  ).toBeVisible();
  const calls = await importCalls(page);
  expect(calls.map((call: any) => call.args)).toEqual([
    {
      root: "/project",
      relative: "src",
      operation: {
        kind: "copy",
        sourceRoot: "/outside folder/",
        source: "notes with spaces.txt",
      },
    },
    {
      root: "/project",
      relative: "src",
      operation: {
        kind: "copy",
        sourceRoot: "/outside folder/",
        source: "second.txt",
      },
    },
  ]);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.editorFiles[
          "/outside folder/notes with spaces.txt"
        ].content,
    ),
  ).toBe("external source stays intact");
  await expect(folder).not.toHaveAttribute("data-explorer-drop-target");
});

test("native Explorer drops resolve files, empty children, root whitespace and heading", async ({
  page,
}) => {
  await setup(page, false, [
    { relative: "src", directory: true },
    { relative: "src/main.ts", directory: false },
    { relative: "empty", directory: true },
  ]);
  await page.getByRole("button", { name: "Expand src", exact: true }).click();
  const file = page.getByRole("button", { name: "main.ts", exact: true });
  await nativeExplorerDrag(page, "drop", file, ["/first.txt"]);
  await expect.poll(async () => (await importCalls(page)).length).toBe(1);
  await page.getByRole("button", { name: "Expand empty", exact: true }).click();
  const empty = page.locator('.tree-message[data-explorer-directory="empty"]');
  await expect(empty).toHaveText("Empty folder");
  await nativeExplorerDrag(page, "drop", empty, ["/second.txt"]);
  await expect.poll(async () => (await importCalls(page)).length).toBe(2);
  const tree = page.locator(".file-tree");
  await nativeExplorerDrag(page, "drop", tree, ["/third.txt"]);
  await expect.poll(async () => (await importCalls(page)).length).toBe(3);
  await nativeExplorerDrag(
    page,
    "drop",
    page.getByRole("button", { name: "Project folder project", exact: true }),
    ["/fourth.txt"],
  );
  await expect.poll(async () => (await importCalls(page)).length).toBe(4);
  expect(
    (await importCalls(page)).map((call: any) => call.args.relative),
  ).toEqual(["src", "empty", "", ""]);
  expect((await importCalls(page))[0].args.operation.sourceRoot).toBe("/");
});

test("native Explorer spring expansion requires a stationary second and cancels outside", async ({
  page,
}) => {
  await setup(page);
  await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  await page.clock.pauseAt(new Date("2026-01-01T00:00:01Z"));
  const folder = page.locator('[data-explorer-folder="src"]');
  await nativeExplorerDrag(page, "enter", folder);
  await page.clock.runFor(600);
  await nativeExplorerDrag(page, "over", folder, [], { x: 1, y: 0 });
  await page.clock.runFor(600);
  await expect(
    page.getByRole("button", { name: "Expand src", exact: true }),
  ).toBeVisible();
  await nativeExplorerDrag(page, "over", folder, [], { x: 1, y: 0 });
  await page.clock.runFor(400);
  await expect(
    page.getByRole("button", { name: "Collapse src", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Collapse src", exact: true }).click();
  for (const target of [
    page.getByRole("button", { name: "Refresh explorer", exact: true }),
    page.locator(".terminal-pane").first(),
  ]) {
    await nativeExplorerDrag(page, "enter", folder);
    await page.clock.runFor(600);
    await nativeExplorerDrag(page, "over", target);
    await page.clock.runFor(1000);
    await expect(
      page.getByRole("button", { name: "Expand src", exact: true }),
    ).toBeVisible();
    await expect(folder).not.toHaveAttribute("data-explorer-drop-target");
  }
  await nativeExplorerDrag(page, "enter", folder);
  await nativeExplorerDrag(page, "leave");
  await page.clock.runFor(1000);
  await expect(
    page.getByRole("button", { name: "Expand src", exact: true }),
  ).toBeVisible();
});

test("native Explorer imports keep partial success and reject overlapping drops", async ({
  page,
}) => {
  await setup(page);
  await seedImports(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.operationDelay = 100;
  });
  const project = page.getByRole("button", {
    name: "Project folder project",
    exact: true,
  });
  await nativeExplorerDrag(page, "drop", project, [
    "/outside folder/README.md",
    "/outside folder/second.txt",
  ]);
  await expect(page.locator("[data-explorer-root]")).toHaveAttribute(
    "aria-busy",
    "true",
  );
  await nativeExplorerDrag(page, "drop", project, [
    "/outside folder/notes with spaces.txt",
  ]);
  await expect.poll(async () => (await importCalls(page)).length).toBe(2);
  await expect(
    page.getByRole("button", { name: "second.txt", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("A file or folder with that name already exists.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator("[data-explorer-root]")).toHaveAttribute(
    "aria-busy",
    "false",
  );
  expect(
    (await importCalls(page)).map((call: any) => call.args.operation.source),
  ).toEqual(["README.md", "second.txt"]);
});

test("native Explorer Windows imports preserve a drive root", async ({
  page,
}) => {
  await setup(page, false, undefined, false, "windows");
  await nativeExplorerDrag(
    page,
    "drop",
    page.getByRole("button", { name: "Project folder project", exact: true }),
    ["C:\\notes with spaces.txt"],
  );
  await expect.poll(async () => (await importCalls(page)).length).toBe(1);
  expect((await importCalls(page))[0].args.operation).toEqual({
    kind: "copy",
    sourceRoot: "C:/",
    source: "notes with spaces.txt",
  });
});

test("native Explorer search hides the tree and cancels pending expansion", async ({
  page,
}) => {
  await setup(page);
  await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  await page.clock.pauseAt(new Date("2026-01-01T00:00:01Z"));
  await nativeExplorerDrag(
    page,
    "enter",
    page.locator('[data-explorer-folder="src"]'),
  );
  await page.clock.runFor(500);
  await page
    .getByRole("button", { name: "Search in project", exact: true })
    .click();
  await page.clock.runFor(1000);
  await page
    .getByRole("button", { name: "Back to Explorer", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Expand src", exact: true }),
  ).toBeVisible();
});

for (const platform of ["linux", "macos", "windows"] as const) {
  test(`native Explorer routes ${platform} scaled and zoomed positions`, async ({
    page,
  }) => {
    await page.addInitScript((platform) => {
      localStorage.setItem("lomi.zoom.main", "150");
      Object.defineProperty(window, "devicePixelRatio", {
        value: platform === "windows" ? 3 : 2,
      });
    }, platform);
    await setup(page, false, undefined, false, platform);
    await expect
      .poll(() => page.evaluate(() => (window as any).__nativeTest.zoom))
      .toBe(1.5);
    const folder = page.locator('[data-explorer-folder="src"]');
    const box = (await folder.boundingBox())!;
    const factor = platform === "windows" ? 3 : 1.5;
    await page.evaluate(
      async ({ box, factor }) => {
        await (window as any).__nativeTest.emitEvent("tauri://drag-drop", {
          paths: ["/external/scaled.txt"],
          position: {
            x: (box.x + box.width / 2) * factor,
            y: (box.y + box.height / 2) * factor,
          },
        });
      },
      { box, factor },
    );
    await expect.poll(async () => (await importCalls(page)).length).toBe(1);
    expect((await importCalls(page))[0].args.relative).toBe("src");
  });
}

test("native Explorer POSIX imports preserve backslashes in filenames", async ({
  page,
}) => {
  await setup(page, false, [
    { relative: "src", directory: true },
    { relative: "src/notes\\draft.txt", directory: false },
  ]);
  await nativeExplorerDrag(
    page,
    "drop",
    page.getByRole("button", { name: "Project folder project", exact: true }),
    ["/outside folder/notes\\draft.txt"],
  );
  await expect.poll(async () => (await importCalls(page)).length).toBe(1);
  expect((await importCalls(page))[0].args.operation).toEqual({
    kind: "copy",
    sourceRoot: "/outside folder/",
    source: "notes\\draft.txt",
  });
  await page.getByRole("button", { name: "Expand src", exact: true }).click();
  await nativeExplorerDrag(
    page,
    "drop",
    page.getByRole("button", { name: "notes\\draft.txt", exact: true }),
    ["/outside/next.txt"],
  );
  await expect.poll(async () => (await importCalls(page)).length).toBe(2);
  expect((await importCalls(page))[1].args.relative).toBe("src");
});

test("native Explorer navigation cancels hover and retains accepted import destination", async ({
  page,
}) => {
  const project = newProject("/project", "local:bash");
  const other = newProject("/other", "local:bash");
  await setup(page, false, undefined, false, "linux", {
    ...newSession(),
    projects: [project, other],
    activeProjectId: project.id,
  });
  await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  await page.clock.pauseAt(new Date("2026-01-01T00:00:01Z"));
  await nativeExplorerDrag(
    page,
    "enter",
    page.locator('[data-explorer-folder="src"]'),
  );
  await page.clock.runFor(500);
  await page.locator(".project-switcher").click();
  await page.getByRole("menuitem", { name: "other", exact: true }).click();
  await page.clock.runFor(1000);
  await expect(
    page.getByRole("button", { name: "Expand src", exact: true }),
  ).toBeVisible();
  await seedImports(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.operationDelay = 2000;
  });
  await nativeExplorerDrag(
    page,
    "drop",
    page.locator('[data-explorer-folder="src"]'),
    ["/outside folder/second.txt", "/outside folder/notes with spaces.txt"],
  );
  await expect.poll(async () => (await importCalls(page)).length).toBe(1);
  await page.locator(".project-switcher").click();
  await page.getByRole("menuitem", { name: "project", exact: true }).click();
  await page.clock.runFor(4000);
  await expect(page.locator('[data-explorer-root="/project"]')).toHaveAttribute(
    "aria-busy",
    "false",
  );
  await expect(
    page.getByRole("button", { name: "Expand src", exact: true }),
  ).toBeVisible();
  expect(
    (await importCalls(page)).map((call: any) => ({
      root: call.args.root,
      relative: call.args.relative,
    })),
  ).toEqual([
    { root: "/other", relative: "src" },
    { root: "/other", relative: "src" },
  ]);
});

test("native Explorer spring expansion cancels after layout moves or unmount", async ({
  page,
}) => {
  await setup(page);
  await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  await page.clock.pauseAt(new Date("2026-01-01T00:00:01Z"));
  const folder = page.locator('[data-explorer-folder="src"]');
  await nativeExplorerDrag(page, "enter", folder);
  await folder.evaluate((element) => {
    (element as HTMLElement).style.transform = "translateY(120px)";
  });
  await page.clock.runFor(1000);
  await expect(
    page.getByRole("button", { name: "Expand src", exact: true }),
  ).toBeVisible();
  await folder.evaluate((element) => {
    (element as HTMLElement).style.transform = "";
  });
  await nativeExplorerDrag(page, "leave");
  await nativeExplorerDrag(page, "enter", folder);
  await page.clock.runFor(500);
  await page.getByRole("button", { name: /^Toggle file explorer/ }).click();
  await page.clock.runFor(1000);
  await page.getByRole("button", { name: /^Toggle file explorer/ }).click();
  await expect(
    page.getByRole("button", { name: "Expand src", exact: true }),
  ).toBeVisible();
});
