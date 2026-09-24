import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function supportAutomaticStart(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    const desktop = window as any;
    const saved = localStorage.getItem("test-agent-control-startup");
    desktop.__nativeTest.agentControlStartup = saved
      ? JSON.parse(saved)
      : { supported: true, autoStart: null, error: null };
  });
}

test("first-run acceptance saves the startup choice", async ({ page }) => {
  await mockDesktop(page, false);
  await supportAutomaticStart(page);
  await page.goto("/");

  const prompt = page.getByRole("dialog", {
    name: "Start the MCP server automatically?",
  });
  await expect(prompt).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Keep disabled", exact: true }),
  ).toBeFocused();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: { command: string }) =>
              call.command === "agent_control_enable",
          ).length,
      ),
    )
    .toBe(0);
  await page.screenshot({
    path: "test-results/agent-control-startup-consent.png",
  });
  await page.evaluate(() => {
    (window as any).__nativeTest.agentControlStartupStartError =
      "The server port is unavailable.";
  });

  await page
    .getByRole("button", { name: "Enable automatic start", exact: true })
    .click();
  await expect(prompt).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.agentControlStartupCalls,
      ),
    )
    .toEqual([{ enabled: true }]);
  await expect(page.getByRole("alert")).toContainText(
    "The saved choice is still enabled",
  );

  await page.reload();
  await expect(
    page.getByRole("dialog", {
      name: "Start the MCP server automatically?",
    }),
  ).toHaveCount(0);
  await expect(page.getByRole("alert")).toContainText(
    "Automatic MCP startup is enabled",
  );
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.agentControlStartup.autoStart,
      ),
    )
    .toBe(true);
});

test("first-run refusal is saved and does not prompt again", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await supportAutomaticStart(page);
  await page.goto("/");

  await page
    .getByRole("button", { name: "Keep disabled", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", {
      name: "Start the MCP server automatically?",
    }),
  ).toHaveCount(0);
  await page.reload();
  await expect(
    page.getByRole("dialog", {
      name: "Start the MCP server automatically?",
    }),
  ).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.agentControlStartup.autoStart,
      ),
    )
    .toBe(false);
});

test("Escape defers the choice without granting or saving consent", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await supportAutomaticStart(page);
  await page.goto("/");
  await expect(
    page.getByRole("dialog", {
      name: "Start the MCP server automatically?",
    }),
  ).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("dialog", {
      name: "Start the MCP server automatically?",
    }),
  ).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.agentControlStartupCalls,
      ),
    )
    .toEqual([]);
  await page.reload();
  await expect(
    page.getByRole("dialog", {
      name: "Start the MCP server automatically?",
    }),
  ).toBeVisible();
});

test("startup save errors stay in the dialog and can be retried", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await supportAutomaticStart(page);
  await page.goto("/");
  await page.evaluate(() => {
    (window as any).__nativeTest.failAgentControlStartupSave = true;
    (window as any).__nativeTest.agentControlStartupSaveDelay = 250;
  });

  await page
    .getByRole("button", { name: "Enable automatic start", exact: true })
    .click();
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(page.getByRole("alert")).toContainText("Disk is full");
  await expect(
    page.getByRole("dialog", {
      name: "Start the MCP server automatically?",
    }),
  ).toBeVisible();

  await page.evaluate(() => {
    (window as any).__nativeTest.failAgentControlStartupSave = false;
    (window as any).__nativeTest.agentControlStartupSaveDelay = 0;
  });
  await page
    .getByRole("button", { name: "Enable automatic start", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", {
      name: "Start the MCP server automatically?",
    }),
  ).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.agentControlStartup.autoStart,
      ),
    )
    .toBe(true);
});

test("Settings changes future startup only and refreshes current control state on events", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.addInitScript(() => {
    const mock = (window as any).__nativeTest;
    mock.agentControlStartup = {
      supported: true,
      autoStart: false,
      error: null,
    };
    mock.agentControlState.supported = true;
  });
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Preferences", exact: true }).click();

  const automaticStart = page.getByRole("switch", {
    name: /Start MCP server when Lomi opens/,
  });
  await expect(automaticStart).toBeEnabled();
  await expect(automaticStart).not.toBeChecked();
  await page.screenshot({
    path: "test-results/agent-control-startup-setting.png",
  });
  await automaticStart.check();
  await expect(automaticStart).toBeChecked();
  await expect(page.locator(".agent-control-badge")).toHaveText("Off");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: { command: string }) =>
              call.command === "agent_control_enable",
          ).length,
      ),
    )
    .toBe(0);

  await page.evaluate(async () => {
    const desktop = (window as any).__nativeTest;
    desktop.agentControlState = {
      supported: true,
      helperPath: null,
      broker: {
        endpoint: {
          instanceId: "fixture",
          endpoint: "fixture",
          brokerSha256: "fixture",
          ipcVersion: 1,
        },
        uiReady: true,
        terminalProfile: null,
        workspaces: [],
        pending: [],
        pendingProjectOpens: [],
        pendingSettingsUpdates: [],
        pendingInstalls: [],
        pendingControls: [],
        sessions: [],
      },
    };
    desktop.agentControlStartup = {
      supported: true,
      autoStart: true,
      error: null,
    };
    await desktop.emitEvent(
      "agent-control-startup-changed",
      desktop.agentControlStartup,
    );
  });
  await expect(automaticStart).toBeChecked();
  await expect(page.locator(".agent-control-badge")).toHaveText("Ready");
});

test("Settings save errors survive a focus refresh", async ({ page }) => {
  await mockDesktop(page, false, null);
  await page.addInitScript(() => {
    const mock = (window as any).__nativeTest;
    mock.agentControlStartup = {
      supported: true,
      autoStart: false,
      error: null,
    };
    mock.agentControlState.supported = true;
  });
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Preferences", exact: true }).click();
  const automaticStart = page.getByRole("switch", {
    name: /Start MCP server when Lomi opens/,
  });
  await expect(automaticStart).toBeEnabled();
  await page.evaluate(() => {
    (window as any).__nativeTest.failAgentControlStartupSettingSave = true;
    (window as any).__nativeTest.agentControlStartupSettingSaveDelay = 250;
  });

  await automaticStart.click();
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(page.getByRole("alert")).toContainText(
    "Settings are unavailable",
  );
  await expect(automaticStart).not.toBeChecked();
});
