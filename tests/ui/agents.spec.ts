import { expectRecordedNotification } from "./application-notification-helpers";
import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { active, addWorkspace, newSession, openFileTab } from "../../src/model";

const installedAgents = [
  { cli: "cursor", name: "Cursor CLI", command: "cursor-agent" },
  { cli: "claude", name: "Claude Code", command: "claude" },
];

async function openAgents(page: import("@playwright/test").Page) {
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Agents", exact: true }).click();
  return page.getByRole("dialog", { name: "Agents" });
}

async function setupAgents(
  page: import("@playwright/test").Page,
  agents = installedAgents,
  blocked = false,
) {
  await mockDesktop(page, false, undefined, undefined, undefined, undefined, {
    installed: agents,
    blocked,
  });
  await page.goto("/");
}

async function releaseScans(page: import("@playwright/test").Page) {
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.installedAgentCliBlocked = false;
    mock.installedAgentCliReleases
      .splice(0)
      .forEach((release: () => void) => release());
  });
}

test("Agents starts four terminals in a two-by-two grid within one new tab", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "light" });
  await setupAgents(page, installedAgents, true);
  const dialog = await openAgents(page);

  await expect(dialog.getByRole("status")).toHaveText(
    "Looking for installed agents…",
  );
  await expect(
    dialog.getByRole("button", { name: "Refresh installed CLI" }),
  ).toBeDisabled();
  await releaseScans(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await expect(
    dialog.getByRole("radio", { name: "Cursor CLI", exact: true }),
  ).toBeChecked();
  await expect(
    dialog.getByRole("radio", { name: "Claude Code", exact: true }),
  ).toBeVisible();
  await expect(dialog.locator(".agents-cli-option")).toHaveText(["", ""]);
  await expect(dialog).not.toContainText("cursor-agent");
  await expect(dialog.getByLabel("Number of terminals")).toHaveValue("4");
  await expect(dialog.locator(".agents-directory code")).toHaveText("/project");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.installedAgentCliCalls.length,
      ),
    )
    .toBeGreaterThanOrEqual(1);
  expect(
    await page.evaluate(
      () => (window as any).__nativeTest.installedAgentCliCalls,
    ),
  ).toEqual(
    expect.arrayContaining([{ profileId: "local:bash", cwd: "/project" }]),
  );
  await dialog.screenshot({
    path: testInfo.outputPath("agents-dialog-light.png"),
  });

  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(
    page.getByRole("tab", { name: "Cursor CLI", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".terminal-pane")).toHaveCount(4);
  const panels = await page.locator(".terminal-pane").evaluateAll((panels) =>
    panels.map((panel) => {
      const { x, y, width, height } = panel.getBoundingClientRect();
      return { x, y, width, height };
    }),
  );
  expect(panels[0].y).toBeCloseTo(panels[1].y, 0);
  expect(panels[2].y).toBeCloseTo(panels[3].y, 0);
  expect(panels[0].x).toBeCloseTo(panels[2].x, 0);
  expect(panels[1].x).toBeCloseTo(panels[3].x, 0);
  expect(panels[1].x).toBeGreaterThan(panels[0].x);
  expect(panels[2].y).toBeGreaterThan(panels[0].y);
  for (const panel of panels) {
    expect(panel.width).toBeGreaterThanOrEqual(240);
    expect(panel.height).toBeGreaterThanOrEqual(120);
    expect(panel.width).toBeCloseTo(panels[0].width, 0);
    expect(panel.height).toBeCloseTo(panels[0].height, 0);
  }

  const agentStarts = () =>
    page.evaluate(() =>
      (window as any).__nativeTest.calls
        .filter((call: any) => call.command === "start_terminal")
        .map((call: any) => call.args.request)
        .filter((request: any) => request.cliLaunch),
    );
  await expect.poll(async () => (await agentStarts()).length).toBe(4);
  const requests = await agentStarts();
  expect(requests.map((request: any) => request.cliLaunch)).toEqual([
    "cursor",
    "cursor",
    "cursor",
    "cursor",
  ]);
  expect(requests.map((request: any) => request.cwd)).toEqual([
    "/project",
    "/project",
    "/project",
    "/project",
  ]);
  expect(requests.map((request: any) => request.profileId)).toEqual([
    "local:bash",
    "local:bash",
    "local:bash",
    "local:bash",
  ]);
  expect(new Set(requests.map((request: any) => request.id)).size).toBe(4);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "write_terminal",
      ),
    ),
  ).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath("agents-grid-light.png") });

  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("tab", { name: "Cursor CLI", exact: true }).click();
  await expect(page.locator(".terminal-pane")).toHaveCount(4);
  expect((await agentStarts()).length).toBe(4);

  await expect
    .poll(() =>
      page.evaluate(() => {
        const saved = JSON.parse(
          localStorage.getItem("test-session") ?? "null",
        );
        return saved?.projects?.[0]?.workspaces?.[0]?.tabs?.length ?? 0;
      }),
    )
    .toBe(2);
  expect(
    await page.evaluate(() =>
      JSON.stringify(
        JSON.parse(localStorage.getItem("test-session") ?? "null"),
      ).includes('"cliLaunch"'),
    ),
  ).toBe(false);

  await page.reload();
  await expect(page.getByRole("tab")).toHaveCount(2);
  await page.getByRole("tab", { name: "Cursor CLI", exact: true }).click();
  await expect(page.locator(".terminal-host")).toHaveCount(4);
  for (const host of await page.locator(".terminal-host").all())
    await expect(host).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls
        .filter((call: any) => call.command === "start_terminal")
        .map((call: any) => call.args.request.cliLaunch)
        .filter(Boolean),
    ),
  ).toEqual([]);
});

