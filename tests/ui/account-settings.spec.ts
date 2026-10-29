import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { unavailableState, type AuthState } from "../../src/auth/model";

const signedOut: AuthState = {
  ...unavailableState,
  revision: 1,
  status: "signed-out",
};
const storageLocked: AuthState = {
  ...signedOut,
  status: "storage-locked",
  message: "The account session is unavailable in the system key store.",
};
const signedIn: AuthState = {
  ...signedOut,
  revision: 10,
  status: "signed-in",
  storage: "persistent",
  user: {
    id: "user-1",
    displayName: "Lomi User",
    email: "user@example.test",
    githubLogin: "lomi-user",
    status: "active",
  },
  session: { id: "session-1", expiresAt: "2026-10-01T12:00:00Z" },
};

async function mockAccount(
  page: Page,
  state = signedOut,
  failInitialLoad = false,
) {
  await mockDesktop(page, false);
  await page.addInitScript(
    ({ initial, failInitialLoad }) => {
      const desktop = window as any;
      desktop.__authTest = {
        state: initial,
        calls: [],
        delayedRefresh: false,
        resolveRefresh: null,
      };
      desktop.__authInvoke = async (command: string, args: any) => {
        const fixture = desktop.__authTest;
        fixture.calls.push({ command, args });
        if (command === "auth_get_state" && failInitialLoad) {
          throw new Error("Account connection is unavailable.");
        }
        if (command === "auth_begin_login") {
          fixture.state = {
            ...fixture.state,
            revision: fixture.state.revision + 1,
            status: "authorizing",
            storage: args.storage,
            attempt: {
              id: "attempt-1",
              expiresAt: new Date(Date.now() + 600000).toISOString(),
            },
          };
        }
        if (command === "auth_cancel_login" || command === "auth_sign_out") {
          fixture.state = {
            ...fixture.state,
            revision: fixture.state.revision + 1,
            status: "signed-out",
            user: null,
            session: null,
            attempt: null,
            storage: null,
            remoteRevocationConfirmed:
              command === "auth_sign_out" ? false : null,
          };
        }
        if (command === "auth_refresh_state" && fixture.delayedRefresh) {
          const captured = structuredClone(fixture.state);
          return new Promise((resolve) => {
            fixture.resolveRefresh = () => resolve(captured);
          });
        }
        return fixture.state;
      };
    },
    { initial: state, failInitialLoad },
  );
  await page.goto("/?window=settings&page=account");
  await expect(
    page.getByRole("heading", { name: "Account", exact: true }),
  ).toBeVisible();
}

test("locked storage explains recovery and keeps session-only login an explicit choice", async ({
  page,
}, testInfo) => {
  await mockAccount(page, storageLocked);
  await expect(
    page.getByText(/Unlock your system credential store and retry access/),
  ).toBeVisible();
  const remember = page.getByRole("switch", {
    name: "Remember me on this device",
  });
  await expect(remember).toBeChecked();
  await expect(
    page.getByRole("button", { name: "Sign in with GitHub", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Sign out of this device" }),
  ).toBeEnabled();
  await page.screenshot({
    path: testInfo.outputPath("locked-account-recovery.png"),
    fullPage: true,
  });
  expect(
    await page.evaluate(() =>
      (window as any).__authTest.calls.filter(
        (call: any) => call.command === "auth_begin_login",
      ),
    ),
  ).toEqual([]);
  await remember.uncheck();
  await expect(
    page.getByRole("button", { name: "Sign in with GitHub", exact: true }),
  ).toBeEnabled();
  await page
    .getByRole("button", { name: "Sign in with GitHub", exact: true })
    .click();
  expect(
    await page.evaluate(() =>
      (window as any).__authTest.calls.filter(
        (call: any) => call.command === "auth_begin_login",
      ),
    ),
  ).toEqual([{ command: "auth_begin_login", args: { storage: "session" } }]);
});

test("explicit local sign-out enables a fresh persistent login and reports unconfirmed revocation", async ({
  page,
}) => {
  await mockAccount(page, storageLocked);
  await page.getByRole("button", { name: "Sign out of this device" }).click();
  await expect(
    page.getByText(/The server could not confirm that its session was revoked/),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Sign in with GitHub", exact: true })
    .click();
  expect(
    await page.evaluate(() =>
      (window as any).__authTest.calls.filter((call: any) =>
        ["auth_sign_out", "auth_begin_login"].includes(call.command),
      ),
    ),
  ).toEqual([
    { command: "auth_sign_out", args: {} },
    { command: "auth_begin_login", args: { storage: "persistent" } },
  ]);
});

test("failed durable sign-out keeps recovery blocked without claiming local success", async ({
  page,
}) => {
  await mockAccount(page, storageLocked);
  await page.evaluate(() => {
    const desktop = window as any;
    const invoke = desktop.__authInvoke;
    desktop.__authInvoke = async (command: string, args: unknown) => {
      const result = await invoke(command, args);
      if (command === "auth_sign_out") {
        result.status = "storage-locked";
        result.message =
          "Could not safely persist local sign-out. Unlock storage and retry.";
      }
      return result;
    };
  });
  await page.getByRole("button", { name: "Sign out of this device" }).click();
  await expect(
    page.getByText(
      "Could not safely persist local sign-out. Unlock storage and retry.",
    ),
  ).toBeVisible();
  await expect(page.getByText(/Signed out on this device/)).toHaveCount(0);
  await expect(
    page.getByText(/The server could not confirm that its session was revoked/),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Sign in with GitHub", exact: true }),
  ).toBeDisabled();
});

test("Retry access can restore a locked saved account without starting a new login", async ({
  page,
}) => {
  await mockAccount(page, storageLocked);
  await page.evaluate((state) => {
    (window as any).__authTest.state = state;
  }, signedIn);
  await page.getByRole("button", { name: "Retry access" }).click();
  await expect(page.getByText("Lomi User", { exact: true })).toBeVisible();
  await expect(
    page.getByText(/Unlock your system credential store and retry access/),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      (window as any).__authTest.calls.filter(
        (call: any) => call.command === "auth_begin_login",
      ),
    ),
  ).toEqual([]);
});

test("session-only login waits for browser approval and survives page changes", async ({
  page,
}) => {
  await mockAccount(page);
  await page
    .getByRole("switch", { name: "Remember me on this device" })
    .uncheck();
  await page
    .getByRole("button", { name: "Sign in with GitHub", exact: true })
    .click();
  await expect(page.getByText("Waiting for browser approval.")).toBeVisible();
  await expect(page.getByText(/verification code/i)).toHaveCount(0);
  await page.getByRole("button", { name: "Open browser again" }).click();
  expect(
    await page.evaluate(() =>
      (window as any).__authTest.calls.filter(
        (c: any) => c.command === "auth_open_verification",
      ),
    ),
  ).toEqual([
    { command: "auth_open_verification", args: { attemptId: "attempt-1" } },
    { command: "auth_open_verification", args: { attemptId: "attempt-1" } },
  ]);
  expect(
    await page.evaluate(() =>
      (window as any).__authTest.calls.filter(
        (c: any) => c.command === "auth_begin_login",
      ),
    ),
  ).toEqual([{ command: "auth_begin_login", args: { storage: "session" } }]);
  await page.getByRole("button", { name: "Themes", exact: true }).click();
  await page.getByRole("button", { name: "Account", exact: true }).click();
  await expect(page.getByText("Waiting for browser approval.")).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as any).__authTest.calls.some(
        (c: any) => c.command === "auth_cancel_login",
      ),
    ),
  ).toBe(false);
  await page.getByRole("button", { name: "Cancel sign-in" }).click();
  await expect(
    page.getByRole("button", { name: "Sign in with GitHub", exact: true }),
  ).toBeVisible();
});

