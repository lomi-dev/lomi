import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockAndroid } from "./android-mock";

const settingsTab = (page: Page) =>
  page.getByRole("tab", { name: "Settings", exact: true });
const calls = (page: Page, command: string) =>
  page.evaluate(
    (command) =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === command,
      ),
    command,
  );
const requestSettings = (page: Page, requestedPage: string | null = null) =>
  page.evaluate(
    (requestedPage) =>
      (window as any).__TAURI_INTERNALS__.invoke("open_settings", {
        page: requestedPage,
      }),
    requestedPage,
  );

test("menu, shortcut and native requests reuse one Settings tab after workspace tabs", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const terminal = page.getByRole("tab").first();
  const terminalName = await terminal.getAttribute("aria-label");
  await page.getByRole("button", { name: "Open account menu" }).click();
  await page.getByRole("menuitem", { name: /^Settings/ }).click();
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await expect(
    page
      .getByRole("navigation", { name: "Settings pages" })
      .getByRole("button", { name: "Keybinds", exact: true }),
  ).toBeFocused();
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(page.getByRole("tab").last()).toHaveAccessibleName("Settings");
  await page.keyboard.press("Control+Comma");
  await requestSettings(page, "terminal");
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await requestSettings(page, "themes");
  await expect(
    page.getByRole("heading", { name: "Themes", exact: true }),
  ).toBeVisible();
  await expect(settingsTab(page)).toHaveCount(1);
  await page.keyboard.press("Control+PageDown");
  await expect(terminal).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".xterm-helper-textarea")).toBeFocused();
  await page.keyboard.type("settings-return-sentinel");
  await expect
    .poll(async () =>
      (await calls(page, "write_terminal"))
        .map((call: any) => call.args.data)
        .join(""),
    )
    .toContain("settings-return-sentinel");
  await page.keyboard.press("Control+PageUp");
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await settingsTab(page).focus();
  await page.keyboard.press("ArrowLeft");
  await expect(terminal).toHaveAttribute("aria-selected", "true");
  await expect(terminal).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await expect(settingsTab(page)).toBeFocused();
  await page
    .getByRole("button", { name: "Close Settings", exact: true })
    .click();
  await expect(settingsTab(page)).toHaveCount(0);
  await expect(terminal).toHaveAttribute("aria-selected", "true");
  expect(await terminal.getAttribute("aria-label")).toBe(terminalName);
  expect(await calls(page, "start_terminal")).toHaveLength(1);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
});

for (const platform of ["linux", "macos"] as const) {
  test(`${platform} closes Settings to Welcome without closing the main window or starting a PTY`, async ({
    page,
  }) => {
    await mockDesktop(page, false, null, undefined, {}, platform);
    await page.goto("/");
    const welcome = page.getByRole("main", { name: "No project open" });
    await expect(welcome).toBeVisible();
    await page.keyboard.press(
      `${platform === "macos" ? "Meta" : "Control"}+Comma`,
    );
    await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
    await page.getByRole("heading", { name: "Keybinds", exact: true }).click();
    await page.keyboard.press(`${platform === "macos" ? "Meta" : "Control"}+w`);
    await expect(settingsTab(page)).toHaveCount(0);
    await expect(welcome).toBeVisible();
    expect(await calls(page, "start_terminal")).toHaveLength(0);
    expect(await calls(page, "plugin:window|close")).toHaveLength(0);
    expect(await calls(page, "plugin:window|destroy")).toHaveLength(0);
  });
}

test("startup handshake recovers a request sent before its listener is ready", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.addInitScript(() => {
    const native = (window as any).__nativeTest;
    native.settingsRevision = 7;
    native.settingsRequest = { revision: 7, page: "terminal" };
  });
  await page.goto("/");
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await page.evaluate(() =>
    (window as any).__nativeTest.emitEvent("settings-tab-requested", {
      revision: 6,
      page: "themes",
    }),
  );
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
  expect(await calls(page, "settings_tab_ready")).toHaveLength(1);
});

