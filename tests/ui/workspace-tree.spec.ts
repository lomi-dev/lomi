import { expect, test, type Page } from "@playwright/test";
import {
  addWorkspace,
  layoutPanes,
  newAgentTaskTab,
  newAndroidTab,
  newBrowserTab,
  newChatTab,
  newSession,
  newTab,
  splitPane,
  tabTitle,
  type LayoutPane,
} from "../../src/model";
import { mockDesktop } from "./desktop";
import {
  notificationInbox,
  openNotifications,
} from "./application-notification-helpers";

function workspaceSession(name = "Build", path = "/work/lomi") {
  const session = addWorkspace(newSession(), path, "local:bash", name);
  const project = session.projects.find(
    (candidate) => candidate.path === path,
  )!;
  const workspace = project.workspaces[0];
  return { session, project, workspace };
}

async function startCount(page: Page) {
  return page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "start_terminal",
      ).length,
  );
}

async function currentTerminal(page: Page) {
  const paneId = (await page
    .locator(".terminal-pane")
    .first()
    .getAttribute("data-pane-id"))!;
  const sessionId = await page.evaluate(async (id) => {
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

async function emitSignal(
  page: Page,
  sessionId: string,
  signal: "finished" | "attention",
) {
  await page.evaluate(
    ({ id, signal }) => {
      const native = (window as any).__nativeTest;
      native.emit(id, "\x1b]777;notify;Lo");
      native.emit(id, `mi;claude;${signal}\x07`);
    },
    { id: sessionId, signal },
  );
}

test("workspace panel list flattens mixed layouts without starting hidden runtimes and selects the requested pane", async ({
  page,
}) => {
  const { session, workspace } = workspaceSession();
  session.sidebar = "workspaces";
  const first = workspace.tabs[0];
  if (first.type !== "terminal") throw new Error("Expected initial terminal");
  first.title = "Active";

  const mixed = newTab("/work/lomi", "local:bash", "Mixed", 2);
  const embeddedFile = {
    type: "file" as const,
    id: "tree-readme",
    title: "README.md",
    root: "/work/lomi",
    relative: "README.md",
  };
  const plugin = {
    type: "plugin" as const,
    id: "tree-plugin",
    title: "Saved plugin",
    owner: "local.fixture",
    viewType: "local.fixture.view",
    stateVersion: 1,
    state: { draft: "retained" },
  };
  const mixedLeaves: LayoutPane[] = [
    newBrowserTab("https://example.com/"),
    embeddedFile,
    newAndroidTab("12345678-1234-4567-8123-123456789abc", "Phone"),
    newChatTab("workspace-tree-chat", "Chat"),
    newAgentTaskTab("workspace-tree-task", "CLI Task"),
    plugin,
  ];
  for (const leaf of mixedLeaves)
    mixed.layout = splitPane(
      mixed.layout,
      mixed.activePaneId,
      "horizontal",
      leaf,
    );
  const mixedPaneIds = layoutPanes(mixed.layout).map((pane) => pane.id);

  const duplicateFile = {
    type: "file" as const,
    id: "tree-readme-other",
    title: "README.md",
    root: "/work/lomi",
    relative: "README.md",
  };
  const duplicateFileTab = { ...duplicateFile };
  const duplicateFileTabId = duplicateFileTab.id;

  const split = newTab("/work/lomi", "local:bash", "Select", 2);
  const splitPanes = layoutPanes(split.layout);
  const targetPane = splitPanes[1];
  workspace.tabs.push(mixed, duplicateFileTab, split);

  const reviewSession = addWorkspace(
    session,
    "/work/review",
    "local:bash",
    "Review",
  );
  reviewSession.sidebar = "workspaces";
  const expectedOwners = [
    { paneId: first.activePaneId, tabId: first.id },
    ...mixedPaneIds.map((paneId) => ({ paneId, tabId: mixed.id })),
    { paneId: duplicateFileTabId, tabId: duplicateFileTabId },
    ...splitPanes.map((pane) => ({ paneId: pane.id, tabId: split.id })),
  ];

  await mockDesktop(page, false, reviewSession);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect.poll(() => startCount(page)).toBe(1);

  const list = page.getByRole("navigation", { name: "Workspace list" });
  const items = list.getByRole("list", {
    name: "Open items in Build",
    exact: true,
  });
  const buildEntry = list.locator(
    `.workspace-list-entry[data-workspace-id="${workspace.id}"]`,
  );
  await expect(buildEntry.locator(".workspace-list-item")).not.toHaveAttribute(
    "aria-current",
    "true",
  );
  await buildEntry
    .getByRole("button", { name: "Expand items in Build", exact: true })
    .click();
  await expect(items).toBeVisible();
  await expect
    .poll(() =>
      items.locator(".workspace-open-item").evaluateAll((items) =>
        items.map((item) => ({
          paneId: item.getAttribute("data-workspace-pane-id"),
          tabId: item.getAttribute("data-owner-tab-id"),
        })),
      ),
    )
    .toEqual(expectedOwners);
  expect(
    layoutPanes(mixed.layout)
      .map((pane) => pane.type)
      .sort(),
  ).toEqual([
    "agent-task",
    "android",
    "browser",
    "chat",
    "file",
    "plugin",
    "terminal",
    "terminal",
  ]);
  const itemLabels = await items
    .locator(".workspace-item-label")
    .allTextContents();
  expect(
    itemLabels.filter((label) => label.startsWith("Terminal")),
  ).toHaveLength(5);
  for (const label of [
    "Saved plugin",
    "CLI Task",
    "Chat",
    "Phone",
    "README.md",
    "Browser",
  ])
    expect(itemLabels).toContain(label);
  expect(itemLabels.filter((label) => label === "README.md")).toHaveLength(2);
  await expect.poll(() => startCount(page)).toBe(1);
  expect(
    await page.evaluate(() => {
      const calls = (window as any).__nativeTest.calls;
      return calls.some((call: any) =>
        [
          "read_editor_file",
          "browser_action",
          "git_commit_details",
          "git_commit_diff",
        ].includes(call.command),
      );
    }),
  ).toBe(false);
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.browsers.size))
    .toBe(0);

  const targetItem = items.locator(
    `.workspace-open-item[data-workspace-pane-id="${targetPane.id}"]`,
  );
  await targetItem.click();
  await expect(targetItem).toHaveAttribute("aria-current", "true");
  await expect(
    list.locator(`[data-workspace-id="${workspace.id}"] .workspace-list-item`),
  ).toHaveAttribute("aria-current", "true");
  await expect(page.getByRole("tab", { selected: true })).toHaveText("Select");
  await expect(
    page.locator(`.terminal-pane[data-pane-id="${targetPane.id}"]`),
  ).toBeVisible();
  await expect.poll(() => startCount(page)).toBe(3);

  const activeItem = items.locator(
    `.workspace-open-item[data-workspace-pane-id="${first.activePaneId}"]`,
  );
  await activeItem.click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await targetItem.click();
  await expect(targetItem).toHaveAttribute("aria-current", "true");
  await expect.poll(() => startCount(page)).toBe(4);
  const terminalCommands = await page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter((call: any) =>
        ["start_terminal", "close_terminal"].includes(call.command),
      )
      .map((call: any) => call.command),
  );
  expect(
    terminalCommands.filter((command) => command === "start_terminal"),
  ).toHaveLength(4);
  expect(
    terminalCommands.filter((command) => command === "close_terminal"),
  ).toHaveLength(0);
});

