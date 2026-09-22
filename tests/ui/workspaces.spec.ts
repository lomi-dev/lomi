import { expect, test } from "@playwright/test";
import { active, addWorkspace, newSession, openFileTab } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";

test("workspace disclosures list and select tabs without activating hidden terminals", async ({
  page,
}, testInfo) => {
  let session = addWorkspace(
    newSession(),
    "/work/simplevoice",
    "local:bash",
    "Voice",
  );
  const voice = active(session)!.workspace;
  const commit = {
    id: "a".repeat(40),
    shortId: "aaaaaaa",
    subject: "feat(ui): add workspace navigation",
    authorName: "Alex",
    authoredAt: "2026-09-06T12:30:00+02:00",
  };
  voice.tabs.push(
    {
      type: "file",
      id: "readme",
      title: "README.md",
      root: "/work/simplevoice",
      relative: "README.md",
    },
    {
      type: "commit",
      id: "commit",
      title: commit.subject,
      root: "/work/simplevoice",
      commit: commit.id,
    },
  );
  session = addWorkspace(session, "/work/lomi", "local:bash", "Bench");
  session.sidebar = "workspaces";
  await mockDesktop(page, true, session, {
    commits: [commit],
    details: {
      [commit.id]: {
        commit,
        authorEmail: "alex@example.test",
        committerName: "Alex",
        committerEmail: "alex@example.test",
        committedAt: commit.authoredAt,
        parents: [],
        message: commit.subject,
        files: [],
      },
    },
    diffs: {},
  });
  await page.addInitScript(() =>
    localStorage.setItem(
      "test-keybindings",
      JSON.stringify({ version: 1, bindings: { newTab: "Ctrl+Shift+KeyT" } }),
    ),
  );
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const list = page.getByRole("navigation", { name: "Workspace list" });
  const voiceTabs = list.getByRole("list", {
    name: "Tabs in Voice",
    exact: true,
  });
  const benchTabs = list.getByRole("list", {
    name: "Tabs in Bench",
    exact: true,
  });
  const toggleVoice = list.getByRole("button", {
    name: /^(Expand|Collapse) tabs in Voice$/,
  });
  await expect(toggleVoice).toHaveAttribute("aria-expanded", "false");
  await expect(voiceTabs).toBeHidden();
  await toggleVoice.focus();
  await page.keyboard.press("Enter");
  await list
    .getByRole("button", { name: "Expand tabs in Bench", exact: true })
    .click();
  await expect(toggleVoice).toHaveAttribute("aria-expanded", "true");
  await expect(voiceTabs.getByRole("button")).toHaveText([
    "Terminal",
    "README.md",
    commit.subject,
  ]);
  await expect(benchTabs).toBeVisible();
  await expect(list.getByRole("button", { name: /^Bench / })).toHaveAttribute(
    "aria-current",
    "true",
  );

  const starts = () =>
    page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    );
  expect(await starts()).toBe(1);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some((call: any) =>
        ["read_editor_file", "git_commit_details"].includes(call.command),
      ),
    ),
  ).toBe(false);
  const readme = voiceTabs.getByRole("button", {
    name: "README.md",
    exact: true,
  });
  await readme.click();
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect(page.getByRole("tab", { selected: true })).toHaveText(
    "README.md",
  );
  await expect(readme).toHaveAttribute("aria-current", "true");
  await expect(page.locator(".project-switcher")).toHaveText("simplevoice");
  expect(await starts()).toBe(1);
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText("Keep this workspace draft");
  await benchTabs
    .getByRole("button", { name: "Terminal", exact: true })
    .click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await readme.click();
  await expect(page.locator(".cm-content")).toHaveText(
    "Keep this workspace draft",
  );
  await voiceTabs
    .getByRole("button", { name: commit.subject, exact: true })
    .click();
  await expect(page.getByRole("article")).toContainText(commit.subject);
  expect(await starts()).toBe(1);
  await voiceTabs
    .getByRole("button", { name: "Terminal", exact: true })
    .click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect.poll(starts).toBe(2);
  await page.keyboard.press("Control+Shift+t");
  await expect(voiceTabs.getByRole("button")).toHaveCount(4);
  await expect(
    voiceTabs.getByRole("button", { name: "Terminal 4", exact: true }),
  ).toHaveAttribute("aria-current", "true");
  await page
    .getByRole("button", { name: "Close Terminal 4", exact: true })
    .click();
  await expect(voiceTabs.getByRole("button")).toHaveCount(3);
  await readme.click();
  await toggleVoice.focus();
  await page.keyboard.press("Space");
  await expect(voiceTabs).toBeHidden();
  await expect(benchTabs).toBeVisible();
  await expect(page.locator(".cm-content")).toHaveText(
    "Keep this workspace draft",
  );
  await page.keyboard.press("Space");
  await expect(voiceTabs).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("workspace-tabs-dark.png"),
  });
  await page.setViewportSize({ width: 800, height: 420 });
  await page.emulateMedia({ colorScheme: "light" });
  await expect(toggleVoice).toBeInViewport();
  await expect(
    voiceTabs.getByRole("button", { name: commit.subject, exact: true }),
  ).toBeInViewport();
  expect(
    await list.evaluate(
      (element) => element.scrollWidth <= element.clientWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("workspace-tabs-light-minimum.png"),
  });
});

