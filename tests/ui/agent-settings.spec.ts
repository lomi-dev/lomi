import { expect, test } from "@playwright/test";
import { newProject, newSession, openFileTab } from "../../src/model";
import { mockDesktop } from "./desktop";

test("settings reads report retained providers, revision pages and explicit recovery without writes", async ({
  page,
}) => {
  const project = newProject("/project", "default");
  let session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  session = openFileTab(
    session,
    project.workspaces[0].id,
    "/project",
    "README.md",
  );
  session.projects[0].workspaces[0].tabs =
    session.projects[0].workspaces[0].tabs.filter((t) => t.type === "file");
  await mockDesktop(page, false, session);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__settingsRead = {
      projection: null,
      replies: [],
      editor: { version: 1, tabSize: 4, insertSpaces: true },
      corrupt: false,
      loads: 0,
      writes: 0,
      prepares: [],
      acks: [],
    };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      const state = desktop.__settingsRead;
      if (command === "agent_control_ui_register") return "settings-epoch";
      if (command === "agent_control_ui_publish") {
        state.projection = args.projection;
        return;
      }
      if (command === "agent_control_settings_read_reply") {
        state.replies.push(args.reply);
        return;
      }
      if (command === "agent_control_ui_claim") return;
      if (command === "agent_control_settings_prepare") {
        state.prepares.push(args);
        return;
      }
      if (command === "agent_control_settings_source")
        return {
          data: null,
          sourceRevision: null,
          definitionsRevision: "b".repeat(64),
          contributions: [],
        };
      if (command === "agent_control_ui_ack") {
        state.acks.push(args.ack);
        return;
      }
      if (
        [
          "load_editor_preferences",
          "load_terminal_preferences",
          "load_keybindings",
          "load_theme_preferences",
        ].includes(command)
      )
        state.loads++;
      if (command === "load_editor_preferences") {
        if (state.corrupt)
          throw Error("PRIVATE_FIXTURE /private/secret-config.json malformed");
        return state.editor;
      }
      if (
        [
          "save_editor_preferences",
          "save_terminal_preferences",
          "save_keybindings",
          "save_theme_preferences",
        ].includes(command)
      )
        state.writes++;
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await expect(page.locator(".cm-content")).toBeVisible();
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText("Settings read keeps this draft 🙂");
  await expect
    .poll(() =>
      page.evaluate(() => Boolean((window as any).__settingsRead.projection)),
    )
    .toBe(true);
  let sequence = 0;
  const read = async (section: string, extra: Record<string, unknown> = {}) => {
    const requestId = `read-${sequence++}`;
    await page.evaluate(
      async ({ section, extra, requestId }) => {
        const desktop = window as any;
        const workspace = desktop.__settingsRead.projection.workspaces[0];
        await desktop.__nativeTest.emitEvent("agent-control-settings-read", {
          requestId,
          uiEpoch: "settings-epoch",
          projectId: workspace.projectId,
          input: {
            workspaceId: workspace.id,
            section,
            offset: 0,
            limit: 100,
            expectedRevision: null,
            ...extra,
          },
        });
      },
      { section, extra, requestId },
    );
    await expect
      .poll(() =>
        page.evaluate(
          (id) =>
            (window as any).__settingsRead.replies.some(
              (r: any) => r.requestId === id,
            ),
          requestId,
        ),
      )
      .toBe(true);
    return page.evaluate(
      (id) =>
        (window as any).__settingsRead.replies.find(
          (r: any) => r.requestId === id,
        ),
      requestId,
    );
  };
  const baseline = await page.evaluate(
    () => (window as any).__settingsRead.loads,
  );
  const editor = await read("editor");
  expect(editor.snapshot.values).toEqual({
    section: "editor",
    tabSize: 4,
    insertSpaces: true,
  });
  expect(editor.snapshot.readiness).toBe("ready");
  const update = async (
    operationId: string,
    expectedSettingsRevision: string,
    patch: Record<string, unknown> = { type: "editor_tab_size", value: 8 },
  ) => {
    await page.evaluate(
      async ({ operationId, expectedSettingsRevision, patch }) => {
        const desktop = window as any;
        const p = desktop.__settingsRead.projection;
        const workspace = p.workspaces[0];
        await desktop.__nativeTest.emitEvent("agent-control-command", {
          operationId,
          nonce: `nonce-${operationId}`,
          uiEpoch: "settings-epoch",
          domainRevision: p.revision,
          projectId: workspace.projectId,
          action: {
            type: "update_settings",
            workspaceId: workspace.id,
            notAfterMillis: String(Date.now() + 120_000),
            input: {
              expectedSettingsRevision,
              patch,
            },
          },
        });
      },
      { operationId, expectedSettingsRevision, patch },
    );
  };
  await update("valid-update", editor.snapshot.revision);
  await expect
    .poll(() => page.evaluate(() => (window as any).__settingsRead.prepares))
    .toEqual([
      {
        operationId: "valid-update",
        nonce: "nonce-valid-update",
        revision: editor.snapshot.revision,
        current: { tabSize: 4, insertSpaces: true },
      },
    ]);
  expect(
    await page.evaluate(() => (window as any).__settingsRead.acks),
  ).toEqual([]);
  // A pending approval releases the main queue. A stale subsequent request is
  // rejected before native staging and cannot overwrite the provider state.
  await update("stale-update", "b".repeat(64));
  await expect
    .poll(() => page.evaluate(() => (window as any).__settingsRead.acks))
    .toEqual([
      {
        operationId: "stale-update",
        nonce: "nonce-stale-update",
        uiEpoch: "settings-epoch",
        result: { kind: "failure", code: "REVISION_CONFLICT" },
      },
    ]);
  const terminal = await read("terminal");
  expect(terminal.snapshot.values).toMatchObject({
    section: "terminal",
    appearanceOverrides: {},
    behavior: { scrollback: 10000 },
  });
  await update("terminal-update", terminal.snapshot.revision, {
    type: "terminal_field",
    field: "appearance.fontSize",
    value: 18,
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__settingsRead.prepares.at(-1)),
    )
    .toEqual({
      operationId: "terminal-update",
      nonce: "nonce-terminal-update",
      revision: terminal.snapshot.revision,
      current: {
        appearance: terminal.snapshot.values.appearanceOverrides,
        behavior: terminal.snapshot.values.behavior,
        windowsShell: terminal.snapshot.values.windowsShell,
        agentNotifications: terminal.snapshot.values.agentNotifications,
        alwaysShowTitles: terminal.snapshot.values.alwaysShowTitles,
      },
    });
  const first = await read("keybinds", { limit: 2 });
  expect(first.snapshot.values.items).toHaveLength(2);
  expect(first.snapshot.values.nextOffset).toBe(2);
  const second = await read("keybinds", {
    offset: 2,
    limit: 2,
    expectedRevision: first.snapshot.revision,
  });
  expect(second.snapshot.revision).toBe(first.snapshot.revision);
  expect(second.snapshot.values.items[0].action).not.toBe(
    first.snapshot.values.items[0].action,
  );
  await update("keybinding-update", first.snapshot.revision, {
    type: "keybinding_set",
    action: "saveFile",
    shortcut: "Ctrl+Alt+F20",
  });
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__settingsRead.prepares.at(-1)?.current,
      ),
    )
    .toMatchObject({
      focusFollowsPointer: false,
      sourceRevision: null,
      definitionsRevision: "b".repeat(64),
      action: {
        id: "saveFile",
        label: "Save file",
        shortcut: "Ctrl+KeyS",
        defaultShortcut: "Ctrl+KeyS",
      },
    });
  await update("keybinding-collision", first.snapshot.revision, {
    type: "keybinding_set",
    action: "chatNew",
    shortcut: "Ctrl+KeyS",
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).__settingsRead.acks.at(-1)))
    .toMatchObject({
      operationId: "keybinding-collision",
      result: { kind: "failure", code: "REVISION_CONFLICT" },
    });
  const theme = await read("themes");
  expect(theme.snapshot.values).toMatchObject({
    section: "themes",
    active: null,
    safeMode: false,
  });
  expect(JSON.stringify(theme)).not.toContain("directory");
  await update("theme-update", theme.snapshot.revision, {
    type: "theme_builtin",
    value: "deepmono",
  });
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__settingsRead.prepares.at(-1)?.current,
      ),
    )
    .toEqual({
      active: null,
      appearance: "system",
      fileIcons: null,
      productIcons: null,
    });
  expect(await page.evaluate(() => (window as any).__settingsRead.loads)).toBe(
    baseline,
  );
  await page.evaluate(async () => {
    const desktop = window as any;
    desktop.__settingsRead.editor = {
      version: 1,
      tabSize: 8,
      insertSpaces: false,
    };
    await desktop.__nativeTest.emitEvent("editor-preferences-changed", {});
  });
  await expect(
    page.getByRole("button", {
      name: "Change indentation settings",
      exact: true,
    }),
  ).toHaveText("Tabs: 8");
  expect(
    (await read("editor", { expectedRevision: editor.snapshot.revision }))
      .error,
  ).toBe("REVISION_CONFLICT");
  expect((await read("editor")).snapshot.values).toMatchObject({
    tabSize: 8,
    insertSpaces: false,
  });
  await page.evaluate(async () => {
    const desktop = window as any;
    desktop.__settingsRead.corrupt = true;
    await desktop.__nativeTest.emitEvent("editor-preferences-changed", {});
  });
  await expect
    .poll(async () => (await read("editor")).snapshot.readiness)
    .toBe("recovery_required");
  const recovered = await read("editor");
  expect(recovered.snapshot.values).toMatchObject({
    tabSize: 8,
    insertSpaces: false,
  });
  expect(JSON.stringify(recovered)).not.toMatch(
    /PRIVATE_FIXTURE|secret-config|private/,
  );
  await update("recovery-update", recovered.snapshot.revision);
  await expect
    .poll(() => page.evaluate(() => (window as any).__settingsRead.acks.at(-1)))
    .toEqual({
      operationId: "recovery-update",
      nonce: "nonce-recovery-update",
      uiEpoch: "settings-epoch",
      result: { kind: "failure", code: "UNSUPPORTED_CAPABILITY" },
    });
  expect(
    await page.evaluate(() => (window as any).__settingsRead.prepares.length),
  ).toBe(4);
  expect(await page.evaluate(() => (window as any).__settingsRead.writes)).toBe(
    0,
  );
  await expect(page.locator(".cm-content")).toHaveText(
    "Settings read keeps this draft 🙂",
  );
});
