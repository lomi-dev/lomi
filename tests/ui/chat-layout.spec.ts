import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockChats } from "./chat-mock";
import { defaultConfig } from "../../src/chat/types";

async function openChat(page: Page) {
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Chat AI", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toBeVisible();
}

async function emptySettings(page: Page) {
  await mockDesktop(page, false);
  await page.addInitScript((defaults) => {
    if (!localStorage.getItem("chat-preferences"))
      localStorage.setItem(
        "chat-preferences",
        JSON.stringify({
          version: 1,
          revision: 0,
          connections: [],
          defaults,
          sendMode: "enter",
        }),
      );
  }, defaultConfig);
  await mockChats(page);
  await page.goto("/?window=settings&page=chat-ai");
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
}

for (const [provider, model, name, label = model] of [
  ["openai", "gpt-4.1", "OpenAI", "GPT-4.1"],
  ["anthropic", "claude-opus-5", "Anthropic", "Claude Opus 5"],
  ["google", "gemini-2.5-flash", "Google (AI Studio)", "Gemini 2.5 Flash"],
  ["xai", "grok-4.6", "xAI"],
  ["openrouter", "openai/gpt-4.1", "OpenRouter"],
  ["deepseek", "deepseek-flash", "DeepSeek"],
  ["nvidia", "meta/llama-3.1-8b-instruct", "NVIDIA Build"],
]) {
  test(`first ${provider} connection saves a usable model and default in one step without a paid test`, async ({
    page,
  }) => {
    await emptySettings(page);
    await page
      .getByRole("dialog")
      .getByRole("button", { name, exact: true })
      .click();
    const modelSelect = page.getByRole("combobox", {
      name: "Model",
      exact: true,
    });
    await expect(modelSelect).toBeDisabled();
    await expect(modelSelect).toHaveText("Enter an API key first");
    await expect(
      page.getByRole("textbox", { name: "Connection name" }),
    ).toBeHidden();
    await page
      .getByLabel("API key", { exact: true })
      .fill("fixture-not-a-real-key");
    await modelSelect.click();
    await page.getByRole("option", { name: label, exact: true }).click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Add provider", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    const saved = await page.evaluate(() =>
      JSON.parse(localStorage.getItem("chat-preferences")!),
    );
    expect(saved.connections).toHaveLength(1);
    expect(saved.connections[0]).toMatchObject({
      name,
      provider,
      secretMode: "system",
    });
    expect(saved.defaults).toMatchObject({
      connectionId: saved.connections[0].id,
      model,
      configured: true,
    });
    expect(
      await page.evaluate(() => (window as any).__chatTest.connectionActions),
    ).toEqual([]);
    await page.goto("/");
    await openChat(page);
    const input = page.getByRole("textbox", { name: "Message", exact: true });
    await input.fill("Ready to chat");
    await expect(
      page.getByRole("button", { name: "Send", exact: true }),
    ).toBeEnabled();
    await input.press("Enter");
    await expect(page.locator(".chat-message-assistant")).toBeVisible();
  });
}

test("failed key storage keeps the form and key available for explicit session-only saving", async ({
  page,
}) => {
  await emptySettings(page);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "OpenAI", exact: true })
    .click();
  await page
    .getByLabel("API key", { exact: true })
    .fill("fixture-not-a-real-key");
  await page.getByRole("combobox", { name: "Model", exact: true }).click();
  await page.getByRole("option", { name: "GPT-4.1", exact: true }).click();
  await page.evaluate(() => {
    (window as any).__chatTest.failPreferences = true;
  });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Add provider", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("alert")).toContainText(
    "credential store is locked",
  );
  await expect(page.getByLabel("API key", { exact: true })).toHaveValue(
    "fixture-not-a-real-key",
  );
  await dialog.getByText("Advanced options", { exact: true }).click();
  await page.getByRole("combobox", { name: "Key storage" }).click();
  await page
    .getByRole("option", {
      name: "Session only · expires when the app closes",
      exact: true,
    })
    .click();
  await page.evaluate(() => {
    (window as any).__chatTest.failPreferences = false;
  });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Add provider", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("chat-preferences")!).connections[0]
          .secretMode,
    ),
  ).toBe("session");
});

test("conversation model changes support custom IDs and preserve advanced configuration", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.getByRole("button", { name: "Choose model" }).click();
  await page.getByRole("combobox", { name: "Model", exact: true }).click();
  await page
    .getByRole("option", { name: "Custom model…", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Custom model ID" })
    .fill("custom-model");
  await page.getByText("Advanced options", { exact: true }).click();
  await page
    .getByRole("textbox", { name: "System instructions" })
    .fill("Keep replies concise.");
  await page.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(page.getByRole("button", { name: "Choose model" })).toHaveText(
    "custom-model",
  );
  await page.getByRole("button", { name: "Choose model" }).click();
  await expect(
    page.getByRole("textbox", { name: "Custom model ID" }),
  ).toHaveValue("custom-model");
  await page.getByText("Advanced options", { exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "System instructions" }),
  ).toHaveValue("Keep replies concise.");
});

