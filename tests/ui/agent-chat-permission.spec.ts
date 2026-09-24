import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("Chat history is opt-in, exact, paginated and cleared when the workspace changes", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__chatPermission = { reads: [], approval: null };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "agent_control_state")
        return {
          supported: true,
          helperPath: null,
          broker: {
            endpoint: {
              instanceId: "fixture",
              endpoint: "fixture",
              brokerSha256: "fixture",
              ipcVersion: 1,
            },
            uiReady: true,
            terminalProfile: null,
            workspaces: [
              {
                id: "work",
                projectId: "project",
                name: "Visible",
                projectName: "Project",
                projectPath: "/project",
              },
              {
                id: "other",
                projectId: "other-project",
                name: "Other",
                projectName: "Other project",
                projectPath: "/other",
              },
            ],
            pending: [
              {
                id: "request",
                clientLabel: "Fixture client",
                certificateSha256: "fixture",
                secondsRemaining: 120,
              },
            ],
            pendingControls: [],
            pendingInstalls: [],
            pendingProjectOpens: [],
            pendingSettingsUpdates: [],
            sessions: [],
          },
        };
      if (command === "agent_control_chat_catalog") {
        desktop.__chatPermission.reads.push(args);
        return args.afterId === null
          ? {
              items: [
                { conversationId: "first", title: "My shared conversation" },
                { conversationId: "private", title: "Private conversation" },
              ],
              next: "private",
            }
          : {
              items: [
                { conversationId: "next", title: "Another conversation" },
              ],
              next: null,
            };
      }
      if (command === "agent_control_approve") {
        desktop.__chatPermission.approval = args;
        return;
      }
      return invoke(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  const permission = page.getByLabel(
    "Allow reading selected Chat AI conversations",
  );
  await expect(permission).toBeDisabled();
  await page.getByLabel("Workspace", { exact: true }).selectOption("work");
  await expect(permission).not.toBeChecked();
  expect(
    await page.evaluate(() => (window as any).__chatPermission.reads),
  ).toEqual([]);
  await permission.check();
  const approve = page.getByRole("button", {
    name: "Approve session",
    exact: true,
  });
  await expect(approve).toBeDisabled();
  await page.getByLabel("My shared conversation first").check();
  await expect(
    page.getByLabel("Private conversation private"),
  ).not.toBeChecked();
  await page
    .getByRole("group", { name: "Conversations to share" })
    .screenshot({ path: "test-results/agent-chat-permission-selection.png" });
  await page
    .getByRole("button", { name: "Next conversations", exact: true })
    .click();
  await page.getByLabel("Another conversation next").check();
  await expect(page.getByText("2 of 64 conversations selected")).toBeVisible();
  await page
    .getByRole("group", { name: "Conversations to share" })
    .screenshot({ path: "test-results/agent-chat-permission.png" });
  await page.getByLabel("Allow editing selected Chat AI drafts").check();
  await page.getByLabel("Allow sending selected Chat AI messages").check();
  await page.getByLabel("Allow stopping selected Chat AI responses").check();
  await page.getByLabel("Allow exporting selected Chat AI text").check();
  await approve.click();
  expect(
    await page.evaluate(() => (window as any).__chatPermission.approval),
  ).toMatchObject({
    workspaceIds: ["work"],
    scopes: [
      "workspace.read",
      "chat.read",
      "chat.draft",
      "chat.send",
      "chat.stop",
      "chat.export",
    ],
    chatConversations: ["first", "next"],
  });
  await page.getByLabel("Workspace", { exact: true }).selectOption("other");
  await expect(permission).not.toBeChecked();
  await expect(
    page.getByRole("group", { name: "Conversations to share" }),
  ).toHaveCount(0);
  await approve.click();
  expect(
    await page.evaluate(() => (window as any).__chatPermission.approval),
  ).toMatchObject({
    workspaceIds: ["other"],
    scopes: ["workspace.read"],
    chatConversations: [],
  });
});
