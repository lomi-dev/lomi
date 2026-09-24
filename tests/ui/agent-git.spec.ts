import { expect, test } from "@playwright/test";
import {
  newProject,
  newSession,
  openDiffTab,
  openFileTab,
  openCommitTab,
  updateTab,
} from "../../src/model";
import { mockDesktop } from "./desktop";

for (const kind of ["diff", "commit"] as const) {
  test(`restored agent ${kind} view denies ordinary Git reads and renders scoped content`, async ({
    page,
  }) => {
    const project = newProject("/project", "default");
    let session = {
      ...newSession(),
      projects: [project],
      activeProjectId: project.id,
    };
    const workspace = project.workspaces[0].id;
    const commit = "a".repeat(40);
    session =
      kind === "diff"
        ? openDiffTab(session, workspace, "/project", "a.txt", false)
        : openCommitTab(
            session,
            workspace,
            "/project",
            commit,
            "Fixture commit",
          );
    const tab = session.projects[0].workspaces[0].tabs.find(
      (t) => t.type === kind,
    )!;
    session = updateTab(session, tab.id, (t) =>
      t.type === "diff" || t.type === "commit" ? { ...t, agentGit: true } : t,
    );
    await mockDesktop(page, false, session);
    await page.goto("/");
    await expect(page.getByRole("alert")).toContainText(
      "This agent Git view is no longer available",
    );
    await page.evaluate(
      async ({ id, kind, commit }) => {
        const path = "/src/agent-git.ts";
        const module = await import(path);
        const native = (window as any).__TAURI_INTERNALS__;
        const invoke = native.invoke.bind(native);
        native.invoke = (command: string, args: any) => {
          if (command === "agent_control_git_read") {
            (window as any).__nativeTest.calls.push({ command, args });
            if (args.relative !== "a.txt")
              return Promise.reject("SCOPE_DENIED");
            return Promise.resolve({
              kind: "diff",
              patch: "@@ -1 +1 @@\n-before\n+Scoped commit change 🙂\n",
              notice: null,
            });
          }
          return invoke(command, args);
        };
        module
          .stageAgentGit(id, {
            root: "/project",
            permitId: "permit",
            observationRevision: "b".repeat(64),
            body:
              kind === "diff"
                ? {
                    kind,
                    patch: "@@ -1 +1 @@\n-before\n+Scoped working change 🙂\n",
                    notice: null,
                  }
                : {
                    kind,
                    commit,
                    author: "Fixture",
                    authorEmail: "fixture@example.invalid",
                    authoredAt: "2026-09-23T10:00:00Z",
                    committer: "Fixture",
                    committerEmail: "fixture@example.invalid",
                    committedAt: "2026-09-23T10:00:00Z",
                    parents: [],
                    message: "Scoped commit 🙂\n\nExact body\n",
                    omittedEntries: 2,
                    files: [{ relativePath: "a.txt", status: "A" }],
                  },
          })
          .commit();
      },
      { id: tab.id, kind, commit },
    );
    if (kind === "diff") {
      await expect(
        page.getByRole("region", { name: "Diff for a.txt" }),
      ).toContainText("Scoped working change 🙂");
    } else {
      await expect(
        page.getByRole("heading", { name: "Scoped commit 🙂" }),
      ).toBeVisible();
      await expect(
        page
          .getByRole("status")
          .filter({ hasText: "2 secret or unsupported file entries omitted" }),
      ).toBeVisible();
      await expect(
        page.getByText("Line statistics are unavailable for this view."),
      ).toBeVisible();
      await expect(
        page.getByRole("region", { name: "File changes" }),
      ).toContainText("Scoped commit change 🙂");
      expect(
        await page.evaluate(
          () =>
            (window as any).__nativeTest.calls.filter(
              (c: any) => c.command === "agent_control_git_read",
            ).length,
        ),
      ).toBe(1);
    }
    const ordinary = await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter((c: any) =>
        ["git_diff", "git_commit_details", "git_commit_diff"].includes(
          c.command,
        ),
      ),
    );
    expect(ordinary).toEqual([]);
    await page.screenshot({ path: `test-results/mcp-agent-git-${kind}.png` });
  });
}