test("Settings preserves dirty editor text, undo history and running terminals", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const terminal = page.getByRole("tab").first();
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New file", exact: true }).click();
  const editorTabId = await page.getByRole("tab").last().getAttribute("id");
  const editorTab = page.locator(`[id="${editorTabId}"]`);
  const editor = page.locator(".cm-content");
  await expect(editor).toBeVisible();
  await page.keyboard.insertText("Keep this draft");
  await requestSettings(page);
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await terminal.click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await settingsTab(page).click();
  await editorTab.click();
  await expect(editor).toHaveText("Keep this draft");
  await editor.focus();
  await page.keyboard.press("Control+z");
  await expect(editor).toBeEmpty();
  await page.keyboard.press("Control+Shift+z");
  await expect(editor).toHaveText("Keep this draft");
  expect(await calls(page, "start_terminal")).toHaveLength(1);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("shortcut recording suppresses workspace commands and saves into the shared provider", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const terminal = page.getByRole("tab").first();
  await requestSettings(page);
  const recorder = page.getByRole("button", {
    name: "Shortcut for New terminal",
    exact: true,
  });
  await recorder.click();
  await recorder.press("Control+w");
  await expect(page.getByRole("alert")).toContainText("already assigned");
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await recorder.press("Escape");
  await recorder.click();
  await recorder.press("Control+k");
  await expect(recorder).toHaveText("Ctrl+K");
  expect(await calls(page, "start_terminal")).toHaveLength(1);
  await terminal.click();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+k");
  await expect(page.locator(".terminal-pane")).toHaveCount(2);
  await settingsTab(page).click();
  await expect(recorder).toHaveText("Ctrl+K");
  await page
    .getByRole("button", { name: "Close Settings", exact: true })
    .click();
  await requestSettings(page);
  await expect(recorder).toHaveText("Ctrl+K");
});

test("switching away retains an unsaved Settings field draft", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const terminal = page.getByRole("tab").first();
  await requestSettings(page, "terminal");
  const fontSize = page.getByRole("spinbutton", {
    name: "Font size",
    exact: true,
  });
  await expect(fontSize).toBeEnabled();
  await fontSize.fill("0");
  await terminal.click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await settingsTab(page).click();
  await expect(fontSize).toHaveValue("0");
  expect(await calls(page, "save_terminal_preferences")).toHaveLength(0);
});

test("a notification action navigates the existing Settings tab", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.addInitScript(() => {
    (window as any).__nativeTest.notifications = {
      revision: 1,
      items: [
        {
          id: "settings-action",
          kind: "attention",
          title: "Review MCP setup",
          body: "Open Agent control to review setup.",
          createdAt: Date.now(),
          read: false,
          source: "mcp",
          action: "open-agent-control-settings",
        },
      ],
    };
  });
  await page.goto("/?window=settings&page=terminal");
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Open account menu" }).click();
  await page
    .getByRole("menuitem", { name: "Notifications", exact: true })
    .click();
  const inbox = page.getByRole("dialog", {
    name: "Notifications",
    exact: true,
  });
  await inbox
    .getByRole("button", { name: "Agent control settings", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Agent control", exact: true }),
  ).toBeVisible();
  await expect(settingsTab(page)).toHaveCount(1);
  expect((await calls(page, "open_settings")).at(-1)?.args).toEqual({
    page: "agent-control",
  });
  expect(await calls(page, "start_terminal")).toHaveLength(0);
});

for (const appearance of ["light", "dark"] as const) {
  test(`Settings fits the main minimum width in ${appearance} appearance`, async ({
    page,
  }, testInfo) => {
    await mockDesktop(page);
    await page.emulateMedia({ colorScheme: appearance });
    await page.setViewportSize({ width: 800, height: 420 });
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    await requestSettings(page, "terminal");
    await expect(
      page.getByRole("heading", { name: "Terminal", exact: true }),
    ).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute(
      "data-appearance",
      appearance,
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(
      await page
        .locator(".terminal-settings-page")
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`settings-main-minimum-${appearance}.png`),
    });
    const about = page
      .getByRole("navigation", { name: "Settings pages" })
      .getByRole("button", { name: "About", exact: true });
    await about.scrollIntoViewIfNeeded();
    await expect(about).toBeInViewport();
    await about.click();
    await expect(
      page.getByRole("heading", { name: "About", exact: true }),
    ).toBeVisible();
  });
}

