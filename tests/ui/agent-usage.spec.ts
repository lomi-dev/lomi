import { expect, test, type Locator, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

type TestUsageWindow = {
  label: string;
  remainingPercent: number | null;
  used: number | null;
  limit: number | null;
  unit: string | null;
  resetsAt: number | null;
};

function context(cli: string, pid: number) {
  return {
    cwd: "/project",
    foregroundProgram: cli,
    titleCli: { cli, pid },
  };
}

function entry(
  id: string,
  cli: string,
  pid: number,
  windows: TestUsageWindow[],
  options: {
    accountKey?: string | null;
    status?: string;
    updatedAt?: number | null;
    retryAt?: number | null;
    message?: string | null;
    source?: string | null;
  } = {},
) {
  return {
    id,
    process: { cli, pid },
    accountKey: options.accountKey,
    status: options.status ?? "ready",
    windows,
    updatedAt: options.updatedAt === undefined ? Date.now() : options.updatedAt,
    retryAt: options.retryAt ?? null,
    message: options.message ?? null,
    source: options.source ?? null,
  };
}

async function setUsage(
  page: Page,
  contexts: Record<string, ReturnType<typeof context>>,
  entries: ReturnType<typeof entry>[],
) {
  await page.evaluate(
    ({ contexts, entries }) => {
      const native = (window as any).__nativeTest;
      native.terminalContexts = contexts;
      native.cliUsageEntries = entries;
    },
    { contexts, entries },
  );
}

async function usageCalls(page: Page) {
  return page.evaluate(() => (window as any).__nativeTest.cliUsageCalls);
}

function providerSummary(trigger: Locator, cli: string) {
  return trigger.locator(`.agent-usage-summary[data-cli="${cli}"]`);
}

async function expectProviderValue(
  trigger: Locator,
  cli: string,
  value: string,
) {
  const summary = providerSummary(trigger, cli);
  await expect(summary).toBeVisible();
  await expect(summary.locator(".agent-usage-trigger-value")).toHaveText(value);
}

async function expectProviderOrder(trigger: Locator, expected: string[]) {
  await expect
    .poll(() =>
      trigger
        .locator(".agent-usage-summary")
        .evaluateAll((summaries) =>
          summaries.map((summary) => summary.getAttribute("data-cli")),
        ),
    )
    .toEqual(expected);
}

function quota(label: string, remainingPercent: number | null) {
  return {
    label,
    remainingPercent,
    used: null,
    limit: null,
    unit: null,
    resetsAt: null,
  };
}

test("remaining usage updates on the timer and focus without opening the menu", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.clock.install();
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const now = await page.evaluate(() => Date.now());
  await page.clock.pauseAt(new Date(now + 1000));
  const contexts = { "live-session": context("codex", 299) };
  const quota = (remainingPercent: number) =>
    entry("live-session", "codex", 299, [
      {
        label: "5-hour",
        remainingPercent,
        used: null,
        limit: null,
        unit: null,
        resetsAt: null,
      },
    ]);
  await setUsage(page, contexts, [quota(80)]);
  await page.clock.runFor(1000);
  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderValue(trigger, "codex", "80%");

  const initialCalls = (await usageCalls(page)).length;
  await setUsage(page, contexts, [quota(60)]);
  await page.clock.runFor(5000);
  await expectProviderValue(trigger, "codex", "80%");
  expect((await usageCalls(page)).length).toBe(initialCalls);
  await page.clock.runFor(10_000);
  await expectProviderValue(trigger, "codex", "60%");
  expect((await usageCalls(page)).length).toBeGreaterThan(initialCalls);
  await expect(page.locator(".agent-usage-menu")).toHaveCount(0);

  await setUsage(page, contexts, [quota(40)]);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expectProviderValue(trigger, "codex", "40%");
  await setUsage(page, {}, []);
  await page.clock.runFor(1000);
  await expect(trigger).toHaveCount(0);
  const stoppedCalls = (await usageCalls(page)).length;
  await page.clock.runFor(30_000);
  expect((await usageCalls(page)).length).toBe(stoppedCalls);
});

