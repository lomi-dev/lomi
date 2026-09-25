import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockChats } from "./chat-mock";

test.beforeEach(async ({ page }) => {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/?window=settings&page=chat-ai");
  await expect(
    page.getByRole("heading", { name: "Chat AI", exact: true }),
  ).toBeVisible();
});

test("provider presets connect with the right service and keep entered keys private", async ({
  page,
}) => {
  const providers = page.getByRole("list", { name: "Your providers" });
  await expect(providers.getByRole("button")).toHaveCount(1);
  await expect(page.getByRole("button", { name: /NVIDIA/ })).toHaveCount(0);
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog
    .getByRole("button", { name: "NVIDIA Build", exact: true })
    .click();
  await expect(dialog).toHaveAccessibleName("Connect NVIDIA Build");
  const key = dialog.getByLabel("API key", { exact: true });
  await key.fill("fixture-key");
  await expect(key).toHaveAttribute("type", "password");
  await dialog.getByRole("button", { name: "Show key" }).click();
  await expect(key).toHaveAttribute("type", "text");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  await dialog
    .getByRole("button", { name: "NVIDIA Build", exact: true })
    .click();
  await expect(key).toHaveValue("");
  await expect(key).toHaveAttribute("type", "password");
  await key.fill("fixture-key");
  await dialog
    .getByRole("button", { name: "Add provider", exact: true })
    .click();
  await expect(
    providers.getByRole("button", { name: /NVIDIA Build/ }),
  ).toContainText("Key saved");
  await page.getByText("Connection tools", { exact: true }).click();
  await expect(
    page.getByRole("region", { name: "NVIDIA Build configuration" }),
  ).toContainText("https://integrate.api.nvidia.com/v1");
  await page.getByRole("button", { name: "Edit key", exact: true }).click();
  await expect(dialog.getByLabel(/Replacement API key/)).toHaveValue("");
  expect(
    await page.evaluate(() => localStorage.getItem("chat-preferences")),
  ).not.toContain("fixture-key");
});

test("models can be added, refreshed, searched and used as defaults without a paid test", async ({
  page,
}) => {
  await page.getByRole("button", { name: /Fixture/ }).click();
  await page.locator(".chat-models-section > summary").click();
  await page.getByRole("button", { name: "Add model", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Add model" });
  await dialog.getByLabel("Model ID").fill("custom/model");
  await dialog.getByRole("button", { name: "Add model", exact: true }).click();
  await page
    .getByRole("button", { name: "Use custom/model by default" })
    .click();
  await expect(
    page.getByRole("button", { name: "custom/model is the default model" }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => (window as any).__chatTest.connectionActions),
  ).toEqual([]);
  await page.getByRole("button", { name: "Refresh models" }).click();
  await page.getByRole("searchbox", { name: "Search models" }).fill("custom");
  await expect(
    page.getByRole("list", { name: "Provider models" }).getByRole("listitem"),
  ).toHaveCount(1);
  await page.reload();
  await page.getByRole("button", { name: /Fixture/ }).click();
  await page.locator(".chat-models-section > summary").click();
  await expect(
    page.getByRole("button", { name: "custom/model is the default model" }),
  ).toBeVisible();
  await page.getByRole("switch", { name: "Enable Fixture" }).uncheck();
  await expect(
    page.getByRole("button", { name: "Refresh models" }),
  ).toBeDisabled();
  await page.getByRole("switch", { name: "Enable Fixture" }).check();
  await page.evaluate(() => {
    (window as any).__chatTest.failConnectionAction = true;
  });
  await page.getByRole("button", { name: "Refresh models" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Could not refresh models",
  );
  await expect(
    page.getByRole("button", { name: "custom/model is the default model" }),
  ).toBeVisible();
});

test("chat defaults save on blur and keep text when saving fails", async ({
  page,
}) => {
  const instructions = page.getByRole("textbox", {
    name: "System instructions",
  });
  const saved = () =>
    page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("chat-preferences") ?? "null")?.defaults
          .system ?? "",
    );
  await instructions.fill("Unfinished draft");
  expect(await saved()).toBe("");
  await instructions.blur();
  await expect(page.getByRole("status")).toHaveText("Saved");
  expect(await saved()).toBe("Unfinished draft");
  await page.evaluate(() => {
    (window as any).__chatTest.failPreferences = true;
  });
  await instructions.fill("Kept after a failure");
  await instructions.blur();
  await expect(page.getByRole("alert")).toContainText(
    "credential store is locked",
  );
  await expect(instructions).toHaveValue("Kept after a failure");
  expect(await saved()).toBe("Unfinished draft");
  await page.evaluate(() => {
    (window as any).__chatTest.failPreferences = false;
  });
  await instructions.focus();
  await instructions.blur();
  await expect(page.getByRole("status")).toHaveText("Saved");
  expect(await saved()).toBe("Kept after a failure");
  await page.getByRole("button", { name: /Fixture/ }).click();
  await page.getByRole("switch", { name: "Enable Fixture" }).uncheck();
  await expect(instructions).toHaveValue("Kept after a failure");
});

test("provider settings fit dark, light, narrow and zoomed windows", async ({
  page,
}, info) => {
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1280, height: 850 });
    await page.screenshot({
      path: info.outputPath(`settings-${colorScheme}.png`),
    });
  }
  await page.getByRole("button", { name: /Fixture/ }).click();
  await page.locator(".chat-models-section > summary").click();
  for (const [width, height, zoom] of [
    [800, 600, 1],
    [640, 360, 1],
    [800, 600, 2],
  ]) {
    await page.setViewportSize({ width, height });
    await page.evaluate((zoom) => {
      document.documentElement.style.zoom = String(zoom);
    }, zoom);
    const panel = page.locator(".chat-settings");
    expect(await panel.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(
      true,
    );
    await page
      .getByRole("button", { name: "Add model", exact: true })
      .scrollIntoViewIfNeeded();
    await expect(
      page.getByRole("button", { name: "Add model", exact: true }),
    ).toBeInViewport();
    await page.screenshot({
      path: info.outputPath(`settings-${width}-${zoom}.png`),
    });
  }
});