test("opening a phone into an existing Android setup target returns to its workspace tab", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockAndroid(page, true);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    (window as any).__androidTest.state.preferences.defaultDeviceId = null;
  });
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page
    .getByRole("menuitem", { name: "New android symulator", exact: true })
    .click();
  const androidTabId = await page.getByRole("tab").last().getAttribute("id");
  await page
    .getByRole("button", { name: "Manage devices", exact: true })
    .click();
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await page
    .getByRole("article", { name: "Test phone" })
    .getByRole("button", { name: "Open", exact: true })
    .click();
  await page.evaluate(async () => {
    const android = (window as any).__androidTest;
    await (window as any).__nativeTest.emitEvent("android-open-request", {
      id: crypto.randomUUID(),
      deviceId: android.deviceId,
      context: android.setup,
      coldBoot: false,
      deadlineMs: Date.now() + 15000,
    });
  });
  await expect(page.locator(`[id="${androidTabId}"]`)).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.locator(".android-screen")).toBeVisible();
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "false");
  expect(await calls(page, "start_terminal")).toHaveLength(1);
});

test("closing Settings restores the selected split terminal for typing", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+d");
  await expect(page.locator(".terminal-pane")).toHaveCount(2);
  const selected = page.locator(".terminal-pane.is-active");
  const selectedId = await selected.getAttribute("data-pane-id");
  await expect.poll(() => calls(page, "start_terminal")).toHaveLength(2);
  const selectedSession = (await calls(page, "start_terminal"))[1].args.request
    .id;
  expect(selectedSession).toBeTruthy();
  await requestSettings(page);
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await page
    .getByRole("button", { name: "Close Settings", exact: true })
    .click();
  await expect(page.locator(".terminal-pane.is-active")).toHaveAttribute(
    "data-pane-id",
    selectedId!,
  );
  await expect(selected.locator(".xterm-helper-textarea")).toBeFocused();
  await page.keyboard.type("second-split-sentinel");
  await expect
    .poll(async () =>
      (await calls(page, "write_terminal"))
        .filter((call: any) => call.args.id === selectedSession)
        .map((call: any) => call.args.data)
        .join(""),
    )
    .toContain("second-split-sentinel");
  await expect(page.locator(".terminal-pane.is-active")).toHaveAttribute(
    "data-pane-id",
    selectedId!,
  );
});

test("dragging a workspace tab to the end keeps Settings last and out of the saved session", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.keyboard.press("Control+Shift+t");
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(3);
  const first = page.getByRole("tab").first();
  const firstId = await first.getAttribute("id");
  await requestSettings(page);
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  const source = (await first.boundingBox())!;
  const target = (await settingsTab(page).boundingBox())!;
  await page.mouse.move(
    source.x + source.width / 2,
    source.y + source.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(target.x + 1, target.y + target.height / 2, {
    steps: 10,
  });
  await page.mouse.up();
  await expect(page.getByRole("tab").nth(2)).toHaveAttribute("id", firstId!);
  await expect(page.getByRole("tab").last()).toHaveAccessibleName("Settings");
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await expect
    .poll(() =>
      page.evaluate(() => {
        const saved = JSON.parse(
          localStorage.getItem("test-session") ?? "null",
        );
        return saved?.projects[0].workspaces[0].tabs.map(
          (tab: any) => `tab-${tab.id}`,
        );
      }),
    )
    .toEqual([
      await page.getByRole("tab").nth(0).getAttribute("id"),
      await page.getByRole("tab").nth(1).getAttribute("id"),
      firstId,
    ]);
});

