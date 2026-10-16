import { unavailableState } from "../../src/auth/model";
import { expect, test, type Page } from "@playwright/test";
import {
  newBrowserTab,
  newId,
  newProject,
  newSession,
  newTab,
} from "../../src/model";
import { mockDesktop } from "./desktop";
import {
  expectRecordedNotification,
  notificationInbox,
  openNotifications,
} from "./application-notification-helpers";

async function failSessionSave(page: Page) {
  await page.evaluate(() => {
    (window as any).__nativeTest.failSave = true;
  });
  await page.evaluate(() =>
    (window as any).__TAURI_INTERNALS__.invoke("request_quit"),
  );
  await expectRecordedNotification(
    page,
    "Could not close the window: Disk is full",
  );
}

for (const mode of ["dark", "light"] as const) {
  test(`${mode} application errors stay quiet and persist in the signed-out inbox`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({ colorScheme: mode, reducedMotion: "reduce" });
    await mockDesktop(page, false);
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    const terminal = page.locator(".xterm-helper-textarea");
    await terminal.focus();
    const before = await page.locator(".work-area").boundingBox();
    const paneBefore = await page.locator(".terminal-pane").boundingBox();
    await failSessionSave(page);
    await expect(terminal).toBeFocused();
    expect(await page.locator(".work-area").boundingBox()).toEqual(before);
    expect(await page.locator(".terminal-pane").boundingBox()).toEqual(
      paneBefore,
    );
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(notificationInbox(page)).toHaveCount(0);
    await failSessionSave(page);
    const entries = await page.evaluate(
      () => (window as any).__nativeTest.notifications.items,
    );
    const closeEntries = entries.filter(
      (item: any) => item.body === "Could not close the window: Disk is full",
    );
    expect(closeEntries).toHaveLength(1);
    expect(closeEntries[0]).toMatchObject({
      kind: "attention",
      source: "app",
      title: "Application needs attention",
      read: false,
    });
    await page.reload();
    const inbox = await openNotifications(page);
    await expect(inbox).toContainText("Disk is full");
    await page.screenshot({
      path: testInfo.outputPath(`application-notifications-${mode}.png`),
    });
    const closeEntry = inbox
      .locator(".notification-item")
      .filter({ hasText: "Could not close the window: Disk is full" });
    await closeEntry
      .getByRole("button", {
        name: "Dismiss Application needs attention",
        exact: true,
      })
      .click();
    await expect(closeEntry).toHaveCount(0);
  });
}

test("panel limit information stays in the inbox after five seconds", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.clock.install();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+d");
  await page.keyboard.press("Control+d");
  await expectRecordedNotification(page, "No room");
  await page.clock.fastForward(15000);
  const inbox = await openNotifications(page);
  await expect(inbox).toContainText("No room");
  expect(
    await page.evaluate(
      () => (window as any).__nativeTest.notifications.items[0],
    ),
  ).toMatchObject({ kind: "info", source: "app", title: "Application update" });
});

test("session recovery is explicit and failed saves retain the inbox action", async ({
  page,
}) => {
  const future = JSON.stringify({ version: 99, projects: [] });
  await mockDesktop(page, false);
  await page.addInitScript(
    (saved) => localStorage.setItem("test-session", saved),
    future,
  );
  await page.goto("/");
  await expectRecordedNotification(page, "unsupported format");
  const recover = page.getByRole("button", {
    name: "Save current layout instead",
    exact: true,
  });
  await expect(recover).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("test-session"))).toBe(
    future,
  );
  const inbox = await openNotifications(page);
  await expect(inbox).toContainText("Session recovery required");
  await page.evaluate(() => {
    (window as any).__nativeTest.failSave = true;
  });
  await recover.click();
  await expectRecordedNotification(page, "Disk is full");
  await expect(recover).toBeEnabled();
  expect(await page.evaluate(() => localStorage.getItem("test-session"))).toBe(
    future,
  );
  await page.evaluate(() => {
    (window as any).__nativeTest.failSave = false;
  });
  await recover.click();
  await expect(recover).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => JSON.parse(localStorage.getItem("test-session")!).version,
      ),
    )
    .toBe(newSession().version);
});

test("MCP errors deduplicate until the startup error clears", async ({
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
  await expectRecordedNotification(page, "The server port is unavailable");
  const count = () =>
    page.evaluate(
      () =>
        (window as any).__nativeTest.notifications.items.filter(
          (item: any) => item.source === "mcp",
        ).length,
    );
  await expect.poll(count).toBe(1);
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    window.dispatchEvent(new Event("focus"));
    await mock.emitEvent(
      "agent-control-startup-changed",
      mock.agentControlStartup,
    );
  });
  await expect.poll(count).toBe(1);
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
  await expect.poll(count).toBe(2);
  await openNotifications(page);
  await expect(notificationInbox(page)).toContainText("MCP needs attention");
  await notificationInbox(page)
    .getByRole("button", { name: "Agent control settings", exact: true })
    .first()
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "open_settings")
            .at(-1)?.args,
      ),
    )
    .toEqual({ page: "agent-control" });
});

