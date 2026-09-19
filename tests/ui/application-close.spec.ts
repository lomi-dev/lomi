import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

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
