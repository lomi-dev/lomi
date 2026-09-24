import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import type { GitChange, GitFileDiff } from "../../src/api";
import { mockDesktop } from "./desktop";

async function openSourceControl(page: Page, changes: GitChange[]) {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate((initialChanges) => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    const state = {
      changes: initialChanges,
      calls: [] as { command: string; args: Record<string, any> }[],
      failStage: false,
      failCommit: false,
      failDiscard: false,
      failDiff: false,
      remoteError: "",
      holdRemote: false,
      releaseRemote: undefined as (() => void) | undefined,
      pulledContent: null as string | null,
      diffs: {} as Record<string, GitFileDiff>,
      diffDelays: {} as Record<string, number>,
    };
    desktop.__sourceControlTest = state;
    desktop.__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: Record<string, any> = {},
    ) => {
      if (!command.startsWith("git_") && command !== "ignore_project_item")
        return invoke(command, args);
      state.calls.push({ command, args });
      if (command === "git_status")
        return { root: args.root, branch: "main", changes: state.changes };
      if (command === "git_repositories")
        return {
          repositories: [
            { root: args.root, branch: "main", changes: state.changes },
          ],
          errors: [],
          limited: false,
        };
      if (command === "git_remotes") return ["origin", "upstream"];
      if (["git_fetch", "git_pull", "git_push"].includes(command)) {
        if (state.holdRemote)
          await new Promise<void>((resolve) => {
            state.releaseRemote = resolve;
          });
        if (state.remoteError) throw new Error(state.remoteError);
        if (command === "git_pull" && state.pulledContent !== null) {
          const file =
            desktop.__nativeTest.editorFiles[`${args.root}/README.md`];
          file.content = state.pulledContent;
          file.revision += "+pulled";
        }
        return;
      }
      if (command === "git_diff") {
        if (state.diffDelays[args.path])
          await new Promise((resolve) =>
            setTimeout(resolve, state.diffDelays[args.path]),
          );
        if (state.failDiff)
          throw new Error("Unable to read this file's changes");
        return (
          state.diffs[args.path] ?? {
            patch: `@@ -1,3 +1,4 @@\n first line\n-${args.staged ? "HEAD" : "index"} version\n+${args.staged ? "staged" : "working"} version\n+added line\n last line\n`,
            truncated: false,
            notice: null,
          }
        );
      }
      if (command === "git_history") return invoke(command, args);
      if (command === "ignore_project_item") return;
      if (command === "git_discard") {
        if (state.failDiscard) throw new Error("Cannot restore the file");
        state.changes = state.changes.flatMap((change) =>
          change.path !== args.change.path
            ? [change]
            : [" ", "?"].includes(change.index)
              ? []
              : [{ ...change, worktree: " " }],
        );
        return;
      }
      if (command === "git_stage") {
        if (state.failStage) throw new Error("Cannot update the index");
        state.changes = state.changes.map((change) => {
          if (!args.paths.includes(change.path)) return change;
          if (args.stage)
            return {
              ...change,
              index: change.worktree === "?" ? "A" : change.worktree,
              worktree: " ",
            };
          return {
            ...change,
            index: change.index === "A" ? "?" : " ",
            worktree: change.index === "A" ? "?" : "M",
          };
        });
        return;
      }
      if (command === "git_commit") {
        if (state.failCommit)
          throw new Error("Commit hook rejected the message");
        state.changes = state.changes
          .filter((change) => change.worktree !== " ")
          .map((change) => ({
            ...change,
            index: change.worktree === "?" ? "?" : " ",
          }));
      }
    };
    window.dispatchEvent(new Event("focus"));
  }, changes);
  await page
    .getByRole("button", {
      name: "Toggle source control (Ctrl+Shift+G)",
      exact: true,
    })
    .click();
  await expect(page.getByRole("tab", { name: /^Changes/ })).toHaveText(
    `Changes (${changes.length})`,
  );
}

const changed = (
  path: string,
  index = " ",
  worktree = "M",
  originalPath: string | null = null,
): GitChange => ({
  path,
  index,
  worktree,
  originalPath,
});

