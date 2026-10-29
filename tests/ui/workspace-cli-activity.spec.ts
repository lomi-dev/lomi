import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  addWorkspace,
  newPane,
  newSession,
  splitPane,
  type Workspace,
} from "../../src/model";
import { mockDesktop } from "./desktop";

type Activity = "working" | "attention" | "error";
type Cli = "codex" | "claude";

const activityLabels: Record<Activity, string> = {
  working: "Working",
  attention: "Needs input",
  error: "CLI error or usage limit reached",
};

function workspaceSession(name = "Build", path = "/work/lomi") {
  const session = addWorkspace(newSession(), path, "local:bash", name);
  session.sidebar = "workspaces";
  const project = session.projects.find(
    (candidate) => candidate.path === path,
  )!;
  const workspace = project.workspaces[0];
  const tab = workspace.tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  return { session, project, workspace, tab };
}

function terminalContext(cli: Cli, pid: number) {
  return {
    cwd: "/work/lomi",
    foregroundProgram: cli,
    titleCli: { cli, pid },
  };
}

function workspaceEntry(page: Page, workspace: Workspace) {
  return page
    .getByRole("navigation", { name: "Workspace list" })
    .locator(`.workspace-list-entry[data-workspace-id="${workspace.id}"]`);
}

async function sessionForPane(page: Page, paneId: string) {
  return page.evaluate(async (id) => {
    const url = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((name) => new URL(name).pathname === "/src/terminal-runtime.ts")
      .at(-1);
    const { runningTerminal } = await import(url ?? "/src/terminal-runtime.ts");
    return runningTerminal(id)?.sessionId ?? null;
  }, paneId);
}

async function waitForSession(page: Page, paneId: string) {
  await expect.poll(() => sessionForPane(page, paneId)).not.toBeNull();
  return (await sessionForPane(page, paneId))!;
}

async function setContexts(
  page: Page,
  contexts: Record<string, ReturnType<typeof terminalContext>>,
) {
  await page.evaluate((value) => {
    (window as any).__nativeTest.terminalContexts = value;
  }, contexts);
}

async function emitOutput(
  page: Page,
  paneId: string,
  sessionId: string,
  output: string,
) {
  await page.evaluate(
    async ({ paneId, sessionId, output }) => {
      const native = (window as any).__nativeTest;
      native.emit(sessionId, output);
      const url = performance
        .getEntriesByType("resource")
        .map((entry) => entry.name)
        .filter((name) => new URL(name).pathname === "/src/terminal-runtime.ts")
        .at(-1);
      const { runningTerminal } = await import(
        url ?? "/src/terminal-runtime.ts"
      );
      const runtime = runningTerminal(paneId)!;
      await new Promise<void>((resolve) => runtime.terminal.write("", resolve));
    },
    { paneId, sessionId, output },
  );
}

async function sendOsc(
  page: Page,
  paneId: string,
  sessionId: string,
  command: string,
) {
  await emitOutput(page, paneId, sessionId, `\x1b]${command}\x07`);
}

async function expandWorkspace(page: Page, workspace: Workspace) {
  const entry = workspaceEntry(page, workspace);
  await entry
    .getByRole("button", {
      name: `Expand items in ${workspace.name}`,
      exact: true,
    })
    .click();
  const items = entry.getByRole("list", {
    name: `Open items in ${workspace.name}`,
    exact: true,
  });
  await expect(items).toBeVisible();
  return { entry, items };
}

async function expectIconActivity(icon: Locator, activity: Activity) {
  await expect(icon).toHaveAttribute("data-activity", activity);
  await expect(icon).toHaveAttribute(
    "title",
    new RegExp(activityLabels[activity], "i"),
  );
  const dot = icon.locator(".workspace-cli-activity-dot");
  await expect(dot).toBeVisible();
  await expect(dot).toHaveAttribute("data-activity", activity);
}

