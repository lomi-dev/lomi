import { expect, test, type Page } from "@playwright/test";
import { newProject, newSession, splitPane } from "../../src/model";
import { mockDesktop } from "./desktop";
import { dragPreviewDivider, storedPreviewRatio } from "./preview-resize";

const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="160">
  <rect width="320" height="160" rx="24" fill="#737373"/>
  <text x="24" y="90" fill="#fff">Zażółć gęślą jaźń</text>
</svg>`;

async function setup(page: Page, saved?: unknown) {
  await mockDesktop(page, false, saved, undefined, {
    "/project/vector.SVG": {
      content: svg,
      revision: "initial",
      encoding: "utf8",
      readOnly: false,
    },
  });
  await page.addInitScript(() => {
    const bridge = (window as any).__TAURI_INTERNALS__;
    const invoke = bridge.invoke;
    bridge.invoke = async (command: string, args: any = {}) => {
      if (command === "list_directory") {
        const entries = await invoke(command, args);
        return args.relative
          ? entries
          : [
              ...entries,
              {
                name: "vector.SVG",
                relativePath: "vector.SVG",
                path: `${args.root}/vector.SVG`,
                isDirectory: false,
                isSymlink: false,
              },
            ];
      }
      return invoke(command, args);
    };
  });
  await page.goto("/");
}

async function openSvg(page: Page) {
  await page.getByRole("button", { name: "vector.SVG", exact: true }).click();
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect(page.locator(".image-canvas img")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Preview SVG", exact: true }),
  ).toBeVisible();
}

async function expectImageWidth(page: Page, width: number) {
  const image = page.locator(".image-canvas img");
  await expect(image).toBeVisible();
  await expect
    .poll(() => image.evaluate((node: HTMLImageElement) => node.naturalWidth))
    .toBe(width);
}

async function replaceText(page: Page, text: string) {
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText(text);
}

test("SVG switches between preview and code with live unsaved edits, undo, save and restoration", async ({
  page,
}, testInfo) => {
  const remote: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("example.com")) remote.push(request.url());
  });
  await setup(page);
  await openSvg(page);
  await expect(page.locator(".cm-content")).toBeFocused();
  await page.screenshot({
    path: testInfo.outputPath("svg-editor-default.png"),
  });
  const draft = svg
    .replace('width="320"', 'width="640"')
    .replace(
      "</svg>",
      '<script>window.__svgExecuted = true</script><image href="https://example.com/private.png" width="20" height="20"/></svg>',
    );
  await replaceText(page, draft);
  const editor = await page.locator(".cm-editor").elementHandle();
  await page.getByRole("button", { name: "Preview SVG", exact: true }).click();
  await expectImageWidth(page, 640);
  expect(
    await editor!.evaluate(
      (node) => node === document.querySelector(".cm-editor"),
    ),
  ).toBe(true);
  const sourceBounds = (await page.locator(".editor-host").boundingBox())!;
  const previewBounds = (await page.locator(".image-preview").boundingBox())!;
  expect(previewBounds.x).toBeGreaterThanOrEqual(
    sourceBounds.x + sourceBounds.width,
  );
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme });
    await page.screenshot({
      path: testInfo.outputPath(`svg-split-${colorScheme}.png`),
    });
  }
  await page
    .getByRole("button", { name: "Close SVG preview", exact: true })
    .click({ button: "right" });
  await page
    .getByRole("menuitemradio", { name: "Preview only", exact: true })
    .click();
  await expect(page.locator(".cm-editor")).toHaveCount(0);
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("tab", { name: /vector.SVG/ }).click();
  await expectImageWidth(page, 640);
  await page.keyboard.press("Control+w");
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page
    .getByRole("button", { name: "Close SVG preview", exact: true })
    .click();
  await expect(page.locator(".cm-content")).toBeFocused();
  await page.keyboard.press("Control+z");
  await expect(page.locator(".cm-content")).not.toContainText('width="640"');
  await page.keyboard.press("Control+Shift+z");
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "save_editor_file",
      ),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("test-session")))
    .not.toContain("previewView");
  await page.reload();
  await expect(page.locator(".cm-content")).toContainText('width="640"');
  await expect(page.locator(".image-canvas img")).toHaveCount(0);
  await page.getByRole("button", { name: "Preview SVG", exact: true }).click();
  await expectImageWidth(page, 640);
  expect(remote).toEqual([]);
  expect(
    await page.evaluate(() => (window as any).__svgExecuted),
  ).toBeUndefined();
});

test("the SVG preview divider resizes the source and keeps the live image rendered", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await setup(page);
  await openSvg(page);
  const draft = svg.replace('width="320"', 'width="640"');
  await replaceText(page, draft);
  const editor = await page.locator(".cm-editor").elementHandle();
  await page.getByRole("button", { name: "Preview SVG", exact: true }).click();
  await expectImageWidth(page, 640);

  const divider = page.getByRole("separator", {
    name: "Resize preview",
  });
  const source = page.locator(".editor-host");
  const preview = page.locator(".image-preview");
  const originalSourceWidth = (await source.boundingBox())!.width;
  const originalPreviewWidth = (await preview.boundingBox())!.width;
  await dragPreviewDivider(page, divider, 70);
  await expect
    .poll(() => storedPreviewRatio(page, "vector.SVG"))
    .toBeGreaterThan(0.6);
  await expect
    .poll(async () => (await source.boundingBox())!.width)
    .toBeGreaterThan(originalSourceWidth + 30);
  await expect
    .poll(async () => (await preview.boundingBox())!.width)
    .toBeLessThan(originalPreviewWidth - 30);
  await expectImageWidth(page, 640);
  await expect(page.locator(".cm-content")).toContainText('width="640"');
  expect(
    await editor!.evaluate(
      (node) => node === document.querySelector(".cm-editor"),
    ),
  ).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("svg-resized-preview.png"),
  });
});

test("SVG preview recovers from invalid edits and follows external changes without losing dirty text", async ({
  page,
}) => {
  await setup(page);
  await openSvg(page);
  await page
    .getByRole("button", { name: "Preview SVG", exact: true })
    .click({ button: "right" });
  await page
    .getByRole("menuitemradio", { name: "Preview beside editor", exact: true })
    .click();
  await expect(page.locator(".cm-content")).toBeVisible();
  await replaceText(page, "<svg");
  await expect(page.getByRole("alert")).toContainText("damaged");
  await replaceText(page, svg.replace('width="320"', 'width="480"'));
  await expectImageWidth(page, 480);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.evaluate((text) => {
    const native = (window as any).__nativeTest;
    native.editorFiles["/project/vector.SVG"].content = text.replace(
      'width="320"',
      'width="800"',
    );
    native.editorFiles["/project/vector.SVG"].revision = "external";
    void native.emitEvent("editor-files-changed", ["/project/vector.SVG"]);
  }, svg);
  await expect(page.getByRole("alert")).toContainText(
    "Your edits are preserved",
  );
  await expectImageWidth(page, 480);
  await page
    .getByRole("button", { name: "Reload from disk…", exact: true })
    .click();
  await page.getByRole("button", { name: "Reload file", exact: true }).click();
  await expectImageWidth(page, 800);
  await page
    .getByRole("button", { name: "Close SVG preview", exact: true })
    .click({ button: "right" });
  await page
    .getByRole("menuitemradio", { name: "Preview only", exact: true })
    .click();
  await page.evaluate((text) => {
    const native = (window as any).__nativeTest;
    native.editorFiles["/project/vector.SVG"].content = text;
    native.editorFiles["/project/vector.SVG"].revision = "external-again";
    void native.emitEvent("editor-files-changed", ["/project/vector.SVG"]);
  }, svg);
  await expectImageWidth(page, 320);
  await expect(page.locator(".cm-editor")).toHaveCount(0);
});

test("SVG controls work with keyboard in a restored split pane at the minimum window size", async ({
  page,
}, testInfo) => {
  const project = newProject("/project", "local:bash");
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw Error("Expected terminal tab");
  tab.layout = splitPane(tab.layout, tab.activePaneId, "horizontal", {
    type: "file",
    id: "svg-pane",
    root: project.path,
    relative: "vector.SVG",
    title: "vector.SVG",
    previewView: "split",
  });
  tab.activePaneId = "svg-pane";
  await page.setViewportSize({ width: 800, height: 420 });
  await setup(page, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect(page.locator(".image-canvas img")).toBeVisible();
  const button = page.getByRole("button", {
    name: "Close SVG preview",
    exact: true,
  });
  const previewBounds = (await page.locator(".image-preview").boundingBox())!;
  for (const control of await page
    .locator(".image-preview .editor-actions button")
    .all()) {
    const bounds = (await control.boundingBox())!;
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(
      previewBounds.x + previewBounds.width,
    );
  }
  await page.screenshot({ path: testInfo.outputPath("svg-panel-split.png") });
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
  await button.focus();
  await page.keyboard.press("Shift+F10");
  const bounds = (await page
    .getByRole("menu", { name: "SVG preview options" })
    .boundingBox())!;
  expect(bounds.x).toBeGreaterThanOrEqual(0);
  expect(bounds.y).toBeGreaterThanOrEqual(0);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(800);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(420);
  await page.screenshot({ path: testInfo.outputPath("svg-panel-menu.png") });
  await page.keyboard.press("Escape");
  await expect(button).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.locator(".cm-content")).toBeFocused();
  await page.keyboard.press("Control+w");
  await expect(page.locator(".cm-editor")).toHaveCount(0);
  await expect(page.locator(".xterm-screen")).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "start_terminal",
      ),
    ),
  ).toHaveLength(1);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "close_terminal",
      ),
    ),
  ).toEqual([]);
});