test("chat contains long content, resizes drafts, and preserves reading position at narrow sizes", async ({
  page,
}, info) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await openChat(page);
  await page.evaluate(() => {
    (window as any).__chatTest.chunks = 1;
    (window as any).__chatTest.response =
      "A quieter space to think.\n\nKeep the conversation readable, with room for your ideas and the details that matter.\n\n```ts\nconst message = 'Hello, SimpleBench';\n```";
  });
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Can you help me simplify this interface?");
  await page.screenshot({ path: info.outputPath("chat-draft.png") });
  await input.press("Enter");
  await expect(page.getByRole("button", { name: "Regenerate" })).toBeVisible();
  await page.screenshot({ path: info.outputPath("chat-dark.png") });
  await page.emulateMedia({ colorScheme: "light" });
  await page.screenshot({ path: info.outputPath("chat-light.png") });
  await page.evaluate(() => {
    (window as any).__chatTest.response =
      `${"A long paragraph with enough detail to scroll.\n\n".repeat(60)}\n\n\`\`\`txt\n${"long_code_".repeat(160)}\n\`\`\``;
  });
  await input.fill("Show a longer response");
  await input.press("Enter");
  await expect(page.getByRole("button", { name: "Regenerate" })).toHaveCount(2);
  const messages = page.locator(".chat-messages");
  await messages.evaluate((element) => {
    element.scrollTop = 120;
    element.dispatchEvent(new Event("scroll"));
  });
  await expect(
    page.getByRole("button", { name: "Jump to latest message" }),
  ).toBeVisible();
  await input.fill("Draft line\n".repeat(40));
  await page.screenshot({ path: info.outputPath("chat-multiline.png") });
  for (const size of [
    { width: 800, height: 420 },
    { width: 500, height: 360 },
    { width: 400, height: 300 },
    { width: 400, height: 210 },
  ]) {
    await page.setViewportSize(size);
    await expect(
      page.getByRole("button", { name: "Send", exact: true }),
    ).toBeInViewport();
    await expect(input).toBeInViewport();
    expect(
      await page
        .locator(".chat-pane")
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
      JSON.stringify(size),
    ).toBe(true);
    expect(
      await messages.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    expect(
      await input.evaluate(
        (element) => element.scrollHeight > element.clientHeight,
      ),
    ).toBe(true);
    expect(
      await messages.evaluate(
        (element) =>
          element.scrollHeight - element.scrollTop - element.clientHeight,
      ),
    ).toBeGreaterThan(500);
  }
  await page.getByRole("button", { name: "Jump to latest message" }).click();
  await expect
    .poll(() =>
      messages.evaluate(
        (element) =>
          element.scrollHeight - element.scrollTop - element.clientHeight,
      ),
    )
    .toBeLessThan(2);
  await input.fill("Short draft");
  expect((await input.boundingBox())!.height).toBeLessThan(50);
  await page.screenshot({ path: info.outputPath("chat-narrow.png") });
});

test("setup and advanced conversation dialogs scroll inside a short window", async ({
  page,
}, info) => {
  await emptySettings(page);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "OpenAI", exact: true })
    .click();
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({ path: info.outputPath("chat-setup.png") });
  await page.getByText("Advanced options", { exact: true }).click();
  await page.setViewportSize({ width: 640, height: 360 });
  const dialog = page.getByRole("dialog");
  const save = dialog.getByRole("button", {
    name: "Add provider",
    exact: true,
  });
  await save.scrollIntoViewIfNeeded();
  await expect(save).toBeInViewport();
  expect(
    await dialog.evaluate(
      (element) => element.getBoundingClientRect().bottom <= innerHeight,
    ),
  ).toBe(true);
  await page.screenshot({ path: info.outputPath("chat-setup-short.png") });
  await dialog
    .getByLabel("API key", { exact: true })
    .fill("fixture-layout-key");
  await dialog.getByRole("combobox", { name: "Model", exact: true }).click();
  await page.getByRole("option", { name: "GPT-4.1", exact: true }).click();
  await save.click();
  await expect(dialog).toHaveCount(0);
  await page.goto("/");
  await openChat(page);
  await page.getByRole("button", { name: "Chat AI settings" }).click();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: { command: string }) => call.command === "open_settings",
      ),
    ),
  ).toContainEqual({ command: "open_settings", args: { page: "chat-ai" } });
  await expect(dialog).toHaveCount(0);
  await page.getByRole("button", { name: "Choose model" }).click();
  await page.getByText("Advanced options", { exact: true }).click();
  const apply = page.getByRole("button", { name: "Apply", exact: true });
  await apply.scrollIntoViewIfNeeded();
  await expect(apply).toBeInViewport();
  await expect(
    page.getByRole("button", { name: "Close dialog" }),
  ).toBeInViewport();
});
