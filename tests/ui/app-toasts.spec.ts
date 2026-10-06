import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import {
  newBrowserTab,
  newId,
  newProject,
  newSession,
  newTab,
} from "../../src/model";
import { mockDesktop } from "./desktop";

async function failSessionSave(page: Page) {
  await page.evaluate(() => {
    (window as any).__nativeTest.failSave = true;
  });
  await page.evaluate(() =>
    (window as any).__TAURI_INTERNALS__.invoke("request_quit"),
  );
  await expect(
    page.locator(".app-toast.notice").filter({ hasText: "Disk is full" }),
  ).toBeVisible();
}

for (const mode of ["dark", "light"] as const) {
  test(`${mode} alerts overlay the bottom right without resizing or focusing the workspace`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({ colorScheme: mode, reducedMotion: "reduce" });
    await page.clock.install();
    await mockDesktop(page);
    await page.goto("/");
    const terminal = page.locator(".xterm-helper-textarea");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    await terminal.focus();
    const before = await page.locator(".work-area").boundingBox();
    const paneBefore = await page.locator(".terminal-pane").boundingBox();
    await failSessionSave(page);
    await expect(terminal).toBeFocused();
    expect(await page.locator(".work-area").boundingBox()).toEqual(before);
    expect(await page.locator(".terminal-pane").boundingBox()).toEqual(
      paneBefore,
    );
    const toast = page.locator(".app-toast.notice");
    await expect(toast).toHaveCSS("animation-name", "none");
    const bounds = (await toast.boundingBox())!;
    expect(bounds.width).toBeLessThanOrEqual(380);
    expect(bounds.x + bounds.width).toBe(1428);
    expect(bounds.y).toBeGreaterThan(750);
    expect(bounds.y + bounds.height).toBeLessThan(900);
    await page.screenshot({ path: testInfo.outputPath(`toast-${mode}.png`) });
    await failSessionSave(page);
    await expect(toast).toHaveCount(1);
    await page.clock.fastForward(15000);
    await expect(toast).toBeVisible();
    await page.setViewportSize({ width: 800, height: 420 });
    await expect(toast).toBeInViewport();
    const compact = (await toast.boundingBox())!;
    expect(compact.x + compact.width).toBe(788);
    expect(compact.y).toBeGreaterThan(280);
    await page.screenshot({
      path: testInfo.outputPath(`toast-compact-${mode}.png`),
    });
    const compactArea = await page.locator(".work-area").boundingBox();
    await toast
      .getByRole("button", { name: "Dismiss message", exact: true })
      .click();
    await expect(toast).toHaveCount(0);
    expect(await page.locator(".work-area").boundingBox()).toEqual(compactArea);
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls.some(
          (call: any) => call.command === "plugin:window|destroy",
        ),
      ),
    ).toBe(false);
  });
}

test("information expires after five seconds and pauses for pointer and keyboard interaction", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.clock.install();
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.locator(".xterm-helper-textarea").focus();
  await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1000));
  await page.keyboard.press("Control+d");
  await page.keyboard.press("Control+d");
  const toast = page.locator(".app-toast.pane-limit-notice");
  await expect(toast).toContainText("No room");
  await page.clock.fastForward(2000);
  await toast.hover({ force: true });
  await page.clock.fastForward(6000);
  await expect(toast).toBeVisible();
  await toast.getByRole("button", { name: "Dismiss panel limit" }).focus();
  await page.mouse.move(0, 0);
  await page.clock.fastForward(6000);
  await expect(toast).toBeVisible();
  await page.locator(".terminal-pane.is-active .xterm-helper-textarea").focus();
  await page.clock.fastForward(1500);
  await expect(toast).toBeVisible();
  await page.clock.fastForward(2000);
  await expect(toast).toHaveCount(0);
});

test("recovery remains available alongside a separate error and never expires", async ({
  page,
}) => {
  await page.clock.install();
  await mockDesktop(page, false, { version: 99, projects: [] });
  await page.addInitScript(() =>
    localStorage.setItem(
      "test-session",
      JSON.stringify({ version: 99, projects: [] }),
    ),
  );
  await page.goto("/");
  const recover = page.getByRole("button", {
    name: "Save current layout instead",
    exact: true,
  });
  await expect(recover).toBeVisible();
  await page.evaluate(() => {
    (window as any).__nativeTest.failZoom = true;
  });
  await page.keyboard.press("Control+Minus");
  await expect(page.locator(".app-toast")).toHaveCount(2);
  await page.clock.fastForward(15000);
  await expect(recover).toBeVisible();
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("test-session")!).version,
    ),
  ).toBe(99);
  await page
    .getByRole("button", { name: "Dismiss message", exact: true })
    .click();
  await expect(recover).toBeVisible();
  await recover.click();
  await expect(page.locator(".app-toast")).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => JSON.parse(localStorage.getItem("test-session")!).version,
      ),
    )
    .toBe(newSession().version);
});