test("Agents handles empty and failed discovery and can retry", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("status")).toHaveText(
    "No supported agent CLI found. Install a CLI, then refresh.",
  );
  await expect(
    dialog.getByRole("button", { name: "Launch agents" }),
  ).toBeDisabled();

  await page.evaluate(() => {
    (window as any).__nativeTest.installedAgentCliError =
      "Agent detection is unavailable";
  });
  await dialog.getByRole("button", { name: "Refresh installed CLI" }).click();
  await expect(dialog.getByRole("alert")).toHaveText(
    "Agent detection is unavailable",
  );

  await page.evaluate((agents) => {
    const mock = (window as any).__nativeTest;
    mock.installedAgentCliError = "";
    mock.installedAgentClis = agents;
  }, installedAgents);
  await dialog.getByRole("button", { name: "Refresh installed CLI" }).click();
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  const discoveryCalls = await page.evaluate(
    () => (window as any).__nativeTest.installedAgentCliCalls,
  );
  expect(discoveryCalls.length).toBeGreaterThanOrEqual(3);
  expect(discoveryCalls).toEqual(
    Array.from(discoveryCalls, () => ({
      profileId: "local:bash",
      cwd: "/project",
    })),
  );
});

test("Agents cancel and Escape restore focus to the New tab button", async ({
  page,
}) => {
  await setupAgents(page);
  const trigger = page.getByRole("button", { name: /^New tab/ });

  let dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(trigger).toBeFocused();

  dialog = await openAgents(page);
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test("Agents validates the terminal count and supports another installed CLI", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await setupAgents(page);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await dialog.getByRole("radio", { name: /Claude Code/ }).check();
  await expect(dialog.getByLabel("Number of terminals")).toHaveValue("4");
  await page.setViewportSize({ width: 560, height: 640 });
  await dialog.screenshot({
    path: testInfo.outputPath("agents-dialog-dark-small.png"),
  });

  const count = dialog.getByLabel("Number of terminals");
  await count.fill("1.5");
  expect(
    await count.evaluate((input: HTMLInputElement) => input.checkValidity()),
  ).toBe(false);
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) =>
          call.command === "start_terminal" && call.args.request.cliLaunch,
      ),
    ),
  ).toEqual([]);

  await count.fill("0");
  expect(
    await count.evaluate((input: HTMLInputElement) => input.checkValidity()),
  ).toBe(false);
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toBeVisible();

  await count.fill("2");
  await page.setViewportSize({ width: 1440, height: 900 });
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(
    page.getByRole("tab", { name: "Claude Code", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".terminal-pane")).toHaveCount(2);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) =>
              call.command === "start_terminal" &&
              call.args.request.cliLaunch === "claude",
          ).length,
      ),
    )
    .toBe(2);
});

