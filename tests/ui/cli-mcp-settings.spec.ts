import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { cliNames } from "../../src/cli-agents";

const clients = [
  {
    cli: "codex",
    name: "Codex",
    configured: false,
    path: "/home/test/.codex/config.toml",
    revision: "codex-revision",
    error: null,
  },
  {
    cli: "claude",
    name: "Claude Code",
    configured: false,
    path: "/home/test/.claude.json",
    revision: "claude-revision",
    error: null,
  },
  {
    cli: "cursor",
    name: "Cursor CLI",
    configured: true,
    path: "/home/test/.cursor/mcp.json",
    revision: "cursor-revision",
    error: null,
  },
  {
    cli: "agy",
    name: "Antigravity CLI",
    configured: false,
    path: "/home/test/.gemini/config/mcp_config.json",
    revision: "agy-revision",
    error: null,
  },
];

type ClientFixture = {
  cli: keyof typeof cliNames;
  name: string;
  configured: boolean;
  path: string;
  revision: string;
  error: string | null;
  manualReason?: string | null;
};

async function setup(
  page: Page,
  fixture: ClientFixture[] = clients,
  options: { installDelayMs?: number } = {},
) {
  await mockDesktop(page, false);
  await page.addInitScript(
    ({ clients, errors, installDelayMs }) => {
      const desktop = window as any;
      const invoke = desktop.__TAURI_INTERNALS__.invoke;
      desktop.__TAURI_INTERNALS__.invoke = async (
        command: string,
        args: Record<string, any> = {},
      ) => {
        if (command === "agent_control_state")
          desktop.__nativeTest.agentControlState = {
            supported: true,
            helperPath: null,
            broker: null,
          };
        if (command === "agent_control_startup_state")
          desktop.__nativeTest.agentControlStartup = {
            supported: true,
            autoStart: false,
            error: null,
          };
        if (
          command === "inspect_mcp_clients" &&
          !desktop.__nativeTest.mcpFixtureReady
        ) {
          desktop.__nativeTest.mcpClients = clients;
          desktop.__nativeTest.mcpInstallErrors = errors;
          desktop.__nativeTest.mcpFixtureReady = true;
        }
        if (command === "install_mcp_client" && installDelayMs)
          await new Promise((resolve) => setTimeout(resolve, installDelayMs));
        return invoke(command, args);
      };
    },
    {
      clients: fixture,
      errors: { claude: "Permission denied" },
      installDelayMs: options.installDelayMs ?? 0,
    },
  );
  await page.goto("/?window=settings&page=agent-control");
  await expect(
    page.getByRole("heading", { name: "Agent control" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Choose a client" }),
  ).toBeVisible();
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

async function setAppearance(page: Page, appearance: "light" | "dark") {
  await page.emulateMedia({ colorScheme: appearance });
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    appearance,
  );
}

function clientList(page: Page) {
  return page.getByRole("list", { name: "Coding agents", exact: true });
}

function clientButton(page: Page, name: string) {
  return clientList(page).getByRole("button", { name, exact: true });
}

async function chooseClient(page: Page, name: string) {
  await clientButton(page, name).click();
  await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
  await expect(clientList(page)).toHaveCount(0);
}

async function backToAgents(page: Page) {
  await page.getByRole("button", { name: "Back to agents" }).click();
  await expect(
    page.getByRole("heading", { name: "Choose a client", exact: true }),
  ).toBeVisible();
  await expect(clientList(page)).toBeVisible();
}

async function clientIconUrls(page: Page) {
  const assets = await page
    .locator(".agent-client-list .cli-agent-icon")
    .evaluateAll((icons) =>
      icons.map((icon) => {
        const image = icon.querySelector("img");
        if (image) return image.currentSrc;
        const mask = icon.querySelector<HTMLElement>(".cli-agent-icon-mask");
        const source = mask?.style.getPropertyValue("--cli-agent-icon") ?? "";
        const match = source.match(/url\(["']?(.+?)["']?\)/);
        return match ? new URL(match[1], location.href).href : null;
      }),
    );
  return assets.filter((asset): asset is string => typeof asset === "string");
}

test("installs per client and reports partial results for bulk installation", async ({
  page,
}) => {
  await setup(page, clients, { installDelayMs: 350 });
  const codingAgents = clientList(page);
  const search = page.getByLabel("Search agents", { exact: true });
  await expect(codingAgents).toBeVisible();
  await expect(codingAgents.getByRole("button")).toHaveCount(4);

  await search.fill("Cursor");
  await expect(clientButton(page, "Cursor CLI")).toBeVisible();
  await expect(codingAgents.getByRole("button")).toHaveCount(1);
  await expect(
    page.getByRole("heading", { name: "Choose a client", exact: true }),
  ).toBeVisible();
  expect(await calls(page, "install_mcp_client")).toHaveLength(0);
  await search.fill("agent-with-no-match");
  await expect(
    page.getByText("No agents match “agent-with-no-match”."),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Clear search" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Choose a client", exact: true }),
  ).toBeVisible();
  expect(await calls(page, "install_mcp_client")).toHaveLength(0);
  await page.getByRole("button", { name: "Clear search" }).click();
  await expect(codingAgents.getByRole("button")).toHaveCount(4);

  await search.fill("Codex");
  const codex = clientButton(page, "Codex");
  await codex.focus();
  await expect(codex).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("heading", { name: "Codex", exact: true }),
  ).toBeVisible();
  await expect(codingAgents).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Install Lomi MCP for Codex" }),
  ).toBeVisible();
  expect(await calls(page, "install_mcp_client")).toHaveLength(0);
  const back = page.getByRole("button", { name: "Back to agents" });
  await back.click();
  await expect(search).toHaveValue("Codex");
  await expect(clientButton(page, "Codex")).toBeFocused();
  expect(await calls(page, "install_mcp_client")).toHaveLength(0);
  await search.fill("");
  await expect(codingAgents.getByRole("button")).toHaveCount(4);

  await chooseClient(page, "Codex");
  await page
    .getByRole("button", { name: "Install Lomi MCP for Codex" })
    .click();
  await expect(back).toBeDisabled();
  await expect(codingAgents).toHaveCount(0);
  await expect(clientButton(page, "Claude Code")).toHaveCount(0);
  await expect(
    page.getByText("Lomi MCP was added to Codex", { exact: false }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Install Lomi MCP for Codex" }),
  ).toHaveCount(0);
  expect(await calls(page, "install_mcp_client")).toEqual([
    {
      command: "install_mcp_client",
      args: {
        cli: "codex",
        path: "/home/test/.codex/config.toml",
        revision: "codex-revision",
      },
    },
  ]);

  await backToAgents(page);
  await page
    .locator("details.agent-control-details > summary")
    .filter({ hasText: "All client configurations" })
    .click();
  await page
    .getByRole("button", { name: "Set up all eligible clients (2)" })
    .click();
  await expect(
    page.getByText("Lomi MCP was added to Antigravity CLI", {
      exact: false,
    }),
  ).toBeVisible();
  await expect(page.getByRole("alert")).toContainText(
    "Claude Code: Permission denied",
  );
  const installs = await calls(page, "install_mcp_client");
  expect(installs.map((call: any) => call.args.cli)).toEqual([
    "codex",
    "claude",
    "agy",
  ]);
  expect(installs[1].args).toEqual({
    cli: "claude",
    path: "/home/test/.claude.json",
    revision: "claude-revision",
  });
  expect(installs[2].args).toEqual({
    cli: "agy",
    path: "/home/test/.gemini/config/mcp_config.json",
    revision: "agy-revision",
  });
  await chooseClient(page, "Claude Code");
  await expect(
    page.getByRole("button", { name: "Install Lomi MCP for Claude Code" }),
  ).toBeVisible();
  await backToAgents(page);
  await chooseClient(page, "Antigravity CLI");
  await expect(
    page.getByRole("button", {
      name: "Install Lomi MCP for Antigravity CLI",
    }),
  ).toHaveCount(0);
});

test("shows the selected catalog and installs only pending clients", async ({
  page,
}, testInfo) => {
  const iconifyRequests: string[] = [];
  page.on("request", (request) => {
    if (/iconify/i.test(request.url())) iconifyRequests.push(request.url());
  });
  const catalog = [
    "claude",
    "codex",
    "gemini",
    "copilot",
    "cursor",
    "opencode",
    "openclaw",
    "hermes",
    "kilo",
    "qwen",
    "kiro",
    "vibe",
    "kimi",
    "grok",
    "agy",
  ] as const;
  const fixture = catalog.map((cli) => ({
    cli,
    name: cliNames[cli],
    configured: !["gemini", "kilo", "claude", "cursor"].includes(cli),
    path: `/home/test/${cli}/settings.json`,
    revision: `${cli}-revision`,
    error: cli === "claude" ? "Permission denied" : null,
    manualReason:
      cli === "cursor"
        ? "This configuration is managed by your organization."
        : null,
  }));
  await setup(page, fixture);
  const codingAgents = clientList(page);
  await expect(codingAgents.getByRole("button")).toHaveCount(15);
  const search = page.getByLabel("Search agents", { exact: true });
  const iconUrls = await clientIconUrls(page);
  expect(iconUrls).toHaveLength(15);
  expect(new Set(iconUrls).size).toBe(15);
  const iconResponses = await page.evaluate(
    async (urls) =>
      Promise.all(
        urls.map(async (url) => {
          const response = await fetch(url, { cache: "no-store" });
          return {
            ok: response.ok,
            contentType: response.headers.get("content-type"),
          };
        }),
      ),
    iconUrls,
  );
  expect(iconResponses).toHaveLength(15);
  expect(iconResponses.every((response) => response.ok)).toBe(true);
  expect(
    iconResponses.every((response) =>
      /image\/svg\+xml/.test(response.contentType ?? ""),
    ),
  ).toBe(true);
  expect(iconifyRequests).toEqual([]);

  await page.setViewportSize({ width: 920, height: 680 });
  for (const appearance of ["light", "dark"] as const) {
    await setAppearance(page, appearance);
    await page.screenshot({
      path: testInfo.outputPath(
        `agent-control-cli-chooser-920x680-${appearance}.png`,
      ),
    });
    await codingAgents.screenshot({
      path: testInfo.outputPath(
        `agent-control-cli-list-all-15-${appearance}.png`,
      ),
    });
    await chooseClient(page, cliNames.gemini);
    await expect(
      page.getByRole("button", {
        name: `Install Lomi MCP for ${cliNames.gemini}`,
      }),
    ).toBeVisible();
    expect(await calls(page, "install_mcp_client")).toHaveLength(0);
    await page.screenshot({
      path: testInfo.outputPath(
        `agent-control-cli-setup-gemini-920x680-${appearance}.png`,
      ),
    });
    await backToAgents(page);
  }

  await page.setViewportSize({ width: 560, height: 420 });
  await setAppearance(page, "dark");
  await page.screenshot({
    path: testInfo.outputPath("agent-control-cli-chooser-560x420-dark.png"),
  });
  const chooserOverflow = await page.evaluate(
    () =>
      document.documentElement.scrollWidth -
      document.documentElement.clientWidth,
  );
  expect(chooserOverflow).toBeLessThanOrEqual(0);
  await chooseClient(page, cliNames.gemini);
  const setupOverflow = await page.evaluate(
    () =>
      document.documentElement.scrollWidth -
      document.documentElement.clientWidth,
  );
  expect(setupOverflow).toBeLessThanOrEqual(0);
  await page.screenshot({
    path: testInfo.outputPath(
      "agent-control-cli-setup-gemini-560x420-dark.png",
    ),
  });
  await backToAgents(page);

  await chooseClient(page, "Claude Code");
  await expect(page.getByRole("alert")).toContainText("Permission denied");
  await expect(
    page.getByRole("button", { name: "Install Lomi MCP for Claude Code" }),
  ).toBeDisabled();
  await backToAgents(page);

  await chooseClient(page, "Cursor CLI");
  await expect(
    page.getByText(/Open Preferences → Manual configuration/),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Install Lomi MCP for Cursor CLI" }),
  ).toHaveCount(0);
  expect(await calls(page, "install_mcp_client")).toHaveLength(0);
  await backToAgents(page);

  await search.fill("no-client-matches-this-query");
  await expect(codingAgents.getByRole("button")).toHaveCount(0);
  await expect(
    page.getByText("No agents match “no-client-matches-this-query”."),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Choose a client", exact: true }),
  ).toBeVisible();
  expect(await calls(page, "install_mcp_client")).toHaveLength(0);
  await page.getByRole("button", { name: "Clear search" }).click();
  await expect(codingAgents.getByRole("button")).toHaveCount(15);

  await page
    .locator("details.agent-control-details > summary")
    .filter({ hasText: "All client configurations" })
    .click();
  const rows = page.locator(".agent-setup-client-list > li");
  await expect(rows).toHaveCount(15);
  await expect(rows.filter({ hasText: "Claude Code" })).toContainText(
    "Setup unavailable",
  );
  await expect(rows.filter({ hasText: "Cursor CLI" })).toContainText(
    "Manual setup",
  );
  expect(await calls(page, "install_mcp_client")).toHaveLength(0);
  await page.setViewportSize({ width: 900, height: 700 });
  await rows.filter({ hasText: "Hermes Agent" }).scrollIntoViewIfNeeded();
  await page.screenshot({
    path: testInfo.outputPath("selected-cli-catalog.png"),
  });
  await page
    .getByRole("button", { name: "Set up all eligible clients (2)" })
    .click();
  await expect(
    page.getByText("Lomi MCP was added to Gemini CLI, Kilo Code CLI", {
      exact: false,
    }),
  ).toBeVisible();
  const installed = await calls(page, "install_mcp_client");
  expect(installed.map((call: any) => call.args.cli)).toEqual([
    "gemini",
    "kilo",
  ]);
  await expect(
    page.getByRole("button", { name: "Set up all eligible clients (0)" }),
  ).toBeDisabled();
});
