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
test("chat keeps one request across tab switches, preserves the next draft after late ACK, and stops natively", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.evaluate(() => {
    (window as any).__chatTest.hold = true;
    (window as any).__chatTest.slowAck = true;
  });
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("First conversation");
  await input.press("Enter");
  await expect(input).toHaveValue("");
  await input.fill("Next draft 日本語");
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.waitForTimeout(800);
  await page
    .getByRole("tab", { name: "First conversation", exact: true })
    .click();
  await expect(input).toHaveValue("Next draft 日本語");
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => (window as any).__chatTest.starts))
    .toBe(1);
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".chat-message-assistant")).toContainText(
    "Zażółć 日本語 👩🏽‍💻",
  );
  expect(await page.evaluate(() => (window as any).__chatTest.stops)).toBe(1);
  await expect
    .poll(() =>
      page.evaluate(() =>
        Object.values((window as any).__chatTest.conversations).map(
          (c: any) => c.draft.text,
        ),
      ),
    )
    .toEqual(["Next draft 日本語"]);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const s = JSON.parse(localStorage.getItem("test-session") ?? "null");
        const w = s?.projects[0]?.workspaces[0];
        return w?.tabs.find((t: any) => t.id === w.activeTabId)?.type;
      }),
    )
    .toBe("chat");
  await page.reload();
  await expect(input).toHaveValue("Next draft 日本語");
  await expect(page.locator(".chat-message-assistant")).toContainText("日本語");
  expect(errors).toEqual([]);
});
test("chat docks into terminal layout, guards failed flush, supports history and minimum size", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Keep this draft");
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  const source = (await page
    .getByRole("tab", { name: "Chat AI", exact: true })
    .boundingBox())!;
  const target = (await page.locator(".terminal-layout").boundingBox())!;
  await page.mouse.move(
    source.x + source.width / 2,
    source.y + source.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    target.x + target.width - 15,
    target.y + target.height / 2,
    { steps: 10 },
  );
  await page.mouse.up();
  await expect(page.getByRole("tab")).toHaveCount(1);
  await expect(input).toHaveValue("Keep this draft");
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
  await page.setViewportSize({ width: 800, height: 420 });
  await expect(input).toBeInViewport();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeInViewport();
  await page.screenshot({ path: "test-results/chat-split-minimum.png" });
  await page.getByRole("button", { name: "Chat history", exact: true }).click();
  await expect(
    page.getByRole("complementary", { name: "Chat history" }),
  ).toBeVisible();
  await page
    .getByRole("searchbox", { name: "Search conversations" })
    .fill("Chat AI");
  await expect(page.locator(".chat-history-item")).toHaveCount(1);
  await page.keyboard.press("Escape");
  await page.evaluate(() => {
    (window as any).__chatTest.failClose = true;
  });
  await page.getByRole("button", { name: "Close chat panel" }).click();
  await expect(
    page.getByRole("dialog", { name: "Conversation could not be saved" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Keep open" }).click();
  await expect(input).toHaveValue("Keep this draft");
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
  await page.evaluate(() => {
    (window as any).__chatTest.failClose = false;
  });
  await page.getByRole("button", { name: "Close chat panel" }).click();
  await expect(page.locator(".chat-pane")).toHaveCount(0);
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
});
test("settings route supports named connections without reading a secret", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/?window=settings&page=chat-ai");
  await expect(
    page.getByRole("heading", { name: "Chat AI", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Google (AI Studio)", exact: true })
    .click();
  await page.getByText("Advanced options", { exact: true }).click();
  await page
    .getByRole("textbox", { name: "Connection name" })
    .fill("Second connection");
  await page
    .getByLabel("API key", { exact: true })
    .fill("fixture-not-a-real-key");
  await page
    .getByRole("combobox", { name: "Key storage", exact: true })
    .click();
  await page
    .getByRole("option", {
      name: "Session only · expires when the app closes",
      exact: true,
    })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Add provider", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: /Second connection/ }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Edit key", exact: true })
    .last()
    .click();
  await expect(page.getByLabel(/Replacement API key/)).toHaveValue("");
});

test("chat resync reconstructs SDK blocks without sending again or duplicating text", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.evaluate(() => {
    (window as any).__chatTest.hold = true;
  });
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Resync");
  await input.press("Enter");
  await expect(page.locator(".chat-message-assistant")).toContainText("日本語");
  await page.evaluate(() => {
    const r = [...(window as any).__chatTest.requests.values()][0] as any;
    r.channel.onmessage({
      type: "resync",
      epoch: r.epoch,
      sequence: r.sequence,
    });
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          ([...(window as any).__chatTest.requests.values()][0] as any).epoch,
      ),
    )
    .toBe(1);
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeVisible();
  const result = await page.evaluate(() => {
    const t = (window as any).__chatTest;
    return {
      starts: t.starts,
      stops: t.stops,
      text: ([...t.requests.values()][0] as any).message.parts[0].text.trim(),
    };
  });
  expect(result.starts).toBe(1);
  expect(result.stops).toBe(1);
  await expect(
    page.locator(".chat-message-assistant .chat-markdown"),
  ).toHaveText(result.text);
});

test("chat preserves IME and multiline input and blocks remote Markdown images and HTML", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  const remote: string[] = [];
  page.on("request", (r) => {
    if (r.url().includes("remote.invalid")) remote.push(r.url());
  });
  await page.goto("/");
  await openChat(page);
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("日本語");
  await input.dispatchEvent("keydown", {
    key: "Enter",
    code: "Enter",
    isComposing: true,
    keyCode: 229,
  });
  expect(await page.evaluate(() => (window as any).__chatTest.starts)).toBe(0);
  await input.press("Shift+Enter");
  await expect(input).toHaveValue("日本語\n");
  await page.evaluate(() => {
    (window as any).__chatTest.response =
      "![external](https://remote.invalid/pixel)\n<script>window.__unsafeChat = true</script>\n\n```sh\necho hello\n```\n";
  });
  await input.press("Enter");
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeVisible();
  await expect(
    page.locator(".chat-message-assistant pre").first(),
  ).toContainText("echo hello");
  expect(
    await page
      .locator(
        ".chat-message-assistant img, .chat-message-assistant script, .chat-message-assistant iframe",
      )
      .count(),
  ).toBe(0);
  expect(
    await page.evaluate(() => (window as any).__unsafeChat),
  ).toBeUndefined();
  expect(remote).toEqual([]);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.evaluate(() => {
    document.documentElement.style.zoom = "2";
  });
  await expect(input).toBeInViewport();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeInViewport();
  await page.screenshot({ path: "test-results/chat-dark-200-percent.png" });
});

