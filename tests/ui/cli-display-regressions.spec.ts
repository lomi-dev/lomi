import { expect, test, type Page } from "@playwright/test";
import { cliNames } from "../../src/cli-agents";
import { cliTerminalDisplayTitle } from "../../src/cli-terminal-title";
import { addWorkspace, newSession } from "../../src/model";
import { defaultTerminalPreferences } from "../../src/terminal-preferences";
import { buffer, mockDesktop } from "./desktop";

test.beforeEach(async ({ page }) => {
  await page.addInitScript((defaults) => {
    localStorage.setItem(
      "test-terminal-preferences",
      JSON.stringify({ version: 1, ...defaults, alwaysShowTitles: true }),
    );
  }, defaultTerminalPreferences);
});

function setupWorkspace() {
  const session = addWorkspace(
    newSession(),
    "/work/lomi",
    "local:bash",
    "Build",
  );
  session.sidebar = null;
  const workspace = session.projects[0].workspaces[0];
  const tab = workspace.tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  return { session, workspace, paneId: tab.activePaneId };
}

async function runningSessionId(page: Page, paneId: string) {
  return page.evaluate(async (id) => {
    const runtimeUrl = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((url) => new URL(url).pathname === "/src/terminal-runtime.ts")
      .at(-1);
    const { runningTerminal } = await import(
      runtimeUrl ?? "/src/terminal-runtime.ts"
    );
    return runningTerminal(id)?.sessionId ?? null;
  }, paneId);
}

async function detectedCliIdentity(page: Page, paneId: string) {
  return page.evaluate(async (id) => {
    const runtimeUrl = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((url) => new URL(url).pathname === "/src/terminal-runtime.ts")
      .at(-1);
    const { getTerminalAgents, getTerminalAgentStates } = await import(
      runtimeUrl ?? "/src/terminal-runtime.ts"
    );
    return {
      cli: getTerminalAgents()[id] ?? null,
      pid: getTerminalAgentStates()[id]?.process?.pid ?? null,
    };
  }, paneId);
}

async function waitForSession(page: Page, paneId: string) {
  await expect.poll(() => runningSessionId(page, paneId)).not.toBeNull();
  await expect.poll(() => buffer(page, paneId)).toContain("bash $ ");
  return (await runningSessionId(page, paneId))!;
}

async function emitOsc(
  page: Page,
  paneId: string,
  sessionId: string,
  command: string,
  terminator = "\x07",
) {
  await page.evaluate(
    async ({ paneId, sessionId, command, terminator }) => {
      const native = (window as any).__nativeTest;
      native.emit(sessionId, `\x1b]${command}${terminator}`);
      const runtimeUrl = performance
        .getEntriesByType("resource")
        .map((entry) => entry.name)
        .filter((url) => new URL(url).pathname === "/src/terminal-runtime.ts")
        .at(-1);
      const { runningTerminal } = await import(
        runtimeUrl ?? "/src/terminal-runtime.ts"
      );
      const runtime = runningTerminal(paneId)!;
      await new Promise<void>((resolve) => runtime.terminal.write("", resolve));
    },
    { paneId, sessionId, command, terminator },
  );
}

async function setCliContext(
  page: Page,
  sessionId: string,
  cli: string | null,
  pid: number | null,
  foregroundProgram: string | null = cli,
) {
  await page.evaluate(
    ({ sessionId, cli, pid, foregroundProgram }) => {
      const contexts = (window as any).__nativeTest.terminalContexts;
      contexts[sessionId] = {
        cwd: "/work/lomi",
        foregroundProgram,
        titleCli: cli && pid !== null ? { cli, pid } : null,
      };
    },
    { sessionId, cli, pid, foregroundProgram },
  );
}

function workspaceEntry(page: Page, workspaceId: string) {
  return page
    .getByRole("navigation", { name: "Workspace list" })
    .locator(`.workspace-list-entry[data-workspace-id="${workspaceId}"]`);
}

async function expandTerminalItem(
  page: Page,
  workspaceId: string,
  paneId: string,
) {
  const entry = workspaceEntry(page, workspaceId);
  await entry
    .getByRole("button", { name: "Expand items in Build", exact: true })
    .click();
  const items = entry.getByRole("list", {
    name: "Open items in Build",
    exact: true,
  });
  await expect(items).toBeVisible();
  return items.locator(
    `.workspace-open-item[data-workspace-pane-id="${paneId}"]`,
  );
}

async function openWorkspaces(page: Page) {
  await page.getByRole("button", { name: "Toggle workspaces" }).click();
  await expect(
    page.getByRole("complementary", { name: "Workspaces", exact: true }),
  ).toBeVisible();
}