async function expectContainerDescription(
  container: Locator,
  activity: Activity,
) {
  await expect(container).toHaveAttribute(
    "aria-description",
    new RegExp(activityLabels[activity], "i"),
  );
}

async function seedUnreadForPane(page: Page, paneId: string) {
  await page.evaluate(async (id) => {
    const native = (window as any).__nativeTest;
    const snapshot = {
      revision: 50,
      items: [
        {
          id: "cli-activity-unread",
          kind: "attention",
          title: "Needs input",
          body: "Saved terminal notification",
          createdAt: Date.now(),
          read: false,
          paneId: id,
          agent: "claude",
        },
      ],
    };
    native.notifications = snapshot;
    localStorage.setItem("test-notifications", JSON.stringify(snapshot));
    await native.emitEvent("notifications-changed", snapshot);
  }, paneId);
}

async function takeSidebarScreenshot(
  page: Page,
  testInfo: { outputPath: (name: string) => string },
  activity: Activity,
  appearance: "dark" | "light",
) {
  await page.emulateMedia({ colorScheme: appearance });
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    appearance,
  );
  await page
    .getByRole("complementary", { name: "Workspaces", exact: true })
    .screenshot({
      path: testInfo.outputPath(`workspace-cli-${activity}-${appearance}.png`),
    });
}

test("CLI activity follows terminal signals and survives a hidden workspace sidebar", async ({
  page,
}, testInfo) => {
  const { session, workspace, tab } = workspaceSession();
  const paneId = tab.activePaneId;
  session.sidebar = null;
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page, false, session);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();

  const sessionId = await waitForSession(page, paneId);
  await setContexts(page, {
    [sessionId]: terminalContext("claude", 301),
  });
  await expect(
    page.locator('.agent-usage-summary[data-cli="claude"]'),
  ).toBeVisible();

  await sendOsc(page, paneId, sessionId, "9;4;2;101");
  await sendOsc(page, paneId, sessionId, "9;4;1;bad");
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.notifications.items.length === 0 &&
        !(window as any).__nativeTest.calls.some(
          (call: any) => call.command === "notify_agent",
        ),
    ),
  ).toBe(true);

  await page.getByRole("button", { name: "Toggle workspaces" }).click();
  const { entry, items } = await expandWorkspace(page, workspace);
  const chip = entry.locator('.workspace-agent-chip[data-cli="claude"]');
  const leaf = items.locator(
    `.workspace-open-item[data-workspace-pane-id="${paneId}"]`,
  );
  const leafIcon = leaf.locator(".workspace-cli-icon");
  await expect(chip).toBeVisible();
  await expect(leaf).toBeVisible();
  await expect(leafIcon.locator(".workspace-cli-activity-dot")).toHaveCount(0);

  // The runtime records parser updates even while Workspaces is unmounted.
  await page.getByRole("button", { name: "Toggle workspaces" }).click();
  await sendOsc(page, paneId, sessionId, "133;C");
  await sendOsc(page, paneId, sessionId, "2;⠋ Claude is working");
  await page.getByRole("button", { name: "Toggle workspaces" }).click();
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");
  await entry
    .getByRole("button", { name: "Expand items in Build", exact: true })
    .click();
  await expect(items).toBeVisible();
  await expectIconActivity(leafIcon, "working");
  await expectContainerDescription(chip, "working");
  await expectContainerDescription(leaf, "working");

  await seedUnreadForPane(page, paneId);
  await expect(leaf.locator(".workspace-notification-dot")).toBeVisible();
  await expect(leafIcon.locator(".workspace-cli-activity-dot")).toBeVisible();
  await takeSidebarScreenshot(page, testInfo, "working", "dark");

  await sendOsc(page, paneId, sessionId, "2;Claude is waiting");
  await expect(leafIcon.locator(".workspace-cli-activity-dot")).toHaveCount(0);
  await sendOsc(page, paneId, sessionId, "777;notify;Lomi;claude;working");
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");
  await expectIconActivity(leafIcon, "working");
  await sendOsc(page, paneId, sessionId, "777;notify;Lomi;claude;attention");
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "attention");
  await expectIconActivity(leafIcon, "attention");
  await sendOsc(page, paneId, sessionId, "2;⠙ Claude is still waiting");
  await expectIconActivity(leafIcon, "attention");
  await expectContainerDescription(leaf, "attention");
  await expect(leaf.locator(".workspace-notification-dot")).toBeVisible();
  await takeSidebarScreenshot(page, testInfo, "attention", "dark");

  await sendOsc(page, paneId, sessionId, "9;4;2;65");
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "error");
  await expectIconActivity(leafIcon, "error");
  await sendOsc(page, paneId, sessionId, "2;⠹ Claude is still working");
  await expectIconActivity(leafIcon, "error");
  await expectContainerDescription(leaf, "error");
  await expect(leaf.locator(".workspace-notification-dot")).toBeVisible();
  await takeSidebarScreenshot(page, testInfo, "error", "dark");

  await sendOsc(page, paneId, sessionId, "9;4;0");
  await expect(leafIcon.locator(".workspace-cli-activity-dot")).toHaveCount(0);
  await expect(chip.locator(".workspace-cli-activity-dot")).toHaveCount(0);

  await sendOsc(page, paneId, sessionId, "9;4;1;42");
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");
  await expectIconActivity(leafIcon, "working");
  await takeSidebarScreenshot(page, testInfo, "working", "light");
  await sendOsc(page, paneId, sessionId, "777;notify;Lomi;claude;attention");
  await expectIconActivity(leafIcon, "attention");
  await takeSidebarScreenshot(page, testInfo, "attention", "light");
  await sendOsc(page, paneId, sessionId, "9;4;2;65");
  await expectIconActivity(leafIcon, "error");
  await takeSidebarScreenshot(page, testInfo, "error", "light");

  await sendOsc(page, paneId, sessionId, "777;notify;Lomi;claude;finished");
  await expect(leafIcon.locator(".workspace-cli-activity-dot")).toHaveCount(0);
  await expect(chip.locator(".workspace-cli-activity-dot")).toHaveCount(0);
  await expect(leaf.locator(".workspace-notification-dot")).toBeVisible();
});

