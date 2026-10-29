import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test.beforeEach(async ({ page }) => {
  await mockDesktop(page, false, null);
  await page.goto("/?window=settings&page=terminal");
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
});

test("moving navigation focus preserves the active page and its unsaved draft", async ({
  page,
}) => {
  const fontSize = page.getByRole("spinbutton", {
    name: "Font size",
    exact: true,
  });
  await expect(fontSize).toBeEnabled();
  // Keep browser validation from refocusing the invalid draft on blur.
  await fontSize.evaluate((input: HTMLInputElement) => {
    input.reportValidity = () => false;
  });
  await fontSize.fill("0");
  const navigation = page.getByRole("navigation", { name: "Settings pages" });
  const terminal = navigation.getByRole("button", {
    name: "Terminal",
    exact: true,
  });
  await terminal.focus();
  await terminal.press("Home");
  const themes = navigation.getByRole("button", {
    name: "Themes",
    exact: true,
  });
  await expect(themes).toBeFocused();
  await themes.press("ArrowDown");
  await expect(
    navigation.getByRole("button", { name: "Keybinds", exact: true }),
  ).toBeFocused();
  await expect(terminal).toHaveAttribute("aria-current", "page");
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await expect(fontSize).toHaveValue("0");
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "save_terminal_preferences",
      ),
    ),
  ).toHaveLength(0);
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

test("disabled Remote ignores activation and native requests focus the requested page", async ({
  page,
}) => {
  const navigation = page.getByRole("navigation", { name: "Settings pages" });
  const remote = navigation.getByRole("button", {
    name: "Remote",
    exact: true,
  });
  await expect(remote).toBeDisabled();
  await remote.evaluate((button: HTMLButtonElement) => button.click());
  await expect(
    page.getByRole("heading", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await expect(
    navigation.getByRole("button", { name: "Terminal", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await page.evaluate(() =>
    (window as any).__TAURI_INTERNALS__.invoke("open_settings", {
      page: "keybinds",
    }),
  );
  await expect(
    page.getByRole("heading", { name: "Keybinds", exact: true }),
  ).toBeVisible();
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
  const editorShortcut = page.getByRole("button", {
    name: "Shortcut for Save file",
    exact: true,
  });
  await expect(editorShortcut).toBeHidden();
  await search.fill("Save file");
  await expect(editorShortcut).toBeVisible();
  await expect(page.locator(".shortcut-recorder")).toHaveCount(1);
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
