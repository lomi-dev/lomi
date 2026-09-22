import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

async function terminal(page: Page) {
  await expect(page.locator(".terminal-host").first()).toHaveCSS(
    "opacity",
    "1",
  );
  const paneId = (await page
    .locator("[data-pane-id]")
    .first()
    .getAttribute("data-pane-id"))!;
  const sessionId = await page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    return runningTerminal(id)!.sessionId;
  }, paneId);
  return { paneId, sessionId };
}
async function emit(page: Page, id: string, signal = "finished") {
  await page.evaluate(
    ({ id, signal }) => {
      const native = (window as any).__nativeTest;
      native.emit(id, "\x1b]777;notify;Lo");
      native.emit(id, `mi;claude;${signal}\x07`);
    },
    { id, signal },
  );
}
async function notices(page: Page) {
  return page.evaluate(() => (window as any).__nativeTest.agentNotifications);
}

test("hidden terminals notify once and preferences disable and re-enable alerts across windows", async ({
  page,
  context,
}, testInfo) => {
  await mockDesktop(page, false);
  await page.goto("/");
  const first = await terminal(page);
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await emit(page, first.sessionId);
  await expect
    .poll(() => notices(page))
    .toEqual([{ kind: "finished", context: "project · Default · Terminal" }]);
  await emit(page, first.sessionId);
  await page.evaluate(
    (id) => (window as any).__nativeTest.emit(id, "\r\nAFTER_SIGNAL\r\n"),
    first.sessionId,
  );
  await expect.poll(() => buffer(page, first.paneId)).toContain("AFTER_SIGNAL");
  expect(await notices(page)).toHaveLength(1);

  const settings = await context.newPage();
  await mockDesktop(settings, false);
  await settings.goto("/?window=settings&page=terminal");
  const toggle = settings.getByRole("switch", {
    name: "Agent notifications",
    exact: true,
  });
  await expect(toggle).toBeChecked();
  await toggle.uncheck();
  await expect(settings.getByRole("status")).toHaveText("Saved");
  await emit(page, first.sessionId, "attention");
  await page.evaluate(
    (id) => (window as any).__nativeTest.emit(id, "\r\nDISABLED_SIGNAL\r\n"),
    first.sessionId,
  );
  await expect
    .poll(() => buffer(page, first.paneId))
    .toContain("DISABLED_SIGNAL");
  expect(await notices(page)).toHaveLength(1);
  await settings.reload();
  await expect(toggle).not.toBeChecked();
  await toggle.check();
  await expect(settings.getByRole("status")).toHaveText("Saved");
  // A separate runtime has an independent cooldown.
  const second = await terminal(page);
  await emit(page, second.sessionId, "attention");
  await expect.poll(() => notices(page)).toHaveLength(2);
  await page.evaluate(() => {
    (window as any).__nativeTest.windowFocused = true;
  });
  await emit(page, second.sessionId, "finished");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (c: any) => c.command === "notify_agent",
          ).length,
      ),
    )
    .toBeGreaterThanOrEqual(3);
  expect(await notices(page)).toHaveLength(2);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (c: any) => c.command === "start_terminal",
        ).length,
    ),
  ).toBe(2);
  await settings.setViewportSize({ width: 920, height: 680 });
  await settings.screenshot({
    path: testInfo.outputPath("notification-settings.png"),
  });
  await settings.setViewportSize({ width: 560, height: 420 });
  expect(
    await settings.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await settings.screenshot({
    path: testInfo.outputPath("notification-settings-small.png"),
  });
});

test("setup requires explicit consent and re-review after a failed write", async ({
  page,
  context,
}, testInfo) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await terminal(page);
  const settings = await context.newPage();
  await mockDesktop(settings, false);
  await settings.goto("/?window=settings&page=terminal");
  await settings
    .getByRole("button", { name: "Configure Claude Code…" })
    .click();
  const dialog = page.getByRole("dialog", {
    name: "Configure Claude Code notifications",
  });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("/home/test/.claude/settings.json");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (c: any) => c.command === "enable_agent_notifications",
      ),
    ),
  ).toHaveLength(0);
  await settings
    .getByRole("button", { name: "Configure Claude Code…" })
    .click();
  await expect(dialog).toBeVisible();
  await page.evaluate(() => {
    (window as any).__nativeTest.agentNotificationSetupError =
      "Claude configuration changed. Review it again.";
  });
  await dialog.getByRole("button", { name: "Enable integration" }).click();
  await expect(dialog.getByRole("alert")).toContainText(
    "configuration changed",
  );
  await page.evaluate(() => {
    (window as any).__nativeTest.agentNotificationSetupError = "";
  });
  await dialog.getByRole("button", { name: "Review again" }).click();
  await expect(
    dialog.getByRole("button", { name: "Enable integration" }),
  ).toBeEnabled();
  await page.screenshot({
    path: testInfo.outputPath("notification-setup.png"),
  });
  await dialog.getByRole("button", { name: "Enable integration" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByText("Claude Code notifications are configured.", {
      exact: false,
    }),
  ).toBeVisible();
});

test("permission denial and disabling during a pending check suppress delivery", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  const first = await terminal(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.agentNotificationPermission = false;
  });
  await emit(page, first.sessionId);
  await expect(
    page.getByText("Agent notifications are blocked.", { exact: false }),
  ).toBeVisible();
  expect(await notices(page)).toHaveLength(0);
  await page.evaluate(() => {
    (window as any).__nativeTest.agentNotificationPermission = true;
    (window as any).__nativeTest.agentNotificationPermissionDelay = 500;
  });
  await emit(page, first.sessionId, "attention");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (c: any) =>
              c.command === "plugin:notification|is_permission_granted",
          ).length,
      ),
    )
    .toBe(2);
  await page.evaluate(async () => {
    const { defaultTerminalPreferences } =
      await import("/src/terminal-preferences.ts");
    localStorage.setItem(
      "test-terminal-preferences",
      JSON.stringify({
        version: 1,
        ...defaultTerminalPreferences,
        agentNotifications: false,
      }),
    );
    await (window as any).__nativeTest.emitEvent(
      "terminal-preferences-changed",
    );
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (c: any) => c.command === "load_terminal_preferences",
          ).length,
      ),
    )
    .toBeGreaterThan(1);
  await page.waitForTimeout(600);
  expect(await notices(page)).toHaveLength(0);
});
