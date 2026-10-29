import { expect, test } from "@playwright/test";
import { mockDesktop, openProjectIfNeeded } from "./desktop";

test("Lomi is the only immutable built-in and persists, synchronizes and duplicates", async ({
  page,
  context,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page);
  await page.goto("/?window=settings&page=themes");
  const main = await context.newPage();
  await main.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(main);
  await main.goto("/");
  await openProjectIfNeeded(main);
  await expect(main.locator(".xterm-screen")).toBeVisible();
  const lomi = page.getByRole("article", { name: "Lomi", exact: true });
  await page
    .getByRole("group", { name: "Theme sources" })
    .getByRole("button", { name: "Built in", exact: true })
    .click();
  await expect(page.getByRole("article")).toHaveCount(1);
  await expect(
    lomi.getByRole("button", { name: "Use Lomi theme" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(lomi).toContainText("Default");
  await expect(lomi).toContainText("Read only");
  await expect(lomi.getByRole("button", { name: "Edit theme" })).toHaveCount(0);
  await expect(
    lomi.getByRole("button", { name: "Open theme folder" }),
  ).toHaveCount(0);
  await page.evaluate(() => document.fonts.load('600 13px "Geist Variable"'));
  expect(
    await page.evaluate(() =>
      [...document.fonts].some(
        (font) =>
          font.family.replaceAll('"', "") === "Geist Variable" &&
          font.status === "loaded",
      ),
    ),
  ).toBe(true);
  await expect(page.locator("html")).toHaveCSS("font-family", /Geist Variable/);
  await expect(lomi.getByRole("button", { name: "Use Lomi theme" })).toHaveCSS(
    "background-color",
    "rgb(200, 255, 61)",
  );
  await page.screenshot({ path: test.info().outputPath("lomi-dark.png") });

  await page.getByRole("radio", { name: "Light", exact: true }).check();
  for (const view of [page, main]) {
    await expect(view.locator("html")).toHaveAttribute("data-theme", "lomi");
    await expect(view.locator(".app-shell")).toHaveCSS(
      "background-color",
      "rgb(247, 248, 243)",
    );
  }
  await page.reload();
  await expect(
    lomi.getByRole("button", { name: "Use Lomi theme" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByRole("radio", { name: "Light", exact: true }),
  ).toBeChecked();

  await lomi.getByRole("button", { name: "Duplicate theme" }).click();
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "Edit JSON" }),
  ).toBeEnabled();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Close", exact: true })
    .click();
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("test-theme-manifests")!).copy.name,
    ),
  ).toBe("Lomi");

  await page
    .getByRole("article", { name: "Lomi", exact: true })
    .filter({
      has: page.getByRole("button", { name: "Edit theme", exact: true }),
    })
    .getByRole("button", { name: "Use Lomi theme" })
    .click();
  await expect(main.locator("html")).toHaveAttribute("data-theme", "copy");
  await page
    .getByRole("group", { name: "Theme sources" })
    .getByRole("button", { name: "Built in", exact: true })
    .click();
  await lomi.getByRole("button", { name: "Use Lomi theme" }).click();
  for (const view of [page, main]) {
    await expect(view.locator("html")).toHaveAttribute("data-theme", "lomi");
    await expect(view.locator(".app-shell")).toHaveCSS(
      "background-color",
      "rgb(247, 248, 243)",
    );
  }
  expect(
    await page.evaluate(() =>
      JSON.parse(localStorage.getItem("test-theme-settings")!),
    ),
  ).toMatchObject({ active: null, appearance: "light" });
  await expect(
    page.locator('.settings-nav-item[aria-current="page"]'),
  ).toHaveCSS("background-color", "rgb(231, 238, 219)");
  await expect(page.locator(".themes-page")).toHaveAttribute(
    "aria-busy",
    "false",
  );
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.screenshot({ path: test.info().outputPath("lomi-light.png") });
  expect(
    await main.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
  expect(
    await main.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "close_terminal",
        ).length,
    ),
  ).toBe(0);
});

test("a saved DeepMono selection falls back to Lomi and preserves light appearance", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page);
  const saved = {
    version: 1,
    active: "@builtin-deepmono",
    appearance: "light",
  };
  await page.addInitScript((saved) => {
    localStorage.setItem("test-theme-settings", JSON.stringify(saved));
  }, saved);
  await page.goto("/?window=settings&page=themes");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "lomi");
  await expect(page.locator(".app-shell")).toHaveCSS(
    "background-color",
    "rgb(247, 248, 243)",
  );
  await expect(
    page.getByRole("radio", { name: "Light", exact: true }),
  ).toBeChecked();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      JSON.parse(localStorage.getItem("test-theme-settings")!),
    ),
  ).toEqual(saved);
});
