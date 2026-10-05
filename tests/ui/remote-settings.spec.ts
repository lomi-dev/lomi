import { test, expect } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockRemoteAccount } from "./remote-auth";

test("settings can resume idle remote and user gestures record activity", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockRemoteAccount(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const state = {
      qualified: true,
      enabled: false,
      paused: true,
      online: false,
      message: "Remote paused after an hour of inactivity.",
      domainEpoch: null,
      workspaces: [
        { id: "workspace", shared: true, online: false, message: null },
      ],
      grants: [],
    };
    desktop.__remoteInvoke = async (command: string) => {
      if (command === "remote_resume")
        Object.assign(state, {
          paused: false,
          enabled: true,
          online: true,
          message: null,
        });
      return structuredClone(state);
    };
  });
  await page.goto("/?window=settings&page=remote");
  const resume = page.getByRole("button", {
    name: "Resume remote",
    exact: true,
  });
  await expect(resume).toBeVisible();
  await resume.click();
  await expect(resume).toHaveCount(0);
  const commands = await page.evaluate(() =>
    (window as any).__nativeTest.calls.map((c: any) => c.command),
  );
  expect(commands).toContain("remote_note_activity");
  expect(commands).toContain("remote_resume");
});

test("Remote uses workspace sharing and keeps browser revocation without manual setup", async ({
  page,
}, testInfo) => {
  await mockDesktop(page, false);
  await mockRemoteAccount(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const state = {
      qualified: true,
      enabled: true,
      online: true,
      message: null,
      domainEpoch: null,
      workspaces: [
        { id: "workspace", shared: true, online: true, message: null },
      ],
      grants: [
        {
          id: "grant",
          sessionIds: ["terminal"],
          permissions: "control",
          expiresAt: 9999999999,
          revoked: false,
        },
      ],
    };
    desktop.__remoteInvoke = async (command: string) => {
      if (command === "remote_revoke_grant") state.grants[0].revoked = true;
      return state;
    };
  });
  await page.goto("/?window=settings&page=remote");
  await expect(
    page.getByRole("heading", { name: "Remote", exact: true }),
  ).toBeVisible();
  await expect(page.getByText(/Right-click a workspace/)).toBeVisible();
  await expect(page.getByRole("textbox")).toHaveCount(0);
  await expect(page.getByRole("checkbox")).toHaveCount(0);
  await page.getByRole("button", { name: "Revoke access" }).click();
  await expect(page.getByText("No browsers have access.")).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("remote-workspace-settings.png"),
    fullPage: true,
  });
});

test("unqualified hosts explain hosting requirements and retain browser revocation", async ({
  page,
}, testInfo) => {
  await mockDesktop(page, false);
  await mockRemoteAccount(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const state = {
      qualified: false,
      enabled: false,
      paused: true,
      online: false,
      message: null,
      domainEpoch: null,
      workspaces: [
        { id: "workspace", shared: true, online: false, message: null },
      ],
      grants: [
        {
          id: "grant",
          sessionIds: ["terminal"],
          permissions: "control",
          expiresAt: 9999999999,
          revoked: false,
        },
      ],
    };
    desktop.__remoteInvoke = async (command: string) => {
      if (command === "remote_revoke_grant") state.grants[0].revoked = true;
      return structuredClone(state);
    };
  });
  await page.goto("/?window=settings&page=remote");
  await expect(page.getByText(/Mac with Apple silicon/)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Resume remote" }),
  ).toBeDisabled();
  await expect(
    page.getByText(/including terminals in unshared workspaces/),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("remote-unqualified-settings.png"),
    fullPage: true,
  });
  await page.getByRole("button", { name: "Revoke access" }).click();
  await expect(page.getByText("No browsers have access.")).toBeVisible();
  const commands = await page.evaluate(() =>
    (window as any).__nativeTest.calls.map((call: any) => call.command),
  );
  expect(commands).toContain("remote_revoke_grant");
  expect(commands).not.toContain("remote_resume");
});