test("provider activity aggregates by priority and stays with its workspace", async ({
  page,
}) => {
  let session = addWorkspace(
    newSession(),
    "/work/build",
    "local:bash",
    "Build",
  );
  const buildProjectId = session.projects[0].id;
  const build = session.projects[0].workspaces[0];
  const buildTab = build.tabs[0];
  if (buildTab.type !== "terminal") throw new Error("Expected terminal tab");
  const buildFirst = buildTab.activePaneId;
  const buildSecond = newPane("/work/build");
  buildTab.layout = splitPane(
    buildTab.layout,
    buildFirst,
    "horizontal",
    buildSecond,
  );
  session = addWorkspace(session, "/work/docs", "local:bash", "Docs");
  const docsProject = session.projects.find(
    (project) => project.path === "/work/docs",
  )!;
  const docs = docsProject.workspaces[0];
  const docsTab = docs.tabs[0];
  if (docsTab.type !== "terminal") throw new Error("Expected terminal tab");
  const docsPane = docsTab.activePaneId;
  session.activeProjectId = buildProjectId;
  session.projects.find(
    (project) => project.id === buildProjectId,
  )!.activeWorkspaceId = build.id;
  session.sidebar = "workspaces";

  await mockDesktop(page, false, session);
  await page.goto("/");
  await expect(page.locator(".xterm-screen").first()).toBeVisible();
  const buildFirstSession = await waitForSession(page, buildFirst);
  const buildSecondSession = await waitForSession(page, buildSecond.id);
  await setContexts(page, {
    [buildFirstSession]: terminalContext("codex", 401),
    [buildSecondSession]: terminalContext("codex", 402),
  });
  const buildTree = await expandWorkspace(page, build);
  const buildChip = buildTree.entry.locator(
    '.workspace-agent-chip[data-cli="codex"]',
  );
  const buildPaneA = buildTree.items.locator(
    `.workspace-open-item[data-workspace-pane-id="${buildFirst}"]`,
  );
  const buildPaneB = buildTree.items.locator(
    `.workspace-open-item[data-workspace-pane-id="${buildSecond.id}"]`,
  );

  await sendOsc(page, buildFirst, buildFirstSession, "9;4;1;50");
  await sendOsc(page, buildSecond.id, buildSecondSession, "9;4;2;70");
  await expectIconActivity(
    buildPaneA.locator(".workspace-cli-icon"),
    "working",
  );
  await expectIconActivity(buildPaneB.locator(".workspace-cli-icon"), "error");
  await expectIconActivity(buildChip.locator(".workspace-cli-icon"), "error");
  await expect(buildChip).toHaveAttribute("data-activity", "error");

  const docsEntry = workspaceEntry(page, docs);
  await docsEntry.locator(".workspace-list-item").click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const docsSession = await waitForSession(page, docsPane);
  await setContexts(page, {
    [buildFirstSession]: terminalContext("codex", 401),
    [buildSecondSession]: terminalContext("codex", 402),
    [docsSession]: terminalContext("claude", 501),
  });
  const docsTree = await expandWorkspace(page, docs);
  const docsChip = docsTree.entry.locator(
    '.workspace-agent-chip[data-cli="claude"]',
  );
  const docsLeaf = docsTree.items.locator(
    `.workspace-open-item[data-workspace-pane-id="${docsPane}"]`,
  );
  await sendOsc(
    page,
    docsPane,
    docsSession,
    "777;notify;Lomi;claude;attention",
  );

  await expectIconActivity(
    docsChip.locator(".workspace-cli-icon"),
    "attention",
  );
  await expectIconActivity(
    docsLeaf.locator(".workspace-cli-icon"),
    "attention",
  );
  await expectIconActivity(buildChip.locator(".workspace-cli-icon"), "error");
  await expect(
    buildTree.entry.locator('.workspace-agent-chip[data-cli="claude"]'),
  ).toHaveCount(0);
  await expect(
    docsTree.entry.locator('.workspace-agent-chip[data-cli="codex"]'),
  ).toHaveCount(0);
});