test("shows sibling repositories together and scopes actions to the selected repository", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__multiRepoCalls = [];
    const repositories = [
      {
        root: "/project/first",
        branch: "main",
        changes: [
          { path: "one.txt", originalPath: null, index: "M", worktree: " " },
        ],
      },
      {
        root: "/project/second",
        branch: "feature",
        changes: [
          { path: "two.txt", originalPath: null, index: " ", worktree: "M" },
        ],
      },
    ];
    desktop.__TAURI_INTERNALS__.invoke = (
      command: string,
      args: Record<string, any> = {},
    ) => {
      if (command === "git_status") return Promise.resolve(null);
      if (command === "git_repositories")
        return Promise.resolve({ repositories, errors: [], limited: false });
      if (command.startsWith("git_"))
        desktop.__multiRepoCalls.push({ command, args });
      return invoke(command, args);
    };
    window.dispatchEvent(new Event("focus"));
  });
  await page
    .getByRole("button", { name: "Toggle source control (Ctrl+Shift+G)" })
    .click();
  const repositories = page.getByRole("navigation", { name: "Repositories" });
  await expect(
    repositories.getByRole("button", { name: "All repositories 2" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "one.txt" })).toBeVisible();
  await expect(page.getByRole("button", { name: "two.txt" })).toBeVisible();
  await page.getByRole("button", { name: "two.txt" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__multiRepoCalls.find(
            (call: any) => call.command === "git_diff",
          )?.args,
      ),
    )
    .toMatchObject({ root: "/project/second", path: "two.txt", staged: false });
  await repositories.getByRole("button", { name: "first 1" }).click();
  await page
    .getByRole("textbox", { name: "Commit message" })
    .fill("Draft for first");
  await repositories.getByRole("button", { name: "second 1" }).click();
  await page.getByRole("checkbox", { name: "Stage two.txt" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__multiRepoCalls.find(
            (call: any) => call.command === "git_stage",
          )?.args,
      ),
    )
    .toMatchObject({
      root: "/project/second",
      paths: ["two.txt"],
      stage: true,
    });
  await repositories.getByRole("button", { name: "first 1" }).click();
  await expect(
    page.getByRole("textbox", { name: "Commit message" }),
  ).toHaveValue("Draft for first");
  await page.getByRole("button", { name: "Commit staged changes" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__multiRepoCalls.find(
            (call: any) => call.command === "git_commit",
          )?.args,
      ),
    )
    .toMatchObject({ root: "/project/first", message: "Draft for first" });
});

test("fetch and pull run explicitly, report progress and errors, and refresh history", async ({
  page,
}) => {
  await openSourceControl(page, [changed("README.md", "M", " ")]);
  const fetch = page.getByRole("button", { name: "Fetch", exact: true });
  const pull = page.getByRole("menuitem", { name: "Pull", exact: true });
  const remoteStatus = page.locator(".git-remote-status");
  const draft = page.getByRole("textbox", {
    name: "Commit message",
    exact: true,
  });
  await draft.fill("Keep this commit draft");
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls.filter((call: any) =>
        ["git_fetch", "git_pull"].includes(call.command),
      ),
    ),
  ).toEqual([]);
  await page.evaluate(() => {
    (window as any).__sourceControlTest.holdRemote = true;
  });
  await fetch.click();
  await page.getByRole("menuitem", { name: "Fetch", exact: true }).click();
  await expect(remoteStatus).toHaveText("Fetching…");
  await expect(fetch).toBeDisabled();
  await expect(
    page.getByRole("menu", { name: "Git remote actions" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Commit staged changes", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("checkbox", { name: "Unstage README.md", exact: true }),
  ).toBeDisabled();
  await page.getByRole("tab", { name: "History", exact: true }).click();
  await expect(
    page.getByText("No commits yet.", { exact: true }),
  ).toBeVisible();
  const historyRequests = await page.evaluate(
    () =>
      (window as any).__sourceControlTest.calls.filter(
        (call: any) => call.command === "git_history",
      ).length,
  );
  await page.evaluate(() => {
    (window as any).__sourceControlTest.releaseRemote();
  });
  await expect(remoteStatus).toHaveText("Fetch complete.");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__sourceControlTest.calls.filter(
            (call: any) => call.command === "git_history",
          ).length,
      ),
    )
    .toBeGreaterThan(historyRequests);

  await fetch.click();
  await pull.click();
  await expect(remoteStatus).toHaveText("Pulling…");
  await expect(fetch).toBeDisabled();
  await expect(
    page.getByRole("menu", { name: "Git remote actions" }),
  ).toHaveCount(0);
  await page.evaluate(() => {
    const state = (window as any).__sourceControlTest;
    state.remoteError = "Cannot fast-forward: branches have diverged.";
    state.releaseRemote();
  });
  await expect(page.getByRole("alert").filter({ hasText: /\S/ })).toContainText(
    "Cannot fast-forward",
  );
  await expect(fetch).toBeEnabled();
  await expect(page.getByText("Pull complete.", { exact: true })).toHaveCount(
    0,
  );
  await page.getByRole("tab", { name: /^Changes/ }).click();
  await expect(draft).toHaveValue("Keep this commit draft");
  await page
    .getByRole("button", { name: "Dismiss message", exact: true })
    .click();
  await page.evaluate(() => {
    const state = (window as any).__sourceControlTest;
    state.holdRemote = false;
    state.remoteError = "";
  });
  await fetch.click();
  await pull.click();
  await expect(remoteStatus).toHaveText("Pull complete.");
  await expect(draft).toHaveValue("Keep this commit draft");
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls.filter((call: any) =>
        ["git_fetch", "git_pull"].includes(call.command),
      ),
    ),
  ).toEqual([
    { command: "git_fetch", args: { root: "/project", remote: undefined } },
    { command: "git_pull", args: { root: "/project", rebase: false } },
    { command: "git_pull", args: { root: "/project", rebase: false } },
  ]);
});

