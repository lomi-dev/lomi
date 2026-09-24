import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { buffer, chooseOption, mockDesktop } from "./desktop";

const theme = {
  version: 1,
  name: "Graphite Glass",
  author: "Test author",
  description: "A local image and softer corners.",
  appearance: "dark",
  tokens: {
    "--color-background": "#161616",
    "--radius-control": "12px",
    "--terminal-padding": "16px",
  },
  terminal: {
    fontSize: 17,
    cursorStyle: "underline",
    cursorBlink: false,
    colors: { foreground: "#e0e0e0", background: "#101010cc", blue: "#7a8fa6" },
  },
  backgrounds: {
    terminal: {
      image: "images/wall paper.svg",
      opacity: 0.4,
      overlay: "#10101080",
    },
  },
  styles: { ".statusbar": { height: "34px" } },
  stylesheets: ["styles/base.css", "styles/components.css"],
};

async function install(page: Page) {
  await mockDesktop(page);
  await page.addInitScript((theme) => {
    if (!localStorage.getItem("test-theme-manifests"))
      localStorage.setItem(
        "test-theme-manifests",
        JSON.stringify({ glass: theme }),
      );
  }, theme);
  await page.route("**/theme-assets/**", async (route) => {
    if (route.request().url().endsWith("/styles/base.css")) {
      await new Promise((resolve) => setTimeout(resolve, 150));
      await route.fulfill({
        contentType: "text/css",
        body: '@import "parts/controls.css"; .settings-page-heading h1 { letter-spacing: 1px; } .tab { border-radius: 8px; } :root { --terminal-letter-spacing: 2px; }',
      });
    } else if (route.request().url().endsWith("/styles/parts/controls.css")) {
      await route.fulfill({
        contentType: "text/css",
        body: '@media (min-width: 500px) { .catalog-content { background-image: url("../../images/wall%20paper.svg"); } }',
      });
    } else if (route.request().url().endsWith(".css"))
      await route.fulfill({
        contentType: "text/css",
        body: ".settings-page-heading h1 { letter-spacing: 3px; } .tab { border-radius: 14px; } :root { --terminal-letter-spacing: 1px; --terminal-font-family: ThemeFace, monospace; }",
      });
    else
      await route.fulfill({
        contentType: "image/svg+xml",
        body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"><rect width="800" height="600" fill="#303030"/><circle cx="500" cy="300" r="200" fill="#6e6e6e"/></svg>',
      });
  });
}
async function settings(page: Page) {
  await page.goto("/?window=settings");
  await page.getByRole("button", { name: "Themes", exact: true }).click();
}
async function calls(page: Page, command: string) {
  return page.evaluate(
    (command) =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === command,
      ),
    command,
  );
}
async function terminal(page: Page, pane: string) {
  return page.evaluate(async (pane) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(pane)!;
    return {
      id: runtime.sessionId,
      fontSize: runtime.terminal.options.fontSize,
      fontFamily: runtime.terminal.options.fontFamily,
      letterSpacing: runtime.terminal.options.letterSpacing,
      theme: runtime.terminal.options.theme,
      cursor: runtime.terminal.options.cursorStyle,
      blink: runtime.terminal.options.cursorBlink,
      renderer: runtime.getSnapshot().renderer,
    };
  }, pane);
}