test("the global list switches folders and independent workspaces without restarting terminals", async ({
  page,
}, testInfo) => {
  let session = newSession();
  for (const [path, name] of [
    ["/work/simplevoice", "Voice 1"],
    ["/work/simplevoice", "Voice 2"],
    ["/work/lomi", "Bench"],
  ]) {
    session = addWorkspace(session, path, "local:bash", name);
  }
  await mockDesktop(page, true, session);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Switch workspace", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Toggle workspaces", exact: true })
    .click();
  const panel = page.getByRole("complementary", {
    name: "Workspaces",
    exact: true,
  });
  const list = panel.getByRole("navigation", { name: "Workspace list" });
  await expect(list.locator(".workspace-list-item")).toHaveCount(3);
  const paneIds: string[] = [];
  for (const name of ["Voice 1", "Voice 2", "Bench"]) {
    const row = list.getByRole("button", { name: new RegExp(`^${name} `) });
    await row.click();
    await expect(row).toHaveAttribute("aria-current", "true");
    await expect(page.locator(".xterm-screen")).toBeVisible();

    await expect(page.locator(".project-switcher")).toHaveText(
      name === "Bench" ? "lomi" : "simplevoice",
    );
    paneIds.push(
      (await page.locator("[data-pane-id]").getAttribute("data-pane-id"))!,
    );
  }
  expect(new Set(paneIds).size).toBe(3);
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    const started = native.calls.find(
      (call: any) =>
        call.command === "start_terminal" &&
        call.args.request.cwd === "/work/simplevoice",
    );
    native.emit(started.args.request.id, "\r\nVOICE AGENT STILL RUNNING\r\n");
  });
  await expect
    .poll(() => buffer(page, paneIds[0]))
    .toContain("VOICE AGENT STILL RUNNING");
  await list.getByRole("button", { name: /^Voice 1 / }).click();
  await expect(page.locator("[data-pane-id]")).toHaveAttribute(
    "data-pane-id",
    paneIds[0],
  );
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(3);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "close_terminal",
        ).length,
    ),
  ).toBe(0);

  await page
    .getByRole("button", { name: "Toggle workspaces", exact: true })
    .click({ button: "right" });
  await page.getByRole("menuitemradio", { name: "Panel on the right" }).click();
  await page.getByRole("button", { name: /^Toggle file explorer/ }).click();
  await expect(panel).toHaveAttribute("data-side", "right");
  await expect(
    page.getByRole("complementary", { name: "Explorer", exact: true }),
  ).toBeVisible();
  await page.getByRole("separator", { name: "Resize right sidebar" }).focus();
  await page.keyboard.press("ArrowLeft");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")
            ?.rightSidebarWidth,
      ),
    )
    .toBeGreaterThan(250);
  await page.screenshot({ path: testInfo.outputPath("workspaces-dark.png") });
  await page.setViewportSize({ width: 800, height: 420 });
  await page.emulateMedia({ colorScheme: "light" });
  await expect(list.getByRole("button").first()).toBeInViewport();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("workspaces-light-minimum.png"),
  });
  await page.reload();
  await expect(panel).toHaveAttribute("data-side", "right");
  await expect(list.getByRole("button", { name: /^Voice 1 / })).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(page.locator(".xterm-screen")).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
});

