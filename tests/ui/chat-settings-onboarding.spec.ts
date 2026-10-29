import { expect, test, type Locator, type Page } from "@playwright/test";
import { defaultConfig, type Connection } from "../../src/chat/types";
import { mockDesktop } from "./desktop";
import { mockChats } from "./chat-mock";

const fixture: Connection = {
  id: "onboarding-service",
  name: "Team OpenAI",
  provider: "openai",
  enabled: true,
  credentialRevision: 0,
  secretMode: "session",
  secretId: null,
  models: ["gpt-4.1"],
  testedModel: null,
  testStatus: null,
};

async function settings(page: Page, connections: Connection[] = []) {
  await mockDesktop(page, false);
  await page.addInitScript(
    ({ connections, defaults }) => {
      localStorage.setItem(
        "chat-preferences",
        JSON.stringify({
          version: 1,
          revision: 0,
          connections,
          defaults: connections.length
            ? {
                ...defaults,
                connectionId: connections[0].id,
                model: connections[0].models[0],
                configured: true,
              }
            : defaults,
          sendMode: "enter",
        }),
      );
    },
    { connections, defaults: defaultConfig },
  );
  await mockChats(page);
  await page.goto("/?window=settings&page=chat-ai");
  await expect(
    page.getByRole("heading", { name: "Chat AI", exact: true }),
  ).toBeVisible();
}

async function reachable(locator: Locator) {
  await expect(locator).toBeInViewport();
  const bounds = (await locator.boundingBox())!;
  const viewport = locator.page().viewportSize()!;
  expect(bounds.x).toBeGreaterThanOrEqual(0);
  expect(bounds.y).toBeGreaterThanOrEqual(0);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(viewport.width + 1);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(viewport.height + 1);
}

async function noPaidRequests(page: Page) {
  expect(
    await page.evaluate(() => ({
      starts: (window as any).__chatTest.starts,
      actions: (window as any).__chatTest.connectionActions,
    })),
  ).toEqual({ starts: 0, actions: [] });
}

const pageErrors = new WeakMap<Page, string[]>();
test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  pageErrors.set(page, errors);
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 800, height: 420 });
});
test.afterEach(async ({ page }) => {
  expect(pageErrors.get(page)).toEqual([]);
  for (const surface of [
    page.locator(".settings-page"),
    page.locator("dialog[open]"),
  ]) {
    if (await surface.count())
      expect(
        await surface.evaluate(
          (element) => element.scrollWidth - element.clientWidth,
        ),
      ).toBeLessThanOrEqual(1);
  }
  await noPaidRequests(page);
});

for (const mode of ["dark", "light"] as const) {
  test(`${mode} empty setup connects a service in a compact window without sending a request`, async ({
    page,
  }, info) => {
    await page.emulateMedia({ colorScheme: mode });
    await settings(page);
    const empty = page.locator(".chat-providers-empty");
    const connect = empty.getByRole("button", {
      name: "Connect AI service",
      exact: true,
    });
    await expect(
      page.getByRole("button", { name: "Connect AI service", exact: true }),
    ).toHaveCount(1);
    await expect(
      page.getByRole("region", { name: "Default for new chats" }),
    ).toHaveCount(0);
    await reachable(connect);
    await page.screenshot({
      path: info.outputPath(`chat-setup-empty-${mode}.png`),
    });
    await connect.click();
    const chooser = page.getByRole("dialog", {
      name: "Choose an AI service",
      exact: true,
    });
    await reachable(chooser);
    await chooser.getByRole("button", { name: "OpenAI", exact: true }).click();
    const dialog = page.getByRole("dialog", {
      name: "Connect OpenAI",
      exact: true,
    });
    const model = dialog.getByRole("combobox", { name: "Model", exact: true });
    await expect(model).toBeDisabled();
    await expect(
      dialog.getByText(
        "Your key will be saved in your system credential store.",
        { exact: true },
      ),
    ).toBeVisible();
    await dialog
      .getByLabel("API key", { exact: true })
      .fill("fixture-not-a-real-key");
    await expect(model).toBeEnabled();
    await model.click();
    await page.getByRole("option", { name: "GPT-4.1", exact: true }).click();
    const save = dialog.getByRole("button", {
      name: "Save connection",
      exact: true,
    });
    await expect(save).toBeEnabled();
    await reachable(save);
    await reachable(
      dialog.getByRole("button", { name: "Cancel", exact: true }),
    );
    await page.screenshot({
      path: info.outputPath(`chat-connect-${mode}.png`),
    });
    await save.click();
    await expect(dialog).toHaveCount(0);
    await expect(
      page.getByRole("list", { name: "Your AI services" }),
    ).toContainText("OpenAI");
    await expect(
      page
        .getByRole("list", { name: "Your AI services" })
        .getByRole("button", { name: /OpenAI.*Manage/ }),
    ).toBeFocused();
    const defaults = page.getByRole("region", {
      name: "Default for new chats",
      exact: true,
    });
    await expect(
      defaults.getByRole("combobox", { name: "Default model", exact: true }),
    ).toBeEnabled();
    expect(
      await page.evaluate(
        () => JSON.parse(localStorage.getItem("chat-preferences")!).defaults,
      ),
    ).toMatchObject({ model: "gpt-4.1", configured: true });
  });
}

