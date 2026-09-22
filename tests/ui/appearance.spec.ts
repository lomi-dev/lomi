import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

async function settings(page: Page) {
  await page.goto("/?window=settings");
  await page.getByRole("button", { name: "Themes", exact: true }).click();
}

async function calls(page: Page, command: string) {
  return page.evaluate(
    (command) =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === command,
      ),
    command,
  );
}

async function terminal(page: Page, pane: string) {
  return page.evaluate(async (pane) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(pane)!;
    return {
      id: runtime.sessionId,
      foreground: runtime.terminal.options.theme?.foreground,
    };
  }, pane);
}

for (const mode of ["light", "dark"] as const) {
  test(`the first interface follows a ${mode} system without saved settings`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: mode });
    await mockDesktop(page);
    await page.addInitScript(() => {
      (window as any).__firstAppearance = [];
      new MutationObserver(() => {
        if (document.querySelector(".app-shell"))
          (window as any).__firstAppearance.push(
            document.documentElement.dataset.appearance,
          );
      }).observe(document, { subtree: true, childList: true });
    });
    await settings(page);
    await expect(
      page.getByRole("radio", { name: "System", exact: true }),
    ).toBeChecked();
    await expect(page.locator("html")).toHaveAttribute("data-appearance", mode);
    await expect(page.locator(".app-shell")).toHaveCSS(
      "background-color",
      mode === "light" ? "rgb(247, 248, 243)" : "rgb(16, 17, 20)",
    );
    await expect(page.locator("html")).toHaveCSS("color-scheme", mode);
    const painted = await page.evaluate(
      () => (window as any).__firstAppearance,
    );
    expect(painted.length).toBeGreaterThan(0);
    expect(new Set(painted)).toEqual(new Set([mode]));
    expect(await calls(page, "save_theme_preferences")).toHaveLength(0);
    expect(
      (await calls(page, "sync_theme_window")).at(-1).args.appearance,
    ).toBe("system");
    await page.screenshot({ path: `test-results/appearance-${mode}.png` });
  });
}

test("system changes update both windows, hidden terminals, and an edited document", async ({
  page,
  context,
}) => {
  await page.emulateMedia({ colorScheme: "light" });
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect.poll(() => calls(page, "start_terminal")).toHaveLength(1);
  const first = (await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  const before = await terminal(page, first);
  expect(before.foreground).toBe("#0b0d0cff");
  await page.evaluate(
    (id) =>
      (window as any).__nativeTest.emit(id, "\r\nappearance preserved\r\n"),
    before.id,
  );
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+Shift+t");
  const second = (await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  expect(second).not.toBe(first);
  const secondBefore = await terminal(page, second);
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  const content = page.locator(".cm-content");
  await expect(content).toContainText("A text file preview.");
  await content.click();
  await page.keyboard.press("Control+End");
  await page.keyboard.type("Unsaved appearance change");
  const draft = await content.innerText();

  const preferences = await context.newPage();
  await preferences.emulateMedia({ colorScheme: "light" });
  await mockDesktop(preferences);
  await settings(preferences);
  for (const mode of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: mode });
    await preferences.emulateMedia({ colorScheme: mode });
    for (const view of [page, preferences])
      await expect(view.locator("html")).toHaveAttribute(
        "data-appearance",
        mode,
      );
    for (const pane of [first, second])
      await expect
        .poll(async () => (await terminal(page, pane)).foreground)
        .toBe(mode === "dark" ? "#f3f4f6ff" : "#0b0d0cff");
    await expect
      .poll(() =>
        page.locator(".cm-editor").evaluate(async (node) => {
          const moduleUrl = performance
            .getEntriesByType("resource")
            .map((entry) => entry.name)
            .find((url) => url.includes("/@codemirror_view.js"))!;
          const { EditorView } = await import(moduleUrl);
          return EditorView.findFromDOM(node as HTMLElement)!.state.facet(
            EditorView.darkTheme,
          );
        }),
      )
      .toBe(mode === "dark");
    expect(await content.innerText()).toBe(draft);
  }
  expect((await terminal(page, first)).id).toBe(before.id);
  expect((await terminal(page, second)).id).toBe(secondBefore.id);
  expect(await buffer(page, first)).toContain("appearance preserved");
  expect(await calls(page, "start_terminal")).toHaveLength(2);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  expect(await calls(page, "save_theme_preferences")).toHaveLength(0);
  expect(await calls(preferences, "save_theme_preferences")).toHaveLength(0);
  await page.screenshot({ path: "test-results/appearance-light-editor.png" });
});

