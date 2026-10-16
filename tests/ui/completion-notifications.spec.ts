import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

interface FixtureOptions {
  refreshError?: string;
  stateError?: string;
  cached?: boolean;
  sourceEnabled?: boolean;
  missingSource?: boolean;
  mutationError?: string;
  partialMutation?: boolean;
  authStatus?: "offline" | "error" | "signed-out";
}

async function fixture(page: Page, gate = false, options: FixtureOptions = {}) {
  await mockDesktop(page, false);
  await page.addInitScript(
    ({ featureEnabled, options }) => {
      const desktop = window as any;
      desktop.__authInvoke = async () => ({
        revision: 10,
        status: options.authStatus ?? "signed-in",
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
        available: !options.refreshError || !!options.cached,
        featureEnabled,
        preferences: {
          enabled: !!options.cached,
          consentEpoch: 1,
          revision: 1,
        },
        sources:
          options.cached && !options.missingSource
            ? [
                {
                  id: "source-a",
                  enabled: options.sourceEnabled ?? true,
                  revision: 1,
                  revoked: false,
                  current: true,
                },
              ]
            : [],
        installations: options.cached
          ? [
              {
                id: "phone-a",
                label: "Ada phone",
                platform: "ios",
                enabled: true,
                retired: false,
                revision: 1,
              },
            ]
          : [],
        currentSourceId:
          options.cached && !options.missingSource ? "source-a" : null,
        errorCode: options.stateError ?? null,
      };
      desktop.__completionFixture = {
        local,
        cloud,
        delaySave: false,
        delayCloud: false,
        resolveCloud: null,
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
        if (command === "push_refresh_state" && options.refreshError)
          throw {
            code: options.refreshError,
            message: "private server diagnostic",
          };
        if (command === "push_set_current_source_enabled") {
          if (desktop.__completionFixture.delayCloud) {
            return new Promise((resolve) => {
              desktop.__completionFixture.resolveCloud = () =>
                resolve({
                  ...cloud,
                  revision: 30,
                  preferences: { ...cloud.preferences, enabled: true },
                });
            });
          }
          if (options.partialMutation) {
            cloud.sources = [
              {
                id: "source-a",
                enabled: true,
                revision: 2,
                revoked: false,
                current: true,
              },
            ];
            cloud.currentSourceId = "source-a";
            cloud.revision++;
            throw { code: "REVISION_CONFLICT" };
          }
          if (options.mutationError) throw { code: options.mutationError };
          if (args.enabled) {
            cloud.currentSourceId = "source-a";
            cloud.sources = [
              {
                id: "source-a",
                enabled: true,
                revision: 2,
                revoked: false,
                current: true,
              },
            ];
          } else if (cloud.sources[0]) cloud.sources[0].enabled = false;
          cloud.revision++;
          cloud.preferences.revision++;
          cloud.preferences.consentEpoch++;
          if (args.enabled) cloud.preferences.enabled = true;
        }
        return structuredClone(cloud);
      };
      function expectRevision(actual: unknown, expected: number) {
        if (actual !== expected) throw { code: "REVISION_CONFLICT" };
      }
    },
    { featureEnabled: gate, options },
  );
  await page.goto("/?window=settings&page=notifications");
  await expect(
    page.getByRole("heading", { name: "Notifications", exact: true }),
  ).toBeVisible();
}

test("local opt-in works with cloud gate closed and never issue push mutations", async ({
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
  await expect(page.getByRole("switch")).toHaveCount(3);
  await expect(
    page.getByRole("button", { name: /test|Disable receiving/ }),
  ).toHaveCount(0);
  await expect(local).not.toBeChecked();
  await expect(
    page.getByRole("switch", {
      name: "Only when the main Lomi window is inactive",
    }),
  ).toBeChecked();
  await expect(cloud).toBeDisabled();
  await local.click();
  await expect(local).toBeChecked();
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
  ).toHaveLength(0);
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
  ).toHaveCount(0);
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
  ).toHaveCount(0);
});

for (const [code, diagnostic] of [
  ["NOT_FOUND", "The server could not find the phone notification settings"],
  [
    "SERVICE_UNAVAILABLE",
    "The notification service is temporarily unavailable",
  ],
  [
    "INVALID_RESPONSE",
    "The notification service returned an unexpected response",
  ],
  ["TRANSPORT_UNKNOWN", "Could not reach the notification service"],
]) {
  test(`signed-in push refresh ${code} shows a service diagnostic and keeps consent disabled`, async ({
    page,
  }) => {
    await fixture(page, true, { refreshError: code });
    await expect(
      page.getByRole("alert").filter({ hasText: diagnostic }),
    ).toBeVisible();
    await expect(
      page.getByText(
        /verified online Lomi account|Sign in to Lomi to manage phone|Check your Lomi account connection to manage phone/,
      ),
    ).toHaveCount(0);
    await expect(page.getByText("private server diagnostic")).toHaveCount(0);
    for (const name of ["Push notifications on my phones"]) {
      await expect(
        page.getByRole("switch", { name, exact: true }),
      ).toBeDisabled();
      await expect(
        page.getByRole("switch", { name, exact: true }),
      ).not.toBeChecked();
    }
    await expect(
      page.getByRole("switch", {
        name: "Notify when tasks finish",
        exact: true,
      }),
    ).toBeEnabled();
    const calls = await page.evaluate(() => (window as any).__nativeTest.calls);
    expect(
      calls.filter(
        (call: any) =>
          call.command.startsWith("push_") &&
          !["push_get_state", "push_refresh_state"].includes(call.command),
      ),
    ).toEqual([]);
  });
}

