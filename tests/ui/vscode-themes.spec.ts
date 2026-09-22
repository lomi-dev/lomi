import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

const vscode = {
  name: "Portable",
  type: "dark",
  colors: {
    "editor.background": "#162435",
    "editor.foreground": "#eeddaa",
    "sideBar.background": "#273849",
    "statusBar.background": "#394a5b",
    "statusBar.foreground": "#abcdef",
    "terminal.ansiBlue": "#345678",
    "extension.future": "#102030",
  },
  tokenColors: [
    {
      scope: "comment",
      settings: { foreground: "#89abcd", fontStyle: "italic" },
    },
    { scope: "source.python variable", settings: { foreground: "#ff0000" } },
  ],
  semanticHighlighting: true,
  semanticTokenColors: {
    "variable.readonly:typescript": { foreground: "#123456", bold: true },
  },
};
const manifest = { version: 2, name: "Portable", appearance: "dark", vscode };

test("imports VS Code data, applies it, reports limits and exports without changing preferences", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/?window=settings&page=themes");
  await page.evaluate((manifest) => {
    (window as any).__nativeTest.vscodeThemes = [manifest];
  }, manifest);
  await page
    .getByRole("button", { name: "Import VS Code", exact: true })
    .click();
  const card = page.getByRole("article", { name: "Portable", exact: true });
  await expect(card).toBeVisible();
  await card.getByRole("button", { name: "Use Portable theme" }).click();
  await expect(page.locator("html")).toHaveAttribute(
    "data-theme",
    "vscode-theme-1",
  );
  expect(
    await page.evaluate(() =>
      getComputedStyle(document.documentElement)
        .getPropertyValue("--statusbar-background")
        .trim(),
    ),
  ).toBe("#394a5b");
  await card.getByRole("button", { name: "Edit theme" }).click();
  await expect(
    page.getByRole("textbox", { name: "--editor-background", exact: true }),
  ).toHaveAttribute("placeholder", "#162435");
  await page.getByText("VS Code compatibility", { exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText(
    "Semantic rules are preserved",
  );
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Close", exact: true })
    .click();
  const before = await page.evaluate(() => ({
    preferences: localStorage.getItem("test-theme-settings"),
    revision: document.documentElement.dataset.themeSourceRevision,
  }));
  await card.getByRole("button", { name: "Export to VS Code" }).click();
  await expect(page.locator(".theme-status")).toContainText(
    "Extensions: Install from VSIX",
  );
  const exported = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls
        .filter((c: any) => c.command === "export_vscode_theme")
        .at(-1).args,
  );
  expect(exported.themes).toHaveLength(1);
  expect(exported.themes[0].theme).toEqual(vscode);
  expect(
    await page.evaluate(() => ({
      preferences: localStorage.getItem("test-theme-settings"),
      revision: document.documentElement.dataset.themeSourceRevision,
    })),
  ).toEqual(before);
  expect(await page.locator("iframe").count()).toBe(0);
  await page.setViewportSize({ width: 800, height: 620 });
  await page.screenshot({ path: test.info().outputPath("vscode-themes.png") });
});

test("exports both Lomi modes and resolves editor, terminal, CSS and syntax overrides", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/?window=settings&page=themes");
  const result = await page.evaluate(async (manifest) => {
    const { exportVSCodeThemes } = await import("/src/theme/vscode-export.ts");
    const { builtinTheme, deepmonoTheme } =
      await import("/src/theme/format.ts");
    const defaults = await exportVSCodeThemes(builtinTheme, null);
    const deepmono = await exportVSCodeThemes(deepmonoTheme, null);
    const customized = await exportVSCodeThemes(
      {
        ...manifest,
        common: {
          tokens: {
            "--terminal-blue": "#fedcba",
            "--color-background": "#654321",
          },
          editor: {
            colors: { background: "#123456" },
            syntax: { comment: { color: "#abcdef", fontStyle: "normal" } },
          },
          styles: { ".statusbar": { color: "#dcbafe" } },
        },
      },
      null,
    );
    return { defaults, deepmono, customized };
  }, manifest);
  expect(result.defaults.map((t: any) => t.theme.type)).toEqual([
    "dark",
    "light",
  ]);
  expect(result.defaults[0].theme.colors["editor.background"]).toBe("#101114");
  expect(result.defaults[1].theme.colors["editor.background"]).not.toBe(
    "#101114",
  );
  expect(
    result.deepmono.map(
      (entry: any) => entry.theme.colors["editor.background"],
    ),
  ).toEqual(["#101010", "#f4f4f4"]);
  const exported = result.customized[0].theme;
  expect(exported.colors["terminal.ansiBlue"]).toBe("#fedcba");
  expect(exported.colors["editor.background"]).toBe("#123456");
  expect(exported.colors["statusBar.foreground"]).toBe("#dcbafe");
  expect(exported.colors["extension.future"]).toBe("#102030");
  expect(exported.semanticTokenColors).toEqual(vscode.semanticTokenColors);
  expect(exported.tokenColors.slice(0, 2)).toEqual(vscode.tokenColors);
  expect(exported.tokenColors.at(-1).settings).toEqual({
    foreground: "#abcdef",
    fontStyle: "",
  });
});

test("export failures retain the selected theme and remove isolated rendering documents", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/?window=settings&page=themes");
  await page.route("**/theme-assets/**", (route) => route.abort());
  const result = await page.evaluate(async () => {
    const { exportVSCodeThemes } = await import("/src/theme/vscode-export.ts");
    const before = document.documentElement.dataset.theme;
    let error = "";
    try {
      await exportVSCodeThemes(
        {
          version: 2,
          name: "Broken",
          resources: { stylesheets: ["missing.css"] },
        },
        { id: "broken", revision: "1", raw: "", directory: "/broken" },
      );
    } catch (e) {
      error = String(e);
    }
    return {
      error,
      before,
      after: document.documentElement.dataset.theme,
      frames: document.querySelectorAll("iframe").length,
    };
  });
  expect(result.error).toMatch(/loading missing\.css (failed|timed out)/);
  expect(result.after).toBe(result.before);
  expect(result.frames).toBe(0);
});
