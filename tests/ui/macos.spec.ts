import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("macOS editor preserves multiline paste, Cmd+Z, Cmd+Shift+Z and save", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await page.goto("/");
  await page.getByRole("button", { name: "README.md", exact: true }).dblclick();
  const editor = page.locator(".cm-content");
  const before = await editor.textContent();
  const text = "First line — zażółć 🦀\nSecond line\n";
  await editor.focus();
  await page.keyboard.press("Meta+a");
  await editor.evaluate((element, text) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", text);
    element.dispatchEvent(
      new ClipboardEvent("paste", {
        clipboardData,
        bubbles: true,
        cancelable: true,
      }),
    );
  }, text);
  await expect(editor).toHaveText(text.replaceAll("\n", ""));
  await page.keyboard.press("Meta+z");
  await expect(editor).toHaveText(before!);
  await page.keyboard.press("Meta+Shift+z");
  await expect(editor).toHaveText(text.replaceAll("\n", ""));
  await page.keyboard.press("Meta+s");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.editorFiles["/project/README.md"]
            .content,
      ),
    )
    .toBe(text);
});

async function enableFileOperationMock(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    const bridge = (window as any).__TAURI_INTERNALS__;
    const invoke = bridge.invoke;
    bridge.invoke = async (command: string, args: any = {}) => {
      if (command !== "file_operation") return invoke(command, args);
      (window as any).__nativeTest.calls.push({
        command,
        args: structuredClone(args),
      });
      return { oldPath: null, newPath: null };
    };
  });
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

test("macOS Explorer routes native clipboard events to file operations", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await enableFileOperationMock(page);
  await page.goto("/");
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
  await expect(source.locator("..")).not.toHaveAttribute("data-cut", "true");
});

test("macOS Explorer uses Cmd+D for duplicate without splitting the terminal and F2 to rename", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await enableFileOperationMock(page);
  await page.goto("/");
  const tree = page.locator(".file-tree");
  const readme = tree.locator('.tree-entry[title="/project/README.md"]');
  await expect(page.locator(".terminal-pane")).toHaveCount(1);
  const terminalCount = await page.locator(".terminal-pane").count();

  await readme.focus();
  await page.keyboard.press("Meta+d");
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
  await expect(page.locator(".terminal-pane")).toHaveCount(terminalCount);

  await readme.focus();
  await page.keyboard.press("F2");
  const rename = tree.getByRole("textbox", { name: "Rename name" });
  await expect(rename).toHaveValue("README.md");
  await rename.press("Escape");
});

test("hidden settings cancel shortcut recording before being reused", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await page.goto("/?window=settings");
  const recorder = page.getByRole("button", {
    name: "Shortcut for New terminal",
    exact: true,
  });
  await recorder.click();
  await expect(recorder).toContainText("Press keys…");
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(recorder).not.toContainText("Press keys…");
  await page.keyboard.press("Meta+w");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls.some(
          (call: any) => call.command === "plugin:window|close",
        ),
      ),
    )
    .toBe(true);
});

test("Cmd+W closes macOS settings while shortcut recording keeps the key", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await page.goto("/?window=settings");
  const closed = () =>
    page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "plugin:window|close",
        ).length,
    );
  const recorder = page.getByRole("button", {
    name: "Shortcut for New terminal",
    exact: true,
  });
  await recorder.click();
  await recorder.press("Meta+w");
  await expect(page.getByRole("alert")).toContainText("already assigned");
  expect(await closed()).toBe(0);
  await recorder.press("Escape");
  await page.keyboard.press("Meta+w");
  await expect.poll(closed).toBe(1);
});

