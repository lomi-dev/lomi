import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

const zoom = (page: Page) =>
  page.evaluate(() => (window as any).__nativeTest.zoom);
const commands = (page: Page, command: string) =>
  page.evaluate(
    (command) =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === command,
      ),
    command,
  );

for (const platform of ["macos", "linux"] as const) {
  test(`${platform} zoom shortcuts capture terminal input and preserve running panels`, async ({
    page,
  }) => {
    const mod = platform === "macos" ? "Meta" : "Control";
    await mockDesktop(page, true, undefined, undefined, {}, platform);
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    await page.locator(".xterm-helper-textarea").focus();
    await page.keyboard.press(`${mod}+d`);
    await expect(page.locator(".xterm-screen")).toHaveCount(2);
    await expect.poll(() => commands(page, "start_terminal")).toHaveLength(2);
    const started = await commands(page, "start_terminal");
    await page.keyboard.press(`${mod}+Minus`);
    await expect.poll(() => zoom(page)).toBe(0.9);
    await page.keyboard.press(`${mod}+Shift+Equal`);
    await expect.poll(() => zoom(page)).toBe(1);
    await page.keyboard.press(`${mod}+Equal`);
    await expect.poll(() => zoom(page)).toBe(1.1);
    await page.keyboard.press(`${mod}+NumpadAdd`);
    await expect.poll(() => zoom(page)).toBe(1.2);
    await page.keyboard.press(`${mod}+NumpadSubtract`);
    await expect.poll(() => zoom(page)).toBe(1.1);
    await page.keyboard.press(`${mod}+Digit0`);
    await expect.poll(() => zoom(page)).toBe(1);
    expect(await commands(page, "start_terminal")).toEqual(started);
    expect(await commands(page, "close_terminal")).toHaveLength(0);
    expect(await commands(page, "write_terminal")).toHaveLength(0);
    await expect(
      page.locator(".terminal-pane.is-active .xterm-helper-textarea"),
    ).toBeFocused();
  });
}

test("zoom works on welcome, remembers its level and respects the native macOS titlebar", async ({
  page,
}) => {
  await mockDesktop(page, false, null, undefined, {}, "macos");
  await page.goto("/");
  await expect(
    page.getByRole("main", { name: "No project open" }),
  ).toBeVisible();
  await page.keyboard.press("Meta+Minus");
  await page.keyboard.press("Meta+Minus");
  await expect.poll(() => zoom(page)).toBe(0.8);
  await expect(page.locator(".titlebar")).toHaveCSS("padding-left", "110px");
  await expect(page.locator(".titlebar")).toHaveCSS("min-height", "55px");
  await page.reload();
  await expect.poll(() => zoom(page)).toBe(0.8);
  expect(await commands(page, "start_terminal")).toHaveLength(0);
  await page.keyboard.press("Meta+Digit0");
  await expect.poll(() => zoom(page)).toBe(1);
});

test("a heavily zoomed minimum window fits the pane without scrolling the workspace", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await page.addInitScript(() => localStorage.setItem("lomi.zoom.main", "200"));
  await page.setViewportSize({ width: 400, height: 210 });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const area = page.locator(".work-area");
  expect(
    await area.evaluate((element) => {
      return element.scrollWidth <= element.clientWidth;
    }),
  ).toBe(true);
  await page.keyboard.press("Meta+Digit0");
  await expect.poll(() => zoom(page)).toBe(1);
});

test("zoom preserves editor text, selection, undo and a modal's input", async ({
  page,
}) => {
  await mockDesktop(page, true, undefined, undefined, {}, "macos");
  await page.goto("/");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New file", exact: true }).click();
  const editor = page.locator(".cm-content");
  await expect(editor).toBeVisible();
  await page.keyboard.insertText("Keep these edits 🦀");
  await page.keyboard.press("Meta+a");
  const selection = await page.evaluate(() => getSelection()?.toString());
  await page.keyboard.press("Meta+Minus");
  await expect.poll(() => zoom(page)).toBe(0.9);
  await expect(editor).toHaveText("Keep these edits 🦀");
  expect(await page.evaluate(() => getSelection()?.toString())).toBe(selection);
  await page.keyboard.press("Meta+z");
  await expect(editor).toBeEmpty();
  await page.keyboard.press("Meta+Shift+z");
  await expect(editor).toHaveText("Keep these edits 🦀");
  await page.keyboard.press("Meta+w");
  const dialog = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Meta+Equal");
  await expect.poll(() => zoom(page)).toBe(1);
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(editor).toHaveText("Keep these edits 🦀");
  await page
    .getByRole("button", { name: "Toggle workspaces", exact: true })
    .click();
  await page
    .getByRole("button", { name: "New workspace", exact: true })
    .click();
  const name = page.getByRole("textbox", { name: "Name", exact: true });
  await name.fill("My workspace");
  await name.press("Meta+Minus");
  await expect.poll(() => zoom(page)).toBe(0.9);
  await expect(name).toHaveValue("My workspace");
  await expect(name).toBeFocused();
});

