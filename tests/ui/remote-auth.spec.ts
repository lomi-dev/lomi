import { expect, test, type Page } from "@playwright/test";
import { active, addWorkspace, newSession } from "../../src/model";
import { mockDesktop } from "./desktop";
import {
  emitRemoteAccount,
  mockRemoteAccount,
  remoteMutationCalls,
  remoteSignedIn,
  remoteSignedOut,
} from "./remote-auth";

async function mockWorkspaces(page: Page, shared = false) {
  let session = addWorkspace(newSession(), "/hidden", "local:bash", "Hidden");
  const hiddenId = active(session)!.workspace.id;
  session = addWorkspace(session, "/project", "local:bash", "Current");
  const currentId = active(session)!.workspace.id;
  session.sidebar = "workspaces";
  await mockDesktop(page, false, session);
  if (shared) {
    await page.addInitScript((currentId) => {
      (window as any).__nativeTest.remoteState = {
        qualified: true,
        enabled: true,
        online: true,
        message: null,
        domainEpoch: null,
        workspaces: [
          { id: currentId, shared: true, online: true, message: null },
        ],
        grants: [],
      };
    }, currentId);
  }
  return { hiddenId, currentId };
}

test("signed-out sharing is grey, ignores clicks and keyboard activation, and enables after login", async ({
  page,
}, testInfo) => {
  const { hiddenId } = await mockWorkspaces(page);
  await mockRemoteAccount(page, remoteSignedOut);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page
    .locator(`[data-workspace-id="${hiddenId}"] .workspace-list-item`)
    .click({ button: "right" });
  const share = page.getByRole("menuitem", { name: "Share remotely" });
  await expect(share).toBeVisible();
  await expect(share).toBeDisabled();
  expect(
    await share.evaluate((button) => Number(getComputedStyle(button).opacity)),
  ).toBeLessThan(1);
  await page.keyboard.press("ArrowDown");
  await expect(
    page.getByRole("menuitem", { name: "Customize workspace…" }),
  ).toBeFocused();
  await share.click({ force: true });
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  expect(await remoteMutationCalls(page)).toEqual([]);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
  await page.screenshot({
    path: testInfo.outputPath("remote-signed-out-menu.png"),
  });
  await emitRemoteAccount(page, remoteSignedIn);
  await expect(share).toBeEnabled();
});

test("unknown auth stays disabled and a delayed signed-in snapshot cannot undo logout", async ({
  page,
}) => {
  await mockWorkspaces(page);
  await mockRemoteAccount(page, remoteSignedIn, true);
  await page.goto("/");
  await page.getByRole("button", { name: "Actions for Hidden" }).click();
  const share = page.getByRole("menuitem", { name: "Share remotely" });
  await expect(share).toBeDisabled();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__remoteAuthTest.initialReads.length),
    )
    .toBe(2);
  await emitRemoteAccount(page, { ...remoteSignedOut, revision: 11 });
  await page.evaluate(() => {
    const fixture = (window as any).__remoteAuthTest;
    fixture.deferInitialRead = false;
    fixture.initialReads.forEach((resolve: () => void) => resolve());
  });
  await expect(
    page.getByRole("button", { name: "Sign In", exact: true }),
  ).toBeVisible();
  await expect(share).toBeDisabled();
  expect(await remoteMutationCalls(page)).toEqual([]);
});

test("logout disables an open sharing confirmation while Cancel remains available", async ({
  page,
}, testInfo) => {
  await mockWorkspaces(page);
  await mockRemoteAccount(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Actions for Hidden" }).click();
  await page.getByRole("menuitem", { name: "Share remotely" }).click();
  const dialog = page.getByRole("alertdialog");
  const confirm = dialog.getByRole("button", { name: "Share remotely" });
  await expect(confirm).toBeEnabled();
  await emitRemoteAccount(page, { ...remoteSignedOut, revision: 11 });
  await expect(confirm).toBeDisabled();
  await expect(dialog.getByText(/sign in/i)).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeEnabled();
  await confirm.click({ force: true });
  expect(await remoteMutationCalls(page)).toEqual([]);
  await page.screenshot({
    path: testInfo.outputPath("remote-signed-out-dialog.png"),
  });
  await emitRemoteAccount(page, { ...remoteSignedIn, revision: 12 });
  await expect(confirm).toBeEnabled();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});

test("a failed delayed auth snapshot preserves a newer login event", async ({
  page,
}) => {
  await mockWorkspaces(page);
  await mockRemoteAccount(page, remoteSignedOut, true);
  await page.goto("/");
  await page.getByRole("button", { name: "Actions for Hidden" }).click();
  const share = page.getByRole("menuitem", { name: "Share remotely" });
  await expect(share).toBeDisabled();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__remoteAuthTest.initialReads.length),
    )
    .toBe(2);
  await emitRemoteAccount(page, remoteSignedIn);
  await expect(share).toBeEnabled();
  await page.evaluate(() => {
    const fixture = (window as any).__remoteAuthTest;
    fixture.failInitialRead = true;
    fixture.initialReads.forEach((resolve: () => void) => resolve());
  });
  await expect(share).toBeEnabled();
  await share.click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
});

