import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function fixture(page: Page, gate = false) {
  await mockDesktop(page, false);
  await page.addInitScript((featureEnabled) => {
    const desktop = window as any;
    desktop.__authInvoke = async () => ({
      revision: 10,
      status: "signed-in",
      storage: "session",
      user: {
        id: "user-a",
        displayName: "Ada",
        email: "ada@example.test",
        githubLogin: "ada",
        status: "active",
      },
      session: {
        id: "session-a",
        expiresAt: new Date(Date.now() + 3600000).toISOString(),
      },
      attempt: null,
      message: null,
      remoteRevocationConfirmed: null,
    });
    const local = {
      revision: 1,
      accountId: "user-a",
      available: true,
      preferences: {
        schemaVersion: 1,
        enabled: false,
        backgroundOnly: true,
        consentEpoch: 1,
        revision: 1,
      },
      permissionStatus: "unknown",
      errorCode: null,
    };
    const cloud = {
      revision: 1,
      accountId: "user-a",
      available: true,
      featureEnabled,
      preferences: { enabled: false, consentEpoch: 1, revision: 1 },
      sources: [],
      installations: [],
      currentSourceId: null,
      errorCode: null,
    };
    desktop.__completionFixture = {
      local,
      cloud,
      delaySave: false,
      resolveSave: null,
    };
    desktop.__completionInvoke = async (command: string, args: any) => {
      if (command === "desktop_notifications_report_permission") {
        local.permissionStatus = args.denied ? "denied" : "unknown";
        local.revision++;
        return structuredClone(local);
      }
      if (command === "desktop_notifications_save_preferences") {
        if (desktop.__completionFixture.delaySave) {
          return new Promise((resolve) => {
            desktop.__completionFixture.resolveSave = () =>
              resolve({
                ...local,
                revision: 30,
                preferences: { ...local.preferences, enabled: true },
              });
          });
        }
        expectRevision(args.expectedRevision, local.preferences.revision);
        local.revision++;
        local.preferences.revision++;
        if (local.preferences.enabled !== args.enabled)
          local.preferences.consentEpoch++;
        local.preferences.enabled = args.enabled;
        local.preferences.backgroundOnly = args.backgroundOnly;
        return structuredClone(local);
      }
      if (command === "desktop_notifications_send_test")
        return { status: "plugin-accepted" };
      if (command.startsWith("desktop_notifications_"))
        return structuredClone(local);
      if (command === "push_update_account_preferences") {
        expectRevision(args.expectedRevision, cloud.preferences.revision);
        cloud.revision++;
        cloud.preferences.revision++;
        cloud.preferences.consentEpoch++;
        cloud.preferences.enabled = args.enabled;
      }
      return structuredClone(cloud);
    };
    function expectRevision(actual: unknown, expected: number) {
      if (actual !== expected) throw { code: "REVISION_CONFLICT" };
    }
  }, gate);
  await page.goto("/?window=settings&page=account");
  await expect(
    page.getByRole("heading", { name: "Account", exact: true }),
  ).toBeVisible();
}

test("local opt-in and test work with cloud gate closed and never issue push mutations", async ({
  page,
}, testInfo) => {
  await fixture(page);
  const local = page.getByRole("switch", {
    name: "Notify when tasks finish",
    exact: true,
  });
  const cloud = page.getByRole("switch", {
    name: "Push notifications on my phones",
  });
  await expect(local).not.toBeChecked();
  await expect(
    page.getByRole("switch", {
      name: "Only when the main Lomi window is inactive",
    }),
  ).toBeChecked();
  await expect(cloud).toBeDisabled();
  await local.click();
  await expect(local).toBeChecked();
  await page
    .getByRole("button", { name: "Send test to this computer" })
    .click();
  await expect(
    page.getByText(/Test requested. Check for a Lomi alert/),
  ).toBeVisible();
  const calls = await page.evaluate(() => (window as any).__nativeTest.calls);
  expect(
    calls.filter(
      (call: any) =>
        call.command.startsWith("push_") &&
        !["push_get_state", "push_refresh_state"].includes(call.command),
    ),
  ).toEqual([]);
  expect(
    calls.filter(
      (call: any) => call.command === "desktop_notifications_send_test",
    ),
  ).toHaveLength(1);
  expect(
    calls.some((call: any) => call.command === "plugin:notification|notify"),
  ).toBe(false);
  await page.screenshot({
    path: testInfo.outputPath("completion-account.png"),
    fullPage: true,
  });
});

test("permission denial leaves local consent off and excludes test", async ({
  page,
}) => {
  await fixture(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.agentNotificationPermission = false;
  });
  const local = page.getByRole("switch", {
    name: "Notify when tasks finish",
    exact: true,
  });
  await local.click();
  await expect(local).not.toBeChecked();
  await expect(page.getByText(/Notifications are blocked/)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Send test to this computer" }),
  ).toBeDisabled();
  const calls = await page.evaluate(() => (window as any).__nativeTest.calls);
  expect(
    calls.some(
      (call: any) => call.command === "desktop_notifications_save_preferences",
    ),
  ).toBe(false);
});

test("account change rejects an old completion-settings acknowledgement", async ({
  page,
}) => {
  await fixture(page);
  await page.evaluate(() => {
    (window as any).__completionFixture.delaySave = true;
  });
  await page
    .getByRole("switch", { name: "Notify when tasks finish", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => !!(window as any).__completionFixture.resolveSave),
    )
    .toBe(true);
  await page.evaluate(async () => {
    const desktop = window as any;
    await desktop.__nativeTest.emitEvent("auth-state-changed", {
      revision: 20,
      status: "signed-out",
      user: null,
      session: null,
      attempt: null,
      storage: null,
      message: null,
      remoteRevocationConfirmed: true,
    });
    desktop.__completionFixture.resolveSave();
  });
  const local = page.getByRole("switch", {
    name: "Notify when tasks finish",
    exact: true,
  });
  await expect(local).not.toBeChecked();
  await expect(local).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Send test to this computer" }),
  ).toBeDisabled();
});