test("pull reloads clean editors and preserves unsaved edits when files change", async ({
  page,
}) => {
  await openSourceControl(page, [changed("README.md")]);
  await page
    .getByRole("button", { name: "View diff for README.md", exact: true })
    .click({ button: "right" });
  await page.getByRole("menuitem", { name: "View File", exact: true }).click();
  const editor = page.locator(".cm-content");
  await expect(editor).toContainText("Project");
  await page.evaluate(() => {
    (window as any).__sourceControlTest.pulledContent = "First remote update";
  });
  await page.getByRole("button", { name: "Fetch", exact: true }).click();
  await page.getByRole("menuitem", { name: "Pull", exact: true }).click();
  await expect(editor).toHaveText("First remote update");
  await editor.fill("Keep my unsaved edits");
  await page.evaluate(() => {
    (window as any).__sourceControlTest.pulledContent = "Second remote update";
  });
  await page.getByRole("button", { name: "Fetch", exact: true }).click();
  await page.getByRole("menuitem", { name: "Pull", exact: true }).click();
  await expect(
    page.getByText("This file changed on disk. Your edits are preserved.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(editor).toHaveText("Keep my unsaved edits");
  await editor.press("Control+z");
  await expect(editor).toHaveText("First remote update");
});

test("remote menu supports keyboard navigation, remote selection and force-push confirmation", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await openSourceControl(page, []);
  const trigger = page.getByRole("button", { name: "Fetch", exact: true });
  const menu = page.getByRole("menu", {
    name: "Git remote actions",
    exact: true,
  });
  await trigger.press("ArrowDown");
  await expect(menu.getByRole("menuitem")).toHaveText([
    "Fetch",
    "Fetch From",
    "Pull",
    "Pull (Rebase)",
    "Push",
    "Push To",
    "Force Push",
  ]);
  await expect(
    menu.getByRole("menuitem", { name: "Fetch", exact: true }),
  ).toBeFocused();
  await menu.press("ArrowDown");
  await expect(
    menu.getByRole("menuitem", { name: "Fetch From", exact: true }),
  ).toBeFocused();
  await menu.press("Escape");
  await expect(trigger).toBeFocused();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
  await trigger.click();
  await trigger.click();
  await expect(menu).toHaveCount(0);
  for (const appearance of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: appearance });
    await trigger.click();
    const box = (await menu.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(800);
    expect(box.y + box.height).toBeLessThanOrEqual(420);
    await page.screenshot({
      path: testInfo.outputPath(`git-remote-menu-${appearance}.png`),
    });
    await page.locator(".source-heading").click();
    await expect(menu).toHaveCount(0);
  }
  const select = async (action: string) => {
    await trigger.click();
    await menu.getByRole("menuitem", { name: action, exact: true }).click();
  };
  await select("Force Push");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls.filter((call: any) =>
        ["git_fetch", "git_pull", "git_push"].includes(call.command),
      ),
    ),
  ).toEqual([]);
  await select("Fetch From");
  const from = page.getByRole("menu", { name: "Fetch From", exact: true });
  await expect(
    from.getByRole("menuitem", { name: "origin", exact: true }),
  ).toBeFocused();
  await from.getByRole("menuitem", { name: "upstream", exact: true }).click();
  await expect(page.locator(".git-remote-status")).toHaveText(
    "Fetch complete.",
  );
  await select("Pull (Rebase)");
  await expect(page.locator(".git-remote-status")).toHaveText(
    "Pull with rebase complete.",
  );
  await select("Push");
  await expect(page.locator(".git-remote-status")).toHaveText("Push complete.");
  await select("Push To");
  await page
    .getByRole("menu", { name: "Push To", exact: true })
    .getByRole("menuitem", { name: "upstream", exact: true })
    .click();
  await expect(page.locator(".git-remote-status")).toHaveText("Push complete.");
  await select("Force Push");
  await page
    .getByRole("dialog", { name: "Force Push", exact: true })
    .getByRole("button", { name: "Force Push", exact: true })
    .click();
  await expect(page.locator(".git-remote-status")).toHaveText("Push complete.");
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls.filter((call: any) =>
        ["git_fetch", "git_pull", "git_push"].includes(call.command),
      ),
    ),
  ).toEqual([
    { command: "git_fetch", args: { root: "/project", remote: "upstream" } },
    { command: "git_pull", args: { root: "/project", rebase: true } },
    {
      command: "git_push",
      args: { root: "/project", remote: undefined, force: false },
    },
    {
      command: "git_push",
      args: { root: "/project", remote: "upstream", force: false },
    },
    {
      command: "git_push",
      args: { root: "/project", remote: undefined, force: true },
    },
  ]);
});

