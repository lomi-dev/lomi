import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { newAgentTaskTab, newProject, newSession } from "../../src/model";
import { cliNames } from "../../src/cli-agents";
async function fixture(
  page: Page,
  options: {
    running?: boolean;
    twoCards?: boolean;
    grant?: boolean;
    empty?: boolean;
    legacy?: boolean;
    missing?: boolean;
    savedVersion4?: boolean;
    pi?: boolean;
    largeHistory?: boolean;
    stopAndContinueQualified?: boolean;
    accountRecovery?: "ownership_unknown" | "effects_review_required";
    failAccountRecovery?: boolean;
    unavailableNative?: boolean;
  } = {},
) {
  const project = newProject("/project", "local:bash");
  const first = newAgentTaskTab(
      options.legacy ? "legacy:old" : "task",
      "Task first",
    ),
    second = newAgentTaskTab(
      options.legacy ? "legacy:old" : "task",
      "Task second",
    );
  if (!options.empty) {
    project.workspaces[0].tabs = options.twoCards ? [first, second] : [first];
    project.workspaces[0].activeTabId = first.id;
  }
  if (options.savedVersion4)
    project.workspaces[0].tabs = project.workspaces[0].tabs.map((tab) =>
      tab.type === "agent-task"
        ? ({
            type: "cli-agent",
            id: tab.id,
            title: tab.title,
            runId: "old",
          } as any)
        : tab,
    );
  await mockDesktop(page, false, {
    ...newSession(),
    ...(options.savedVersion4 ? { version: 4 } : {}),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.addInitScript(
    ({ options, names }) => {
      const w = window as any;
      const mock = w.__nativeTest;
      const accounts = {
        schema: 1,
        revision: 1,
        accounts: ["a", "b", "c"].map((id) => ({
          accountId: id,
          cli: options.pi ? "pi" : "codex",
          label: `Account ${id.toUpperCase()}`,
          enabled: true,
          revision: 1,
          authRevision: 1,
          authState: "verified",
          availabilityReason: null,
          acceptedVersion: options.pi ? "1.0.1" : "0.160.0",
          ...(id === "a" && options.accountRecovery
            ? {
                recovery: {
                  state: options.accountRecovery,
                  reason: "The previous native operation requires recovery.",
                  operationIds: ["interrupted-helper"],
                  recoverable:
                    options.accountRecovery === "effects_review_required",
                  requiresVerifiedBootChange:
                    options.accountRecovery === "ownership_unknown",
                },
              }
            : {}),
        })),
        capabilities: Object.keys(names).map((cli) => ({
          cli,
          accountTerminal: !options.unavailableNative,
          managedExecution: cli === (options.pi ? "pi" : "codex"),
          versions: ["0.160.0"],
          crossAccountNativeResume: false,
          reviewedTransfer: !!options.pi && cli === "pi",
          stopAndContinueQualified: !!options.stopAndContinueQualified,
          stopAndContinueReason: options.stopAndContinueQualified
            ? null
            : "This CLI’s background work cannot yet be verified as stopped.",
          reason: "Local fixture qualification; real accounts pending.",
        })),
      };
      const task = {
        taskId: options.legacy ? "legacy:old" : "task",
        cwd: "/project",
        title: "Task first",
        cli: options.pi ? "pi" : "codex",
        model: options.pi ? "openai/gpt-5" : "gpt-5",
        reasoningEffort: null,
        revision: 1,
        historyRevision: 1,
        generation: 1,
        state: options.running ? "running" : "completed",
        nextAccountId: "a",
        activeAccountId: options.running ? "a" : null,
        activeAttemptId: options.running ? "attempt-a" : null,
        statusMessage: "Ready",
        attempts: [
          {
            attemptId: "attempt-a",
            operationId: "initial",
            accountId: "a",
            authRevision: 1,
            generation: 1,
            input: "Initial task",
            continuationMethod: "fresh",
            state: options.running ? "running" : "completed",
            output: "A result",
            nativeRef: null,
            version: options.pi ? "1.0.1" : "0.160.0",
            effectsState: "settled",
          },
        ],
        history: [
          {
            sequence: 1,
            attemptId: "attempt-a",
            accountId: "a",
            authRevision: 1,
            kind: "assistant",
            state: "completed",
            content: options.largeHistory
              ? "Entire observed history ".repeat(8000)
              : "A result",
          },
        ],
        grants: [
          { accountId: "a", authRevision: 1, revision: 1 },
          ...(options.grant
            ? [
                { accountId: "b", authRevision: 1, revision: 1 },
                { accountId: "c", authRevision: 1, revision: 1 },
              ]
            : []),
        ],
        switches: [] as any[],
      };
      mock.permissions = [];
      mock.freezeComplete = true;
      mock.accountSnapshot = accounts;
      mock.task = task;
      mock.failTarget = "";
      mock.failAccountRecovery = !!options.failAccountRecovery;
      mock.agentCalls = [];
      mock.switchDelay = 100;
      mock.resync = () =>
        mock.emitEvent("agent-runtime-changed", {
          revision: ++accounts.revision,
          taskId: task.taskId,
        });
      w.__agentInvoke = async (command: string, args: any) => {
        if (command === "agent_accounts_snapshot")
          return structuredClone(accounts);
        if (command === "agent_tasks_snapshot")
          return {
            schema: 1,
            revision: accounts.revision,
            tasks:
              (options.empty && !mock.created) ||
              (options.missing && !mock.imported)
                ? []
                : [
                    structuredClone(task),
                    ...(mock.otherTask
                      ? [structuredClone(mock.otherTask)]
                      : []),
                  ],
          };
        if (command === "agent_task_snapshot") return structuredClone(task);
        if (command === "agent_legacy_migration_preview") {
          mock.previewSession = JSON.parse(
            localStorage.getItem("test-session") ?? "null",
          );
          return {
            migrationId: "fixture-migration",
            digest: "fixture-digest",
            accountCount: 3,
            taskCount: 1,
            missingCount: 0,
            phase: "staged",
          };
        }
        if (command === "agent_legacy_migration_apply") {
          if (
            args.request.migrationId !== "fixture-migration" ||
            args.request.expectedDigest !== "fixture-digest"
          )
            throw new Error("Archive changed");
          mock.imported = true;
          task.state = "archived";
          task.statusMessage =
            "Imported task archive. Review before continuation.";
          task.revision++;
          await mock.resync();
          return {
            migrationId: "fixture-migration",
            digest: "fixture-digest",
            accountCount: 3,
            taskCount: 1,
            missingCount: 0,
            phase: "committed",
          };
        }
        if (command === "agent_legacy_migration_rollback") {
          if (
            args.request.migrationId !== "fixture-migration" ||
            args.request.expectedDigest !== "fixture-digest"
          )
            throw new Error("Archive changed");
          mock.rollbackRequest = structuredClone(args.request);
          return { exportId: "f".repeat(32) };
        }
        if (command === "agent_legacy_migration_export_open") {
          if (args.exportId !== "f".repeat(32))
            throw new Error("Unknown export identity");
          mock.openedExport = args.exportId;
          return;
        }
        if (command === "agent_permission_snapshot")
          return structuredClone(mock.permissions);
        mock.agentCalls.push({ command, args: structuredClone(args) });
        if (
          command === "agent_task_prepare_close" ||
          command === "agent_task_close_release"
        )
          return;
        if (command === "agent_permission_freeze")
          return { editorFreezeToken: "retained-freeze" };
        if (command === "agent_permission_freeze_complete")
          return { complete: mock.freezeComplete };
        if (command === "agent_permission_reply") {
          mock.permissions = [];
          await mock.resync();
          return structuredClone(task);
        }
        const r = args.request;
        if (command.startsWith("agent_account_")) {
          if (command === "agent_account_create")
            accounts.accounts.push({
              accountId: `added-${accounts.accounts.length}`,
              cli: r.cli,
              label: r.label,
              enabled: true,
              revision: 1,
              authRevision: 1,
              authState: "unverified",
              availabilityReason: null,
              acceptedVersion: null,
            } as any);
          else {
            const a = accounts.accounts.find(
              (a: any) => a.accountId === r.accountId,
            )!;
            if (a.revision !== r.expectedRevision)
              throw new Error("Account revision changed");
            if (command === "agent_account_update") {
              if (r.label !== null) a.label = r.label;
              if (r.enabled !== null) a.enabled = r.enabled;
              a.revision++;
            }
            if (command === "agent_account_remove")
              accounts.accounts = accounts.accounts.filter(
                (a: any) => a.accountId !== r.accountId,
              );
            if (command === "agent_account_verify") {
              a.authState = "verified";
              a.revision++;
            }
            if (command === "agent_account_recover") {
              if (mock.failAccountRecovery)
                throw new Error(
                  "The interrupted operation could not be settled.",
                );
              if (
                !a.recovery?.recoverable ||
                a.recovery.requiresVerifiedBootChange ||
                !r.acknowledgeEffects
              )
                throw new Error(
                  "The previous native ownership remains unknown.",
                );
              delete a.recovery;
              a.authRevision++;
              a.authState = "unverified";
              a.revision++;
            }
          }
          accounts.revision++;
          return structuredClone(accounts);
        }
        if (command === "agent_task_create") {
          mock.created = true;
          Object.assign(task, {
            title: r.title,
            model: r.model,
            nextAccountId: r.accountId,
            state: "idle",
            attempts: [],
            history: [],
            grants: [],
            historyRevision: 0,
          });
          return structuredClone(task);
        }
        if (task.revision !== r.expectedRevision)
          throw new Error("Task revision changed");
        const account =
          r.accountId &&
          accounts.accounts.find((a: any) => a.accountId === r.accountId);
        if (account && r.authRevision !== account.authRevision)
          throw new Error(
            "The selected account login changed; review its history access again.",
          );
        if (
          command === "agent_task_transfer_preview" ||
          command === "agent_task_transfer_apply"
        ) {
          if (r.expectedHistoryRevision !== task.historyRevision)
            throw new Error("History changed");
          if (
            !task.grants.some(
              (grant: any) =>
                grant.accountId === r.accountId &&
                grant.authRevision === r.authRevision,
            )
          )
            throw new Error("Review target history access first");
          if (command === "agent_task_transfer_preview")
            return {
              digest: "d".repeat(64),
              bytes: 270000,
              taskId: task.taskId,
              accountId: r.accountId,
              authRevision: r.authRevision,
              coverage: task.historyRevision,
              sourceAttemptId: "attempt-a",
            };
          if (r.expectedDigest !== "d".repeat(64) || r.expectedBytes !== 270000)
            throw new Error("Reviewed native session changed");
          task.nextAccountId = r.accountId;
          task.switches.push({
            operationId: r.operationId,
            sourceAttemptId: "attempt-a",
            accountId: r.accountId,
            authRevision: r.authRevision,
            historyRevision: task.historyRevision,
            mode: "reviewed_transfer",
            phase: "prepared",
            continuationMethod: "reviewed_transfer",
            coverage: task.historyRevision,
            budgetBytes: 131072,
            contextDigest: r.expectedDigest,
            reason: null,
            stopSupervisionQualified: true,
          });
        }
        if (command === "agent_task_update_history_grant") {
          if (r.expectedHistoryRevision !== task.historyRevision)
            throw new Error("History changed");
          task.grants.push({
            accountId: r.accountId,
            authRevision: r.authRevision,
            revision: task.revision,
          });
        }
        if (command === "agent_task_prepare_switch") {
          task.nextAccountId = r.accountId;
          task.switches.push({
            operationId: r.operationId,
            sourceAttemptId: task.activeAttemptId,
            accountId: r.accountId,
            authRevision: r.authRevision,
            historyRevision: task.historyRevision,
            mode: r.mode,
            phase: r.mode === "next_turn" ? "prepared" : "stopping_source",
            continuationMethod: "handoff",
            coverage: task.history.length,
            budgetBytes: 100000,
            contextDigest: "fixture",
            reason: null,
          });
          if (r.mode === "stop_and_continue") {
            task.state = "stopping";
            setTimeout(() => {
              task.state = "prepared";
              task.activeAccountId = null;
              task.activeAttemptId = null;
              task.switches.at(-1)!.phase = "prepared";
              task.revision++;
              void mock.resync();
            }, mock.switchDelay);
          }
        }
        if (
          command === "agent_task_send" ||
          command === "agent_task_commit_switch"
        ) {
          if (mock.failTarget === r.accountId)
            throw new Error(
              "Selected account B could not start; history prepared, work not started.",
            );
          if (
            command === "agent_task_commit_switch" &&
            task.state !== "prepared"
          )
            throw new Error("Source has not settled");
          const id = `attempt-${++task.generation}`;
          task.nextAccountId = r.accountId;
          task.state = "completed";
          task.activeAccountId = null;
          task.activeAttemptId = null;
          task.historyRevision++;
          task.attempts.push({
            attemptId: id,
            operationId: r.operationId,
            accountId: r.accountId,
            authRevision: r.authRevision,
            generation: task.generation,
            input: r.text || "Continue",
            continuationMethod: "handoff",
            state: "completed",
            output: `${r.accountId} result`,
            nativeRef: null,
            version: "0.160.0",
            effectsState: "settled",
          });
          task.history.push({
            sequence: task.historyRevision,
            attemptId: id,
            accountId: r.accountId,
            authRevision: r.authRevision,
            kind: "assistant",
            state: "completed",
            content: `${r.accountId} result`,
          });
        }
        if (command === "agent_task_stop") {
          task.state = "stopped";
          task.activeAccountId = null;
          task.activeAttemptId = null;
        }
        task.revision++;
        await mock.resync();
        return structuredClone(task);
      };
    },
    { options, names: cliNames },
  );
  await page.goto("/");
  if (!options.empty && !options.missing)
    await expect(page.getByLabel("Next account")).toBeVisible();
  return page.locator(".agent-task-pane").filter({ visible: true });
}
async function calls(page: Page, command: string) {
  return page.evaluate(
    (command) =>
      (window as any).__nativeTest.agentCalls.filter(
        (call: any) => call.command === command,
      ),
    command,
  );
}
test("Send and Continue target B, A, C in one task with unknown quota", async ({
  page,
}) => {
  const pane = await fixture(page, { grant: true });
  for (const [id, continuing] of [
    ["b", false],
    ["a", true],
    ["c", false],
    ["a", true],
  ] as const) {
    await pane.getByLabel("Next account").selectOption(id);
    if (!continuing)
      await pane.getByLabel("Message", { exact: true }).fill(`Step ${id}`);
    await pane
      .getByRole("button", {
        name: continuing ? "Continue on selected account" : "Send",
        exact: true,
      })
      .click();
    await expect(
      pane.getByRole("button", { name: "Send", exact: true }),
    ).toBeDisabled();
  }
  expect(
    (await calls(page, "agent_task_send")).map((call: any) => [
      call.args.request.accountId,
      call.args.request.authRevision,
      call.args.request.continue,
    ]),
  ).toEqual([
    ["b", 1, false],
    ["a", 1, true],
    ["c", 1, false],
    ["a", 1, true],
  ]);
  await pane.locator(".agent-task-content").evaluate((element) => {
    element.scrollTop = 0;
  });
  await page.screenshot({ path: "test-results/agent-task-switch.png" });
});
for (const state of ["recovery_required", "delivery_uncertain"])
  test(`${state} exposes an explicit stop for the previous native process`, async ({
    page,
  }) => {
    await fixture(page, { running: true, grant: true });
    await page.evaluate(async (state) => {
      const mock = (window as any).__nativeTest;
      mock.task.state = state;
      mock.task.revision++;
      await mock.resync();
    }, state);
    await page
      .getByRole("button", { name: "Stop previous process", exact: true })
      .click();
    await expect
      .poll(async () => (await calls(page, "agent_task_stop")).length)
      .toBe(1);
    await expect(
      page.getByRole("button", { name: "Stop previous process", exact: true }),
    ).toHaveCount(0);
    expect(await calls(page, "agent_task_send")).toHaveLength(0);
  });

test("provenance-unknown imported history remains readable without attempt records", async ({
  page,
}) => {
  await fixture(page, { legacy: true });
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.task.cli = null;
    mock.task.availabilityReason = "Original CLI provenance is unavailable.";
    mock.task.state = "archived";
    mock.task.attempts = [];
    mock.task.history = [
      {
        sequence: 1,
        attemptId: "original-run",
        accountId: "original-account",
        authRevision: 0,
        kind: "legacy_record",
        state: "archived",
        content: {
          input: "Original saved input",
          output: "Entire original retained output",
          evidence: { retained: true },
        },
      },
    ];
    mock.task.revision++;
    await mock.resync();
  });
  await expect(
    page.getByRole("heading", { name: "Archived history", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Unknown CLI", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Original CLI provenance is unavailable.", { exact: true }),
  ).toBeVisible();
  const history = page.locator(".agent-task-history");
  await expect(history).toContainText("original-account");
  await expect(history.locator("pre")).toContainText("Original saved input");
  await expect(history.locator("pre")).toContainText(
    "Entire original retained output",
  );
  await expect(history.locator("pre")).toContainText('"retained": true');
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
});

test("history grant is reviewed once and survives later history", async ({
  page,
}) => {
  const pane = await fixture(page);
  await pane.getByLabel("Next account").selectOption("b");
  await pane.getByLabel("Message", { exact: true }).fill("B next");
  await pane.getByRole("button", { name: "Send", exact: true }).click();
  const review = page.getByRole("dialog", {
    name: "Allow account access to task history?",
  });
  await expect(review).toContainText("Account B");
  await review.getByRole("button", { name: "Allow and continue" }).click();
  await expect(review).toHaveCount(0);
  await pane
    .getByRole("button", { name: "Continue on selected account" })
    .click();
  expect(await calls(page, "agent_task_update_history_grant")).toHaveLength(1);
  expect(await calls(page, "agent_task_send")).toHaveLength(2);
});
test("failed B retains selected target and exact shared draft without fallback", async ({
  page,
}) => {
  const pane = await fixture(page, { grant: true });
  await page.evaluate(() => ((window as any).__nativeTest.failTarget = "b"));
  await pane.getByLabel("Next account").selectOption("b");
  await pane.getByLabel("Message", { exact: true }).fill("Keep exact draft");
  await pane.getByRole("button", { name: "Send", exact: true }).click();
  await expect(pane.getByRole("alert")).toContainText("could not start");
  await expect(pane.getByLabel("Next account")).toHaveValue("b");
  await expect(pane.getByLabel("Message", { exact: true })).toHaveValue(
    "Keep exact draft",
  );
  expect(
    (await calls(page, "agent_task_send")).map(
      (call: any) => call.args.request.accountId,
    ),
  ).toEqual(["b"]);
});
test("unqualified active stop-and-continue shows the reason and retains manual Stop and next selection", async ({
  page,
}) => {
  const pane = await fixture(page, { running: true, grant: true });
  await pane.getByLabel("Next account").selectOption("b");
  await expect(
    pane.getByRole("button", {
      name: "Stop and continue on selected account",
      exact: true,
    }),
  ).toBeDisabled();
  await expect(
    pane.getByRole("status").filter({
      hasText: "This CLI’s background work cannot yet be verified as stopped.",
    }),
  ).toBeVisible();
  await expect(
    pane.getByRole("button", { name: "Stop", exact: true }),
  ).toBeEnabled();
  await pane
    .getByRole("button", { name: "Use after this turn", exact: true })
    .click();
  await expect(pane.getByLabel("Next account")).toHaveValue("b");
  const switches = await calls(page, "agent_task_prepare_switch");
  expect(switches).toHaveLength(1);
  expect(switches[0].args.request.mode).toBe("next_turn");
  expect(await calls(page, "agent_task_commit_switch")).toHaveLength(0);
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
});

test("next-turn preserves active A and stop-and-continue waits for source settlement", async ({
  page,
}) => {
  const pane = await fixture(page, {
    running: true,
    grant: true,
    stopAndContinueQualified: true,
  });
  await pane.getByLabel("Next account").selectOption("b");
  await pane.getByRole("button", { name: "Use after this turn" }).click();
  await expect(pane.locator("dd").first()).toHaveText("Account A");
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
  await page.evaluate(() => ((window as any).__nativeTest.switchDelay = 700));
  await pane
    .getByRole("button", { name: "Stop and continue on selected account" })
    .click();
  await page.waitForTimeout(100);
  expect(await calls(page, "agent_task_commit_switch")).toHaveLength(0);
  await expect
    .poll(async () => (await calls(page, "agent_task_commit_switch")).length)
    .toBe(1);
  expect(
    (await calls(page, "agent_task_commit_switch"))[0].args.request.accountId,
  ).toBe("b");
});
test("relogin during review rejects stale target while preserving draft", async ({
  page,
}) => {
  const pane = await fixture(page);
  await pane.getByLabel("Next account").selectOption("b");
  await pane.getByLabel("Message", { exact: true }).fill("Retain me");
  await pane.getByRole("button", { name: "Send", exact: true }).click();
  const review = page.getByRole("dialog", {
    name: "Allow account access to task history?",
  });
  await page.evaluate(() => {
    (window as any).__nativeTest.accountSnapshot.accounts[1].authRevision++;
  });
  await review.getByRole("button", { name: "Allow and continue" }).click();
  await expect(review.getByRole("alert")).toContainText("login changed");
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
  await review.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(pane.getByLabel("Message", { exact: true })).toHaveValue(
    "Retain me",
  );
  await expect(pane.getByLabel("Next account")).toHaveValue("b");
});
test("two task cards share draft and target; closing one never drains shared owner", async ({
  page,
}) => {
  const pane = await fixture(page, { twoCards: true, grant: true });
  await pane.getByLabel("Message", { exact: true }).fill("Shared draft");
  await pane.getByLabel("Next account").selectOption("b");
  await page.getByRole("tab", { name: "Task second" }).click();
  const second = page.locator(".agent-task-pane").filter({ visible: true });
  await expect(second.getByLabel("Message", { exact: true })).toHaveValue(
    "Shared draft",
  );
  await expect(second.getByLabel("Next account")).toHaveValue("b");
  await second.getByRole("button", { name: "Close task" }).click();
  expect(await calls(page, "agent_task_prepare_close")).toHaveLength(0);
  await page
    .locator(".agent-task-pane")
    .filter({ visible: true })
    .getByRole("button", { name: "Close task" })
    .click();
  await expect
    .poll(async () => (await calls(page, "agent_task_prepare_close")).length)
    .toBe(1);
});
test("reconnect and remount resync without dispatch; removed B stays selected", async ({
  page,
}) => {
  const pane = await fixture(page, { grant: true });
  await pane.getByLabel("Next account").selectOption("b");
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.accountSnapshot.accounts = mock.accountSnapshot.accounts.filter(
      (a: any) => a.accountId !== "b",
    );
    await mock.resync();
  });
  await expect(pane.getByLabel("Next account")).toHaveValue("b");
  await expect(
    pane.getByRole("button", { name: "Continue on selected account" }),
  ).toBeDisabled();
  await page.reload();
  await expect(page.getByLabel("Next account")).toBeVisible();
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
});
test("task creation selects explicit account/model and creates without dispatch", async ({
  page,
}) => {
  await fixture(page, { empty: true });
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Agents", exact: true }).click();
  await page.getByRole("button", { name: "New CLI task", exact: true }).click();
  const dialog = page.locator("dialog[open]").filter({
    has: page.getByRole("heading", { name: "New CLI task", exact: true }),
  });
  await expect(
    page.getByRole("heading", { name: "New CLI task", exact: true }),
  ).toBeVisible();
  await dialog.getByLabel("Account", { exact: true }).selectOption("b");
  await dialog.getByLabel("Task title").fill("New coding task");
  await dialog.getByLabel("Model", { exact: true }).fill("gpt-5");
  const notice = dialog.getByRole("note", {
    name: "CLI background work limitation",
  });
  await expect(notice).toBeVisible();
  await expect(notice).toContainText(
    "Completed local work can release its account",
  );
  await expect(notice).toContainText(
    "Acknowledging effects alone cannot release the lock",
  );
  await dialog.getByRole("button", { name: "Create task" }).click();
  await expect(page.locator(".agent-task-pane")).toBeVisible();
  expect(
    (await calls(page, "agent_task_create"))[0].args.request.accountId,
  ).toBe("b");
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
});
test("restricted native task explains conditional recovery before sending", async ({
  page,
}) => {
  const pane = await fixture(page, { grant: true });
  await pane
    .getByLabel("Message", { exact: true })
    .fill("Inspect this project");
  const notice = pane.getByRole("note", {
    name: "CLI background work limitation",
  });
  await expect(notice).toBeVisible();
  await expect(notice).toContainText(
    "account switching is unavailable while work is running",
  );
  await expect(notice).toContainText(
    "Completed local work can release its account",
  );
  await expect(notice).toContainText(
    "Unknown tool or remote effects keep the task and account protected",
  );
  await expect(notice).toContainText(
    "Acknowledging effects alone cannot release the lock",
  );
  await expect(
    pane.getByRole("button", { name: "Send", exact: true }),
  ).toBeEnabled();
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
});

