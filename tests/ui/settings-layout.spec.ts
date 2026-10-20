import { expect, test } from "@playwright/test";
import { mockAndroid } from "./android-mock";
import { mockChats } from "./chat-mock";
import { mockDesktop } from "./desktop";

const pages = [
  ["keybinds", "Keybinds"],
  ["terminal", "Terminal"],
  ["themes", "Themes"],
  ["notifications", "Notifications"],
  ["plugins", "Plugins"],
  ["chat-ai", "Chat AI"],
  ["android", "Android"],
  ["agent-control", "Agent control"],
  ["account", "Account"],
  ["remote", "Remote"],
  ["about", "About"],
] as const;

for (const appearance of ["dark", "light"] as const) {
  for (const compact of [false, true]) {
    test(`settings pages fit ${compact ? "800×420" : "1440×900"} in ${appearance}`, async ({
      page,
    }, testInfo) => {
      await mockDesktop(page, false, null, undefined, {}, "macos");
      await mockAndroid(page);
      await mockChats(page);
      await page.emulateMedia({
        colorScheme: appearance,
        reducedMotion: compact ? "reduce" : "no-preference",
      });
      await page.setViewportSize(
        compact ? { width: 800, height: 420 } : { width: 1440, height: 900 },
      );
      await page.goto("/");
      for (const [id, title] of pages) {
        await page.evaluate(
          (id) =>
            (window as any).__TAURI_INTERNALS__.invoke("open_settings", {
              page: id,
            }),
          id,
        );
        const content = page.getByRole("main", { name: title, exact: true });
        await expect(content).toBeVisible();
        await expect(
          content.getByText("Loading…", { exact: true }),
        ).toHaveCount(0);
        await expect(page.locator("html")).toHaveAttribute(
          "data-appearance",
          appearance,
        );
        expect(
          await content.evaluate(
            (element) => element.scrollWidth <= element.clientWidth,
          ),
        ).toBe(true);
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true);
        if (id === "keybinds") {
          const toggle = page.getByRole("switch", {
            name: "Focus follows pointer",
          });
          await expect(toggle).toBeEnabled();
          if (compact) {
            expect(
              await toggle.evaluate(
                (element) => getComputedStyle(element).transitionDuration,
              ),
            ).toBe("0s");
            expect(
              await toggle.evaluate(
                (element) =>
                  getComputedStyle(element, "::before").transitionDuration,
              ),
            ).toBe("0s");
          }
        }
        await page.screenshot({
          path: testInfo.outputPath(
            `${id}-${appearance}-${compact ? "compact" : "desktop"}.png`,
          ),
          animations: "disabled",
        });
      }
    });
  }
}

test("horizontal theme navigation leaves content usable at the minimum window size", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.setViewportSize({ width: 800, height: 420 });
  await page.goto("/?window=settings&page=terminal");
  await expect(
    page.getByRole("main", { name: "Terminal", exact: true }),
  ).toBeVisible();
  for (const settingsNavigation of ["top", "bottom"] as const) {
    await page.evaluate(async (settingsNavigation) => {
      const { prepareTheme } = await import("/src/theme/runtime.ts");
      await (
        await prepareTheme(
          {
            id: "settings-layout",
            revision: "test",
            directory: "/app/themes/settings-layout",
            raw: JSON.stringify({
              version: 2,
              name: "Settings layout",
              common: { layout: { settingsNavigation } },
            }),
          },
          { version: 1, active: "settings-layout", appearance: "dark" },
        )
      ).commit();
    }, settingsNavigation);
    const navigation = page.getByRole("navigation", { name: "Settings pages" });
    await expect(
      navigation.getByRole("button", { name: "About", exact: true }),
    ).toBeInViewport();
    await expect(
      navigation.getByRole("textbox", { name: "Find a settings page" }),
    ).toBeVisible();
    expect(
      (await page
        .getByRole("main", { name: "Terminal", exact: true })
        .boundingBox())!.height,
    ).toBeGreaterThan(140);
    expect(
      await navigation.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
  }
});

test("custom themes retain scalar and shorthand settings spacing", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.goto("/?window=settings&page=terminal");
  const content = page.getByRole("main", { name: "Terminal", exact: true });
  await expect(content).toBeVisible();
  for (const padding of ["8px", "12px 20px"]) {
    await page.evaluate(async (padding) => {
      const { prepareTheme } = await import("/src/theme/runtime.ts");
      await (
        await prepareTheme(
          {
            id: "custom-spacing",
            revision: "test",
            directory: "/app/themes/custom-spacing",
            raw: JSON.stringify({
              version: 2,
              name: "Custom spacing",
              common: {
                tokens: {
                  "--settings-page-padding": padding,
                  "--settings-heading-margin": "8px",
                  "--settings-nav-width": "180px",
                },
              },
            }),
          },
          { version: 1, active: "custom-spacing", appearance: "dark" },
        )
      ).commit();
    }, padding);
    await expect(content).toHaveCSS("padding", padding);
    await expect(content.locator(".settings-page-heading")).toHaveCSS(
      "margin-bottom",
      "8px",
    );
    await expect(
      page.getByRole("navigation", { name: "Settings pages" }),
    ).toHaveCSS("width", "180px");
  }
});