test("changing providers clears the draft key and cancelling restores keyboard focus", async ({
  page,
}) => {
  const add = page.getByRole("button", { name: "Add provider", exact: true });
  await add.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "OpenAI", exact: true }).click();
  await expect(dialog.getByLabel("API key", { exact: true })).toBeFocused();
  await dialog
    .getByLabel("API key", { exact: true })
    .fill("unsaved-private-key");
  await dialog.getByRole("button", { name: "Show key" }).click();
  await dialog.getByRole("button", { name: "Change provider" }).click();
  await expect(
    dialog.getByRole("button", { name: "OpenAI", exact: true }),
  ).toBeFocused();
  await dialog.getByRole("button", { name: "Anthropic", exact: true }).click();
  await expect(dialog.getByLabel("API key", { exact: true })).toHaveValue("");
  await expect(dialog.getByLabel("API key", { exact: true })).toHaveAttribute(
    "type",
    "password",
  );
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(add).toBeFocused();
  await expect(
    page.getByRole("list", { name: "Your providers" }).getByRole("listitem"),
  ).toHaveCount(1);
  expect(
    await page.evaluate(() => localStorage.getItem("chat-preferences")),
  ).toBeNull();
});

test("the provider context menu supports keyboard dismissal and confirmed removal", async ({
  page,
}, info) => {
  const provider = page.getByRole("button", { name: /Fixture/ });
  const menu = page.getByRole("menu", { name: "Provider actions" });
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 640, height: 360 });
    await provider.click({ button: "right" });
    await expect(menu).toBeInViewport({ ratio: 1 });
    await expect(
      menu.getByRole("menuitem", { name: "Remove provider…" }),
    ).toBeFocused();
    await expect(provider).toHaveAttribute("aria-expanded", "false");
    await page.screenshot({
      path: info.outputPath(`provider-menu-${colorScheme}.png`),
    });
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
    await expect(provider).toBeFocused();
  }
  await provider.press("Shift+F10");
  await expect(menu).toBeVisible();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Remove provider?" });
  await expect(dialog).toContainText("Fixture");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(provider).toBeVisible();
  await provider.click({ button: "right" });
  await menu.getByRole("menuitem", { name: "Remove provider…" }).click();
  await page.keyboard.press("Escape");
  await expect(provider).toBeVisible();
  await provider.click({ button: "right" });
  await menu.getByRole("menuitem", { name: "Remove provider…" }).click();
  await dialog
    .getByRole("button", { name: "Remove provider", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Connect your first provider" }),
  ).toBeVisible();
  await expect(page.getByRole("list", { name: "Your providers" })).toHaveCount(
    0,
  );
  const saved = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("chat-preferences")!),
  );
  expect(saved.connections).toEqual([]);
  expect(saved.defaults.connectionId).toBeNull();
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Connect your first provider" }),
  ).toBeVisible();
  await page.screenshot({ path: info.outputPath("settings-empty.png") });
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  await expect(
    page
      .getByRole("dialog")
      .getByRole("button", { name: "OpenAI", exact: true }),
  ).toBeVisible();
});

