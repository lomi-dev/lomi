import { expectRecordedNotification } from "./application-notification-helpers";
import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import {
  newId,
  newPane,
  newProject,
  newSession,
  newTab,
  panes,
} from "../../src/model";
import type { Layout, Session } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";

function savedPanels(count: number) {
  const project = newProject("/project", "local:bash");
  const tab = newTab("/project", "local:bash");
  const tree = (start: number, count: number): Layout => {
    if (count === 1) return newPane(`/project/panel-${start}`);
    const firstCount = Math.floor(count / 2);
    return {
      type: "split",
      id: newId(),
      axis: "horizontal",
      ratio: 0.5,
      first: tree(start, firstCount),
      second: tree(start + firstCount, count - firstCount),
    };
  };
  tab.layout = tree(0, count);
  tab.activePaneId = panes(tab.layout).at(-1)!.id;
  project.workspaces[0].tabs = [tab];
  project.workspaces[0].activeTabId = tab.id;
  return {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
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

async function savedSession(page: Page): Promise<Session | null> {
  return page.evaluate(() =>
    JSON.parse(localStorage.getItem("test-session") ?? "null"),
  );
}

async function expectUsablePanels(page: Page, count: number) {
  await expect(page.locator("[data-pane-id]")).toHaveCount(count);
  const sizes = await page.locator("[data-pane-id]").evaluateAll((panels) =>
    panels.map((panel) => {
      const { width, height } = panel.getBoundingClientRect();
      return { width, height };
    }),
  );
  for (const size of sizes) {
    expect(size.width).toBeGreaterThanOrEqual(239.9);
    expect(size.height).toBeGreaterThanOrEqual(119.9);
  }
}

test("a queued shortcut burst respects panel dimensions and never reaches the PTY", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    for (let index = 0; index < 200; index++)
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "d",
          code: "KeyD",
          ctrlKey: true,
          bubbles: true,
          cancelable: true,
        }),
      );
  });
  await expectUsablePanels(page, 2);
  await expectRecordedNotification(page, "No room");
  await expect
    .poll(async () => (await calls(page, "start_terminal")).length)
    .toBe(2);
  await page.keyboard.press("Control+Shift+d");
  await expectUsablePanels(page, 3);
  await page.keyboard.press("Control+Shift+d");
  await expectUsablePanels(page, 3);
  expect(await calls(page, "write_terminal")).toHaveLength(0);
  await page.screenshot({ path: "test-results/panel-limit-minimum.png" });

  await page.setViewportSize({ width: 1800, height: 900 });
  await page.keyboard.press("Control+d");
  await expectUsablePanels(page, 4);
  await expect(page.locator(".pane-limit-notice")).toHaveCount(0);
  await expect(page.getByRole("tab")).toHaveCount(1);
  await page.evaluate(() => {
    for (let index = 0; index < 4; index++)
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "w",
          code: "KeyW",
          ctrlKey: true,
          bubbles: true,
          cancelable: true,
        }),
      );
  });
  // Repeated closes during a process check must not spill into neighboring panes.
  await expectUsablePanels(page, 3);
  await page.keyboard.press("Control+w");
  await expectUsablePanels(page, 2);
  await page.keyboard.press("Control+w");
  await expectUsablePanels(page, 1);
  await expect(page.getByRole("tab")).toHaveCount(1);
});