type UsageStatus = "ready" | "unsupported" | "error";

function usageEntry(
  id: string,
  pid: number,
  options: {
    cli?: Cli;
    status?: UsageStatus;
    remainingPercent?: number | null;
    used?: number | null;
    limit?: number | null;
    updatedAt?: number;
  } = {},
) {
  return {
    id,
    process: { cli: options.cli ?? "claude", pid },
    accountKey: "account-test",
    status: options.status ?? "ready",
    windows: [
      {
        label: "5-hour",
        remainingPercent:
          options.remainingPercent === undefined ? 0 : options.remainingPercent,
        used: options.used ?? null,
        limit: options.limit ?? null,
        unit: "tokens",
        resetsAt: null,
      },
    ],
    updatedAt: options.updatedAt ?? Date.now(),
    retryAt: null,
    message: null,
    source: "oauth",
  };
}

async function usageCallCount(page: Page) {
  return page.evaluate(() => (window as any).__nativeTest.cliUsageCalls.length);
}

async function refreshUsage(page: Page) {
  const before = await usageCallCount(page);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect.poll(() => usageCallCount(page)).toBeGreaterThan(before);
}

test("only fresh matching quota exhaustion turns Claude red", async ({
  page,
}) => {
  const { session, workspace, tab } = workspaceSession();
  const paneId = tab.activePaneId;
  await mockDesktop(page, false, session);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const sessionId = await waitForSession(page, paneId);
  await setContexts(page, {
    [sessionId]: terminalContext("claude", 601),
  });
  const { entry, items } = await expandWorkspace(page, workspace);
  const chip = entry.locator('.workspace-agent-chip[data-cli="claude"]');
  const leaf = items.locator(
    `.workspace-open-item[data-workspace-pane-id="${paneId}"]`,
  );
  const leafIcon = leaf.locator(".workspace-cli-icon");
  await sendOsc(page, paneId, sessionId, "9;4;1;40");
  await expectIconActivity(leafIcon, "working");

  await page.evaluate(
    (value) => {
      (window as any).__nativeTest.cliUsageEntries = [value];
    },
    usageEntry(`${sessionId}-other`, 601, { remainingPercent: 0 }),
  );
  await refreshUsage(page);
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");

  await page.evaluate(
    (value) => {
      (window as any).__nativeTest.cliUsageEntries = [value];
    },
    usageEntry(sessionId, 699, { remainingPercent: 0 }),
  );
  await refreshUsage(page);
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");

  await page.evaluate(
    (value) => {
      (window as any).__nativeTest.cliUsageEntries = [value];
    },
    usageEntry(sessionId, 601, { remainingPercent: 0 }),
  );
  await refreshUsage(page);
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "error");
  await expectIconActivity(leafIcon, "error");

  const freshUsage = page.locator('.agent-usage-summary[data-cli="claude"]');
  await page.evaluate(
    (value) => {
      (window as any).__nativeTest.cliUsageEntries = [value];
    },
    usageEntry(sessionId, 601, {
      remainingPercent: null,
      used: 100,
      limit: 100,
    }),
  );
  await refreshUsage(page);
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "error");
  await expect(freshUsage.locator(".agent-usage-trigger-value")).toHaveText(
    "—",
  );

  await page.evaluate(() => {
    (window as any).__nativeTest.cliUsageError = "usage reader unavailable";
  });
  await refreshUsage(page);
  await expect(freshUsage).toHaveAttribute("data-stale", "true");
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");
  await expectIconActivity(leafIcon, "working");

  await page.evaluate(
    ({ entries, error }) => {
      const native = (window as any).__nativeTest;
      native.cliUsageEntries = entries;
      native.cliUsageError = error;
    },
    {
      entries: [usageEntry(sessionId, 601, { remainingPercent: 45 })],
      error: "",
    },
  );
  await refreshUsage(page);
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");
  await expectIconActivity(leafIcon, "working");
  await expect(freshUsage.locator(".agent-usage-trigger-value")).toHaveText(
    "45%",
  );
  await expect(freshUsage).not.toHaveAttribute("data-stale", "true");

  await page.evaluate(
    (value) => {
      (window as any).__nativeTest.cliUsageEntries = [value];
    },
    usageEntry(sessionId, 601, {
      remainingPercent: 0,
      updatedAt: Date.now() - 60_000,
    }),
  );
  await refreshUsage(page);
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");
  await expectIconActivity(leafIcon, "working");
  await expect(freshUsage.locator(".agent-usage-trigger-value")).toHaveText(
    "0%",
  );
  await expect(freshUsage).toHaveAttribute("data-stale", "true");

  await page.evaluate(
    (value) => {
      (window as any).__nativeTest.cliUsageEntries = [value];
    },
    usageEntry(sessionId, 601, { status: "unsupported", remainingPercent: 0 }),
  );
  await refreshUsage(page);
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");
  await expectIconActivity(leafIcon, "working");
  await expect(freshUsage.locator(".agent-usage-trigger-value")).toHaveText(
    "0%",
  );
  await expect(freshUsage).toHaveAttribute("data-stale", "true");

  await page.evaluate(() => {
    (window as any).__nativeTest.cliUsageEntries = [];
  });
  await refreshUsage(page);
  await expectIconActivity(chip.locator(".workspace-cli-icon"), "working");
  await expectIconActivity(leafIcon, "working");
  await expect(freshUsage.locator(".agent-usage-trigger-value")).toHaveText(
    "—",
  );
  await expect(freshUsage).not.toHaveAttribute("data-stale", "true");
});

