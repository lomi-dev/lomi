import { expectRecordedNotification } from "./application-notification-helpers";
import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { newProject, newSession, newTab } from "../../src/model";
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
  await page.keyboard.press("Meta+Equal");
  await page.keyboard.press("Meta+Equal");
  await expect.poll(() => zoom(page)).toBe(1.2);
  await expect(page.locator(".titlebar")).toHaveCSS("padding-left", "88px");
  await expect(page.locator(".titlebar")).toHaveCSS("min-height", "44px");
});

test("a heavily zoomed minimum window keeps titlebar controls reachable", async ({
  page,
}, testInfo) => {
  const project = newProject("/project", "local:bash");
  const workspace = project.workspaces[0];
  workspace.tabs = Array.from({ length: 3 }, (_, index) =>
    newTab("/project", "local:bash", `Terminal ${index + 1}`),
  );
  workspace.activeTabId = workspace.tabs[0].id;
  await mockDesktop(
    page,
    true,
    { ...newSession(), projects: [project], activeProjectId: project.id },
    undefined,
    {},
    "macos",
  );
  await page.addInitScript(() => localStorage.setItem("lomi.zoom.main", "200"));
  await page.setViewportSize({ width: 400, height: 210 });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect(page.getByRole("tab")).toHaveCount(3);
  await expect(page.locator(".titlebar")).toHaveCSS("padding-left", "88px");
  await expect(page.locator(".titlebar")).toHaveCSS("min-height", "44px");
  const switcher = page.locator(".project-switcher");
  const switcherBox = await switcher.boundingBox();
  expect(switcherBox?.x).toBeGreaterThanOrEqual(88);
  const projectButton = page.locator(".project-switcher");
  const accountButton = page.getByRole("button", { name: "Open account menu" });
  const rightArrow = page.getByRole("button", { name: "Scroll tabs right" });
  await expect(rightArrow).toBeEnabled();
  const navigation = page.locator(".tab-navigation");
  const account = page.locator(".titlebar-account-control");
  const navigationBox = (await navigation.boundingBox())!;
  const accountBox = (await account.boundingBox())!;
  expect(navigationBox.x + navigationBox.width).toBeLessThanOrEqual(
    accountBox.x,
  );
  for (const target of [projectButton, rightArrow, account]) {
    expect(
      await target.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const hit = document.elementFromPoint(
          bounds.left + bounds.width / 2,
          bounds.top + bounds.height / 2,
        );
        return hit === element || element.contains(hit);
      }),
    ).toBe(true);
  }
  const clickCenter = async (target: typeof projectButton) => {
    const box = (await target.boundingBox())!;
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  };
  await clickCenter(projectButton);
  await expect(projectButton).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("Escape");
  await clickCenter(accountButton);
  await expect(accountButton).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("Escape");
  const strip = page.locator(".tab-strip");
  const initialScrollLeft = await strip.evaluate(
    (element) => element.scrollLeft,
  );
  await clickCenter(rightArrow);
  await expect
    .poll(() => strip.evaluate((element) => element.scrollLeft))
    .toBeGreaterThan(initialScrollLeft);
  const newTabButton = page.getByRole("button", { name: /^New tab/ });
  let newTabInView = false;
  for (let index = 0; index < 20; index++) {
    newTabInView = await newTabButton.evaluate((button) => {
      const buttonBounds = button.getBoundingClientRect();
      const stripBounds = button.closest(".tab-strip")!.getBoundingClientRect();
      return (
        buttonBounds.left >= stripBounds.left &&
        buttonBounds.right <= stripBounds.right
      );
    });
    if (newTabInView) break;
    await page.getByRole("button", { name: "Scroll tabs right" }).click();
  }
  expect(newTabInView).toBe(true);
  await clickCenter(newTabButton);
  await expect(
    page.getByRole("menuitem", { name: "New terminal" }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("zoomed-minimum-titlebar.png"),
  });
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

test("Settings records zoom keys and applies bindings to the same main window", async ({
  page,
}) => {
  await mockDesktop(page, false, undefined, undefined, {}, "macos");
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.keyboard.press("Meta+Comma");
  const recorder = page.getByRole("button", {
    name: "Shortcut for Zoom out",
    exact: true,
  });
  await expect(recorder).toHaveText("Cmd+-");
  await recorder.click();
  await recorder.press("Meta+Minus");
  await expect(recorder).toHaveText("Cmd+-");
  expect(await zoom(page)).toBe(1);
  await recorder.click();
  await recorder.press("Meta+j");
  await expect(recorder).toHaveText("Cmd+J");
  await page
    .getByRole("button", { name: "Close Settings", exact: true })
    .click();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Meta+j");
  await expect.poll(() => zoom(page)).toBe(0.9);
  await page.keyboard.press("Meta+Minus");
  expect(await zoom(page)).toBe(0.9);
  await page.keyboard.press("Meta+Comma");
  await expect(recorder).toHaveText("Cmd+J");
  await page.getByRole("heading", { name: "Keybinds", exact: true }).click();
  await page.keyboard.press("Meta+Equal");
  await expect.poll(() => zoom(page)).toBe(1);
  expect(
    await page.evaluate(() => localStorage.getItem("lomi.zoom.main")),
  ).toBe("100");
  expect(await commands(page, "start_terminal")).toHaveLength(1);
  expect(await commands(page, "close_terminal")).toHaveLength(0);
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
  await expectRecordedNotification(page, "Could not change zoom");
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