test("sidebar actions manage inactive workspaces and can remove the last workspace without recreating it", async ({
  page,
}, testInfo) => {
  let session = addWorkspace(newSession(), "/voice", "local:bash", "Voice");
  session = addWorkspace(session, "/bench", "local:bash", "Bench");
  session.sidebar = "workspaces";
  await mockDesktop(page, false, session);
  await page.setViewportSize({ width: 800, height: 420 });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const list = page.getByRole("navigation", { name: "Workspace list" });
  await list.getByRole("button", { name: /^Voice / }).click();
  await expect(list.getByRole("button", { name: /^Voice / })).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await list.getByRole("button", { name: /^Bench / }).click();
  await expect(list.getByRole("button", { name: /^Bench / })).toHaveAttribute(
    "aria-current",
    "true",
  );
  const activeTabId = await page
    .getByRole("tab", { selected: true })
    .getAttribute("id");
  const voice = list.getByRole("button", { name: /^Voice / });
  await voice.focus();
  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Workspace actions" });
  await expect(menu).toBeInViewport();
  await expect(
    menu.getByRole("menuitem", { name: "Rename workspace", exact: true }),
  ).toBeFocused();
  await page.screenshot({
    path: testInfo.outputPath("workspace-actions-minimum.png"),
  });
  await page.keyboard.press("Escape");
  await expect(voice).toBeFocused();
  await voice.click({ button: "right" });
  await menu
    .getByRole("menuitem", { name: "Rename workspace", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Name", exact: true })
    .fill("Voice agent");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    list.getByRole("button", { name: /^Voice agent / }),
  ).toBeVisible();
  await expect(page.getByRole("tab", { selected: true })).toHaveAttribute(
    "id",
    activeTabId!,
  );
  await expect(list.getByRole("button", { name: /^Bench / })).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")
            ?.projects[0]?.workspaces[0]?.name,
      ),
    )
    .toBe("Voice agent");
  for (const name of ["Voice agent", "Bench"]) {
    await list
      .getByRole("button", { name: new RegExp(`^${name} `) })
      .click({ button: "right" });
    await menu
      .getByRole("menuitem", { name: "Delete workspace…", exact: true })
      .click();
    await expect(
      page.getByRole("button", { name: "Continue", exact: true }),
    ).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(
      list.getByRole("button", { name: new RegExp(`^${name} `) }),
    ).toHaveCount(0);
    if (name === "Voice agent") {
      await expect(page.getByRole("tab", { selected: true })).toHaveAttribute(
        "id",
        activeTabId!,
      );
      await expect(
        list.getByRole("button", { name: /^Bench / }),
      ).toHaveAttribute("aria-current", "true");
    }
  }
  await expect(
    page.getByRole("main", { name: "No project open" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "close_terminal",
          ).length,
      ),
    )
    .toBe(2);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some(
        (call: any) => call.command === "file_operation",
      ),
    ),
  ).toBe(false);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")?.projects,
      ),
    )
    .toEqual([]);
  await page.reload();
  await expect(
    page.getByRole("main", { name: "No project open" }),
  ).toBeVisible();
  await expect(list.getByRole("button")).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some(
        (call: any) => call.command === "start_terminal",
      ),
    ),
  ).toBe(false);
  await page
    .getByRole("button", { name: "New workspace", exact: true })
    .click();
  await page.getByRole("textbox", { name: "Name", exact: true }).press("Enter");
  await expect(list.locator(".workspace-list-item")).toHaveCount(1);
  await expect(page.locator(".xterm-screen")).toBeVisible();
});

