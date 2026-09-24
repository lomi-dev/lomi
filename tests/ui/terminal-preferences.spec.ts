import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { buffer, chooseOption, mockDesktop } from "./desktop";

async function settingsPage(page: Page) {
  await mockDesktop(page, false);
  await page.goto("/?window=settings&page=terminal");
  await expect(
    page.getByRole("spinbutton", { name: "Font size", exact: true }),
  ).toBeEnabled();
}
async function field(page: Page, label: string, value: string) {
  const input = page.getByLabel(label, { exact: true });
  await expect(input).toBeEnabled();
  await input.fill(value);
  await input.press("Enter");
  await expect(page.getByRole("status")).toHaveText("Saved");
  await expect(input).toBeEnabled();
}
async function options(page: Page, id: string) {
  return page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(id);
    if (!runtime) return null;
    const { fontSize, fontFamily, scrollback, cursorStyle, theme } =
      runtime.terminal.options;
    return {
      fontSize,
      fontFamily,
      scrollback,
      cursorStyle,
      theme,
      session: runtime.sessionId,
      cols: runtime.terminal.cols,
    };
  }, id);
}
async function savedPreferences(page: Page) {
  return page.evaluate(() => {
    const value = localStorage.getItem("test-terminal-preferences");
    return value ? JSON.parse(value) : null;
  });
}
async function settingsContentFits(page: Page) {
  return page.evaluate(() => {
    const settings = document.querySelector(".terminal-settings-page")!;
    return (
      document.documentElement.scrollWidth <= innerWidth &&
      settings.scrollWidth <= settings.clientWidth
    );
  });
}

test("settings update visible and hidden terminals without restarting PTYs and survive reload", async ({
  page,
  context,
}, testInfo) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  const first = (await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  const before = (await options(page, first))!;
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  const second = (await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  const settings = await context.newPage();
  await settings.emulateMedia({ colorScheme: "dark" });
  await settingsPage(settings);
  await expect(
    settings.getByRole("combobox", { name: "Font", exact: true }),
  ).toBeVisible();
  await expect(
    settings.getByRole("switch", {
      name: "Always show terminal titles",
      exact: true,
    }),
  ).toBeVisible();
  await settings.setViewportSize({ width: 920, height: 680 });
  await settings.screenshot({
    path: testInfo.outputPath("terminal-settings-default-dark.png"),
  });
  await settings.emulateMedia({ colorScheme: "light" });
  await expect
    .poll(() => settings.locator("html").getAttribute("data-appearance"))
    .toBe("light");
  await settings.screenshot({
    path: testInfo.outputPath("terminal-settings-default-light.png"),
  });
  await settings.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(() => settings.locator("html").getAttribute("data-appearance"))
    .toBe("dark");
  await chooseOption(
    settings.getByRole("combobox", { name: "Font", exact: true }),
    "Custom font…",
  );
  await field(settings, "Font family", "monospace");
  await field(settings, "Font size", "22");
  await settings.getByText("Advanced settings", { exact: true }).click();
  await field(settings, "Line height", "1.35");
  await chooseOption(
    settings.getByLabel("Cursor style", { exact: true }),
    "Block",
  );
  await expect(
    settings.getByLabel("Cursor style", { exact: true }),
  ).toBeEnabled();
  await settings.getByText("Customize colors", { exact: true }).click();
  await field(settings, "Background", "#102030");
  await field(settings, "Text", "#f0e0d0");
  await settings
    .getByText("ANSI palette and search colors", { exact: true })
    .click();
  await field(settings, "Red", "#ff1234");
  await field(settings, "Scrollback lines", "12000");
  for (const id of [first, second]) {
    await expect
      .poll(() => options(page, id))
      .toMatchObject({
        fontSize: 22,
        fontFamily:
          '"JetBrains Mono", "Noto Sans Symbols", "Noto Sans Symbols 2", "Symbols Nerd Font Mono", monospace',
        cursorStyle: "block",
        scrollback: 12000,
        theme: {
          background: "#10203000",
          foreground: "#f0e0d0ff",
          red: "#ff1234ff",
        },
      });
  }
  expect((await options(page, first))!.session).toBe(before.session);
  await page.evaluate(
    (session) =>
      (window as any).__nativeTest.emit(
        session,
        "\r\nOutput after settings change\r\n",
      ),
    before.session,
  );
  await expect
    .poll(() => buffer(page, first))
    .toContain("Output after settings change");
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await expect(page.locator(".terminal-pane")).toHaveCSS(
    "background-color",
    "rgb(16, 32, 48)",
  );
  expect((await options(page, first))!.cols).toBeLessThan(before.cols);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(2);
  await page.bringToFront();
  await page.locator(".xterm-helper-textarea").focus();
  await expect(page.locator(".xterm-helper-textarea")).toBeFocused();
  await page.keyboard.type("still typing");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) => call.command === "write_terminal")
          .map((call: any) => call.args.data)
          .join(""),
      ),
    )
    .toBe("still typing");
  expect(
    await settings.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(0);
  await settings.reload();
  await expect(settings.getByLabel("Font size", { exact: true })).toHaveValue(
    "22",
  );
  await expect(settings.getByLabel("Background", { exact: true })).toHaveValue(
    "#102030",
  );
  await page.reload();
  await expect
    .poll(() => options(page, first))
    .toMatchObject({ fontSize: 22, scrollback: 12000 });
  await settings.setViewportSize({ width: 920, height: 680 });
  await settings.screenshot({
    path: testInfo.outputPath("terminal-settings-custom-dark.png"),
  });
  await settings.emulateMedia({ colorScheme: "light" });
  await expect
    .poll(() => settings.locator("html").getAttribute("data-appearance"))
    .toBe("light");
  await settings.screenshot({
    path: testInfo.outputPath("terminal-settings-custom-light.png"),
  });
  await settings.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(() => settings.locator("html").getAttribute("data-appearance"))
    .toBe("dark");
  await settings.screenshot({
    path: testInfo.outputPath("terminal-settings.png"),
  });
  await settings.setViewportSize({ width: 560, height: 420 });
  expect(await settingsContentFits(settings)).toBe(true);
  await settings.screenshot({
    path: testInfo.outputPath("terminal-settings-small.png"),
  });
});