test("all verified CLI names replace only provider-shaped title placeholders", () => {
  const placeholders = [
    "019afe3e-47cf-70c0-a0fd-23b477911a91",
    "v0.42.1",
    "0.42.1",
  ];
  const meaningfulTitle =
    "Review 019afe3e-47cf-70c0-a0fd-23b477911a91 in build v0.42.1";

  for (const [rawCli, label] of Object.entries(cliNames)) {
    const cli = rawCli as keyof typeof cliNames;
    for (const title of placeholders)
      expect(cliTerminalDisplayTitle(title, "/usr/bin/cli", cli)).toBe(label);
    expect(cliTerminalDisplayTitle(meaningfulTitle, "cli", cli)).toBe(
      meaningfulTitle,
    );
  }

  expect(cliTerminalDisplayTitle("service-build-v2.5.4", "node", null)).toBe(
    "service-build-v2.5.4",
  );
  expect(cliTerminalDisplayTitle("", "node", null)).toBe("node");
});

test("verified provider identity updates the terminal and Workspaces title while hidden", async ({
  page,
}, testInfo) => {
  const { session, workspace, paneId } = setupWorkspace();
  await mockDesktop(page, false, session);
  await page.goto("/");
  const pane = page.locator(`[data-pane-id="${paneId}"]`);
  await expect(pane.locator(".xterm-screen")).toBeVisible();
  const sessionId = await waitForSession(page, paneId);
  const heading = pane.locator(".terminal-title");

  await emitOsc(page, paneId, sessionId, "133;C");
  await emitOsc(
    page,
    paneId,
    sessionId,
    "2;019afe3e-47cf-70c0-a0fd-23b477911a91",
  );
  await expect(heading).toHaveText("019afe3e-47cf-70c0-a0fd-23b477911a91");
  await setCliContext(page, sessionId, "codex", 301);
  await expect(heading).toHaveText("Codex");
  await expect
    .poll(() => detectedCliIdentity(page, paneId))
    .toEqual({
      cli: "codex",
      pid: 301,
    });

  await openWorkspaces(page);
  let leaf = await expandTerminalItem(page, workspace.id, paneId);
  await expect(leaf.locator(".workspace-item-label")).toHaveText("Codex");

  for (const appearance of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: appearance });
    await expect(page.locator("html")).toHaveAttribute(
      "data-appearance",
      appearance,
    );
    await pane.locator(".terminal-title-box").screenshot({
      path: testInfo.outputPath(`cli-fallback-terminal-${appearance}.png`),
    });
    await leaf.screenshot({
      path: testInfo.outputPath(`cli-fallback-workspace-${appearance}.png`),
    });
  }

  await emitOsc(page, paneId, sessionId, "2;v0.42.1");
  await expect(heading).toHaveText("Codex");
  await expect(leaf.locator(".workspace-item-label")).toHaveText("Codex");
  await emitOsc(page, paneId, sessionId, "2;Review checkout recovery");
  await expect(heading).toHaveText("Review checkout recovery");
  await expect(leaf.locator(".workspace-item-label")).toHaveText(
    "Review checkout recovery",
  );

  await setCliContext(page, sessionId, "codex", 302);
  await expect(heading).toHaveText("Review checkout recovery");
  await expect
    .poll(() => detectedCliIdentity(page, paneId))
    .toEqual({
      cli: "codex",
      pid: 302,
    });
  await emitOsc(
    page,
    paneId,
    sessionId,
    "2;019afe3e-47cf-70c0-a0fd-23b477911a91",
  );
  await setCliContext(page, sessionId, "claude", 303);
  await expect(heading).toHaveText("Claude Code");
  await expect
    .poll(() => detectedCliIdentity(page, paneId))
    .toEqual({
      cli: "claude",
      pid: 303,
    });
  await expect(leaf.locator(".workspace-item-label")).toHaveText("Claude Code");

  await page.getByRole("button", { name: "Toggle workspaces" }).click();
  await emitOsc(page, paneId, sessionId, "2;0.42.1");
  await setCliContext(page, sessionId, "gemini", 304);
  await expect(heading).toHaveText("Gemini CLI");
  await expect
    .poll(() => detectedCliIdentity(page, paneId))
    .toEqual({
      cli: "gemini",
      pid: 304,
    });
  await openWorkspaces(page);
  leaf = await expandTerminalItem(page, workspace.id, paneId);
  await expect(leaf.locator(".workspace-item-label")).toHaveText("Gemini CLI");
  await emitOsc(page, paneId, sessionId, "2;Implement stream recovery");
  await expect(heading).toHaveText("Implement stream recovery");
  await expect(leaf.locator(".workspace-item-label")).toHaveText(
    "Implement stream recovery",
  );

  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
  expect(await runningSessionId(page, paneId)).toBe(sessionId);
});