test("a delayed ready snapshot preserves the pending deep link before a newer page-less request", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "settings_tab_ready") {
        desktop.__settingsReadyRequested = true;
        await new Promise<void>((resolve) => {
          desktop.__releaseSettingsReady = resolve;
        });
        return { revision: 1, page: "terminal" };
      }
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await expect
    .poll(() => page.evaluate(() => !!(window as any).__settingsReadyRequested))
    .toBe(true);
  await page.evaluate(async () => {
    const native = (window as any).__nativeTest;
    await native.emitEvent("settings-tab-requested", {
      revision: 2,
      page: null,
    });
    (window as any).__releaseSettingsReady();
  });
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
});

test("Commands offers Settings tab actions while disabling hidden terminal actions", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await requestSettings(page);
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Control+Shift+p");
  const commands = page.getByRole("dialog", { name: "Commands", exact: true });
  await expect(commands).toBeVisible();
  await expect(
    commands.getByRole("button", {
      name: "New terminal Terminals",
      exact: true,
    }),
  ).toBeDisabled();
  await expect(
    commands.getByRole("button", {
      name: "Run command input Terminals",
      exact: true,
    }),
  ).toBeDisabled();
  await expect(
    commands.getByRole("button", { name: "New tab Tabs", exact: true }),
  ).toBeEnabled();
  await expect(
    commands.getByRole("button", { name: "Close tab Tabs", exact: true }),
  ).toBeEnabled();
  await page.keyboard.press("Escape");
  expect(await calls(page, "write_terminal")).toHaveLength(0);
  expect(await calls(page, "start_terminal")).toHaveLength(1);
});

test("closing the underlying clean file keeps Settings selected and retains its draft", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New file", exact: true }).click();
  await expect(page.locator(".cm-content")).toBeVisible();
  const fileTabId = await page.getByRole("tab").last().getAttribute("id");
  const fileTab = page.locator(`[id="${fileTabId}"]`);
  await expect(fileTab).toHaveAttribute("aria-selected", "true");
  await requestSettings(page, "terminal");
  const fontSize = page.getByRole("spinbutton", {
    name: "Font size",
    exact: true,
  });
  await expect(fontSize).toBeEnabled();
  await fontSize.fill("0");
  await fileTab
    .locator("..")
    .getByRole("button", { name: /^Close / })
    .click();
  await expect(fileTab).toHaveCount(0);
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await expect(fontSize).toBeVisible();
  await expect(fontSize).toHaveValue("0");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await calls(page, "save_terminal_preferences")).toHaveLength(0);
  expect(await calls(page, "start_terminal")).toHaveLength(1);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
});

test("Settings clears the plugin host panel context and restores the selected terminal on return", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const terminal = page.getByRole("tab").first();
  const paneId = await page
    .locator(".terminal-pane.is-active")
    .getAttribute("data-pane-id");
  const context = () =>
    page.evaluate(async () => {
      const runtimeUrl = performance
        .getEntriesByType("resource")
        .map((entry) => entry.name)
        .filter((url) => new URL(url).pathname === "/src/plugins/runtime.ts")
        .at(-1);
      const { pluginHost } = await import(
        runtimeUrl ?? "/src/plugins/runtime.ts"
      );
      const { activePanelId, viewType, projectPath, workspaceName } =
        pluginHost.context;
      return { activePanelId, viewType, projectPath, workspaceName };
    });
  await expect
    .poll(context)
    .toMatchObject({ activePanelId: paneId, viewType: "terminal" });
  const before = await context();
  await requestSettings(page);
  await expect(settingsTab(page)).toHaveAttribute("aria-selected", "true");
  await expect
    .poll(context)
    .toEqual({ ...before, activePanelId: null, viewType: null });
  await terminal.click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect.poll(context).toEqual(before);
  await settingsTab(page).click();
  await expect
    .poll(context)
    .toEqual({ ...before, activePanelId: null, viewType: null });
  await page
    .getByRole("button", { name: "Close Settings", exact: true })
    .click();
  await expect.poll(context).toEqual(before);
  expect(await calls(page, "start_terminal")).toHaveLength(1);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
});
