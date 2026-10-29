import { expect, test, type Locator, type Page } from "@playwright/test";
import { addWorkspace, newSession } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";

const codexPid = 46_425;

function setupWorkspace() {
  const session = addWorkspace(
    newSession(),
    "/work/lomi",
    "local:bash",
    "Build",
  );
  session.sidebar = "workspaces";
  const workspace = session.projects[0].workspaces[0];
  const tab = workspace.tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  return { session, workspace, paneId: tab.activePaneId };
}

async function terminalSnapshot(page: Page, paneId: string) {
  return page.evaluate(async (id) => {
    const runtimeUrl = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((name) => new URL(name).pathname === "/src/terminal-runtime.ts")
      .at(-1);
    const { getTerminalAgentStates, runningTerminal } = await import(
      runtimeUrl ?? "/src/terminal-runtime.ts"
    );
    const runtime = runningTerminal(id);
    const state = getTerminalAgentStates()[id];
    return {
      sessionId: runtime?.sessionId ?? null,
      process: state?.process ?? null,
      activity: state?.activity ?? null,
      programStatus: runtime?.getSnapshot().programStatus ?? null,
    };
  }, paneId);
}

async function waitForTerminal(page: Page, paneId: string) {
  await expect
    .poll(() => terminalSnapshot(page, paneId))
    .toMatchObject({
      sessionId: expect.any(String),
    });
  await expect.poll(() => buffer(page, paneId)).toContain("bash $ ");
  return (await terminalSnapshot(page, paneId)).sessionId!;
}

async function setCodexContext(page: Page, sessionId: string, pid: number) {
  await page.evaluate(
    ({ sessionId, pid }) => {
      (window as any).__nativeTest.terminalContexts[sessionId] = {
        cwd: "/work/lomi",
        foregroundProgram: "codex",
        titleCli: { cli: "codex", pid },
      };
    },
    { sessionId, pid },
  );
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
        .filter((name) => new URL(name).pathname === "/src/terminal-runtime.ts")
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

async function setQuotaAndRefresh(page: Page, sessionId: string) {
  await page.evaluate(
    ({ sessionId, pid }) => {
      const native = (window as any).__nativeTest;
      const now = Date.now();
      native.cliUsageEntries = [
        {
          id: sessionId,
          process: { cli: "codex", pid },
          accountKey: "account-test",
          status: "ready",
          windows: [
            {
              label: "5-hour",
              remainingPercent: 0,
              used: 100,
              limit: 100,
              unit: "tokens",
              resetsAt: now + 75 * 60_000,
            },
            {
              label: "7-day",
              remainingPercent: 69,
              used: 310,
              limit: 1000,
              unit: "tokens",
              resetsAt: now + 6 * 24 * 60 * 60_000,
            },
          ],
          updatedAt: now,
          retryAt: null,
          message: null,
          source: "oauth",
        },
      ];
    },
    { sessionId, pid: codexPid },
  );
  const callsBefore = await page.evaluate(
    () => (window as any).__nativeTest.cliUsageCalls.length,
  );
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__nativeTest.cliUsageCalls.length),
    )
    .toBeGreaterThan(callsBefore);
  await expect
    .poll(() =>
      page.evaluate(
        ({ callsBefore, sessionId, pid }) => {
          const calls = (window as any).__nativeTest.cliUsageCalls;
          return calls
            .slice(callsBefore)
            .some((call: any) =>
              call.targets?.some(
                (target: any) =>
                  target.id === sessionId &&
                  target.process.cli === "codex" &&
                  target.process.pid === pid,
              ),
            );
        },
        { callsBefore, sessionId, pid: codexPid },
      ),
    )
    .toBe(true);
}

function workspaceEntry(page: Page, workspaceId: string) {
  return page
    .getByRole("navigation", { name: "Workspace list" })
    .locator(`.workspace-list-entry[data-workspace-id="${workspaceId}"]`);
}

async function expandTerminal(page: Page, workspaceId: string, paneId: string) {
  const entry = workspaceEntry(page, workspaceId);
  await entry
    .getByRole("button", { name: "Expand items in Build", exact: true })
    .click();
  const items = entry.getByRole("list", {
    name: "Open items in Build",
    exact: true,
  });
  await expect(items).toBeVisible();
  return {
    entry,
    chip: entry.locator('.workspace-agent-chip[data-cli="codex"]'),
    leaf: items.locator(
      `.workspace-open-item[data-workspace-pane-id="${paneId}"]`,
    ),
  };
}

