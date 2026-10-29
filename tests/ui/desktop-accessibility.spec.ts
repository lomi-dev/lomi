import { expect, test, type Locator } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { openNotifications } from "./application-notification-helpers";

async function focusContrast(target: Locator, surface: Locator) {
  const foreground = await target.evaluate(
    (element) => getComputedStyle(element).outlineColor,
  );
  const background = await surface.evaluate(
    (element) => getComputedStyle(element).backgroundColor,
  );
  const luminance = (color: string) => {
    const [r, g, b] = color
      .match(/[\d.]+/g)!
      .slice(0, 3)
      .map(Number)
      .map((value) => {
        const channel = value / 255;
        return channel <= 0.04045
          ? channel / 12.92
          : ((channel + 0.055) / 1.055) ** 2.4;
      });
    return r * 0.2126 + g * 0.7152 + b * 0.0722;
  };
  const light = luminance(foreground);
  const dark = luminance(background);
  return (Math.max(light, dark) + 0.05) / (Math.min(light, dark) + 0.05);
}

test("light appearance keeps command search and inbox keyboard focus visible", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.keyboard.press("Control+Shift+p");
  const dialog = page.getByRole("dialog", { name: "Commands", exact: true });
  await expect(
    dialog.getByRole("textbox", { name: "Find command" }),
  ).toBeFocused();
  const search = dialog.locator(".command-picker-search");
  expect(await focusContrast(search, search)).toBeGreaterThanOrEqual(3);
  await page.keyboard.press("Escape");
  const inbox = await openNotifications(page);
  await page.keyboard.press("Tab");
  const close = inbox.getByRole("button", {
    name: "Close notifications",
    exact: true,
  });
  await close.focus();
  await expect(close).toBeFocused();
  expect(await focusContrast(close, inbox)).toBeGreaterThanOrEqual(3);
});

for (const mode of ["dark", "light"] as const) {
  test(`compact commands keep focus, results and footer inside the ${mode} Lomi theme`, async ({
    page,
  }, testInfo) => {
    await mockDesktop(page);
    await page.emulateMedia({ colorScheme: mode });
    await page.setViewportSize({ width: 800, height: 420 });
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    await page.keyboard.press("Control+Shift+p");
    const dialog = page.getByRole("dialog", {
      name: "Commands",
      exact: true,
    });
    await expect(
      dialog.getByRole("textbox", { name: "Find command" }),
    ).toBeFocused();
    for (const target of [dialog, dialog.locator(".command-picker-footer")]) {
      const bounds = (await target.boundingBox())!;
      expect(bounds.y).toBeGreaterThanOrEqual(0);
      expect(bounds.y + bounds.height).toBeLessThanOrEqual(420);
    }
    await expect(
      dialog.getByRole("button", { name: "Close dialog", exact: true }),
    ).toBeInViewport({ ratio: 1 });
    await dialog
      .getByRole("textbox", { name: "Find command" })
      .fill("Open settings");
    await expect(dialog.locator(".command-picker-list > button")).toHaveCount(
      1,
    );
    await expect(
      dialog.locator(".command-picker-list > button"),
    ).toBeInViewport({ ratio: 1 });
    await page.screenshot({
      path: testInfo.outputPath(`commands-lomi-${mode}.png`),
    });
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  });
}
