import { test, expect, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { mockDesktop, chooseOption } from "./desktop";
import manifest from "../fixtures/context-plugin/plugin.json" with { type: "json" };
const revision = "a".repeat(64);
const entry = {
  id: manifest.id,
  manifest,
  revision,
  source: "/external/package space żółć",
  enabled: false,
  trustedRevision: null,
  error: null,
  restartRequired: false,
  evaluated: false,
};
async function packageRoutes(page: Page) {
  await page.route("**/plugin-assets/**", async (route) => {
    const path = decodeURIComponent(
      new URL(route.request().url()).pathname.split(`/${revision}/`)[1],
    );
    const bytes = await readFile(
      `tests/fixtures/context-plugin/package/${path}`,
    );
    await route.fulfill({
      body: bytes,
      contentType: path.endsWith(".js")
        ? "text/javascript"
        : path.endsWith(".css")
          ? "text/css"
          : "image/svg+xml",
    });
  });
}
async function commands(page: Page, label: string) {
  await expect(page.locator(".statusbar")).toBeVisible();
  await page.keyboard.press("Control+Shift+P");
  const dialog = page.getByRole("dialog", { name: "Commands", exact: true });
  await dialog.getByLabel("Find command").fill(label);
  await dialog.getByRole("button", { name: new RegExp("^" + label) }).click();
}
test("plugin search and filters combine without enabling code and remain usable at minimum size", async ({
  page,
}) => {
  await page.setViewportSize({ width: 920, height: 680 });
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page);
  const catalog = [
    entry,
    {
      ...entry,
      id: "local.notes",
      enabled: true,
      trustedRevision: revision,
      manifest: {
        ...manifest,
        id: "local.notes",
        name: "Project notes",
        description: "Keep project notes close to your terminals and editors.",
        contributes: {
          themes: [{ id: "local.notes.theme", path: "theme" }],
          views: [
            {
              id: "local.notes.view",
              title: "Notes",
              placement: "central",
              multiple: false,
              stateVersion: 1,
            },
          ],
        },
      },
    },
    {
      ...entry,
      id: "local.actions",
      enabled: true,
      trustedRevision: revision,
      manifest: {
        ...manifest,
        id: "local.actions",
        name: "Quick actions",
        description: "Keep frequently used workspace actions within reach.",
        contributes: {
          commands: [
            {
              id: "local.actions.open",
              label: "Quick actions",
              description: "Open workspace actions.",
            },
          ],
        },
      },
    },
    {
      ...entry,
      id: "local.themes",
      manifest: {
        schemaVersion: 1,
        id: "local.themes",
        name: "Warm graphite",
        version: "1.2.0",
        description:
          "A quiet pair of light and dark themes for your workspace.",
        hostApi: 1,
        contributes: {
          themes: [{ id: "local.themes.graphite", path: "graphite" }],
        },
      },
    },
    {
      ...entry,
      id: "local.unavailable",
      manifest: null,
      source: "/external/" + "very-long-package-folder-".repeat(15),
      error: "This package requires a newer version of Lomi.",
    },
  ];
  await page.addInitScript((entries) => {
    localStorage.setItem("test-plugins", JSON.stringify(entries));
  }, catalog);
  await page.goto("/?window=settings&page=plugins");
  const cards = page.getByRole("article");
  await expect(cards).toHaveCount(4);
  const first = await cards.nth(0).boundingBox();
  const second = await cards.nth(1).boundingBox();
  expect(first!.y).toBe(second!.y);
  expect(second!.x).toBeGreaterThan(first!.x);
  await page.screenshot({ path: test.info().outputPath("plugins-dark.png") });
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  await page.screenshot({ path: test.info().outputPath("plugins-light.png") });

  const toggle = page.getByRole("switch", { name: "Enable Workspace context" });
  await toggle.focus();
  await toggle.press("Space");
  await expect(page.getByRole("dialog")).toContainText(entry.source);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(toggle).not.toBeChecked();
  await expect(toggle).toBeFocused();

  const search = page.getByRole("searchbox", { name: "Search plugins" });
  await search.fill("  FOLDER  ");
  await expect(cards).toHaveCount(1);
  await expect(cards).toHaveAttribute("aria-label", "Workspace context");
  await chooseOption(
    page.getByRole("combobox", { name: "Plugin status" }),
    "Enabled",
  );
  await expect(cards).toHaveCount(0);
  await expect(
    page.getByText("No matching plugins", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  await expect(cards).toHaveCount(4);
  await expect(search).toBeFocused();

  await chooseOption(
    page.getByRole("combobox", { name: "Plugin status" }),
    "Disabled",
  );
  await expect(cards).toHaveCount(1);
  await expect(cards).toHaveAttribute("aria-label", "Workspace context");
  await chooseOption(
    page.getByRole("combobox", { name: "Plugin status" }),
    "All plugins",
  );
  const categories = page.getByRole("group", { name: "Plugin categories" });
  await expect(
    categories.getByRole("button", { name: "Themes", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("article", { name: "Warm graphite" }),
  ).toHaveCount(0);
  await expect(page.locator(".catalog-count")).toHaveText("4 installed");
  await search.fill("local.themes");
  await expect(cards).toHaveCount(0);
  await page.evaluate((entry) => {
    (window as any).__nativeTest.folder = entry.source;
    (window as any).__nativeTest.pluginImport = entry;
  }, entry);
  await page
    .getByRole("button", { name: "Import plugin", exact: true })
    .click();
  await expect(cards).toHaveCount(4);
  await expect(
    categories.getByRole("button", { name: "All types" }),
  ).toHaveAttribute("aria-pressed", "true");
  await categories
    .getByRole("button", { name: "Commands", exact: true })
    .click();
  await expect(cards).toHaveCount(2);
  await chooseOption(
    page.getByRole("combobox", { name: "Plugin status" }),
    "Enabled",
  );
  await expect(cards).toHaveAttribute("aria-label", "Quick actions");

  await categories.getByRole("button", { name: "All types" }).click();
  await chooseOption(
    page.getByRole("combobox", { name: "Plugin status" }),
    "Needs attention",
  );
  await expect(cards).toHaveCount(1);
  await expect(cards.getByRole("alert")).toContainText("newer version");
  await cards.locator("summary").click();
  await expect(cards.getByText(catalog[4].source)).toBeVisible();
  await page.setViewportSize({ width: 560, height: 420 });
  expect(
    await page
      .locator(".plugins-page")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await page.getByRole("button", { name: "Refresh plugins" }).click();
  await expect(cards).toHaveCount(1);
  await chooseOption(
    page.getByRole("combobox", { name: "Plugin status" }),
    "All plugins",
  );
  await expect(cards).toHaveCount(4);
  const narrowFirst = await cards.nth(0).boundingBox();
  const narrowSecond = await cards.nth(1).boundingBox();
  expect(narrowFirst!.x).toBe(narrowSecond!.x);
  expect(narrowSecond!.y).toBeGreaterThan(narrowFirst!.y);
  await page.screenshot({
    path: test.info().outputPath("plugins-minimum.png"),
  });
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter((call: any) =>
        ["enable_plugin", "prepare_plugin", "request_plugin_removal"].includes(
          call.command,
        ),
      ),
    ),
  ).toEqual([]);
});
test("installed external ESM stays inert until trust, opens once, persists state and cleans contributions on disable", async ({
  page,
  context,
}) => {
  await mockDesktop(page);
  await packageRoutes(page);
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
  await expect(
    settings.getByRole("switch", { name: "Enable Workspace context" }),
  ).not.toBeChecked();
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (c: any) => c.command === "prepare_plugin",
        ).length,
    ),
  ).toBe(0);
  await settings
    .getByRole("switch", { name: "Enable Workspace context" })
    .click();
  await expect(settings.getByRole("dialog")).toContainText(entry.source);
  await settings.getByRole("button", { name: "Trust and enable" }).click();
  await expect(
    settings.getByRole("switch", { name: "Enable Workspace context" }),
  ).toBeChecked();
  await settings.setViewportSize({ width: 800, height: 420 });
  await settings.screenshot({
    path: test.info().outputPath("plugin-management-minimum.png"),
  });
  await commands(page, "Show workspace context");
  const panel = page.locator(".plugin-panel");
  await expect(
    panel.getByRole("heading", { name: "Workspace context" }),
  ).toBeVisible();
  await expect(panel).toContainText("/project");
  await panel.getByRole("button", { name: "Show details" }).click();
  await expect(panel).toContainText("relative ESM chunk");
  await page.getByRole("button", { name: "Context", exact: true }).click();
  await expect(page.locator(".plugin-panel")).toHaveCount(1);
  await expect
    .poll(() =>
      page.evaluate(() =>
        JSON.parse(
          localStorage.getItem("test-session")!,
        ).projects[0].workspaces[0].tabs.some(
          (tab: any) => tab.type === "plugin" && tab.state.expanded,
        ),
      ),
    )
    .toBeTruthy();
  await settings
    .getByRole("switch", { name: "Enable Workspace context" })
    .click();
  await expect(
    panel.getByRole("heading", { name: "Plugin view unavailable" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Context", exact: true }),
  ).toHaveCount(0);
  await expect(page.locator("link[data-plugin]")).toHaveCount(0);
  await settings
    .getByRole("switch", { name: "Enable Workspace context" })
    .click();
  await settings.getByRole("button", { name: "Trust and enable" }).click();
  await expect(
    panel.getByRole("button", { name: "Hide details" }),
  ).toBeVisible();
  expect(
    await settings.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (c: any) => c.command === "prepare_plugin",
        ).length,
    ),
  ).toBe(0);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (c: any) => c.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
});
test("dirty plugin views stop disable on cancel and failed save, then retain their descriptor after uninstall", async ({
  page,
  context,
}) => {
  await mockDesktop(page);
  await packageRoutes(page);
  await page.addInitScript(
    (entry) =>
      localStorage.setItem(
        "test-plugins",
        JSON.stringify([
          { ...entry, enabled: true, trustedRevision: entry.revision },
        ]),
      ),
    entry,
  );
  await page.goto("/");
  await commands(page, "Show workspace context");
  await expect(
    page.getByRole("heading", { name: "Workspace context" }),
  ).toBeVisible();
  await page.evaluate(async () => {
    const { pluginHost } = await import("/src/plugins/runtime.ts");
    const id = document.querySelector<HTMLElement>("[data-plugin-pane-id]")!
      .dataset.pluginPaneId!;
    pluginHost.dirtyViews.set(id, {
      owner: "lomi.context",
      view: {
        title: "Plugin draft",
        isDirty: () => true,
        save: async () => {
          throw new Error("Plugin save failed");
        },
        discard: () => {
          pluginHost.dirtyViews.delete(id);
        },
      },
    });
  });
  const settings = await context.newPage();
  await mockDesktop(settings);
  await settings.goto("/?window=settings");
  await settings.getByRole("button", { name: "Plugins", exact: true }).click();
  await settings
    .getByRole("switch", { name: "Enable Workspace context" })
    .click();
  const guard = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await expect(guard).toContainText("Plugin draft");
  await guard.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(settings.getByRole("alert")).toContainText("canceled");
  await expect(
    page.getByRole("heading", { name: "Workspace context" }),
  ).toBeVisible();
  await settings
    .getByRole("button", { name: "Uninstall", exact: true })
    .click();
  await guard
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(guard.getByRole("alert")).toContainText("Plugin save failed");
  await expect(
    settings.getByRole("button", { name: "Uninstall", exact: true }),
  ).toBeDisabled();
  await guard.getByRole("button", { name: "Discard changes" }).click();
  await expect(settings.getByText("No plugins installed.")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Plugin view unavailable" }),
  ).toBeVisible();
});

