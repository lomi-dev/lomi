import { expect, test } from "@playwright/test";
import { buffer, mockDesktop, openProjectIfNeeded } from "./desktop";
import { mockChats } from "./chat-mock";

for (const mode of ["dark", "light"] as const) {
  test(`Lomi ${mode} dialogs retain keyboard guards and fit a compact window`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({ colorScheme: mode });
    await mockDesktop(page);
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__nativeTest.sessions.size),
      )
      .toBe(1);
    const pane = (await page
      .locator("[data-pane-id]")
      .first()
      .getAttribute("data-pane-id"))!;
    await page.evaluate(() => {
      const native = (window as any).__nativeTest;
      const id = [...native.sessions.keys()][0];
      native.busyTerminals = [id];
      native.emit(
        id,
        "\x1b[2J\x1b[H\x1b[1mLomi workspace\x1b[0m\r\n\r\n\x1b[32m✓\x1b[0m Dependencies installed\r\n\x1b[32m✓\x1b[0m Application ready\r\n\r\n\x1b[34m~/workspace\x1b[0m  main\r\n$ pnpm dev\r\n\r\n  Local:   http://localhost:1420/\r\n  Ready in 240 ms\r\n",
      );
    });
    await expect.poll(() => buffer(page, pane)).toContain("Ready in 240 ms");
    await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
    await expect(page.locator(".terminal-pane")).toHaveCSS(
      "background-color",
      mode === "dark" ? "rgb(25, 26, 29)" : "rgb(255, 255, 255)",
    );
    await page.screenshot({
      path: testInfo.outputPath(`terminal-${mode}.png`),
    });
    await page.evaluate(() =>
      (window as any).__TAURI_INTERNALS__.invoke("request_quit"),
    );
    const dialog = page.getByRole("dialog", { name: "Quit Lomi?" });
    await expect(
      dialog.getByRole("button", { name: "Cancel", exact: true }),
    ).toBeFocused();
    await expect(dialog.locator(".modal-symbol")).toBeVisible();
    await expect(
      dialog.getByRole("button", { name: "Quit anyway" }),
    ).not.toHaveCSS("background-color", "rgb(200, 255, 61)");
    await page.screenshot({ path: testInfo.outputPath(`alert-${mode}.png`) });
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "close_terminal",
        ),
      ),
    ).toHaveLength(0);

    await page
      .getByRole("button", { name: "README.md", exact: true })
      .dblclick();
    await page.locator(".cm-content").fill("Keep this unsaved work");
    await page.keyboard.press("Control+w");
    const save = page.getByRole("dialog", {
      name: "Save changes before closing?",
    });
    await expect(
      save.getByRole("button", { name: "Save changes", exact: true }),
    ).toBeFocused();
    await page.screenshot({ path: testInfo.outputPath(`save-${mode}.png`) });
    await page.setViewportSize({ width: 560, height: 420 });
    for (const name of ["Cancel", "Discard changes", "Save changes"]) {
      await expect(
        save.getByRole("button", { name, exact: true }),
      ).toBeInViewport({ ratio: 1 });
    }
    const bounds = await save.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.y).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(560);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(420);
    await page.screenshot({
      path: testInfo.outputPath(`save-compact-${mode}.png`),
    });
    await page.keyboard.press("Escape");
    await expect(save).toHaveCount(0);
    await expect(page.locator(".cm-content")).toHaveText(
      "Keep this unsaved work",
    );
  });
}