test("quiet application errors preserve native browser visibility and navigation", async ({
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
  const expected = [
    { url: left.url, visible: true, visits: 1 },
    { url: right.url, visible: true, visits: 1 },
  ];
  await expect.poll(browsers).toEqual(expected);
  const bounds = await page.locator(".work-area").boundingBox();
  await page.evaluate(() => {
    (window as any).__nativeTest.failZoom = true;
  });
  await page.keyboard.press("Control+Minus");
  await expectRecordedNotification(page, "Could not change zoom");
  await expect.poll(browsers).toEqual(expected);
  expect(await page.locator(".work-area").boundingBox()).toEqual(bounds);
});

test("notification storage failures stay quiet until the inbox opens", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    (window as any).__nativeTest.failNotificationCommand =
      "record_app_notification";
    (window as any).__nativeTest.failZoom = true;
  });
  await page.keyboard.press("Control+Minus");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "record_app_notification",
          ).length,
      ),
    )
    .toBe(1);
  await expect(page.locator(".app-toast")).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(notificationInbox(page)).toHaveCount(0);
  const inbox = await openNotifications(page);
  await expect(inbox.getByRole("status")).toContainText(
    "Notification storage is unavailable",
  );
  await expect(inbox).toContainText("Could not change zoom");
  await expect(inbox).toContainText("Not saved yet");
  await page.evaluate(() => {
    (window as any).__nativeTest.failNotificationCommand = null;
  });
  await inbox.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(inbox.getByRole("status")).toHaveCount(0);
  await expect(inbox).not.toContainText("Not saved yet");
  await expectRecordedNotification(page, "Could not change zoom");
  await expect(inbox.locator(".notification-item")).toHaveCount(1);
});

test("MCP startup loading can be retried from its inbox entry", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    (window as any).__nativeTest.failAgentControlStartupLoad = true;
  });
  await page.goto("/");
  await expectRecordedNotification(page, "MCP startup state is unavailable");
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toHaveCount(0);
  const inbox = await openNotifications(page);
  await expect(inbox).toContainText("MCP needs attention");
  expect(
    await page.evaluate(
      () => (window as any).__nativeTest.notifications.items[0],
    ),
  ).toMatchObject({
    source: "mcp",
    kind: "attention",
    action: "retry-mcp-startup",
  });
  const startupLoads = () =>
    page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "agent_control_startup_state",
        ).length,
    );
  const beforeRetry = await startupLoads();
  await page.evaluate(() => {
    (window as any).__nativeTest.failAgentControlStartupLoad = false;
  });
  await inbox.getByRole("button", { name: "Retry", exact: true }).click();
  await expect.poll(startupLoads).toBe(beforeRetry + 1);
});

test("application entries remain accessible after signing out", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript((unavailable) => {
    const desktop = window as any;
    let state = {
      ...unavailable,
      revision: 1,
      status: "signed-in",
      user: {
        id: "user",
        displayName: "Example User",
        email: null,
        githubLogin: null,
        status: "active",
      },
      session: { id: "session", expiresAt: "2030-01-01" },
    };
    desktop.__authInvoke = async (command: string) => {
      if (command === "auth_sign_out")
        state = {
          ...state,
          revision: state.revision + 1,
          status: "signed-out",
          user: null as any,
          session: null as any,
        };
      return state;
    };
  }, unavailableState);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await failSessionSave(page);
  const entries = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("test-notifications")!).items.map(
      (item: any) => item.id,
    ),
  );
  await page.locator(".titlebar-account-avatar").click();
  await page.getByRole("menuitem", { name: "Sign out", exact: true }).click();
  await expect(
    page.locator(".titlebar .titlebar-account-menu-trigger"),
  ).toBeVisible();
  await openNotifications(page);
  await expect(notificationInbox(page)).toContainText("Disk is full");
  expect(
    await page.evaluate(() =>
      JSON.parse(localStorage.getItem("test-notifications")!).items.map(
        (item: any) => item.id,
      ),
    ),
  ).toEqual(expect.arrayContaining(entries));
});

test("repeated operational errors deduplicate for five seconds", async ({
  page,
}) => {
  await page.clock.install();
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    (window as any).__nativeTest.failZoom = true;
  });
  const count = () =>
    page.evaluate(
      () => (window as any).__nativeTest.notifications.items.length,
    );
  await page.keyboard.press("Control+Minus");
  await expect.poll(count).toBe(1);
  await page.keyboard.press("Control+Minus");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) =>
              call.command === "plugin:webview|set_webview_zoom" &&
              call.args.value === 0.9,
          ).length,
      ),
    )
    .toBe(2);
  expect(await count()).toBe(1);
  await page.clock.fastForward(5001);
  await page.keyboard.press("Control+Minus");
  await expect.poll(count).toBe(2);
});

test("clearing the inbox keeps unresolved session recovery available", async ({
  page,
}) => {
  const future = JSON.stringify({ version: 99, projects: [] });
  await mockDesktop(page, false);
  await page.addInitScript(
    (saved) => localStorage.setItem("test-session", saved),
    future,
  );
  await page.goto("/");
  await expectRecordedNotification(page, "unsupported format");
  const inbox = await openNotifications(page);
  await inbox
    .getByRole("button", { name: "Clear all notifications", exact: true })
    .click();
  await expect(inbox.locator(".notification-item")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("test-session"))).toBe(
    future,
  );
  const recover = inbox.getByRole("button", {
    name: "Save current layout instead",
    exact: true,
  });
  await expect(recover).toBeVisible();
  await recover.click();
  await expect(recover).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => JSON.parse(localStorage.getItem("test-session")!).version,
      ),
    )
    .toBe(newSession().version);
});
