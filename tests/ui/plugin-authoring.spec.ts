import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { mockDesktop } from "./desktop";
const directory = process.env.LOMI_AUTHOR_PACKAGE;
test("generated archive panel uses host context and persists its preference", async ({
  page,
  context,
}) => {
  test.skip(
    !directory,
    "Set LOMI_AUTHOR_PACKAGE to a generated candidate package. Native commands are mocked.",
  );
  const manifest = JSON.parse(
    await readFile(join(directory!, "plugin.json"), "utf8"),
  );
  const revision = "b".repeat(64);
  const entry = {
    id: manifest.id,
    manifest,
    revision,
    source: directory!,
    enabled: false,
    trustedRevision: null,
    error: null,
    restartRequired: false,
    evaluated: false,
  };
  await mockDesktop(page);
  await page.route("**/plugin-assets/**", async (route) => {
    const path = decodeURIComponent(
      new URL(route.request().url()).pathname.split(`/${revision}/`)[1],
    );
    const bytes = await readFile(join(directory!, path));
    await route.fulfill({
      body: bytes,
      contentType: path.endsWith(".js") ? "text/javascript" : "text/css",
    });
  });
  await page.goto("/");
  await expect(page.locator(".terminal-pane")).toBeVisible();
  const settings = await context.newPage();
  await mockDesktop(settings);
  await settings.goto("/?window=settings");
  await settings.getByRole("button", { name: "Plugins", exact: true }).click();
  await settings.evaluate((entry) => {
    (window as any).__nativeTest.folder = entry.source;
    (window as any).__nativeTest.pluginImport = entry;
  }, entry);
  await settings
    .getByRole("button", { name: "Import plugin", exact: true })
    .click();
  await settings
    .getByRole("switch", { name: `Enable ${manifest.name}` })
    .click();
  await settings.getByRole("button", { name: "Trust and enable" }).click();
  await page.keyboard.press("Control+Shift+P");
  const picker = page.getByRole("dialog", { name: "Commands", exact: true });
  await picker.getByLabel("Find command").fill(manifest.name);
  await picker
    .getByRole("button", {
      name: new RegExp(
        "^" + manifest.name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
      ),
    })
    .click();
  const panel = page.locator(".plugin-panel");
  await expect(panel.getByLabel("Show project path")).toBeChecked();
  await expect(panel).toContainText("/project");
  await panel.getByLabel("Show project path").uncheck();
  await expect(panel).not.toContainText("/project");
  await expect
    .poll(() =>
      page.evaluate(() =>
        JSON.parse(
          localStorage.getItem("test-session")!,
        ).projects[0].workspaces[0].tabs.some(
          (tab: any) => tab.type === "plugin" && tab.state.showPath === false,
        ),
      ),
    )
    .toBeTruthy();
  await page.screenshot({
    path: test.info().outputPath("generated-panel.png"),
  });
});