test("provider summaries retain independent values and details show stale multi-agent usage", async ({
  page,
}) => {
  await page.setViewportSize({ width: 920, height: 680 });
  await page.emulateMedia({ colorScheme: "light" });
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "start_terminal",
          ).length,
      ),
    )
    .toBe(1);
  const startsBefore = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "start_terminal",
      ).length,
  );

  const reset = Date.now() + 12 * 60 * 60 * 1000;
  await setUsage(
    page,
    {
      "codex-session": context("codex", 301),
      "claude-session": context("claude", 302),
    },
    [
      entry(
        "codex-session",
        "codex",
        301,
        [
          {
            label: "5-hour",
            remainingPercent: 65.25,
            used: 6_950,
            limit: 20_000,
            unit: "tokens",
            resetsAt: reset,
          },
          {
            label: "Weekly",
            remainingPercent: 12.4,
            used: 876,
            limit: 1_000,
            unit: "requests",
            resetsAt: reset,
          },
        ],
        {
          status: "rate-limited",
          updatedAt: Date.now() - 60_000,
          retryAt: Date.now() + 60_000,
          message:
            "Usage refresh is limited. Try again after the current window.",
          source: "oauth",
        },
      ),
      entry(
        "claude-session",
        "claude",
        302,
        [
          {
            label: "Current session",
            remainingPercent: 42.5,
            used: 575,
            limit: 1_000,
            unit: "messages",
            resetsAt: reset,
          },
        ],
        { source: "https://usage.example/account?secret=hidden" },
      ),
    ],
  );

  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderOrder(trigger, ["codex", "claude"]);
  await expectProviderValue(trigger, "codex", "12.4%");
  await expectProviderValue(trigger, "claude", "42.5%");
  await expect(providerSummary(trigger, "codex")).toHaveAttribute(
    "data-tone",
    "warning",
  );
  await expect(providerSummary(trigger, "codex")).toHaveAttribute(
    "data-stale",
    "true",
  );
  await expect(providerSummary(trigger, "claude")).toHaveAttribute(
    "data-tone",
    "normal",
  );
  await expect(providerSummary(trigger, "claude")).not.toHaveAttribute(
    "data-stale",
    "true",
  );
  const label = (await trigger.getAttribute("aria-label"))!;
  const title = (await trigger.getAttribute("title"))!;
  for (const summaryText of [label, title]) {
    expect(summaryText).toContain("Codex");
    expect(summaryText).toContain("Claude");
    expect(summaryText).toContain("12.4%");
    expect(summaryText).toContain("42.5%");
    expect(summaryText).toMatch(/stale data/i);
  }
  expect(
    await trigger.evaluate(
      (element) =>
        element.compareDocumentPosition(
          document.querySelector(".titlebar-account-control")!,
        ) & Node.DOCUMENT_POSITION_FOLLOWING,
    ),
  ).toBeTruthy();

  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("group")).toHaveCount(2);
  await expect(menu).toContainText("5-hour");
  await expect(menu).toContainText("Weekly");
  const weeklyQuota = menu.getByRole("progressbar", {
    name: "Weekly quota: 12.4% remaining",
  });
  await expect(weeklyQuota).toBeVisible();
  await expect(weeklyQuota).toHaveJSProperty("value", 12.4);
  await expect(menu).toContainText("876 requests used / 1,000 requests");
  await expect(menu).toContainText("Source · Connected account");
  await expect(menu).toContainText("Usage refresh is limited.");
  await expect(menu).not.toContainText("usage.example");
  await page.screenshot({ path: "/tmp/lomi-agent-usage-menu-light.png" });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.dataset.appearance),
    )
    .toBe("dark");
  await page.screenshot({ path: "/tmp/lomi-agent-usage-menu-dark.png" });

  const refresh = menu.getByRole("menuitem", { name: "Refresh", exact: true });
  const beforeRefresh = await usageCalls(page);
  await refresh.click();
  await expect
    .poll(async () => (await usageCalls(page)).length)
    .toBeGreaterThan(beforeRefresh.length);
  await expect
    .poll(async () => (await usageCalls(page)).at(-1)?.force)
    .toBe(true);

  const protectedCalls = await page.evaluate(() =>
    (window as any).__nativeTest.calls.filter(
      (call: any) =>
        call.command === "write_terminal" ||
        call.command === "enable_cli_integration" ||
        call.command === "enable_cli_titles",
    ),
  );
  const startsAfter = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "start_terminal",
      ).length,
  );
  expect(protectedCalls).toHaveLength(0);
  expect(startsAfter).toBe(startsBefore);
});

