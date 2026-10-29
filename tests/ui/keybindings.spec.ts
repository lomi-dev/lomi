import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function openTerminal(page: Page) {
  await mockDesktop(page, true, null);
  await page.goto("/");
  await page
    .getByRole("button", { name: "Open Recent Project", exact: true })
    .click();
  await page
    .getByRole("menuitem", { name: "Open Local Folder…", exact: true })
    .click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.locator(".xterm-helper-textarea").focus();
}

async function calls(page: Page, command: string) {
  return page.evaluate(
    (command) =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === command,
      ),
    command,
  );
}

async function input(page: Page) {
  return (await calls(page, "write_terminal"))
    .map((call: any) => call.args.data)
    .join("");
}

test("Shift+Enter reaches the CLI as a distinct key without submitting", async ({
  page,
}) => {
  await openTerminal(page);
  await page.keyboard.type("first line");
  await page.keyboard.press("Shift+Enter");
  await page.keyboard.type("second line");
  await page.keyboard.press("Enter");
  await expect
    .poll(() => input(page))
    .toBe("first line\x1b[13;2usecond line\r");

  await page.keyboard.press("Shift+NumpadEnter");
  await page.keyboard.press("Control+Enter");
  await page.keyboard.press("Alt+Enter");
  await page.keyboard.press("Control+Shift+Enter");
  await page.keyboard.press("Alt+Shift+Enter");
  await page.keyboard.down("Shift");
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await page.keyboard.up("Shift");
  const expected =
    "first line\x1b[13;2usecond line\r" +
    "\x1b[13;2u\r\x1b\r\r\x1b\r\x1b[13;2u\x1b[13;2u";
  await expect.poll(() => input(page)).toBe(expected);

  await page.locator(".xterm-helper-textarea").evaluate((textarea) => {
    textarea.dispatchEvent(new CompositionEvent("compositionstart"));
    for (const isComposing of [true, false]) {
      textarea.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          code: "Enter",
          shiftKey: true,
          keyCode: 229,
          isComposing,
          bubbles: true,
          cancelable: true,
        }),
      );
    }
    textarea.dispatchEvent(new CompositionEvent("compositionend"));
  });
  await page.keyboard.type("after composition");
  await expect.poll(() => input(page)).toBe(expected + "after composition");
});

test("Shift+Enter preserves form editing and modified Enter shortcuts", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-keybindings",
      JSON.stringify({
        version: 1,
        bindings: { newTerminal: "Ctrl+Shift+Enter" },
      }),
    );
  });
  await openTerminal(page);
  await page.keyboard.press("Control+Shift+i");
  const composer = page.getByRole("textbox", { name: "Command input" });
  await composer.fill("first line");
  await composer.press("Shift+Enter");
  await composer.pressSequentially("second line");
  await expect(composer).toHaveValue("first line\nsecond line");
  expect(await input(page)).toBe("");
  await page.getByRole("button", { name: "Close command input" }).click();

  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+Shift+Enter");
  await expect(page.locator("[data-pane-id]")).toHaveCount(2);
  expect(await input(page)).toBe("");
});

test("panel shortcuts capture control keys before xterm and keep the current tab", async ({
  page,
}) => {
  await openTerminal(page);
  await expect(page.locator(".tab-context, .pane-toolbar")).toHaveCount(0);
  const original = await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id");
  await page.keyboard.down("Control");
  await page.keyboard.down("d");
  await expect(page.locator("[data-pane-id]")).toHaveCount(2);
  const added = await page
    .locator(".terminal-pane.is-active")
    .getAttribute("data-pane-id");
  const addedSession = await page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    return runningTerminal(id!)!.sessionId;
  }, added);
  await page.keyboard.down("d");
  await page.keyboard.up("d");
  await page.keyboard.up("Control");
  await expect(page.locator("[data-pane-id]")).toHaveCount(2);
  await expect(page.getByRole("tab")).toHaveCount(1);
  await page.keyboard.press("Control+w");
  await expect(page.locator("[data-pane-id]")).toHaveCount(1);
  await expect(page.locator("[data-pane-id]")).toHaveAttribute(
    "data-pane-id",
    original!,
  );
  await expect
    .poll(async () =>
      (await calls(page, "close_terminal")).some(
        (call: any) => call.args.id === addedSession,
      ),
    )
    .toBe(true);
  await page.keyboard.press("Control+c");
  await expect.poll(() => input(page)).toContain("\x03");
  expect(await input(page)).not.toMatch(/[\x04\x17]/);
  await page.keyboard.press("Control+Shift+v");
  await expect.poll(() => input(page)).toContain("clipboard text");
  await page.keyboard.press("Control+Shift+d");
  await expect(
    page.getByRole("separator", { name: "Resize terminal rows" }),
  ).toBeVisible();
  await page.screenshot({ path: "test-results/terminal-clean-split.png" });
});

