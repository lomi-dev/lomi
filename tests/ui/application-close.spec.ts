import { expect, test, type Page } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";
import { mockChats } from "./chat-mock";

async function prepare(page: Page) {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.calls.length = 0;
    mock.androidExitDelay = 700;
  });
}

async function actions(page: Page) {
  return page.evaluate(
    () =>
      (window as any).__nativeTest.calls
        .filter((call: any) =>
          [
            "android_exit",
            "agent_control_closing",
            "agent_runtime_prepare_close",
            "agent_runtime_cancel_close",
            "agent_tasks_drain",
            "save_session",
            "restart_plugins",
            "plugin:window|destroy",
          ].includes(call.command),
        )
        .map((call: any) =>
          call.command === "android_exit"
            ? call.args.action.type
            : call.command === "agent_control_closing"
              ? `agent-control:${call.args.closing ? "freeze" : "resume"}`
              : call.command === "agent_runtime_prepare_close"
                ? "runtime:freeze"
                : call.command === "agent_runtime_cancel_close"
                  ? "runtime:resume"
                  : call.command,
        ) as string[],
  );
}

test("a pending autosave cannot run after the final close save and resumes after cancellation", async ({
  page,
}) => {
  await prepare(page);
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+d");
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.calls.length = 0;
    mock.androidExitHold = true;
    void mock.emitEvent("lomi-quit-requested");
  });
  const progress = page.getByRole("dialog", { name: "Preparing to close" });
  await expect(progress).toBeVisible();
  await expect.poll(() => actions(page)).toContain("finish");
  await page.keyboard.press("Escape");
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.androidExitHold = false;
    mock.finishAndroidExit();
  });
  await expect(progress).toHaveCount(0);
  const order = await actions(page);
  const duringShutdown = order.slice(0, order.indexOf("resume"));
  expect(
    duringShutdown.filter((action) => action === "save_session"),
  ).toHaveLength(1);
  expect(duringShutdown.lastIndexOf("save_session")).toBeLessThan(
    duringShutdown.indexOf("finish"),
  );
  const count = order.filter((action) => action === "save_session").length;
  await page.locator(".xterm-helper-textarea").last().focus();
  await page.keyboard.press("Control+d");
  await expect(page.locator(".xterm-screen")).toHaveCount(3);
  await expect
    .poll(
      async () =>
        (await actions(page)).filter((action) => action === "save_session")
          .length,
    )
    .toBeGreaterThan(count);
  expect(await actions(page)).not.toContain("plugin:window|destroy");
});

for (const event of ["lomi-quit-requested", "plugin-restart-request"]) {
  test(`${event} can cancel after saving and waits for native stop before releasing its gate`, async ({
    page,
  }) => {
    await prepare(page);
    await page.evaluate(() => {
      (window as any).__nativeTest.androidExitHold = true;
    });
    await page.evaluate(
      (event) => void (window as any).__nativeTest.emitEvent(event),
      event,
    );
    const progress = page.getByRole("dialog", { name: "Preparing to close" });
    await expect(progress).toBeVisible();
    await expect(
      progress.getByRole("button", { name: "Cancel closing" }),
    ).toBeFocused();
    await expect.poll(() => actions(page)).toContain("finish");
    await page.keyboard.press("Escape");
    await expect(
      progress.getByRole("button", { name: "Cancelling…" }),
    ).toBeDisabled();
    await expect(progress).toContainText("Stopped phones will stay stopped");
    expect(await actions(page)).not.toContain("runtime:resume");
    await page.evaluate(() => {
      const mock = (window as any).__nativeTest;
      mock.androidExitHold = false;
      mock.finishAndroidExit();
    });
    await expect(progress).toHaveCount(0);
    const order = await actions(page);
    expect(order.indexOf("runtime:freeze")).toBeLessThan(
      order.indexOf("agent-control:freeze"),
    );
    expect(order.lastIndexOf("save_session")).toBeLessThan(
      order.indexOf("agent_tasks_drain"),
    );
    expect(order.indexOf("agent_tasks_drain")).toBeLessThan(
      order.indexOf("finish"),
    );
    expect(order.indexOf("finish")).toBeLessThan(
      order.indexOf("runtime:resume"),
    );
    expect(order.indexOf("agent-control:freeze")).toBeLessThan(
      order.indexOf("begin"),
    );
    expect(order.indexOf("begin")).toBeLessThan(
      order.lastIndexOf("save_session"),
    );
    expect(order.lastIndexOf("save_session")).toBeLessThan(
      order.indexOf("finish"),
    );
    expect(order.indexOf("finish")).toBeLessThan(order.indexOf("resume"));
    expect(order.indexOf("resume")).toBeLessThan(
      order.indexOf("agent-control:resume"),
    );
    expect(order).not.toContain("restart_plugins");
    expect(order).not.toContain("plugin:window|destroy");
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__nativeTest.androidPreparation),
      )
      .toBeNull();
    await page.evaluate(
      (event) => void (window as any).__nativeTest.emitEvent(event),
      event,
    );
    await expect
      .poll(() => actions(page))
      .toContain(
        event === "plugin-restart-request"
          ? "restart_plugins"
          : "plugin:window|destroy",
      );
  });
}

test("a failed Android stop keeps the current workspace and allows another close attempt", async ({
  page,
}) => {
  await prepare(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.androidExitError =
      "Phone is still stopping. Retry Stop.";
    void (window as any).__nativeTest.emitEvent("lomi-quit-requested");
  });
  await expect(
    page.getByText(
      "Could not close the window: Phone is still stopping. Retry Stop.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("dialog", { name: "Preparing to close" }),
  ).toHaveCount(0);
  await expect(page.locator(".xterm-screen")).toBeVisible();
  expect(await actions(page)).not.toContain("plugin:window|destroy");
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__nativeTest.androidPreparation),
    )
    .toBeNull();
  await page.evaluate(() => {
    (window as any).__nativeTest.androidExitError = "";
    void (window as any).__nativeTest.emitEvent("lomi-quit-requested");
  });
  await expect.poll(() => actions(page)).toContain("plugin:window|destroy");
});

