import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("Android metadata approval requires a selected device and grants only read access", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__agentTest = {
      androidReads: 0,
      approval: null,
      pendingInstalls: [],
      pendingProjectOpens: [],
      projectDecision: null,
      pendingSettingsUpdates: [],
      settingsDecision: null,
      installDecision: null,
      recoveryOpens: 0,
    };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "agent_control_state")
        return {
          supported: true,
          helperPath: null,
          broker: {
            endpoint: {
              instanceId: "fixture",
              endpoint: "fixture",
              brokerSha256: "fixture",
              ipcVersion: 1,
            },
            uiReady: true,
            terminalProfile: null,
            workspaces: [
              {
                id: "workspace",
                projectId: "project",
                name: "Fixture",
                projectName: "Project",
                projectPath: "/project",
              },
              {
                id: "additional",
                projectId: "project",
                name: "Additional workspace",
                projectName: "Project",
                projectPath: "/project",
              },
            ],
            pending: [
              {
                id: "request",
                clientLabel: "Fixture client",
                certificateSha256: "fixture",
                secondsRemaining: 120,
              },
            ],
            pendingControls: [],
            pendingInstalls: desktop.__agentTest.pendingInstalls,
            pendingProjectOpens: desktop.__agentTest.pendingProjectOpens,
            pendingSettingsUpdates: desktop.__agentTest.pendingSettingsUpdates,
            sessions: [],
          },
        };
      if (command === "android_state") {
        desktop.__agentTest.androidReads++;
        return {
          devices: {
            devices: [
              { id: "device-one", name: "Selected phone" },
              { id: "device-two", name: "Private phone" },
            ],
          },
        };
      }
      if (command === "agent_control_project_open_decide") {
        desktop.__agentTest.projectDecision = args;
        return;
      }
      if (command === "agent_control_settings_decide") {
        desktop.__agentTest.settingsDecision = args;
        desktop.__agentTest.pendingSettingsUpdates = [];
        return;
      }
      if (command === "agent_control_decide_install") {
        desktop.__agentTest.installDecision = args;
        return;
      }
      if (command === "agent_control_approve") {
        desktop.__agentTest.approval = args;
        return;
      }
      if (command === "agent_control_open_recovery")
        return ++desktop.__agentTest.recoveryOpens > 1;
      return invoke(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  const recovery = page.getByRole("button", {
    name: "Show recovery folder",
    exact: true,
  });
  await recovery.click();
  await expect(page.getByRole("status")).toHaveText(
    "No file recovery data has been created.",
  );
  await recovery.click();
  await expect(page.getByRole("status")).toHaveText("Recovery folder opened.");
  await recovery.scrollIntoViewIfNeeded();
  await page.screenshot({ path: "test-results/agent-control-recovery.png" });
  await page.getByLabel("Workspace", { exact: true }).selectOption("workspace");
  const settingsOpening = page.getByLabel("Allow opening Settings sections");
  await expect(settingsOpening).not.toBeChecked();
  await settingsOpening.check();
  await page
    .getByRole("button", { name: "Approve session", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(["workspace.read", "settings.open"]);
  await settingsOpening.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/agent-control-settings-permission.png",
  });
  const settingsReading = page.getByLabel(
    "Allow reading nonsecret application preferences",
  );
  await expect(settingsReading).not.toBeChecked();
  const settingsWriting = page.getByLabel(
    "Allow requesting application preference changes",
  );
  await expect(settingsWriting).not.toBeChecked();
  await expect(settingsWriting).toBeDisabled();
  await settingsReading.check();
  await page
    .getByRole("button", { name: "Approve session", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(["workspace.read", "settings.open", "settings.read"]);
  await settingsReading.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/agent-control-settings-read-permission.png",
  });
  await settingsWriting.check();
  await page
    .getByRole("button", { name: "Approve session", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual([
      "workspace.read",
      "settings.open",
      "settings.read",
      "settings.write",
    ]);
  await settingsReading.uncheck();
  await expect(settingsWriting).not.toBeChecked();
  await expect(settingsWriting).toBeDisabled();
  await settingsOpening.uncheck();
  for (const approve of [false, true]) {
    await page.evaluate(() => {
      (window as any).__agentTest.pendingSettingsUpdates = [
        {
          operationId: "preferences-operation",
          clientLabel: "Fixture client",
          requestKey: "change-tab-size",
          before: { tabSize: 4, insertSpaces: true },
          after: { tabSize: 8, insertSpaces: true },
          secondsRemaining: 119,
        },
      ];
    });
    const request = page.locator(
      '[data-settings-operation="preferences-operation"]',
    );
    await expect(request).toContainText("4 → 8");
    await expect(request).toContainText("Spaces → Spaces");
    await request.scrollIntoViewIfNeeded();
    await page.screenshot({
      path: "test-results/agent-control-settings-update-approval.png",
    });
    await request
      .getByRole("button", {
        name: approve ? "Apply change" : "Reject change",
        exact: true,
      })
      .click();
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__agentTest.settingsDecision),
      )
      .toEqual({ operationId: "preferences-operation", approve });
    await expect(request).toHaveCount(0);
  }

  await page.evaluate(() => {
    (window as any).__agentTest.pendingSettingsUpdates = [
      {
        operationId: "terminal-preferences-operation",
        section: "terminal",
        clientLabel: "Fixture client",
        requestKey: "inherit-font",
        before: { appearance: { fontSize: 18, colors: { red: "#123456" } } },
        after: { appearance: { colors: { red: "#123456" } } },
        secondsRemaining: 119,
      },
    ];
  });
  const terminalChange = page.locator(
    '[data-settings-operation="terminal-preferences-operation"]',
  );
  await expect(terminalChange).toContainText("Terminal preferences");
  await expect(terminalChange).toContainText("Font size");
  await expect(terminalChange).toContainText("18 → Theme default");
  await expect(terminalChange).not.toContainText("#123456");
  await terminalChange.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/agent-control-terminal-preference-approval.png",
  });
  await terminalChange
    .getByRole("button", { name: "Reject change", exact: true })
    .click();
  await expect(terminalChange).toHaveCount(0);

  for (const reset of [false, true]) {
    await page.evaluate((reset) => {
      (window as any).__agentTest.pendingSettingsUpdates = [
        {
          operationId: "keybindings-operation",
          section: "keybinds",
          patch: { type: reset ? "keybinding_reset" : "keybinding_set" },
          clientLabel: "Fixture client",
          requestKey: "shortcut-change",
          before: {
            action: {
              id: "saveFile",
              label: "Save file",
              shortcut: "Ctrl+Alt+F20",
            },
          },
          after: { action: { shortcut: reset ? "Meta+KeyS" : null } },
          secondsRemaining: 119,
        },
      ];
    }, reset);
    const shortcut = page.locator(
      '[data-settings-operation="keybindings-operation"]',
    );
    await expect(shortcut).toContainText("Keyboard shortcuts");
    await expect(shortcut).toContainText(
      reset ? "Restore default shortcut" : "Disable shortcut",
    );
    await expect(shortcut).toContainText("Save file (saveFile)");
    await expect(shortcut).toContainText(
      reset ? "Ctrl+Alt+F20 → Cmd+S" : "Ctrl+Alt+F20 → Disabled",
    );
    await shortcut.scrollIntoViewIfNeeded();
    await page.screenshot({
      path: "test-results/agent-control-keybindings-approval.png",
    });
    await shortcut
      .getByRole("button", { name: "Apply change", exact: true })
      .click();
    await expect(shortcut).toHaveCount(0);
  }

  const movement = page.getByLabel("Allow rearranging existing panels");
  await page.evaluate(() => {
    (window as any).__agentTest.pendingSettingsUpdates = [
      {
        operationId: "theme-operation",
        section: "themes",
        clientLabel: "Fixture client",
        requestKey: "deepmono",
        before: { active: null, appearance: "system" },
        after: { active: "@builtin-deepmono", appearance: "light" },
        secondsRemaining: 119,
      },
    ];
  });
  const themeChange = page.locator(
    '[data-settings-operation="theme-operation"]',
  );
  await expect(themeChange).toContainText("Lomi → DeepMono");
  await expect(themeChange).toContainText("Follow system → Light");
  await themeChange.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/agent-control-theme-approval.png",
  });
  await themeChange
    .getByRole("button", { name: "Reject change", exact: true })
    .click();
  await expect(themeChange).toHaveCount(0);
  const workspaceChanges = page.getByLabel(
    "Allow workspace creation, renaming and selection",
  );
  const projectOpening = page.getByLabel(
    "Allow requesting access to new project folders",
  );
  await expect(projectOpening).toBeDisabled();
  await expect(movement).toBeDisabled();
  await expect(movement).not.toBeChecked();
  const closing = page.getByLabel("Allow requesting workspace closure");
  const panelChanges = page.getByLabel("Allow selecting and closing panels");
  await expect(closing).toBeDisabled();
  await workspaceChanges.check();
  await projectOpening.check();
  await expect(closing).toBeDisabled();
  await panelChanges.check();
  await closing.check();
  const projectClosing = page.getByLabel("Allow requesting project closure");
  const additional = page.getByLabel("Additional workspace", { exact: true });
  await expect(projectClosing).toBeDisabled();
  await additional.check();
  await projectClosing.check();
  await projectClosing.scrollIntoViewIfNeeded();
  await page.screenshot({ path: "test-results/agent-project-permission.png" });
  await additional.uncheck();
  await expect(projectClosing).toBeDisabled();
  await expect(projectClosing).not.toBeChecked();
  await additional.check();
  await projectClosing.check();
  await panelChanges.uncheck();
  await expect(projectClosing).not.toBeChecked();
  await additional.uncheck();
  await expect(closing).toBeDisabled();
  await expect(closing).not.toBeChecked();
  await panelChanges.check();
  await expect(closing).not.toBeChecked();
  await closing.check();
  await workspaceChanges.uncheck();
  await expect(projectOpening).not.toBeChecked();
  await expect(projectOpening).toBeDisabled();
  await expect(closing).toBeDisabled();
  await expect(closing).not.toBeChecked();
  await panelChanges.uncheck();
  await workspaceChanges.check();
  await movement.check();
  await expect(movement).toBeChecked();
  await workspaceChanges.uncheck();
  await expect(movement).toBeDisabled();
  await expect(movement).not.toBeChecked();
  expect(
    await page.evaluate(() => (window as any).__agentTest.androidReads),
  ).toBe(0);
  await page.getByLabel("Allow reading selected Android device status").check();
  const inputPermission = page.getByLabel(
    "Allow touch, keys and text in this Android device",
  );
  const observationPermission = page.getByLabel(
    "Allow reading screen content in this Android device",
  );
  const capturePermission = page.getByLabel(
    "Allow screenshots of this Android device",
  );
  await expect(capturePermission).not.toBeChecked();
  await expect(capturePermission).toBeDisabled();
  await expect(observationPermission).not.toBeChecked();
  await expect(observationPermission).toBeDisabled();
  await expect(inputPermission).not.toBeChecked();
  await expect(inputPermission).toBeDisabled();
  const approve = page.getByRole("button", {
    name: "Approve session",
    exact: true,
  });
  await expect(approve).toBeDisabled();
  await page.getByLabel("Managed Android device").selectOption("device-one");
  await expect(approve).toBeEnabled();
  await approve.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/agent-control-android-permission.png",
  });
  await approve.click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__agentTest.approval))
    .toEqual({
      requestId: "request",
      workspaceIds: ["workspace"],
      scopes: ["workspace.read", "android.read"],
      browserOrigins: [],
      androidDevices: ["device-one"],
      androidPackages: [],
      chatConversations: [],
    });
  await page
    .getByLabel("Allow starting and stopping this Android device")
    .check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(["workspace.read", "android.read", "android.control"]);
  await expect(inputPermission).toBeEnabled();
  await expect(observationPermission).toBeEnabled();
  await expect(inputPermission).not.toBeChecked();
  await inputPermission.check();
  await observationPermission.check();
  await capturePermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual([
      "workspace.read",
      "android.read",
      "android.control",
      "android.interact",
      "android.observe",
      "android.capture",
    ]);
  await page
    .getByLabel("Allow starting and stopping this Android device")
    .uncheck();
  await expect(inputPermission).not.toBeChecked();
  await expect(observationPermission).not.toBeChecked();
  await expect(observationPermission).toBeDisabled();
  await expect(capturePermission).not.toBeChecked();
  await expect(capturePermission).toBeDisabled();
  await expect(inputPermission).toBeDisabled();
  await page
    .getByLabel("Allow starting and stopping this Android device")
    .check();
  await inputPermission.check();
  await page
    .getByLabel("Allow reading selected Android device status")
    .uncheck();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.androidDevices),
    )
    .toEqual([]);
  expect(
    await page.evaluate(() => (window as any).__agentTest.approval.scopes),
  ).toEqual(["workspace.read"]);
  await page.getByLabel("Allow reading selected Android device status").check();
  await expect(inputPermission).not.toBeChecked();
  await expect(inputPermission).toBeDisabled();
  await page
    .getByLabel("Allow reading selected Android device status")
    .uncheck();
  const importPermission = page.getByLabel(
    "Allow importing APK files from this project",
  );
  await expect(importPermission).not.toBeChecked();
  await expect(importPermission).toBeDisabled();
  const filePermission = page.getByLabel("Allow reading project files");
  const trashPermission = page.getByLabel(
    "Allow moving project files and folders to Trash",
  );
  await expect(trashPermission).not.toBeChecked();
  await expect(trashPermission).toBeDisabled();
  const renamePermission = page.getByLabel(
    "Allow renaming and moving project files and folders",
  );
  await expect(renamePermission).not.toBeChecked();
  await expect(renamePermission).toBeDisabled();
  const createPermission = page.getByLabel(
    "Allow creating project files and folders",
  );
  await expect(createPermission).not.toBeChecked();
  await expect(createPermission).toBeDisabled();
  const bufferPermission = page.getByLabel(
    "Allow reading unsaved editor buffers",
  );
  await expect(bufferPermission).toBeDisabled();
  await expect(bufferPermission).not.toBeChecked();
  const editPermission = page.getByLabel("Allow editing loaded buffers");
  const savePermission = page.getByLabel("Allow saving editor files to disk");
  await expect(savePermission).not.toBeChecked();
  await expect(savePermission).toBeDisabled();
  await expect(editPermission).toBeDisabled();
  await expect(editPermission).not.toBeChecked();

  const importFiles = page.getByLabel(
    "Allow importing project files as artifacts",
  );
  const exportArtifacts = page.getByLabel(
    "Allow exporting artifacts to new project files",
  );
  await expect(importFiles).toBeDisabled();
  await expect(exportArtifacts).toBeDisabled();
  await filePermission.check();
  await expect(importFiles).toBeEnabled();
  await expect(exportArtifacts).toBeDisabled();
  await importPermission.check();
  await approve.click();
  expect(
    await page.evaluate(() => (window as any).__agentTest.approval.scopes),
  ).not.toContain("artifact.import_file");
  await importPermission.uncheck();
  await importFiles.check();
  await createPermission.check();
  await exportArtifacts.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual([
      "workspace.read",
      "files.read",
      "files.mutate",
      "files.create",
      "artifact.import_file",
      "artifact.export",
    ]);
  await exportArtifacts.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/agent-control-artifact-permissions.png",
  });
  await createPermission.uncheck();
  await expect(exportArtifacts).not.toBeChecked();
  await expect(exportArtifacts).toBeDisabled();
  await filePermission.uncheck();
  await expect(importFiles).not.toBeChecked();
  await expect(importFiles).toBeDisabled();

  const navigateBrowser = page.getByLabel(
    "Allow opening and navigating isolated browser panels",
  );
  const readBrowser = page.getByLabel(
    "Allow reading page text, form structure and browser logs",
  );
  const downloadBrowser = page.getByLabel(
    "Allow downloading page files as private artifacts",
  );
  await expect(downloadBrowser).toBeDisabled();
  await expect(downloadBrowser).not.toBeChecked();
  await navigateBrowser.check();
  await page
    .getByLabel("Allowed browser origins")
    .fill("http://localhost:3000");
  await readBrowser.check();
  await approve.click();
  expect(
    await page.evaluate(() => (window as any).__agentTest.approval.scopes),
  ).not.toContain("browser.download");
  await downloadBrowser.check();
  await approve.click();
  expect(
    await page.evaluate(() => (window as any).__agentTest.approval.scopes),
  ).toContain("browser.download");
  await readBrowser.uncheck();
  await expect(downloadBrowser).not.toBeChecked();
  await expect(downloadBrowser).toBeDisabled();
  await navigateBrowser.uncheck();

  const gitPermission = page.getByLabel(
    "Allow reading Git information in this project",
  );
  await expect(gitPermission).toBeDisabled();
  await expect(gitPermission).not.toBeChecked();
  await filePermission.check();
  await gitPermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(["workspace.read", "files.read", "git.read"]);
  const writeGit = page.getByLabel(
    "Allow requesting Git changes and executing configured Git code",
  );
  const networkGit = page.getByLabel(
    "Allow requesting contact with configured Git remotes",
  );
  await expect(networkGit).toBeDisabled();
  await expect(networkGit).not.toBeChecked();
  await writeGit.check();
  await expect(networkGit).toBeEnabled();
  await networkGit.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual([
      "workspace.read",
      "files.read",
      "git.read",
      "git.write",
      "git.execute",
      "git.network",
    ]);
  const pushGit = page.getByLabel("Allow requesting Git pushes", {
    exact: true,
  });
  await expect(pushGit).not.toBeChecked();
  await pushGit.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual([
      "workspace.read",
      "files.read",
      "git.read",
      "git.write",
      "git.execute",
      "git.network",
      "git.push",
    ]);
  const discardGit = page.getByLabel(
    "Allow requesting discard of working Git changes",
  );
  await expect(discardGit).not.toBeChecked();
  await discardGit.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toContain("git.discard");
  const pullGit = page.getByLabel("Allow requesting Git pulls", {
    exact: true,
  });
  await expect(pullGit).not.toBeChecked();
  await pullGit.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toContain("git.pull");
  await networkGit.uncheck();
  await expect(pullGit).not.toBeChecked();
  await expect(pullGit).toBeDisabled();
  await expect(pushGit).not.toBeChecked();
  await expect(pushGit).toBeDisabled();
  await writeGit.uncheck();
  await expect(discardGit).not.toBeChecked();
  await expect(discardGit).toBeDisabled();
  await expect(networkGit).not.toBeChecked();
  await expect(networkGit).toBeDisabled();
  await filePermission.uncheck();
  await expect(gitPermission).not.toBeChecked();
  await expect(gitPermission).toBeDisabled();
  await filePermission.check();
  await createPermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(["workspace.read", "files.read", "files.mutate", "files.create"]);
  await createPermission.uncheck();
  await renamePermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(["workspace.read", "files.read", "files.mutate", "files.rename"]);
  await renamePermission.uncheck();
  await trashPermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(["workspace.read", "files.read", "files.mutate", "files.trash"]);
  await filePermission.uncheck();
  await expect(trashPermission).not.toBeChecked();
  await expect(trashPermission).toBeDisabled();
  await filePermission.check();
  await importPermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(["workspace.read", "files.read", "artifact.import"]);
  await bufferPermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual([
      "workspace.read",
      "files.read",
      "editor.read",
      "artifact.import",
    ]);

  await editPermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual([
      "workspace.read",
      "files.read",
      "editor.read",
      "editor.write",
      "artifact.import",
    ]);
  await savePermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual([
      "workspace.read",
      "files.read",
      "editor.read",
      "editor.write",
      "files.mutate",
      "artifact.import",
    ]);
  await expect(createPermission).not.toBeChecked();
  await editPermission.uncheck();
  await expect(savePermission).not.toBeChecked();
  await expect(savePermission).toBeDisabled();
  await editPermission.check();
  await expect(savePermission).not.toBeChecked();
  await filePermission.uncheck();
  await expect(editPermission).not.toBeChecked();
  await expect(editPermission).toBeDisabled();

  await expect(bufferPermission).not.toBeChecked();
  await expect(bufferPermission).toBeDisabled();
  await expect(importPermission).not.toBeChecked();
  await expect(importPermission).toBeDisabled();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(["workspace.read"]);
  const installPermission = page.getByLabel(
    "Allow requesting APK installation on this Android device",
  );
  await expect(installPermission).not.toBeChecked();
  await expect(installPermission).toBeDisabled();
  await page.getByLabel("Allow reading selected Android device status").check();
  await page
    .getByLabel("Allow starting and stopping this Android device")
    .check();
  await filePermission.check();
  await importPermission.check();
  await expect(installPermission).toBeEnabled();
  await installPermission.check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual([
      "workspace.read",
      "files.read",
      "artifact.import",
      "android.install",
      "android.read",
      "android.control",
    ]);
  await page
    .getByLabel("Allow reading selected Android device status")
    .uncheck();
  await expect(installPermission).not.toBeChecked();
  await expect(installPermission).toBeDisabled();
  await page.getByLabel("Allow reading selected Android device status").check();
  const launchApps = page.getByLabel("Allow launching approved Android apps");
  const appLogs = page.getByLabel(
    "Allow reading logs from approved Android apps",
  );
  await expect(launchApps).toBeDisabled();
  await expect(appLogs).toBeDisabled();
  await page
    .getByLabel("Allow starting and stopping this Android device")
    .check();
  await launchApps.check();
  await appLogs.check();
  await page
    .getByLabel("Allowed Android packages")
    .fill("org.lomi.inputtest\norg.example.app");
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.androidPackages),
    )
    .toEqual(["org.lomi.inputtest", "org.example.app"]);
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.scopes),
    )
    .toEqual(expect.arrayContaining(["android.launch", "android.logs"]));
  await page.getByLabel("Allowed Android packages").scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/agent-control-android-apps.png",
  });
  await page
    .getByLabel("Allow starting and stopping this Android device")
    .uncheck();
  await expect(launchApps).not.toBeChecked();
  await expect(appLogs).not.toBeChecked();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.approval.androidPackages),
    )
    .toEqual([]);
  await page
    .getByLabel("Allow reading selected Android device status")
    .uncheck();
  await page.evaluate(() => {
    (window as any).__agentTest.pendingInstalls = [
      {
        operationId: "install-one",
        clientLabel: "Fixture client",
        workspaceId: "workspace",
        deviceId: "device-one",
        generation: "generation-one",
        title: "Selected phone",
        relativePath: "build/app.apk",
        artifactId: "apk-one",
        sha256: "a".repeat(64),
        byteLength: 12695,
        secondsRemaining: 90,
      },
    ];
  });
  const install = page.getByRole("button", {
    name: "Install this APK",
    exact: true,
  });
  await expect(install).toBeVisible();
  await expect(page.getByText("build/app.apk (12,695 bytes)")).toBeVisible();
  await expect(page.getByText("a".repeat(64), { exact: true })).toBeVisible();
  await install.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/agent-control-apk-approval.png",
  });
  await page
    .getByRole("button", { name: "Deny installation", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.installDecision),
    )
    .toEqual({ operationId: "install-one", approve: false });
  await install.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.installDecision),
    )
    .toEqual({ operationId: "install-one", approve: true });
  await page.evaluate(() => {
    (window as any).__agentTest.pendingInstalls = [];
    (window as any).__agentTest.pendingProjectOpens = [
      {
        operationId: "open-project-one",
        clientLabel: "Fixture client",
        projectPath: "/new-approved-project",
        workspaceName: "Approved workspace",
        scopes: [
          "workspace.read",
          "workspace.write",
          "panel.create",
          "project.open",
          "files.read",
        ],
        requestKey: "exact-open-request",
        secondsRemaining: 90,
      },
    ];
  });
  const folderApproval = page.getByRole("button", {
    name: "Approve folder",
    exact: true,
  });
  await expect(folderApproval).toBeVisible();
  await expect(
    page.getByText("/new-approved-project", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Approved workspace", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Operation open-project-one; request exact-open-request. Expires in 90s.",
    ),
  ).toBeVisible();
  await folderApproval.scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "test-results/agent-project-open-approval.png",
  });
  await page
    .getByRole("button", { name: "Reject folder", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.projectDecision),
    )
    .toEqual({ operationId: "open-project-one", approved: false });
  await folderApproval.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__agentTest.projectDecision),
    )
    .toEqual({ operationId: "open-project-one", approved: true });
});
