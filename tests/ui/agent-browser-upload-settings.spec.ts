import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";
test("upload consent presents immutable bytes and sends only a one-use decision", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.uploadRequests = [
      {
        operationId: "upload-one",
        clientLabel: "Fixture client",
        workspaceId: "workspace",
        panelId: "panel",
        elementRef: "f1-e2",
        target: {
          origin: "http://localhost:3000",
          documentUrl: "http://localhost:3000/form",
          frameId: "f1",
          label: "Profile picture",
        },
        fileName: "Zażółć.bin",
        byteLength: 4096,
        sha256: "a".repeat(64),
        secondsRemaining: 60,
      },
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
            terminalProfile: null,
            workspaces: [],
            pending: [],
            pendingControls: [],
            pendingInstalls: [],
            pendingProjectOpens: [],
            pendingSettingsUpdates: [],
            pendingBrowserUploads: desktop.uploadRequests,
            sessions: [],
          },
        };
      if (command === "agent_browser_upload_decide") {
        desktop.uploadDecision = args;
        desktop.uploadRequests = [];
        return;
      }
      return invoke(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  await expect(
    page.getByRole("heading", { name: "Browser upload requests" }),
  ).toBeVisible();
  await expect(
    page.getByText("http://localhost:3000/form", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("a".repeat(64), { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Deny upload", exact: true }).click();
  expect(await page.evaluate(() => (window as any).uploadDecision)).toEqual({
    operationId: "upload-one",
    approve: false,
  });
  await expect(
    page.getByRole("heading", { name: "Browser upload requests" }),
  ).toHaveCount(0);
});