test("manual modes persist across windows and relaunches, then System resumes following the OS", async ({
  page,
  context,
}) => {
  await page.emulateMedia({ colorScheme: "light" });
  await mockDesktop(page);
  await settings(page);
  const other = await context.newPage();
  await other.emulateMedia({ colorScheme: "light" });
  await mockDesktop(other);
  await other.goto("/");
  await expect(other.locator(".xterm-screen")).toBeVisible();
  for (const [label, appearance] of [
    ["Dark", "dark"],
    ["Light", "light"],
  ] as const) {
    await page.getByRole("radio", { name: label, exact: true }).check();
    for (const view of [page, other])
      await expect(view.locator("html")).toHaveAttribute(
        "data-appearance",
        appearance,
      );
    await page.emulateMedia({
      colorScheme: appearance === "dark" ? "light" : "dark",
    });
    await page.reload();
    await page.getByRole("button", { name: "Themes", exact: true }).click();
    await expect(
      page.getByRole("radio", { name: label, exact: true }),
    ).toBeChecked();
    await expect(page.locator("html")).toHaveAttribute(
      "data-appearance",
      appearance,
    );
    expect(
      (await calls(page, "sync_theme_window")).at(-1).args.appearance,
    ).toBe(appearance);
  }
  await page.getByRole("radio", { name: "System", exact: true }).check();
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "dark");
  await expect(other.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  expect((await calls(page, "sync_theme_window")).at(-1).args.appearance).toBe(
    "system",
  );
  await page.setViewportSize({ width: 560, height: 420 });
  expect(
    await page
      .locator(".themes-page")
      .evaluate((node) => node.scrollWidth <= node.clientWidth),
  ).toBe(true);
  await page.screenshot({ path: "test-results/appearance-minimum.png" });
});

test("adaptive custom themes follow the mode and fixed themes retain their palette", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "light" });
  await mockDesktop(page);
  await page.addInitScript(() => {
    if (!localStorage.getItem("test-theme-manifests"))
      localStorage.setItem(
        "test-theme-manifests",
        JSON.stringify({
          adaptive: {
            version: 1,
            name: "Adaptive",
            tokens: { "--radius-control": "11px" },
          },
          fixed: {
            version: 1,
            name: "Fixed",
            appearance: "dark",
            tokens: { "--color-background": "#222222" },
          },
          paper: {
            version: 1,
            name: "Paper",
            appearance: "light",
            tokens: { "--color-background": "#fafafa" },
          },
        }),
      );
    if (!localStorage.getItem("test-theme-settings"))
      localStorage.setItem(
        "test-theme-settings",
        '{"version":1,"active":"adaptive","customCss":false}',
      );
  });
  await settings(page);
  await expect(
    page.getByRole("radio", { name: "System", exact: true }),
  ).toBeChecked();
  await expect(page.locator(".button").first()).toHaveCSS(
    "border-radius",
    "11px",
  );
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "dark");
  await expect(page.locator(".button").first()).toHaveCSS(
    "border-radius",
    "11px",
  );
  await page.getByRole("button", { name: "Use Fixed theme" }).click();
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "dark");
  await expect(page.locator(".app-shell")).toHaveCSS(
    "background-color",
    "rgb(34, 34, 34)",
  );
  await expect(
    page.getByRole("radio", { name: "Dark", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Use Paper theme" }).click();
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  await expect(page.locator(".app-shell")).toHaveCSS(
    "background-color",
    "rgb(250, 250, 250)",
  );
  await page.getByRole("button", { name: "Use Lomi theme" }).click();
  await expect(
    page.getByRole("radio", { name: "System", exact: true }),
  ).toBeChecked();
  await expect(
    page.getByRole("radio", { name: "System", exact: true }),
  ).toBeEnabled();
});

test("a failed color mode save preserves the current appearance", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "light" });
  await mockDesktop(page);
  await settings(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.failThemeSave = true;
  });
  await page.getByRole("radio", { name: "Dark", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Disk is full");
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  await expect(
    page.getByRole("radio", { name: "System", exact: true }),
  ).toBeChecked();
  expect(
    await page.evaluate(() => localStorage.getItem("test-theme-settings")),
  ).toBeNull();
});
