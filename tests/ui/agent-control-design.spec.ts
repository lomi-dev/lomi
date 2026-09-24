import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

type PendingRequest = {
  id: string;
  clientLabel: string;
  certificateSha256: string;
  secondsRemaining: number;
};

function broker(pending: PendingRequest[]) {
  return {
    endpoint: {
      instanceId: "fixture",
      endpoint: "fixture",
      brokerSha256: "fixture",
      ipcVersion: 1,
    },
    uiReady: true,
    terminalProfile: null,
    terminalProfiles: [],
    workspaces: [
      {
        id: "workspace",
        projectId: "project",
        name: "Fixture workspace",
        projectName: "Project",
        projectPath: "/project",
      },
    ],
    pending,
    pendingControls: [],
    pendingInstalls: [],
    pendingProjectOpens: [],
    pendingSettingsUpdates: [],
    pendingAndroidManagement: [],
    pendingBrowserUploads: [],
    sessions: [],
  };
}

async function setup(
  page: Page,
  initialBroker: ReturnType<typeof broker> | null,
) {
  await mockDesktop(page, false, null);
  await page.addInitScript((initialBroker) => {
    const desktop = window as any;
    desktop.__nativeTest.agentControlState = {
      supported: true,
      helperPath: null,
      broker: initialBroker,
    };
    desktop.__nativeTest.agentControlStartup = {
      supported: true,
      autoStart: false,
      yoloMode: false,
      error: null,
    };
    desktop.__nativeTest.mcpClients = [
      {
        cli: "codex",
        name: "Codex",
        configured: false,
        path: "/home/test/.codex/config.toml",
        revision: "codex-revision",
        error: null,
      },
    ];
  }, initialBroker);
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

async function openPermissionGroup(page: Page, name: string) {
  const summary = page
    .locator("details.agent-permission-group > summary")
    .filter({ hasText: name })
    .first();
  const details = summary.locator("xpath=..");
  await expect(summary).toBeVisible();
  if (
    !(await details.evaluate((element) => (element as HTMLDetailsElement).open))
  )
    await summary.click();
  await expect
    .poll(() =>
      details.evaluate((element) => (element as HTMLDetailsElement).open),
    )
    .toBe(true);
}

test("Connect setup is the default and fits the native Settings window sizes", async ({
  page,
}, testInfo) => {
  await setup(page, null);
  await page.goto("/?window=settings&page=agent-control");

  const connectTab = page.getByRole("tab", {
    name: "Connect an agent",
    exact: true,
  });
  await expect(connectTab).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("tabpanel", { name: "Connect an agent" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Choose a client", exact: true }),
  ).toBeVisible();
  const codingAgents = clientList(page);
  await expect(codingAgents).toBeVisible();
  await expect(codingAgents.getByRole("button")).toHaveCount(1);
  await expect(page.getByLabel("Search agents", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Install Lomi MCP for Codex" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Start server" }),
  ).toBeVisible();

  await page.setViewportSize({ width: 920, height: 680 });
  await setAppearance(page, "light");
  await page.screenshot({
    path: testInfo.outputPath("agent-control-design-920x680-light.png"),
  });
  await codingAgents
    .getByRole("button", { name: "Codex", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Codex", exact: true }),
  ).toBeVisible();
  await expect(codingAgents).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Install Lomi MCP for Codex" }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("agent-control-design-setup-920x680-light.png"),
  });
  await page.getByRole("button", { name: "Back to agents" }).click();
  await expect(codingAgents).toBeVisible();

  await page.setViewportSize({ width: 1440, height: 900 });
  await setAppearance(page, "dark");
  await page.screenshot({
    path: testInfo.outputPath("agent-control-design-1440x900-dark.png"),
  });

  await page.getByRole("tab", { name: "Preferences", exact: true }).click();
  await expect(
    page.getByRole("switch", { name: /Start MCP server when Lomi opens/ }),
  ).toBeVisible();
  await expect(page.getByRole("switch", { name: "YOLO mode" })).toBeVisible();
  for (const label of ["Manual configuration", "File recovery"]) {
    const summary = page
      .locator("details.agent-control-details > summary")
      .filter({ hasText: label });
    await expect(summary).toBeVisible();
    await expect
      .poll(() =>
        summary
          .locator("xpath=..")
          .evaluate((element) => (element as HTMLDetailsElement).open),
      )
      .toBe(false);
  }

  await page
    .getByRole("tab", { name: "Connect an agent", exact: true })
    .click();
  await page.setViewportSize({ width: 560, height: 420 });
  await setAppearance(page, "dark");
  await expect(connectTab).toHaveAttribute("aria-selected", "true");
  const search = page.getByLabel("Search agents", { exact: true });
  await search.scrollIntoViewIfNeeded();
  await expect(search).toBeInViewport();
  await expect(codingAgents).toBeInViewport();
  const horizontalOverflow = await page.evaluate(
    () =>
      document.documentElement.scrollWidth -
      document.documentElement.clientWidth,
  );
  expect(horizontalOverflow).toBeLessThanOrEqual(0);
  await page.screenshot({
    path: testInfo.outputPath("agent-control-design-560x420-dark.png"),
  });
  await codingAgents
    .getByRole("button", { name: "Codex", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Install Lomi MCP for Codex" }),
  ).toBeVisible();
  const setupOverflow = await page.evaluate(
    () =>
      document.documentElement.scrollWidth -
      document.documentElement.clientWidth,
  );
  expect(setupOverflow).toBeLessThanOrEqual(0);
  await page.screenshot({
    path: testInfo.outputPath("agent-control-design-setup-560x420-dark.png"),
  });
});

test("pending requests select Sessions once and later requests show a review banner", async ({
  page,
}, testInfo) => {
  const first = {
    id: "request-one",
    clientLabel: "First client",
    certificateSha256: "first-certificate",
    secondsRemaining: 120,
  };
  await setup(page, broker([first]));
  await page.goto("/?window=settings&page=agent-control");

  const sessionsTab = page.getByRole("tab", { name: /^Sessions/ });
  await expect(sessionsTab).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("heading", { name: "New session requests" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "First client" }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("agent-control-design-pending-sessions.png"),
  });

  await page
    .getByRole("tab", { name: "Connect an agent", exact: true })
    .click();
  const second = {
    id: "request-two",
    clientLabel: "Second client",
    certificateSha256: "second-certificate",
    secondsRemaining: 120,
  };
  await page.evaluate((second) => {
    const mock = (window as any).__nativeTest;
    mock.agentControlState = {
      ...mock.agentControlState,
      broker: {
        ...mock.agentControlState.broker,
        pending: [...mock.agentControlState.broker.pending, second],
      },
    };
  }, second);
  await expect(
    page.locator('.agent-control-notice[role="status"]'),
  ).toContainText("2 requests need review.");
  await page.getByRole("button", { name: "Review requests" }).click();
  await expect(sessionsTab).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("heading", { name: "Second client" }),
  ).toBeVisible();
});

