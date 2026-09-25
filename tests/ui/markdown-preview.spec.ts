import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import {
  active,
  fileTabs,
  mergeTabs,
  newProject,
  newSession,
  openFileTab,
} from "../../src/model";
import { dragPreviewDivider, storedPreviewRatio } from "./preview-resize";

const markdown = [
  "# Lomi",
  "",
  "A **live preview** with *Markdown* and `code`.",
  "",
  "## Checklist",
  "- [x] Edit the document",
  "- [ ] Save when ready",
  "",
  "| Feature | Status |",
  "| --- | --- |",
  "| Preview | Live |",
  "",
  "> Zażółć gęślą jaźń.",
  "",
  "```ts",
  'const message = "Hello";',
  "```",
].join("\n");

async function openReadme(page: Page) {
  await mockDesktop(page, false);
  await page.goto("/");
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  await expect(page.locator(".cm-content")).toBeVisible();
}

async function replaceText(page: Page, text: string) {
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText(text);
}

test("the corner button opens a live split preview of unsaved Markdown without replacing the editor", async ({
  page,
}, testInfo) => {
  await openReadme(page);
  expect(
    await page.evaluate(() =>
      performance
        .getEntriesByType("resource")
        .some((entry) => entry.name.includes("/src/MarkdownPreview.tsx")),
    ),
  ).toBe(false);
  await replaceText(page, markdown);
  const editor = await page.locator(".cm-editor").elementHandle();
  await page
    .getByRole("button", { name: "Preview Markdown", exact: true })
    .click();
  const preview = page.getByRole("region", {
    name: "Markdown preview for README.md",
  });
  await expect(preview.getByRole("heading", { name: "Lomi" })).toBeVisible();
  await expect(preview.getByRole("table")).toBeVisible();
  await expect(preview.getByRole("checkbox").first()).toBeChecked();
  await expect(preview.getByRole("checkbox").first()).toBeDisabled();
  await expect(preview.locator("pre code")).toContainText(
    'const message = "Hello";',
  );
  expect(
    await editor!.evaluate(
      (node) => node === document.querySelector(".cm-editor"),
    ),
  ).toBe(true);
  const sourceBounds = await page.locator(".editor-host").boundingBox();
  const previewBounds = await preview.boundingBox();
  expect(previewBounds!.x).toBeGreaterThanOrEqual(
    sourceBounds!.x + sourceBounds!.width,
  );
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+End");
  await page.keyboard.insertText("\n\n## Unsaved section");
  await expect(
    preview.getByRole("heading", { name: "Unsaved section" }),
  ).toBeVisible();
  await page.evaluate(async () => {
    const { fileTabs } = await import("/src/model.ts");
    const { loadedEditor } = await import("/src/editor-service.ts");
    const file = fileTabs(JSON.parse(localStorage.getItem("test-session")!))[0];
    const document = loadedEditor(file)!;
    (document as any).view.dispatch({
      changes: {
        from: 2,
        to: document.state.doc.line(1).to,
        insert: "ChangedName",
      },
      selection: document.state.selection,
    });
  });
  await expect(
    preview.getByRole("heading", { name: "ChangedName" }),
  ).toBeVisible();
  await expect(preview.locator("#changedname")).toHaveCount(1);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({
    path: testInfo.outputPath("markdown-split-dark.png"),
  });
  await page.emulateMedia({ colorScheme: "light" });
  await page.screenshot({
    path: testInfo.outputPath("markdown-split-light.png"),
  });
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "save_editor_file",
      ),
    ),
  ).toEqual([]);
});

