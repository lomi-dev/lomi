import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockChats } from "./chat-mock";

async function openChat(page: import("@playwright/test").Page) {
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Chat AI", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toBeVisible();
}

async function waitForChatSession(page: import("@playwright/test").Page) {
  await expect
    .poll(() =>
      page.evaluate(() => {
        const session = JSON.parse(
          localStorage.getItem("test-session") ?? "null",
        );
        const workspace = session?.projects[0]?.workspaces[0];
        return workspace?.tabs.find(
          (tab: any) => tab.id === workspace.activeTabId,
        )?.type;
      }),
    )
    .toBe("chat");
}

test("dynamic tool result survives reload and replays parts in order without sending again", async ({
  page,
}) => {
  const externalRequests: string[] = [];
  page.on("request", (request) => {
    if (request.url().startsWith("https://example.invalid/"))
      externalRequests.push(request.url());
  });
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.evaluate(() => {
    (window as any).__chatTest.toolScenario = "success";
  });
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Use a tool");
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .press("Enter");

  const tool = page.locator(".chat-tool");
  await expect(tool).toBeVisible();
  await expect(tool.locator(".chat-tool-heading")).toContainText(
    "lomi_workspace_list",
  );
  await expect(tool.locator("[role=status]")).toHaveText("Completed");
  await tool.getByText("Input", { exact: true }).click();
  await expect(tool.locator("pre").first()).toContainText("workspace-fixture");
  await tool.getByText("Result details", { exact: true }).click();
  await expect(tool).toContainText("Found one workspace.");
  await expect(tool).toContainText("Structured content");
  await expect(tool).toContainText("Fixture workspace");
  await expect(tool).toContainText("[binary content omitted]");
  await expect(tool).toContainText("Image omitted (unsupported or too large).");
  await expect(tool.locator("img")).toHaveCount(1);
  await expect(tool.locator("img")).toHaveAttribute(
    "src",
    /^data:image\/png;base64,/,
  );
  await expect
    .poll(() =>
      tool
        .locator("img")
        .evaluate((image) => (image as HTMLImageElement).naturalWidth),
    )
    .toBe(96);
  expect(externalRequests).toEqual([]);
  await page.screenshot({ path: "test-results/chat-mcp-tool-result.png" });

  await waitForChatSession(page);
  await page.reload();
  const assistantParts = page
    .locator(".chat-message-assistant .chat-message-content")
    .first()
    .locator(":scope > *");
  await expect(assistantParts).toHaveCount(3);
  await expect(assistantParts.nth(0)).toHaveClass(/chat-markdown/);
  await expect(assistantParts.nth(1)).toHaveClass(/chat-tool/);
  await expect(assistantParts.nth(2)).toHaveClass(/chat-markdown/);
  await expect(
    page.locator(".chat-message-assistant .chat-markdown"),
  ).toHaveText(["Before the tool.", "After the tool."]);
  expect(await page.evaluate(() => (window as any).__chatTest.starts)).toBe(1);
  await expect(page.locator(".chat-mcp-guidance")).toContainText(
    "Settings → Agent control",
  );
});

test("pending tools show live progress and an interrupted result after reload", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.evaluate(() => {
    (window as any).__chatTest.toolScenario = "pending";
  });
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Start a tool");
  await input.press("Enter");
  const tool = page.locator(".chat-tool");
  await expect(tool.locator("[role=status]")).toHaveText("In progress");

  await waitForChatSession(page);
  await page.reload();
  await expect(page.locator(".chat-tool [role=status]")).toHaveText(
    "In progress",
  );
  expect(await page.evaluate(() => (window as any).__chatTest.starts)).toBe(1);

  await page.evaluate(() => {
    const fixture = (window as any).__chatTest;
    const loaded = Object.values(fixture.conversations)[0] as any;
    loaded.request.status = "interrupted";
    loaded.messages.find(
      (message: any) => message.role === "assistant",
    ).status = "interrupted";
    localStorage.setItem(
      "chat-conversations",
      JSON.stringify(fixture.conversations),
    );
  });
  await page.reload();
  await expect(page.locator(".chat-tool [role=status]")).toHaveText(
    "Interrupted · no result recorded",
  );
  await expect(page.locator(".chat-tool [role=status]")).not.toContainText(
    "In progress",
  );
  expect(await page.evaluate(() => (window as any).__chatTest.starts)).toBe(1);
});

test("dynamic tool SDK errors are displayed as failures with their error text", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.evaluate(() => {
    (window as any).__chatTest.toolScenario = "error";
  });
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Call a failing tool");
  await input.press("Enter");
  const tool = page.locator(".chat-tool");
  await expect(tool.locator("[role=status]")).toHaveText("Failed");
  await tool.getByText("Error details", { exact: true }).click();
  await expect(tool).toContainText("The fixture tool was denied.");
});

test("MCP isError results use the failure state", async ({ page }) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.evaluate(() => {
    (window as any).__chatTest.toolScenario = "mcp-error";
  });
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Read from a failing MCP server");
  await input.press("Enter");
  const tool = page.locator(".chat-tool");
  await expect(tool.locator("[role=status]")).toHaveText("Failed");
  await tool.getByText("Error details", { exact: true }).click();
  await expect(tool).toContainText("The MCP server reported an error.");
});