test("unavailable native accounts cannot start verification or login", async ({
  page,
}) => {
  await fixture(page, { empty: true, unavailableNative: true });
  await page.goto("/?window=settings");
  await page
    .getByRole("button", { name: "Agent control", exact: true })
    .click();
  await page.getByRole("tab", { name: "CLI Accounts" }).click();
  const account = page.getByRole("region", { name: "Account A", exact: true });
  await expect(
    account.getByRole("button", { name: "Verify account" }),
  ).toBeDisabled();
  await expect(
    account.getByRole("button", { name: "Open login terminal" }),
  ).toBeDisabled();
  expect(await calls(page, "agent_account_verify")).toHaveLength(0);
  expect(await calls(page, "plugin:event|emit_to")).toHaveLength(0);
});

test("CLI Accounts show capabilities and perform native account CRUD", async ({
  page,
}) => {
  await fixture(page, { empty: true });
  await page.goto("/?window=settings");
  await page
    .getByRole("button", { name: "Agent control", exact: true })
    .click();
  await page.getByRole("tab", { name: "CLI Accounts" }).click();
  await expect(page.getByRole("table")).toBeVisible();
  await expect(page.getByRole("table").locator("tbody tr")).toHaveCount(
    Object.keys(cliNames).length,
  );
  await page.getByLabel("Account label", { exact: true }).fill("New account");
  await page.getByRole("button", { name: "Add account", exact: true }).click();
  const account = page.getByRole("region", {
    name: "New account",
    exact: true,
  });
  await expect(account).toBeVisible();
  await account.getByRole("button", { name: "Verify account" }).click();
  await expect(account.getByRole("status")).toContainText("verified");
  await account.getByRole("button", { name: "Open login terminal" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.find(
            (call: any) => call.command === "plugin:event|emit_to",
          )?.args.target,
      ),
    )
    .toEqual({ kind: "Webview", label: "main" });
  await account.getByRole("button", { name: "Remove account" }).click();
  const review = page.getByRole("dialog", { name: "Remove CLI account?" });
  await review
    .getByRole("button", { name: "Remove account", exact: true })
    .click();
  await expect(account).toHaveCount(0);
  await page.evaluate(() => {
    for (const element of document.querySelectorAll("*"))
      if (element instanceof HTMLElement) element.scrollTop = 0;
  });
  await page.screenshot({ path: "test-results/agent-accounts-settings.png" });
});