test("the Markdown preview divider resizes the live split and supports keyboard bounds and reset", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await openReadme(page);
  await replaceText(
    page,
    "# Resized preview\n\nUnsaved Markdown remains live.",
  );
  const editor = await page.locator(".cm-editor").elementHandle();
  await page
    .getByRole("button", { name: "Preview Markdown", exact: true })
    .click();
  const preview = page.getByRole("region", {
    name: "Markdown preview for README.md",
  });
  await expect(
    preview.getByRole("heading", { name: "Resized preview" }),
  ).toBeVisible();
  const divider = page.getByRole("separator", {
    name: "Resize preview",
  });
  await expect(divider).toHaveCount(1);
  await expect(page.locator(".split-divider.file-preview-divider")).toHaveCount(
    1,
  );
  const source = page.locator(".editor-host");
  const markdownPreview = page.locator(".markdown-preview");
  const originalSourceWidth = (await source.boundingBox())!.width;
  const originalPreviewWidth = (await markdownPreview.boundingBox())!.width;
  await dragPreviewDivider(page, divider, 80);
  await expect
    .poll(() => storedPreviewRatio(page, "README.md"))
    .toBeGreaterThan(0.6);
  await expect
    .poll(async () => (await source.boundingBox())!.width)
    .toBeGreaterThan(originalSourceWidth + 30);
  await expect
    .poll(async () => (await markdownPreview.boundingBox())!.width)
    .toBeLessThan(originalPreviewWidth - 30);
  expect(
    await editor!.evaluate(
      (node) => node === document.querySelector(".cm-editor"),
    ),
  ).toBe(true);
  await expect(page.locator(".cm-content")).toContainText("Unsaved Markdown");
  await expect(
    preview.getByRole("heading", { name: "Resized preview" }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("markdown-resized-preview.png"),
  });

  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+z");
  await expect(page.locator(".cm-content")).toContainText(
    "A text file preview.",
  );
  await expect(preview.getByRole("heading", { name: "Project" })).toBeVisible();
  await page.keyboard.press("Control+Shift+z");
  await expect(
    preview.getByRole("heading", { name: "Resized preview" }),
  ).toBeVisible();

  await divider.focus();
  const draggedRatio = (await storedPreviewRatio(page, "README.md"))!;
  await page.keyboard.press("ArrowRight");
  await expect
    .poll(() => storedPreviewRatio(page, "README.md"))
    .toBeCloseTo(draggedRatio + 0.05, 2);
  await page.keyboard.press("End");
  await expect.poll(() => storedPreviewRatio(page, "README.md")).toBe(0.9);
  await page.keyboard.press("Home");
  await expect.poll(() => storedPreviewRatio(page, "README.md")).toBe(0.1);
  await divider.dblclick();
  await expect.poll(() => storedPreviewRatio(page, "README.md")).toBe(0.5);
});

test("preview-only mode survives tab switches and restoration, and returning to code preserves edits and undo", async ({
  page,
}) => {
  await openReadme(page);
  await replaceText(page, "# Unsaved draft");
  await page
    .getByRole("button", { name: "Preview Markdown", exact: true })
    .click();
  const divider = page.getByRole("separator", {
    name: "Resize preview",
  });
  await dragPreviewDivider(page, divider, 60);
  await expect
    .poll(() => storedPreviewRatio(page, "README.md"))
    .toBeGreaterThan(0.55);
  const resizedRatio = (await storedPreviewRatio(page, "README.md"))!;
  await page
    .getByRole("button", { name: "Close Markdown preview", exact: true })
    .click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Markdown preview options" });
  await expect(menu.getByRole("menuitemradio")).toHaveCount(2);
  await menu
    .getByRole("menuitemradio", { name: "Preview only", exact: true })
    .click();
  await expect(page.locator(".cm-editor")).toHaveCount(0);
  await expect(divider).toHaveCount(0);
  const previewOnlyBounds = (await page
    .locator(".markdown-preview")
    .boundingBox())!;
  const previewContentBounds = (await page
    .locator(".editor-content")
    .boundingBox())!;
  expect(
    Math.abs(previewOnlyBounds.width - previewContentBounds.width),
  ).toBeLessThan(2);
  await expect(
    page.getByRole("heading", { name: "Unsaved draft" }),
  ).toBeVisible();
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("tab", { name: /README.md/ }).click();
  await expect(
    page.getByRole("heading", { name: "Unsaved draft" }),
  ).toBeVisible();
  await expect(page.locator(".cm-editor")).toHaveCount(0);
  await expect
    .poll(() => storedPreviewRatio(page, "README.md"))
    .toBeCloseTo(resizedRatio, 2);
  await page.keyboard.press("Control+w");
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Unsaved draft" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Close Markdown preview", exact: true })
    .click();
  await expect(page.locator(".cm-content")).toContainText("# Unsaved draft");
  await expect(divider).toHaveCount(0);
  const sourceBounds = (await page.locator(".editor-host").boundingBox())!;
  const codeContentBounds = (await page
    .locator(".editor-content")
    .boundingBox())!;
  expect(Math.abs(sourceBounds.width - codeContentBounds.width)).toBeLessThan(
    2,
  );
  await page.keyboard.press("Control+z");
  await expect(page.locator(".cm-content")).toContainText(
    "A text file preview.",
  );
  await page.keyboard.press("Control+Shift+z");
  await page
    .getByRole("button", { name: "Preview Markdown", exact: true })
    .click();
  await expect(divider).toBeVisible();
  await expect
    .poll(() => storedPreviewRatio(page, "README.md"))
    .toBeCloseTo(resizedRatio, 2);
  await page
    .getByRole("button", { name: "Close Markdown preview", exact: true })
    .click({ button: "right" });
  await page
    .getByRole("menuitemradio", { name: "Preview only", exact: true })
    .click();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("test-session")))
    .toContain('"previewView":"preview"');
  await expect
    .poll(() => storedPreviewRatio(page, "README.md"))
    .toBeCloseTo(resizedRatio, 2);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const workspace = JSON.parse(
          localStorage.getItem("test-session") ?? "{}",
        ).projects?.[0].workspaces[0];
        return workspace?.tabs.find(
          (tab: any) => tab.id === workspace.activeTabId,
        )?.relative;
      }),
    )
    .toBe("README.md");
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Unsaved draft" }),
  ).toBeVisible();
  await expect(page.locator(".cm-editor")).toHaveCount(0);
  await expect
    .poll(() => storedPreviewRatio(page, "README.md"))
    .toBeCloseTo(resizedRatio, 2);
  await page
    .getByRole("button", { name: "Close Markdown preview", exact: true })
    .click();
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("test-session")))
    .not.toContain("previewView");
  await page
    .getByRole("button", { name: "Preview Markdown", exact: true })
    .click();
  await expect(divider).toBeVisible();
  await expect
    .poll(() => storedPreviewRatio(page, "README.md"))
    .toBeCloseTo(resizedRatio, 2);
  const restoredSourceWidth = (await page
    .locator(".editor-host")
    .boundingBox())!.width;
  const restoredPreviewWidth = (await page
    .locator(".markdown-preview")
    .boundingBox())!.width;
  expect(
    Math.abs(
      restoredSourceWidth / (restoredSourceWidth + restoredPreviewWidth) -
        resizedRatio,
    ),
  ).toBeLessThan(0.03);
  await page
    .getByRole("button", { name: "Close Markdown preview", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("test-session")))
    .not.toContain("previewView");
});