test("keyboard docking retains a plugin instance beside its terminal and can reposition it", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page);
  await packageRoutes(page);
  await page.addInitScript(
    (entry) =>
      localStorage.setItem(
        "test-plugins",
        JSON.stringify([
          { ...entry, enabled: true, trustedRevision: entry.revision },
        ]),
      ),
    entry,
  );
  await page.goto("/");
  await commands(page, "Show workspace context");
  await expect(
    page.getByRole("heading", { name: "Workspace context" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Show details" }).click();
  const id = await page
    .locator(".plugin-panel")
    .getAttribute("data-plugin-pane-id");
  await commands(page, "Dock current tab");
  await page
    .getByRole("dialog", { name: "Dock current tab" })
    .getByRole("button", { name: "Dock", exact: true })
    .click();
  await expect(page.locator(".dock-pane-host .plugin-panel")).toHaveAttribute(
    "data-plugin-pane-id",
    id!,
  );
  await expect(
    page.getByRole("button", { name: "Hide details" }),
  ).toBeVisible();
  await expect(page.locator(".terminal-pane")).toHaveCount(1);
  await commands(page, "Move active panel");
  await chooseOption(
    page.getByRole("combobox", { name: "Docking position" }),
    "left",
  );
  await page
    .getByRole("dialog", { name: "Move active panel" })
    .getByRole("button", { name: "Dock", exact: true })
    .click();
  await expect
    .poll(async () => {
      const plugin = await page.locator(".plugin-panel").boundingBox(),
        terminal = await page.locator(".terminal-pane").boundingBox();
      return plugin!.x < terminal!.x;
    })
    .toBeTruthy();
  await page.screenshot({
    path: test.info().outputPath("mixed-plugin-dark.png"),
    animations: "disabled",
  });
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  await page.screenshot({
    path: test.info().outputPath("mixed-plugin-light.png"),
    animations: "disabled",
  });
  const pane = page.locator(".plugin-panel");
  const host = pane.locator("..");
  expect(
    Math.abs(
      (await pane.boundingBox())!.width - (await host.boundingBox())!.width,
    ),
  ).toBeLessThan(1);
  const handle = (await pane.locator(".editor-path").boundingBox())!;
  const terminal = (await page.locator(".terminal-pane").boundingBox())!;
  await page.keyboard.down("Control");
  await page.mouse.move(handle.x + 20, handle.y + handle.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    terminal.x + terminal.width / 2,
    terminal.y + terminal.height - 20,
    { steps: 8 },
  );
  await expect(page.locator(".pane-drop-preview")).toBeVisible();
  await page.mouse.up();
  await page.keyboard.up("Control");
  await expect
    .poll(
      async () =>
        (await pane.boundingBox())!.y >
        (await page.locator(".terminal-pane").boundingBox())!.y,
    )
    .toBeTruthy();
  await expect(pane).toHaveAttribute("data-plugin-pane-id", id!);
  await expect(
    pane.getByRole("button", { name: "Hide details" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (c: any) => c.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (c: any) => c.command === "close_terminal",
        ).length,
    ),
  ).toBe(0);
});

test("sidebar contributions retain placement, instance state and focused context independently of the central tab", async ({
  page,
}) => {
  const sidebar = {
    ...entry,
    enabled: true,
    trustedRevision: entry.revision,
    manifest: {
      ...manifest,
      contributes: {
        ...manifest.contributes,
        views: manifest.contributes.views.map((view) => ({
          ...view,
          placement: "sidebar",
        })),
      },
    },
  };
  await mockDesktop(page);
  await packageRoutes(page);
  await page.addInitScript((entry) => {
    if (!localStorage.getItem("test-plugins"))
      localStorage.setItem("test-plugins", JSON.stringify([entry]));
  }, sidebar);
  await page.goto("/");
  await commands(page, "Show workspace context");
  const panel = page.getByRole("complementary", { name: "Workspace context" });
  await expect(
    panel.getByRole("heading", { name: "Workspace context" }),
  ).toBeVisible();
  await panel.getByRole("button", { name: "Show details" }).click();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const { pluginHost } = await import("/src/plugins/runtime.ts");
        return pluginHost.context.viewType;
      }),
    )
    .toBe("lomi.context.view");
  await page
    .getByRole("button", { name: "Workspace context", exact: true })
    .click({ button: "right" });
  await page
    .getByRole("menuitemradio", { name: "Panel on the right", exact: true })
    .click();
  await expect(panel).toHaveAttribute("data-side", "right");
  await page
    .getByRole("button", { name: "Workspace context", exact: true })
    .click();
  await expect(panel).toHaveCount(0);
  await page
    .getByRole("button", { name: "Workspace context", exact: true })
    .click();
  await expect(
    panel.getByRole("button", { name: "Hide details" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")
            ?.sidebarSides["lomi.context.view"],
      ),
    )
    .toBe("right");
  await page.reload();
  await expect(
    panel.getByRole("button", { name: "Hide details" }),
  ).toBeVisible();
  await expect(panel).toHaveAttribute("data-side", "right");
  await expect(page.locator(".terminal-pane")).toHaveCount(1);
  await page.setViewportSize({ width: 800, height: 420 });
  await page.screenshot({
    path: test.info().outputPath("plugin-sidebar-minimum.png"),
  });
});