test("keybindings update another window immediately and survive reloading", async ({
  page,
  context,
}) => {
  await openTerminal(page);
  const settings = await context.newPage();
  await mockDesktop(settings);
  await settings.goto("/?window=settings");
  const recorder = settings.getByRole("button", {
    name: "Shortcut for New terminal",
    exact: true,
  });
  await expect(recorder).toHaveText("Ctrl+D");
  const loads = (await calls(page, "load_keybindings")).length;
  await recorder.click();
  await recorder.press("Control+k");
  await expect(recorder).toHaveText("Ctrl+K");
  await expect
    .poll(async () => (await calls(page, "load_keybindings")).length)
    .toBeGreaterThan(loads);
  expect((await calls(page, "start_terminal")).length).toBe(1);
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+k");
  await expect(page.locator("[data-pane-id]")).toHaveCount(2);
  await page.keyboard.press("Control+w");
  await expect(page.locator("[data-pane-id]")).toHaveCount(1);
  await page.keyboard.press("Control+d");
  await expect.poll(() => input(page)).toContain("\x04");
  await expect(page.locator("[data-pane-id]")).toHaveCount(1);
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("test-session")))
    .toContain("activePaneId");
  await settings.reload();
  await expect(recorder).toHaveText("Ctrl+K");
  await page.reload();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+k");
  await expect(page.locator("[data-pane-id]")).toHaveCount(2);
  await settings
    .getByRole("button", {
      name: "Reset shortcut for New terminal",
      exact: true,
    })
    .click();
  await expect(recorder).toHaveText("Ctrl+D");
  await page.locator(".terminal-pane.is-active .xterm-helper-textarea").focus();
  await page.keyboard.press("Control+d");
  await expect(page.locator("[data-pane-id]")).toHaveCount(3);
});

test("Explorer click mode persists and syncs without changing other preferences", async ({
  page,
  context,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-keybindings",
      JSON.stringify({
        version: 1,
        bindings: { newTerminal: "Ctrl+KeyK" },
        focusFollowsPointer: true,
      }),
    );
  });
  await openTerminal(page);
  const settings = await context.newPage();
  await mockDesktop(settings);
  await settings.goto("/?window=settings");

  const mode = settings.getByRole("switch", {
    name: "Open with one click",
  });
  await expect(mode).not.toBeChecked();
  const loads = (await calls(page, "load_keybindings")).length;
  await mode.click();
  await expect(mode).toBeChecked();
  await expect
    .poll(async () => (await calls(page, "load_keybindings")).length)
    .toBeGreaterThan(loads);
  expect(
    await settings.evaluate(() =>
      JSON.parse(localStorage.getItem("test-keybindings")!),
    ),
  ).toMatchObject({
    bindings: { newTerminal: "Ctrl+KeyK" },
    focusFollowsPointer: true,
    explorerOpenOnSingleClick: true,
  });

  const readme = page.locator('.file-tree .tree-entry[title$="/README.md"]');
  const editorPath = page.locator(".editor-path");
  await expect(readme).toBeVisible();
  await readme.click();
  await page.waitForTimeout(100);
  await expect(editorPath).toContainText("README.md");

  await settings.reload();
  await expect(mode).toBeChecked();
  const loadsBeforeChange = (await calls(page, "load_keybindings")).length;
  await mode.click();
  await expect(mode).not.toBeChecked();
  await expect
    .poll(async () => (await calls(page, "load_keybindings")).length)
    .toBeGreaterThan(loadsBeforeChange);
  expect(
    await settings.evaluate(() =>
      JSON.parse(localStorage.getItem("test-keybindings")!),
    ),
  ).toMatchObject({
    bindings: { newTerminal: "Ctrl+KeyK" },
    focusFollowsPointer: true,
    explorerOpenOnSingleClick: false,
  });

  await page
    .getByRole("button", { name: "Close README.md", exact: true })
    .click();
  await expect(editorPath).toHaveCount(0);
  const readmeRow = readme.locator("..");
  await readme.click();
  await expect(readmeRow).toHaveAttribute("data-selected", "true");
  await expect(editorPath).toHaveCount(0);
  await readme.dblclick();
  await expect(editorPath).toContainText("README.md");
});