test("preview controls remain clickable at the minimum window size and support the keyboard", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await openReadme(page);
  const button = page.getByRole("button", {
    name: "Preview Markdown",
    exact: true,
  });
  await button.focus();
  await page.keyboard.press("Shift+F10");
  await expect(
    page.getByRole("menuitemradio", {
      name: "Preview beside editor",
      exact: true,
    }),
  ).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(page.locator(".cm-editor")).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "Project", exact: true }),
  ).toBeVisible();
  const close = page.getByRole("button", {
    name: "Close Markdown preview",
    exact: true,
  });
  await close.click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Markdown preview options" });
  const bounds = await menu.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.y).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(800);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(420);
  await page.screenshot({
    path: testInfo.outputPath("markdown-preview-menu.png"),
  });
  await page.keyboard.press("Escape");
  await expect(close).toBeFocused();
  await close.click();
  await expect(page.locator(".cm-content")).toBeVisible();
});

test("preview links and local images use scoped native access without running HTML or loading remote images", async ({
  page,
}) => {
  const remote: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("example.com")) remote.push(request.url());
  });
  await mockDesktop(page, false);
  await page.goto("/");
  await page.evaluate(() => {
    const invoke = (window as any).__TAURI_INTERNALS__.invoke;
    (window as any).__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: any,
    ) => {
      if (command === "read_markdown_image") {
        (window as any).__nativeTest.calls.push({ command, args });
        return new TextEncoder().encode(
          '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#787878"/></svg>',
        ).buffer;
      }
      return invoke(command, args);
    };
  });
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  await replaceText(
    page,
    "# Safe\n\n<script>window.markdownExecuted = true</script>\n\n[Unsafe](javascript:alert(1))\n\n[Website](https://example.com/docs)\n\n[Other file](docs/guide.md)\n\n![Local diagram](images/diagram.svg)\n\n![Remote diagram](https://example.com/private.png)",
  );
  await page
    .getByRole("button", { name: "Preview Markdown", exact: true })
    .click();
  const preview = page.locator(".markdown-preview");
  await expect(
    preview.getByRole("img", { name: "Local diagram" }),
  ).toBeVisible();
  expect(
    await preview
      .getByRole("img", { name: "Local diagram" })
      .evaluate((image) => (image as HTMLImageElement).naturalWidth),
  ).toBe(40);
  await expect(
    preview.getByRole("link", { name: "Unsafe", exact: true }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).markdownExecuted),
  ).toBeUndefined();
  await expect(preview.locator("script, iframe")).toHaveCount(0);
  expect(remote).toEqual([]);
  const before = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "read_markdown_image",
      ).length,
  );
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+End");
  await page.keyboard.insertText("\n\nNew text");
  await expect(preview).toContainText("New text");
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "read_markdown_image",
        ).length,
    ),
  ).toBe(before);
  await preview.getByRole("link", { name: "Website", exact: true }).click();
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.find(
          (call: any) => call.command === "plugin:opener|open_url",
        )?.args.url,
    ),
  ).toBe("https://example.com/docs");
  await preview.getByRole("link", { name: "Other file", exact: true }).click();
  await expect(
    page.getByRole("tab", { name: "guide.md", exact: true }),
  ).toBeVisible();
  expect(new URL(page.url()).pathname).toBe("/");
});