test("logout disables the footer, stop confirmation, and workspace stop action", async ({
  page,
}) => {
  await mockWorkspaces(page, true);
  await mockRemoteAccount(page);
  await page.goto("/");
  const trigger = page.getByRole("button", { name: "Stop sharing remotely" });
  await trigger.click();
  const dialog = page.getByRole("alertdialog");
  const confirm = dialog.getByRole("button", {
    name: "Stop sharing",
    exact: true,
  });
  await emitRemoteAccount(page, { ...remoteSignedOut, revision: 11 });
  await expect(trigger).toBeDisabled();
  await expect(confirm).toBeDisabled();
  await confirm.click({ force: true });
  expect(await remoteMutationCalls(page)).toEqual([]);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await trigger.click({ force: true });
  await expect(dialog).toHaveCount(0);
  await page.getByRole("button", { name: "Actions for Current" }).click();
  await expect(
    page.getByRole("menuitem", { name: "Stop sharing remotely" }),
  ).toBeDisabled();
});

test("logout during terminal preparation does not issue a sharing mutation", async ({
  page,
}) => {
  await mockWorkspaces(page);
  await mockRemoteAccount(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "start_terminal" && args.request.cwd === "/hidden") {
        await new Promise<void>((resolve) => {
          desktop.__finishRemotePreparation = resolve;
        });
      }
      return invoke(command, args);
    };
  });
  await page.getByRole("button", { name: "Actions for Hidden" }).click();
  await page.getByRole("menuitem", { name: "Share remotely" }).click();
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("button", { name: "Share remotely" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => typeof (window as any).__finishRemotePreparation),
    )
    .toBe("function");
  await emitRemoteAccount(page, { ...remoteSignedOut, revision: 11 });
  await page.evaluate(() => (window as any).__finishRemotePreparation());
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeEnabled();
  expect(await remoteMutationCalls(page)).toEqual([]);
});

test("Remote navigation and browser revocation follow login and logout events", async ({
  page,
}, testInfo) => {
  await mockDesktop(page, false);
  await mockRemoteAccount(page, remoteSignedOut);
  await page.addInitScript(() => {
    const desktop = window as any;
    desktop.__nativeTest.remoteState = {
      qualified: true,
      enabled: true,
      online: true,
      message: null,
      domainEpoch: null,
      workspaces: [],
      grants: [
        {
          id: "grant",
          fingerprint: "fingerprint",
          sessionIds: ["terminal"],
          permissions: "control",
          expiresAt: 9999999999,
          revoked: false,
        },
      ],
    };
  });
  await page.goto("/?window=settings&page=remote");
  const remoteNav = page.getByRole("button", { name: "Remote", exact: true });
  const revoke = page.getByRole("button", { name: "Revoke access" });
  await expect(remoteNav).toBeDisabled();
  await expect(revoke).toBeDisabled();
  await expect(page.getByText(/sign in.*remote/i)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Show workspace" }),
  ).toBeEnabled();
  await expect(page.getByRole("button", { name: "Quit Lomi" })).toBeEnabled();
  await revoke.click({ force: true });
  expect(await remoteMutationCalls(page)).toEqual([]);
  await page.screenshot({
    path: testInfo.outputPath("remote-signed-out-settings.png"),
    fullPage: true,
  });
  await emitRemoteAccount(page, remoteSignedIn);
  await expect(remoteNav).toBeEnabled();
  await expect(revoke).toBeEnabled();
  await emitRemoteAccount(page, { ...remoteSignedOut, revision: 11 });
  await expect(remoteNav).toBeDisabled();
  await expect(revoke).toBeDisabled();
  await page.getByRole("button", { name: "Account", exact: true }).click();
  await remoteNav.click({ force: true });
  await expect(
    page.getByRole("heading", { name: "Account", exact: true }),
  ).toBeVisible();
  expect(await remoteMutationCalls(page)).toEqual([]);
});

test("unqualified hosts block new sharing while retained consent can be stopped", async ({
  page,
}, testInfo) => {
  const { currentId, hiddenId } = await mockWorkspaces(page, true);
  await mockRemoteAccount(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    Object.assign(desktop.__nativeTest.remoteState, {
      qualified: false,
      online: false,
    });
    desktop.__remoteInvoke = async (command: string, args: any) => {
      if (command === "remote_share_workspace")
        desktop.__nativeTest.remoteState.workspaces.find(
          (workspace: any) => workspace.id === args.workspaceId,
        ).shared = args.shared;
      return structuredClone(desktop.__nativeTest.remoteState);
    };
  });
  await page.goto("/");
  await page
    .locator(`[data-workspace-id="${hiddenId}"] .workspace-list-item`)
    .click({ button: "right" });
  const share = page.getByRole("menuitem", { name: "Share remotely" });
  await expect(share).toBeDisabled();
  await share.click({ force: true });
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(page.getByText(/Mac with Apple silicon/)).toBeVisible();
  expect(await remoteMutationCalls(page)).toEqual([]);
  await page.getByRole("menuitem", { name: "Rename workspace" }).focus();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await page
    .locator(`[data-workspace-id="${currentId}"] .workspace-list-item`)
    .click({ button: "right" });
  const stop = page.getByRole("menuitem", { name: "Stop sharing remotely" });
  await expect(stop).toBeEnabled();
  await stop.click();
  await expect(stop).toHaveCount(0);
  expect(await remoteMutationCalls(page)).toEqual([
    {
      command: "remote_share_workspace",
      args: { workspaceId: currentId, shared: false },
    },
  ]);
  await page.screenshot({
    path: testInfo.outputPath("remote-unqualified-workspaces.png"),
  });
});
