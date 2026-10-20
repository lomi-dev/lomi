import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import {
  expectRecordedNotification,
  openNotifications,
} from "./application-notification-helpers";

const future = { version: 99, projects: [{ preserve: true }] };

async function prepare(page: Page) {
  await mockDesktop(page, false, future);
  await page.goto("/");
  await expectRecordedNotification(page, "unsupported format");
  await page
    .getByRole("button", { name: "Open Recent Project", exact: true })
    .click();
  await page.getByRole("menuitem", { name: "Open Local Folder…" }).click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    const state = ((window as any).__recoveryTest = {
      requests: [],
      saved: [],
    });
    const invoke = (window as any).__TAURI_INTERNALS__.invoke;
    (window as any).__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: any,
    ) => {
      if (command === "save_session") {
        const request: any = { args: structuredClone(args) };
        state.requests.push(request);
        if (args.recovery)
          await new Promise<void>((resolve, reject) => {
            request.resolve = resolve;
            request.reject = reject;
          });
        const result = await invoke(command, args);
        state.saved.push(request.args);
        return result;
      }
      return invoke(command, args);
    };
  });
}

async function startRecovery(page: Page) {
  await openNotifications(page);
  await page
    .getByRole("button", { name: "Save current layout instead", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__recoveryTest.requests.length),
    )
    .toBe(1);
  await page
    .getByRole("button", { name: "Close notifications", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "Notifications", exact: true }),
  ).toHaveCount(0);
}

async function lifecycleCalls(page: Page) {
  return page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter((call: any) =>
        [
          "plugin:window|destroy",
          "restart_plugins",
          "agent_tasks_drain",
        ].includes(call.command),
      )
      .map((call: any) => call.command),
  );
}

async function addTab(page: Page) {
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
}

for (const event of ["lomi-quit-requested", "plugin-restart-request"]) {
  test(`${event} waits for recovery and saves edits made during recovery`, async ({
    page,
  }) => {
    await prepare(page);
    await startRecovery(page);
    await addTab(page);
    await page.evaluate((event) => {
      void (window as any).__nativeTest.emitEvent(event);
    }, event);
    await expect(
      page.getByRole("dialog", { name: "Preparing to close" }),
    ).toBeVisible();
    expect(await lifecycleCalls(page)).toEqual([]);
    expect(
      await page.evaluate(() => (window as any).__recoveryTest.requests.length),
    ).toBe(1);
    await page.evaluate(() =>
      (window as any).__recoveryTest.requests[0].resolve(),
    );
    await expect
      .poll(() => lifecycleCalls(page))
      .toContain(
        event === "lomi-quit-requested"
          ? "plugin:window|destroy"
          : "restart_plugins",
      );
    const saves = await page.evaluate(
      () => (window as any).__recoveryTest.saved,
    );
    expect(saves[0].recovery).toBe(true);
    expect(saves[0].data.projects[0].workspaces[0].tabs).toHaveLength(1);
    expect(saves.at(-1).recovery).toBe(false);
    expect(saves.at(-1).data.projects[0].workspaces[0].tabs).toHaveLength(2);
  });
}

test("failed recovery blocks closing until an explicit successful recovery retry", async ({
  page,
}) => {
  await prepare(page);
  await startRecovery(page);
  await page.evaluate(() => {
    void (window as any).__nativeTest.emitEvent("lomi-quit-requested");
  });
  await expect(
    page.getByRole("dialog", { name: "Preparing to close" }),
  ).toBeVisible();
  await page.evaluate(() =>
    (window as any).__recoveryTest.requests[0].reject(
      new Error("Recovery fsync failed"),
    ),
  );
  await expectRecordedNotification(page, "Recovery fsync failed");
  await expect(
    page.getByRole("dialog", { name: "Preparing to close" }),
  ).toHaveCount(0);
  expect(await lifecycleCalls(page)).toEqual([]);
  await page.evaluate(() => {
    void (window as any).__nativeTest.emitEvent("lomi-quit-requested");
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "agent_runtime_cancel_close",
          ).length,
      ),
    )
    .toBeGreaterThanOrEqual(2);
  expect(await lifecycleCalls(page)).toEqual([]);
  expect(
    await page.evaluate(() => (window as any).__recoveryTest.requests.length),
  ).toBe(1);
  await openNotifications(page);
  await page
    .getByRole("button", { name: "Save current layout instead", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__recoveryTest.requests.length),
    )
    .toBe(2);
  await page.evaluate(() =>
    (window as any).__recoveryTest.requests[1].resolve(),
  );
  await expect(
    page.getByRole("button", {
      name: "Save current layout instead",
      exact: true,
    }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Close notifications", exact: true })
    .click();
  await page.evaluate(() => {
    void (window as any).__nativeTest.emitEvent("lomi-quit-requested");
  });
  await expect
    .poll(() => lifecycleCalls(page))
    .toContain("plugin:window|destroy");
});

test("session publication waits for recovery, propagates failure and saves the latest layout on retry", async ({
  page,
}) => {
  await prepare(page);
  await startRecovery(page);
  await addTab(page);
  const publish = () =>
    page.evaluate(async () => {
      const modulePath = "/src/agent-runtime/migration-runtime.ts";
      const { acquireSessionPublication } = await import(modulePath);
      const state = (window as any).__recoveryTest;
      state.publication = "pending";
      void acquireSessionPublication()
        .then(async (release: () => Promise<void>) => {
          state.publication = "ready";
          await release();
        })
        .catch((error: Error) => {
          state.publication = error.message;
        });
    });
  await publish();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__nativeTest.runtimeClosing),
    )
    .toBe(true);
  expect(
    await page.evaluate(() => (window as any).__recoveryTest.publication),
  ).toBe("pending");
  await page.evaluate(() =>
    (window as any).__recoveryTest.requests[0].reject(
      new Error("Recovery publication failed"),
    ),
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).__recoveryTest.publication))
    .toBe("Recovery publication failed");
  await openNotifications(page);
  await page
    .getByRole("button", { name: "Save current layout instead", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__recoveryTest.requests.length),
    )
    .toBe(2);
  await page
    .getByRole("button", { name: "Close notifications", exact: true })
    .click();
  await publish();
  expect(
    await page.evaluate(() => (window as any).__recoveryTest.publication),
  ).toBe("pending");
  await page.evaluate(() =>
    (window as any).__recoveryTest.requests[1].resolve(),
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).__recoveryTest.publication))
    .toBe("ready");
  const saved = await page.evaluate(() =>
    (window as any).__recoveryTest.saved.at(-1),
  );
  expect(saved.recovery).toBe(false);
  expect(saved.data.projects[0].workspaces[0].tabs).toHaveLength(2);
});

test("an unrecovered corrupt session with no pending save can close without replacing it", async ({
  page,
}) => {
  await prepare(page);
  await page.evaluate(() => {
    void (window as any).__nativeTest.emitEvent("lomi-quit-requested");
  });
  await expect
    .poll(() => lifecycleCalls(page))
    .toContain("plugin:window|destroy");
  expect(
    await page.evaluate(() => (window as any).__recoveryTest.requests),
  ).toEqual([]);
});