for (const settings of [false, true]) {
  test(`macOS ${settings ? "settings" : "workspace"} reclaims native-control space in fullscreen and restores it on exit`, async ({
    page,
  }, testInfo) => {
    await mockDesktop(page, true, undefined, undefined, {}, "macos");
    await page.addInitScript(() => {
      (window as any).__nativeTest.fullscreen = true;
    });
    await page.goto(settings ? "/?window=settings" : "/");
    const titlebar = page.locator(".titlebar");
    await expect(titlebar).toHaveCSS("padding-left", "8px");
    const setFullscreen = (fullscreen: boolean) =>
      page.evaluate(async (fullscreen) => {
        const desktop = (window as any).__nativeTest;
        desktop.fullscreen = fullscreen;
        await desktop.emitEvent("tauri://resize", {
          width: innerWidth,
          height: innerHeight,
        });
      }, fullscreen);
    await setFullscreen(false);
    await expect(titlebar).toHaveCSS("padding-left", "88px");
    await page.keyboard.press("Meta+Minus");
    await page.keyboard.press("Meta+Minus");
    await expect(titlebar).toHaveCSS("padding-left", "110px");
    await expect(titlebar).toHaveCSS("min-height", "55px");
    await setFullscreen(true);
    await expect(titlebar).toHaveCSS("padding-left", "8px");
    await expect(titlebar).toHaveCSS("height", "44px");
    const content = await titlebar
      .locator(":scope > :first-child")
      .boundingBox();
    expect(content!.x).toBeLessThan(20);
    await page.screenshot({
      path: testInfo.outputPath("macos-fullscreen.png"),
    });
    await setFullscreen(false);
    await expect(titlebar).toHaveCSS("padding-left", "110px");
    await expect(titlebar).toHaveCSS("min-height", "55px");
    await page.keyboard.press("Meta+Digit0");
    await expect(titlebar).toHaveCSS("padding-left", "88px");
    await page.keyboard.press("Meta+Equal");
    await page.keyboard.press("Meta+Equal");
    await expect(titlebar).toHaveCSS("padding-left", "88px");
    await expect(titlebar).toHaveCSS("min-height", "44px");
    const zoomedContent = await titlebar
      .locator(":scope > :first-child")
      .boundingBox();
    expect(zoomedContent!.x).toBeGreaterThanOrEqual(88);
  });

  test(`macOS ${settings ? "settings" : "workspace"} leaves room for native controls at minimum size`, async ({
    page,
  }, testInfo) => {
    await mockDesktop(page, true, undefined, undefined, {}, "macos");
    await page.setViewportSize({ width: settings ? 560 : 800, height: 420 });
    await page.goto(settings ? "/?window=settings" : "/");
    await expect(page.locator(".titlebar")).toBeVisible();
    await expect(page.locator(".window-controls")).toHaveCount(0);
    await expect(page.locator(".titlebar")).toHaveCSS("padding-left", "88px");
    const content = await page
      .locator(".titlebar > :first-child")
      .boundingBox();
    expect(content!.x).toBeGreaterThanOrEqual(88);
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBe(settings ? 560 : 800);
    await page.screenshot({ path: testInfo.outputPath("macos-minimum.png") });
  });
}

test("macOS panel shortcuts preserve PTYs and leave Control keys available to the shell", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Meta+d");
  await expect(page.locator(".terminal-pane")).toHaveCount(2);
  await page.keyboard.press("Meta+w");
  await expect(page.locator(".terminal-pane")).toHaveCount(1);
  await page.keyboard.press("Control+c");
  await page.keyboard.press("Control+d");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) => call.command === "write_terminal")
          .map((call: any) => call.args.data)
          .join(""),
      ),
    )
    .toContain("\x03\x04");
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some(
        (call: any) => call.command === "plugin:window|destroy",
      ),
    ),
  ).toBe(false);
  await page.keyboard.press("Control+Tab");
  await expect(page.locator(".terminal-overview")).toBeVisible();
  await page.keyboard.press("Control+Tab");
  await expect(page.locator(".xterm-screen")).toBeVisible();
});

test("macOS native Quit requests retain dirty editors on cancellation and save failure", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await page.goto("/");
  await page.getByRole("button", { name: "README.md", exact: true }).dblclick();
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Meta+a");
  await page.keyboard.insertText("Unsaved on macOS — Zażółć 🦀");
  await page.evaluate(() => {
    void (window as any).__nativeTest.emitEvent("tauri://close-requested");
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "hide_main_window",
          ).length,
      ),
    )
    .toBe(1);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".cm-content")).toContainText(
    "Unsaved on macOS — Zażółć 🦀",
  );
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter((call: any) =>
        [
          "save_editor_file",
          "close_terminal",
          "plugin:window|destroy",
        ].includes(call.command),
      ),
    ),
  ).toHaveLength(0);
  const requestQuit = () =>
    page.evaluate(() => {
      void (window as any).__nativeTest.emitEvent("lomi-quit-requested");
    });
  const dialog = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await requestQuit();
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.evaluate(() => {
    (window as any).__nativeTest.failFileSave = true;
  });
  await requestQuit();
  await dialog
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(dialog).toContainText("Disk is full");
  await expect(page.locator(".cm-content")).toContainText("Zażółć 🦀");
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some(
        (call: any) => call.command === "plugin:window|destroy",
      ),
    ),
  ).toBe(false);
  await page.evaluate(() => {
    (window as any).__nativeTest.failFileSave = false;
  });
  await dialog
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls.some(
          (call: any) => call.command === "plugin:window|destroy",
        ),
      ),
    )
    .toBe(true);
});