test("unknown native helper ownership protects account actions and has no UI override", async ({
  page,
}) => {
  await fixture(page, { empty: true, accountRecovery: "ownership_unknown" });
  await page.goto("/?window=settings");
  await page
    .getByRole("button", { name: "Agent control", exact: true })
    .click();
  await page.getByRole("tab", { name: "CLI Accounts" }).click();
  const account = page.getByRole("region", { name: "Account A", exact: true });
  await expect(account.getByRole("alert")).toContainText("requires recovery");
  await expect(account).toContainText("Restart your computer");
  for (const name of [
    "Review interrupted operation",
    "Open login terminal",
    "Verify account",
    "Remove account",
    "Rename",
    "Disable",
  ])
    await expect(
      account.getByRole("button", { name, exact: true }),
    ).toBeDisabled();
  expect(await calls(page, "agent_account_recover")).toHaveLength(0);
  expect(await calls(page, "agent_account_verify")).toHaveLength(0);
});

test("account recovery requires effect review and retains a failed review for explicit retry", async ({
  page,
}) => {
  await fixture(page, {
    empty: true,
    accountRecovery: "effects_review_required",
    failAccountRecovery: true,
  });
  await page.goto("/?window=settings");
  await page
    .getByRole("button", { name: "Agent control", exact: true })
    .click();
  await page.getByRole("tab", { name: "CLI Accounts" }).click();
  const account = page.getByRole("region", { name: "Account A", exact: true });
  await account
    .getByRole("button", { name: "Review interrupted operation" })
    .click();
  const review = page.getByRole("dialog", { name: "Review Account A" });
  await expect(
    review.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  const complete = review.getByRole("button", { name: "Complete recovery" });
  await expect(complete).toBeDisabled();
  const effects = review.getByRole("checkbox", {
    name: "I reviewed the account and project effects",
  });
  await effects.check();
  await complete.click();
  await expect(review.getByRole("alert")).toContainText("could not be settled");
  await expect(effects).toBeChecked();
  await expect(review).toBeVisible();
  await page.setViewportSize({ width: 800, height: 420 });
  await review.screenshot({
    path: "test-results/agent-account-recovery-review.png",
  });
  await page.evaluate(() => {
    (window as any).__nativeTest.failAccountRecovery = false;
  });
  await complete.click();
  await expect(review).toHaveCount(0);
  await expect(account.getByRole("status")).toContainText("unverified");
  await expect(account.getByRole("status")).toContainText("login revision 2");
  await expect(
    account.getByRole("button", { name: "Verify account" }),
  ).toBeEnabled();
  const recoveries = await calls(page, "agent_account_recover");
  expect(recoveries).toHaveLength(2);
  expect(
    new Set(recoveries.map((call: any) => call.args.request.operationId)).size,
  ).toBe(2);
  for (const call of recoveries)
    expect(call.args.request).toMatchObject({
      accountId: "a",
      expectedRevision: 1,
      acknowledgeEffects: true,
    });
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
});

test("a target account with unresolved helper ownership keeps the draft and blocks dispatch", async ({
  page,
}) => {
  const pane = await fixture(page, {
    grant: true,
    accountRecovery: "ownership_unknown",
  });
  const draft = pane.getByLabel("Message", { exact: true });
  await draft.fill("Preserve this exact message");
  for (const name of [
    "Send",
    "Continue on selected account",
    "Use after this turn",
  ])
    await expect(
      pane.getByRole("button", { name, exact: true }),
    ).toBeDisabled();
  await expect(pane.getByRole("alert")).toContainText("requires recovery");
  await expect(draft).toHaveValue("Preserve this exact message");
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
  expect(await calls(page, "agent_task_prepare_switch")).toHaveLength(0);
});

test("reviewed Pi transfer includes oversized history and prepares B before explicit Send", async ({
  page,
}) => {
  const pane = await fixture(page, { pi: true, largeHistory: true });
  const notice = pane.getByRole("note", {
    name: "CLI background work limitation",
  });
  await expect(notice).toContainText(
    "Managed Pi runs with extensions disabled",
  );
  await expect(notice).toContainText("even after a text-only run");
  await expect(notice).toContainText("restarting your computer");
  await expect(notice).toContainText("explicitly recovering the task");
  await expect(notice).toContainText(
    "Acknowledging effects alone cannot release the lock",
  );
  await pane.getByLabel("Next account").selectOption("b");
  await pane
    .getByLabel("Message", { exact: true })
    .fill("New work after review");
  await pane
    .getByRole("button", {
      name: "Review native Pi history transfer",
      exact: true,
    })
    .click();
  const grant = page.getByRole("dialog", {
    name: "Allow account access to task history?",
  });
  await grant
    .getByRole("button", { name: "Allow history access", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Review native Pi history transfer",
    exact: true,
  });
  await expect(review).toContainText("Account B");
  await expect(review).toContainText("history through record 1");
  await expect(review).toContainText("270,000 bytes");
  await review
    .getByText("Reviewed history fingerprint", { exact: true })
    .click();
  await expect(review.locator("pre")).toContainText("d".repeat(64));
  await page.screenshot({ path: "test-results/agent-pi-transfer-review.png" });
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
  expect(await calls(page, "agent_task_transfer_apply")).toHaveLength(0);
  await review
    .getByRole("button", { name: "Apply reviewed transfer", exact: true })
    .click();
  await expect(review).toHaveCount(0);
  const applied = (await calls(page, "agent_task_transfer_apply"))[0].args
    .request;
  expect(applied).toMatchObject({
    taskId: "task",
    accountId: "b",
    authRevision: 1,
    expectedRevision: 2,
    expectedHistoryRevision: 1,
    expectedDigest: "d".repeat(64),
    expectedBytes: 270000,
  });
  await expect(pane.getByLabel("Message", { exact: true })).toHaveValue(
    "New work after review",
  );
  await expect(pane.getByLabel("Next account")).toHaveValue("b");
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
  expect(await calls(page, "agent_task_update_history_grant")).toHaveLength(1);
  expect(
    await page.evaluate(
      () => JSON.stringify((window as any).__nativeTest.task.history).length,
    ),
  ).toBeGreaterThan(131072);
  await pane.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(async () => (await calls(page, "agent_task_send")).length)
    .toBe(1);
  expect((await calls(page, "agent_task_send"))[0].args.request).toMatchObject({
    accountId: "b",
    authRevision: 1,
    text: "New work after review",
  });
});

for (const change of ["history", "login"])
  test(`Pi transfer refuses changed ${change} while retaining B and the draft`, async ({
    page,
  }) => {
    const pane = await fixture(page, { pi: true, grant: true });
    await pane.getByLabel("Next account").selectOption("b");
    await pane.getByLabel("Message", { exact: true }).fill("Retained Pi draft");
    await pane
      .getByRole("button", {
        name: "Review native Pi history transfer",
        exact: true,
      })
      .click();
    const review = page.getByRole("dialog", {
      name: "Review native Pi history transfer",
      exact: true,
    });
    await expect(review).toBeVisible();
    await page.evaluate(async (change) => {
      const mock = (window as any).__nativeTest;
      if (change === "history") {
        mock.task.historyRevision++;
        mock.task.revision++;
      } else
        mock.accountSnapshot.accounts.find(
          (account: any) => account.accountId === "b",
        ).authRevision++;
      await mock.resync();
    }, change);
    await review
      .getByRole("button", { name: "Apply reviewed transfer", exact: true })
      .click();
    await expect(review.getByRole("alert")).toContainText(
      change === "history" ? "Task revision changed" : "login changed",
    );
    await review.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(pane.getByLabel("Message", { exact: true })).toHaveValue(
      "Retained Pi draft",
    );
    await expect(pane.getByLabel("Next account")).toHaveValue("b");
    expect(await calls(page, "agent_task_send")).toHaveLength(0);
    expect(await calls(page, "agent_task_commit_switch")).toHaveLength(0);
    if (change === "login")
      expect(await calls(page, "agent_task_transfer_apply")).toHaveLength(0);
  });

for (const family of [
  {
    name: "Codex",
    choices: ["accept", "decline", "cancel"],
    allow: "accept",
    deny: "decline",
    allowLabel: "accept",
    denyLabel: "decline",
    raw: {},
  },
  {
    name: "Claude",
    choices: ["allow", "deny"],
    allow: "allow",
    deny: "deny",
    allowLabel: "allow",
    denyLabel: "deny",
    raw: {},
  },
  {
    name: "Grok",
    choices: [
      { optionId: "yes", name: "Allow", kind: "allow_once" },
      { optionId: "no", name: "Deny", kind: "reject_once" },
    ],
    allow: "yes",
    deny: "no",
    allowLabel: "Allow",
    denyLabel: "Deny",
    raw: {},
  },
  {
    name: "Kimi",
    choices: ["approved", "rejected", "cancelled"],
    allow: "approved",
    deny: "rejected",
    allowLabel: "approved",
    denyLabel: "rejected",
    raw: {},
  },
  {
    name: "Kilo",
    choices: ["once", "always", "reject"],
    allow: "once",
    deny: "reject",
    allowLabel: "once",
    denyLabel: "reject",
    raw: {},
  },
  {
    name: "OpenCode",
    choices: ["once", "always", "reject"],
    allow: "once",
    deny: "reject",
    allowLabel: "once",
    denyLabel: "reject",
    raw: {},
  },
  {
    name: "Pi confirm",
    choices: [],
    allow: true,
    deny: false,
    allowLabel: "Confirm",
    denyLabel: "Cancel",
    raw: { type: "extension_ui_request", method: "confirm" },
  },
  {
    name: "Pi input",
    choices: [],
    allow: "Typed response",
    deny: null,
    allowLabel: "Submit response",
    denyLabel: "Cancel",
    raw: { type: "extension_ui_request", method: "input", title: "Response" },
  },
  {
    name: "Pi editor",
    choices: [],
    allow: "Typed response",
    deny: null,
    allowLabel: "Submit response",
    denyLabel: "Cancel",
    raw: { type: "extension_ui_request", method: "editor", title: "Response" },
  },
  ...["cancel", "reject"].map((value) => ({
    name: `Pi select ${value}`,
    choices: ["cancel", "reject"],
    allow: value,
    deny: null,
    allowLabel: value,
    denyLabel: "Cancel",
    raw: {
      type: "extension_ui_request",
      method: "select",
      options: ["cancel", "reject"],
    },
  })),
])
  test(`${family.name} permission controls send exact native allow and deny values`, async ({
    page,
  }) => {
    await fixture(page, { grant: true });
    for (const allow of [true, false]) {
      await page.evaluate(
        async ({ family, allow }) => {
          const mock = (window as any).__nativeTest;
          mock.permissions = [
            {
              taskId: "task",
              attemptId: "attempt-a",
              generation: 1,
              approvalToken: `approval-${allow}`,
              projectRoot: "/project",
              permission: {
                requestId: "native-id",
                sessionId: "native-session",
                turnId: "turn",
                toolId: "tool",
                choices: family.choices,
                raw: family.raw,
              },
            },
          ];
          await mock.resync();
        },
        { family, allow },
      );
      const review = page.getByRole("dialog", {
        name: "Review native CLI permission",
      });
      await expect(review).toBeVisible();
      if (allow && ["Pi input", "Pi editor"].includes(family.name))
        await review
          .getByLabel("Response", { exact: true })
          .fill("Typed response");
      await review
        .getByRole("button", {
          name: allow ? family.allowLabel : family.denyLabel,
          exact: true,
        })
        .click();
      await expect(review).toHaveCount(0);
      const request = (await calls(page, "agent_permission_reply")).at(-1).args
        .request;
      expect(request.choice).toEqual(allow ? family.allow : family.deny);
      expect(request.allow).toBe(allow);
      expect(request.attemptId).toBe("attempt-a");
      expect(request.generation).toBe(1);
      expect(request.editorFreezeToken).toBe(allow ? "retained-freeze" : null);
    }
  });

test("import after v4 restoration and v5 autosave preserves domain and gates publication", async ({
  page,
}) => {
  await fixture(page, { legacy: true, missing: true, savedVersion4: true });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")?.version,
      ),
    )
    .toBe(5);
  const before = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("test-session")!),
  );
  await page
    .getByRole("button", { name: "Import saved CLI history", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Import saved CLI history?",
  });
  await expect(review).toBeVisible();
  await expect(review.getByRole("status")).toContainText("paused");
  expect(
    await page.evaluate(
      () => (window as any).__nativeTest.previewSession.version,
    ),
  ).toBe(5);
  await page.evaluate(() => {
    (window as any).__nativeTest.calls.length = 0;
  });
  await page.waitForTimeout(600);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (c: any) => c.command === "save_session",
      ),
    ),
  ).toHaveLength(0);
  await review.getByRole("button", { name: "Import reviewed archive" }).click();
  await expect(review).toHaveCount(0);
  await expect(page.getByLabel("Next account")).toBeVisible();
  await expect(page.locator(".agent-task-pane")).toHaveAttribute(
    "data-task-id",
    "legacy:old",
  );
  expect(
    await page.evaluate(() =>
      JSON.parse(localStorage.getItem("test-session")!),
    ),
  ).toEqual(before);
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
});
test("reviewed rollback export uses native identity and preserves the workspace", async ({
  page,
}) => {
  await fixture(page, { legacy: true, missing: true });
  await page
    .getByRole("button", { name: "Import saved CLI history", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Import saved CLI history?",
  });
  await review
    .getByRole("button", { name: "Export rollback archive", exact: true })
    .click();
  await expect(review).toHaveCount(0);
  await expect(
    page.getByRole("status").filter({ hasText: "Rollback archive exported" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Open rollback archive folder" })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.openedExport))
    .toBe("f".repeat(32));
  const request = await page.evaluate(
    () => (window as any).__nativeTest.rollbackRequest,
  );
  expect(request.migrationId).toBe("fixture-migration");
  expect(request.expectedDigest).toBe("fixture-digest");
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
  await expect(
    page.getByText("This saved CLI task is preserved as legacy history.", {
      exact: false,
    }),
  ).toBeVisible();
});

test("resumed imported legacy task drains only on its last domain view", async ({
  page,
}) => {
  await fixture(page, {
    legacy: true,
    twoCards: true,
    running: true,
    grant: true,
  });
  await page
    .locator(".agent-task-pane")
    .filter({ visible: true })
    .getByRole("button", { name: "Close task" })
    .click();
  expect(await calls(page, "agent_task_prepare_close")).toHaveLength(0);
  await page
    .locator(".agent-task-pane")
    .filter({ visible: true })
    .getByRole("button", { name: "Close task" })
    .click();
  await page
    .getByRole("dialog", { name: "Close running processes?" })
    .getByRole("button", { name: "Close anyway" })
    .click();
  await expect
    .poll(async () => (await calls(page, "agent_task_prepare_close")).length)
    .toBe(1);
  expect(
    (await calls(page, "agent_task_prepare_close"))[0].args.taskIds,
  ).toEqual(["legacy:old"]);
});
test("second saved idle task hydrates on existing subscription without change event", async ({
  page,
}) => {
  await fixture(page, { grant: true });
  await page.evaluate(async () => {
    const runtime = await import("/src/agent-runtime/task-runtime.ts");
    const model = await import("/src/model.ts");
    const mock = (window as any).__nativeTest;
    mock.otherTask = {
      ...structuredClone(mock.task),
      taskId: "idle-other",
      title: "Idle other",
    };
    const project = model.newProject("/project", "local:bash");
    project.workspaces[0].tabs = [
      model.newAgentTaskTab("task"),
      model.newAgentTaskTab("idle-other"),
    ];
    runtime.retainAgentTasks({
      ...model.newSession(),
      projects: [project],
      activeProjectId: project.id,
    });
    (window as any).__idleOther = runtime.getTaskView("idle-other");
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).__idleOther.state.loaded))
    .toBe(true);
  expect(
    await page.evaluate(() => (window as any).__idleOther.state.task.taskId),
  ).toBe("idle-other");
  expect(await calls(page, "agent_task_send")).toHaveLength(0);
});
test("final view close cancels delayed stop-and-continue before B commit", async ({
  page,
}) => {
  const pane = await fixture(page, {
    running: true,
    grant: true,
    stopAndContinueQualified: true,
  });
  await page.evaluate(() => ((window as any).__nativeTest.switchDelay = 900));
  await pane.getByLabel("Next account").selectOption("b");
  await pane
    .getByRole("button", { name: "Stop and continue on selected account" })
    .click();
  await expect
    .poll(async () => (await calls(page, "agent_task_prepare_switch")).length)
    .toBe(1);
  await page.evaluate(async () => {
    const runtime = await import("/src/agent-runtime/task-runtime.ts");
    const id = document
      .querySelector('[data-task-id="task"]')!
      .closest('[role="tabpanel"]')
      ?.getAttribute("data-tab-id");
    void id;
    const model = await import("/src/model.ts");
    const mock = (window as any).__nativeTest;
    const saved = JSON.parse(localStorage.getItem("test-session") ?? "null");
    const tabs = saved ? model.agentTaskTabs(saved) : [];
    const viewId =
      tabs[0]?.id ??
      document
        .querySelector('[role="tab"][aria-selected="true"]')!
        .id.replace(/^tab-/, "");
    const close = await runtime.closeAgentTaskViews(new Set([viewId]));
    runtime.retainAgentTasks(undefined);
    await close.release();
    mock.closedSwitch = true;
  });
  await page.waitForTimeout(1100);
  expect(await calls(page, "agent_task_commit_switch")).toHaveLength(0);
});

test("owned account terminal keeps its view when native retirement fails and retries before removal", async ({
  page,
}) => {
  await fixture(page, { empty: true });
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    await mock.emitEvent("agent-account-open-terminal", { accountId: "a" });
  });
  const tab = page.getByRole("tab", { name: "Codex · Account A", exact: true });
  await expect(tab).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (c: any) =>
              c.command === "start_terminal" &&
              c.args.request.accountId === "a",
          ).length,
      ),
    )
    .toBe(1);
  await page.evaluate(() => {
    (window as any).__nativeTest.closeTerminalError =
      "Owned retirement is still unresolved";
  });
  await page
    .getByRole("button", { name: "Close Codex · Account A", exact: true })
    .click();
  const failure = page.getByRole("dialog", {
    name: "Views could not be closed",
  });
  await expect(failure).toContainText("Owned retirement is still unresolved");
  await expect(tab).toBeVisible();
  await failure.getByRole("button", { name: "Keep open" }).click();
  await page.evaluate(() => {
    (window as any).__nativeTest.closeTerminalError = "";
  });
  await page
    .getByRole("button", { name: "Close Codex · Account A", exact: true })
    .click();
  await expect(tab).toHaveCount(0);
  const closeCalls = await page.evaluate(() =>
    (window as any).__nativeTest.calls.filter(
      (c: any) => c.command === "close_terminal",
    ),
  );
  expect(closeCalls).toHaveLength(2);
  expect(closeCalls[0].args.id).toBe(closeCalls[1].args.id);
});
