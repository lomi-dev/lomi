import { expectRecordedNotification } from "./application-notification-helpers";
import { expect, test } from "@playwright/test";
import { newFileTab, newProject, newSession } from "../../src/model";
import { mockDesktop } from "./desktop";

for (const colorScheme of ["dark", "light"] as const) {
  test(`welcome opens a repository and fits the minimum window in ${colorScheme} mode`, async ({
    page,
    browserName,
  }, testInfo) => {
    await page.emulateMedia({ colorScheme });
    await mockDesktop(page, true, null);
    await page.goto("/");
    const welcome = page.getByRole("main", { name: "No project open" });
    const openFolder = welcome.getByRole("button", {
      name: "Open folder or repository",
      exact: true,
    });
    await expect(openFolder).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath("welcome.png") });
    await page.setViewportSize({ width: 800, height: 420 });
    await expect(openFolder).toBeInViewport({ ratio: 1 });
    await expect(
      welcome.getByRole("button", { name: "Create a new file", exact: true }),
    ).toBeInViewport({ ratio: 1 });
    expect(
      await welcome.evaluate(
        (element) =>
          element.scrollWidth <= element.clientWidth &&
          element.scrollHeight <= element.clientHeight,
      ),
    ).toBe(true);
    await page.screenshot({ path: testInfo.outputPath("welcome-minimum.png") });
    await openFolder.focus();
    // macOS WebKit uses Option+Tab when full keyboard access is disabled.
    const tab =
      browserName === "webkit" && process.platform === "darwin"
        ? "Alt+Tab"
        : "Tab";
    await page.keyboard.press(tab);
    await expect(
      welcome.getByRole("button", { name: "Create a new file", exact: true }),
    ).toBeFocused();
    await page.keyboard.press(`Shift+${tab}`);
    await page.keyboard.press("Enter");
    await expect(welcome).toHaveCount(0);
    await expect(page.locator(".project-switcher")).toHaveText("chosen folder");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    await expect(
      page.getByRole("button", { name: /^Toggle source control/ }),
    ).toBeVisible();
    await expect
      .poll(() => page.evaluate(() => localStorage.getItem("test-session")))
      .toContain('"path":"/chosen folder"');
    await page.reload();
    await expect(page.locator(".xterm-screen")).toBeVisible();
    await expect(welcome).toHaveCount(0);
  });
}

test("welcome stays usable while resizing an empty workspace sidebar", async ({
  page,
}, testInfo) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 800, height: 420 });
  await mockDesktop(page, false, null);
  await page.goto("/");
  await page
    .getByRole("button", { name: "Toggle workspaces", exact: true })
    .click();
  const divider = page.getByRole("separator", {
    name: "Resize sidebar",
    exact: true,
  });
  const bounds = (await divider.boundingBox())!;
  const maximum = await divider.getAttribute("aria-valuemax");
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + 30);
  await page.mouse.down();
  await page.mouse.move(750, bounds.y + 30);
  await page.mouse.up();
  await expect(divider).toHaveAttribute("aria-valuenow", maximum!);
  const welcome = page.getByRole("main", { name: "No project open" });
  const createFile = welcome.getByRole("button", {
    name: "Create a new file",
    exact: true,
  });
  await createFile.scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath("welcome-sidebar.png") });
  await expect(createFile).toBeInViewport({ ratio: 0.99 });
  expect(
    await welcome.evaluate(
      (element) => element.scrollWidth <= element.clientWidth,
    ),
  ).toBe(true);
  await createFile.click();
  await expect(page.locator(".cm-content")).toBeVisible();
  expect(errors).toEqual([]);
});

test("welcome creates a draft without starting a shell and preserves edits through save and restoration", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.goto("/");
  await page
    .getByRole("button", { name: "Create a new file", exact: true })
    .click();
  const editor = page.locator(".cm-content");
  await expect(editor).toBeVisible();
  await expect(editor).toBeEmpty();
  await expect(page.getByRole("tab")).toHaveCount(1);
  await expect(page.getByRole("tab", { selected: true })).toHaveText(
    "Untitled-1",
  );
  const calls = await page.evaluate(() => (window as any).__nativeTest.calls);
  expect(
    calls.find((call: any) => call.command === "plugin:dialog|open").args
      .options,
  ).toMatchObject({
    directory: true,
    multiple: false,
    title: "Choose a folder for your new file",
  });
  expect(
    calls.some((call: any) =>
      [
        "start_terminal",
        "read_editor_file",
        "save_editor_file",
        "save_new_editor_file",
        "file_operation",
      ].includes(call.command),
    ),
  ).toBe(false);
  await page.keyboard.insertText("My first file");
  await page.keyboard.press("Control+w");
  const dialog = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(editor).toHaveText("My first file");
  await page.evaluate(() => {
    (window as any).__nativeTest.newFilePath = "/chosen folder/hello.txt";
  });
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("tab", { selected: true })).toHaveText(
    "hello.txt",
  );
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("test-session")))
    .toContain('"relative":"hello.txt"');
  await page.reload();
  await expect(editor).toHaveText("My first file");
  await expect(page.getByRole("main", { name: "No project open" })).toHaveCount(
    0,
  );
});

