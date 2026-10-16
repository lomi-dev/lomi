import { expect, type Page } from "@playwright/test";

export const notificationInbox = (page: Page) =>
  page.getByRole("dialog", { name: "Notifications", exact: true });

export async function openNotifications(page: Page) {
  await page
    .locator(
      ".titlebar .titlebar-account-avatar, .titlebar .titlebar-account-menu-trigger",
    )
    .click();
  await page
    .getByRole("menuitem", { name: "Notifications", exact: true })
    .click();
  await expect(notificationInbox(page)).toBeVisible();
  return notificationInbox(page);
}

export async function expectRecordedNotification(
  page: Page,
  body: string | RegExp,
) {
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.notifications.items
          .map((item: any) => item.body)
          .join("\n"),
      ),
    )
    .toMatch(body);
  await expect(page.locator(".app-toast, .pane-limit-notice")).toHaveCount(0);
}

export async function dismissRecordedNotification(page: Page, body: string) {
  const inbox = await openNotifications(page);
  const entry = inbox.locator(".notification-item").filter({ hasText: body });
  await entry.getByRole("button", { name: /^Dismiss / }).click();
  await expect(entry).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(inbox).toHaveCount(0);
}