test("three provider summaries update independently and group duplicate provider sessions", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await page.emulateMedia({ colorScheme: "light" });
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();

  const agents = [
    { id: "codex-quota-a", cli: "codex", pid: 601, label: "Codex primary" },
    {
      id: "codex-quota-b",
      cli: "codex",
      pid: 602,
      label: "Codex secondary",
    },
    { id: "cursor-quota", cli: "cursor", pid: 603, label: "Cursor window" },
    { id: "claude-quota", cli: "claude", pid: 604, label: "Claude window" },
  ];
  const contexts = Object.fromEntries(
    agents.map(({ id, cli, pid }) => [id, context(cli, pid)]),
  );
  const percentages = {
    "codex-quota-a": 80,
    "codex-quota-b": 100,
    "cursor-quota": 25,
    "claude-quota": 0,
  };
  const quotaEntries = (values: Record<string, number>) =>
    agents.map(({ id, cli, pid, label }) =>
      entry(id, cli, pid, [quota(label, values[id])]),
    );

  await setUsage(page, contexts, quotaEntries(percentages));
  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderOrder(trigger, ["codex", "cursor", "claude"]);
  await expect(trigger.locator(".agent-usage-summary")).toHaveCount(3);
  await expectProviderValue(trigger, "codex", "80%");
  await expectProviderValue(trigger, "cursor", "25%");
  await expectProviderValue(trigger, "claude", "0%");
  await expect(providerSummary(trigger, "codex")).toHaveAttribute(
    "data-tone",
    "normal",
  );
  await expect(providerSummary(trigger, "cursor")).toHaveAttribute(
    "data-tone",
    "warning",
  );
  await expect(providerSummary(trigger, "claude")).toHaveAttribute(
    "data-tone",
    "critical",
  );
  const summaryList = trigger.locator(".agent-usage-trigger-list");
  const listBounds = await summaryList.boundingBox();
  expect(listBounds).not.toBeNull();
  for (const cli of ["codex", "cursor", "claude"]) {
    const summary = providerSummary(trigger, cli);
    const summaryParts = [
      summary,
      summary.locator(".cli-agent-icon"),
      summary.locator(".agent-usage-trigger-value"),
    ];
    for (const part of summaryParts) {
      await expect(part).toBeVisible();
      const bounds = await part.boundingBox();
      expect(bounds).not.toBeNull();
      expect(bounds!.x).toBeGreaterThanOrEqual(listBounds!.x);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(
        listBounds!.x + listBounds!.width,
      );
    }
  }
  await page.screenshot({ path: "/tmp/lomi-per-agent-usage-three-light.png" });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.dataset.appearance),
    )
    .toBe("dark");
  await page.screenshot({ path: "/tmp/lomi-per-agent-usage-three-dark.png" });
  await page.emulateMedia({ colorScheme: "light" });
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.dataset.appearance),
    )
    .toBe("light");
  for (const text of [
    (await trigger.getAttribute("aria-label"))!,
    (await trigger.getAttribute("title"))!,
  ]) {
    expect(text).toContain("Codex");
    expect(text).toContain("80%");
    expect(text).toContain("Cursor");
    expect(text).toContain("25%");
    expect(text).toContain("Claude");
    expect(text).toContain("0%");
  }
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu.getByRole("group")).toHaveCount(4);

  const expectValues = async (values: Record<string, number>) => {
    await expectProviderValue(
      trigger,
      "codex",
      `${Math.min(values["codex-quota-a"], values["codex-quota-b"])}%`,
    );
    await expectProviderValue(trigger, "cursor", `${values["cursor-quota"]}%`);
    await expectProviderValue(trigger, "claude", `${values["claude-quota"]}%`);
    await expect(menu.getByRole("progressbar")).toHaveCount(agents.length);
    for (const agent of agents) {
      const expected = values[agent.id];
      const progress = menu.getByRole("progressbar", {
        name: `${agent.label} quota: ${expected}% remaining`,
      });
      await expect(progress).toHaveJSProperty("value", expected);
    }
  };

  await expectValues(percentages);
  const updateAndRefresh = async (values: Record<string, number>) => {
    await setUsage(page, contexts, quotaEntries(values));
    const callsBefore = (await usageCalls(page)).length;
    await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
    await expect
      .poll(async () => (await usageCalls(page)).length)
      .toBeGreaterThan(callsBefore);
    await expectValues(values);
  };

  await updateAndRefresh({
    "codex-quota-a": 65,
    "codex-quota-b": 90,
    "cursor-quota": 25,
    "claude-quota": 0,
  });
  await updateAndRefresh({
    "codex-quota-a": 65,
    "codex-quota-b": 90,
    "cursor-quota": 100,
    "claude-quota": 0,
  });
  await updateAndRefresh({
    "codex-quota-a": 65,
    "codex-quota-b": 90,
    "cursor-quota": 100,
    "claude-quota": 100,
  });
});