test("a native agent startup error is visible and does not trigger a duplicate start", async ({
  page,
}) => {
  await setupAgents(page, [installedAgents[0]]);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(1);
  await page.evaluate(() => {
    (window as any).__nativeTest.agentStartupFailures = 1;
  });
  await dialog.getByRole("button", { name: "Launch agents" }).click();

  await expect(dialog).toHaveCount(0);
  await expectRecordedNotification(
    page,
    "1 of 4 agents could not start. Agent CLI failed to start",
  );
  const cliRequests = () =>
    page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) =>
          call.command === "start_terminal" && call.args.request.cliLaunch,
      ),
    );
  await expect.poll(async () => (await cliRequests()).length).toBe(4);
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("tab", { name: "Cursor CLI", exact: true }).click();
  const visiblePanels = page.locator(".terminal-pane:visible");
  await expect(visiblePanels).toHaveCount(4);
  for (const panel of await visiblePanels.all())
    await panel.click({ position: { x: 10, y: 10 } });
  await expectRecordedNotification(page, "1 of 4 agents could not start");
  expect((await cliRequests()).length).toBe(4);
});

for (const count of [1, 3, 5]) {
  test(`Agents starts exactly ${count} CLI panels in one new tab`, async ({
    page,
  }) => {
    await setupAgents(page, [installedAgents[0]]);
    const dialog = await openAgents(page);
    await expect(dialog.getByRole("radio")).toHaveCount(1);
    await dialog.getByLabel("Number of terminals").fill(String(count));
    await dialog.getByRole("button", { name: "Launch agents" }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole("tab")).toHaveCount(2);
    await expect(page.locator(".terminal-pane")).toHaveCount(count);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls.filter(
              (call: any) =>
                call.command === "start_terminal" &&
                call.args.request.cliLaunch === "cursor",
            ).length,
        ),
      )
      .toBe(count);
    for (const panel of await page.locator(".terminal-pane").all())
      await expect(panel).toBeVisible();
  });
}

test("startup prewarms discovery and reopening uses the completed scan immediately", async ({
  page,
}) => {
  await setupAgents(page, installedAgents, true);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.installedAgentCliCalls.length,
      ),
    )
    .toBe(1);
  await expect(page.getByRole("dialog", { name: "Agents" })).toHaveCount(0);
  await releaseScans(page);
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const { cachedInstalledAgentClis } = await import(
          performance
            .getEntriesByType("resource")
            .map((entry) => entry.name)
            .filter(
              (url) => new URL(url).pathname === "/src/installed-agent-clis.ts",
            )
            .at(-1)!
        );
        const info = await (window as any).__TAURI_INTERNALS__.invoke(
          "app_info",
        );
        return cachedInstalledAgentClis(info.profiles[0], "/project")?.length;
      }),
    )
    .toBe(2);
  for (let index = 0; index < 2; index++) {
    const dialog = await openAgents(page);
    // Inspect the first rendered dialog without waiting for discovery to finish.
    expect(await dialog.getByRole("radio").count()).toBe(2);
    await expect(
      dialog.getByRole("button", { name: "Launch agents" }),
    ).toBeEnabled();
    await expect(
      dialog.getByRole("radio", { name: /Cursor CLI/ }),
    ).toBeChecked();
    await dialog.getByRole("button", { name: "Cancel" }).click();
  }
  expect(
    await page.evaluate(
      () => (window as any).__nativeTest.installedAgentCliCalls.length,
    ),
  ).toBe(1);
});

test("refresh retains usable choices and selection through pending discovery and errors", async ({
  page,
}) => {
  await setupAgents(page);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await dialog.getByRole("radio", { name: /Claude Code/ }).check();
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.installedAgentCliBlocked = true;
    mock.installedAgentCliError = "Refresh failed";
  });
  await dialog.getByRole("button", { name: "Refresh installed CLI" }).click();
  await expect(dialog.getByRole("status")).toBeEmpty();
  expect(await dialog.getByRole("radio").count()).toBe(2);
  await expect(
    dialog.getByRole("radio", { name: /Claude Code/ }),
  ).toBeChecked();
  await expect(
    dialog.getByRole("button", { name: "Launch agents" }),
  ).toBeEnabled();
  await expect(
    dialog.getByRole("button", { name: "Refresh installed CLI" }),
  ).toBeDisabled();
  await releaseScans(page);
  await expect(dialog.getByRole("alert")).toHaveText("Refresh failed");
  await expect(
    dialog.getByRole("radio", { name: /Claude Code/ }),
  ).toBeChecked();
  await expect(
    dialog.getByRole("button", { name: "Launch agents" }),
  ).toBeEnabled();
  await dialog.getByLabel("Number of terminals").fill("1");
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("tab", { name: "Claude Code", exact: true }),
  ).toBeVisible();
});

