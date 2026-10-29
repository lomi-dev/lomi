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
} from "../../src/model";
import { mockAndroid } from "./android-mock";
import { mockDesktop } from "./desktop";

function panelFixture() {
  let session = addWorkspace(
    newSession(),
    "/work/panels",
    "local:bash",
    "Panels",
  );
  const workspace = session.projects[0].workspaces[0];
  const mixed = newTab("/work/panels", "local:bash", "Shells", 2);
  const browser = newBrowserTab("https://example.test/panel");
  mixed.layout = splitPane(
    mixed.layout,
    mixed.activePaneId,
    "horizontal",
    browser,
    true,
  );

  const firstReadme = {
    type: "file" as const,
    id: "panel-readme-a",
    title: "README.md",
    root: "/work/panels",
    relative: "README.md",
  };
  const secondReadme = { ...firstReadme, id: "panel-readme-b" };
  const android = newAndroidTab(null, "Phone placeholder");
  const chat = newChatTab("missing-panel-conversation", "Chat placeholder");
  const task = newAgentTaskTab("missing-panel-task", "Task placeholder");
  const plugin = {
    type: "plugin" as const,
    id: "panel-plugin",
    title: "Plugin placeholder",
    owner: "local.missing",
    viewType: "local.missing.view",
    stateVersion: 1,
    state: {},
  };
  const commit = {
    type: "commit" as const,
    id: "panel-commit",
    title: "feat(ui): reveal the selected panel",
    root: "/work/panels",
    commit: "c".repeat(40),
  };
  const diff = {
    type: "diff" as const,
    id: "panel-diff",
    title: "README diff",
    root: "/work/panels",
    relative: "README.md",
    staged: false,
  };
  workspace.tabs = [
    mixed,
    firstReadme,
    secondReadme,
    android,
    chat,
    task,
    plugin,
    commit,
    diff,
  ];
  workspace.activeTabId = mixed.id;

  session = addWorkspace(session, "/work/review", "local:bash", "Review");
  session.sidebar = "workspaces";

  const commitSummary = {
    id: commit.commit,
    shortId: "ccccccc",
    subject: commit.title,
    authorName: "Fixture author",
    authoredAt: "2026-10-25T09:00:00Z",
  };
  const history = {
    commits: [commitSummary],
    details: {
      [commit.commit]: {
        commit: commitSummary,
        authorEmail: "fixture@example.test",
        committerName: "Fixture author",
        committerEmail: "fixture@example.test",
        committedAt: commitSummary.authoredAt,
        parents: [],
        message: commit.title,
        files: [],
      },
    },
    diffs: {},
  };
  const panels = [
    ...layoutPanes(mixed.layout).map((pane) => ({
      paneId: pane.id,
      ownerTabId: mixed.id,
      type: pane.type,
    })),
    ...workspace.tabs.slice(1).map((tab) => ({
      paneId: tab.id,
      ownerTabId: tab.id,
      type: tab.type,
    })),
  ];
  return {
    session,
    workspace,
    mixed,
    browser,
    firstReadme,
    secondReadme,
    android,
    chat,
    task,
    plugin,
    commit,
    diff,
    history,
    panels,
    terminals: layoutPanes(mixed.layout).filter(
      (pane) => pane.type === "terminal",
    ),
  };
}

function panelItem(page: Page, paneId: string) {
  return page.locator(
    `.workspace-open-item[data-workspace-pane-id="${paneId}"]`,
  );
}

function revealTarget(page: Page, paneId: string) {
  return page.locator(`[data-desktop-panel-id="${paneId}"]`);
}

async function nativeCallCount(page: Page, command: string) {
  return page.evaluate(
    (name) =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === name,
      ).length,
    command,
  );
}

async function expectVisibleRevealFrame(page: Page, paneId: string) {
  const target = revealTarget(page, paneId);
  const style = await target.evaluate((element) => {
    const frame = getComputedStyle(element, "::after");
    return {
      name: frame.animationName,
      opacity: frame.opacity,
      borderTopWidth: frame.borderTopWidth,
    };
  });
  expect(style).toEqual({
    name: "panel-reveal",
    opacity: "1",
    borderTopWidth: "2px",
  });
}