test("file changes open full read-only tabs, preserve dirty editors and restore without starting shells", async ({
  page,
}, testInfo) => {
  await openSourceControl(page, [changed("README.md", "M", "M")]);
  const source = page.getByRole("button", {
    name: "View diff for README.md",
    exact: true,
  });
  await source.click();
  const tab = page.getByRole("tab", {
    name: "README.md · Changes",
    exact: true,
  });
  await expect(tab).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const diff = page.getByRole("region", {
    name: "Diff for README.md",
    exact: true,
  });
  await expect(diff.locator(".diff-line-context").first()).toContainText(
    "first line",
  );
  await expect(diff.locator(".diff-line-context").last()).toContainText(
    "last line",
  );
  await expect(diff.locator(".diff-line-addition")).toHaveCount(2);
  await expect(diff.locator(".diff-line-deletion")).toContainText(
    "index version",
  );
  await expect(
    diff
      .locator(".diff-line-addition")
      .first()
      .locator(".diff-line-number")
      .last(),
  ).toHaveText("2");
  await expect(diff.locator("[contenteditable=true], textarea")).toHaveCount(0);
  for (const mode of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: mode });
    await expect(page.locator("html")).toHaveAttribute("data-appearance", mode);
    await page
      .locator(".working-file-diff")
      .screenshot({ path: testInfo.outputPath(`file-diff-${mode}.png`) });
  }
  await source.click();
  await expect(tab).toHaveCount(1);
  await page.getByRole("button", { name: "Open file", exact: true }).click();
  await page.locator(".cm-content").fill("unsaved editor text");
  await source.click();
  await expect(diff).toContainText("working version");
  await page.getByRole("button", { name: "Open file", exact: true }).click();
  await expect(page.locator(".cm-content")).toHaveText("unsaved editor text");
  await source.click();
  await page
    .getByRole("button", {
      name: "View staged diff for README.md",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("tab", { name: "README.md · Staged changes", exact: true }),
  ).toBeVisible();
  await expect(diff).toContainText("HEAD version");
  await expect(diff).toContainText("staged version");
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
  await page
    .getByRole("button", { name: "Close README.md", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Discard changes", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        JSON.parse(
          localStorage.getItem("test-session")!,
        ).projects[0].workspaces[0].tabs.map((tab: any) => tab.type),
      ),
    )
    .toEqual(["terminal", "diff", "diff"]);
  await page.reload();
  await expect(
    page.getByRole("tab", { name: "README.md · Staged changes", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("region", { name: "Diff for README.md", exact: true }),
  ).toContainText("new");
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(0);
});

test("file diff errors retry in place and stale responses cannot replace another file", async ({
  page,
}, testInfo) => {
  await openSourceControl(page, [changed("README.md"), changed("other.md")]);
  await page.evaluate(() => {
    (window as any).__sourceControlTest.failDiff = true;
  });
  await page
    .getByRole("button", { name: "View diff for README.md", exact: true })
    .click();
  await expect(page.locator(".working-file-diff [role=alert]")).toContainText(
    "Unable to read",
  );
  await page.evaluate(() => {
    (window as any).__sourceControlTest.failDiff = false;
  });
  await page.getByRole("button", { name: "Retry diff", exact: true }).click();
  await expect(page.locator(".working-file-diff")).toContainText("last line");
  await page.evaluate(() => {
    const state = (window as any).__sourceControlTest;
    state.diffDelays["README.md"] = 600;
    state.diffs["README.md"] = {
      patch: "@@ -1 +1 @@\n-old\n+stale response\n",
      truncated: false,
      notice: null,
    };
    state.diffs["other.md"] = {
      patch: "@@ -1 +1 @@\n-old\n+new file\n",
      truncated: true,
      notice: null,
    };
  });
  await page
    .getByRole("button", { name: "Refresh file changes", exact: true })
    .click();
  await page
    .getByRole("button", { name: "View diff for other.md", exact: true })
    .click();
  await expect(page.locator(".working-file-diff")).toContainText("new file");
  await page.waitForTimeout(700);
  await expect(page.locator(".working-file-diff")).not.toContainText(
    "stale response",
  );
  await expect(page.locator(".working-file-diff")).toContainText(
    "preview limit",
  );
  await page.setViewportSize({ width: 800, height: 420 });
  await page.screenshot({ path: testInfo.outputPath("file-diff-minimum.png") });
  await expect(
    page.getByRole("button", { name: "Refresh file changes", exact: true }),
  ).toBeInViewport();
});