test("cached phone consent cannot be changed or tested after refresh failure", async ({
  page,
}) => {
  await fixture(page, true, {
    refreshError: "SERVICE_UNAVAILABLE",
    cached: true,
  });
  await expect(
    page.getByRole("alert").filter({ hasText: "temporarily unavailable" }),
  ).toBeVisible();
  for (const name of ["Push notifications on my phones"]) {
    await expect(page.getByRole("switch", { name, exact: true })).toBeChecked();
    await expect(
      page.getByRole("switch", { name, exact: true }),
    ).toBeDisabled();
  }
  await expect(page.getByRole("button", { name: /Ada phone/ })).toHaveCount(0);
});

test("native preference storage error disables cached phone controls", async ({
  page,
}) => {
  await fixture(page, true, {
    stateError: "STORAGE_UNAVAILABLE",
    cached: true,
  });
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "could not access notification preferences" }),
  ).toBeVisible();
  await expect(
    page.getByRole("switch", { name: "Push notifications on my phones" }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Send test to Ada phone" }),
  ).toHaveCount(0);
  await expect(
    page.getByText(
      /verified online Lomi account|Sign in to Lomi to manage phone/,
    ),
  ).toHaveCount(0);
});

for (const [authStatus, notice] of [
  ["offline", "Reconnect your Lomi account to manage phone notifications."],
  [
    "error",
    "Check your Lomi account connection to manage phone notifications.",
  ],
] as const) {
  test(`${authStatus} account has an account notice and disabled phone controls`, async ({
    page,
  }) => {
    await fixture(page, true, { authStatus, refreshError: "SESSION_REQUIRED" });
    await expect(page.getByText(notice, { exact: true })).toBeVisible();
    await expect(
      page.getByRole("switch", { name: "Push notifications on my phones" }),
    ).toBeDisabled();
    await expect(
      page.getByText("Check your account connection before continuing."),
    ).toHaveCount(0);
  });
}

for (const options of [
  {},
  { cached: true, sourceEnabled: false },
  { cached: true, missingSource: true },
]) {
  test(`explicit phone opt-in enables this computer ${JSON.stringify(options)}`, async ({
    page,
  }) => {
    await fixture(page, true, options);
    const phone = page.getByRole("switch", {
      name: "Push notifications on my phones",
    });
    await expect(phone).not.toBeChecked();
    await phone.click();
    await expect(phone).toBeChecked();
    const mutations = await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (c: any) => c.command === "push_set_current_source_enabled",
      ),
    );
    expect(mutations).toHaveLength(1);
    expect(mutations[0].args.enabled).toBe(true);
  });
}

test("phone off disables this computer while preserving account consent", async ({
  page,
}) => {
  await fixture(page, true, { cached: true });
  const phone = page.getByRole("switch", {
    name: "Push notifications on my phones",
  });
  await expect(phone).toBeChecked();
  await phone.click();
  await expect(phone).not.toBeChecked();
  expect(
    await page.evaluate(
      () => (window as any).__completionFixture.cloud.preferences.enabled,
    ),
  ).toBe(true);
});

test("failed phone change keeps confirmed state and makes no automatic mutation retry", async ({
  page,
}) => {
  await fixture(page, true, { mutationError: "TRANSPORT_UNKNOWN" });
  const phone = page.getByRole("switch", {
    name: "Push notifications on my phones",
  });
  await phone.click();
  await expect(phone).not.toBeChecked();
  await expect(
    page.getByRole("alert").filter({ hasText: "Could not reach" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (c: any) => c.command === "push_set_current_source_enabled",
        ).length,
    ),
  ).toBe(1);
});

test("notification controls are exclusive to Notifications and accessible by keyboard", async ({
  page,
}) => {
  await fixture(page, true);
  const local = page.getByRole("switch", {
    name: "Notify when tasks finish",
    exact: true,
  });
  await local.focus();
  await page.keyboard.press("Space");
  await expect(local).toBeChecked();
  await page.getByRole("button", { name: "Account", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Account", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("switch")).toHaveCount(0);
  await page.evaluate(async () =>
    (window as any).__nativeTest.emitEvent("settings-tab-requested", {
      revision: 1,
      page: "notifications",
    }),
  );
  await expect(
    page.getByRole("heading", { name: "Notifications", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("switch")).toHaveCount(3);
});

test("partial phone opt-in failure displays confirmed combined state", async ({
  page,
}) => {
  await fixture(page, true, { partialMutation: true });
  const phone = page.getByRole("switch", {
    name: "Push notifications on my phones",
  });
  await phone.click();
  await expect(
    page.getByRole("alert").filter({ hasText: "Settings changed" }),
  ).toBeVisible();
  await expect(phone).not.toBeChecked();
  expect(
    await page.evaluate(
      () => (window as any).__completionFixture.cloud.sources[0].enabled,
    ),
  ).toBe(true);
});

test("account change discards pending phone settings acknowledgement", async ({
  page,
}) => {
  await fixture(page, true);
  await page.evaluate(() => {
    (window as any).__completionFixture.delayCloud = true;
  });
  await page
    .getByRole("switch", { name: "Push notifications on my phones" })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => !!(window as any).__completionFixture.resolveCloud),
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
    desktop.__completionFixture.resolveCloud();
  });
  const phone = page.getByRole("switch", {
    name: "Push notifications on my phones",
  });
  await expect(phone).not.toBeChecked();
  await expect(phone).toBeDisabled();
});