async function expectActivity(
  containers: Locator[],
  icons: Locator[],
  activity: "working" | "attention" | "error",
) {
  for (const container of containers)
    await expect(container).toHaveAttribute(
      "aria-description",
      new RegExp(
        {
          working: "Working",
          attention: "Needs input",
          error: "CLI error or usage limit reached",
        }[activity],
        "i",
      ),
    );
  for (const icon of icons) {
    await expect(icon).toHaveAttribute("data-activity", activity);
    await expect(icon.locator(".workspace-cli-activity-dot")).toHaveAttribute(
      "data-activity",
      activity,
    );
  }
}

async function expectNoActivity(containers: Locator[], icons: Locator[]) {
  for (const container of containers) {
    await expect(container).not.toHaveAttribute("data-activity");
    await expect(container).not.toHaveAttribute(
      "aria-description",
      /Working|Needs input|CLI error or usage limit reached/i,
    );
  }
  for (const icon of icons) {
    await expect(icon).not.toHaveAttribute("data-activity");
    await expect(icon.locator(".workspace-cli-activity-dot")).toHaveCount(0);
  }
}

test("Codex lifecycle status stays authoritative with fresh exhausted quota", async ({
  page,
}, testInfo) => {
  const { session, workspace, paneId } = setupWorkspace();
  await mockDesktop(page, false, session);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();

  const sessionId = await waitForTerminal(page, paneId);
  await setCodexContext(page, sessionId, codexPid);
  await expect
    .poll(() => terminalSnapshot(page, paneId))
    .toMatchObject({
      process: { cli: "codex", pid: codexPid },
    });

  await emitOsc(page, paneId, sessionId, "133;C");
  const codexStatus = (state: string, kind?: string) =>
    emitOsc(
      page,
      paneId,
      sessionId,
      `7501;state=${state}:app=codex${kind ? `:kind=${kind}` : ""}`,
      "\x1b\\",
    );
  await codexStatus("idle");
  await expect
    .poll(() => terminalSnapshot(page, paneId))
    .toMatchObject({
      activity: null,
      programStatus: { state: "idle", app: "codex" },
    });

  const { chip, leaf } = await expandTerminal(page, workspace.id, paneId);
  await expect(chip).toBeVisible();
  await expect(leaf).toBeVisible();
  const containers = [chip, leaf];
  const icons = [
    chip.locator(".workspace-cli-icon"),
    leaf.locator(".workspace-cli-icon"),
  ];
  const usageSummary = page.locator('.agent-usage-summary[data-cli="codex"]');

  await setQuotaAndRefresh(page, sessionId);
  await expect(usageSummary).toBeVisible();
  await expect(usageSummary.locator(".agent-usage-trigger-value")).toHaveText(
    "0%",
  );
  await expect(usageSummary).toHaveAttribute("data-tone", "critical");
  await expectNoActivity(containers, icons);

  await codexStatus("working");
  await expectActivity(containers, icons, "working");
  await setQuotaAndRefresh(page, sessionId);
  await expectActivity(containers, icons, "working");

  await codexStatus("blocked", "permission");
  await expectActivity(containers, icons, "attention");
  await setQuotaAndRefresh(page, sessionId);
  await expectActivity(containers, icons, "attention");

  await codexStatus("done");
  await expect
    .poll(() => terminalSnapshot(page, paneId))
    .toMatchObject({
      activity: null,
      programStatus: { state: "done", app: "codex" },
    });
  await expectNoActivity(containers, icons);
  await setQuotaAndRefresh(page, sessionId);
  await expectNoActivity(containers, icons);

  await codexStatus("error");
  await expectActivity(containers, icons, "error");
  await setQuotaAndRefresh(page, sessionId);
  await expectActivity(containers, icons, "error");

  await usageSummary.click();
  const usageMenu = page.locator(".agent-usage-menu");
  const usageWindows = usageMenu.locator(
    '.agent-usage-agent[aria-label="Codex usage 1"] .agent-usage-window',
  );
  await expect(usageWindows).toHaveCount(2);
  await expect(usageWindows.nth(0)).toContainText("0% remaining");
  await expect(usageWindows.nth(1)).toContainText("69% remaining");
  await page.keyboard.press("Escape");
  await expect(usageMenu).toBeHidden();

  await codexStatus("idle");
  await expect
    .poll(() => terminalSnapshot(page, paneId))
    .toMatchObject({
      activity: null,
      programStatus: { state: "idle", app: "codex" },
    });
  await expectNoActivity(containers, icons);
  await expect(usageSummary.locator(".agent-usage-trigger-value")).toHaveText(
    "0%",
  );
  await page.screenshot({
    path: testInfo.outputPath("codex-idle-zero-quota.png"),
  });

  const finalTerminal = await terminalSnapshot(page, paneId);
  expect(finalTerminal.sessionId).toBe(sessionId);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
});