test("non-CLI terminal titles remain raw when the foreground process is known", async ({
  page,
}) => {
  const { session, workspace, paneId } = setupWorkspace();
  await mockDesktop(page, false, session);
  await page.goto("/");
  const pane = page.locator(`[data-pane-id="${paneId}"]`);
  await expect(pane.locator(".xterm-screen")).toBeVisible();
  const sessionId = await waitForSession(page, paneId);
  const title = "service-build-v2.5.4";

  await emitOsc(page, paneId, sessionId, "133;C");
  await emitOsc(page, paneId, sessionId, `2;${title}`);
  await setCliContext(page, sessionId, null, null, "node");
  await expect(pane.locator(".terminal-title")).toHaveText(title);

  await openWorkspaces(page);
  const leaf = await expandTerminalItem(page, workspace.id, paneId);
  await expect(leaf.locator(".workspace-item-label")).toHaveText(title);
  await expect(
    workspaceEntry(page, workspace.id).locator(".workspace-agent-chip"),
  ).toHaveCount(0);
});

test("Codex lifecycle status stays authoritative over free-form OSC 9 text", async ({
  page,
}) => {
  const { session, workspace, paneId } = setupWorkspace();
  await mockDesktop(page, false, session);
  await page.goto("/");
  const pane = page.locator(`[data-pane-id="${paneId}"]`);
  await expect(pane.locator(".xterm-screen")).toBeVisible();
  const sessionId = await waitForSession(page, paneId);
  const activity = pane.locator(".terminal-activity");
  await emitOsc(page, paneId, sessionId, "133;C");
  const codexStatus = (state: string, kind?: string) =>
    emitOsc(
      page,
      paneId,
      sessionId,
      `7501;state=${state}:app=codex${kind ? `:kind=${kind}` : ""}`,
      "\x1b\\",
    );

  await codexStatus("working");
  await expect(activity).toHaveCount(0);
  await setCliContext(page, sessionId, "codex", 401);

  await openWorkspaces(page);
  const leaf = await expandTerminalItem(page, workspace.id, paneId);
  const cliIcon = leaf.locator(".workspace-cli-icon");
  const activityDot = cliIcon.locator(".workspace-cli-activity-dot");
  await expect(
    workspaceEntry(page, workspace.id).locator(
      '.workspace-agent-chip[data-cli="codex"]',
    ),
  ).toBeVisible();
  await expect(activity).toHaveText("Working");
  await expect(activityDot).toHaveAttribute("data-activity", "working");
  await codexStatus("working");
  await expect(activity).toHaveText("Working");
  await expect(activityDot).toHaveAttribute("data-activity", "working");

  await codexStatus("blocked", "permission");
  await expect(activity).toHaveText("Needs input");
  await expect(activityDot).toHaveAttribute("data-activity", "attention");
  await emitOsc(
    page,
    paneId,
    sessionId,
    "9;The implementation is complete; no approval is needed.",
  );
  await expect(activity).toHaveText("Needs input");
  await codexStatus("done");
  await expect(activity).toHaveText("Done");
  await expect(activityDot).toHaveCount(0);

  await emitOsc(
    page,
    paneId,
    sessionId,
    "9;I completed this task; the response mentions an approval request.",
  );
  await expect(activity).toHaveText("Done");
  await expect(activityDot).toHaveCount(0);

  await codexStatus("blocked", "permission");
  await expect(activityDot).toHaveAttribute("data-activity", "attention");
  await emitOsc(page, paneId, sessionId, "133;D;0");
  await emitOsc(page, paneId, sessionId, "133;A");
  await expect(activity).toHaveCount(0);
  await expect(activityDot).toHaveCount(0);

  await emitOsc(page, paneId, sessionId, "133;C");
  await codexStatus("working");
  await expect(activity).toHaveText("Working");
  await codexStatus("blocked", "question");
  await expect(activity).toHaveText("Needs input");
  await codexStatus("error");
  await expect(activity).toHaveText("Error");
  await expect(activityDot).toHaveAttribute("data-activity", "error");
  await codexStatus("idle");
  await expect(activity).toHaveCount(0);
  await expect(activityDot).toHaveCount(0);

  await codexStatus("blocked", "permission");
  await expect(activityDot).toHaveAttribute("data-activity", "attention");
  await emitOsc(page, paneId, sessionId, "7501;state=clear", "\x1b\\");
  await expect(activity).toHaveCount(0);
  await expect(activityDot).toHaveCount(0);

  await codexStatus("blocked", "auth");
  await expect(activity).toHaveText("Needs input");
  await expect(activityDot).toHaveAttribute("data-activity", "attention");
  await codexStatus("error");
  await expect(activity).toHaveText("Error");
  await expect(activityDot).toHaveAttribute("data-activity", "error");
  await codexStatus("done");
  await expect(activity).toHaveText("Done");
  await expect(activityDot).toHaveCount(0);

  await emitOsc(
    page,
    paneId,
    sessionId,
    "7501;state=blocked:app=unknown-cli:kind=permission",
    "\x1b\\",
  );
  await emitOsc(
    page,
    paneId,
    sessionId,
    "7501;state=not-a-state:app=codex",
    "\x1b\\",
  );
  await emitOsc(
    page,
    paneId,
    sessionId,
    "777;notify;Lomi;unknown-cli;attention",
  );
  await expect(activity).toHaveText("Done");
  await expect(activityDot).toHaveCount(0);

  await codexStatus("working");
  await expect(activity).toHaveText("Working");
  await setCliContext(page, sessionId, "claude", 402);
  await expect(
    workspaceEntry(page, workspace.id).locator(
      '.workspace-agent-chip[data-cli="claude"]',
    ),
  ).toBeVisible();
  await expect(activity).toHaveCount(0);
  await expect(activityDot).toHaveCount(0);
  await emitOsc(
    page,
    paneId,
    sessionId,
    "7501;state=blocked:app=codex:kind=permission",
    "\x1b\\",
  );
  await expect(activity).toHaveCount(0);
  await expect(activityDot).toHaveCount(0);

  await setCliContext(page, sessionId, "codex", 401);
  await expect
    .poll(() => detectedCliIdentity(page, paneId))
    .toEqual({
      cli: "codex",
      pid: 401,
    });
  await codexStatus("working");
  await expect(activity).toHaveText("Working");
  await emitOsc(page, paneId, sessionId, "133;D;0");
  await emitOsc(page, paneId, sessionId, "133;A");
  await expect(activity).toHaveCount(0);
  await expect(activityDot).toHaveCount(0);

  await setCliContext(page, sessionId, null, null, null);
  await expect
    .poll(() => detectedCliIdentity(page, paneId))
    .toEqual({
      cli: null,
      pid: null,
    });
  await emitOsc(
    page,
    paneId,
    sessionId,
    "7501;state=blocked:app=codex:kind=permission",
    "\x1b\\",
  );
  await expect(activity).toHaveCount(0);
  await expect(activityDot).toHaveCount(0);

  await setCliContext(page, sessionId, "codex", 999);
  await expect
    .poll(() => detectedCliIdentity(page, paneId))
    .toEqual({
      cli: "codex",
      pid: 999,
    });
  await expect(activity).toHaveCount(0);
  await expect(activityDot).toHaveCount(0);
  await emitOsc(page, paneId, sessionId, "133;C");
  await codexStatus("working");
  await expect(activity).toHaveText("Working");
  await expect(activityDot).toHaveAttribute("data-activity", "working");
});

