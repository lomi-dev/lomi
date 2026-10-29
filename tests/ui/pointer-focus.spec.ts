import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import {
  newId,
  newPane,
  newProject,
  newSession,
  splitPane,
} from "../../src/model";
import { mockDesktop } from "./desktop";

async function setup(page: Page, enabled?: boolean) {
  if (enabled !== undefined)
    await page.addInitScript((focusFollowsPointer) => {
      localStorage.setItem(
        "test-keybindings",
        JSON.stringify({
          version: 1,
          bindings: {},
          focusFollowsPointer,
        }),
      );
    }, enabled);
  const project = newProject("/project", "local:bash");
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  const firstId = tab.activePaneId;
  const second = newPane("/project/2");
  const third = newPane("/project/3");
  const fourth = newPane("/project/4");
  tab.layout = splitPane(tab.layout, firstId, "horizontal", third);
  tab.layout = splitPane(tab.layout, firstId, "vertical", second);
  tab.layout = splitPane(tab.layout, third.id, "vertical", fourth);
  await mockDesktop(page, true, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toHaveCount(4);
  await expect
    .poll(async () => (await calls(page, "start_terminal")).length)
    .toBe(4);
  return {
    first: page.locator(`[data-pane-id="${firstId}"]`),
    fourth: page.locator(`[data-pane-id="${fourth.id}"]`),
  };
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

async function input(page: Page, cwd: string) {
  const started = (await calls(page, "start_terminal")).find(
    (call: any) => call.args.request.cwd === cwd,
  );
  return (await calls(page, "write_terminal"))
    .filter((call: any) => call.args.id === started.args.request.id)
    .map((call: any) => call.args.data)
    .join("");
}

test("click focus is the default for typing, paste, control keys, splitting and closing", async ({
  page,
}) => {
  const { first, fourth } = await setup(page);
  await first.click();
  await fourth.hover();
  await expect(first.locator(".xterm-helper-textarea")).toBeFocused();
  await page.keyboard.insertText("zażółć 🦀");
  await page.keyboard.press("Control+Shift+v");
  await expect
    .poll(() => input(page, "/project"))
    .toBe("zażółć 🦀clipboard text");
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("Control+c");
  await expect
    .poll(() => input(page, "/project"))
    .toBe("zażółć 🦀clipboard text\x1b[D\x03");
  expect(await input(page, "/project/4")).toBe("");
  const firstBounds = (await first.boundingBox())!;
  const fourthBounds = await fourth.boundingBox();
  await page.keyboard.press("Control+d");
  await expect(page.locator("[data-pane-id]")).toHaveCount(5);
  expect((await first.boundingBox())!.width).toBeLessThan(firstBounds.width);
  expect(await fourth.boundingBox()).toEqual(fourthBounds);
  await page.keyboard.press("Control+w");
  await expect(page.locator("[data-pane-id]")).toHaveCount(4);
  await expect(fourth).toBeVisible();
  await expect(first.locator(".xterm-helper-textarea")).toBeFocused();
});

test("pointer focus routes text, paste and panel shortcuts to terminal four", async ({
  page,
}) => {
  const { first, fourth } = await setup(page, true);
  await first.click();
  await fourth.hover();
  await expect(fourth.locator(".xterm-helper-textarea")).toBeFocused();
  await page.keyboard.insertText("zażółć 🦀");
  await page.keyboard.press("Control+Shift+v");
  await expect
    .poll(() => input(page, "/project/4"))
    .toBe("zażółć 🦀clipboard text");
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("Control+c");
  await expect
    .poll(() => input(page, "/project/4"))
    .toBe("zażółć 🦀clipboard text\x1b[D\x03");
  expect(await input(page, "/project")).toBe("");
  await page.keyboard.press("Control+Shift+f");
  await expect(
    fourth.getByRole("textbox", { name: "Search terminal output" }),
  ).toBeFocused();
  await page.keyboard.type("query");
  await expect(
    fourth.getByRole("textbox", { name: "Search terminal output" }),
  ).toHaveValue("query");
  await first.hover();
  await page.keyboard.press("Control+Shift+i");
  await expect(
    first.getByRole("textbox", { name: "Command input", exact: true }),
  ).toBeFocused();
  await page.keyboard.type("draft");
  await expect(
    first.getByRole("textbox", { name: "Command input", exact: true }),
  ).toHaveValue("draft");
  expect(await input(page, "/project")).toBe("");
});

test("the settings switch applies across windows, survives reloads and preserves shortcuts", async ({
  page,
  context,
}, testInfo) => {
  const { first, fourth } = await setup(page);
  const settings = await context.newPage();
  await mockDesktop(settings);
  await settings.goto("/?window=settings");
  const toggle = settings.getByRole("switch", {
    name: "Focus follows pointer",
  });
  await expect(toggle).not.toBeChecked();
  await toggle.check();
  await expect(toggle).toBeChecked();
  const recorder = settings.getByRole("button", {
    name: "Shortcut for New terminal",
    exact: true,
  });
  await recorder.click();
  await recorder.press("Control+k");
  await expect(recorder).toHaveText("Ctrl+K");
  await settings.reload();
  await expect(toggle).toBeChecked();
  await expect(recorder).toHaveText("Ctrl+K");
  await page.bringToFront();
  await first.click();
  await fourth.hover();
  await expect(fourth.locator(".xterm-helper-textarea")).toBeFocused();
  expect(await calls(page, "start_terminal")).toHaveLength(4);
  await settings.bringToFront();
  await settings.setViewportSize({ width: 560, height: 420 });
  await settings.screenshot({
    path: testInfo.outputPath("pointer-focus-settings.png"),
  });
  await toggle.uncheck();
  await expect(toggle).not.toBeChecked();
  await page.bringToFront();
  await page.reload();
  await first.click();
  await fourth.hover();
  await expect(first.locator(".xterm-helper-textarea")).toBeFocused();
  await page.keyboard.type("clicked");
  await expect.poll(() => input(page, "/project")).toBe("clicked");
  await settings.bringToFront();
  await settings.reload();
  await expect(toggle).not.toBeChecked();
  await expect(recorder).toHaveText("Ctrl+K");
  expect(await calls(settings, "start_terminal")).toHaveLength(0);
});

test("invalid pointer preferences are preserved and failed saves keep the previous mode", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.addInitScript(() =>
    localStorage.setItem(
      "test-keybindings",
      '{"version":1,"bindings":{},"focusFollowsPointer":"yes"}',
    ),
  );
  await page.goto("/?window=settings");
  const toggle = page.getByRole("switch", { name: "Focus follows pointer" });
  await expect(toggle).toBeDisabled();
  await expect(page.getByRole("alert")).toContainText("left intact");
  expect(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("test-keybindings")!)
          .focusFollowsPointer,
    ),
  ).toBe("yes");
  await page
    .getByRole("button", { name: "Reset to defaults", exact: true })
    .click();
  await expect(toggle).toBeEnabled();
  await expect(toggle).not.toBeChecked();
  await page.evaluate(() => {
    (window as any).__nativeTest.failKeybindingsSave = true;
  });
  await toggle.click();
  await expect(page.getByRole("alert")).toContainText("Disk is full");
  await expect(toggle).not.toBeChecked();
  expect(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("test-keybindings")!)
          .focusFollowsPointer,
    ),
  ).toBe(false);
});