test("partial custom themes retain primary states, borders and window geometry", async ({
  page,
}) => {
  await mockDesktop(page);
  await mockChats(page);
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-theme-manifests",
      JSON.stringify({
        custom: {
          version: 2,
          name: "Custom controls",
          appearance: "adaptive",
          common: {
            tokens: {
              "--color-primary": "#2244aa",
              "--color-primary-text": "#ffffff",
              "--color-outline": "#cc4411",
              "--input-background": "#7a8576",
              "--radius-window": "18px",
              "--radius-tab": "12px",
            },
          },
        },
      }),
    );
    localStorage.setItem(
      "test-theme-settings",
      JSON.stringify({ version: 1, active: "custom", appearance: "system" }),
    );
  });
  await page.goto("/");
  await openProjectIfNeeded(page);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "custom");
  await expect(page.locator(".app-shell")).toHaveCSS("border-radius", "18px");
  await expect(page.locator(".tab").first()).toHaveCSS("border-radius", "12px");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Chat AI", exact: true }).click();
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Check custom appearance");
  await input.blur();
  const composer = page.locator(".chat-composer-surface");
  const send = page.getByRole("button", { name: "Send", exact: true });
  await expect(send).toBeEnabled();
  for (const colorScheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme });
    await expect(page.locator("html")).toHaveAttribute(
      "data-appearance",
      colorScheme,
    );
    await expect(composer).toHaveCSS("border-top-color", "rgb(204, 68, 17)");
    await expect(composer).toHaveCSS("background-color", "rgb(122, 133, 118)");
    await send.hover();
    await expect(send).toHaveCSS("background-color", "rgb(34, 68, 170)");
    await expect(send).toHaveCSS("color", "rgb(255, 255, 255)");
  }
});

test("dark settings keep text and fields readable with distinct navigation and popup surfaces", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page);
  await page.goto("/?window=settings&page=themes");
  const field = page.getByRole("searchbox", {
    name: "Search themes",
    exact: true,
  });
  await expect(field).toBeVisible();
  const surfaces = await field.evaluate((field) => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const context = canvas.getContext("2d")!;
    const color = (value: string) => {
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = value;
      context.fillRect(0, 0, 1, 1);
      return Array.from(context.getImageData(0, 0, 1, 1).data);
    };
    const background = (element: Element): number[] => {
      const own = color(getComputedStyle(element).backgroundColor);
      if (own[3] === 255 || !element.parentElement) return own;
      const parent = background(element.parentElement);
      const alpha = own[3] / 255;
      return own.map((channel, index) =>
        index === 3 ? 255 : channel * alpha + parent[index] * (1 - alpha),
      );
    };
    const luminance = (rgb: number[]) =>
      rgb.slice(0, 3).reduce((sum, value, index) => {
        const channel = value / 255;
        return (
          sum +
          (channel <= 0.04045
            ? channel / 12.92
            : ((channel + 0.055) / 1.055) ** 2.4) *
            [0.2126, 0.7152, 0.0722][index]
        );
      }, 0);
    const contrast = (first: number[], second: number[]) => {
      const a = luminance(first);
      const b = luminance(second);
      return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
    };
    const primary = document.querySelector(".themes-page h1, .themes-page h2")!;
    const secondary = document.querySelector(".settings-navigation-subtitle")!;
    const content = document.querySelector(".themes-page")!;
    const navigation = document.querySelector(".settings-navigation")!;
    const border = color(getComputedStyle(field).borderTopColor);
    return {
      primaryContrast: contrast(
        color(getComputedStyle(primary).color),
        background(primary),
      ),
      secondaryContrast: contrast(
        color(getComputedStyle(secondary).color),
        background(secondary),
      ),
      fieldBorderContrast: contrast(border, background(field)),
      surroundingBorderContrast: contrast(
        border,
        background(field.parentElement!),
      ),
      content: background(content).slice(0, 3),
      navigation: background(navigation).slice(0, 3),
    };
  });
  expect(surfaces.primaryContrast).toBeGreaterThanOrEqual(4.5);
  expect(surfaces.secondaryContrast).toBeGreaterThanOrEqual(4.5);
  expect(surfaces.fieldBorderContrast).toBeGreaterThanOrEqual(3);
  expect(surfaces.surroundingBorderContrast).toBeGreaterThanOrEqual(3);
  expect(surfaces.content).not.toEqual(surfaces.navigation);
  await page.keyboard.press("Control+Shift+p");
  const popup = page.getByRole("dialog", { name: "Commands", exact: true });
  await expect(popup).toBeVisible();
  const popupBackground = await popup.evaluate((element) => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const context = canvas.getContext("2d")!;
    context.fillStyle = getComputedStyle(element).backgroundColor;
    context.fillRect(0, 0, 1, 1);
    return Array.from(context.getImageData(0, 0, 1, 1).data).slice(0, 3);
  });
  expect(popupBackground).not.toEqual(surfaces.content);
  expect(popupBackground).not.toEqual(surfaces.navigation);
});