test("remaining percentages stay aligned across provider summaries and quota bars", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();

  const agents = [
    { id: "codex-quota", cli: "codex", pid: 611, label: "Codex window" },
    { id: "claude-quota", cli: "claude", pid: 612, label: "Claude window" },
    { id: "cursor-quota", cli: "cursor", pid: 613, label: "Cursor window" },
    { id: "kimi-quota", cli: "kimi", pid: 614, label: "Kimi window" },
    { id: "agy-quota", cli: "agy", pid: 615, label: "Antigravity window" },
  ];
  const contexts = Object.fromEntries(
    agents.map(({ id, cli, pid }) => [id, context(cli, pid)]),
  );
  const quotaEntries = (remainingPercent: number) =>
    agents.map(({ id, cli, pid, label }) =>
      entry(id, cli, pid, [quota(label, remainingPercent)]),
    );
  const catalogOrder = ["codex", "agy", "cursor", "claude", "kimi"];

  await setUsage(page, contexts, quotaEntries(100));
  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderOrder(trigger, catalogOrder);
  await expect(trigger.locator(".agent-usage-summary")).toHaveCount(5);
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");

  const expectRemaining = async (remainingPercent: number) => {
    for (const cli of catalogOrder)
      await expectProviderValue(trigger, cli, `${remainingPercent}%`);
    await expect(menu.getByRole("progressbar")).toHaveCount(agents.length);
    for (const agent of agents) {
      const progress = menu.getByRole("progressbar", {
        name: `${agent.label} quota: ${remainingPercent}% remaining`,
      });
      await expect(progress).toHaveJSProperty("value", remainingPercent);
    }
  };

  await expectRemaining(100);
  for (const remainingPercent of [75, 0]) {
    await setUsage(page, contexts, quotaEntries(remainingPercent));
    const callsBefore = (await usageCalls(page)).length;
    await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
    await expect
      .poll(async () => (await usageCalls(page)).length)
      .toBeGreaterThan(callsBefore);
    await expectRemaining(remainingPercent);
  }
});

test("keyboard, outside click, and provider removal dismiss the anchored usage menu", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await setUsage(
    page,
    {
      "first-session": context("codex", 401),
      "second-session": context("claude", 402),
    },
    [
      entry("first-session", "codex", 401, [
        {
          label: "Daily",
          remainingPercent: 44,
          used: 56,
          limit: 100,
          unit: "requests",
          resetsAt: Date.now() + 60_000,
        },
      ]),
      entry("second-session", "claude", 402, [quota("Session", 72)]),
    ],
  );

  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger).toBeVisible();
  await trigger.focus();
  await page.keyboard.press("ArrowDown");
  const menu = page.locator(".agent-usage-menu");
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Refresh" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(trigger).toBeFocused();

  await trigger.click({ button: "right" });
  await expect(menu).toBeVisible();
  await page.setViewportSize({ width: 800, height: 600 });
  await expect(menu).toHaveCount(0);
  await trigger.click();
  await expect(menu).toBeVisible();
  await page.locator(".work-area").click({ position: { x: 20, y: 300 } });
  await expect(menu).toHaveCount(0);

  await setUsage(page, { "second-session": context("claude", 402) }, [
    entry("second-session", "claude", 402, [quota("Session", 72)]),
  ]);
  await expectProviderOrder(trigger, ["claude"]);
  await expectProviderValue(trigger, "claude", "72%");
  await expect(providerSummary(trigger, "codex")).toHaveCount(0);

  await trigger.click();
  await expect(menu).toBeVisible();
  await setUsage(page, {}, []);
  await expect(trigger).toHaveCount(0);
  await expect(menu).toHaveCount(0);

  await setUsage(page, { "next-session": context("claude", 402) }, [
    entry("next-session", "claude", 402, [
      {
        label: "Session",
        remainingPercent: 72,
        used: 28,
        limit: 100,
        unit: "messages",
        resetsAt: null,
      },
    ]),
  ]);
  await expect(trigger).toBeVisible();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
  await expect(menu).toHaveCount(0);
});

test("an unavailable provider shows an em dash without hiding known quota", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await setUsage(
    page,
    {
      "codex-session": context("codex", 451),
      "claude-session": context("claude", 452),
    },
    [
      entry("codex-session", "codex", 451, [quota("Codex quota", 48)]),
      entry("claude-session", "claude", 452, [quota("Account quota", null)], {
        status: "unsupported",
        updatedAt: null,
        message: "This account is using a quota mode the CLI cannot read.",
      }),
    ],
  );

  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderOrder(trigger, ["codex", "claude"]);
  await expectProviderValue(trigger, "codex", "48%");
  await expectProviderValue(trigger, "claude", "—");
  await expect(providerSummary(trigger, "claude")).toHaveAttribute(
    "data-tone",
    "unknown",
  );
  const label = (await trigger.getAttribute("aria-label"))!;
  const title = (await trigger.getAttribute("title"))!;
  expect(label).toContain("Codex");
  expect(label).toContain("48%");
  expect(label).toContain("Claude");
  expect(label).toMatch(/Usage unavailable/i);
  expect(title).toMatch(/Usage unavailable/i);
  await expect(
    providerSummary(trigger, "claude").locator(".agent-usage-stale-dot"),
  ).toHaveCount(0);
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu).toContainText("Usage unavailable");
  await expect(menu).toContainText("This account is using a quota mode");
  await expect(menu).toContainText("Quota unknown");
  await expect(menu.getByRole("progressbar")).toHaveCount(1);
  const unavailableGroup = menu.getByRole("group", {
    name: /Claude Code usage/,
  });
  await expect(unavailableGroup).toContainText("Quota unknown");
  await expect(unavailableGroup.getByRole("progressbar")).toHaveCount(0);
  await expect(menu.locator(".agent-usage-status svg")).toHaveCount(0);
});

