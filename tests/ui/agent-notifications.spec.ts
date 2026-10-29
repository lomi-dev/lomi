import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

const notificationId = expect.stringMatching(
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
);

async function optedInDesktop(page: Page) {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-terminal-notification-policy",
      JSON.stringify({
        accountId: "user-a",
        available: true,
        preferences: { enabled: true, backgroundOnly: true },
      }),
    );
    localStorage.setItem(
      "test-terminal-preferences",
      JSON.stringify({
        version: 1,
        agentNotifications: false,
      }),
    );
  });
}

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
    // Vite versions modules after edits. Read the same runtime instance that
    // mounted the terminal instead of creating a second, empty registry.
    const runtimeUrl = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((url) => new URL(url).pathname === "/src/terminal-runtime.ts")
      .at(-1);
    const { runningTerminal } = await import(
      runtimeUrl ?? "/src/terminal-runtime.ts"
    );
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

test("hidden terminals obey native account policy while ignoring the legacy preference", async ({
  page,
  context,
}, testInfo) => {
  await optedInDesktop(page);
  await page.goto("/");
  const first = await terminal(page);
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await emit(page, first.sessionId);
  await expect
    .poll(() => notices(page))
    .toEqual([
      {
        notificationId,
        kind: "finished",
        context: "project · Default · Terminal",
        paneId: first.paneId,
        source: "claude",
        sessionId: first.sessionId,
      },
    ]);
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
  await expect(
    settings.getByRole("switch", { name: "Agent notifications", exact: true }),
  ).toHaveCount(0);
  await expect(
    settings.getByRole("button", { name: "Configure Claude Code…" }),
  ).toHaveCount(0);
  await page.evaluate(() => {
    const policy = JSON.parse(
      localStorage.getItem("test-terminal-notification-policy")!,
    );
    policy.preferences.enabled = false;
    localStorage.setItem(
      "test-terminal-notification-policy",
      JSON.stringify(policy),
    );
  });
  await emit(page, first.sessionId, "attention");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (c: any) => c.command === "notify_agent",
          ).length,
      ),
    )
    .toBe(2);
  expect(await notices(page)).toHaveLength(1);
  await page.evaluate(() => {
    const policy = JSON.parse(
      localStorage.getItem("test-terminal-notification-policy")!,
    );
    policy.preferences.enabled = true;
    localStorage.setItem(
      "test-terminal-notification-policy",
      JSON.stringify(policy),
    );
  });
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
  await optedInDesktop(page);
  await page.goto("/");
  await terminal(page);
  const settings = await context.newPage();
  await mockDesktop(settings, false);
  await settings.goto("/?window=settings&page=terminal");
  await settings.evaluate(async () => {
    await (window as any).__TAURI_INTERNALS__.invoke(
      "request_agent_notification_setup",
    );
  });
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
  await settings.evaluate(async () => {
    await (window as any).__TAURI_INTERNALS__.invoke(
      "request_agent_notification_setup",
    );
  });
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
  // Successful setup follows the application's quiet, persistent inbox flow.
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(
    page.getByRole("dialog", { name: "Notifications", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Open account menu" }).click();
  await page
    .getByRole("menuitem", { name: "Notifications", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "Notifications", exact: true }),
  ).toContainText("Claude Code notifications are configured.");
});

test("denied system permission keeps inbox notifications and receives no in-app alert", async ({
  page,
}) => {
  await optedInDesktop(page);
  await page.goto("/");
  const first = await terminal(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.agentNotificationPermission = false;
  });
  const permissionChecksBeforeSignal = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (c: any) => c.command === "plugin:notification|is_permission_granted",
      ).length,
  );
  await emit(page, first.sessionId);
  await expect
    .poll(() =>
      page.evaluate(
        async () =>
          (
            await (window as any).__TAURI_INTERNALS__.invoke(
              "load_notifications",
            )
          ).items.length,
      ),
    )
    .toBe(1);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("dialog", { name: "Notifications" })).toHaveCount(
    0,
  );
  expect(await notices(page)).toHaveLength(0);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (c: any) => c.command === "plugin:notification|is_permission_granted",
      ),
    ),
  ).toHaveLength(permissionChecksBeforeSignal);
  await emit(page, first.sessionId, "attention");
  await expect
    .poll(() =>
      page.evaluate(
        async () =>
          (
            await (window as any).__TAURI_INTERNALS__.invoke(
              "load_notifications",
            )
          ).items.length,
      ),
    )
    .toBe(2);
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(await notices(page)).toHaveLength(0);
  expect(
    await page.evaluate(async () => {
      const inbox = await (window as any).__TAURI_INTERNALS__.invoke(
        "load_notifications",
      );
      return inbox.items.map((item: any) => item.agent);
    }),
  ).toEqual(["claude", "claude"]);
});

