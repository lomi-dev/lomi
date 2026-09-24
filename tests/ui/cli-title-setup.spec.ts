import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import type { CliAgent } from "../../src/cli-agents";

const cliPath = "/home/user/.codex/config.toml";

function status(cli: "codex" | "claude" = "codex", revision = "original") {
  const directory =
    cli === "codex" ? "/home/user/.codex" : "/home/user/.claude";
  return {
    cli,
    features: (["notifications", "mcp", "titlebar"] as const).map(
      (feature) => ({
        feature,
        configured: false,
        path:
          feature === "notifications"
            ? `${directory}/settings.json`
            : feature === "mcp"
              ? `${directory}/mcp.json`
              : cli === "codex"
                ? cliPath
                : `${directory}/settings.json`,
        revision,
        error: null,
      }),
    ),
  };
}

async function setup(page: Page) {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.sessions.size))
    .toBeGreaterThan(0);
  await page.evaluate((codex) => {
    (window as any).__nativeTest.cliIntegrationStatuses.codex = codex;
  }, status());
}

async function detect(page: Page, cli: CliAgent = "codex", pid = 123) {
  const previousPolls = await calls(page, "terminal_contexts").then(
    (items) => items.length,
  );
  await page.evaluate(
    ({ cli, pid }) => {
      const state = (window as any).__nativeTest;
      for (const id of state.sessions.keys())
        state.terminalContexts[id] = {
          cwd: "/project",
          foregroundProgram: cli,
          titleCli: { cli, pid },
        };
    },
    { cli, pid },
  );
  await expect
    .poll(() => calls(page, "terminal_contexts").then((items) => items.length))
    .toBeGreaterThan(previousPolls);
  await expect
    .poll(() => calls(page, "inspect_cli_integrations"))
    .not.toHaveLength(0);
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

test("offers three Codex integrations in the status bar and saves only after a click", async ({
  page,
}, testInfo) => {
  await setup(page);
  await detect(page);

  const group = page.getByRole("group", { name: "Codex integrations" });
  await expect(group).toBeVisible();
  await expect(
    group.getByRole("button", { name: "Enable Codex notifications" }),
  ).toBeVisible();
  await expect(
    group.getByRole("button", { name: "Enable Codex Lomi MCP" }),
  ).toBeVisible();
  await expect(
    group.getByRole("button", { name: "Enable Codex titlebar" }),
  ).toBeVisible();
  await expect(group.getByRole("button")).toHaveCount(6);
  await expect(
    group.getByRole("button", { name: "Enable Codex titlebar" }),
  ).toHaveAttribute("title", expect.stringContaining(cliPath));
  expect(await calls(page, "enable_cli_integration")).toHaveLength(0);
  expect(await calls(page, "dismiss_cli_integrations")).toHaveLength(0);

  await page.emulateMedia({ colorScheme: "dark" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({
    path: testInfo.outputPath("cli-statusbar-wide.png"),
  });
  await page.setViewportSize({ width: 800, height: 420 });
  await expect(group).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("cli-statusbar-small.png"),
  });

  const starts = await calls(page, "start_terminal");
  await group.getByRole("button", { name: "Enable Codex titlebar" }).click();
  await expect(
    group.getByRole("button", { name: "Enable Codex titlebar" }),
  ).toHaveCount(0);
  await expect(
    group.getByRole("button", { name: "Enable Codex notifications" }),
  ).toBeVisible();
  await expect(
    group.getByRole("button", { name: "Enable Codex Lomi MCP" }),
  ).toBeVisible();
  const enabled = await calls(page, "enable_cli_integration");
  expect(enabled).toHaveLength(1);
  expect(enabled[0].args).toEqual({
    id: expect.any(String),
    process: { cli: "codex", pid: 123 },
    feature: "titlebar",
    path: cliPath,
    revision: "original",
  });
  expect(await calls(page, "start_terminal")).toHaveLength(starts.length);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  expect(await calls(page, "write_terminal")).toHaveLength(0);
});

test("dismisses each feature independently across tabs and CLI processes", async ({
  page,
}) => {
  await setup(page);
  await detect(page);
  const group = page.getByRole("group", { name: "Codex integrations" });
  const mcp = group.getByRole("button", { name: "Enable Codex Lomi MCP" });
  const notifications = group.getByRole("button", {
    name: "Enable Codex notifications",
  });
  const titlebar = group.getByRole("button", { name: "Enable Codex titlebar" });
  const closeMcp = group.getByRole("button", {
    name: "Dismiss Codex Lomi MCP suggestion until Lomi restarts",
  });
  await closeMcp.focus();
  await page.keyboard.press("Enter");
  await expect(mcp).toHaveCount(0);
  await expect(notifications).toBeVisible();
  await expect(titlebar).toBeVisible();
  await expect(group.getByRole("button")).toHaveCount(4);
  expect(await calls(page, "dismiss_cli_integrations")).toEqual([
    {
      command: "dismiss_cli_integrations",
      args: { cli: "codex", feature: "mcp" },
    },
  ]);
  expect(await calls(page, "enable_cli_integration")).toHaveLength(0);

  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await detect(page, "codex", 789);
  await expect(mcp).toHaveCount(0);
  await expect(notifications).toBeVisible();
  await expect(titlebar).toBeVisible();

  await page.evaluate((claude) => {
    (window as any).__nativeTest.cliIntegrationStatuses.claude = claude;
  }, status("claude"));
  await detect(page, "claude", 456);
  await expect(
    page.getByRole("button", { name: "Enable Claude Code Lomi MCP" }),
  ).toBeVisible();
  await detect(page, "codex", 999);
  await expect(mcp).toHaveCount(0);

  await group
    .getByRole("button", {
      name: "Dismiss Codex notifications suggestion until Lomi restarts",
    })
    .click();
  await expect(notifications).toHaveCount(0);
  await expect(titlebar).toBeVisible();
  await group
    .getByRole("button", {
      name: "Dismiss Codex titlebar suggestion until Lomi restarts",
    })
    .click();
  await expect(group).toHaveCount(0);
  expect(await calls(page, "enable_cli_integration")).toHaveLength(0);
  expect(
    (await calls(page, "dismiss_cli_integrations")).map(
      (call: any) => call.args.feature,
    ),
  ).toEqual(["mcp", "notifications", "titlebar"]);
});

test("refreshes the offer when the detected CLI changes", async ({ page }) => {
  await setup(page);
  await page.evaluate(
    (claude) => {
      (window as any).__nativeTest.cliIntegrationStatuses.claude = claude;
    },
    status("claude", "claude-revision"),
  );
  await detect(page);
  await expect(
    page.getByRole("group", { name: "Codex integrations" }),
  ).toBeVisible();

  const previousInspections = (await calls(page, "inspect_cli_integrations"))
    .length;
  await page.evaluate(() => {
    const state = (window as any).__nativeTest;
    for (const id of state.sessions.keys())
      state.terminalContexts[id] = {
        cwd: "/project",
        foregroundProgram: "claude",
        titleCli: { cli: "claude", pid: 456 },
      };
  });
  await expect
    .poll(() =>
      calls(page, "inspect_cli_integrations").then((items) => items.length),
    )
    .toBeGreaterThan(previousInspections);
  await expect(
    page.getByRole("group", { name: "Claude Code integrations" }),
  ).toBeVisible();
  await expect(
    page.getByRole("group", { name: "Codex integrations" }),
  ).toHaveCount(0);
});

test("requires a fresh click after a configuration conflict refreshes the revision", async ({
  page,
}) => {
  await setup(page);
  await detect(page);
  const group = page.getByRole("group", { name: "Codex integrations" });
  await page.evaluate(() => {
    const state = (window as any).__nativeTest;
    state.cliIntegrationError = "Codex configuration changed. Review it again.";
    const titlebar = state.cliIntegrationStatuses.codex.features.find(
      (feature: any) => feature.feature === "titlebar",
    );
    titlebar.revision = "external-edit";
  });

  await group.getByRole("button", { name: "Enable Codex titlebar" }).click();
  await expect(page.getByRole("alert")).toContainText("configuration changed");
  await expect(
    group.getByRole("button", { name: "Enable Codex titlebar" }),
  ).toBeVisible();
  const failedAttempt = await calls(page, "enable_cli_integration");
  expect(failedAttempt).toHaveLength(1);
  expect(failedAttempt[0].args.revision).toBe("original");

  await page.evaluate(() => {
    (window as any).__nativeTest.cliIntegrationError = "";
  });
  await expect
    .poll(() => calls(page, "inspect_cli_integrations"))
    .toHaveLength(2);
  expect(await calls(page, "enable_cli_integration")).toHaveLength(1);
  await group.getByRole("button", { name: "Enable Codex titlebar" }).click();
  await expect(
    group.getByRole("button", { name: "Enable Codex titlebar" }),
  ).toHaveCount(0);
  const attempts = await calls(page, "enable_cli_integration");
  expect(attempts).toHaveLength(2);
  expect(attempts[1].args.revision).toBe("external-edit");
});

test("notification permission denial does not write CLI configuration", async ({
  page,
}) => {
  await setup(page);
  await detect(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.agentNotificationPermission = false;
  });

  await page
    .getByRole("group", { name: "Codex integrations" })
    .getByRole("button", { name: "Enable Codex notifications" })
    .click();
  await expect(
    page.getByText("Notifications are blocked.", { exact: false }),
  ).toBeVisible();
  expect(
    await calls(page, "plugin:notification|request_permission"),
  ).toHaveLength(1);
  expect(await calls(page, "enable_cli_integration")).toHaveLength(0);
});

test("offers only native features for newly detected CLIs", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.cliIntegrationStatuses.kilo = {
      cli: "kilo",
      features: [
        {
          feature: "mcp",
          configured: false,
          path: "/home/test/.config/kilo/kilo.jsonc",
          revision: "kilo-revision",
          error: null,
        },
      ],
    };
  });
  await detect(page, "kilo", 456);
  const group = page.getByRole("group", { name: "Kilo Code CLI integrations" });
  await expect(
    group.getByRole("button", { name: "Enable Kilo Code CLI Lomi MCP" }),
  ).toBeVisible();
  await expect(
    group.getByRole("button", { name: /titlebar|notifications/ }),
  ).toHaveCount(0);
  expect(await calls(page, "enable_cli_integration")).toHaveLength(0);
  await group
    .getByRole("button", { name: "Enable Kilo Code CLI Lomi MCP" })
    .click();
  expect((await calls(page, "enable_cli_integration"))[0].args).toEqual({
    id: expect.any(String),
    process: { cli: "kilo", pid: 456 },
    feature: "mcp",
    path: "/home/test/.config/kilo/kilo.jsonc",
    revision: "kilo-revision",
  });
  expect(await calls(page, "write_terminal")).toHaveLength(0);
});