test("exhausted quota ages out while its next usage refresh is still pending", async ({
  page,
}) => {
  const { session, workspace, tab } = workspaceSession();
  await page.clock.install();
  await mockDesktop(page, false, session);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();

  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.sessions.size))
    .toBe(1);
  const sessionId = await page.evaluate(
    () => [...(window as any).__nativeTest.sessions.keys()][0] as string,
  );
  await page.evaluate(
    ({ sessionId, entry }) => {
      const native = (window as any).__nativeTest;
      native.terminalContexts = {
        [sessionId]: { ...entry.context },
      };
      native.cliUsageEntries = [entry.usage];
    },
    {
      sessionId,
      entry: {
        context: terminalContext("claude", 801),
        usage: usageEntry(sessionId, 801, { remainingPercent: 0 }),
      },
    },
  );
  await page.clock.runFor(1000);
  const { entry } = await expandWorkspace(page, workspace);
  const chip = entry.locator('.workspace-agent-chip[data-cli="claude"]');
  const usage = page.locator('.agent-usage-summary[data-cli="claude"]');
  await expect(chip).toHaveAttribute("data-activity", "error");
  await expect(usage.locator(".agent-usage-trigger-value")).toHaveText("0%");

  await page.evaluate(() => {
    (window as any).__nativeTest.cliUsageDelay = 70_000;
  });
  const callsBefore = await usageCallCount(page);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect.poll(() => usageCallCount(page)).toBeGreaterThan(callsBefore);
  const pendingCallCount = await usageCallCount(page);

  await page.clock.runFor(60_000);
  await expect(usage).toHaveAttribute("data-stale", "true");
  await expect(chip.locator(".workspace-cli-activity-dot")).toHaveCount(0);
  await expect.poll(() => usageCallCount(page)).toBe(pendingCallCount);

  await page.clock.runFor(10_000);
  await expect(chip.locator(".workspace-cli-activity-dot")).toHaveCount(0);
});

