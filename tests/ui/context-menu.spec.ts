import { expect, test } from "@playwright/test";
import type { Locator } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function expectBrowserMenuBlocked(target: Locator) {
  await expect(target).toBeVisible();
  expect(
    await target.evaluate((element) =>
      element.dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
          button: 2,
        }),
      ),
    ),
  ).toBe(false);
}

test("the main window blocks browser menus and preserves custom tab actions", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expectBrowserMenuBlocked(page.locator(".xterm-screen"));
  await expectBrowserMenuBlocked(page.locator(".titlebar"));
  await expectBrowserMenuBlocked(page.locator("body"));
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  await expectBrowserMenuBlocked(page.locator(".cm-content"));
  await page.getByRole("tab", { name: "README.md", exact: true }).click({
    button: "right",
  });
  const menu = page.getByRole("menu", { name: "Tab actions" });
  await expect(menu).toBeVisible();
  await expectBrowserMenuBlocked(menu);
  await menu.getByRole("menuitem", { name: "Close", exact: true }).click();
  await expect(page.getByRole("tab")).toHaveCount(1);
  await expect(page.locator(".xterm-screen")).toBeVisible();
});

test("settings blocks browser menus on its background and form fields", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/?window=settings&page=terminal");
  await expectBrowserMenuBlocked(page.getByLabel("Font size", { exact: true }));
  await expectBrowserMenuBlocked(page.locator(".titlebar"));
  await expectBrowserMenuBlocked(page.locator("body"));
});