test("file menus stage, unstage renames, open both diffs and copy exact paths", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await openSourceControl(page, [
    changed("README.md", "M", "M"),
    changed("renamed [1].md", "R", " ", "original.md"),
  ]);
  const file = page.getByRole("button", {
    name: "View diff for README.md",
    exact: true,
  });
  const menu = page.getByRole("menu", {
    name: "Source control actions for README.md",
    exact: true,
  });
  await file.click({ button: "right" });
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    menu.getByRole("menuitem", { name: "Stage File", exact: true }),
  ).toBeFocused();
  await expect(
    menu.getByRole("menuitem", { name: "Add to .gitignore", exact: true }),
  ).toBeDisabled();
  await page.screenshot({
    path: testInfo.outputPath("source-control-menu-dark.png"),
  });
  await menu.press("Escape");
  await expect(file).toBeFocused();
  await file.press("Shift+F10");
  await menu
    .getByRole("menuitem", { name: "Staged Changes", exact: true })
    .click();
  await expect(
    page.getByRole("tab", { name: "README.md · Staged changes", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".working-file-diff")).toContainText(
    "staged version",
  );
  await page
    .getByRole("button", {
      name: "Close README.md · Staged changes",
      exact: true,
    })
    .click();
  await file.click({ button: "right" });
  await menu
    .getByRole("menuitem", { name: "Unstaged Changes", exact: true })
    .click();
  await expect(
    page.getByRole("tab", { name: "README.md · Changes", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".working-file-diff")).toContainText(
    "working version",
  );
  await page
    .getByRole("button", { name: "Close README.md · Changes", exact: true })
    .click();
  for (const label of ["Copy Path", "Copy Relative Path"]) {
    await file.click({ button: "right" });
    await menu.getByRole("menuitem", { name: label, exact: true }).click();
  }
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls
        .filter(
          (call: any) => call.command === "plugin:clipboard-manager|write_text",
        )
        .map((call: any) => call.args.text),
    ),
  ).toEqual(["/project/README.md", "README.md"]);
  await file.click({ button: "right" });
  await menu.getByRole("menuitem", { name: "Stage File", exact: true }).click();
  await expect(file).toHaveCount(0);
  const renamed = page.getByRole("button", {
    name: "View staged diff for renamed [1].md",
    exact: true,
  });
  await renamed.click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "Unstage File", exact: true })
    .click();
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls
        .filter((call: any) => call.command === "git_stage")
        .map((call: any) => call.args),
    ),
  ).toEqual([
    { root: "/project", paths: ["README.md"], stage: true },
    {
      root: "/project",
      paths: ["renamed [1].md", "original.md"],
      stage: false,
    },
  ]);
});

test("file menus open files and history, ignore only new files, and fit the minimum window", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await page.emulateMedia({ colorScheme: "light" });
  await openSourceControl(page, [
    changed("README.md"),
    changed("new [1].txt", "?", "?"),
    changed("deleted.md", " ", "D"),
  ]);
  const file = page.getByRole("button", {
    name: "View diff for README.md",
    exact: true,
  });
  await file.click({ button: "right" });
  const menu = page.getByRole("menu");
  const box = (await menu.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(800);
  expect(box.y + box.height).toBeLessThanOrEqual(420);
  await page.screenshot({
    path: testInfo.outputPath("source-control-menu-light-minimum.png"),
  });
  await menu.getByRole("menuitem", { name: "View File", exact: true }).click();
  await expect(page.locator(".cm-content")).toBeVisible();
  await file.click({ button: "right" });
  await menu
    .getByRole("menuitem", { name: "View File History", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText("No commits yet.");
  expect(
    await page.evaluate(
      () =>
        (window as any).__sourceControlTest.calls.find(
          (call: any) => call.command === "git_history",
        ).args,
    ),
  ).toMatchObject({ root: "/project", path: "README.md" });
  await page.getByRole("button", { name: "Close dialog", exact: true }).click();
  for (const label of ["Add to .gitignore", "Add to .git/info/exclude"]) {
    await page
      .getByRole("button", { name: "View diff for new [1].txt", exact: true })
      .click({ button: "right" });
    await menu.getByRole("menuitem", { name: label, exact: true }).click();
  }
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls
        .filter((call: any) => call.command === "ignore_project_item")
        .map((call: any) => call.args),
    ),
  ).toEqual([
    { root: "/project", relative: "new [1].txt", local: false },
    { root: "/project", relative: "new [1].txt", local: true },
  ]);
  await page
    .getByRole("button", { name: "View diff for deleted.md", exact: true })
    .click({ button: "right" });
  await expect(
    menu.getByRole("menuitem", { name: "View File", exact: true }),
  ).toBeDisabled();
  await page.getByRole("tab", { name: "README.md", exact: true }).click();
  await expect(menu).toHaveCount(0);
});

test("discard requires confirmation, retains failed changes and protects dirty buffers", async ({
  page,
}) => {
  await openSourceControl(page, [changed("README.md"), changed("other.md")]);
  const file = page.getByRole("button", {
    name: "View diff for README.md",
    exact: true,
  });
  const ask = async () => {
    await file.click({ button: "right" });
    await page
      .getByRole("menuitem", { name: "Discard Changes…", exact: true })
      .click();
  };
  await ask();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls.filter(
        (call: any) => call.command === "git_discard",
      ),
    ),
  ).toEqual([]);
  await page.evaluate(() => {
    (window as any).__sourceControlTest.failDiscard = true;
  });
  await ask();
  await page
    .getByRole("button", { name: "Discard Changes", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText(
    "Cannot restore the file",
  );
  await expect(file).toBeVisible();
  await page.evaluate(() => {
    (window as any).__sourceControlTest.failDiscard = false;
  });
  await page
    .getByRole("button", { name: "Discard Changes", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(file).toHaveCount(0);
  const other = page.getByRole("button", {
    name: "View diff for other.md",
    exact: true,
  });
  await other.click({ button: "right" });
  await page.getByRole("menuitem", { name: "View File", exact: true }).click();
  const editor = page.locator(".cm-content");
  await editor.fill("Keep these unsaved edits");
  await other.click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "Discard Changes…", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Discard Changes", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText(
    "Save or discard unsaved editor changes",
  );
  await expect(editor).toContainText("Keep these unsaved edits");
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls
        .filter((call: any) => call.command === "git_discard")
        .map((call: any) => call.args.change.path),
    ),
  ).toEqual(["README.md", "README.md"]);
});

