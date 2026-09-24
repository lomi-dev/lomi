import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("pairing binds terminal execution to the selected available shell", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__terminalApproval = null;
    desktop.__terminalProfiles = [
      { id: "local:zsh", revision: "zsh-native" },
      { id: "local:bash", revision: "bash-native" },
    ];
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
            terminalProfile: desktop.__terminalProfiles[0],
            terminalProfiles: desktop.__terminalProfiles,
            workspaces: [
              {
                id: "workspace",
                projectId: "project",
                name: "Fixture",
                projectName: "Project",
                projectPath: "/project",
              },
            ],
            pending: [
              {
                id: "request",
                clientLabel: "Terminal fixture",
                certificateSha256: "fixture",
                secondsRemaining: 120,
              },
            ],
            pendingControls: [],
            pendingInstalls: [],
            pendingProjectOpens: [],
            pendingSettingsUpdates: [],
            sessions: [],
          },
        };
      if (command === "agent_control_approve") {
        desktop.__terminalApproval = args;
        return;
      }
      return invoke(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  await page.getByLabel("Workspace", { exact: true }).selectOption("workspace");
  const permission = page.getByLabel(
    "Allow terminal creation, command execution and output reads",
  );
  const shell = page.getByLabel("Approved terminal shell");
  const approve = page.getByRole("button", {
    name: "Approve session",
    exact: true,
  });
  await expect(permission).not.toBeChecked();
  await expect(shell).toHaveCount(0);
  await permission.check();
  await expect(shell).toHaveValue("local:zsh");
  await shell.selectOption("local:bash");
  await shell.scrollIntoViewIfNeeded();
  await page.screenshot({ path: "test-results/agent-terminal-shell.png" });
  await approve.click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__terminalApproval))
    .toMatchObject({
      terminalProfileId: "local:bash",
      scopes: [
        "workspace.read",
        "panel.create",
        "terminal.execute",
        "terminal.read",
      ],
    });
  await page.evaluate(() => {
    (window as any).__terminalProfiles = [
      { id: "local:zsh", revision: "zsh-native" },
    ];
  });
  await expect(approve).toBeDisabled();
  await shell.selectOption("local:zsh");
  await expect(approve).toBeEnabled();
  await permission.uncheck();
  await approve.click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__terminalApproval))
    .toMatchObject({ scopes: ["workspace.read"] });
  expect(
    await page.evaluate(
      () => (window as any).__terminalApproval.terminalProfileId,
    ),
  ).toBeUndefined();
});