test("appearance overrides survive theme changes and resetting follows the active theme", async ({
  page,
  context,
}) => {
  await settingsPage(page);
  await field(page, "Font size", "20");
  await page.getByText("Customize colors", { exact: true }).click();
  await page
    .getByText("ANSI palette and search colors", { exact: true })
    .click();
  await field(page, "Red", "#ff1234");
  const main = await context.newPage();
  await mockDesktop(main, false);
  await main.goto("/");
  await expect(main.locator(".terminal-host")).toHaveCSS("opacity", "1");
  const id = (await main
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  for (const target of [page, main])
    await target.evaluate(async () => {
      localStorage.setItem(
        "test-theme-manifests",
        JSON.stringify({
          test: {
            version: 2,
            name: "Test",
            common: { terminal: { fontSize: 17, colors: { red: "#aabbcc" } } },
          },
        }),
      );
      localStorage.setItem(
        "test-theme-settings",
        JSON.stringify({ version: 1, active: "test", appearance: "dark" }),
      );
      const { prepareTheme } = await import("/src/theme/runtime.ts");
      const prepared = await prepareTheme(
        {
          id: "test",
          revision: "test",
          directory: "/app/themes/test",
          raw: JSON.stringify({
            version: 2,
            name: "Test",
            common: { terminal: { fontSize: 17, colors: { red: "#aabbcc" } } },
          }),
        },
        { version: 1, active: "test", appearance: "dark" },
      );
      await prepared.commit();
    });
  await expect
    .poll(() => options(main, id))
    .toMatchObject({ fontSize: 20, theme: { red: "#ff1234ff" } });
  await page
    .getByRole("button", {
      name: "Restore theme default for Font size",
      exact: true,
    })
    .click();
  await expect(page.getByLabel("Font size", { exact: true })).toHaveValue("17");
  await expect
    .poll(() => options(main, id))
    .toMatchObject({ fontSize: 17, theme: { red: "#ff1234ff" } });
  await page
    .getByRole("button", { name: "Reset defaults", exact: true })
    .click();
  await expect
    .poll(() => options(main, id))
    .toMatchObject({ fontSize: 17, theme: { red: "#aabbccff" } });
});

test("font choices, collapsed settings, and the CSS preview follow saved appearance", async ({
  page,
}, testInfo) => {
  await mockDesktop(page, false);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/?window=settings&page=terminal");
  await expect(page.getByLabel("Font size", { exact: true })).toBeEnabled();

  const font = page.getByRole("combobox", { name: "Font", exact: true });
  await expect(font).toHaveText("Theme default");
  await expect(page.getByLabel("Font family", { exact: true })).toHaveCount(0);
  await expect(
    page.getByRole("switch", {
      name: "Always show terminal titles",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("switch", { name: "Agent notifications", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Configure Claude Code…", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Reset defaults", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("More settings", { exact: true })).toHaveCount(0);
  await expect(page.getByLabel("Line height", { exact: true })).toBeHidden();
  await expect(
    page.getByLabel("Scrollback lines", { exact: true }),
  ).toBeHidden();
  await expect(page.getByLabel("Background", { exact: true })).toBeHidden();
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(0);

  await page.setViewportSize({ width: 560, height: 420 });
  await expect.poll(() => settingsContentFits(page)).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("terminal-settings-font-default-dark-small.png"),
  });
  await page.setViewportSize({ width: 920, height: 680 });
  await page.locator(".terminal-settings-page").evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await page.screenshot({
    path: testInfo.outputPath("terminal-settings-default-lower-dark.png"),
  });
  await page.locator(".terminal-settings-page").evaluate((element) => {
    element.scrollTop = 0;
  });
  await page.setViewportSize({ width: 560, height: 420 });
  await page.emulateMedia({ colorScheme: "light" });
  await expect
    .poll(() => page.locator("html").getAttribute("data-appearance"))
    .toBe("light");
  await page.screenshot({
    path: testInfo.outputPath("terminal-settings-font-default-light-small.png"),
  });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(() => page.locator("html").getAttribute("data-appearance"))
    .toBe("dark");

  await chooseOption(font, "Custom font…");
  await expect(page.getByLabel("Font family", { exact: true })).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "save_terminal_preferences",
        ).length,
    ),
  ).toBe(0);
  expect(await savedPreferences(page)).toBeNull();
  await field(page, "Font family", "Fira Code, monospace");
  await expect
    .poll(async () => (await savedPreferences(page))?.appearance?.fontFamily)
    .toBe("Fira Code, monospace");
  await page.reload();
  await expect(
    page.getByRole("combobox", { name: "Font", exact: true }),
  ).toHaveText("Custom font…");
  await expect(page.getByLabel("Font family", { exact: true })).toHaveValue(
    "Fira Code, monospace",
  );
  await expect(page.getByLabel("Font family", { exact: true })).toBeEnabled();

  await page.evaluate(() => {
    (window as any).__nativeTest.failTerminalPreferencesSave = true;
  });
  await page
    .getByRole("button", {
      name: "Restore theme default for Font family",
      exact: true,
    })
    .click();
  await expect(page.getByRole("alert")).toContainText("Disk is full");
  await expect(
    page.getByRole("combobox", { name: "Font", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByRole("combobox", { name: "Font", exact: true }),
  ).toHaveText("Custom font…");
  await expect(page.getByLabel("Font family", { exact: true })).toHaveValue(
    "Fira Code, monospace",
  );
  await expect
    .poll(async () => (await savedPreferences(page))?.appearance?.fontFamily)
    .toBe("Fira Code, monospace");
  await page.evaluate(() => {
    (window as any).__nativeTest.failTerminalPreferencesSave = false;
  });
  await page.getByRole("button", { name: "Dismiss", exact: true }).click();
  await page
    .getByRole("button", {
      name: "Restore theme default for Font family",
      exact: true,
    })
    .click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  await expect(
    page.getByRole("combobox", { name: "Font", exact: true }),
  ).toHaveText("Theme default");
  expect((await savedPreferences(page)).appearance).not.toHaveProperty(
    "fontFamily",
  );

  await chooseOption(
    page.getByRole("combobox", { name: "Font", exact: true }),
    "JetBrains Mono",
  );
  await expect(page.getByRole("status")).toHaveText("Saved");
  await expect
    .poll(async () => (await savedPreferences(page))?.appearance?.fontFamily)
    .toContain("JetBrains Mono");
  await chooseOption(
    page.getByRole("combobox", { name: "Font", exact: true }),
    "Theme default",
  );
  await expect(page.getByRole("status")).toHaveText("Saved");
  expect((await savedPreferences(page)).appearance).not.toHaveProperty(
    "fontFamily",
  );

  await chooseOption(
    page.getByRole("combobox", { name: "Font", exact: true }),
    "Custom font…",
  );
  await field(page, "Font family", "Fira Code, monospace");
  await field(page, "Font size", "21");
  await chooseOption(page.getByLabel("Cursor style", { exact: true }), "Block");
  await expect(page.getByRole("status")).toHaveText("Saved");
  await page.getByText("Customize colors", { exact: true }).click();
  await field(page, "Background", "#102030");
  await field(page, "Text", "#f0e0d0");

  const preview = page.getByRole("img", {
    name: "Terminal preview showing Welcome to Lomi and a prompt",
    exact: true,
  });
  await expect(preview).toBeVisible();
  await expect(preview).toHaveCSS("font-family", /Fira Code/);
  await expect(preview).toHaveCSS("font-size", "21px");
  await expect(preview).toHaveCSS("background-color", "rgb(16, 32, 48)");
  await expect(preview).toHaveCSS("color", "rgb(240, 224, 208)");
  const previewCursor = preview.locator(".terminal-preview-cursor");
  await expect(previewCursor).toBeVisible();
  await expect
    .poll(async () =>
      Number.parseFloat(
        await previewCursor.evaluate(
          (element) => getComputedStyle(element).width,
        ),
      ),
    )
    .toBeGreaterThan(1);

  await page.getByText("Customize colors", { exact: true }).click();
  await page.setViewportSize({ width: 920, height: 680 });
  await page.locator(".terminal-settings-page").evaluate((element) => {
    element.scrollTop = 0;
  });
  await page.screenshot({
    path: testInfo.outputPath("terminal-settings-font-custom-dark.png"),
  });
  await page.emulateMedia({ colorScheme: "light" });
  await expect
    .poll(() => page.locator("html").getAttribute("data-appearance"))
    .toBe("light");
  await page.screenshot({
    path: testInfo.outputPath("terminal-settings-font-custom-light.png"),
  });
  await page.setViewportSize({ width: 560, height: 420 });
  await page.locator(".terminal-settings-page").evaluate((element) => {
    element.scrollTop = 0;
  });
  await expect.poll(() => settingsContentFits(page)).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("terminal-settings-font-custom-small.png"),
  });

  await page
    .getByRole("button", { name: "Reset defaults", exact: true })
    .click();
  await expect(page.getByRole("status")).toHaveText("Saved");
  expect((await savedPreferences(page)).appearance).not.toHaveProperty(
    "fontFamily",
  );
  await expect(
    page.getByRole("combobox", { name: "Font", exact: true }),
  ).toHaveText("Theme default");
});

