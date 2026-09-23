import { expect, test } from "@playwright/test";
import {
  newProject,
  newSession,
  newWorkspace,
  openFileTab,
} from "../../src/model";
import { mockDesktop } from "./desktop";

for (const ancestor of ["workspace", "project"] as const)
  test(`${ancestor} close preserves cancellation and failed saves, then requires a fresh request after saving`, async ({
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
      "README.md",
    );
    const workspace = session.projects[0].workspaces[0];
    workspace.tabs = workspace.tabs.filter((t) => t.type === "file");
    if (ancestor === "project") {
      const second = newWorkspace("/project", "default");
      second.tabs = workspace.tabs.map((t) => ({
        ...t,
        id: crypto.randomUUID(),
      }));
      second.activeTabId = second.tabs[0].id;
      session.projects[0].workspaces.push(second);
    }

    await mockDesktop(page, false, session);
    await page.addInitScript(() => {
      const desktop = window as any;
      const invoke = desktop.__TAURI_INTERNALS__.invoke;
      desktop.__workspaceClose = {
        projection: null,
        acks: [],
        commits: 0,
        pending: true,
        failSave: false,
      };
      desktop.__TAURI_INTERNALS__.invoke = async (
        command: string,
        args: any,
      ) => {
        const state = desktop.__workspaceClose;
        if (command === "agent_control_ui_register") return "close-epoch";
        if (command === "agent_control_ui_publish") {
          state.projection = args.projection;
          return;
        }
        if (command === "agent_control_ui_claim") return;
        if (command === "agent_control_workspace_close_pending")
          return state.pending;
        if (command === "agent_control_ui_commit_close") {
          state.commits++;
          if (state.holdCommit)
            await new Promise((resolve) => {
              state.finishCommit = resolve;
            });
          return;
        }
        if (command === "agent_control_ui_ack") {
          state.acks.push(args.ack);
          return;
        }
        if (command === "save_editor_file" && state.failSave)
          throw Error("Fixture disk is full");
        return invoke(command, args);
      };
    });
    await page.goto("/");
    await expect(page.locator(".cm-content")).toBeVisible();
    await page.locator(".cm-content").focus();
    await page.keyboard.press("Control+a");
    await page.keyboard.insertText("Workspace unsaved Zażółć 🙂\n");
    await expect
      .poll(() =>
        page.evaluate(() =>
          Boolean((window as any).__workspaceClose.projection),
        ),
      )
      .toBe(true);
    const request = (operation: string) =>
      page.evaluate(
        async ({ operation, ancestor }) => {
          const desktop = window as any;
          const state = desktop.__workspaceClose;
          state.pending = true;
          const p = state.projection;
          const command: any = {
            operationId: operation,
            nonce: `${operation}-nonce`,
            uiEpoch: "close-epoch",
            domainRevision: p.revision,
            projectId: p.workspaces[0].projectId,
            action: {
              type: "close_workspace",
              workspaceId: p.workspaces[0].id,
              notAfterMillis: String(Date.now() + 120000),
              panels: p.panels
                .map((panel: any) => ({
                  panelId: panel.id,
                  tabId: panel.tabId,
                  kind: panel.kind,
                  terminalSessionId: panel.terminalSessionId,
                  browserGeneration: panel.browserGeneration,
                  androidDeviceId: panel.androidDeviceId,
                }))
                .sort((a: any, b: any) =>
                  a.panelId < b.panelId ? -1 : a.panelId > b.panelId ? 1 : 0,
                ),
            },
          };
          if (ancestor === "project") {
            command.action = {
              type: "close_project",
              workspaceId: p.workspaces[0].id,
              notAfterMillis: String(Date.now() + 120000),
              workspaces: p.workspaces
                .map((w: any) => ({
                  workspaceId: w.id,
                  notAfterMillis: String(Date.now() + 120000),
                  panels: command.action.panels.filter(
                    (panel: any) =>
                      p.panels.find((old: any) => old.id === panel.panelId)
                        .workspaceId === w.id,
                  ),
                }))
                .sort((a: any, b: any) =>
                  a.workspaceId.localeCompare(b.workspaceId),
                ),
            };
          }
          await desktop.__nativeTest.emitEvent(
            "agent-control-command",
            command,
          );
        },
        { operation, ancestor },
      );
    const result = async (operation: string) => {
      await expect
        .poll(() =>
          page.evaluate(
            (operation) =>
              (window as any).__workspaceClose.acks.find(
                (a: any) => a.operationId === operation,
              ),
            operation,
          ),
        )
        .toBeTruthy();
      return await page.evaluate(
        (operation) =>
          (window as any).__workspaceClose.acks.find(
            (a: any) => a.operationId === operation,
          ).result,
        operation,
      );
    };
    const dialog = page.getByRole("dialog", {
      name: "Save changes before closing?",
    });
    await request("cancel");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    expect(await result("cancel")).toMatchObject({
      kind: "failure",
      code: "CONTROL_REVOKED",
    });
    await expect(page.locator(".cm-content")).toContainText(
      "Workspace unsaved Zażółć 🙂",
    );
    await request("revoked");
    await expect(dialog).toBeVisible();
    await page.evaluate(() => {
      (window as any).__workspaceClose.pending = false;
    });
    await expect(dialog).toBeHidden();
    expect(await result("revoked")).toMatchObject({
      kind: "failure",
      code: "CONTROL_REVOKED",
    });
    await request("save-failed");
    await expect(dialog).toBeVisible();
    await page.evaluate(() => {
      (window as any).__workspaceClose.failSave = true;
    });
    await dialog
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(dialog).toContainText("Fixture disk is full");
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    expect(await result("save-failed")).toMatchObject({
      kind: "failure",
      code: "OUTCOME_UNKNOWN",
    });
    await page.evaluate(() => {
      (window as any).__workspaceClose.failSave = false;
    });
    await request("save");
    await expect(dialog).toBeVisible();
    await dialog
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    expect(await result("save")).toMatchObject({
      kind: `${ancestor}_closure`,
      closed: false,
      ...(ancestor === "workspace" ? { projectClosed: false } : {}),
    });
    expect(
      await page.evaluate(() => (window as any).__workspaceClose.commits),
    ).toBe(0);
    await expect(page.locator(".cm-content")).toContainText(
      "Workspace unsaved Zażółć 🙂",
    );
    await page.evaluate(() => {
      (window as any).__workspaceClose.holdCommit = true;
    });
    await request("edit-during-close");
    await expect
      .poll(() => page.evaluate(() => (window as any).__workspaceClose.commits))
      .toBe(1);
    await page.locator(".cm-content").focus();
    await page.keyboard.press("Control+a");
    await page.keyboard.insertText(
      "Typed while native resources were stopping 🙂",
    );
    await page.evaluate(() => {
      const s = (window as any).__workspaceClose;
      s.holdCommit = false;
      s.finishCommit();
    });
    expect(await result("edit-during-close")).toMatchObject({
      kind: "failure",
      code: "REVISION_CONFLICT",
    });
    await expect(page.locator(".cm-content")).toContainText(
      "Typed while native resources were stopping 🙂",
    );
    await request("close-saved");
    await expect(dialog).toBeVisible();
    await dialog
      .getByRole("button", { name: "Discard changes", exact: true })
      .click();
    expect(await result("close-saved")).toMatchObject({
      kind: `${ancestor}_closure`,
      closed: true,
      ...(ancestor === "workspace" ? { projectClosed: true } : {}),
    });
    expect(
      await page.evaluate(() => (window as any).__workspaceClose.commits),
    ).toBe(2);
    await expect(page.locator(".cm-content")).toHaveCount(0);
    await page.screenshot({
      path: `test-results/agent-${ancestor}-closed.png`,
    });
  });