test("keyboard tab changes retain Chat permission dependencies and submitted scope", async ({
  page,
}) => {
  const first = {
    id: "request-one",
    clientLabel: "Chat client",
    certificateSha256: "chat-certificate",
    secondsRemaining: 120,
  };
  await setup(page, broker([first]));
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__chatDesign = { approval: null };
    desktop.__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: Record<string, unknown>,
    ) => {
      if (command === "agent_control_chat_catalog")
        return {
          items: [{ conversationId: "conversation-one", title: "Shared chat" }],
          next: null,
        };
      if (command === "agent_control_approve") {
        desktop.__chatDesign.approval = args;
        return;
      }
      return invoke(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");

  const sessionsTab = page.getByRole("tab", { name: /^Sessions/ });
  const connectTab = page.getByRole("tab", {
    name: "Connect an agent",
    exact: true,
  });
  await sessionsTab.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(connectTab).toBeFocused();
  await expect(connectTab).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ArrowRight");
  await expect(sessionsTab).toBeFocused();
  await expect(sessionsTab).toHaveAttribute("aria-selected", "true");

  await page.getByLabel("Workspace", { exact: true }).selectOption("workspace");
  await openPermissionGroup(page, "Chat AI");
  const read = page.getByLabel("Allow reading selected Chat AI conversations");
  const open = page.getByLabel("Allow opening selected Chat AI conversations");
  const create = page.getByLabel("Allow creating Chat AI conversations");
  await expect(open).toBeDisabled();
  await expect(create).toBeDisabled();
  await read.check();
  await page.getByLabel("Shared chat conversation-one").check();
  await open.check();
  await expect(create).toBeEnabled();
  await create.check();

  await page.getByRole("tab", { name: "Preferences", exact: true }).click();
  await expect(
    page.getByRole("tabpanel", { name: "Preferences" }),
  ).toBeVisible();
  await page.getByRole("tab", { name: "Preferences", exact: true }).focus();
  await page.keyboard.press("ArrowLeft");
  await expect(sessionsTab).toHaveAttribute("aria-selected", "true");
  await expect(read).toBeChecked();
  await expect(open).toBeChecked();
  await expect(create).toBeChecked();
  await expect(page.getByLabel("Shared chat conversation-one")).toBeChecked();

  await page
    .getByRole("button", { name: "Approve session", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__chatDesign.approval))
    .toMatchObject({
      requestId: "request-one",
      workspaceIds: ["workspace"],
      scopes: [
        "workspace.read",
        "chat.read",
        "chat.open",
        "panel.create",
        "panel.focus",
        "chat.create",
      ],
      chatConversations: ["conversation-one"],
    });
});