test("divider resizing protects nested panels and a smaller window keeps their shells alive", async ({
  page,
}) => {
  await mockDesktop(page, true, savedPanels(3));
  await page.goto("/");
  await expectUsablePanels(page, 3);
  await expect
    .poll(async () => (await calls(page, "start_terminal")).length)
    .toBe(3);
  const firstPane = await page
    .locator("[data-pane-id]")
    .first()
    .getAttribute("data-pane-id");
  const separator = page
    .getByRole("separator", { name: "Resize terminal columns" })
    .first();
  await separator.focus();
  for (let index = 0; index < 20; index++)
    await page.keyboard.press("ArrowRight");
  await expectUsablePanels(page, 3);
  const bounds = (await separator.boundingBox())!;
  const area = (await page.locator(".terminal-layout").boundingBox())!;
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + 40);
  await page.mouse.down();
  await page.mouse.move(area.x + 1, bounds.y + 40, { steps: 4 });
  await page.mouse.up();
  await expectUsablePanels(page, 3);

  await page.setViewportSize({ width: 800, height: 420 });
  await expect(page.locator(".layout-recovery")).toBeVisible();
  await expect(page.locator(".xterm-screen")).toHaveCount(0);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  await page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const native = (window as any).__nativeTest;
    native.emit(runningTerminal(id!)!.sessionId, "\r\nBackground output\r\n");
  }, firstPane);
  await expect
    .poll(() => buffer(page, firstPane!))
    .toContain("Background output");
  await page.screenshot({ path: "test-results/panel-recovery-minimum.png" });
  await page.keyboard.press("Control+Shift+e");
  await expectUsablePanels(page, 3);
  await expect(page.locator(".layout-recovery")).toHaveCount(0);
  expect(await calls(page, "start_terminal")).toHaveLength(3);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  await page.screenshot({
    path: "test-results/panels-after-window-resize.png",
  });
});

test("an oversized saved tab starts no shells and preserves its layout until recovery", async ({
  page,
}) => {
  const saved = savedPanels(512);
  const workspace = saved.projects[0].workspaces[0];
  const tab = workspace.tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected a terminal tab");
  const kept = panes(tab.layout).at(-1)!;
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await mockDesktop(page, true, saved);
  await page.goto("/");
  await expect(page.locator(".layout-recovery")).toContainText("512 panels");
  await expect(page.locator(".terminal-host")).toHaveCount(0);
  expect(await calls(page, "start_terminal")).toHaveLength(0);
  await expect.poll(() => savedSession(page)).toEqual(saved);
  await page.reload();
  await expect(page.locator(".layout-recovery")).toBeVisible();
  expect(await calls(page, "start_terminal")).toHaveLength(0);
  await page.keyboard.press("Control+Shift+T");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await expect(page.locator(".layout-recovery")).toBeVisible();
  expect(await calls(page, "start_terminal")).toHaveLength(1);
  await page
    .getByRole("button", { name: "Keep only the active panel" })
    .click();
  await expect(page.locator("[data-pane-id]")).toHaveAttribute(
    "data-pane-id",
    kept.id,
  );
  await expect
    .poll(async () => (await calls(page, "start_terminal")).length)
    .toBe(2);
  expect((await calls(page, "start_terminal"))[1].args.request.cwd).toBe(
    kept.cwd,
  );
  await expect
    .poll(async () => {
      const restored = await savedSession(page);
      return restored?.projects[0].workspaces[0].tabs[0];
    })
    .toEqual({ ...tab, layout: kept });
  await page.reload();
  await expect(page.locator("[data-pane-id]")).toHaveAttribute(
    "data-pane-id",
    kept.id,
  );
  await expect(page.getByRole("tab")).toHaveCount(2);
  expect(await calls(page, "start_terminal")).toHaveLength(1);
  expect(errors).toEqual([]);
});

test("closing a saved panel can make the remaining layout fit without starting the closed shell", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  const saved = savedPanels(3);
  const tab = saved.projects[0].workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected a terminal tab");
  const closedId = tab.activePaneId;
  await mockDesktop(page, true, saved);
  await page.goto("/");
  await expect(page.locator(".layout-recovery")).toBeVisible();
  expect(await calls(page, "start_terminal")).toHaveLength(0);
  await page.keyboard.press("Control+w");
  await expectUsablePanels(page, 2);
  await expect(page.locator(`[data-pane-id="${closedId}"]`)).toHaveCount(0);
  await expect
    .poll(async () => (await calls(page, "start_terminal")).length)
    .toBe(2);
  expect(
    (await calls(page, "start_terminal")).map(
      (call: any) => call.args.request.cwd,
    ),
  ).toEqual(["/project/panel-0", "/project/panel-1"]);
});