test("startup and application errors share a stack without covering each other", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    (window as any).__nativeTest.agentControlStartup = {
      supported: true,
      autoStart: true,
      error: "The server port is unavailable.",
    };
  });
  await page.goto("/");
  await expect(page.locator(".app-toast")).toContainText(
    "The server port is unavailable",
  );
  await failSessionSave(page);
  await expect(page.locator(".app-toast")).toHaveCount(2);
  const cards = await page.locator(".app-toast").all();
  const first = (await cards[0].boundingBox())!;
  const second = (await cards[1].boundingBox())!;
  expect(first.y + first.height).toBeLessThan(second.y);
  await page
    .getByRole("button", { name: "Dismiss MCP startup message", exact: true })
    .click();
  await expect(page.locator(".app-toast")).toHaveCount(1);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(page.locator(".app-toast")).toHaveCount(1);
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.agentControlStartup = { ...mock.agentControlStartup, error: null };
    await mock.emitEvent(
      "agent-control-startup-changed",
      mock.agentControlStartup,
    );
    await new Promise(requestAnimationFrame);
  });
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.agentControlStartup = {
      ...mock.agentControlStartup,
      error: "The server port is unavailable.",
    };
    await mock.emitEvent(
      "agent-control-startup-changed",
      mock.agentControlStartup,
    );
  });
  await expect(page.locator(".app-toast")).toHaveCount(2);
});

test("only native browsers intersecting the toast hide and they return without navigation", async ({
  page,
}) => {
  const project = newProject("/project", "local:bash");
  const tab = newTab("/project", "local:bash");
  const left = newBrowserTab("https://example.com/left");
  const right = newBrowserTab("https://example.com/right");
  tab.layout = {
    type: "split",
    id: newId(),
    axis: "horizontal",
    ratio: 0.5,
    first: left,
    second: right,
  };
  tab.activePaneId = left.id;
  project.workspaces[0].tabs = [tab];
  project.workspaces[0].activeTabId = tab.id;
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.goto("/");
  const browsers = () =>
    page.evaluate(() =>
      [...(window as any).__nativeTest.browsers.values()].map(
        (browser: any) => ({
          url: browser.url,
          visible: browser.visible,
          visits: browser.visits,
        }),
      ),
    );
  await expect.poll(browsers).toEqual([
    { url: left.url, visible: true, visits: 1 },
    { url: right.url, visible: true, visits: 1 },
  ]);
  await page.evaluate(() => {
    (window as any).__nativeTest.failZoom = true;
  });
  await page.keyboard.press("Control+Minus");
  const toast = page.locator(".app-toast.notice");
  await expect(toast).toContainText("Could not change zoom");
  await expect.poll(browsers).toEqual([
    { url: left.url, visible: true, visits: 1 },
    { url: right.url, visible: false, visits: 1 },
  ]);
  await toast
    .getByRole("button", { name: "Dismiss message", exact: true })
    .click();
  await expect.poll(browsers).toEqual([
    { url: left.url, visible: true, visits: 1 },
    { url: right.url, visible: true, visits: 1 },
  ]);
});

test("a new browser can navigate while an existing toast covers its native viewport", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await failSessionSave(page);
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page
    .getByRole("menuitem", { name: "New browser", exact: true })
    .click();
  const address = page.getByRole("combobox", { name: "Web address" });
  await address.fill("https://example.com/new");
  await address.press("Enter");
  await expect(address).toHaveValue("https://example.com/new");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "save_session")
            .at(-1)
            ?.args.data.projects[0].workspaces[0].tabs.at(-1)?.url,
      ),
    )
    .toBe("https://example.com/new");
  await expect(page.locator(".browser-message[role=alert]")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Dismiss message", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...(window as any).__nativeTest.browsers.values()].map(
          (browser: any) => browser.url,
        ),
      ),
    )
    .toEqual(["https://example.com/new"]);
});