test("theme cards filter local and package themes and preserve selection and immutable sources", async ({
  page,
}) => {
  await page.setViewportSize({ width: 920, height: 680 });
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page);
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-theme-manifests",
      JSON.stringify({
        linen: {
          version: 2,
          name: "Linen",
          description: "Soft contrast for a brighter workspace.",
          author: "Local author",
          appearance: "light",
          common: {},
        },
        graphite: {
          version: 2,
          name: "Graphite",
          description: "A restrained palette that follows your color mode.",
          appearance: "adaptive",
          common: {},
        },
        packaged: {
          version: 2,
          name: "Quiet graphite",
          description: "A shared theme from an installed package.",
          author: "Theme collection",
          appearance: "adaptive",
          common: {},
        },
        broken: {
          version: 99,
          name: "Unfinished theme",
          description: "A theme that needs attention before it can be used.",
        },
      }),
    );
    localStorage.setItem(
      "test-plugins",
      JSON.stringify([
        {
          id: "local.colors",
          source: "/external/colors",
          revision: "a".repeat(64),
          enabled: false,
          trustedRevision: null,
          error: null,
          evaluated: false,
          restartRequired: false,
          themeIds: ["packaged"],
          manifest: {
            schemaVersion: 1,
            hostApi: 1,
            id: "local.colors",
            name: "Quiet graphite",
            version: "1.0.0",
            description: "A theme collection.",
            contributes: {
              themes: [{ id: "local.colors.graphite", path: "graphite" }],
            },
          },
        },
      ]),
    );
  });
  await page.goto("/?window=settings&page=themes");
  const cards = page.getByRole("article");
  await expect(cards).toHaveCount(6);
  const first = await cards.nth(0).boundingBox();
  const second = await cards.nth(1).boundingBox();
  const list = await page.locator(".theme-list").boundingBox();
  expect(first!.width).toBe(list!.width);
  expect(first!.x).toBe(second!.x);
  expect(second!.y).toBeGreaterThanOrEqual(first!.y + first!.height);
  await page.screenshot({
    path: test.info().outputPath("themes-cards-dark.png"),
  });
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  await page.screenshot({
    path: test.info().outputPath("themes-cards-light.png"),
  });

  const search = page.getByRole("searchbox", { name: "Search themes" });
  const status = page.getByRole("combobox", { name: "Theme status" });
  const sources = page.getByRole("group", { name: "Theme sources" });
  await search.fill("  LOCAL AUTHOR  ");
  await expect(cards).toHaveCount(1);
  await expect(cards).toHaveAttribute("aria-label", "Linen");
  await chooseOption(status, "Active");
  await expect(cards).toHaveCount(0);
  await expect(
    page.getByText("No matching themes", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  await expect(search).toBeFocused();
  await expect(cards).toHaveCount(6);

  await sources.getByRole("button", { name: "Packages", exact: true }).click();
  await expect(cards).toHaveCount(1);
  await expect(cards).toHaveAttribute("aria-label", "Quiet graphite");
  await cards.getByRole("button", { name: "Use Quiet graphite theme" }).click();
  await expect(
    cards.getByRole("button", { name: "Use Quiet graphite theme" }),
  ).toHaveAttribute("aria-pressed", "true");
  await cards.getByRole("button", { name: "View theme" }).click();
  const editor = page.getByRole("dialog");
  await expect(editor).toContainText("immutable plugin package");
  await expect(
    editor.getByRole("button", { name: "Show controls" }),
  ).toBeDisabled();
  await expect(
    editor.getByRole("button", { name: "Save theme" }),
  ).toBeDisabled();
  await editor.getByRole("button", { name: "Close", exact: true }).click();
  await cards.getByRole("button", { name: "Duplicate theme" }).click();
  await expect(editor.getByRole("button", { name: "Edit JSON" })).toBeEnabled();
  await expect(
    editor.getByText("immutable plugin package", { exact: false }),
  ).toHaveCount(0);
  await editor.getByRole("button", { name: "Close", exact: true }).click();
  await expect(cards).toHaveCount(7);
  await expect(
    sources.getByRole("button", { name: "All sources" }),
  ).toHaveAttribute("aria-pressed", "true");
  await chooseOption(status, "Needs attention");
  await expect(cards).toHaveCount(1);
  await expect(
    cards.getByRole("button", { name: "Use Unfinished theme theme" }),
  ).toBeDisabled();
  await expect(cards.getByRole("button", { name: "Edit theme" })).toBeEnabled();
  await expect(cards.getByRole("alert")).toContainText(
    "Unsupported theme version",
  );

  await page
    .getByRole("button", { name: "Import folder", exact: true })
    .click();
  await expect(cards).toHaveCount(8);
  expect(await calls(page, "save_theme_preferences")).toHaveLength(1);
  expect(await calls(page, "prepare_plugin")).toHaveLength(0);
  expect(await calls(page, "enable_plugin")).toHaveLength(0);
  await chooseOption(status, "Active");
  await expect(cards).toHaveCount(1);
  await chooseOption(status, "All themes");
  await page.getByRole("button", { name: "Use Lomi theme" }).click();
  await expect(cards).toHaveCount(8);
  await expect(
    page.getByRole("button", { name: "Use Lomi theme" }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.setViewportSize({ width: 560, height: 420 });
  await page.locator(".themes-page").evaluate((element) => {
    element.scrollTop = 0;
  });
  expect(
    await page
      .locator(".themes-page")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  const narrowFirst = await cards.nth(0).boundingBox();
  const narrowSecond = await cards.nth(1).boundingBox();
  expect(narrowFirst!.x).toBe(narrowSecond!.x);
  expect(narrowSecond!.y).toBeGreaterThan(narrowFirst!.y);
  await page.screenshot({
    path: test.info().outputPath("themes-cards-minimum.png"),
  });
  await page.getByRole("button", { name: "Plugins", exact: true }).click();
  await expect(page.getByRole("article")).toHaveCount(0);
  await expect(
    page.getByText("No plugins installed.", { exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("test-plugins")!).length,
    ),
  ).toBe(1);
});

test("a theme updates both windows and hidden terminals without replacing PTYs", async ({
  page,
  context,
}, testInfo) => {
  await install(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const first = (await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  await expect.poll(async () => (await terminal(page, first)).id).toBeTruthy();
  const before = await terminal(page, first);
  await expect
    .poll(() =>
      page.evaluate(
        (id) => (window as any).__nativeTest.sessions.has(id),
        before.id,
      ),
    )
    .toBe(true);
  await page.evaluate(
    (id) => (window as any).__nativeTest.emit(id, "\r\noutput to preserve\r\n"),
    before.id,
  );
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+Shift+t");
  const second = (await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  expect(second).not.toBe(first);
  const preferences = await context.newPage();
  await install(preferences);
  await settings(preferences);
  await preferences
    .getByRole("button", { name: "Use Graphite Glass theme" })
    .click();
  await expect(
    preferences.getByRole("button", { name: "Use Graphite Glass theme" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(async () => (await terminal(page, first)).fontSize)
    .toBe(17);
  await expect
    .poll(async () => (await terminal(page, second)).fontSize)
    .toBe(17);
  expect((await terminal(page, first)).id).toBe(before.id);
  expect(await buffer(page, first)).toContain("output to preserve");
  expect((await calls(page, "start_terminal")).length).toBe(2);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  expect((await terminal(page, second)).theme?.background).toBe("#10101000");
  expect((await terminal(page, second)).cursor).toBe("underline");
  await expect
    .poll(async () => (await terminal(page, second)).fontFamily)
    .toContain("ThemeFace");
  expect((await terminal(page, second)).letterSpacing).toBe(1);
  expect((await terminal(page, second)).blink).toBe(false);
  await expect(page.locator(".statusbar")).toHaveCSS("height", "34px");
  await expect(page.locator(".tab").first()).toHaveCSS("border-radius", "14px");
  await expect(preferences.locator("h1")).toHaveCSS("letter-spacing", "3px");
  expect(
    await page
      .locator(".terminal-pane")
      .evaluate((node) => getComputedStyle(node, "::before").backgroundImage),
  ).toContain("wall%20paper.svg");
  await page.screenshot({ path: testInfo.outputPath("theme-terminal.png") });
  await preferences.screenshot({ path: testInfo.outputPath("themes.png") });
  await preferences.evaluate(() => {
    const manifests = JSON.parse(localStorage.getItem("test-theme-manifests")!);
    manifests.glass.stylesheets = [];
    localStorage.setItem("test-theme-manifests", JSON.stringify(manifests));
  });
  await preferences
    .getByRole("button", { name: "Refresh", exact: true })
    .click();
  await expect(page.locator(".tab").first()).toHaveCSS("border-radius", "8px");
  await expect(page.locator(".statusbar")).toHaveCSS("height", "34px");
  await expect
    .poll(async () => (await terminal(page, second)).letterSpacing)
    .toBe(0);
  await expect(preferences.locator('link[data-theme-layer="css"]')).toHaveCount(
    0,
  );
  await preferences.evaluate(() => {
    const manifests = JSON.parse(localStorage.getItem("test-theme-manifests")!);
    manifests.glass.stylesheets = ["styles/components.css", "styles/base.css"];
    localStorage.setItem("test-theme-manifests", JSON.stringify(manifests));
  });
  await preferences
    .getByRole("button", { name: "Refresh", exact: true })
    .click();
  await expect(page.locator(".tab").first()).toHaveCSS("border-radius", "8px");
  await expect
    .poll(async () => (await terminal(page, second)).letterSpacing)
    .toBe(2);
  await preferences.reload();
  await preferences
    .getByRole("button", { name: "Themes", exact: true })
    .click();
  await expect(preferences.locator("h1")).toHaveCSS("letter-spacing", "1px");
  await expect(preferences.locator('link[data-theme-layer="css"]')).toHaveCount(
    2,
  );
  expect(
    await preferences
      .locator(".catalog-content")
      .evaluate((node) => getComputedStyle(node).backgroundImage),
  ).toContain("/images/wall%20paper.svg");
  await preferences.getByRole("button", { name: "Use Lomi theme" }).click();
  await expect
    .poll(async () => (await terminal(page, first)).fontSize)
    .toBe(16);
  await expect(page.locator(".statusbar")).toHaveCSS("height", "28px");
  await expect(page.locator('link[data-theme-layer="css"]')).toHaveCount(0);
  expect(
    await page
      .locator(".terminal-pane")
      .evaluate((node) => getComputedStyle(node, "::before").backgroundImage),
  ).not.toContain("wall%20paper.svg");
  expect((await calls(page, "start_terminal")).length).toBe(2);
});

test("invalid edits and failed saves preserve the previous theme, and reset recovers saved settings", async ({
  page,
}) => {
  await install(page);
  await settings(page);
  await page.getByRole("button", { name: "Use Graphite Glass theme" }).click();
  await expect(page.locator("h1")).toHaveCSS("letter-spacing", "3px");
  await page.evaluate(() => {
    const manifests = JSON.parse(localStorage.getItem("test-theme-manifests")!);
    manifests.glass.terminal.fontSize = 0;
    localStorage.setItem("test-theme-manifests", JSON.stringify(manifests));
  });
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("fontSize");
  await expect(page.locator("h1")).toHaveCSS("letter-spacing", "3px");
  expect(
    JSON.parse(
      (await page.evaluate(() => localStorage.getItem("test-theme-settings")))!,
    ).active,
  ).toBe("glass");
  await page.evaluate(() => {
    (window as any).__nativeTest.failThemeSave = true;
  });
  await page.getByRole("button", { name: "Use Lomi theme" }).click();
  await expect(page.getByRole("alert")).toContainText("Disk is full");
  await expect(page.locator("h1")).toHaveCSS("letter-spacing", "3px");
  await page.evaluate(() => {
    (window as any).__nativeTest.failThemeSave = false;
    localStorage.setItem(
      "test-theme-settings",
      '{"version":99,"active":"future","customCss":true}',
    );
  });
  await page.reload();
  await page.getByRole("button", { name: "Themes", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("left intact");
  expect(await calls(page, "save_theme_preferences")).toHaveLength(0);
  await page.getByRole("button", { name: "Use Lomi theme" }).click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(
    JSON.parse(
      (await page.evaluate(() => localStorage.getItem("test-theme-settings")))!,
    ).active,
  ).toBeNull();
});

test("folder controls import themes without selecting them and fit the minimum window", async ({
  page,
}, testInfo) => {
  await install(page);
  await settings(page);
  await page
    .getByRole("button", { name: "Import folder", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Use Imported theme theme" }),
  ).toBeVisible();
  await page
    .getByRole("article", { name: "Imported theme" })
    .getByRole("button", { name: "Open theme folder" })
    .click();
  expect((await calls(page, "open_themes_folder"))[0].args.id).toBe("imported");
  expect(await calls(page, "save_theme_preferences")).toHaveLength(0);
  await page.getByRole("button", { name: "Use Imported theme theme" }).click();
  await expect(
    page.getByRole("button", { name: "Import folder", exact: true }),
  ).toHaveCSS("border-radius", "12px");
  await page.setViewportSize({ width: 560, height: 420 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect(
    await page
      .locator(".themes-page")
      .evaluate((node) => node.scrollWidth <= node.clientWidth),
  ).toBe(true);
  await expect(page.getByLabel("Use theme CSS")).toHaveCount(0);
  await expect(page.getByText("Creating a theme", { exact: true })).toHaveCount(
    0,
  );
  await page.screenshot({ path: testInfo.outputPath("themes-minimum.png") });
  expect(await calls(page, "start_terminal")).toHaveLength(0);
});

test("a missing stylesheet or undecodable image cannot replace a working theme", async ({
  page,
}) => {
  await install(page);
  await settings(page);
  await page.route("**/theme-assets/**/styles/components.css", (route) =>
    route.abort(),
  );
  await page.getByRole("button", { name: "Use Graphite Glass theme" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Cannot load styles/components.css",
  );
  expect(await calls(page, "save_theme_preferences")).toHaveLength(0);
  await expect(page.locator('link[href*="theme-assets"]')).toHaveCount(0);
  await page.unroute("**/theme-assets/**/styles/components.css");
  await page.route("**/theme-assets/**/*.svg", (route) =>
    route.fulfill({ contentType: "image/svg+xml", body: "broken image" }),
  );
  await page.getByRole("button", { name: "Use Graphite Glass theme" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Cannot decode background",
  );
  expect(await calls(page, "save_theme_preferences")).toHaveLength(0);
  await expect(
    page.getByRole("button", { name: "Use Lomi theme" }),
  ).toHaveAttribute("aria-pressed", "true");
});

test("JSON style validation rejects malformed declarations before saving", async ({
  page,
}) => {
  await install(page);
  await settings(page);
  const result = await page.evaluate(async () => {
    const { compileTheme } = await import("/src/theme/runtime.ts");
    const attempt = (styles: Record<string, Record<string, string>>) => {
      try {
        return {
          css: compileTheme(
            { version: 1, name: "Style test", styles },
            () => "",
          ),
          error: "",
        };
      } catch (error) {
        return { css: "", error: String(error) };
      }
    };
    return {
      invalid: attempt({ ".tab": { "border-radius": "not-a-radius" } }),
      injection: attempt({ ".tab {} body": { display: "none" } }),
      supported: attempt({
        ".tab:hover, .button:focus-visible": {
          "border-radius": "12px !important",
        },
      }),
    };
  });
  expect(result.invalid.error).toContain("Invalid CSS value");
  expect(result.injection.error).toContain("Invalid CSS selector");
  expect(result.supported.error).toBe("");
  expect(result.supported.css).toContain("12px !important");
  expect(await calls(page, "save_theme_preferences")).toHaveLength(0);
});

test("legacy CSS preferences cannot disable a stylesheet declared by JSON", async ({
  page,
}) => {
  await install(page);
  const saved =
    '{"version":1,"active":"legacy","customCss":false,"appearance":"light"}';
  await page.addInitScript(
    ({ saved }) => {
      if (!localStorage.getItem("test-theme-settings")) {
        localStorage.setItem("test-theme-settings", saved);
        localStorage.setItem(
          "test-theme-manifests",
          JSON.stringify({
            legacy: { version: 1, name: "Legacy", stylesheet: "theme.css" },
          }),
        );
      }
    },
    { saved },
  );
  await settings(page);
  await expect(page.locator("h1")).toHaveCSS("letter-spacing", "3px");
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  expect(await calls(page, "save_theme_preferences")).toHaveLength(0);
  expect(
    await page.evaluate(() => localStorage.getItem("test-theme-settings")),
  ).toBe(saved);
  await page.getByRole("button", { name: "Use Lomi theme" }).click();
  await expect(page.locator('link[data-theme-layer="css"]')).toHaveCount(0);
  expect(
    JSON.parse(
      (await page.evaluate(() => localStorage.getItem("test-theme-settings")))!,
    ),
  ).toEqual({ version: 1, active: null, appearance: "light" });
});

test("theme controls save section spacing and layouts in both windows without replacing panels", async ({
  page,
  context,
}, testInfo) => {
  await install(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+d");
  await expect(page.locator(".terminal-pane")).toHaveCount(2);
  const pane = (await page
    .locator("[data-pane-id]")
    .first()
    .getAttribute("data-pane-id"))!;
  const before = await terminal(page, pane);
  await page
    .locator(".split-child")
    .first()
    .evaluate((element) => {
      (window as any).__themePanel = element;
    });
  const preferences = await context.newPage();
  await install(preferences);
  await settings(preferences);
  await preferences
    .getByRole("button", { name: "Use Graphite Glass theme" })
    .click();
  await preferences
    .getByRole("button", { name: "Edit theme", exact: true })
    .click();
  const editor = preferences.getByRole("dialog", {
    name: "Edit theme: Graphite Glass",
    exact: true,
  });
  await chooseOption(editor.getByLabel("Tab placement"), "Below");
  await chooseOption(editor.getByLabel("Status bar placement"), "Top");
  await chooseOption(
    editor.getByLabel("Settings navigation", { exact: true }),
    "Right",
  );
  const overrides = {
    "--work-area-padding": "12px",
    "--sidebar-section-gap": "10px",
    "--pane-spacing": "6px",
    "--pane-border": "3px solid var(--color-outline)",
    "--radius-pane": "18px",
    "--tab-gap": "12px",
    "--editor-font-size": "18px",
    "--sidebar-min-width": "140px",
    "--sidebar-max-width": "700px",
  };
  for (const [name, value] of Object.entries(overrides)) {
    await editor.getByRole("searchbox").fill(name);
    await editor.getByRole("textbox", { name, exact: true }).fill(value);
  }
  await editor.getByRole("button", { name: "Save theme", exact: true }).click();
  await expect(editor.getByRole("status")).toContainText(
    "Open windows have been notified",
  );
  await expect(page.locator(".work-area")).toHaveCSS("padding", "12px");
  await expect(page.locator(".terminal-pane").first()).toHaveCSS(
    "border-top-width",
    "3px",
  );
  await expect(page.locator(".terminal-pane").first()).toHaveCSS(
    "border-radius",
    "18px",
  );
  await expect(page.locator(".split-child").first()).toHaveCSS(
    "padding",
    "6px",
  );
  await expect(page.locator(".tab-strip")).toHaveCSS("gap", "12px");
  const tabs = (await page.locator(".tab-bar").boundingBox())!;
  const project = (await page.locator(".project-switcher").boundingBox())!;
  const statusbar = (await page.locator(".statusbar").boundingBox())!;
  const work = (await page.locator(".work-area").boundingBox())!;
  expect(tabs.y).toBeGreaterThanOrEqual(project.y + project.height);
  expect(statusbar.y + statusbar.height).toBeLessThanOrEqual(work.y + 1);
  expect((await terminal(page, pane)).id).toBe(before.id);
  expect(
    await page
      .locator(".split-child")
      .first()
      .evaluate((element) => element === (window as any).__themePanel),
  ).toBe(true);
  expect(await calls(page, "start_terminal")).toHaveLength(2);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  await editor.getByRole("searchbox").fill("radius");
  const fields = (await editor.locator(".theme-editor-fields").boundingBox())!;
  const footer = (await editor.locator(".theme-editor-footer").boundingBox())!;
  expect(fields.y + fields.height).toBeLessThanOrEqual(footer.y + 1);
  await preferences.screenshot({
    path: testInfo.outputPath("theme-editor.png"),
  });
  await editor.getByRole("button", { name: "Close", exact: true }).click();
  const navigation = (await preferences
    .locator(".settings-navigation")
    .boundingBox())!;
  const content = (await preferences.locator(".themes-page").boundingBox())!;
  expect(navigation.x).toBeGreaterThanOrEqual(content.x + content.width - 1);
  await page.setViewportSize({ width: 800, height: 420 });
  await expect(
    page.getByRole("button", { name: "Close window" }),
  ).toBeInViewport();
  await expect(page.locator(".terminal-pane").first()).toBeInViewport();
  await page.screenshot({
    path: testInfo.outputPath("theme-layout-minimum.png"),
  });
  await preferences.reload();
  await preferences
    .getByRole("button", { name: "Themes", exact: true })
    .click();
  await expect(preferences.locator(".settings-layout")).toHaveCSS(
    "flex-direction",
    "row-reverse",
  );
  await preferences.getByRole("button", { name: "Use Lomi theme" }).click();
  await expect(page.locator(".work-area")).toHaveCSS("padding", "0px");
  await expect(page.locator(".tab-bar")).toHaveCSS("order", "0");
  await expect(page.locator(".terminal-pane").first()).toHaveCSS(
    "border-top-width",
    "0px",
  );
  await expect(preferences.locator(".settings-layout")).toHaveCSS(
    "flex-direction",
    "row",
  );
  await preferences
    .getByRole("button", { name: "Create theme", exact: true })
    .click();
  const starter = preferences.getByRole("dialog", {
    name: "Edit theme: My theme",
    exact: true,
  });
  await expect(starter).toBeVisible();
  await preferences.setViewportSize({ width: 560, height: 420 });
  await expect(
    starter.getByRole("button", { name: "Close", exact: true }),
  ).toBeInViewport();
  expect(
    await starter.evaluate(
      (element) => element.scrollWidth <= element.clientWidth,
    ),
  ).toBe(true);
});

test("above-tab and horizontal settings navigation layouts fit without moving DOM nodes", async ({
  page,
  context,
}) => {
  await install(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const preferences = await context.newPage();
  await install(preferences);
  await settings(preferences);
  for (const settingsNavigation of ["top", "bottom"] as const) {
    for (const target of [page, preferences]) {
      await target.evaluate(async (settingsNavigation) => {
        const { prepareTheme } = await import("/src/theme/runtime.ts");
        const prepared = await prepareTheme(
          {
            id: "layout",
            revision: "test",
            directory: "/app/themes/layout",
            raw: JSON.stringify({
              version: 2,
              name: "Layout",
              common: { layout: { tabs: "above", settingsNavigation } },
            }),
          },
          { version: 1, active: "layout", appearance: "dark" },
        );
        await prepared.commit();
      }, settingsNavigation);
    }
    const tabs = (await page.locator(".tab-bar").boundingBox())!;
    const project = (await page.locator(".project-switcher").boundingBox())!;
    expect(tabs.y + tabs.height).toBeLessThanOrEqual(project.y);
    const navigation = (await preferences
      .locator(".settings-navigation")
      .boundingBox())!;
    const content = (await preferences.locator(".themes-page").boundingBox())!;
    if (settingsNavigation === "top")
      expect(navigation.y + navigation.height).toBeLessThanOrEqual(
        content.y + 1,
      );
    else
      expect(navigation.y).toBeGreaterThanOrEqual(
        content.y + content.height - 1,
      );
    await expect(
      preferences.getByRole("button", { name: "Themes", exact: true }),
    ).toBeInViewport();
  }
});

test("theme editing preserves drafts on validation, disk failures, external edits and cancellation", async ({
  page,
}) => {
  await install(page);
  await settings(page);
  await page.getByRole("button", { name: "Edit theme", exact: true }).click();
  const editor = page.getByRole("dialog", {
    name: "Edit theme: Graphite Glass",
    exact: true,
  });
  await editor.getByRole("button", { name: "Edit JSON", exact: true }).click();
  const json = editor.getByRole("textbox", { name: "Theme JSON", exact: true });
  await json.fill(
    '{"version":2,"name":"Draft","common":{"layout":{"tabs":"broken"}}}',
  );
  await expect(
    editor.getByRole("button", { name: "Save theme", exact: true }),
  ).toBeDisabled();
  await expect(editor.getByRole("alert")).toContainText("layout.tabs");
  expect(await calls(page, "save_theme_manifest")).toHaveLength(0);
  const draft = JSON.stringify({
    version: 2,
    name: "My draft",
    common: { tokens: theme.tokens },
    resources: { stylesheets: [] },
  });
  await json.fill(draft);
  await page.evaluate(() => {
    (window as any).__nativeTest.failThemeSave = true;
  });
  await editor.getByRole("button", { name: "Save theme", exact: true }).click();
  await expect(editor.getByRole("alert")).toContainText("Disk is full");
  await expect(json).toHaveText(draft);
  await page.evaluate(() => {
    (window as any).__nativeTest.failThemeSave = false;
    const manifests = JSON.parse(localStorage.getItem("test-theme-manifests")!);
    manifests.glass.name = "External edit";
    localStorage.setItem("test-theme-manifests", JSON.stringify(manifests));
  });
  await editor.getByRole("button", { name: "Save theme", exact: true }).click();
  await expect(editor.getByRole("alert")).toContainText("changed on disk");
  await expect(json).toHaveText(draft);
  await editor.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Keep editing", exact: true }).click();
  await expect(json).toHaveText(draft);
  await editor.getByRole("button", { name: "Close", exact: true }).click();
  await page
    .getByRole("button", { name: "Discard changes", exact: true })
    .click();
  await expect(editor).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("test-theme-manifests")!).glass.name,
    ),
  ).toBe("External edit");
});

test("preview stays local and cancel restores working layers even when current files become invalid", async ({
  page,
  context,
}) => {
  await install(page);
  await settings(page);
  await page.getByRole("button", { name: "Use Graphite Glass theme" }).click();
  const main = await context.newPage();
  await install(main);
  await main.goto("/");
  await expect(main.locator(".terminal-pane")).toBeVisible();
  await page.getByRole("button", { name: "Edit theme", exact: true }).click();
  const editor = page.getByRole("dialog", {
    name: "Edit theme: Graphite Glass",
    exact: true,
  });
  await editor.getByRole("searchbox").fill("radius-control");
  await editor
    .getByRole("textbox", { name: "--radius-control", exact: true })
    .fill("17px");
  for (let attempt = 0; attempt < 3; attempt++) {
    await editor.getByRole("button", { name: "Preview", exact: true }).click();
    await expect(page.locator("html")).toHaveCSS("--radius-control", "17px");
    await expect(main.locator("html")).toHaveCSS("--radius-control", "12px");
    if (attempt === 2)
      await page.evaluate(() => {
        const themes = JSON.parse(
          localStorage.getItem("test-theme-manifests")!,
        );
        themes.glass.version = 99;
        localStorage.setItem("test-theme-manifests", JSON.stringify(themes));
      });
    await editor
      .getByRole("button", { name: "Cancel preview", exact: true })
      .click();
    await expect(page.locator("html")).toHaveCSS("--radius-control", "12px");
    await expect(page.locator("[data-theme-held]")).toHaveCount(0);
  }
  await expect(
    editor.getByRole("textbox", { name: "--radius-control", exact: true }),
  ).toHaveValue("17px");
  await expect(
    editor.getByRole("button", { name: "Save theme", exact: true }),
  ).toBeEnabled();
  await page.screenshot({
    path: test.info().outputPath("theme-preview-recovery.png"),
  });
});

test("required font failures retain the active theme before any layer is swapped", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".terminal-pane")).toBeVisible();
  await page.route("**/broken.woff2", (route) =>
    route.fulfill({ contentType: "font/woff2", body: "not a font" }),
  );
  const result = await page.evaluate(async () => {
    const { prepareTheme, effectiveTheme } =
      await import("/src/theme/runtime.ts");
    const before = effectiveTheme().revision;
    const background = getComputedStyle(
      document.documentElement,
    ).getPropertyValue("--color-background");
    let failure = "";
    try {
      await prepareTheme(
        {
          id: "broken",
          directory: "/themes/broken",
          revision: "test",
          raw: JSON.stringify({
            version: 2,
            name: "Broken",
            common: { tokens: { "--color-background": "#abcdef" } },
            resources: { assets: { font: "broken.woff2" } },
          }),
        },
        { version: 1, active: "broken", appearance: "dark" },
      );
    } catch (error) {
      failure = String(error);
    }
    return {
      failure,
      unchanged:
        before === effectiveTheme().revision &&
        background ===
          getComputedStyle(document.documentElement).getPropertyValue(
            "--color-background",
          ),
    };
  });
  expect(result.failure).toContain("Cannot load required font");
  expect(result.unchanged).toBe(true);
});