test("Git mutation approval preserves exact targets, defaults to cancel and dismisses on revoke", async ({
  page,
}) => {
  const project = newProject("/project", "default");
  let session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  session = openFileTab(
    session,
    project.workspaces[0].id,
    "/project",
    "Unicode 🙂.txt",
  );
  await mockDesktop(page, false, session);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__gitApproval = {
      projection: null,
      operation: "stage",
      pending: true,
      decisions: [],
      commits: [],
      acknowledgements: [],
    };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      const state = desktop.__gitApproval;
      if (command === "agent_control_ui_register") return "git-epoch";
      if (command === "agent_control_ui_publish") {
        state.projection = args.projection;
        return;
      }
      if (command === "agent_control_ui_claim") return;
      if (command === "agent_control_git_mutation_prepare")
        return {
          planHash: "a".repeat(64),
          repositoryPath: "/project",
          operation: state.operation,
          pull:
            state.operation === "pull"
              ? {
                  target: {
                    remote: "origin",
                    reference: "refs/heads/main",
                    sourceCommit: "b".repeat(40),
                    expectedRemoteCommit: "f".repeat(40),
                    mode: "rebase",
                  },
                  network: {
                    location: "https://example.invalid/team/repo.git",
                    destination: "refs/remotes/origin/main",
                  },
                  replayCommits: ["b".repeat(40)],
                  affectedPaths: ["Unicode 🙂.txt"],
                }
              : null,
          discard:
            state.operation === "discard"
              ? [
                  {
                    relativePath: "Unicode 🙂.txt",
                    indexObject: "e".repeat(40),
                    indexMode: "100644",
                    patch: "@@ -1 +1 @@\n-staged\n+working to discard 🙂\n",
                  },
                ]
              : null,
          push:
            state.operation === "push"
              ? {
                  location: "https://example.invalid/team/repo.git",
                  target: {
                    remote: "origin",
                    reference: "refs/heads/main",
                    sourceCommit: "f".repeat(40),
                    expectedRemoteCommit: "b".repeat(40),
                  },
                }
              : null,
          network:
            state.operation === "fetch"
              ? {
                  remote: "origin",
                  reference: "refs/heads/main",
                  destination: "refs/remotes/origin/main",
                  location: "https://example.invalid/team/repo.git",
                  previousCommit: null,
                }
              : null,
          commit:
            state.operation === "commit"
              ? {
                  message: "  Exact commit 🙂  \n\nBody  \n",
                  finalNewlineAdded: false,
                  author: {
                    name: "Fixture Identity",
                    email: "fixture@example.invalid",
                  },
                  committer: {
                    name: "Fixture Identity",
                    email: "fixture@example.invalid",
                  },
                  changes: [
                    {
                      relativePath: "Unicode 🙂.txt",
                      status: "M",
                      oldObject: "d".repeat(40),
                      newObject: "e".repeat(40),
                    },
                  ],
                }
              : null,
          branch: "refs/heads/fixture",
          head: "b".repeat(40),
          files: [
            {
              relativePath: "Unicode 🙂.txt",
              sha256: "c".repeat(64),
              byteLength: 24,
            },
          ],
        };
      if (command === "agent_control_git_mutation_pending")
        return state.pending;
      if (command === "agent_control_git_mutation_decide") {
        state.decisions.push(args);
        state.pending = false;
        return;
      }
      if (command === "agent_control_git_mutation_commit") {
        state.commits.push(args);
        return {
          workspaceId: state.projection.workspaces[0].id,
          repositoryRelative: "",
          operation: "stage",
          planHash: args.planHash,
        };
      }
      if (command === "agent_control_ui_ack") {
        state.acknowledgements.push(args.ack);
        return;
      }
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await expect
    .poll(() =>
      page.evaluate(() => Boolean((window as any).__gitApproval.projection)),
    )
    .toBe(true);
  const request = async (operationId: string) =>
    page.evaluate(async (operationId) => {
      const desktop = window as any;
      const state = desktop.__gitApproval;
      state.pending = true;
      const projection = state.projection;
      await desktop.__nativeTest.emitEvent("agent-control-command", {
        operationId,
        nonce: `${operationId}-nonce`,
        uiEpoch: "git-epoch",
        domainRevision: projection.revision,
        projectId: projection.workspaces[0].projectId,
        action: {
          type: "git_mutate",
          workspaceId: projection.workspaces[0].id,
          notAfterMillis: String(Date.now() + 120000),
        },
      });
    }, operationId);
  await request("cancelled");
  const dialog = page.getByRole("dialog", {
    name: "Stage files for this agent?",
  });
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  await expect(dialog).toContainText("Unsaved editor changes are not included");
  await expect(dialog).toContainText("Unicode 🙂.txt");
  await expect(dialog).toContainText("c".repeat(64));
  await page.screenshot({ path: "test-results/mcp-git-mutation-approval.png" });
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits),
  ).toEqual([]);
  await request("revoked");
  await expect(dialog).toBeVisible();
  await page.evaluate(() => {
    (window as any).__gitApproval.pending = false;
  });
  await expect(dialog).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits),
  ).toEqual([]);
  await request("approved");
  await expect(dialog).toBeVisible();
  await dialog
    .getByRole("button", { name: "Stage these files", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__gitApproval.acknowledgements.length,
      ),
    )
    .toBe(1);
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits),
  ).toEqual([
    {
      operationId: "approved",
      nonce: "approved-nonce",
      planHash: "a".repeat(64),
    },
  ]);
  expect(
    await page.evaluate(() =>
      (window as any).__gitApproval.decisions.filter((v: any) => v.approved),
    ),
  ).toEqual([
    {
      operationId: "approved",
      nonce: "approved-nonce",
      planHash: "a".repeat(64),
      approved: true,
    },
  ]);
  await page.evaluate(() => {
    (window as any).__gitApproval.operation = "commit";
  });
  await request("commit-cancelled");
  const commit = page.getByRole("dialog", {
    name: "Create this commit for the agent?",
  });
  await expect(commit).toBeVisible();
  await expect(
    commit.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  await expect(commit.getByLabel("Commit message")).toHaveValue(
    "  Exact commit 🙂  \n\nBody  \n",
  );
  await expect(commit).toContainText(
    "Fixture Identity <fixture@example.invalid>",
  );
  await expect(commit.getByLabel("Staged changes")).toContainText(
    "e".repeat(40),
  );
  await expect(commit.getByLabel("Staged changes")).not.toContainText(
    "c".repeat(64),
  );
  await page.setViewportSize({ width: 900, height: 600 });
  await expect(
    commit.getByRole("button", { name: "Create this commit", exact: true }),
  ).toBeInViewport();
  await page.screenshot({ path: "test-results/mcp-git-commit-approval.png" });
  await page.keyboard.press("Escape");
  await expect(commit).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits.length),
  ).toBe(1);
  await request("commit-approved");
  await expect(commit).toBeVisible();
  await commit
    .getByRole("button", { name: "Create this commit", exact: true })
    .click();
  await expect(commit).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__gitApproval.commits.length),
    )
    .toBe(2);
  await page.evaluate(() => {
    (window as any).__gitApproval.operation = "fetch";
  });
  await request("fetch-cancelled");
  const fetch = page.getByRole("dialog", {
    name: "Fetch this branch for the agent?",
  });
  await expect(fetch).toBeVisible();
  await expect(fetch).toContainText("https://example.invalid/team/repo.git");
  await expect(fetch).toContainText("refs/remotes/origin/main");
  await expect(fetch).not.toContainText("Unicode 🙂.txt");
  await page.screenshot({ path: "test-results/mcp-git-fetch-approval.png" });
  await page.keyboard.press("Escape");
  await expect(fetch).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits.length),
  ).toBe(2);
  await request("fetch-approved");
  await expect(fetch).toBeVisible();
  await fetch
    .getByRole("button", { name: "Fetch this branch", exact: true })
    .click();
  await expect(fetch).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__gitApproval.commits.length),
    )
    .toBe(3);
  await page.evaluate(() => {
    (window as any).__gitApproval.operation = "push";
  });
  await request("push-cancelled");
  const push = page.getByRole("dialog", {
    name: "Push this commit for the agent?",
  });
  await expect(push).toBeVisible();
  await expect(push).toContainText("f".repeat(40));
  await expect(push).toContainText("b".repeat(40));
  await expect(push).toContainText("refs/heads/main");
  await page.screenshot({ path: "test-results/mcp-git-push-approval.png" });
  await page.keyboard.press("Escape");
  await expect(push).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits.length),
  ).toBe(3);
  await request("push-approved");
  await expect(push).toBeVisible();
  await push
    .getByRole("button", { name: "Push this commit", exact: true })
    .click();
  await expect(push).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__gitApproval.commits.length),
    )
    .toBe(4);
  await page.evaluate(() => {
    (window as any).__gitApproval.operation = "discard";
  });
  await request("discard-cancelled");
  const discard = page.getByRole("dialog", {
    name: "Discard working changes for the agent?",
  });
  await expect(discard).toBeVisible();
  await expect(
    discard.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  await expect(discard).toContainText("not saved in Git or Trash");
  await expect(
    discard.getByLabel("Working diff for Unicode 🙂.txt"),
  ).toHaveValue("@@ -1 +1 @@\n-staged\n+working to discard 🙂\n");
  await page.screenshot({ path: "test-results/mcp-git-discard-approval.png" });
  await page.keyboard.press("Escape");
  await expect(discard).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits.length),
  ).toBe(4);
  await request("discard-approved");
  await discard
    .getByRole("button", { name: "Discard working changes", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__gitApproval.commits.length),
    )
    .toBe(5);

  await expect(discard).toHaveCount(0);
  await page.evaluate(() => {
    (window as any).__gitApproval.operation = "pull";
  });
  await request("pull-cancelled");
  const pull = page.getByRole("dialog", {
    name: "Pull this branch for the agent?",
  });
  await expect(pull).toBeVisible();
  await expect(pull).toContainText("A conflict stays in the repository");
  await expect(pull.getByLabel("Commits to rebase")).toContainText(
    "b".repeat(40),
  );
  await expect(pull).toContainText("f".repeat(40));
  await page.screenshot({ path: "test-results/mcp-git-pull-approval.png" });
  await page.keyboard.press("Escape");
  await expect(pull).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits.length),
  ).toBe(5);
  await request("pull-approved");
  await pull
    .getByRole("button", { name: "Pull this branch", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__gitApproval.commits.length),
    )
    .toBe(6);
  await expect(pull).toHaveCount(0);
  await page.evaluate(() => {
    (window as any).__gitApproval.operation = "discard";
  });
  const editor = page.locator(".cm-content");
  await editor.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type("dirty shared buffer");
  await request("discard-dirty");
  await discard
    .getByRole("button", { name: "Discard working changes", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__gitApproval.acknowledgements.at(-1)?.result?.code,
      ),
    )
    .toBe("TARGET_BUSY");
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits.length),
  ).toBe(6);
  await expect(editor).toContainText("dirty shared buffer");
  await page.evaluate(() => {
    (window as any).__gitApproval.operation = "pull";
  });
  await request("pull-dirty");
  await pull
    .getByRole("button", { name: "Pull this branch", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__gitApproval.acknowledgements.at(-1)?.result?.code,
      ),
    )
    .toBe("TARGET_BUSY");
  expect(
    await page.evaluate(() => (window as any).__gitApproval.commits.length),
  ).toBe(6);
  await expect(editor).toContainText("dirty shared buffer");
});