test("late usage results are ignored after the running agent changes", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    (window as any).__nativeTest.cliUsageDelay = 350;
  });
  await setUsage(page, { "codex-session": context("codex", 501) }, [
    entry("codex-session", "codex", 501, [
      {
        label: "Old account",
        remainingPercent: 4,
        used: 96,
        limit: 100,
        unit: "requests",
        resetsAt: null,
      },
    ]),
    entry("claude-session", "claude", 502, [
      {
        label: "New account",
        remainingPercent: 37.4,
        used: 626,
        limit: 1_000,
        unit: "tokens",
        resetsAt: null,
      },
    ]),
  ]);
  await expect.poll(async () => (await usageCalls(page)).length).toBe(1);
  await setUsage(page, { "claude-session": context("claude", 502) }, [
    entry("codex-session", "codex", 501, [
      {
        label: "Old account",
        remainingPercent: 4,
        used: 96,
        limit: 100,
        unit: "requests",
        resetsAt: null,
      },
    ]),
    entry("claude-session", "claude", 502, [
      {
        label: "New account",
        remainingPercent: 37.4,
        used: 626,
        limit: 1_000,
        unit: "tokens",
        resetsAt: null,
      },
    ]),
  ]);

  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderValue(trigger, "claude", "37.4%");
  const calls = await usageCalls(page);
  expect(calls).toHaveLength(2);
  expect(calls[0].targets).toEqual([
    { id: "codex-session", process: { cli: "codex", pid: 501 } },
  ]);
  expect(calls[1].targets).toEqual([
    { id: "claude-session", process: { cli: "claude", pid: 502 } },
  ]);
});

test("the details list can be scrolled with the keyboard when many agents are active", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const contexts = Object.fromEntries(
    Array.from({ length: 12 }, (_, index) => [
      `agent-session-${index + 1}`,
      context("codex", 650 + index),
    ]),
  );
  const entries = Object.entries(contexts).map(([id, value], index) =>
    entry(id, "codex", value.titleCli.pid, [
      {
        label: `Window ${index + 1}`,
        remainingPercent: 80 - index,
        used: 20 + index,
        limit: 100,
        unit: "requests",
        resetsAt: null,
      },
    ]),
  );
  await setUsage(page, contexts, entries);

  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderValue(trigger, "codex", "69%");
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  const list = menu.getByRole("region", { name: "Usage details by agent" });
  await expect(list).toBeVisible();
  await page.keyboard.press("Shift+Tab");
  await expect(list).toBeFocused();
  const before = await list.evaluate((element) => element.scrollTop);
  await page.keyboard.press("End");
  await expect
    .poll(() => list.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(before);
  await expect(
    menu.getByRole("group", { name: "Codex usage 12" }),
  ).toBeInViewport();
});