test("macOS Explorer supports Cmd+Backspace to Move to Trash and Opt+Cmd+Backspace to Delete Permanently", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await page.goto("/");
  const file = page.getByRole("button", { name: "README.md", exact: true });
  await file.focus();

  // Cmd+Backspace should trigger Move to Trash
  await page.keyboard.press("Meta+Backspace");
  const trashDialog = page.getByRole("dialog", { name: "Move to Trash" });
  await expect(trashDialog).toBeVisible();
  await trashDialog
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await expect(trashDialog).toHaveCount(0);

  // Opt+Cmd+Backspace should trigger Delete Permanently
  await file.focus();
  await page.keyboard.press("Alt+Meta+Backspace");
  const deleteDialog = page.getByRole("dialog", {
    name: "Delete Permanently",
  });
  await expect(deleteDialog).toBeVisible();
  await deleteDialog
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await expect(deleteDialog).toHaveCount(0);

  // Context menu should show macOS shortcuts
  await file.click({ button: "right" });
  const menu = page.locator(".menu");
  await expect(menu).toBeVisible();
  await expect(menu).toContainText("⌘⌫");
  await expect(menu).toContainText("⌥⌘⌫");
  await expect(menu).toContainText("⌘C");
  await expect(menu).toContainText("⌘V");
  await expect(menu).toContainText("⌘X");
});

test("macOS File Explorer uses Command+C, Command+X, and Command+V", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await page.addInitScript(() => {
    const bridge = (window as any).__TAURI_INTERNALS__;
    const native = (window as any).__nativeTest;
    const invoke = bridge.invoke;
    bridge.invoke = async (command: string, args: any = {}) => {
      if (command !== "file_operation") return invoke(command, args);
      native.calls.push({ command, args });
      const source = args.operation.source ?? args.relative;
      return {
        oldPath:
          args.operation.kind === "move"
            ? `${args.operation.sourceRoot ?? args.root}/${source}`
            : null,
        newPath: `${args.root}/${args.relative}/${source.split("/").at(-1)}`,
      };
    };
  });
  await page.goto("/");
  const file = page.locator(
    '.file-tree .tree-entry[title="/project/README.md"]',
  );
  const row = file.locator("..");

  await file.click();
  await page.keyboard.press("Control+x");
  await expect(row).not.toHaveAttribute("data-cut", "true");

  await page.keyboard.press("Meta+x");
  await expect(row).toHaveAttribute("data-cut", "true");
  await expect(row.locator(".tree-cut-mark")).toBeVisible();

  // Control+C is ignored on macOS; Command+C replaces the cut state.
  await page.keyboard.press("Control+c");
  await expect(row).toHaveAttribute("data-cut", "true");
  await page.keyboard.press("Meta+c");
  await expect(row).not.toHaveAttribute("data-cut", "true");

  const folder = page.locator('.file-tree .tree-entry[title="/project/src"]');

  // Control+V is ignored on macOS, while Command+V copies into another folder.
  await folder.focus();
  await page.keyboard.press("Control+v");
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
  await page.keyboard.press("Meta+v");
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

  // A cut pasted into another folder moves the source and clears its mark.
  await file.focus();
  await page.keyboard.press("Meta+x");
  await expect(row).toHaveAttribute("data-cut", "true");
  await folder.focus();
  await page.keyboard.press("Meta+v");
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
        kind: "move",
        sourceRoot: "/project",
        source: "README.md",
      },
    });
  await expect(row).not.toHaveAttribute("data-cut", "true");
});
