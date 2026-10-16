import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { openNotifications } from "./application-notification-helpers";

test("failed notification writes retain a bounded normalized queue and report overflow on retry", async ({
  page,
}) => {
  test.setTimeout(120000);
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.failNotificationCommand = "record_app_notification";
    const path = "/src/app-notifications.ts";
    const { reportAppNotification } = await import(path);
    await Promise.all(
      Array.from({ length: 205 }, (_, i) =>
        reportAppNotification({
          title: "A".repeat(150),
          body: `${i}:\n${i === 204 ? "🧪".repeat(5000) : "Application failure"}`,
        }),
      ),
    );
  });
  const lengths = await page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter((call: any) => call.command === "record_app_notification")
      .map((call: any) => ({
        title: Array.from(call.args.title).length,
        body: Array.from(call.args.body).length,
        controls: /[\x00-\x1f\x7f-\x9f]/.test(call.args.body),
      })),
  );
  expect(lengths).toHaveLength(205);
  expect(
    lengths.every(
      (value: any) =>
        value.title === 100 && value.body <= 4000 && !value.controls,
    ),
  ).toBe(true);
  expect(lengths.at(-1)?.body).toBe(4000);
  const inbox = await openNotifications(page);
  await expect(inbox.getByRole("status")).toContainText(
    "Older unsaved notifications could not be retained",
  );
  await page.evaluate(() => {
    const desktop = window as any;
    desktop.__nativeTest.failNotificationCommand = null;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    let failSummary = true;
    desktop.__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: any,
      options: any,
    ) => {
      if (
        command === "record_app_notification" &&
        args.title === "Some notifications could not be saved" &&
        failSummary
      ) {
        failSummary = false;
        throw new Error("Summary storage is unavailable");
      }
      return invoke(command, args, options);
    };
  });
  await inbox.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(inbox.getByRole("status")).toContainText(
    "Summary storage is unavailable",
  );
  await expect(inbox.locator("[data-unsaved]")).toContainText(
    "5 older application notifications could not be saved",
  );
  await inbox.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(inbox.getByRole("status")).toHaveCount(0);
  const saved = await page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter((call: any) => call.command === "record_app_notification")
      .slice(205)
      .map((call: any) => call.args.body),
  );
  expect(saved).toHaveLength(201);
  expect(saved[0]).toMatch(/^5: /);
  expect(saved.at(-1)).toBe(
    "5 older application notifications could not be saved while notification storage was unavailable.",
  );
});

test("replaying unsaved notifications disables dismissal and clearing until persistence finishes", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.failNotificationCommand = "record_app_notification";
    const path = "/src/app-notifications.ts";
    const { reportAppNotification } = await import(path);
    await reportAppNotification({
      body: "Original Android failure",
      source: "android",
      action: "open-android-settings",
    });
  });
  const inbox = await openNotifications(page);
  await expect(inbox.locator("[data-unsaved]")).toContainText(
    "Original Android failure",
  );
  await expect(inbox.locator("[data-unsaved]")).toContainText("Not saved yet");
  await page.evaluate(() => {
    const desktop = window as any;
    desktop.__nativeTest.failNotificationCommand = null;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: any,
      options: any,
    ) => {
      if (command === "record_app_notification")
        await new Promise<void>((resolve) => {
          desktop.__finishNotificationWrite = resolve;
        });
      return invoke(command, args, options);
    };
  });
  await inbox.getByRole("button", { name: "Retry", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() => Boolean((window as any).__finishNotificationWrite)),
    )
    .toBe(true);
  await expect(
    inbox.getByRole("button", { name: "Clear all notifications", exact: true }),
  ).toBeDisabled();
  await expect(
    inbox.getByRole("button", {
      name: "Dismiss Android needs attention",
      exact: true,
    }),
  ).toBeDisabled();
  await page.evaluate(() => (window as any).__finishNotificationWrite());
  await expect(inbox.getByRole("status")).toHaveCount(0);
  await expect(inbox.locator("[data-unsaved]")).toHaveCount(0);
  await expect(inbox.locator(".notification-item")).toHaveCount(1);
  await expect(inbox).toContainText("Original Android failure");
});