test("recording rejects conflicts, supports clearing and preserves bindings after a failed save", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/?window=settings");
  await expect(
    page.locator(".keybindings-page .settings-page-actions"),
  ).toHaveCount(0);
  const recorder = page.getByRole("button", {
    name: "Shortcut for New terminal",
    exact: true,
  });
  await recorder.click();
  await recorder.press("Control+w");
  await expect(page.getByRole("alert")).toContainText(
    "already assigned to “Close terminal”",
  );
  expect(await calls(page, "save_keybindings")).toHaveLength(0);
  await recorder.press("Escape");
  await expect(recorder).toHaveText("Ctrl+D");
  await page
    .getByRole("button", {
      name: "Clear shortcut for New terminal",
      exact: true,
    })
    .click();
  await expect(recorder).toHaveText("Not set");
  await page.reload();
  await expect(recorder).toHaveText("Not set");
  await page.evaluate(() => {
    (window as any).__nativeTest.failKeybindingsSave = true;
  });
  await recorder.click();
  await recorder.press("Control+k");
  await expect(page.getByRole("alert")).toContainText("Disk is full");
  await expect(recorder).toHaveText("Not set");
  expect(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("test-keybindings")!).bindings
          .newTerminal,
    ),
  ).toBeNull();
  await page.evaluate(() => {
    (window as any).__nativeTest.failKeybindingsSave = false;
  });
  await page
    .getByRole("button", {
      name: "Reset shortcut for New terminal",
      exact: true,
    })
    .click();
  await expect(recorder).toHaveText("Ctrl+D");
  expect(await calls(page, "start_terminal")).toHaveLength(0);
  await page.screenshot({ path: "test-results/keybindings.png" });
  await page.setViewportSize({ width: 560, height: 420 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({ path: "test-results/keybindings-minimum.png" });
});

test("terminal shortcuts leave form editing alone and environment selection is transient", async ({
  page,
}) => {
  await openTerminal(page);
  await page.keyboard.press("Control+Shift+i");
  const composer = page.getByRole("textbox", {
    name: "Command input",
    exact: true,
  });
  await expect(composer).toBeFocused();
  await composer.fill("echo draft");
  await composer.press("Control+d");
  await composer.press("Control+w");
  await expect(page.locator("[data-pane-id]")).toHaveCount(1);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  expect(await input(page)).not.toMatch(/[\x04\x17]/);
  await page
    .getByRole("button", { name: "Close command input", exact: true })
    .click();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+Shift+l");
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Terminal environment" }),
  ).toHaveText("Local · bash");
  await page.keyboard.press("Control+d");
  await expect(page.locator("[data-pane-id]")).toHaveCount(1);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("combobox")).toHaveCount(0);
});

test("unsupported keybindings remain intact until an explicit reset", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/?window=settings");
  const saved = JSON.stringify({
    version: 99,
    bindings: { future: "keep me" },
  });
  await page.evaluate(
    (saved) => localStorage.setItem("test-keybindings", saved),
    saved,
  );
  await page.reload();
  await expect(page.getByRole("alert")).toContainText("left intact");
  await expect(
    page.getByRole("button", {
      name: "Shortcut for New terminal",
      exact: true,
    }),
  ).toBeDisabled();
  expect(await calls(page, "save_keybindings")).toHaveLength(0);
  expect(
    await page.evaluate(() => localStorage.getItem("test-keybindings")),
  ).toBe(saved);
  await page
    .getByRole("button", { name: "Reset to defaults", exact: true })
    .click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("test-keybindings")!).version,
    ),
  ).toBe(1);
});

test("invalid Explorer click mode stays intact until an explicit reset", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/?window=settings");
  const saved = JSON.stringify({
    version: 1,
    bindings: {},
    explorerOpenOnSingleClick: "yes",
  });
  await page.evaluate(
    (saved) => localStorage.setItem("test-keybindings", saved),
    saved,
  );
  await page.reload();
  await expect(page.getByRole("alert")).toContainText("left intact");
  await expect(
    page.getByRole("switch", { name: "Open with one click" }),
  ).toBeDisabled();
  expect(await calls(page, "save_keybindings")).toHaveLength(0);
  expect(
    await page.evaluate(() => localStorage.getItem("test-keybindings")),
  ).toBe(saved);

  await page
    .getByRole("button", { name: "Reset to defaults", exact: true })
    .click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("test-keybindings")!)
          .explorerOpenOnSingleClick,
    ),
  ).toBe(false);
});