for (const trigger of ["Quit command", "native Quit request"]) {
  test(`${trigger} asks before interrupting a hidden AI response`, async ({
    page,
  }, testInfo) => {
    await mockDesktop(page, false);
    await mockChats(page);
    await page.goto("/");
    await page.getByRole("button", { name: /^New tab/ }).click();
    await page.getByRole("menuitem", { name: "Chat AI", exact: true }).click();
    await page.evaluate(() => {
      (window as any).__chatTest.hold = true;
    });
    const input = page.getByRole("textbox", { name: "Message", exact: true });
    await input.fill("Keep working");
    await input.press("Enter");
    await expect(page.locator(".chat-message-assistant")).toContainText(
      "日本語",
    );
    await page.getByRole("tab", { name: "Terminal", exact: true }).click();
    const close = async () => {
      if (trigger === "Quit command")
        await page.evaluate(() =>
          (window as any).__TAURI_INTERNALS__.invoke("request_quit"),
        );
      else
        await page.evaluate(() => {
          void (window as any).__nativeTest.emitEvent("lomi-quit-requested");
        });
    };
    const dialog = page.getByRole("dialog", { name: "Quit Lomi?" });
    await close();
    await expect(dialog).toContainText(
      "Chat AI is still generating a response",
    );
    await expect(
      dialog.getByRole("button", { name: "Cancel", exact: true }),
    ).toBeFocused();
    expect(await page.evaluate(() => (window as any).__chatTest.stops)).toBe(0);
    expect(await actions(page)).toContain("runtime:freeze");
    expect(await actions(page)).not.toContain("agent_tasks_drain");
    expect(await actions(page)).not.toContain("plugin:window|destroy");
    await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__nativeTest.androidPreparation),
      )
      .toBeNull();
    expect(await page.evaluate(() => (window as any).__chatTest.stops)).toBe(0);
    expect(await actions(page)).toContain("runtime:resume");
    expect(await actions(page)).not.toContain("agent_tasks_drain");
    await close();
    await expect(dialog).toBeVisible();
    await page.evaluate(() => {
      void (window as any).__nativeTest.emitEvent("lomi-quit-requested");
    });
    await expect(page.getByRole("dialog")).toHaveCount(1);
    if (trigger === "Quit command") {
      await page.setViewportSize({ width: 800, height: 420 });
      await page.screenshot({
        path: testInfo.outputPath("quit-confirmation.png"),
      });
    }
    await dialog.getByRole("button", { name: "Quit anyway" }).click();
    await expect.poll(() => actions(page)).toContain("plugin:window|destroy");
    expect(await page.evaluate(() => (window as any).__chatTest.stops)).toBe(1);
    const order = await actions(page);
    expect(order.lastIndexOf("save_session")).toBeLessThan(
      order.indexOf("plugin:window|destroy"),
    );
    expect(
      order.filter((action) => action === "plugin:window|destroy"),
    ).toHaveLength(1);
  });
}

test("a finished AI response does not ask for quit confirmation", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Chat AI", exact: true }).click();
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Finish this response");
  await input.press("Enter");
  await expect(page.locator(".chat-message-assistant")).toContainText("日本語");
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toHaveCount(0);
  await page.evaluate(() =>
    (window as any).__TAURI_INTERNALS__.invoke("request_quit"),
  );
  await expect.poll(() => actions(page)).toContain("plugin:window|destroy");
  await expect(page.getByRole("dialog", { name: "Quit Lomi?" })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__chatTest.stops)).toBe(0);
});

test("closing the workspace hides its window and retains busy terminal runtimes", async ({
  page,
}) => {
  await prepare(page);
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.sessions.size))
    .toBe(1);
  const paneId = (await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.busyTerminals = [...native.sessions.keys()];
  });
  await page.getByRole("button", { name: "Close window" }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls.some(
          (c: any) => c.command === "hide_main_window",
        ),
      ),
    )
    .toBe(true);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.emit(native.busyTerminals[0], "BACKGROUND WORK CONTINUES\r\n");
  });
  await expect
    .poll(() => buffer(page, paneId))
    .toContain("BACKGROUND WORK CONTINUES");
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter((call: any) =>
        ["busy_terminals", "close_terminal"].includes(call.command),
      ),
    ),
  ).toHaveLength(0);
  const calls = await actions(page);
  expect(calls).not.toContain("plugin:window|destroy");
  expect(calls).not.toContain("agent-control:freeze");
  expect(calls).not.toContain("finish");
});

test("a failed final session save releases runtime admission without draining runs", async ({
  page,
}) => {
  await prepare(page);
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.failSave = true;
    void mock.emitEvent("lomi-quit-requested");
  });
  await expect(
    page.getByText("Could not close the window: Disk is full", { exact: true }),
  ).toBeVisible();
  const failed = await actions(page);
  expect(failed).toContain("runtime:freeze");
  expect(failed).toContain("runtime:resume");
  expect(failed).not.toContain("agent_tasks_drain");
  expect(failed).not.toContain("finish");
  expect(failed).not.toContain("plugin:window|destroy");
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.failSave = false;
    void mock.emitEvent("lomi-quit-requested");
  });
  await expect.poll(() => actions(page)).toContain("plugin:window|destroy");
});