test("project opening waits for Settings and preserves existing dirty buffers without starting terminals", async ({
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
    "README.md",
  );
  session.projects[0].workspaces[0].tabs =
    session.projects[0].workspaces[0].tabs.filter((t) => t.type === "file");
  await mockDesktop(page, false, session);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__projectOpen = {
      projection: null,
      acks: [],
      approved: false,
      rejected: false,
      commits: 0,
      terminalStarts: 0,
      readiness: 0,
    };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      const state = desktop.__projectOpen;
      if (command === "agent_control_ui_register") return "open-epoch";
      if (command === "agent_control_ui_publish") {
        state.projection = args.projection;
        return;
      }
      if (command === "agent_control_ui_claim") return;
      if (command === "agent_control_project_open_ready") {
        state.readiness++;
        if (state.rejected) throw "CONTROL_REVOKED";
        return state.approved;
      }
      if (command === "agent_control_project_open_commit") {
        state.commits++;
        return;
      }
      if (command === "agent_control_ui_ack") {
        state.acks.push(args.ack);
        return;
      }
      if (command === "start_terminal") state.terminalStarts++;
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await expect(page.locator(".cm-content")).toBeVisible();
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText("Existing project unsaved 🙂\n");
  await expect
    .poll(() =>
      page.evaluate(() => Boolean((window as any).__projectOpen.projection)),
    )
    .toBe(true);
  const send = (operation: string) =>
    page.evaluate(async (operation) => {
      const desktop = window as any;
      const state = desktop.__projectOpen;
      const p = state.projection;
      await desktop.__nativeTest.emitEvent("agent-control-command", {
        operationId: operation,
        nonce: operation + "-nonce",
        uiEpoch: "open-epoch",
        domainRevision: p.revision,
        projectId: p.workspaces[0].projectId,
        action: {
          type: "open_project",
          workspaceId: p.workspaces[0].id,
          projectId: "new-project",
          projectPath: "/new-project",
          newWorkspaceId: "new-workspace",
          tabId: "new-blank",
          name: "New workspace",
          requestKey: operation,
          notAfterMillis: String(Date.now() + 120000),
        },
      });
    }, operation);
  await send("rejected");
  await expect
    .poll(() => page.evaluate(() => (window as any).__projectOpen.readiness))
    .toBeGreaterThan(1);
  expect(
    await page.evaluate(() =>
      (window as any).__projectOpen.projection.workspaces.some(
        (w: any) => w.id === "new-workspace",
      ),
    ),
  ).toBe(false);
  await page.evaluate(() => {
    (window as any).__projectOpen.rejected = true;
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__projectOpen.acks[0]?.result),
    )
    .toMatchObject({ kind: "failure", code: "CONTROL_REVOKED" });
  await expect(page.locator(".cm-content")).toContainText(
    "Existing project unsaved 🙂",
  );
  expect(await page.evaluate(() => (window as any).__projectOpen.commits)).toBe(
    0,
  );
  await page.evaluate(() => {
    (window as any).__projectOpen.rejected = false;
    (window as any).__projectOpen.approved = true;
  });
  await send("approved");
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__projectOpen.acks[1]?.result),
    )
    .toMatchObject({
      kind: "project_opened",
      projectId: "new-project",
      workspaceId: "new-workspace",
      opened: true,
    });
  const projection = await page.evaluate(
    () => (window as any).__projectOpen.projection,
  );
  expect(projection.workspaces).toHaveLength(2);
  expect(
    projection.panels.filter((p: any) => p.workspaceId === "new-workspace"),
  ).toEqual([
    expect.objectContaining({
      id: "new-blank",
      kind: "file",
      terminalSessionId: null,
    }),
  ]);
  expect(
    await page.evaluate(() => (window as any).__projectOpen.terminalStarts),
  ).toBe(0);
  const document = await page.evaluate(async () => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const doc = runtime
      .documents()
      .find((d: any) => d.location.relative === "README.md");
    return { text: doc.state.doc.toString(), dirty: !doc.matchesSaved() };
  });
  expect(document).toEqual({
    text: "Existing project unsaved 🙂\n",
    dirty: true,
  });
  await page.screenshot({ path: "test-results/agent-project-opened.png" });
});