test("draft write failure stops the active native request and retains unsaved input until retry", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.evaluate(() => {
    (window as any).__chatTest.hold = true;
  });
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Begin");
  await input.press("Enter");
  await expect(page.locator(".chat-message-assistant")).toContainText("日本語");
  await page.evaluate(() => {
    (window as any).__chatTest.failDraft = true;
  });
  await input.fill("Keep unsaved next draft");
  await expect
    .poll(() => page.evaluate(() => (window as any).__chatTest.stops))
    .toBe(1);
  await expect(input).toHaveValue("Keep unsaved next draft");
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeDisabled();
  await page.evaluate(() => {
    (window as any).__chatTest.failDraft = false;
  });
  await page.getByRole("button", { name: "Retry saving" }).click();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeEnabled();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (Object.values((window as any).__chatTest.conversations)[0] as any)
            .draft.text,
      ),
    )
    .toBe("Keep unsaved next draft");
});

test("declining the editor close guard leaves the chat request and PTYs running", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.evaluate(() => {
    (window as any).__chatTest.hold = true;
  });
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Guarded response");
  await input.press("Enter");
  await expect(page.locator(".chat-message-assistant")).toContainText("日本語");
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  await page.locator(".cm-content").focus();
  await page.keyboard.insertText("Unsaved editor change");
  await page.getByRole("button", { name: "Close window", exact: true }).click();
  const dialog = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await page.evaluate(() => (window as any).__chatTest.stops)).toBe(0);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some(
        (c: any) => c.command === "close_terminal",
      ),
    ),
  ).toBe(false);
  await page
    .getByRole("tab", { name: "Guarded response", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Stop", exact: true }).click();
});

test("history reuses a conversation already open in the workspace", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Existing conversation");
  await input.press("Enter");
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeVisible();
  await openChat(page);
  await input.fill("Preserve second draft");
  await page.getByRole("button", { name: "Chat history", exact: true }).click();
  await page
    .locator(".chat-history-item")
    .filter({ hasText: "Existing conversation" })
    .getByRole("button")
    .first()
    .click();
  await expect(
    page.getByRole("tab", { name: "Existing conversation", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("tab")).toHaveCount(3);
  await page.getByRole("tab", { name: "Chat AI", exact: true }).click();
  await expect(input).toHaveValue("Preserve second draft");
});