test("notification attribution survives process changes and unknown senders stay generic", async ({
  page,
}) => {
  await optedInDesktop(page);
  await page.goto("/");
  const first = await terminal(page);
  await page.evaluate((id) => {
    const native = (window as any).__nativeTest;
    native.terminalContexts[id] = {
      cwd: "/project",
      foregroundProgram: "codex",
      titleCli: { cli: "codex", pid: 42 },
    };
    native.emit(id, "\x1b]9;PRIVATE_SOURCE_TEXT\x07");
  }, first.sessionId);
  const agents = () =>
    page.evaluate(async () => {
      const inbox = await (window as any).__TAURI_INTERNALS__.invoke(
        "load_notifications",
      );
      return inbox.items.map((item: any) => item.agent ?? null);
    });
  await expect.poll(agents).toEqual(["codex"]);
  await page.evaluate((id) => {
    (window as any).__nativeTest.terminalContexts[id].titleCli = {
      cli: "claude",
      pid: 43,
    };
  }, first.sessionId);
  expect(await agents()).toEqual(["codex"]);
  await page.reload();
  const second = await terminal(page);
  expect(await agents()).toEqual(["codex"]);
  await page.evaluate((id) => {
    (window as any).__nativeTest.emit(id, "\x1b]9;Notification from Codex\x07");
  }, second.sessionId);
  await expect.poll(agents).toEqual([null, "codex"]);
  expect(
    await page.evaluate(async () =>
      JSON.stringify(
        await (window as any).__TAURI_INTERNALS__.invoke("load_notifications"),
      ),
    ),
  ).not.toContain("PRIVATE_SOURCE_TEXT");
});

test("hidden OSC 9 alerts expose no terminal text and ignore duplicate and progress reports", async ({
  page,
}) => {
  await optedInDesktop(page);
  await page.goto("/");
  const first = await terminal(page);
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);

  const privateText = "PRIVATE_OSC9_TERMINAL_CONTENT";
  await page.evaluate(
    ({ id, privateText }) =>
      (window as any).__nativeTest.emit(id, `\x1b]9;${privateText}\x07`),
    { id: first.sessionId, privateText },
  );
  await expect
    .poll(() => notices(page))
    .toEqual([
      {
        notificationId,
        kind: "attention",
        context: "project · Default · Terminal",
        paneId: first.paneId,
        source: "terminal",
        sessionId: first.sessionId,
      },
    ]);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "notify_agent",
      ),
    ),
  ).toEqual([
    {
      command: "notify_agent",
      args: {
        notificationId,
        kind: "attention",
        context: "project · Default · Terminal",
        paneId: first.paneId,
        source: "terminal",
        sessionId: first.sessionId,
      },
    },
  ]);

  const previousAcknowledgements = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "acknowledge_terminal",
      ).length,
  );
  await page.evaluate(
    ({ id, privateText }) => {
      const native = (window as any).__nativeTest;
      native.emit(id, `\x1b]9;${privateText}\x07`);
      native.emit(id, "\x1b]9;4;50;100\x07");
      native.emit(id, "\x1b]9;4;0\x07");
    },
    { id: first.sessionId, privateText },
  );
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "acknowledge_terminal",
          ).length,
      ),
    )
    .toBeGreaterThan(previousAcknowledgements);
  await expect.poll(() => notices(page)).toHaveLength(1);
  expect(JSON.stringify(await notices(page))).not.toContain(privateText);
  expect(await buffer(page, first.paneId)).not.toContain(privateText);
});

test("signed-out observations reach native policy without recording local alerts", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  const first = await terminal(page);
  await emit(page, first.sessionId);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (c: any) => c.command === "notify_agent",
          ).length,
      ),
    )
    .toBe(1);
  expect(await notices(page)).toHaveLength(0);
  expect(
    await page.evaluate(
      async () =>
        (await (window as any).__TAURI_INTERNALS__.invoke("load_notifications"))
          .items,
    ),
  ).toHaveLength(0);
});
