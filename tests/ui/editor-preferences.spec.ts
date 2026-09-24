import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("persisted editor defaults still apply to indentation and live buffers", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-editor-preferences",
      JSON.stringify({ version: 1, tabSize: 2, insertSpaces: false }),
    );
  });
  await mockDesktop(page);
  await page.goto("/");
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  const indentation = page.getByRole("button", {
    name: "Change indentation settings",
  });
  const lines = () =>
    page
      .locator(".cm-line")
      .allTextContents()
      .then((content) => content.join("\n"));
  await expect(indentation).toHaveText("Tabs: 2");
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText("first\nsecond");
  await page.keyboard.press("Control+a");
  await page.keyboard.press("Tab");
  await expect.poll(lines).toBe("\tfirst\n\tsecond");
  await page.keyboard.press("Shift+Tab");
  await expect.poll(lines).toBe("first\nsecond");

  await page.evaluate(async () => {
    localStorage.setItem(
      "test-editor-preferences",
      JSON.stringify({ version: 1, tabSize: 8, insertSpaces: true }),
    );
    await (window as any).__nativeTest.emitEvent("editor-preferences-changed");
  });
  await expect(indentation).toHaveText("Spaces: 8");
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+End");
  await page.keyboard.press("Tab");
  await expect.poll(lines).toBe("first\nsecond        ");
});

test("removed Editor settings routes fall back to Keybinds", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/?window=settings&page=editor");
  await expect(page.getByRole("heading", { name: "Keybinds" })).toBeVisible();
  await expect(
    page
      .getByRole("navigation", { name: "Settings pages" })
      .getByRole("button", { name: "Editor", exact: true }),
  ).toHaveCount(0);
  await page.evaluate(() =>
    (window as any).__nativeTest.emitEvent("settings-page-changed", "editor"),
  );
  await expect(page.getByRole("heading", { name: "Keybinds" })).toBeVisible();
});
