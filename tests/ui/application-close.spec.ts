import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
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
            "save_session",
            "restart_plugins",
            "plugin:window|destroy",
          ].includes(call.command),
        )
        .map((call: any) =>
          call.command === "android_exit"
            ? call.args.action.type
            : call.command,
        ) as string[],
  );
}

for (const event of ["tauri://close-requested", "plugin-restart-request"]) {
  test(`${event} can cancel after saving and waits for native stop before releasing its gate`, async ({
    page,
  }) => {
    await prepare(page);
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
    await expect(progress).toHaveCount(0);
    const order = await actions(page);
    expect(order.indexOf("begin")).toBeLessThan(order.indexOf("save_session"));
    expect(order.indexOf("save_session")).toBeLessThan(order.indexOf("finish"));
    expect(order.indexOf("finish")).toBeLessThan(order.indexOf("resume"));
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
    void (window as any).__nativeTest.emitEvent("tauri://close-requested");
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
    void (window as any).__nativeTest.emitEvent("tauri://close-requested");
  });
  await expect.poll(() => actions(page)).toContain("plugin:window|destroy");
});

for (const trigger of ["close button", "native close request"]) {
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
      if (trigger === "close button")
        await page.getByRole("button", { name: "Close window" }).click();
      else
        await page.evaluate(() => {
          void (window as any).__nativeTest.emitEvent(
            "tauri://close-requested",
          );
        });
    };
    const dialog = page.getByRole("dialog", { name: "Quit SimpleBench?" });
    await close();
    await expect(dialog).toContainText(
      "Chat AI is still generating a response",
    );
    await expect(
      dialog.getByRole("button", { name: "Cancel", exact: true }),
    ).toBeFocused();
    expect(await page.evaluate(() => (window as any).__chatTest.stops)).toBe(0);
    expect(await actions(page)).not.toContain("plugin:window|destroy");
    await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__nativeTest.androidPreparation),
      )
      .toBeNull();
    expect(await page.evaluate(() => (window as any).__chatTest.stops)).toBe(0);
    await close();
    await expect(dialog).toBeVisible();
    await page.evaluate(() => {
      void (window as any).__nativeTest.emitEvent("tauri://close-requested");
    });
    await expect(page.getByRole("dialog")).toHaveCount(1);
    if (trigger === "close button") {
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
  await page.getByRole("button", { name: "Close window" }).click();
  await expect.poll(() => actions(page)).toContain("plugin:window|destroy");
  await expect(
    page.getByRole("dialog", { name: "Quit SimpleBench?" }),
  ).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__chatTest.stops)).toBe(0);
});