test("a changed CLI process clears activity from the previous process", async ({
  page,
}) => {
  const { session, workspace, tab } = workspaceSession();
  const paneId = tab.activePaneId;
  await mockDesktop(page, false, session);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const sessionId = await waitForSession(page, paneId);
  await setContexts(page, {
    [sessionId]: terminalContext("codex", 701),
  });
  const { entry, items } = await expandWorkspace(page, workspace);
  const chip = entry.locator('.workspace-agent-chip[data-cli="codex"]');
  const leaf = items.locator(
    `.workspace-open-item[data-workspace-pane-id="${paneId}"]`,
  );
  await sendOsc(page, paneId, sessionId, "9;4;2;55");
  await expect(chip).toHaveAttribute("data-activity", "error");
  await expect(leaf.locator(".workspace-cli-icon")).toHaveAttribute(
    "data-activity",
    "error",
  );

  await setContexts(page, {
    [sessionId]: terminalContext("codex", 702),
  });
  await expect(chip.locator(".workspace-cli-activity-dot")).toHaveCount(0);
  await expect(leaf.locator(".workspace-cli-activity-dot")).toHaveCount(0);

  await sendOsc(page, paneId, sessionId, "9;4;1;30");
  await expect(chip).toHaveAttribute("data-activity", "working");
  await setContexts(page, {
    [sessionId]: terminalContext("claude", 703),
  });
  const claudeChip = entry.locator('.workspace-agent-chip[data-cli="claude"]');
  await expect(claudeChip).toBeVisible();
  await expect(claudeChip.locator(".workspace-cli-activity-dot")).toHaveCount(
    0,
  );
  await expect(
    entry.locator('.workspace-agent-chip[data-cli="codex"]'),
  ).toHaveCount(0);
  await expect(leaf.locator(".workspace-cli-activity-dot")).toHaveCount(0);
});
