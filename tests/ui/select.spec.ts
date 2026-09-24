import { expect, test } from "@playwright/test";
import { newProject, newSession } from "../../src/model";
import { mockDesktop } from "./desktop";

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
});

test("selects support keyboard navigation, cancellation, typeahead and small windows", async ({
  page,
}, testInfo) => {
  await mockDesktop(page, false);
  await page.goto("/?window=settings&page=themes");
  const status = page.getByRole("combobox", { name: "Theme status" });
  await status.click();
  await expect(
    page.getByRole("option", { name: "All themes", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await status.press("End");
  const last = page.getByRole("option", {
    name: "Needs attention",
    exact: true,
  });
  await expect(last).toBeInViewport();
  await expect(status).toHaveAttribute(
    "aria-activedescendant",
    (await last.getAttribute("id"))!,
  );
  await status.press("Escape");
  await expect(page.getByRole("listbox")).toHaveCount(0);
  await expect(status).toHaveText("All themes");
  await expect(status).toBeFocused();
  await status.click();
  await status.click();
  await expect(page.getByRole("listbox")).toHaveCount(0);
  await status.press("n");
  await status.press("Enter");
  await expect(status).toHaveText("Needs attention");
  await status.press("Home");
  await status.press("Enter");
  await expect(status).toHaveText("All themes");
  await expect(page.getByRole("listbox")).toHaveCount(0);
  await page.setViewportSize({ width: 560, height: 420 });
  await status.click();
  await status.press("End");
  await expect(last).toBeInViewport();
  const bounds = (await page.getByRole("listbox").boundingBox())!;
  expect(bounds.y).toBeGreaterThanOrEqual(0);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(420);
  await page.screenshot({
    path: testInfo.outputPath("select-small-window.png"),
  });
  await page.getByRole("heading", { name: "Themes", exact: true }).click();
  await expect(page.getByRole("listbox")).toHaveCount(0);
  await expect(status).toHaveText("All themes");
});

test("theme dropdowns stay above a scrolling dialog and Enter never submits the form", async ({
  page,
}, testInfo) => {
  await mockDesktop(page, false);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-theme-manifests",
      JSON.stringify({
        custom: {
          version: 1,
          name: "Custom",
          tokens: { "--radius-control": "9px" },
        },
      }),
    );
  });
  await page.goto("/?window=settings&page=themes");
  await page
    .getByRole("button", { name: "Use Custom theme", exact: true })
    .click();
  await page.getByRole("button", { name: "Edit theme", exact: true }).click();
  const dialog = page.getByRole("dialog");
  const tabs = dialog.getByRole("combobox", {
    name: "Tab placement",
    exact: true,
  });
  await tabs.click();
  await tabs.press("ArrowDown");
  await tabs.press("Escape");
  await expect(dialog).toBeVisible();
  await expect(tabs).toHaveText("Inline");
  await tabs.press("b");
  await tabs.press("Enter");
  await expect(tabs).toHaveText("Below");
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "save_theme_manifest",
      ),
    ),
  ).toHaveLength(0);
  await page.setViewportSize({ width: 560, height: 420 });
  await tabs.click();
  const above = page.getByRole("option", { name: "Above", exact: true });
  await expect(above).toBeInViewport();
  await page.screenshot({
    path: testInfo.outputPath("select-theme-dialog-dark.png"),
  });
  await page.emulateMedia({ colorScheme: "light" });
  await page.screenshot({
    path: testInfo.outputPath("select-theme-dialog-light.png"),
  });
  await above.click();
  await expect(tabs).toHaveText("Above");
  await expect(tabs).toHaveCSS("border-radius", "9px");
  await expect(page.getByRole("listbox")).toHaveCount(0);
});

test("unavailable environments cannot be selected and choosing does not restart terminals", async ({
  page,
}) => {
  const project = newProject("/project", "missing:shell");
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Shell unavailable" }),
  ).toBeVisible();
  await page.keyboard.press("Control+Shift+l");
  const select = page.getByRole("combobox", { name: "Terminal environment" });
  await expect(select).toBeFocused();
  await select.click();
  const missing = page.getByRole("option", { name: "Unavailable environment" });
  await expect(missing).toHaveAttribute("aria-disabled", "true");
  await missing.click({ force: true });
  await expect(select).toHaveText("Unavailable environment");
  await select.press("Home");
  await select.press("Enter");
  await expect(select).toHaveText("Local · bash");
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Restart terminals" }),
  ).toBeEnabled();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "close_terminal",
      ),
    ),
  ).toHaveLength(0);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
});