test("many provider summaries keep titlebar controls visible at minimum width", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await page.emulateMedia({ colorScheme: "light" });
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();

  const providers = [
    "codex",
    "agy",
    "cursor",
    "claude",
    "gemini",
    "copilot",
    "opencode",
    "openclaw",
    "hermes",
    "pi",
    "kilo",
    "qwen",
    "kiro",
    "vibe",
    "kimi",
  ];
  const contexts = Object.fromEntries(
    providers.map((cli, index) => [
      `minimum-${cli}`,
      context(cli, 700 + index),
    ]),
  );
  const entries = providers.map((cli, index) =>
    entry(`minimum-${cli}`, cli, 700 + index, [quota(`${cli} quota`, 80)]),
  );
  await setUsage(page, contexts, entries);

  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderOrder(trigger, providers);
  await expect(trigger.locator(".agent-usage-summary")).toHaveCount(
    providers.length,
  );
  const account = page.locator(".titlebar-account-control");
  const windowControls = page.locator(".window-controls");
  await expect(trigger).toBeInViewport();
  await expect(account).toBeInViewport();
  await expect(windowControls).toBeInViewport();

  const assertInWidth = async (locator: Locator) => {
    const bounds = await locator.boundingBox();
    expect(bounds).not.toBeNull();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(800);
  };
  await assertInWidth(trigger);
  await assertInWidth(account);
  await assertInWidth(windowControls);
  await page.screenshot({
    path: "/tmp/lomi-per-agent-usage-minimum-light.png",
  });

  await page.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.dataset.appearance),
    )
    .toBe("dark");
  await page.screenshot({
    path: "/tmp/lomi-per-agent-usage-minimum-dark.png",
  });

  const list = trigger.locator(".agent-usage-trigger-list");
  const overflow = await list.evaluate(
    (element) => element.scrollWidth > element.clientWidth,
  );
  expect(overflow).toBe(true);
  await trigger.focus();
  for (let step = 0; step < 20; step++) await page.keyboard.press("ArrowRight");
  const lastSummary = providerSummary(trigger, "kimi");
  await expect
    .poll(() =>
      lastSummary.evaluate((summary) => {
        const list = summary.parentElement!;
        const summaryBounds = summary.getBoundingClientRect();
        const listBounds = list.getBoundingClientRect();
        return (
          summaryBounds.left >= listBounds.left &&
          summaryBounds.right <= listBounds.right + 1
        );
      }),
    )
    .toBe(true);
  expect(await list.evaluate((element) => element.scrollLeft)).toBeGreaterThan(
    0,
  );
  await assertInWidth(account);
  await assertInWidth(windowControls);

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ colorScheme: "light" });
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.dataset.appearance),
    )
    .toBe("light");
  await expect.poll(() => page.evaluate(() => innerWidth)).toBe(1440);
  await expect(trigger).toBeInViewport();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
  await trigger.scrollIntoViewIfNeeded();
  await trigger.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator(".agent-usage-menu")).toBeVisible();
  await list.evaluate((element: HTMLElement) => {
    element.scrollLeft = 0;
  });
  await page.screenshot({ path: "/tmp/lomi-per-agent-usage-normal-light.png" });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.dataset.appearance),
    )
    .toBe("dark");
  await page.screenshot({ path: "/tmp/lomi-per-agent-usage-normal-dark.png" });
  await page.keyboard.press("Escape");
});

test("usage details stay within a compact titlebar at reduced zoom", async ({
  page,
}) => {
  await page.setViewportSize({ width: 440, height: 300 });
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await setUsage(page, { "codex-session": context("codex", 601) }, [
    entry("codex-session", "codex", 601, [
      {
        label: "5-hour",
        remainingPercent: 0.4,
        used: 999.6,
        limit: 1_000,
        unit: "tokens",
        resetsAt: Date.now() + 24 * 60 * 60 * 1000,
      },
    ]),
  ]);
  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderValue(trigger, "codex", "0.4%");
  await page.keyboard.press("Control+Minus");
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.zoom))
    .toBe(0.9);
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu).toBeVisible();
  const bounds = await menu.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(8);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(432);
  await page.screenshot({ path: "/tmp/lomi-agent-usage-menu-compact.png" });
});

test("account usage groups four sessions and retains different accounts with identical quotas", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const contexts = Object.fromEntries(
    [1, 2, 3, 4].map((id) => [
      `account-session-${id}`,
      context("codex", 800 + id),
    ]),
  );
  const entries = [1, 2, 3, 4].map((id) =>
    entry(`account-session-${id}`, "codex", 800 + id, [quota("Weekly", 70)], {
      accountKey: "account-a",
    }),
  );
  await setUsage(page, contexts, entries);
  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderValue(trigger, "codex", "70%");
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu.getByRole("group")).toHaveCount(1);
  await expect(menu).toContainText("4 active CLI sessions");
  expect((await usageCalls(page)).at(-1).targets).toHaveLength(4);
  await page.screenshot({ path: "/tmp/lomi-account-usage-grouped.png" });

  await setUsage(
    page,
    contexts,
    entries.map((value, index) => ({
      ...value,
      accountKey: index < 2 ? "account-a" : "account-b",
    })),
  );
  await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
  await expect(menu.getByRole("group")).toHaveCount(2);
  await expect(menu.getByRole("progressbar")).toHaveCount(2);
  await expect(menu.getByRole("progressbar").nth(0)).toHaveAttribute(
    "value",
    "70",
  );
  await expect(menu.getByRole("progressbar").nth(1)).toHaveAttribute(
    "value",
    "70",
  );
});