test("cache isolates directories and complete shell profiles and refresh waits for pending scans", async ({
  page,
}) => {
  await setupAgents(page, [], true);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.installedAgentCliCalls.length,
      ),
    )
    .toBe(1);
  const observed = await page.evaluate(async () => {
    const service = await import(
      performance
        .getEntriesByType("resource")
        .map((entry) => entry.name)
        .filter(
          (url) => new URL(url).pathname === "/src/installed-agent-clis.ts",
        )
        .at(-1)!
    );
    const mock = (window as any).__nativeTest;
    const info = await (window as any).__TAURI_INTERNALS__.invoke("app_info");
    const profile = info.profiles[0];
    const first = service.loadInstalledAgentClis(profile, "/project");
    const second = service.loadInstalledAgentClis(profile, "/project");
    const refreshed = service.loadInstalledAgentClis(profile, "/project", true);
    const forcedAgain = service.loadInstalledAgentClis(
      profile,
      "/project",
      true,
    );
    const before = mock.installedAgentCliCalls.length;
    mock.installedAgentCliBlocked = false;
    mock.installedAgentCliReleases
      .splice(0)
      .forEach((release: () => void) => release());
    await refreshed;
    const after = mock.installedAgentCliCalls.length;
    const emptyCached = service.cachedInstalledAgentClis(profile, "/project");
    await service.loadInstalledAgentClis(profile, "/project");
    const emptyReused = mock.installedAgentCliCalls.length === after;
    const otherDirectory = service.cachedInstalledAgentClis(profile, "/other");
    const modifiedProfile = service.cachedInstalledAgentClis(
      { ...profile, program: "/other/bash" },
      "/project",
    );
    await service.loadInstalledAgentClis(profile, "/other");
    await service.loadInstalledAgentClis(
      { ...profile, program: "/other/bash" },
      "/project",
    );
    return {
      deduplicated: first === second,
      coalesced: refreshed === forcedAgain,
      before,
      after,
      emptyCached,
      emptyReused,
      otherDirectory,
      modifiedProfile,
      calls: mock.installedAgentCliCalls,
    };
  });
  expect(observed).toMatchObject({
    deduplicated: true,
    coalesced: true,
    before: 1,
    after: 2,
    emptyCached: [],
    emptyReused: true,
    otherDirectory: undefined,
    modifiedProfile: undefined,
  });
  expect(observed.calls).toEqual([
    { profileId: "local:bash", cwd: "/project" },
    { profileId: "local:bash", cwd: "/project" },
    { profileId: "local:bash", cwd: "/other" },
    { profileId: "local:bash", cwd: "/project" },
  ]);
});

test("stale cached choices remain launchable during background revalidation", async ({
  page,
}) => {
  await setupAgents(page);
  let dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.installedAgentCliBlocked = true;
    const now = Date.now;
    mock.restoreClock = () => {
      Date.now = now;
    };
    Date.now = () => now() + 5 * 60 * 1000 + 1;
  });
  dialog = await openAgents(page);
  expect(await dialog.getByRole("radio").count()).toBe(2);
  await expect(dialog.getByRole("status")).toBeEmpty();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.installedAgentCliCalls.length,
      ),
    )
    .toBe(2);
  await page.evaluate(() => (window as any).__nativeTest.restoreClock());
  await dialog.getByLabel("Number of terminals").fill("1");
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("tab", { name: "Cursor CLI", exact: true }),
  ).toBeVisible();
  await releaseScans(page);
});