test("provider removal targets the clicked connection and preserves it after a failed save", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  const setup = page.getByRole("dialog");
  await setup.getByRole("button", { name: "OpenAI", exact: true }).click();
  await setup.getByLabel("API key", { exact: true }).fill("second-fixture-key");
  await setup.getByText("Advanced options", { exact: true }).click();
  await setup.getByLabel("Connection name").fill("Second OpenAI");
  await setup
    .getByRole("button", { name: "Add provider", exact: true })
    .click();
  const second = page.getByRole("button", { name: /Second OpenAI/ });
  await expect(second).toHaveAttribute("aria-expanded", "true");
  const before = await page.evaluate(() =>
    localStorage.getItem("chat-preferences"),
  );
  await page
    .getByRole("button", { name: /Fixture/ })
    .click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove provider…" }).click();
  const dialog = page.getByRole("dialog", { name: "Remove provider?" });
  await expect(dialog).toContainText("Fixture");
  await page.evaluate(() => {
    (window as any).__chatTest.failPreferences = true;
  });
  await dialog
    .getByRole("button", { name: "Remove provider", exact: true })
    .click();
  await expect(dialog.getByRole("alert")).toContainText(
    "credential store is locked",
  );
  expect(
    await page.evaluate(() => localStorage.getItem("chat-preferences")),
  ).toBe(before);
  await page.evaluate(() => {
    (window as any).__chatTest.failPreferences = false;
  });
  await dialog
    .getByRole("button", { name: "Remove provider", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("button", { name: /Fixture/ })).toHaveCount(0);
  await expect(second).toHaveAttribute("aria-expanded", "true");
  const saved = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("chat-preferences")!),
  );
  expect(saved.connections).toEqual([JSON.parse(before!).connections[1]]);
  expect(saved.defaults.connectionId).toBeNull();
  await page.reload();
  await expect(second).toBeVisible();
  await expect(page.getByRole("button", { name: /Fixture/ })).toHaveCount(0);
});

test("saved providers stay manageable when disabled or missing a key", async ({
  page,
}, info) => {
  await page.evaluate(() => {
    const connection = {
      enabled: true,
      credentialRevision: 1,
      secretMode: "system",
      secretId: "fixture-key-id",
      models: [],
      testedModel: null,
      testStatus: null,
    };
    localStorage.setItem(
      "chat-preferences",
      JSON.stringify({
        version: 1,
        revision: 0,
        sendMode: "enter",
        connections: [
          { ...connection, id: "work", provider: "openai", name: "OpenAI" },
          {
            ...connection,
            id: "claude",
            provider: "anthropic",
            name: "Anthropic",
            enabled: false,
          },
          {
            ...connection,
            id: "router",
            provider: "openrouter",
            name: "OpenRouter",
            secretId: null,
          },
        ],
        defaults: {
          connectionId: "work",
          model: "gpt-4.1",
          system: "",
          maxOutputTokens: 4096,
          temperature: null,
          configured: true,
        },
      }),
    );
  });
  await page.reload();
  const providers = page.getByRole("list", { name: "Your providers" });
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1000, height: 720 });
    await expect(
      providers.getByRole("button", { name: /Anthropic/ }),
    ).toContainText("Disabled");
    await expect(
      providers.getByRole("button", { name: /OpenRouter/ }),
    ).toContainText("Needs API key");
    await page.screenshot({
      path: info.outputPath(`settings-providers-${colorScheme}.png`),
    });
  }
  await providers.getByRole("button", { name: /OpenRouter/ }).click();
  await page.getByRole("button", { name: "Add key", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByLabel("API key", { exact: true })).toHaveAttribute(
    "required",
    "",
  );
  await dialog
    .getByLabel("API key", { exact: true })
    .fill("replacement-fixture-key");
  await dialog.getByRole("button", { name: "Save changes" }).click();
  await expect(
    providers.getByRole("button", { name: /OpenRouter/ }),
  ).toContainText("Key saved");
  await providers.getByRole("button", { name: /Anthropic/ }).click();
  await page.getByRole("switch", { name: "Enable Anthropic" }).check();
  await expect(
    providers.getByRole("button", { name: /Anthropic/ }),
  ).not.toContainText("Disabled");
  const saved = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("chat-preferences")!),
  );
  expect(saved.connections).toHaveLength(3);
  expect(saved.connections[2].credentialRevision).toBe(2);
});