async function expectSelectedPanel(
  page: Page,
  paneId: string,
  ownerTabId: string,
) {
  const item = panelItem(page, paneId);
  await expect(item).toHaveAttribute("aria-current", "true");
  await expect(page.getByRole("tab", { selected: true })).toHaveAttribute(
    "id",
    `tab-${ownerTabId}`,
  );
  const target = revealTarget(page, paneId);
  await expect(target).toHaveClass(/is-panel-revealed/);
  const bounds = await target.boundingBox();
  expect(bounds?.width).toBeGreaterThan(0);
  expect(bounds?.height).toBeGreaterThan(0);
  return target;
}

test("workspace list flattens every panel in layout order without opening inactive panels", async ({
  page,
}) => {
  const fixture = panelFixture();
  await mockDesktop(page, false, fixture.session, fixture.history);
  await mockAndroid(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect.poll(() => nativeCallCount(page, "start_terminal")).toBe(1);

  const entry = page.locator(
    `.workspace-list-entry[data-workspace-id="${fixture.workspace.id}"]`,
  );
  await entry
    .getByRole("button", { name: "Expand items in Panels", exact: true })
    .click();
  const items = entry.getByRole("list", {
    name: "Open items in Panels",
    exact: true,
  });
  await expect(items).toBeVisible();
  const listed = items.locator(".workspace-open-item");
  await expect(listed).toHaveCount(fixture.panels.length);
  await expect
    .poll(() =>
      listed.evaluateAll((elements) =>
        elements.map((element) => ({
          paneId: element.getAttribute("data-workspace-pane-id"),
          ownerTabId: element.getAttribute("data-owner-tab-id"),
        })),
      ),
    )
    .toEqual(
      fixture.panels.map(({ paneId, ownerTabId }) => ({
        paneId,
        ownerTabId,
      })),
    );
  expect(
    await listed.locator(".workspace-item-label").allTextContents(),
  ).toEqual([
    "Browser",
    "Terminal 1",
    "Terminal 2",
    "README.md",
    "README.md",
    "Phone placeholder",
    "Chat placeholder",
    "Task placeholder",
    "Plugin placeholder",
    fixture.commit.title,
    "README diff",
  ]);
  await expect.poll(() => nativeCallCount(page, "start_terminal")).toBe(1);
  expect(await nativeCallCount(page, "read_editor_file")).toBe(0);
  expect(await nativeCallCount(page, "git_commit_details")).toBe(0);
  expect(await nativeCallCount(page, "git_diff")).toBe(0);
  expect(
    await page.evaluate(() => (window as any).__nativeTest.browsers.size),
  ).toBe(0);
  expect(
    await page.evaluate(() => (window as any).__androidTest.startRequests),
  ).toBe(0);
});

test("double-click reveals each supported panel at its own rendered bounds", async ({
  page,
}, testInfo) => {
  const fixture = panelFixture();
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page, false, fixture.session, fixture.history);
  await mockAndroid(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const entry = page.locator(
    `.workspace-list-entry[data-workspace-id="${fixture.workspace.id}"]`,
  );
  await entry
    .getByRole("button", { name: "Expand items in Panels", exact: true })
    .click();
  const items = entry.getByRole("list", {
    name: "Open items in Panels",
    exact: true,
  });
  await page.clock.install();
  await page.clock.pauseAt(await page.evaluate(() => Date.now()));
  await page.addStyleTag({
    content: `
      .is-panel-revealed::after {
        animation-delay: -350ms !important;
        animation-play-state: paused !important;
      }
    `,
  });

  const types = new Set(fixture.panels.map((panel) => panel.type));
  expect(types).toEqual(
    new Set([
      "terminal",
      "file",
      "browser",
      "android",
      "chat",
      "agent-task",
      "plugin",
      "commit",
      "diff",
    ]),
  );

  let darkBrowserBounds: unknown;
  for (const panel of fixture.panels) {
    await panelItem(page, panel.paneId).dblclick();
    await page.clock.runFor(32);
    const target = await expectSelectedPanel(
      page,
      panel.paneId,
      panel.ownerTabId,
    );
    if (panel.paneId === fixture.browser.id) {
      await expect(page.locator("html")).toHaveAttribute(
        "data-appearance",
        "dark",
      );
      await expectVisibleRevealFrame(page, panel.paneId);
      await page.screenshot({
        path: testInfo.outputPath("workspace-panels-browser-mixed-dark.png"),
      });
      await expect
        .poll(() =>
          page.evaluate((id) => {
            const pane = document.querySelector(
              `[data-browser-pane-id="${CSS.escape(id)}"] .browser-viewport`,
            );
            const bounds = (window as any).__nativeTest.browsers.get(
              id,
            )?.bounds;
            if (!pane || !bounds) return null;
            const rect = pane.getBoundingClientRect();
            return {
              bounds,
              rect: {
                x: rect.left,
                y: rect.top,
                width: rect.width,
                height: rect.height,
              },
            };
          }, fixture.browser.id),
        )
        .not.toBeNull();
      darkBrowserBounds = await page.evaluate((id) => {
        const pane = document.querySelector(
          `[data-browser-pane-id="${CSS.escape(id)}"] .browser-viewport`,
        )!;
        const rect = pane.getBoundingClientRect();
        const bounds = (window as any).__nativeTest.browsers.get(id).bounds;
        return {
          bounds,
          rect: {
            x: rect.left,
            y: rect.top,
            width: rect.width,
            height: rect.height,
          },
        };
      }, fixture.browser.id);
      const measured = darkBrowserBounds as {
        bounds: { x: number; y: number; width: number; height: number };
        rect: { x: number; y: number; width: number; height: number };
      };
      expect(measured.bounds.x).toBeCloseTo(measured.rect.x, 1);
      expect(measured.bounds.y).toBeCloseTo(measured.rect.y, 1);
      expect(measured.bounds.width).toBeCloseTo(measured.rect.width, 1);
      expect(measured.bounds.height).toBeCloseTo(measured.rect.height, 1);
      expect(await target.getAttribute("data-desktop-panel-id")).toBe(
        fixture.browser.id,
      );
    }
  }

  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  await panelItem(page, fixture.browser.id).dblclick();
  await page.clock.runFor(32);
  await expectSelectedPanel(page, fixture.browser.id, fixture.mixed.id);
  await expectVisibleRevealFrame(page, fixture.browser.id);
  await page.screenshot({
    path: testInfo.outputPath("workspace-panels-browser-mixed-light.png"),
  });
  const lightBrowserBounds = await page.evaluate((id) => {
    const pane = document.querySelector(
      `[data-browser-pane-id="${CSS.escape(id)}"] .browser-viewport`,
    )!;
    const rect = pane.getBoundingClientRect();
    const bounds = (window as any).__nativeTest.browsers.get(id).bounds;
    return {
      bounds,
      rect: {
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height,
      },
    };
  }, fixture.browser.id);
  expect(lightBrowserBounds.bounds).toEqual(
    (darkBrowserBounds as { bounds: unknown }).bounds,
  );
  expect(await nativeCallCount(page, "start_terminal")).toBe(3);
  expect(await nativeCallCount(page, "close_terminal")).toBe(0);
  expect(
    await page.evaluate(() => (window as any).__androidTest.startRequests),
  ).toBe(0);
});

test("single and double clicks select exact owners, cancel and restart the frame, and return from Settings safely", async ({
  page,
}) => {
  const fixture = panelFixture();
  await mockDesktop(page, false, fixture.session, fixture.history);
  await mockAndroid(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const entry = page.locator(
    `.workspace-list-entry[data-workspace-id="${fixture.workspace.id}"]`,
  );
  await entry
    .getByRole("button", { name: "Expand items in Panels", exact: true })
    .click();
  const items = entry.getByRole("list", {
    name: "Open items in Panels",
    exact: true,
  });
  const firstFile = panelItem(page, fixture.firstReadme.id);
  const secondFile = panelItem(page, fixture.secondReadme.id);
  const firstTarget = revealTarget(page, fixture.firstReadme.id);
  const secondTarget = revealTarget(page, fixture.secondReadme.id);
  const validateCount = await nativeCallCount(page, "validate_directory");

  await page.getByRole("button", { name: /^Toggle terminal overview/ }).click();
  await expect(page.locator(".terminal-overview")).toBeVisible();
  await firstFile.click();
  await expect(firstFile).toHaveAttribute("aria-current", "true");
  await expect(page.getByRole("tab", { selected: true })).toHaveAttribute(
    "id",
    `tab-${fixture.firstReadme.id}`,
  );
  await expect(page.locator(".terminal-overview")).toHaveCount(0);
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect(firstTarget).not.toHaveClass(/is-panel-revealed/);
  await expect(firstFile.locator(".workspace-notification-dot")).toHaveCount(0);
  expect(await nativeCallCount(page, "validate_directory")).toBe(validateCount);

  await firstFile.dblclick();
  await expect(firstTarget).toHaveClass(/is-panel-revealed/);
  await secondFile.click();
  await expect(secondFile).toHaveAttribute("aria-current", "true");
  await expect(page.getByRole("tab", { selected: true })).toHaveAttribute(
    "id",
    `tab-${fixture.secondReadme.id}`,
  );
  await expect(firstTarget).toHaveCount(0);
  await expect(secondTarget).not.toHaveClass(/is-panel-revealed/);

  await firstFile.dblclick();
  await expect(firstTarget).toHaveClass(/is-panel-revealed/);
  await page.waitForTimeout(700);
  await firstFile.dblclick();
  await expect(firstTarget).toHaveClass(/is-panel-revealed/);
  await page.waitForTimeout(500);
  await expect(firstTarget).toHaveClass(/is-panel-revealed/);
  await page.waitForTimeout(600);
  await expect(firstTarget).not.toHaveClass(/is-panel-revealed/);

  const firstTerminal = fixture.terminals[0];
  const secondTerminal = fixture.terminals[1];
  const firstTerminalItem = panelItem(page, firstTerminal.id);
  const secondTerminalItem = panelItem(page, secondTerminal.id);
  await firstTerminalItem.click();
  const firstTerminalPane = page.locator(
    `.terminal-pane[data-pane-id="${firstTerminal.id}"]`,
  );
  await page.keyboard.down("Control");
  const maximizeButton = firstTerminalPane.getByRole("button", {
    name: "Maximize terminal",
    exact: true,
  });
  await expect(maximizeButton).toBeVisible();
  await page.keyboard.up("Control");
  await maximizeButton.click();
  await expect(
    firstTerminalPane.getByRole("button", {
      name: "Restore terminal size",
      exact: true,
    }),
  ).toBeVisible();
  await secondTerminalItem.dblclick();
  const secondTerminalPane = page.locator(
    `.terminal-pane[data-pane-id="${secondTerminal.id}"]`,
  );
  await expect(revealTarget(page, secondTerminal.id)).toHaveClass(
    /is-panel-revealed/,
  );
  await expect(firstTerminalPane).toBeVisible();
  await expect(secondTerminalPane).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Restore terminal size", exact: true }),
  ).toHaveCount(0);

  await secondTerminalItem.dblclick();
  await expect(revealTarget(page, secondTerminal.id)).toHaveClass(
    /is-panel-revealed/,
  );
  await page.getByRole("button", { name: "Open account menu" }).click();
  await page.getByRole("menuitem", { name: /^Settings/ }).click();
  const settingsTab = page.getByRole("tab", {
    name: "Settings",
    exact: true,
  });
  await expect(settingsTab).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".terminal-stage.is-panel-revealed")).toHaveCount(
    0,
  );
  await expect(page.locator(".terminal-stage .is-panel-revealed")).toHaveCount(
    0,
  );
  await page.getByRole("tab", { name: "Shells", exact: true }).click();
  await expect(settingsTab).toHaveAttribute("aria-selected", "false");
  await expect(secondTerminalPane).toBeVisible();
  await expect(revealTarget(page, secondTerminal.id)).not.toHaveClass(
    /is-panel-revealed/,
  );
  await expect.poll(() => nativeCallCount(page, "start_terminal")).toBe(3);
  await expect.poll(() => nativeCallCount(page, "close_terminal")).toBe(0);

  await page.emulateMedia({ reducedMotion: "reduce" });
  await secondFile.dblclick();
  await expect(secondTarget).toHaveClass(/is-panel-revealed/);
  const reducedMotionStyle = await secondTarget.evaluate((element) => {
    const frame = getComputedStyle(element, "::after");
    return { animationName: frame.animationName, opacity: frame.opacity };
  });
  expect(reducedMotionStyle).toEqual({ animationName: "none", opacity: "1" });
  await page.waitForTimeout(1100);
  await expect(secondTarget).not.toHaveClass(/is-panel-revealed/);
  expect(await nativeCallCount(page, "validate_directory")).toBe(validateCount);
  await expect(items).toBeVisible();
});