test("source control groups changes and keeps staged and working diffs separate", async ({
  page,
}) => {
  await openSourceControl(page, [
    changed("src/App.tsx", "M", "M"),
    changed("README.md"),
    changed("src/new file.ts", "?", "?"),
    changed("src/renamed.ts", "R", " ", "src/original.ts"),
  ]);
  const tracked = page.getByRole("region", {
    name: "Tracked changes",
    exact: true,
  });
  const untracked = page.getByRole("region", {
    name: "Untracked changes",
    exact: true,
  });
  const staged = page.getByRole("region", {
    name: "Staged changes",
    exact: true,
  });
  await expect(tracked.getByRole("listitem")).toHaveCount(2);
  await expect(untracked.getByRole("listitem")).toHaveCount(1);
  await expect(staged.getByRole("listitem")).toHaveCount(2);

  await tracked.getByRole("button", { name: "Tracked 2", exact: true }).click();
  await expect(tracked.getByRole("list")).toBeHidden();
  await tracked
    .getByRole("button", { name: "Tracked 2", exact: true })
    .press("Enter");
  await tracked
    .getByRole("button", { name: "View diff for src/App.tsx", exact: true })
    .click();
  await expect(page.locator(".working-file-diff")).toContainText(
    "working version",
  );
  await page
    .getByRole("button", { name: "Close App.tsx · Changes", exact: true })
    .click();
  await staged
    .getByRole("button", {
      name: "View staged diff for src/App.tsx",
      exact: true,
    })
    .click();
  await expect(page.locator(".working-file-diff")).toContainText(
    "staged version",
  );
  await page
    .getByRole("button", {
      name: "Close App.tsx · Staged changes",
      exact: true,
    })
    .click();
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls
        .filter((call: any) => call.command === "git_diff")
        .map((call: any) => call.args)
        .filter(
          (args: any, index: number, calls: any[]) =>
            index === 0 ||
            JSON.stringify(args) !== JSON.stringify(calls[index - 1]),
        ),
    ),
  ).toEqual([
    { root: "/project", path: "src/App.tsx", staged: false },
    { root: "/project", path: "src/App.tsx", staged: true },
  ]);
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls.filter((call: any) =>
        ["git_stage", "git_commit"].includes(call.command),
      ),
    ),
  ).toEqual([]);

  await page
    .getByRole("checkbox", { name: "Stage README.md", exact: true })
    .press("Space");
  await expect(
    page.getByRole("checkbox", { name: "Unstage README.md", exact: true }),
  ).toBeChecked();
  await page
    .getByRole("checkbox", { name: "Stage untracked changes", exact: true })
    .click();
  await expect(
    page.getByRole("checkbox", {
      name: "Unstage src/new file.ts",
      exact: true,
    }),
  ).toBeChecked();
  await expect(untracked).toHaveCount(0);
  await page
    .getByRole("checkbox", { name: "Unstage src/renamed.ts", exact: true })
    .click();
  await expect(
    page.getByRole("checkbox", { name: "Stage src/renamed.ts", exact: true }),
  ).not.toBeChecked();
  await page
    .getByRole("button", { name: "Stage all changes", exact: true })
    .click();
  await expect(tracked).toHaveCount(0);
  await page
    .getByRole("button", { name: "Unstage all changes", exact: true })
    .click();
  await expect(staged).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls
        .filter((call: any) => call.command === "git_stage")
        .map((call: any) => call.args),
    ),
  ).toEqual([
    { root: "/project", paths: ["README.md"], stage: true },
    { root: "/project", paths: ["src/new file.ts"], stage: true },
    {
      root: "/project",
      paths: ["src/renamed.ts", "src/original.ts"],
      stage: false,
    },
    { root: "/project", paths: ["src/App.tsx", "src/renamed.ts"], stage: true },
    {
      root: "/project",
      paths: [
        "src/App.tsx",
        "README.md",
        "src/new file.ts",
        "src/renamed.ts",
        "src/original.ts",
      ],
      stage: false,
    },
  ]);
});

