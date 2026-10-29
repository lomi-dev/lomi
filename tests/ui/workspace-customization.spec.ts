import { expect, test } from "@playwright/test";
import {
  active,
  addWorkspace,
  layoutPanes,
  newSession,
  splitPane,
} from "../../src/model";
import { mockDesktop } from "./desktop";

test("workspace appearance presets save on an inactive workspace and survive reload, cancellation and reset", async ({
  page,
}, testInfo) => {
  let session = addWorkspace(newSession(), "/voice", "local:bash", "Voice");
  session = addWorkspace(session, "/bench", "local:bash", "Bench");
  session.sidebar = "workspaces";
  await mockDesktop(page, false, session);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const voice = page.locator(".workspace-list-entry").filter({
    has: page.locator(".workspace-list-item", { hasText: "Voice" }),
  });
  const customize = async () => {
    await voice
      .getByRole("button", { name: "Actions for Voice", exact: true })
      .click();
    await page
      .getByRole("menuitem", { name: "Customize workspace…", exact: true })
      .click();
  };
  const savedAppearance = () =>
    page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("test-session") ?? "null")
          ?.projects?.[0]?.workspaces?.[0]?.appearance,
    );
  await customize();
  const dialog = page.getByRole("dialog", {
    name: "Customize workspace",
    exact: true,
  });
  await expect(dialog.getByRole("textbox")).toHaveCount(0);
  await expect(
    dialog.locator('input[type="file"], input[type="color"]'),
  ).toHaveCount(0);
  await dialog
    .getByRole("button", { name: "Rocket icon", exact: true })
    .click();
  await dialog
    .getByRole("button", { name: "Use color #6ba8ff", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Rocket icon", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.screenshot({
    path: testInfo.outputPath("workspace-appearance-dark.png"),
  });
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    voice.locator(".workspace-avatar svg.lucide-rocket"),
  ).toBeVisible();
  await expect(voice.locator(".workspace-avatar")).toHaveCSS(
    "color",
    "rgb(107, 168, 255)",
  );
  await expect(voice.locator(".workspace-folder")).toHaveText("/voice");
  await expect(
    page.locator(".workspace-list-item[aria-current=true]"),
  ).toContainText("Bench");
  await expect
    .poll(savedAppearance)
    .toEqual({ icon: "rocket", color: "#6ba8ff" });
  await page.reload();
  await expect(
    voice.locator(".workspace-avatar svg.lucide-rocket"),
  ).toBeVisible();
  await customize();
  await expect(
    dialog.getByRole("button", { name: "Rocket icon", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    dialog.getByRole("button", { name: "Use color #6ba8ff", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await dialog
    .getByRole("button", { name: "Folder icon", exact: true })
    .click();
  await dialog
    .getByRole("button", { name: "Use color #ef8aab", exact: true })
    .click();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect
    .poll(savedAppearance)
    .toEqual({ icon: "rocket", color: "#6ba8ff" });
  await customize();
  await dialog.getByRole("button", { name: "Reset", exact: true }).click();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    voice.locator(".workspace-avatar svg.lucide-rocket"),
  ).toBeVisible();
  await expect
    .poll(savedAppearance)
    .toEqual({ icon: "rocket", color: "#6ba8ff" });
  await page.setViewportSize({ width: 800, height: 420 });
  await page.emulateMedia({ colorScheme: "light" });
  await customize();
  await expect(
    dialog.getByRole("button", { name: "Save", exact: true }),
  ).toBeInViewport();
  await page.screenshot({
    path: testInfo.outputPath("workspace-appearance-light-minimum.png"),
  });
  await dialog.getByRole("button", { name: "Reset", exact: true }).click();
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(savedAppearance).toBeUndefined();
  await expect(voice.locator(".workspace-avatar")).toHaveCount(0);
});

for (const image of [
  undefined,
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=",
]) {
  test(`workspace customization preserves a stored custom ${image ? "image" : "icon"} and color until a preset is selected`, async ({
    page,
  }) => {
    const session = addWorkspace(newSession(), "/voice", "local:bash", "Voice");
    const appearance = {
      icon: "🚀",
      color: "#123456",
      ...(image ? { image } : {}),
    };
    session.projects[0].workspaces[0].appearance = appearance;
    session.sidebar = "workspaces";
    await mockDesktop(page, false, session);
    await page.goto("/");
    const voice = page.locator(".workspace-list-entry").filter({
      has: page.locator(".workspace-list-item", { hasText: "Voice" }),
    });
    const customize = async () => {
      await voice
        .getByRole("button", { name: "Actions for Voice", exact: true })
        .click();
      await page
        .getByRole("menuitem", { name: "Customize workspace…", exact: true })
        .click();
    };
    const savedAppearance = () =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")
            ?.projects?.[0]?.workspaces?.[0]?.appearance,
      );
    const dialog = page.getByRole("dialog", {
      name: "Customize workspace",
      exact: true,
    });
    await customize();
    if (image)
      await expect(dialog.locator(".workspace-avatar img")).toBeVisible();
    else await expect(dialog.locator(".workspace-avatar")).toHaveText("🚀");
    await expect(
      dialog.locator('.workspace-icon-options button[aria-pressed="true"]'),
    ).toHaveCount(0);
    await expect(
      dialog.locator('.workspace-color-options button[aria-pressed="true"]'),
    ).toHaveCount(0);
    await dialog.getByRole("button", { name: "Save", exact: true }).click();
    await expect.poll(savedAppearance).toEqual(appearance);
    await page.reload();
    await customize();
    if (image)
      await expect(dialog.locator(".workspace-avatar img")).toBeVisible();
    else await expect(dialog.locator(".workspace-avatar")).toHaveText("🚀");
    await dialog
      .getByRole("button", { name: "Folder icon", exact: true })
      .click();
    await expect(dialog.locator(".workspace-avatar img")).toHaveCount(0);
    await dialog.getByRole("button", { name: "Save", exact: true }).click();
    await expect
      .poll(savedAppearance)
      .toEqual({ icon: "folder", color: "#123456" });
  });
}

test("workspace summaries track real CLI processes in split tabs without starting inactive terminals", async ({
  page,
}, testInfo) => {
  let session = addWorkspace(newSession(), "/project", "local:bash", "Build");
  const tab = active(session)!.tab;
  if (tab.type !== "terminal") throw new Error("Expected terminal");
  tab.layout = splitPane(tab.layout, tab.activePaneId, "horizontal", {
    type: "terminal",
    id: "second-pane",
    cwd: "/project",
  });
  session = addWorkspace(session, "/other", "local:bash", "Review");
  session.sidebar = "workspaces";
  session.sidebarWidth = 300;
  session.activeProjectId = session.projects[0].id;
  await mockDesktop(page, true, session);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(() => {
    const native = (window as any).__nativeTest;
    const original = (window as any).__TAURI_INTERNALS__.invoke;
    (window as any).__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: any,
    ) => {
      if (command === "git_repositories")
        return {
          repositories:
            args.root === "/project"
              ? [
                  { root: "/project", branch: "main", changes: [] },
                  { root: "/project/nested", branch: "main", changes: [] },
                ]
              : [],
          errors: [],
          limited: false,
        };
      return original(command, args);
    };
    native.terminalContexts = {};
  });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  const build = page.locator(".workspace-list-entry").filter({
    has: page.locator(".workspace-list-item", { hasText: "Build" }),
  });
  const review = page.locator(".workspace-list-entry").filter({
    has: page.locator(".workspace-list-item", { hasText: "Review" }),
  });
  await expect(build.locator(".workspace-metadata")).toHaveText("main");
  await expect(review.locator(".workspace-metadata")).toHaveCount(0);
  await expect(build.locator(".workspace-avatar")).toHaveCount(0);
  await expect(build.locator(".workspace-name-row > svg")).toBeVisible();
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    const starts = native.calls.filter(
      (call: any) => call.command === "start_terminal",
    );
    native.terminalContexts = Object.fromEntries(
      starts.map((call: any, index: number) => [
        call.args.request.id,
        {
          cwd: "/project",
          foregroundProgram: index ? "claude" : "codex",
          titleCli: { cli: index ? "claude" : "codex", pid: 200 + index },
        },
      ]),
    );
  });
  await expect(build.locator(".workspace-agent-chip")).toHaveCount(2);
  await expect(build.locator(".workspace-agent-chip")).toHaveText(["", ""]);
  await expect(
    build.getByRole("img", { name: "Codex · 1 agent", exact: true }),
  ).toBeVisible();
  await expect(
    build.getByRole("img", { name: "Claude Code · 1 agent", exact: true }),
  ).toBeVisible();
  await expect(review.locator(".workspace-agent-chip")).toHaveCount(0);
  await build.locator(".workspace-list-item").focus();
  await expect(build.locator(".workspace-menu-trigger")).toHaveCSS(
    "opacity",
    "1",
  );
  await expect(build.locator(".workspace-tabs-toggle")).toHaveCSS(
    "opacity",
    "1",
  );
  await build
    .getByRole("button", { name: "Expand items in Build", exact: true })
    .click();
  const items = build.getByRole("list", {
    name: "Open items in Build",
    exact: true,
  });
  const panels = items.locator(".workspace-open-item");
  const panes = layoutPanes(tab.layout);
  await expect(panels).toHaveCount(2);
  for (const pane of panes)
    await expect(
      items.locator(
        `.workspace-open-item[data-workspace-pane-id="${pane.id}"]`,
      ),
    ).toHaveAttribute("data-owner-tab-id", tab.id);
  await expect(
    items
      .locator('.workspace-open-item[data-workspace-pane-id="second-pane"]')
      .locator('.workspace-cli-icon[title^="Claude Code"]'),
  ).toBeVisible();
  await expect(
    items
      .locator(
        `.workspace-open-item[data-workspace-pane-id="${tab.activePaneId}"]`,
      )
      .locator('.workspace-cli-icon[title^="Codex"]'),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("workspace-agents-dark.png"),
  });
  await review.locator(".workspace-list-item").click();
  await expect(build.locator(".workspace-agent-chip")).toHaveCount(2);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(3);
  await page.evaluate(() => {
    (window as any).__nativeTest.terminalContexts = {};
  });
  await expect(build.locator(".workspace-agent-chip")).toHaveCount(0);
});