test("failed saves retain working preferences and corrupt files require explicit recovery", async ({
  page,
}) => {
  await settingsPage(page);
  await field(page, "Font size", "19");
  await page.evaluate(() => {
    (window as any).__nativeTest.failTerminalPreferencesSave = true;
  });
  await page.getByLabel("Font size", { exact: true }).fill("23");
  await page.getByLabel("Font size", { exact: true }).press("Enter");
  await expect(page.getByRole("alert")).toContainText("Disk is full");
  await expect(page.getByLabel("Font size", { exact: true })).toHaveValue("19");
  expect(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("test-terminal-preferences")!)
          .appearance.fontSize,
    ),
  ).toBe(19);
  await page.evaluate(() => {
    (window as any).__nativeTest.failTerminalPreferencesSave = false;
    localStorage.setItem(
      "test-terminal-preferences",
      '{"version":99,"future":"preserve"}',
    );
    return (window as any).__nativeTest.emitEvent(
      "terminal-preferences-changed",
    );
  });
  await expect(page.getByRole("alert")).toContainText("left intact");
  await expect(page.getByLabel("Font size", { exact: true })).toBeDisabled();
  await page
    .getByRole("button", { name: "Retry loading", exact: true })
    .click();
  expect(
    await page.evaluate(() =>
      localStorage.getItem("test-terminal-preferences"),
    ),
  ).toBe('{"version":99,"future":"preserve"}');
  await page
    .getByRole("button", { name: "Reset defaults", exact: true })
    .click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByLabel("Font size", { exact: true })).toHaveValue("16");
});