test("account rows keep a good snapshot through duplicate failure and representative removal", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const contexts = {
    first: context("codex", 811),
    second: context("codex", 812),
  };
  const now = Date.now();
  const entries = [
    entry("first", "codex", 811, [], {
      accountKey: "account-a",
      status: "error",
      message: "Duplicate check failed",
    }),
    entry("second", "codex", 812, [quota("Weekly", 65)], {
      accountKey: "account-a",
      updatedAt: now,
    }),
  ];
  await setUsage(page, contexts, entries);
  const trigger = page.locator(".agent-usage-trigger");
  await expectProviderValue(trigger, "codex", "65%");
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu.getByRole("group")).toHaveCount(1);
  await expect(menu.getByRole("group")).toContainText("Current");
  await expect(menu.getByRole("group")).not.toContainText(
    "Duplicate check failed",
  );

  const saved = [
    entry("first", "codex", 811, [quota("Weekly", 50)], {
      accountKey: "account-a",
      updatedAt: now - 1000,
    }),
    entry("second", "codex", 812, [quota("Weekly", 40)], {
      accountKey: "account-a",
      status: "rate-limited",
      updatedAt: now,
    }),
  ];
  await setUsage(page, contexts, saved);
  await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
  await expect(menu.getByRole("group")).toHaveCount(1);
  await expect(menu.getByRole("progressbar")).toHaveAttribute("value", "40");
  await expect(menu.getByRole("group")).toContainText(
    "Rate limited · showing saved values",
  );
  await expect(menu.locator(".agent-usage-status")).toHaveAttribute(
    "data-stale",
    "true",
  );
  await expectProviderValue(trigger, "codex", "40%");

  await setUsage(page, contexts, [{ ...saved[0], updatedAt: now }, saved[1]]);
  await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
  await expect(menu.getByRole("progressbar")).toHaveAttribute("value", "50");
  await expect(menu.getByRole("group")).toContainText("Current");
  await expectProviderValue(trigger, "codex", "40%");

  const healthy = [
    entry("first", "codex", 811, [quota("Weekly", 66)], {
      accountKey: "account-a",
      updatedAt: now - 1000,
    }),
    entries[1],
  ];
  await setUsage(page, contexts, healthy);
  await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
  await expect(menu.getByRole("progressbar")).toHaveAttribute("value", "65");
  await setUsage(page, { first: contexts.first }, healthy);
  await expect(menu.getByRole("group")).toHaveCount(1);
  await expect(menu.getByRole("progressbar")).toHaveAttribute("value", "66");

  await setUsage(page, contexts, [
    healthy[0],
    { ...entries[1], accountKey: "account-b" },
  ]);
  await expect(menu.getByRole("group")).toHaveCount(2);
  await setUsage(page, contexts, [
    healthy[0],
    { ...entries[1], accountKey: null },
  ]);
  await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
  await expect(menu.getByRole("group")).toHaveCount(2);
  await expect(menu.getByRole("progressbar")).toHaveCount(2);
});

const cursorDenied = (id = "cursor-session", pid = 901) =>
  entry(id, "cursor", pid, [], {
    status: "keychain-access-required",
    updatedAt: null,
  });

async function setupCursorKeychain(page: Page) {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await setUsage(page, { "cursor-session": context("cursor", 901) }, [
    cursorDenied(),
  ]);
  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger).toContainText("—");
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu).toContainText("Keychain access needed");
  return {
    trigger,
    menu,
    allow: menu.getByRole("menuitem", { name: "Allow Keychain access…" }),
  };
}

async function authorizationCalls(page: Page) {
  return page.evaluate(
    () => (window as any).__nativeTest.cliUsageAuthorizationCalls,
  );
}

test("Cursor Keychain authorization is explicit and keyboard accessible", async ({
  page,
}) => {
  const { trigger, menu, allow } = await setupCursorKeychain(page);
  await page.clock.install();
  await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.clock.runFor(16_000);
  expect(await authorizationCalls(page)).toEqual([]);
  await expect(allow).toBeEnabled();
  await page.keyboard.press("Home");
  await expect(allow).toBeFocused();
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.cliUsageAuthorizationDelay = 600;
    native.cliUsageAuthorizationEntries = native.cliUsageEntries;
  });
  await page.keyboard.press("Enter");
  await expect(
    menu.getByRole("menuitem", { name: "Waiting for macOS…" }),
  ).toBeDisabled();
  expect(await authorizationCalls(page)).toHaveLength(1);
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(menu).toHaveCount(0);
  await trigger.click();
  const waiting = menu.getByRole("menuitem", { name: "Waiting for macOS…" });
  await expect(waiting).toBeDisabled();
  await waiting.evaluate((button: HTMLButtonElement) => button.click());
  expect(await authorizationCalls(page)).toHaveLength(1);
  await setUsage(page, { "cursor-session": context("cursor", 901) }, [
    entry("cursor-session", "cursor", 901, [quota("Monthly", 72)]),
  ]);
  await page.clock.runFor(1000);
  await expectProviderValue(trigger, "cursor", "72%");
  await expect(allow).toHaveCount(0);
  expect((await usageCalls(page)).at(-1).force).toBe(true);
});