test("a missing key explains the unavailable default and opens direct key editing", async ({
  page,
}, info) => {
  await settings(page, [fixture]);
  const defaults = page.getByRole("region", {
    name: "Default for new chats",
    exact: true,
  });
  await expect(defaults.getByRole("note")).toContainText(
    "Team OpenAI needs an API key",
  );
  await expect(
    defaults.getByRole("combobox", { name: "Default model", exact: true }),
  ).toBeDisabled();
  const addKey = page.getByRole("button", {
    name: "Add API key for Team OpenAI",
    exact: true,
  });
  await addKey.click();
  const dialog = page.getByRole("dialog", { name: "Edit OpenAI", exact: true });
  await expect(dialog.getByLabel("API key", { exact: true })).toBeVisible();
  await expect(dialog.getByLabel("API key", { exact: true })).toHaveValue("");
  await reachable(dialog.getByRole("button", { name: "Cancel", exact: true }));
  await page.screenshot({ path: info.outputPath("chat-missing-key.png") });
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(addKey).toBeFocused();
  await expect(
    defaults.getByRole("combobox", { name: "Default model", exact: true }),
  ).toBeDisabled();
  await addKey.click();
  await dialog
    .getByLabel("API key", { exact: true })
    .fill("fixture-not-a-real-key");
  await dialog
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page
      .getByRole("list", { name: "Your AI services" })
      .getByRole("button", { name: /Team OpenAI.*Manage/ }),
  ).toBeFocused();
  await expect(
    defaults.getByRole("combobox", { name: "Default model", exact: true }),
  ).toBeEnabled();
});

test("a disabled default explains how to restore access through Manage", async ({
  page,
}, info) => {
  await settings(page, [
    { ...fixture, enabled: false, secretId: "fixture-session-key" },
  ]);
  const defaults = page.getByRole("region", {
    name: "Default for new chats",
    exact: true,
  });
  await expect(defaults.getByRole("note")).toContainText(
    "Team OpenAI is turned off",
  );
  await expect(
    defaults.getByRole("combobox", { name: "AI service", exact: true }),
  ).toContainText("Team OpenAI");
  await expect(
    defaults.getByRole("combobox", { name: "Default model", exact: true }),
  ).toBeDisabled();
  const manage = page
    .getByRole("list", { name: "Your AI services" })
    .getByRole("button", { name: /Team OpenAI.*Manage/ });
  await expect(manage).toHaveAttribute("aria-expanded", "false");
  await manage.click();
  await expect(manage).toHaveAttribute("aria-expanded", "true");
  const enabled = page.getByRole("switch", { name: "Enable Team OpenAI" });
  await expect(enabled).not.toBeChecked();
  await page.screenshot({ path: info.outputPath("chat-disabled-default.png") });
  await enabled.check();
  await expect(defaults.getByRole("note")).toHaveCount(0);
  await expect(
    defaults.getByRole("combobox", { name: "Default model", exact: true }),
  ).toBeEnabled();
});

test("a keyless custom service keeps usable defaults and saves a different local model", async ({
  page,
}, info) => {
  await settings(page, [
    {
      ...fixture,
      name: "Local server",
      provider: "custom",
      baseUrl: "http://localhost:11434/v1",
      apiFormat: "chat-completions",
      models: ["local-one", "local-two"],
    },
  ]);
  const defaults = page.getByRole("region", {
    name: "Default for new chats",
    exact: true,
  });
  await expect(defaults.getByRole("note")).toHaveCount(0);
  const model = defaults.getByRole("combobox", {
    name: "Default model",
    exact: true,
  });
  await expect(model).toBeEnabled();
  await model.click();
  await page.getByRole("option", { name: "local-two", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("chat-preferences")!).defaults.model,
      ),
    )
    .toBe("local-two");
  await expect(
    page.getByRole("button", {
      name: "Add API key for Local server",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("list", { name: "Your AI services" }),
  ).toContainText("Requests sent without authentication");
  await page.screenshot({ path: info.outputPath("chat-keyless-custom.png") });
});