test("pointer focus preserves drags, composition, dialogs and ordinary form editing", async ({
  page,
}) => {
  const { first, fourth } = await setup(page, true);
  await first.click();
  await page.mouse.down();
  const bounds = (await fourth.boundingBox())!;
  await page.mouse.move(bounds.x + 30, bounds.y + 30);
  await expect(first.locator(".xterm-helper-textarea")).toBeFocused();
  await page.mouse.up();
  await first.click();
  await first
    .locator(".xterm-helper-textarea")
    .dispatchEvent("compositionstart", { data: "" });
  await fourth.hover();
  await expect(first.locator(".xterm-helper-textarea")).toBeFocused();
  await first
    .locator(".xterm-helper-textarea")
    .dispatchEvent("compositionend", { data: "" });
  await expect(fourth.locator(".xterm-helper-textarea")).toBeFocused();
  await page.keyboard.press("Control+Shift+l");
  const environment = page.getByRole("combobox", {
    name: "Terminal environment",
  });
  await environment.focus();
  await expect(environment).toBeFocused();
  await page.mouse.move(bounds.x + 10, bounds.y + 10);
  await expect(environment).toBeFocused();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.keyboard.press("Control+Shift+g");
  const message = page.getByRole("textbox", { name: "Commit message" });
  await message.fill("draft");
  await fourth.hover();
  await page.keyboard.type(" stays here");
  await expect(message).toHaveValue("draft stays here");
});

test("a split keeps keyboard input under a stationary pointer", async ({
  page,
}) => {
  const { fourth } = await setup(page, true);
  await fourth.hover({ position: { x: 20, y: 20 } });
  await page.keyboard.press("Control+d");
  await expect(page.locator("[data-pane-id]")).toHaveCount(5);
  await expect(fourth.locator(".xterm-helper-textarea")).toBeFocused();
  await page.keyboard.type("still here");
  await expect.poll(() => input(page, "/project/4")).toBe("still here");
  await page.keyboard.press("Control+w");
  await expect(fourth).toHaveCount(0);
  await expect(page.locator("[data-pane-id]")).toHaveCount(4);
  await expect(
    page.locator("[data-pane-id]").last().locator(".xterm-helper-textarea"),
  ).toBeFocused();
});

test("pointer focus switches between a terminal and a docked file editor", async ({
  page,
}) => {
  const project = newProject("/project", "local:bash");
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  tab.layout = splitPane(tab.layout, tab.activePaneId, "horizontal", {
    type: "file",
    id: newId(),
    title: "README.md",
    root: "/project",
    relative: "README.md",
  });
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.addInitScript(() =>
    localStorage.setItem(
      "test-keybindings",
      '{"version":1,"bindings":{},"focusFollowsPointer":true}',
    ),
  );
  await page.goto("/");
  const terminal = page.locator(".terminal-pane");
  const editor = page.locator(".cm-content");
  await expect(editor).toBeVisible();
  await terminal.click();
  await editor.hover();
  await expect(editor).toBeFocused();
  await page.keyboard.press("Control+End");
  await page.keyboard.type(" pointer text");
  await expect(editor).toContainText("pointer text");
  await page.keyboard.press("Control+s");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.editorFiles["/project/README.md"]
            .content,
      ),
    )
    .toContain("pointer text");
  await terminal.hover();
  await page.keyboard.type("terminal text");
  await expect.poll(() => input(page, "/project")).toBe("terminal text");
});