test("Codex OSC 9 notifications save workspace context without terminal text", async ({
  page,
}) => {
  const { session, workspace, paneId } = setupWorkspace();
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-terminal-notification-policy",
      JSON.stringify({
        accountId: "user-a",
        available: true,
        preferences: { enabled: true, backgroundOnly: false },
      }),
    );
  });
  await mockDesktop(page, false, session);
  await page.goto("/");
  const pane = page.locator(`[data-pane-id="${paneId}"]`);
  await expect(pane.locator(".xterm-screen")).toBeVisible();
  const sessionId = await waitForSession(page, paneId);
  await setCliContext(page, sessionId, "codex", 501);

  await openWorkspaces(page);
  const leaf = await expandTerminalItem(page, workspace.id, paneId);
  await expect(
    workspaceEntry(page, workspace.id).locator(
      '.workspace-agent-chip[data-cli="codex"]',
    ),
  ).toBeVisible();
  const privateTerminalText = "PRIVATE_TERMINAL_COMMAND_MUST_NOT_BE_SAVED";
  await emitOsc(
    page,
    paneId,
    sessionId,
    `9;Approval requested: ${privateTerminalText}`,
  );

  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.notifications.items.length,
      ),
    )
    .toBe(1);
  await expect(leaf.locator(".workspace-notification-dot")).toBeVisible();
  const savedNotifications = await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    return {
      items: native.notifications.items,
      storage: localStorage.getItem("test-notifications"),
    };
  });
  expect(savedNotifications.items[0].body).toBe("lomi · Build · Terminal");
  expect(JSON.stringify(savedNotifications)).not.toContain(privateTerminalText);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "notify_agent",
        ).length,
    ),
  ).toBe(1);
});
