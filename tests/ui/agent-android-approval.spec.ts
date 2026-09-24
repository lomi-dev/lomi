import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("Android approval binds provider terms and destructive confirmation to the exact plan", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__androidApproval = {
      decisions: [],
      request: {
        operationId: "operation",
        clientLabel: "Fixture client",
        workspaceId: "work",
        secondsRemaining: 600,
        plan: {
          planId: "plan",
          revision: "first",
          target: "Install private Android tools",
          downloadBytes: 1024,
          downloads: [
            {
              id: "tools",
              name: "Android tools",
              revision: "23.0",
              url: "https://example.invalid/tools",
              bytes: 1024,
              checksum: "fixture",
            },
          ],
        },
        licenses: [
          {
            id: "SDK terms",
            digest: "sdk",
            text: "Fixture SDK provider terms",
          },
          {
            id: "Java terms",
            digest: "java",
            text: "Fixture Java provider terms",
          },
        ],
        action: null,
      },
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
            workspaces: [],
            terminalProfile: null,
            pending: [],
            pendingControls: [],
            pendingInstalls: [],
            pendingProjectOpens: [],
            pendingSettingsUpdates: [],
            pendingAndroidManagement: [desktop.__androidApproval.request],
            sessions: [],
          },
        };
      if (command === "agent_control_decide_android_management") {
        desktop.__androidApproval.decisions.push(args);
        return;
      }
      return invoke(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  const approve = page.getByRole("button", {
    name: "Approve Android operation",
    exact: true,
  });
  await expect(approve).toBeDisabled();
  await page.getByText("SDK terms — provider terms", { exact: true }).click();
  await expect(
    page.getByText("Fixture SDK provider terms", { exact: true }),
  ).toBeVisible();
  const terms = page.getByRole("checkbox", {
    name: "I have read and accept these provider terms",
  });
  await terms.nth(0).check();
  await expect(approve).toBeDisabled();
  await terms.nth(1).check();
  await expect(approve).toBeEnabled();
  // A replaced request cannot inherit consent from the old plan.
  await page.evaluate(() => {
    (window as any).__androidApproval.request.plan.revision = "second";
  });
  await expect(terms.nth(0)).not.toBeChecked();
  await expect(approve).toBeDisabled();
  await terms.nth(0).check();
  await terms.nth(1).check();
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__androidApproval.decisions),
    )
    .toEqual([
      {
        operationId: "operation",
        revision: "second",
        approve: true,
        accepted: ["sdk", "java"],
        confirmation: null,
      },
    ]);
  await page.evaluate(() => {
    const request = (window as any).__androidApproval.request;
    request.operationId = "delete";
    request.plan.revision = "third";
    request.plan.target = "Delete test phone and all its data";
    request.plan.downloads = [];
    request.licenses = [];
    request.action = {
      type: "delete",
      deviceId: "device",
      confirmation: "Test phone",
    };
  });
  const confirmation = page.getByLabel(
    "Erase all data: type “Test phone” to confirm",
  );
  await expect(confirmation).toBeVisible();
  await expect(approve).toBeDisabled();
  await confirmation.fill("test phone");
  await expect(approve).toBeDisabled();
  await confirmation.fill("Test phone");
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__androidApproval.decisions.at(-1)),
    )
    .toEqual({
      operationId: "delete",
      revision: "third",
      approve: true,
      accepted: [],
      confirmation: "Test phone",
    });
  await page.evaluate(() => {
    const request = (window as any).__androidApproval.request;
    request.operationId = "reset";
    request.plan.revision = "fourth";
    request.plan.target = "Reset Android preferences";
    request.action = {
      type: "restore_metadata",
      file: "preferences",
      reset: true,
    };
  });
  const reset = page.getByLabel(
    "Reset metadata: type “RESET PREFERENCES” to confirm",
  );
  await expect(reset).toBeVisible();
  await expect(approve).toBeDisabled();
  await reset.fill("RESET PREFERENCES");
  await approve.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__androidApproval.decisions.at(-1)),
    )
    .toEqual({
      operationId: "reset",
      revision: "fourth",
      approve: true,
      accepted: [],
      confirmation: "RESET PREFERENCES",
    });
});