test("source control preserves an exact commit draft after failed Git actions", async ({
  page,
}) => {
  await openSourceControl(page, [changed("README.md")]);
  const input = page.getByRole("textbox", {
    name: "Commit message",
    exact: true,
  });
  const commit = page.getByRole("button", {
    name: "Commit staged changes",
    exact: true,
  });
  const draft =
    "docs(readme): describe usage\n\nKeep spacing, Unicode: zażółć 🦀\n\nValidation:\n- Manual review\n";
  await input.fill(draft);
  await expect(commit).toBeDisabled();
  await page.evaluate(() => {
    (window as any).__sourceControlTest.failStage = true;
  });
  await page
    .getByRole("checkbox", { name: "Stage README.md", exact: true })
    .click();
  await expect(page.getByRole("alert").filter({ hasText: /\S/ })).toContainText(
    "Cannot update the index",
  );
  await expect(
    page.getByRole("checkbox", { name: "Stage README.md", exact: true }),
  ).not.toBeChecked();
  await expect(input).toHaveValue(draft);
  await page.evaluate(() => {
    (window as any).__sourceControlTest.failStage = false;
    (window as any).__sourceControlTest.failCommit = true;
  });
  await page
    .getByRole("checkbox", { name: "Stage README.md", exact: true })
    .click();
  await commit.click();
  await expect(page.getByRole("alert").filter({ hasText: /\S/ })).toContainText(
    "Commit hook rejected the message",
  );
  await expect(input).toHaveValue(draft);
  await expect(commit).toBeEnabled();
  await page.evaluate(() => {
    (window as any).__sourceControlTest.failCommit = false;
  });
  await page
    .getByRole("button", { name: "Dismiss message", exact: true })
    .click();
  await commit.click();
  await expect(
    page.getByText("Working tree clean.", { exact: true }),
  ).toBeVisible();
  await expect(input).toHaveValue("");
  await expect(commit).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Stage all changes", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(() =>
      (window as any).__sourceControlTest.calls
        .filter((call: any) => call.command === "git_commit")
        .map((call: any) => call.args.message),
    ),
  ).toEqual([draft, draft]);
});