test("cancelled Cursor authorization stays recoverable and does not expose native errors", async ({
  page,
}) => {
  const { menu, allow } = await setupCursorKeychain(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.cliUsageAuthorizationError =
      "secret native error";
  });
  await allow.click();
  await expect(menu.getByRole("status")).toContainText(
    "The Keychain access request could not finish. Try again.",
  );
  await expect(menu).not.toContainText("secret native error");
  await expect(allow).toBeEnabled();
  await page.evaluate(() => {
    (window as any).__nativeTest.cliUsageAuthorizationError = "";
  });
  await allow.click();
  await expect
    .poll(async () => (await authorizationCalls(page)).length)
    .toBe(2);
  await expect(allow).toBeEnabled();
  await expect(menu).toContainText("Keychain access needed");
});

test("Cursor authorization ignores results for a replaced process", async ({
  page,
}) => {
  const { trigger, menu, allow } = await setupCursorKeychain(page);
  await page.evaluate(
    (entries) => {
      const native = (window as any).__nativeTest;
      native.cliUsageAuthorizationDelay = 700;
      native.cliUsageAuthorizationEntries = entries;
    },
    [entry("cursor-session", "cursor", 901, [quota("Old account", 3)])],
  );
  await allow.click();
  await setUsage(page, { "cursor-session": context("cursor", 902) }, [
    cursorDenied("cursor-session", 902),
  ]);
  await expect(
    menu.getByRole("menuitem", { name: "Waiting for macOS…" }),
  ).toHaveCount(0);
  await expect(allow).toBeEnabled();
  await expect(menu).not.toContainText("Old account");
  await expectProviderValue(trigger, "cursor", "—");
  await allow.click();
  await expect
    .poll(async () => (await authorizationCalls(page)).length)
    .toBe(2);
  expect((await authorizationCalls(page))[1].process.pid).toBe(902);
});

test("Cursor Keychain recovery fits narrow and desktop menus in both appearances", async ({
  page,
}) => {
  const { trigger, menu } = await setupCursorKeychain(page);
  for (const width of [360, 800]) {
    for (const colorScheme of ["light", "dark"] as const) {
      if (await menu.isVisible()) await page.keyboard.press("Escape");
      await page.setViewportSize({ width, height: 600 });
      await page.emulateMedia({ colorScheme });
      await expect
        .poll(() =>
          page.evaluate(() => document.documentElement.dataset.appearance),
        )
        .toBe(colorScheme);
      await page.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
      await trigger.click();
      await expect(
        menu.getByRole("menuitem", { name: "Allow Keychain access…" }),
      ).toBeVisible();
      const bounds = await menu.boundingBox();
      expect(bounds!.x).toBeGreaterThanOrEqual(8);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width - 8);
      await page.screenshot({
        path: `/tmp/lomi-cursor-keychain-${width}-${colorScheme}.png`,
      });
    }
  }
});

test("Cursor grant recovery ignores an older passive read and disables other grants", async ({
  page,
}) => {
  const { menu, allow, trigger } = await setupCursorKeychain(page);
  const contexts = {
    "cursor-session": context("cursor", 901),
    "other-cursor": context("cursor", 903),
  };
  await setUsage(page, contexts, [
    cursorDenied(),
    cursorDenied("other-cursor", 903),
  ]);
  await expect(
    menu.getByRole("menuitem", { name: "Allow Keychain access…" }),
  ).toHaveCount(2);
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.cliUsageDelay = 700;
    native.cliUsageAuthorizationDelay = 100;
  });
  const before = (await usageCalls(page)).length;
  await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
  await expect
    .poll(async () => (await usageCalls(page)).length)
    .toBe(before + 1);
  await allow.first().click();
  await expect(
    menu.getByRole("menuitem", { name: "Allow Keychain access…" }),
  ).toBeDisabled();
  const ready = entry("cursor-session", "cursor", 901, [quota("Monthly", 64)]);
  await page.evaluate((ready) => {
    const native = (window as any).__nativeTest;
    native.cliUsageEntries = [
      ready,
      ...native.cliUsageEntries.filter((value: any) => value.id !== ready.id),
    ];
    native.cliUsageAuthorizationEntries = [ready];
  }, ready);
  await expectProviderValue(trigger, "cursor", "64%");
  await expect(menu.getByRole("progressbar")).toHaveAttribute("value", "64");
  expect(await authorizationCalls(page)).toHaveLength(1);
});