test("deleting the final workspace retains dirty files after cancellation or failed saves", async ({
  page,
}) => {
  let session = addWorkspace(newSession(), "/project", "local:bash", "Agent");
  session = openFileTab(
    session,
    active(session)!.workspace.id,
    "/project",
    "README.md",
  );
  session.sidebar = "workspaces";
  await mockDesktop(page, false, session);
  await page.goto("/");
  await expect(page.locator(".cm-content")).toBeVisible();
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.getByRole("tab", { name: "README.md", exact: true }).click();
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText("Retain this draft");
  const row = page
    .getByRole("navigation", { name: "Workspace list" })
    .getByRole("button", { name: /^Agent / });
  for (const attempt of ["cancel", "failed save", "discard"]) {
    await row.click({ button: "right" });
    await page
      .getByRole("menuitem", { name: "Delete workspace…", exact: true })
      .click();
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    const guard = page.getByRole("dialog", {
      name: "Save changes before closing?",
    });
    await expect(guard).toBeVisible();
    if (attempt === "failed save") {
      await page.evaluate(() => {
        (window as any).__nativeTest.failFileSave = true;
      });
      await guard
        .getByRole("button", { name: "Save changes", exact: true })
        .click();
      await expect(
        guard.getByRole("alert").filter({ hasText: /\S/ }),
      ).toContainText("Disk is full");
    }
    if (attempt === "discard") {
      await guard
        .getByRole("button", { name: "Discard changes", exact: true })
        .click();
    } else {
      await guard.getByRole("button", { name: "Cancel", exact: true }).click();
      await expect(page.locator(".cm-content")).toHaveText("Retain this draft");
      await expect(row).toBeVisible();
      expect(
        await page.evaluate(() =>
          (window as any).__nativeTest.calls.some(
            (call: any) => call.command === "close_terminal",
          ),
        ),
      ).toBe(false);
    }
  }
  await expect(
    page.getByRole("main", { name: "No project open" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "close_terminal",
          ).length,
      ),
    )
    .toBe(1);
});

test("adding a workspace chooses a folder every time, including the same folder, and cancellation preserves the session", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.goto("/");
  await page
    .getByRole("button", { name: "Toggle workspaces", exact: true })
    .click();
  const panel = page.getByRole("complementary", {
    name: "Workspaces",
    exact: true,
  });
  const create = panel.getByRole("button", {
    name: "New workspace",
    exact: true,
  });
  await create.click();
  await page
    .getByRole("textbox", { name: "Name", exact: true })
    .fill("Agent 1");
  await page.getByRole("textbox", { name: "Name", exact: true }).press("Enter");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await create.click();
  await page
    .getByRole("textbox", { name: "Name", exact: true })
    .fill("Agent 2");
  await page.getByRole("textbox", { name: "Name", exact: true }).press("Enter");
  await expect(
    panel.getByRole("button", { name: /^Agent 2 / }),
  ).toHaveAttribute("aria-current", "true");
  await expect(panel.locator(".workspace-list-item")).toHaveCount(2);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")
            ?.projects[0]?.workspaces.length,
      ),
    )
    .toBe(2);
  await page.evaluate(() => {
    (window as any).__nativeTest.folder = "/work/lomi";
  });
  await create.click();
  await page.getByRole("textbox", { name: "Name", exact: true }).press("Enter");
  await expect(page.locator(".project-switcher")).toHaveText("lomi");
  await expect(panel.locator(".workspace-list-item")).toHaveCount(3);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")?.projects
            .length,
      ),
    )
    .toBe(2);
  const before = await page.evaluate(() =>
    localStorage.getItem("test-session"),
  );
  await create.click();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.evaluate(() => {
    (window as any).__nativeTest.folder = null;
  });
  await create.click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("test-session"))).toBe(
    before,
  );
  await page.evaluate(() => {
    (window as any).__nativeTest.directoryError = "Folder not found";
  });
  await panel.getByRole("button", { name: /^Agent 1 / }).click();
  await expect(page.getByRole("alert").filter({ hasText: /\S/ })).toContainText(
    "Folder not found",
  );
  await expect(panel.getByRole("button", { name: /^lomi / })).toHaveAttribute(
    "aria-current",
    "true",
  );
});

test("a configured workspace shortcut works before folder selection and is captured before terminal input", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.addInitScript(() =>
    localStorage.setItem(
      "test-keybindings",
      JSON.stringify({
        version: 1,
        bindings: { toggleWorkspaces: "Ctrl+Shift+KeyB" },
      }),
    ),
  );
  await page.goto("/");
  await expect(page.locator(".statusbar")).toBeVisible();
  await page.keyboard.press("Control+Shift+b");
  const panel = page.getByRole("complementary", {
    name: "Workspaces",
    exact: true,
  });
  await expect(panel).toBeVisible();
  await panel
    .getByRole("button", { name: "New workspace", exact: true })
    .click();
  await page.getByRole("textbox", { name: "Name", exact: true }).press("Enter");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+Shift+b");
  await expect(panel).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "write_terminal",
        ).length,
    ),
  ).toBe(0);
});
