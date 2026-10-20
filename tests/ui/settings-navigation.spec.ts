import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test.beforeEach(async ({ page }) => {
  await mockDesktop(page, false, null);
  await page.goto("/?window=settings&page=terminal");
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
});

test("page search preserves the active page and its unsaved draft", async ({
  page,
}) => {
  const search = page.getByRole("textbox", { name: "Find a settings page" });
  const fontSize = page.getByRole("spinbutton", {
    name: "Font size",
    exact: true,
  });
  await expect(fontSize).toBeEnabled();
  // Keep the browser validation popup from refocusing the invalid draft.
  await fontSize.evaluate((input: HTMLInputElement) => {
    input.reportValidity = () => false;
  });
  await fontSize.fill("0");
  await search.fill("appearance");
  const navigation = page.getByRole("navigation", { name: "Settings pages" });
  await expect(navigation.locator(".settings-nav-item")).toHaveText(["Themes"]);
  await expect(search).toBeFocused();
  await expect(fontSize).toHaveValue("0");
  await search.fill("no page has these words");
  await expect(navigation.getByRole("status")).toHaveText("No matching pages.");
  await expect(fontSize).toHaveValue("0");
  await navigation
    .getByRole("button", { name: "Clear search", exact: true })
    .click();
  await expect(search).toHaveValue("");
  await expect(search).toBeFocused();
  await expect(fontSize).toHaveValue("0");
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "save_terminal_preferences",
      ),
    ),
  ).toHaveLength(0);
});

test("search keyboard actions navigate matches and Escape keeps search focused", async ({
  page,
}) => {
  const navigation = page.getByRole("navigation", { name: "Settings pages" });
  const search = navigation.getByRole("textbox", {
    name: "Find a settings page",
  });
  const themes = navigation.getByRole("button", {
    name: "Themes",
    exact: true,
  });
  await search.fill("dark");
  await search.press("Escape");
  await expect(search).toHaveValue("");
  await expect(search).toBeFocused();
  await search.fill("terminal");
  await search.press("Enter");
  await expect(search).toHaveValue("");
  await expect(
    navigation.getByRole("button", { name: "Terminal", exact: true }),
  ).toBeFocused();
  await search.fill("dark");
  await search.press("ArrowDown");
  await expect(themes).toBeFocused();
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await search.focus();
  await search.press("Enter");
  await expect(
    page.getByRole("heading", { name: "Themes", exact: true }),
  ).toBeVisible();
  await expect(themes).toBeFocused();
  await expect(search).toHaveValue("");
});

test("navigation keys move focus without changing pages and skip unavailable Remote", async ({
  page,
}) => {
  const navigation = page.getByRole("navigation", { name: "Settings pages" });
  const terminal = navigation.getByRole("button", {
    name: "Terminal",
    exact: true,
  });
  const themes = navigation.getByRole("button", {
    name: "Themes",
    exact: true,
  });
  const about = navigation.getByRole("button", { name: "About", exact: true });
  const account = navigation.getByRole("button", {
    name: "Account",
    exact: true,
  });
  const agent = navigation.getByRole("button", {
    name: "Agent control",
    exact: true,
  });
  await expect(terminal).toBeFocused();
  await terminal.press("Home");
  await expect(themes).toBeFocused();
  await themes.press("End");
  await expect(about).toBeFocused();
  await about.press("ArrowUp");
  await expect(agent).toBeFocused();
  await agent.press("ArrowUp");
  await expect(account).toBeFocused();
  await account.press("ArrowDown");
  await expect(agent).toBeFocused();
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await agent.press("Home");
  await themes.press("Space");
  await expect(
    page.getByRole("heading", { name: "Themes", exact: true }),
  ).toBeVisible();
});

test("filtered Remote remains disabled and native requests clear a hiding search", async ({
  page,
}) => {
  const navigation = page.getByRole("navigation", { name: "Settings pages" });
  const search = navigation.getByRole("textbox", {
    name: "Find a settings page",
  });
  await search.fill("remote");
  await expect(navigation.locator(".settings-nav-item")).toHaveText(["Remote"]);
  await expect(
    navigation.getByRole("button", { name: "Remote", exact: true }),
  ).toBeDisabled();
  await search.press("ArrowDown");
  await expect(search).toBeFocused();
  await search.press("Enter");
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await page.evaluate(() =>
    (window as any).__TAURI_INTERNALS__.invoke("open_settings", {
      page: "keybinds",
    }),
  );
  await expect(
    page.getByRole("heading", { name: "Keybinds", exact: true }),
  ).toBeVisible();
  await expect(search).toHaveValue("");
  await expect(
    navigation.getByRole("button", { name: "Keybinds", exact: true }),
  ).toBeFocused();
});

test("native page requests reveal the focused category in a compact sidebar", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  const navigation = page.getByRole("navigation", { name: "Settings pages" });
  for (const [id, label] of [
    ["about", "About"],
    ["android", "Android"],
    ["keybinds", "Keybinds"],
  ]) {
    await page.evaluate(
      (id) =>
        (window as any).__TAURI_INTERNALS__.invoke("open_settings", {
          page: id,
        }),
      id,
    );
    const category = navigation.getByRole("button", {
      name: label,
      exact: true,
    });
    await expect(category).toBeFocused();
    await expect(category).toBeInViewport();
  }
});

test("shortcut search filters actions and clears without changing bindings", async ({
  page,
}) => {
  await page
    .getByRole("navigation", { name: "Settings pages" })
    .getByRole("button", { name: "Keybinds", exact: true })
    .click();
  const search = page.getByRole("textbox", {
    name: "Search shortcuts",
    exact: true,
  });
  const recorder = page.getByRole("button", {
    name: "Shortcut for New terminal",
    exact: true,
  });
  await expect(recorder).toBeEnabled();
  const binding = await recorder.textContent();
  await search.fill("New terminal side");
  await expect(page.locator(".shortcut-recorder")).toHaveCount(1);
  await expect(recorder).toHaveText(binding!);
  await expect(
    page.getByRole("switch", { name: "Focus follows pointer" }),
  ).toHaveCount(0);
  await search.fill("no shortcut has these words");
  await expect(
    page.locator(".settings-shortcut-empty").getByRole("status"),
  ).toHaveText("No matching shortcuts.");
  await page
    .getByRole("button", { name: "Clear shortcut search", exact: true })
    .click();
  await expect(search).toBeFocused();
  await expect(search).toHaveValue("");
  await expect(recorder).toHaveText(binding!);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "save_keybindings",
      ),
    ),
  ).toHaveLength(0);
});

test("recording a numpad shortcut keeps the plus key in one keycap", async ({
  page,
}) => {
  await page
    .getByRole("navigation", { name: "Settings pages" })
    .getByRole("button", { name: "Keybinds", exact: true })
    .click();
  const recorder = page.getByRole("button", {
    name: "Shortcut for New terminal",
    exact: true,
  });
  await expect(recorder).toBeEnabled();
  await recorder.click();
  await recorder.press("Control+NumpadAdd");
  await expect(recorder).toHaveText("Ctrl+Numpad +");
  await expect(recorder.locator("kbd")).toHaveText(["Ctrl", "Numpad +"]);
  await page.reload();
  await page
    .getByRole("navigation", { name: "Settings pages" })
    .getByRole("button", { name: "Keybinds", exact: true })
    .click();
  await expect(recorder).toHaveText("Ctrl+Numpad +");
});