test("source control keeps its commit form visible while a long list scrolls at minimum size", async ({
  page,
}, testInfo) => {
  await openSourceControl(page, [
    ...[
      "src/App.tsx",
      "src/SplitView.tsx",
      "src/TerminalPane.tsx",
      "src/terminal-runtime.ts",
      "src-tauri/capabilities/settings.json",
      "src-tauri/src/files.rs",
      "src-tauri/src/lib.rs",
    ].map((path) => changed(path)),
    ...[
      "src/KeybindingsProvider.tsx",
      "src/SettingsWindow.tsx",
      "src/TabBar.tsx",
      "src/keybindings.ts",
      "src-tauri/src/keybindings.rs",
    ].map((path) => changed(path, "?", "?")),
  ]);
  await page
    .locator(".source-panel")
    .screenshot({ path: testInfo.outputPath("source-control-default.png") });
  const divider = page.getByRole("separator", {
    name: "Resize sidebar",
    exact: true,
  });
  const resize = async (key: "ArrowLeft" | "ArrowRight") => {
    const width = Number(await divider.getAttribute("aria-valuenow"));
    await divider.press(key);
    await expect(divider).toHaveAttribute(
      "aria-valuenow",
      String(Math.max(180, width + (key === "ArrowLeft" ? -20 : 20))),
    );
  };
  await divider.focus();
  for (let count = 0; count < 4; count++) await resize("ArrowRight");
  await page
    .locator(".source-panel")
    .screenshot({ path: testInfo.outputPath("source-control-wide.png") });

  await page.evaluate(() => {
    const state = (window as any).__sourceControlTest;
    state.changes.push(
      ...Array.from({ length: 40 }, (_, index) => ({
        path: `src/deeply/nested/directory/component-${index}.tsx`,
        originalPath: null,
        index: " ",
        worktree: "M",
      })),
    );
    window.dispatchEvent(new Event("focus"));
  });
  await expect(page.getByRole("tab", { name: /^Changes/ })).toHaveText(
    "Changes (52)",
  );
  await page.setViewportSize({ width: 800, height: 420 });
  await divider.focus();
  for (let count = 0; count < 8; count++) await resize("ArrowLeft");
  await expect(divider).toHaveAttribute("aria-valuenow", "180");
  await expect(
    page.getByRole("button", { name: "Fetch", exact: true }),
  ).toBeInViewport();
  await page.getByRole("button", { name: "Fetch", exact: true }).click();
  await page.getByRole("menuitem", { name: "Fetch", exact: true }).click();
  await expect(page.locator(".git-remote-status")).toHaveText(
    "Fetch complete.",
  );
  const form = page.locator(".commit-form");
  const before = (await form.boundingBox())!;
  const list = page.locator(".git-groups");
  await list.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await expect(
    page.getByRole("checkbox", {
      name: "Stage src-tauri/src/keybindings.rs",
      exact: true,
    }),
  ).toBeInViewport();
  const after = (await form.boundingBox())!;
  expect(after.y).toBe(before.y);
  await expect(
    page.getByRole("textbox", { name: "Commit message", exact: true }),
  ).toBeInViewport();
  await expect(
    page.getByRole("button", { name: "Commit staged changes", exact: true }),
  ).toBeInViewport();
  expect(
    await page
      .locator(".source-panel")
      .evaluate(
        (panel) =>
          panel.scrollWidth <= panel.clientWidth &&
          panel.scrollHeight <= panel.clientHeight,
      ),
  ).toBe(true);
  await page
    .getByRole("textbox", { name: "Commit message", exact: true })
    .fill("fix(ui): keep controls reachable");
  await page
    .getByRole("checkbox", {
      name: "Stage src-tauri/src/keybindings.rs",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("button", { name: "Commit staged changes", exact: true }),
  ).toBeEnabled();
  await page.screenshot({
    path: testInfo.outputPath("source-control-minimum.png"),
  });
});

test("All changes hides clean repositories and remembers independently collapsed groups", async ({
  page,
}, testInfo) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    const repositories = [
      { root: "/project/brandbook", branch: "main", changes: [] },
      {
        root: "/project/lomi",
        branch: "main",
        changes: [
          {
            path: "src/SourceControl.tsx",
            originalPath: null,
            index: " ",
            worktree: "M",
          },
        ],
      },
      {
        root: "/project/plugin-sdk",
        branch: "feature",
        changes: [
          {
            path: "src/index.ts",
            originalPath: null,
            index: "M",
            worktree: "M",
          },
        ],
      },
    ];
    desktop.__allChanges = repositories;
    desktop.__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: any = {},
    ) => {
      if (command === "git_repositories")
        return { repositories, errors: [], limited: true };
      return invoke(command, args);
    };
  });
  await page.goto("/");
  const toggle = page.getByRole("button", {
    name: "Toggle source control (Ctrl+Shift+G)",
  });
  await toggle.click();
  const navigation = page.getByRole("navigation", { name: "Repositories" });
  const list = page.locator(".source-all-list");
  const lomi = list.getByRole("region", { name: "lomi", exact: true });
  const sdk = list.getByRole("region", { name: "plugin-sdk", exact: true });
  await expect(
    navigation.getByRole("button", { name: "brandbook 0", exact: true }),
  ).toBeVisible();
  await expect(list.getByRole("region")).toHaveCount(2);
  await expect(
    list.getByText("Working tree clean", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText("Repository scan incomplete", { exact: true }),
  ).toHaveCount(0);
  const scan = page.getByRole("button", {
    name: "Repository scan details",
    exact: true,
  });
  // WebKit does not focus buttons on pointer activation. Exercise restoration
  // from a keyboard trigger, which has focus before the native dialog opens.
  await scan.focus();
  await scan.press("Enter");
  await expect(
    page.getByRole("dialog", { name: "Repository scan", exact: true }),
  ).toContainText("Only part of this folder was scanned");
  await page.keyboard.press("Escape");
  await expect(scan).toBeFocused();
  const lomiHeader = lomi.locator(".source-all-repository-heading");
  const sdkHeader = sdk.locator(".source-all-repository-heading");
  await expect(lomiHeader).toHaveAttribute("aria-expanded", "true");
  await expect(sdk.locator(".source-all-file")).toHaveCount(2);
  await lomiHeader.click();
  await expect(lomiHeader).toHaveAttribute("aria-expanded", "false");
  await expect(lomi.locator(".source-all-file")).toBeHidden();
  await expect(sdk.locator(".source-all-file").first()).toBeVisible();
  await navigation.getByRole("button", { name: "lomi 1", exact: true }).click();
  await navigation
    .getByRole("button", { name: "All repositories 3", exact: true })
    .click();
  await expect(lomiHeader).toHaveAttribute("aria-expanded", "false");
  await toggle.click();
  await toggle.click();
  await expect(lomiHeader).toHaveAttribute("aria-expanded", "false");
  await lomiHeader.press("Space");
  await expect(lomiHeader).toHaveAttribute("aria-expanded", "true");
  await expect(lomi.locator(".source-all-file")).toBeVisible();
  await sdkHeader.press("Enter");
  await expect(sdkHeader).toHaveAttribute("aria-expanded", "false");
  await expect(sdk.locator(".source-all-file").first()).toBeHidden();
  await page.setViewportSize({ width: 800, height: 420 });
  for (const mode of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: mode });
    await page
      .locator(".source-panel")
      .screenshot({ path: testInfo.outputPath(`all-changes-${mode}.png`) });
  }
  await page.evaluate(() => {
    for (const repository of (window as any).__allChanges)
      repository.changes = [];
    window.dispatchEvent(new Event("focus"));
  });
  await expect(list.getByRole("region")).toHaveCount(0);
  await expect(list.getByText("No changes.", { exact: true })).toBeVisible();
  await expect(
    navigation.getByRole("button", { name: "lomi 0", exact: true }),
  ).toBeVisible();
});
