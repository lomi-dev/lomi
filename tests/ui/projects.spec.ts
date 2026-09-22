import { test, expect } from "@playwright/test";
import { newProject, newSession } from "../../src/model";
import { mockDesktop, buffer } from "./desktop";

test("first launch waits for a folder and restores it after selection", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await mockDesktop(page, false, null);
  await page.goto("/");
  const trigger = page.getByRole("button", {
    name: "Open Recent Project",
    exact: true,
  });
  await expect(trigger).toBeVisible();
  await page.keyboard.press("Control+Shift+T");
  await expect(
    page.getByRole("main", { name: "No project open" }),
  ).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some((call: any) =>
        ["start_terminal", "list_directory", "git_status"].includes(
          call.command,
        ),
      ),
    ),
  ).toBe(false);
  await page.screenshot({ path: testInfo.outputPath("empty-project.png") });
  await trigger.click();
  await expect(page.getByRole("menuitem")).toHaveCount(1);
  await page.screenshot({
    path: testInfo.outputPath("empty-project-menu.png"),
  });
  await page.getByRole("menuitem", { name: "Open Local Folder…" }).click();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(page.locator(".project-switcher")).toHaveText("chosen folder");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.find(
          (call: any) => call.command === "plugin:dialog|open",
        ).args.options,
    ),
  ).toMatchObject({
    directory: true,
    multiple: false,
    defaultPath: "/home/test",
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")
            ?.projects[0]?.path,
      ),
    )
    .toBe("/chosen folder");
  await page.reload();
  await expect(page.locator(".project-switcher")).toHaveText("chosen folder");
  await expect(page.locator(".xterm-screen")).toBeVisible();
});

test("recent projects keep their workspaces and streaming terminals in order of last use", async ({
  page,
}, testInfo) => {
  const projects = ["/work/lomi", "/work/api", "/work/docs"].map((path) =>
    newProject(path, "local:bash"),
  );
  projects[1].workspaces[0].name = "Review";
  await mockDesktop(page, false, { ...newSession(), projects });
  await page.goto("/");
  await page.locator(".project-switcher").click();
  await expect(page.getByRole("menuitem")).toHaveText([
    "lomi",
    "api",
    "docs",
    "Open Local Folder…",
  ]);
  await page.screenshot({ path: testInfo.outputPath("recent-projects.png") });
  await page.getByRole("menuitem", { name: "api", exact: true }).click();
  await page
    .getByRole("button", { name: "Toggle workspaces", exact: true })
    .click();
  await expect(
    page
      .getByRole("navigation", { name: "Workspace list" })
      .getByRole("button", { name: /^Review / }),
  ).toHaveAttribute("aria-current", "true");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const paneId = await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id");
  await page.locator(".project-switcher").click();
  await page.getByRole("menuitem", { name: "lomi", exact: true }).click();
  await expect(page.locator(".project-switcher")).toHaveText("lomi");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    const started = native.calls.find(
      (call: any) =>
        call.command === "start_terminal" &&
        call.args.request.cwd === "/work/api",
    );
    native.emit(started.args.request.id, "\r\nPROJECT BACKGROUND OUTPUT\r\n");
  });
  await expect
    .poll(() => buffer(page, paneId!))
    .toContain("PROJECT BACKGROUND OUTPUT");
  await page.locator(".project-switcher").click();
  await page.getByRole("menuitem", { name: "api", exact: true }).click();
  await expect(page.locator("[data-pane-id]")).toHaveAttribute(
    "data-pane-id",
    paneId!,
  );
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(2);
  await page.locator(".project-switcher").click();
  await expect(page.getByRole("menuitem")).toHaveText([
    "api",
    "lomi",
    "docs",
    "Open Local Folder…",
  ]);
  await expect(
    page.getByRole("menuitem", { name: "api", exact: true }),
  ).toHaveAttribute("aria-current", "true");
  await page.screenshot({
    path: testInfo.outputPath("selected-project-menu.png"),
  });
  await expect
    .poll(() =>
      page.evaluate(() =>
        JSON.parse(
          localStorage.getItem("test-session") ?? "null",
        )?.projects.map((project: any) => project.path),
      ),
    )
    .toEqual(["/work/api", "/work/lomi", "/work/docs"]);
  await page.reload();
  await page.locator(".project-switcher").click();
  await expect(page.getByRole("menuitem")).toHaveText([
    "api",
    "lomi",
    "docs",
    "Open Local Folder…",
  ]);
});

