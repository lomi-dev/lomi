import { expect, test } from "@playwright/test";
import { addWorkspace, newSession } from "../../src/model";
import { mockDesktop } from "./desktop";

test.beforeEach(async ({ page }) => {
  let session = addWorkspace(newSession(), "/other", "local:bash", "Other");
  session = addWorkspace(session, "/project", "local:bash", "Project");
  session.sidebar = "workspaces";
  session.rightSidebar = "git";
  session.sidebarSides.git = "right";
  await mockDesktop(page, false, session);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    const repositories = ["first", "second"].map((name) => ({
      root: `/project/${name}`,
      branch: "main",
      changes: [
        { path: "file.txt", index: "M", worktree: " ", originalPath: null },
      ],
    }));
    const state = (desktop.__sourceState = {
      failDiscovery: false,
      partial: false,
      limited: false,
      holdCommit: false,
      releaseCommit: undefined as (() => void) | undefined,
      calls: [] as { command: string; args: any }[],
    });
    desktop.__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: any = {},
    ) => {
      if (command.startsWith("git_")) state.calls.push({ command, args });
      if (command === "git_repositories") {
        if (state.failDiscovery) throw new Error("Repository scan unavailable");
        if (args.root === "/other")
          return {
            repositories: [{ root: "/other", branch: "main", changes: [] }],
            errors: [],
            limited: false,
          };
        return {
          repositories: state.partial ? repositories.slice(1) : repositories,
          errors: state.partial
            ? [{ root: "/project/first", message: "Corrupt repository index" }]
            : [],
          limited: state.limited,
        };
      }
      if (command === "git_commit") {
        if (state.holdCommit)
          await new Promise<void>((resolve) => {
            state.releaseCommit = resolve;
          });
        return;
      }
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await page
    .getByRole("navigation", { name: "Repositories" })
    .getByRole("button", { name: "first 1" })
    .click();
});

test("draft and repository selection survive panel and project switches", async ({
  page,
}) => {
  const draft = page.getByRole("textbox", { name: "Commit message" });
  await draft.fill("Keep this draft\n\nExact text 🦀\n");
  const toggle = page.getByRole("button", {
    name: "Toggle source control (Ctrl+Shift+G)",
  });
  await toggle.click();
  await expect(draft).toHaveCount(0);
  await toggle.click();
  await expect(draft).toHaveValue("Keep this draft\n\nExact text 🦀\n");
  const workspaces = page.getByRole("navigation", { name: "Workspace list" });
  await workspaces.getByRole("button", { name: /^Other / }).click();
  await draft.fill("Other draft");
  await workspaces.getByRole("button", { name: /^Project / }).click();
  await expect(draft).toHaveValue("Keep this draft\n\nExact text 🦀\n");
  await expect(
    page
      .getByRole("navigation", { name: "Repositories" })
      .getByRole("button", { name: "first 1" }),
  ).toHaveAttribute("aria-current", "page");
});

test("in-flight commit remains locked across repository and panel switches", async ({
  page,
}) => {
  await page.evaluate(() => {
    (window as any).__sourceState.holdCommit = true;
  });
  const draft = page.getByRole("textbox", { name: "Commit message" });
  await draft.fill("First commit");
  await page.getByRole("button", { name: "Commit staged changes" }).click();
  const repositories = page.getByRole("navigation", { name: "Repositories" });
  await repositories.getByRole("button", { name: "second 1" }).click();
  await draft.fill("Second repository draft");
  await repositories.getByRole("button", { name: "first 1" }).click();
  await expect(draft).toHaveAttribute("readonly", "");
  const toggle = page.getByRole("button", {
    name: "Toggle source control (Ctrl+Shift+G)",
  });
  await toggle.click();
  await toggle.click();
  await expect(draft).toHaveAttribute("readonly", "");
  await expect(
    page.getByRole("button", { name: "Commit staged changes" }),
  ).toBeDisabled();
  await page.evaluate(() => (window as any).__sourceState.releaseCommit());
  await expect(draft).not.toHaveAttribute("readonly", "");
  await expect(draft).toHaveValue("");
  await repositories.getByRole("button", { name: "second 1" }).click();
  await expect(draft).toHaveValue("Second repository draft");
  expect(
    await page.evaluate(
      () =>
        (window as any).__sourceState.calls.filter(
          (call: any) => call.command === "git_commit",
        ).length,
    ),
  ).toBe(1);
});

test("scan failures preserve repositories and drafts, show errors, and recover", async ({
  page,
}, testInfo) => {
  const draft = page.getByRole("textbox", { name: "Commit message" });
  await draft.fill("Keep after scan error");
  await page.evaluate(() => {
    (window as any).__sourceState.failDiscovery = true;
    window.dispatchEvent(new Event("focus"));
  });
  const scanButton = page.getByRole("button", {
    name: "Repository scan errors",
    exact: true,
  });
  const details = page.getByRole("dialog", {
    name: "Repository scan",
    exact: true,
  });
  await scanButton.click();
  await expect(details).toContainText("Repository scan unavailable");
  await details.getByRole("button", { name: "Close", exact: true }).click();
  await expect(draft).toHaveValue("Keep after scan error");
  await expect(
    page.getByRole("navigation", { name: "Repositories" }).getByRole("button"),
  ).toHaveCount(3);
  await page.evaluate(() => {
    const state = (window as any).__sourceState;
    state.failDiscovery = false;
    state.partial = true;
    state.limited = true;
  });
  await scanButton.click();
  await page.getByRole("button", { name: "Retry repository scan" }).click();
  await scanButton.click();
  await expect(details).toContainText("Corrupt repository index");
  await expect(details).toContainText("Only part of this folder was scanned");
  await details.getByRole("button", { name: "Close", exact: true }).click();
  await expect(draft).toHaveValue("Keep after scan error");
  await page.setViewportSize({ width: 800, height: 420 });
  await expect(
    page.getByRole("button", { name: "Commit staged changes" }),
  ).toBeInViewport();
  await expect(draft).toBeInViewport();
  await page.screenshot({
    path: testInfo.outputPath("source-control-scan-error.png"),
  });
  await page.evaluate(() => {
    const state = (window as any).__sourceState;
    state.partial = false;
    state.limited = false;
  });
  await scanButton.click();
  await page.getByRole("button", { name: "Retry repository scan" }).click();
  await expect(scanButton).toHaveCount(0);
  await expect(draft).toHaveValue("Keep after scan error");
});

test("background refresh reads known repositories without repeating discovery", async ({
  page,
}) => {
  await expect
    .poll(
      () =>
        page.evaluate(() =>
          (window as any).__sourceState.calls.some(
            (call: any) =>
              call.command === "git_repositories" &&
              call.args.knownRoots?.length === 2,
          ),
        ),
      { timeout: 6500 },
    )
    .toBe(true);
  const scans = await page.evaluate(
    () =>
      (window as any).__sourceState.calls.filter(
        (call: any) =>
          call.command === "git_repositories" && !call.args.knownRoots,
      ).length,
  );
  expect(scans).toBe(1);
});