test("CLI icons and terminal quantity can be configured and launched entirely by mouse", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.setViewportSize({ width: 800, height: 420 });
  await setupAgents(page, [
    { cli: "codex", name: "Codex", command: "codex" },
    ...installedAgents,
    { cli: "gemini", name: "Gemini CLI", command: "gemini" },
    { cli: "copilot", name: "GitHub Copilot CLI", command: "copilot" },
    { cli: "opencode", name: "OpenCode", command: "opencode" },
    { cli: "grok", name: "Grok Build", command: "grok" },
    { cli: "hermes", name: "Hermes Agent", command: "hermes" },
  ]);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(8);
  await expect(
    dialog.getByRole("radio", { name: "Codex", exact: true }),
  ).toBeInViewport();
  await expect(
    dialog.getByRole("radio", { name: "Hermes Agent", exact: true }),
  ).toBeInViewport();
  const count = dialog.getByLabel("Number of terminals");
  const decrease = dialog.getByRole("button", {
    name: "Decrease terminal count",
    exact: true,
  });
  const increase = dialog.getByRole("button", {
    name: "Increase terminal count",
    exact: true,
  });
  await dialog.getByRole("radio", { name: "Claude Code", exact: true }).click();
  await expect(
    dialog.getByRole("radio", { name: "Claude Code", exact: true }),
  ).toBeChecked();
  await dialog
    .getByRole("button", { name: "Use 1 terminal", exact: true })
    .click();
  await expect(count).toHaveValue("1");
  await expect(decrease).toBeDisabled();
  await increase.click();
  await expect(count).toHaveValue("2");
  await expect(
    dialog.getByRole("button", { name: "Use 2 terminals", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await dialog
    .getByRole("button", { name: "Use 4 terminals", exact: true })
    .click();
  await decrease.click();
  await expect(count).toHaveValue("3");
  await increase.click();
  await expect(count).toHaveValue("4");
  await dialog
    .getByRole("button", { name: "Use 2 terminals", exact: true })
    .click();
  const launch = dialog.getByRole("button", {
    name: "Launch agents",
    exact: true,
  });
  await expect(launch).toBeInViewport();
  expect(
    await dialog.evaluate(
      (element) => element.scrollWidth <= element.clientWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("agents-mouse-dark-minimum.png"),
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({
    path: testInfo.outputPath("agents-mouse-expanded.png"),
  });
  await launch.click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator(".terminal-pane")).toHaveCount(2);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) =>
              call.command === "start_terminal" &&
              call.args.request.cliLaunch === "claude",
          ).length,
      ),
    )
    .toBe(2);
});

test("Agents limits quantity to the available stage and recalculates on resize", async ({
  page,
}, testInfo) => {
  await setupAgents(page);
  const dialog = await openAgents(page);
  const count = dialog.getByLabel("Number of terminals");
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  const initialMaximum = Number(await count.getAttribute("max"));
  expect(initialMaximum).toBeGreaterThan(8);
  await count.fill("999999999");
  await expect(
    dialog.getByRole("button", { name: "Launch agents" }),
  ).toBeDisabled();
  await expect(dialog.locator(".agents-layout-summary")).toHaveText(
    `Only ${initialMaximum} agents fit in the current window.`,
  );
  // Bypassing the disabled submit button must not bypass count validation.
  await dialog
    .locator("form")
    .evaluate((form: HTMLFormElement) => form.requestSubmit());
  await expect(dialog.getByRole("alert")).toHaveText(
    `Only ${initialMaximum} agents fit in the current window.`,
  );
  await expect(page.getByRole("tab")).toHaveCount(1);
  await count.fill(String(initialMaximum));
  await page.setViewportSize({ width: 800, height: 420 });
  await expect
    .poll(async () => Number(await count.getAttribute("max")))
    .toBeLessThan(initialMaximum);
  const maximum = Number(await count.getAttribute("max"));
  expect(maximum).toBeGreaterThan(1);
  expect(maximum).toBeLessThan(8);
  await expect(count).toHaveValue(String(maximum));
  await expect(
    dialog.getByRole("button", { name: "Increase terminal count" }),
  ).toBeDisabled();
  await expect(
    dialog.getByRole("button", { name: "Use 8 terminals", exact: true }),
  ).toBeDisabled();
  await dialog.screenshot({
    path: testInfo.outputPath("agents-capacity-small.png"),
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(count).toHaveAttribute("max", String(initialMaximum));
  await expect(count).toHaveValue(String(maximum));
  await expect(
    dialog.getByRole("button", { name: "Increase terminal count" }),
  ).toBeEnabled();
  await expect(
    dialog.getByRole("button", { name: "Use 8 terminals", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) =>
          call.command === "start_terminal" && call.args.request.cliLaunch,
      ),
    ),
  ).toEqual([]);
});

test("Agents launches the maximum count in a wide short window with every pane fitting", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1300, height: 420 });
  await setupAgents(page);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  const count = dialog.getByLabel("Number of terminals");
  const maximum = Number(await count.getAttribute("max"));
  expect(maximum).toBeGreaterThan(4);
  await count.fill(String(maximum));
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator(".terminal-pane")).toHaveCount(maximum);
  await expect(page.locator(".layout-recovery")).toHaveCount(0);
  for (const panel of await page.locator(".terminal-pane").all()) {
    const bounds = await panel.boundingBox();
    expect(bounds!.width).toBeGreaterThanOrEqual(240);
    expect(bounds!.height).toBeGreaterThanOrEqual(120);
  }
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) =>
              call.command === "start_terminal" && call.args.request.cliLaunch,
          ).length,
      ),
    )
    .toBe(maximum);
});