test("cancelling the folder picker preserves an empty session", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.goto("/");
  await page.locator(".project-switcher").click();
  await page.evaluate(() => {
    (window as any).__nativeTest.folder = null;
  });
  await page.getByRole("menuitem", { name: "Open Local Folder…" }).click();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(page.locator(".project-switcher")).toHaveText(
    "Open Recent Project",
  );
  await expect(page.locator(".project-switcher")).toBeFocused();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some((call: any) =>
        ["validate_directory", "start_terminal"].includes(call.command),
      ),
    ),
  ).toBe(false);
  await expect
    .poll(() =>
      page.evaluate(() =>
        JSON.parse(localStorage.getItem("test-session") ?? "null"),
      ),
    )
    .toEqual(newSession());
  await page.reload();
  await expect(
    page.getByRole("main", { name: "No project open" }),
  ).toBeVisible();
});

test("an unavailable recent folder leaves the current project and history intact", async ({
  page,
}) => {
  const projects = ["/work/lomi", "/missing"].map((path) =>
    newProject(path, "local:bash"),
  );
  await mockDesktop(page, false, {
    ...newSession(),
    projects,
    activeProjectId: projects[0].id,
  });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const paneId = await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id");
  await page.evaluate(() => {
    (window as any).__nativeTest.directoryError =
      "Cannot open directory: folder not found";
  });
  await page.locator(".project-switcher").click();
  await page.getByRole("menuitem", { name: "missing", exact: true }).click();
  await expect(page.getByRole("alert").filter({ hasText: /\S/ })).toContainText(
    "Cannot open directory",
  );
  await expect(page.locator(".project-switcher")).toHaveText("lomi");
  await expect(page.locator("[data-pane-id]")).toHaveAttribute(
    "data-pane-id",
    paneId!,
  );
  await page.locator(".project-switcher").click();
  await expect(page.getByRole("menuitem")).toHaveText([
    "lomi",
    "missing",
    "Open Local Folder…",
  ]);
});

test("the compact menu supports keyboard navigation and long histories at minimum size", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 800, height: 420 });
  const projects = Array.from({ length: 20 }, (_, index) =>
    newProject(
      `/work/${index === 0 ? "a-very-long-project-name-that-should-stay-on-one-line" : `project-${index}`}`,
      "local:bash",
    ),
  );
  await mockDesktop(page, false, {
    ...newSession(),
    projects,
    activeProjectId: projects[0].id,
  });
  await page.goto("/");
  const trigger = page.locator(".project-switcher");
  await trigger.focus();
  await page.keyboard.press("ArrowUp");
  const open = page.getByRole("menuitem", { name: "Open Local Folder…" });
  await expect(open).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(page.getByRole("menuitem").first()).toBeFocused();
  await page.keyboard.press("End");
  await expect(open).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(
    page.getByRole("menuitem", { name: "project-19", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Home");
  await expect(page.getByRole("menuitem").first()).toBeFocused();
  await page.screenshot({
    path: testInfo.outputPath("project-menu-minimum.png"),
  });
  const bounds = await page.locator(".project-menu").boundingBox();
  expect(bounds!.width).toBeLessThanOrEqual(250);
  expect(bounds!.y + bounds!.height).toBeLessThan(420);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(page.getByRole("tab", { selected: true })).toBeFocused();
  await trigger.click();
  await page.locator(".titlebar-space").click();
  await expect(page.getByRole("menu")).toHaveCount(0);
});