test("provider picker and setup fit both themes and small windows", async ({
  page,
}, info) => {
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  const dialog = page.getByRole("dialog");
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1000, height: 720 });
    await page.screenshot({
      path: info.outputPath(`provider-picker-${colorScheme}.png`),
    });
  }
  for (const [width, height, zoom] of [
    [640, 360, 1],
    [800, 600, 2],
  ]) {
    await page.setViewportSize({ width, height });
    await page.evaluate((zoom) => {
      document.documentElement.style.zoom = String(zoom);
    }, zoom);
    const choice = dialog.getByRole("button", {
      name: "Anthropic",
      exact: true,
    });
    await choice.scrollIntoViewIfNeeded();
    await expect(choice).toBeInViewport();
    expect(
      await dialog.evaluate((el) => el.scrollWidth <= el.clientWidth),
    ).toBe(true);
    await choice.click();
    await expect(dialog.getByLabel("API key", { exact: true })).toBeFocused();
    const save = dialog.getByRole("button", {
      name: "Add provider",
      exact: true,
    });
    await save.scrollIntoViewIfNeeded();
    await expect(save).toBeInViewport();
    await page.screenshot({
      path: info.outputPath(`provider-setup-${width}-${zoom}.png`),
    });
    await dialog.getByRole("button", { name: "Change provider" }).click();
  }
});