test("Agents rechecks stage space synchronously before starting terminals", async ({
  page,
}) => {
  await setupAgents(page);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await expect(dialog.getByLabel("Number of terminals")).toHaveValue("4");
  // Change geometry and submit in one task, before ResizeObserver updates the UI.
  await dialog.locator("form").evaluate((form: HTMLFormElement) => {
    const stage = document.querySelector<HTMLElement>(".terminal-stage")!;
    stage.style.flex = "0 0 300px";
    stage.style.height = "130px";
    stage.style.padding = "0";
    form.requestSubmit();
  });
  await expect(dialog.getByRole("alert")).toHaveText(
    "Only 1 agent fits in the current window.",
  );
  await expect(page.getByRole("tab")).toHaveCount(1);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) =>
          call.command === "start_terminal" && call.args.request.cliLaunch,
      ),
    ),
  ).toEqual([]);
});

test("Agents measures content space from a file tab with both sidebars and padding", async ({
  page,
}) => {
  let session = addWorkspace(newSession(), "/project", "local:bash");
  session = openFileTab(
    session,
    active(session)!.workspace.id,
    "/project",
    "README.md",
  );
  session.sidebar = "files";
  session.rightSidebar = "workspaces";
  session.sidebarSides.workspaces = "right";
  await mockDesktop(page, false, session, undefined, undefined, undefined, {
    installed: installedAgents,
  });
  await page.goto("/");
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect(page.locator(".terminal-layout")).toHaveCount(0);
  await expect(
    page.getByRole("complementary", { name: "Explorer", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("complementary", { name: "Workspaces", exact: true }),
  ).toBeVisible();
  await page.locator(".terminal-stage").evaluate((stage: HTMLElement) => {
    stage.style.padding = "40px";
    stage.style.border = "2px solid transparent";
  });
  const dialog = await openAgents(page);
  const maximum = Number(
    await dialog.getByLabel("Number of terminals").getAttribute("max"),
  );
  expect(maximum).toBeGreaterThan(0);
  const stage = await page.locator(".terminal-stage").boundingBox();
  expect(stage!.width).toBeLessThan(1000);
  // Content space excludes the 40px padding and 2px border on all sides.
  const expected =
    Math.floor((stage!.width - 84 + 3) / 243) *
    Math.floor((stage!.height - 84 + 3) / 123);
  expect(maximum).toBe(expected);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await page.locator(".terminal-stage").evaluate((stage: HTMLElement) => {
    stage.style.height = "100px";
  });
  const smallDialog = await openAgents(page);
  await expect(smallDialog.getByLabel("Number of terminals")).toHaveAttribute(
    "max",
    "0",
  );
  await expect(
    smallDialog.getByRole("button", { name: "Launch agents" }),
  ).toBeDisabled();
  await expect(smallDialog.locator(".agents-layout-summary")).toHaveText(
    "Not enough space for a terminal.",
  );
  await page.locator(".terminal-stage").evaluate((stage: HTMLElement) => {
    stage.style.height = "";
  });
  await expect(smallDialog.getByLabel("Number of terminals")).toHaveAttribute(
    "max",
    String(expected),
  );
  await expect(
    smallDialog.getByRole("button", { name: "Launch agents" }),
  ).toBeEnabled();
});