for (const action of ["Open folder or repository", "Create a new file"]) {
  test(`${action} keeps the welcome screen usable after cancellation and an invalid folder`, async ({
    page,
  }) => {
    await mockDesktop(page, false, null);
    await page.goto("/");
    await page.evaluate(() => {
      (window as any).__nativeTest.folder = null;
    });
    const button = page.getByRole("button", { name: action, exact: true });
    await button.click();
    await expect(button).toHaveAttribute("aria-disabled", "false");
    await expect(button).toBeFocused();
    await expect
      .poll(() =>
        page.evaluate(() =>
          JSON.parse(localStorage.getItem("test-session") ?? "null"),
        ),
      )
      .toEqual(newSession());
    await page.evaluate(() => {
      const native = (window as any).__nativeTest;
      native.folder = "/missing";
      native.directoryError = "Cannot open directory: folder not found";
    });
    await button.press("Enter");
    await expectRecordedNotification(page, "folder not found");
    await expect(button).toHaveAttribute("aria-disabled", "false");
    await expect(button).toBeFocused();
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls.some((call: any) =>
          [
            "start_terminal",
            "list_directory",
            "git_status",
            "file_operation",
          ].includes(call.command),
        ),
      ),
    ).toBe(false);
    await page.evaluate(() => {
      const native = (window as any).__nativeTest;
      native.folder = "/chosen folder";
      native.directoryError = "";
    });
    await button.press("Enter");
    await expect(
      page.getByRole("main", { name: "No project open" }),
    ).toHaveCount(0);
    await expect(page.locator(".project-switcher")).toHaveText("chosen folder");
  });
}

test("welcome prevents overlapping folder pickers and recovers from dialog errors", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.goto("/");
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    const bridge = (window as any).__TAURI_INTERNALS__;
    const invoke = bridge.invoke;
    bridge.invoke = async (command: string, args: any) => {
      if (command === "plugin:dialog|open") {
        native.pickerCount = (native.pickerCount ?? 0) + 1;
        await new Promise((_, reject) => {
          native.rejectPicker = reject;
        });
      }
      return invoke(command, args);
    };
  });
  await page
    .getByRole("button", { name: "Create a new file", exact: true })
    .click();
  const openFolder = page.getByRole("button", {
    name: "Open folder or repository",
    exact: true,
  });
  await expect(openFolder).toHaveAttribute("aria-disabled", "true");
  await openFolder.focus();
  await page.keyboard.press("Enter");
  expect(
    await page.evaluate(() => (window as any).__nativeTest.pickerCount),
  ).toBe(1);
  await page.evaluate(() =>
    (window as any).__nativeTest.rejectPicker(
      "Could not open the folder picker",
    ),
  );
  await expectRecordedNotification(page, "Could not open the folder picker");
  await expect(openFolder).toHaveAttribute("aria-disabled", "false");
  await expect(openFolder).toBeFocused();
});

test("creating a file from welcome reuses an existing folder and preserves its tabs", async ({
  page,
}) => {
  const session = newSession();
  const project = newProject("/chosen folder", "local:bash");
  const draft = newFileTab(session);
  project.workspaces[0].tabs.push(draft);
  await mockDesktop(page, false, { ...session, projects: [project] });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Create a new file", exact: true })
    .click();
  await expect(page.getByRole("tab")).toHaveCount(3);
  await expect(page.getByRole("tab", { selected: true })).toHaveText(
    "Untitled-2",
  );
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")?.projects,
      ),
    )
    .toMatchObject([
      {
        id: project.id,
        workspaces: [
          {
            id: project.activeWorkspaceId,
            tabs: [
              project.workspaces[0].tabs[0],
              draft,
              { type: "file", title: "Untitled-2" },
            ],
          },
        ],
      },
    ]);
});