test("custom model selects support keyboard choices without closing or submitting setup", async ({
  page,
}, info) => {
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog
    .getByRole("button", { name: "Google (AI Studio)", exact: true })
    .click();
  await dialog
    .getByLabel("API key", { exact: true })
    .fill("fixture-select-key");
  await dialog
    .getByRole("checkbox", { name: "Use for new conversations" })
    .check();
  const model = dialog.getByRole("combobox", { name: "Model", exact: true });
  await model.click();
  await page
    .getByRole("option", { name: "Gemini 2.5 Flash", exact: true })
    .click();
  await expect(model).toHaveText("Gemini 2.5 Flash");
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme });
    for (const [width, height] of [
      [1000, 720],
      [640, 360],
      [400, 300],
    ]) {
      await page.setViewportSize({ width, height });
      await model.click();
      const list = page.getByRole("listbox");
      await expect(list).toBeInViewport({ ratio: 1 });
      await expect(
        page.getByRole("option", { name: "Gemini 2.5 Flash", exact: true }),
      ).toHaveAttribute("aria-selected", "true");
      await model.press("End");
      await expect(
        page.getByRole("option", { name: "Gemini 2.5 Pro", exact: true }),
      ).toBeInViewport();
      await page.screenshot({
        path: info.outputPath(`model-select-${colorScheme}-${width}.png`),
      });
      await model.press("Escape");
      await expect(list).toHaveCount(0);
      await expect(dialog).toBeVisible();
      await expect(model).toBeFocused();
      await expect(model).toHaveText("Gemini 2.5 Flash");
    }
  }
  await model.press("Home");
  await model.press("ArrowDown");
  await model.press("Enter");
  await expect(model).toHaveText("Gemini 2.5 Pro");
  await expect(dialog).toBeVisible();
  expect(
    await page.evaluate(() => localStorage.getItem("chat-preferences")),
  ).toBeNull();
  await model.click();
  await expect(
    page.getByRole("option", { name: "Custom model…", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("option", { name: "Gemini 2.5 Flash", exact: true })
    .click();
  await expect(dialog.getByLabel("Custom model ID")).toHaveCount(0);
  await dialog
    .getByRole("button", { name: "Add provider", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  const saved = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("chat-preferences")!),
  );
  expect(saved.defaults.model).toBe("gemini-2.5-flash");
});

test("new provider models load only after a key and ignore stale responses", async ({
  page,
}, info) => {
  await page.evaluate(() => {
    (window as any).__chatTest.holdModelPreviews = true;
  });
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "OpenAI", exact: true }).click();
  await dialog
    .getByRole("checkbox", { name: "Use for new conversations" })
    .check();
  const key = dialog.getByLabel("API key", { exact: true });
  const model = dialog.getByRole("combobox", { name: "Model", exact: true });
  const add = dialog.getByRole("button", { name: "Add provider", exact: true });
  await expect(model).toBeDisabled();
  await expect(model).toHaveText("Enter an API key first");
  await expect(add).toBeDisabled();
  await page.screenshot({ path: info.outputPath("models-no-key.png") });
  await key.fill("   ");
  await expect(model).toBeDisabled();
  expect(
    await page.evaluate(() => (window as any).__chatTest.modelPreviews),
  ).toEqual([]);
  await key.fill("first-private-key");
  await expect(model).toHaveText("Loading models…");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__chatTest.pendingModelPreviews.length,
      ),
    )
    .toBe(1);
  await page.screenshot({ path: info.outputPath("models-loading.png") });
  await key.fill("second-private-key");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__chatTest.pendingModelPreviews.length,
      ),
    )
    .toBe(2);
  await page.evaluate(() => {
    (window as any).__chatTest.pendingModelPreviews[1]({
      status: "completed",
      models: ["api-only-model", "another-api-model"],
    });
  });
  await expect(model).toBeEnabled();
  await expect(model).toHaveText("Choose a model");
  await expect(add).toBeDisabled();
  await page.evaluate(() => {
    (window as any).__chatTest.pendingModelPreviews[0]({
      status: "completed",
      models: ["stale-model"],
    });
  });
  await model.click();
  await expect(page.getByRole("option")).toHaveText([
    "Choose a model",
    "api-only-model",
    "another-api-model",
  ]);
  await page.screenshot({ path: info.outputPath("models-loaded.png") });
  await page
    .getByRole("option", { name: "api-only-model", exact: true })
    .click();
  await expect(add).toBeEnabled();
  expect(
    await page.evaluate(() => localStorage.getItem("chat-preferences")),
  ).toBeNull();
  await key.fill("");
  await expect(model).toHaveText("Enter an API key first");
  await expect(model).toBeDisabled();
  await expect(add).toBeDisabled();
  await key.fill("third-private-key");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__chatTest.pendingModelPreviews.length,
      ),
    )
    .toBe(3);
  await dialog.getByRole("button", { name: "Change provider" }).click();
  await dialog.getByRole("button", { name: "Anthropic", exact: true }).click();
  await dialog
    .getByRole("checkbox", { name: "Use for new conversations" })
    .check();
  await page.evaluate(() => {
    (window as any).__chatTest.pendingModelPreviews[2]({
      status: "completed",
      models: ["previous-provider-model"],
    });
  });
  await expect(key).toHaveValue("");
  await expect(model).toBeDisabled();
  await expect(model).toHaveText("Enter an API key first");
  await key.fill("fourth-private-key");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__chatTest.pendingModelPreviews.length,
      ),
    )
    .toBe(4);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.evaluate(() => {
    (window as any).__chatTest.pendingModelPreviews[3]({
      status: "completed",
      models: ["closed-provider-model"],
    });
  });
  await expect(dialog).toHaveCount(0);
  expect(
    await page.evaluate(() => localStorage.getItem("chat-preferences")),
  ).toBeNull();
});