test("a clean preview refreshes after an external change and keeps the code editor hidden", async ({
  page,
}) => {
  await openReadme(page);
  await page
    .getByRole("button", { name: "Preview Markdown", exact: true })
    .click({ button: "right" });
  await page
    .getByRole("menuitemradio", { name: "Preview only", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Project", exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.editorFiles["/project/README.md"].content = "# Changed outside";
    native.editorFiles["/project/README.md"].revision = "external";
    void native.emitEvent("editor-files-changed", ["/project/README.md"]);
  });
  await expect(
    page.getByRole("heading", { name: "Changed outside" }),
  ).toBeVisible();
  await expect(page.locator(".cm-editor")).toHaveCount(0);
});

test("Markdown controls also work in a file panel without restarting its neighboring terminal", async ({
  page,
}) => {
  const project = newProject("/project", "local:bash");
  const workspace = project.workspaces[0];
  const terminal = workspace.tabs[0];
  const state = openFileTab(
    { ...newSession(), projects: [project], activeProjectId: project.id },
    workspace.id,
    project.path,
    "README.Md",
  );
  const file = fileTabs(state)[0];
  state.projects[0].workspaces[0] = mergeTabs(
    active(state)!.workspace,
    file.id,
    terminal.id,
    "right",
    { width: 1440, height: 900 },
  );
  await mockDesktop(page, false, state);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await replaceText(page, "# Panel preview");
  await page
    .getByRole("button", { name: "Preview Markdown", exact: true })
    .click();
  const fileEditor = page.locator(".file-editor");
  const fileContent = fileEditor.locator(".editor-content");
  const divider = fileEditor.getByRole("separator", {
    name: "Resize preview",
  });
  const source = fileEditor.locator(".editor-host");
  const preview = fileEditor.locator(".markdown-preview");
  const sourceWidth = (await source.boundingBox())!.width;
  const previewWidth = (await preview.boundingBox())!.width;
  await dragPreviewDivider(page, divider, 45);
  await expect
    .poll(() => storedPreviewRatio(page, "README.Md"))
    .toBeGreaterThan(0.52);
  await expect
    .poll(async () => (await source.boundingBox())!.width)
    .toBeGreaterThan(sourceWidth + 15);
  await expect
    .poll(async () => (await preview.boundingBox())!.width)
    .toBeLessThan(previewWidth - 15);
  await expect(
    page.getByRole("heading", { name: "Panel preview" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Close Markdown preview", exact: true })
    .click({ button: "right" });
  await page
    .getByRole("menuitemradio", { name: "Preview only", exact: true })
    .click();
  await expect(page.locator(".cm-editor")).toHaveCount(0);
  await expect(divider).toHaveCount(0);
  const previewOnlyBounds = (await preview.boundingBox())!;
  const previewContentBounds = (await fileContent.boundingBox())!;
  expect(
    Math.abs(previewOnlyBounds.width - previewContentBounds.width),
  ).toBeLessThan(2);
  await page
    .getByRole("button", { name: "Close Markdown preview", exact: true })
    .click();
  await expect(page.locator(".cm-content")).toBeFocused();
  await expect(divider).toHaveCount(0);
  const codeBounds = (await source.boundingBox())!;
  const codeContentBounds = (await fileContent.boundingBox())!;
  expect(Math.abs(codeBounds.width - codeContentBounds.width)).toBeLessThan(2);
  await page.keyboard.press("Control+z");
  await expect(page.locator(".cm-content")).not.toContainText("Panel preview");
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
  await page
    .getByRole("button", { name: "it's a file.txt", exact: true })
    .click();
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect(page.locator(".markdown-preview-toggle")).toHaveCount(0);
});