test("remembered login can finish with a clearly disclosed session-only fallback", async ({
  page,
}) => {
  await page.setViewportSize({ width: 760, height: 650 });
  await mockAccount(page);
  await expect(
    page.getByText(/otherwise sign in only until Lomi closes/),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Sign in with GitHub", exact: true })
    .click();
  await expect(page.getByText("Waiting for browser approval.")).toBeVisible();

  await page.evaluate(
    async (account) => {
      const desktop = window as any;
      desktop.__authTest.state = account;
      await desktop.__nativeTest.emitEvent("auth-state-changed", account);
    },
    {
      ...signedIn,
      storage: "session",
      message:
        "The system credential store is unavailable. Signed in for this session only; sign in again after closing Lomi.",
    },
  );

  await expect(page.getByText("Lomi User", { exact: true })).toBeVisible();
  await expect(page.getByText("Waiting for browser approval.")).toHaveCount(0);
  await expect(page.getByText(/Signed in for this session only/)).toBeVisible();
  await expect(
    page.getByText("This session only", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("You will need to sign in again after closing Lomi."),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "/tmp/lomi-account-session-fallback.png",
    fullPage: true,
  });

  await expect(
    page.getByRole("button", { name: /Retry (access|connection)/ }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      (window as any).__authTest.calls.filter(
        (call: any) => call.command === "auth_begin_login",
      ),
    ),
  ).toEqual([{ command: "auth_begin_login", args: { storage: "persistent" } }]);
});

test("new logout event wins over an old refresh response", async ({ page }) => {
  await mockAccount(page, { ...signedIn, status: "offline" });
  await expect(
    page.getByText("user@example.test", { exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    (window as any).__authTest.delayedRefresh = true;
  });
  await page.getByRole("button", { name: "Retry connection" }).click();
  await page.evaluate(async () => {
    const desktop = window as any;
    await desktop.__nativeTest.emitEvent("auth-state-changed", {
      ...desktop.__authTest.state,
      revision: 11,
      status: "signed-out",
      user: null,
      session: null,
      storage: null,
    });
    desktop.__authTest.resolveRefresh();
  });
  await expect(
    page.getByText("user@example.test", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Sign in with GitHub", exact: true }),
  ).toBeVisible();
});

test("offline logout communicates local removal without claiming server revocation", async ({
  page,
}) => {
  await mockAccount(page, { ...signedIn, status: "offline" });
  await expect(page.getByText(/Your local work is unaffected/)).toBeVisible();
  await page.getByRole("button", { name: "Sign out of this device" }).click();
  await expect(page.getByText(/server could not confirm/)).toBeVisible();
  await expect(
    page.getByText("user@example.test", { exact: true }),
  ).toHaveCount(0);
});

test("account remains usable at the minimum settings width", async ({
  page,
}) => {
  await page.setViewportSize({ width: 760, height: 650 });
  await mockAccount(page);
  await page
    .getByRole("button", { name: "Sign in with GitHub", exact: true })
    .click();
  await expect(page.getByText("Waiting for browser approval.")).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "/tmp/lomi-account-settings.png",
    fullPage: true,
  });
});

test("failed initial account load can be retried from its error notice", async ({
  page,
}) => {
  await mockAccount(page, signedIn, true);
  await expect(page.getByRole("alert")).toContainText(
    "Account connection is unavailable.",
  );
  await page.getByRole("button", { name: "Retry connection" }).click();
  await expect(page.getByText("Lomi User", { exact: true })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Retry connection" }),
  ).toHaveCount(0);
});