test("model discovery errors and empty catalogs block saving and allow retry", async ({
  page,
}, info) => {
  await page.evaluate(() => {
    (window as any).__chatTest.failModelPreview = "auth";
  });
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog
    .getByRole("button", { name: "Google (AI Studio)", exact: true })
    .click();
  await dialog
    .getByRole("checkbox", { name: "Use for new conversations" })
    .check();
  await dialog
    .getByLabel("API key", { exact: true })
    .fill("private-catalog-key");
  const model = dialog.getByRole("combobox", { name: "Model", exact: true });
  const add = dialog.getByRole("button", { name: "Add provider", exact: true });
  await expect(dialog.getByRole("alert")).toContainText(
    "API key was not accepted",
  );
  await expect(model).toBeDisabled();
  await expect(add).toBeDisabled();
  await page.screenshot({ path: info.outputPath("models-error.png") });
  await page.evaluate(() => {
    (window as any).__chatTest.failModelPreview = false;
    (window as any).__chatTest.previewModels = [];
  });
  await dialog.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText(
    "No chat models are available",
  );
  await expect(model).toBeDisabled();
  await expect(add).toBeDisabled();
  await page.evaluate(() => {
    (window as any).__chatTest.previewModels = ["api-returned-model"];
  });
  await dialog.getByRole("button", { name: "Retry", exact: true }).click();
  await model.click();
  await page
    .getByRole("option", { name: "api-returned-model", exact: true })
    .click();
  await add.click();
  await expect(dialog).toHaveCount(0);
  const saved = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("chat-preferences")!),
  );
  expect(saved.connections[1].models).toEqual(["api-returned-model"]);
  expect(saved.defaults.model).toBe("api-returned-model");
  expect(JSON.stringify(saved)).not.toContain("private-catalog-key");
  expect(
    await page.evaluate(() => (window as any).__chatTest.connectionActions),
  ).toEqual([]);
});

test("custom selects save conversation defaults and disable models without a connection", async ({
  page,
}, info) => {
  const connection = page.getByRole("combobox", {
    name: "Connection",
    exact: true,
  });
  const model = page.getByRole("combobox", { name: "Model", exact: true });
  await connection.click();
  await page
    .getByRole("option", { name: "Choose connection", exact: true })
    .click();
  await expect(model).toBeDisabled();
  await expect(model).toHaveText("Choose a model");
  await connection.click();
  await page.getByRole("option", { name: "Fixture", exact: true }).click();
  await expect(model).toBeEnabled();
  await model.click();
  await page
    .getByRole("option", { name: "GPT-4.1 · Apr 2025", exact: true })
    .click();
  const sendMode = page.getByRole("combobox", { name: "Send message with" });
  await sendMode.click();
  await page
    .getByRole("option", {
      name: "Ctrl+Enter",
      exact: true,
    })
    .click();
  await expect(sendMode).toHaveAccessibleDescription("Enter adds a new line.");
  const tokens = page.getByRole("spinbutton", {
    name: "Maximum output tokens",
  });
  await tokens.fill("8192");
  await page
    .getByLabel("System instructions", { exact: true })
    .fill("Answer in Polish and keep replies concise.");
  await tokens.focus();
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.screenshot({
      path: info.outputPath(`chat-preferences-${colorScheme}.png`),
    });
  }
  await tokens.blur();
  await expect(page.getByRole("status")).toHaveText("Saved");
  for (const [width, height, zoom] of [
    [640, 700, 1],
    [800, 600, 2],
  ]) {
    await page.setViewportSize({ width, height });
    await page.evaluate((zoom) => {
      document.documentElement.style.zoom = String(zoom);
    }, zoom);
    await tokens.scrollIntoViewIfNeeded();
    expect(
      await page
        .locator(".chat-settings")
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBe(true);
    await expect(tokens).toBeInViewport();
    await page.screenshot({
      path: info.outputPath(`chat-preferences-${width}-${zoom}.png`),
    });
  }
  await page.reload();
  await expect(model).toHaveText("GPT-4.1 · Apr 2025");
  await expect(sendMode).toHaveText("Ctrl+Enter");
  await expect(
    page.getByLabel("System instructions", { exact: true }),
  ).toHaveValue("Answer in Polish and keep replies concise.");
  await expect(tokens).toHaveValue("8192");
  await tokens.fill("0");
  await tokens.blur();
  await expect(tokens).toHaveAccessibleDescription("Enter 1 to 32,768.");
  await page.reload();
  await expect(tokens).toHaveValue("8192");
  expect(
    await page.evaluate(() => (window as any).__chatTest.connectionActions),
  ).toEqual([]);
});
