import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

async function calls(page: Page, command: string) {
  return page.evaluate(
    (command) =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === command,
      ),
    command,
  );
}

async function markBusy(page: Page, index = 0) {
  // xterm mounts before font loading finishes and the native session starts.
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.sessions.size))
    .toBeGreaterThan(index);
  return page.evaluate((index) => {
    const native = (window as any).__nativeTest;
    const id = [...native.sessions.keys()][index] as string;
    native.busyTerminals = [id];
    return id;
  }, index);
}

for (const target of ["panel", "tab", "window"]) {
  test(`closing a busy ${target} cancels with Escape and confirms with Enter`, async ({
    page,
  }, testInfo) => {
    await mockDesktop(page);
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    if (target === "panel") {
      await page.keyboard.press("Control+d");
      await expect(page.locator(".xterm-screen")).toHaveCount(2);
    }
    const paneId = (await page
      .locator("[data-pane-id]")
      .last()
      .getAttribute("data-pane-id"))!;
    const pane = page.locator(`[data-pane-id="${paneId}"]`);
    const sessionId = await markBusy(page, target === "panel" ? 1 : 0);
    const close = async () => {
      if (target === "window")
        await page.getByRole("button", { name: "Close window" }).click();
      else {
        await pane.locator(".xterm-helper-textarea").focus();
        await page.keyboard.press(
          target === "panel" ? "Control+w" : "Control+Shift+w",
        );
      }
    };
    await close();
    const dialog = page.getByRole("dialog", {
      name:
        target === "window" ? "Quit SimpleBench?" : "Close running processes?",
    });
    await expect(dialog).toBeVisible();
    await expect(
      dialog.getByRole("button", {
        name: target === "window" ? "Cancel" : "Close anyway",
        exact: true,
      }),
    ).toBeFocused();
    expect(await calls(page, "close_terminal")).toHaveLength(0);
    expect(await calls(page, "plugin:window|destroy")).toHaveLength(0);
    await page.evaluate((id) => {
      (window as any).__nativeTest.emit(id, "\r\nSTILL WORKING\r\n");
    }, sessionId);
    await expect.poll(() => buffer(page, paneId)).toContain("STILL WORKING");
    if (target === "panel")
      await page.screenshot({
        path: testInfo.outputPath("terminal-close-dialog.png"),
      });
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(pane).toBeVisible();
    expect(await calls(page, "close_terminal")).toHaveLength(0);
    await close();
    if (target === "window")
      await dialog.getByRole("button", { name: "Quit anyway" }).focus();
    await page.keyboard.press("Enter");
    expect(await calls(page, "write_terminal")).toHaveLength(0);
    if (target === "window") {
      await expect
        .poll(async () => (await calls(page, "plugin:window|destroy")).length)
        .toBe(1);
      expect(await calls(page, "save_session")).not.toHaveLength(0);
    } else {
      await expect(pane).toHaveCount(0);
      await expect
        .poll(async () =>
          (await calls(page, "close_terminal")).map(
            (call: any) => call.args.id,
          ),
        )
        .toEqual([sessionId]);
    }
  });
}

test("idle panes close without asking while a hidden busy terminal protects the window", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const busy = await markBusy(page);
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect
    .poll(async () => (await calls(page, "start_terminal")).length)
    .toBe(2);
  await page.keyboard.press("Control+w");
  await expect(page.getByRole("tab")).toHaveCount(1);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect((await calls(page, "close_terminal"))[0].args.id).not.toBe(busy);
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await page.getByRole("button", { name: "Close window" }).click();
  const dialog = page.getByRole("dialog", { name: "Quit SimpleBench?" });
  await expect(dialog).toContainText("This terminal has running processes");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await calls(page, "plugin:window|destroy")).toHaveLength(0);
});

test("process inspection failures and repeated close requests cannot silently close the window", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.terminalProcessError = "Process inspection failed";
    native.terminalProcessDelay = 100;
    void native.emitEvent("tauri://close-requested");
    void native.emitEvent("tauri://close-requested");
  });
  const dialog = page.getByRole("dialog", { name: "Quit SimpleBench?" });
  await expect(dialog).toContainText("Process inspection failed");
  expect(await calls(page, "busy_terminals")).toHaveLength(1);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await calls(page, "plugin:window|destroy")).toHaveLength(0);
  await page.evaluate(() => {
    (window as any).__nativeTest.terminalProcessError = "";
  });
  await page.getByRole("button", { name: "Close window" }).click();
  await expect
    .poll(async () => (await calls(page, "plugin:window|destroy")).length)
    .toBe(1);
});

test("confirming terminal closure still protects dirty editors and failed saves", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await markBusy(page);
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  await expect(page.locator(".cm-content")).toBeVisible();
  await page.locator(".cm-content").fill("unsaved work");
  await page.evaluate(() => {
    (window as any).__nativeTest.failFileSave = true;
  });
  await page.getByRole("button", { name: "Close window" }).click();
  await expect(
    page.getByRole("dialog", { name: "Quit SimpleBench?" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Quit anyway" }).click();
  const editor = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await expect(
    editor.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(editor.getByRole("alert")).toHaveText("Disk is full");
  expect(await calls(page, "plugin:window|destroy")).toHaveLength(0);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  await editor.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.locator(".cm-content")).toHaveText("unsaved work");
});