test("settings records zoom keys and updates main-window bindings without changing its zoom", async ({
  page,
  context,
}) => {
  await mockDesktop(page, false, undefined, undefined, {}, "macos");
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const settings = await context.newPage();
  await mockDesktop(settings, false, null, undefined, {}, "macos");
  await settings.goto("/?window=settings");
  const recorder = settings.getByRole("button", {
    name: "Shortcut for Zoom out",
    exact: true,
  });
  await expect(recorder).toHaveText("Cmd+-");
  await recorder.click();
  await recorder.press("Meta+Minus");
  await expect(recorder).toHaveText("Cmd+-");
  expect(await zoom(settings)).toBe(1);
  await recorder.click();
  await recorder.press("Meta+j");
  await expect(recorder).toHaveText("Cmd+J");
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const native = (window as any).__nativeTest;
        return native.calls.filter(
          (call: any) => call.command === "load_keybindings",
        ).length;
      }),
    )
    .toBeGreaterThan(1);
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Meta+j");
  await expect.poll(() => zoom(page)).toBe(0.9);
  await page.keyboard.press("Meta+Minus");
  expect(await zoom(page)).toBe(0.9);
  await settings.locator("h1").click();
  await settings.keyboard.press("Meta+Equal");
  await expect.poll(() => zoom(settings)).toBe(1.1);
  expect(await zoom(page)).toBe(0.9);
  await settings.reload();
  await expect.poll(() => zoom(settings)).toBe(1.1);
  await expect(recorder).toHaveText("Cmd+J");
});

test("queued zoom shortcuts respect limits, composition and native failures", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.locator(".xterm-helper-textarea").focus();
  await page.evaluate(() => {
    (window as any).__nativeTest.zoomDelay = 20;
    for (let index = 0; index < 20; index++)
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          code: "Minus",
          key: "-",
          ctrlKey: true,
          repeat: index > 0,
          bubbles: true,
          cancelable: true,
        }),
      );
  });
  await expect.poll(() => zoom(page)).toBe(0.5);
  await page.keyboard.press("Control+Digit0");
  await expect.poll(() => zoom(page)).toBe(1);
  await page.evaluate(() => {
    for (let index = 0; index < 20; index++)
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          code: "Equal",
          key: "+",
          ctrlKey: true,
          shiftKey: true,
          repeat: index > 0,
          bubbles: true,
          cancelable: true,
        }),
      );
  });
  await expect.poll(() => zoom(page)).toBe(2);
  await page.keyboard.press("Control+Digit0");
  await expect.poll(() => zoom(page)).toBe(1);
  await page.evaluate(() => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        code: "Minus",
        key: "-",
        ctrlKey: true,
        isComposing: true,
        bubbles: true,
      }),
    );
    (window as any).__nativeTest.failZoom = true;
  });
  expect(await zoom(page)).toBe(1);
  await page.keyboard.press("Control+Minus");
  await expect(page.getByRole("alert").filter({ hasText: /\S/ })).toContainText(
    "Could not change zoom",
  );
  expect(await zoom(page)).toBe(1);
  expect(
    await page.evaluate(() => localStorage.getItem("lomi.zoom.main")),
  ).toBe("100");
  await page.evaluate(() => {
    (window as any).__nativeTest.failZoom = false;
  });
  await page.keyboard.press("Control+Minus");
  await expect.poll(() => zoom(page)).toBe(0.9);
  expect(await commands(page, "write_terminal")).toHaveLength(0);
});
