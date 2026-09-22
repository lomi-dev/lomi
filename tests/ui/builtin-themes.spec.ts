import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("Lomi is the default; both immutable built-ins persist, synchronize and duplicate", async ({
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
  await expect(main.locator(".xterm-screen")).toBeVisible();
  const lomi = page.getByRole("article", { name: "Lomi", exact: true });
  const deepmono = page.getByRole("article", { name: "DeepMono", exact: true });
  await page
    .getByRole("group", { name: "Theme sources" })
    .getByRole("button", { name: "Built in", exact: true })
    .click();
  await expect(page.getByRole("article")).toHaveCount(2);
  await expect(
    lomi.getByRole("button", { name: "Use Lomi theme" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(lomi).toContainText("Default");
  for (const card of [lomi, deepmono]) {
    await expect(card).toContainText("Read only");
    await expect(card.getByRole("button", { name: "Edit theme" })).toHaveCount(
      0,
    );
    await expect(
      card.getByRole("button", { name: "Open theme folder" }),
    ).toHaveCount(0);
  }
  await page.evaluate(() => document.fonts.load('600 13px "Manrope"'));
  expect(
    await page.evaluate(() =>
      [...document.fonts].some(
        (font) => font.family === "Manrope" && font.status === "loaded",
      ),
    ),
  ).toBe(true);
  await expect(page.locator("html")).toHaveCSS(
    "font-family",
    "Manrope, Arial, sans-serif",
  );
  await expect(lomi.getByRole("button", { name: "Use Lomi theme" })).toHaveCSS(
    "background-color",
    "rgb(200, 255, 61)",
  );
  await page.screenshot({ path: test.info().outputPath("lomi-dark.png") });

  await deepmono.getByRole("button", { name: "Use DeepMono theme" }).click();
  for (const view of [page, main]) {
    await expect(view.locator("html")).toHaveAttribute(
      "data-theme",
      "@builtin-deepmono",
    );
    await expect(view.locator(".app-shell")).toHaveCSS(
      "background-color",
      "rgb(16, 16, 16)",
    );
    await expect(view.locator("html")).not.toHaveCSS(
      "font-family",
      "Manrope, Arial, sans-serif",
    );
  }
  await expect(
    deepmono.getByRole("button", { name: "Use DeepMono theme" }),
  ).toHaveCSS("border-radius", "5px");
  await page.reload();
  await expect(
    deepmono.getByRole("button", { name: "Use DeepMono theme" }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("radio", { name: "Light", exact: true }).check();
  await expect(main.locator(".app-shell")).toHaveCSS(
    "background-color",
    "rgb(244, 244, 244)",
  );

  await deepmono.getByRole("button", { name: "Duplicate theme" }).click();
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
  ).toBe("DeepMono");

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
  ).toHaveCSS("background-color", "rgb(228, 232, 223)");
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