test("terminal notification dots restore after a hidden sidebar reload and clear on explicit inbox actions", async ({
  page,
}, testInfo) => {
  const { session, workspace } = workspaceSession("Voice");
  session.sidebar = null;
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page, false, session);
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-terminal-notification-policy",
      JSON.stringify({
        accountId: "user-a",
        available: true,
        preferences: { enabled: true, backgroundOnly: true },
      }),
    );
  });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  let terminal = await currentTerminal(page);
  await page.evaluate((id) => {
    (window as any).__nativeTest.emit(id, "\x1b]9;PRIVATE_OSC9_TEXT\x07");
  }, terminal.sessionId);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.notifications.items.length,
      ),
    )
    .toBe(1);
  await emitSignal(page, terminal.sessionId, "finished");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.notifications.items.length,
      ),
    )
    .toBe(2);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some(
        (call: any) => call.command === "notify_agent",
      ),
    ),
  ).toBe(true);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.notifications.items.map(
        (item: any) => item.paneId,
      ),
    ),
  ).toEqual([terminal.paneId, terminal.paneId]);

  const toggle = page.getByRole("button", {
    name: "Toggle workspaces",
    exact: true,
  });
  const list = page.getByRole("navigation", { name: "Workspace list" });
  const workspaceEntry = list.locator(`[data-workspace-id="${workspace.id}"]`);
  const workspaceDot = workspaceEntry
    .locator(".workspace-name-row")
    .locator(".workspace-notification-dot");
  const paneDot = workspaceEntry
    .locator(".workspace-open-item")
    .locator(".workspace-notification-dot");
  const items = list.getByRole("list", {
    name: "Open items in Voice",
    exact: true,
  });
  const revealDots = async () => {
    await list
      .getByRole("button", { name: "Expand items in Voice", exact: true })
      .click();
    await expect(items).toBeVisible();
    await expect(paneDot).toBeVisible();
  };
  await toggle.click();
  await expect(workspaceDot).toBeVisible();
  await expect(items).toBeHidden();
  await expect(paneDot).toHaveCount(0);
  await revealDots();
  await expect(workspaceDot).toHaveAttribute("role", "img");
  await expect(workspaceDot).toHaveAttribute(
    "aria-label",
    "Unread terminal notifications",
  );
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "dark");
  await page
    .getByRole("complementary", { name: "Workspaces", exact: true })
    .screenshot({
      path: testInfo.outputPath("workspace-tree-sidebar-unread-dark.png"),
    });
  await page.screenshot({
    path: testInfo.outputPath("workspace-tree-unread-dark.png"),
  });
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  await page.screenshot({
    path: testInfo.outputPath("workspace-tree-unread-light.png"),
  });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "dark");
  await toggle.click();
  await expect(list).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const calls = (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "save_session",
        );
        const latest = calls.at(-1);
        return latest ? latest.args.data.sidebar : "not-saved";
      }),
    )
    .toBe(null);

  await page.reload();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect(list).toHaveCount(0);
  await toggle.click();
  await expect(workspaceDot).toBeVisible();
  await expect(items).toBeHidden();
  await revealDots();
  await expect(workspaceDot).toBeVisible();
  await expect(paneDot).toBeVisible();

  let inbox = await openNotifications(page);
  const finished = inbox
    .locator(".notification-item")
    .filter({ hasText: "Claude Code finished responding" });
  await finished.locator(".notification-item-open").click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.notifications.items.find(
            (item: any) => item.title === "Claude Code finished responding",
          )?.read,
      ),
    )
    .toBe(true);
  expect(
    await page.evaluate(() =>
      JSON.stringify((window as any).__nativeTest.notifications),
    ),
  ).not.toContain("PRIVATE_OSC9_TEXT");
  await expect(workspaceDot).toBeVisible();
  await expect(paneDot).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(notificationInbox(page)).toHaveCount(0);

  inbox = notificationInbox(page);
  await openNotifications(page);
  const attention = inbox
    .locator(".notification-item")
    .filter({ hasText: "Agent needs your input" });
  await attention
    .getByRole("button", {
      name: "Dismiss Agent needs your input",
      exact: true,
    })
    .click();
  await expect(attention).toHaveCount(0);
  await expect(workspaceDot).toHaveCount(0);
  await expect(paneDot).toHaveCount(0);
});

test("application and unscoped legacy notifications do not mark terminal workspaces", async ({
  page,
}) => {
  const { session, workspace } = workspaceSession("Clean");
  session.sidebar = "workspaces";
  const first = workspace.tabs[0];
  if (first.type !== "terminal") throw new Error("Expected initial terminal");
  const paneId = first.activePaneId;
  await page.addInitScript((id) => {
    localStorage.setItem(
      "test-notifications",
      JSON.stringify({
        revision: 2,
        items: [
          {
            id: "application-with-pane",
            kind: "attention",
            title: "Application needs attention",
            body: "A workspace save failed.",
            createdAt: 1,
            read: false,
            source: "app",
            paneId: id,
          },
          {
            id: "legacy-without-pane",
            kind: "finished",
            title: "Legacy notification",
            body: "A prior notification has no pane link.",
            createdAt: 2,
            read: false,
            agent: "claude",
          },
        ],
      }),
    );
  }, paneId);
  await mockDesktop(page, false, session);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const entry = page.locator(`[data-workspace-id="${workspace.id}"]`);
  await expect(entry.locator(".workspace-notification-dot")).toHaveCount(0);
});
