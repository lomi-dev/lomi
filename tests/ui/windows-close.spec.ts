import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function prepare(page: Page) {
  await mockDesktop(page, true, undefined, undefined, {}, "windows");
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.sessions.size))
    .toBe(1);
  await page.evaluate(() => {
    (window as any).__nativeTest.calls.length = 0;
  });
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

async function nativeClose(page: Page) {
  await page.evaluate(() => {
    void (window as any).__nativeTest.emitEvent("tauri://close-requested");
  });
}

for (const trigger of ["titlebar button", "native close event"]) {
  test(`Windows ${trigger} saves and drains before destroying the window once`, async ({
    page,
  }) => {
    await prepare(page);
    await page.evaluate(() => {
      (window as any).__nativeTest.androidExitHold = true;
    });
    if (trigger === "titlebar button")
      await page.getByRole("button", { name: "Close window" }).click();
    else await nativeClose(page);
    const progress = page.getByRole("dialog", { name: "Preparing to close" });
    await expect(progress).toBeVisible();
    await expect
      .poll(async () =>
        (await calls(page, "android_exit")).map(
          (call: any) => call.args.action.type,
        ),
      )
      .toContain("finish");
    await page.evaluate(() => {
      const mock = (window as any).__nativeTest;
      void mock.emitEvent("tauri://close-requested");
      void mock.emitEvent("lomi-quit-requested");
    });
    expect(await calls(page, "plugin:window|destroy")).toHaveLength(0);
    expect(await calls(page, "agent_runtime_prepare_close")).toHaveLength(1);
    await page.evaluate(() => {
      const mock = (window as any).__nativeTest;
      mock.androidExitHold = false;
      mock.finishAndroidExit();
    });
    await expect
      .poll(async () => (await calls(page, "plugin:window|destroy")).length)
      .toBe(1);
    const order = await page.evaluate(() =>
      (window as any).__nativeTest.calls.map((call: any) =>
        call.command === "android_exit" ? call.args.action.type : call.command,
      ),
    );
    for (const [before, after] of [
      ["agent_runtime_prepare_close", "agent_control_closing"],
      ["agent_control_closing", "begin"],
      ["begin", "save_session"],
      ["save_session", "agent_tasks_drain"],
      ["agent_tasks_drain", "finish"],
      ["finish", "plugin:window|destroy"],
    ]) {
      expect(order).toContain(before);
      expect(order.indexOf(before)).toBeLessThan(order.indexOf(after));
    }
    expect(await calls(page, "hide_main_window")).toHaveLength(0);
    expect(await calls(page, "agent_runtime_cancel_close")).toHaveLength(0);
  });
}

test("Windows close keeps a busy terminal on cancellation and confirms a retry", async ({
  page,
}) => {
  await prepare(page);
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.busyTerminals = [...mock.sessions.keys()];
    mock.terminalProcessDelay = 100;
  });
  await nativeClose(page);
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    void mock.emitEvent("tauri://close-requested");
    void mock.emitEvent("lomi-quit-requested");
  });
  const dialog = page.getByRole("dialog", { name: "Quit Lomi?" });
  await expect(dialog).toContainText("This terminal has running processes");
  await expect(
    dialog.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  expect(await calls(page, "busy_terminals")).toHaveLength(1);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(async () => (await calls(page, "agent_runtime_cancel_close")).length)
    .toBe(1);
  await expect(page.locator(".xterm-screen")).toBeVisible();
  expect(await calls(page, "plugin:window|destroy")).toHaveLength(0);
  expect(await calls(page, "agent_tasks_drain")).toHaveLength(0);
  await page.getByRole("button", { name: "Close window" }).click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Quit anyway" }).click();
  await expect
    .poll(async () => (await calls(page, "plugin:window|destroy")).length)
    .toBe(1);
  expect(await calls(page, "hide_main_window")).toHaveLength(0);
});

test("Windows close releases shutdown gates when window destruction fails and allows retry", async ({
  page,
}) => {
  await prepare(page);
  await page.evaluate(() => {
    const internals = (window as any).__TAURI_INTERNALS__;
    const invoke = internals.invoke;
    internals.invoke = async (command: string, args: unknown) => {
      if (command === "plugin:window|destroy") {
        internals.invoke = invoke;
        throw new Error("Window destruction failed");
      }
      return invoke(command, args);
    };
  });
  await nativeClose(page);

  await expect
    .poll(async () => (await calls(page, "agent_runtime_cancel_close")).length)
    .toBe(1);
  await expect
    .poll(async () =>
      (await calls(page, "agent_control_closing")).map(
        (call: any) => call.args.closing,
      ),
    )
    .toEqual([true, false]);
  await expect(
    page.getByRole("dialog", { name: "Preparing to close" }),
  ).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(
    page.getByRole("dialog", { name: "Notifications", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Open account menu" }).click();
  await page
    .getByRole("menuitem", { name: "Notifications", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "Notifications", exact: true }),
  ).toContainText("Could not close the window: Window destruction failed");
  await page
    .getByRole("button", { name: "Close notifications", exact: true })
    .click();
  await nativeClose(page);
  await expect
    .poll(async () => (await calls(page, "plugin:window|destroy")).length)
    .toBe(1);
  expect(await calls(page, "hide_main_window")).toHaveLength(0);
});

test("Windows close preserves dirty edits after a failed save and cancellation, then retries", async ({
  page,
}) => {
  await prepare(page);
  await page.getByRole("button", { name: "README.md", exact: true }).dblclick();
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText("Windows unsaved — Zażółć 🦀");
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("button", { name: "Close window" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await expect(dialog).toContainText("/project/README.md");
  await page.evaluate(() => {
    (window as any).__nativeTest.failFileSave = true;
  });
  await dialog
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(dialog.getByRole("alert")).toHaveText("Disk is full");
  await nativeClose(page);
  await expect(page.getByRole("dialog")).toHaveCount(1);
  expect(await calls(page, "agent_tasks_drain")).toHaveLength(0);
  expect(await calls(page, "plugin:window|destroy")).toHaveLength(0);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(async () => (await calls(page, "agent_runtime_cancel_close")).length)
    .toBe(1);
  await page.getByRole("tab", { name: /README.md/ }).click();
  await expect(page.locator(".cm-content")).toContainText(
    "Windows unsaved — Zażółć 🦀",
  );
  await page.evaluate(() => {
    (window as any).__nativeTest.failFileSave = false;
  });
  await nativeClose(page);
  await dialog
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect
    .poll(async () => (await calls(page, "plugin:window|destroy")).length)
    .toBe(1);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.editorFiles["/project/README.md"].content,
    ),
  ).toBe("Windows unsaved — Zażółć 🦀");
  expect(await calls(page, "hide_main_window")).toHaveLength(0);
});
