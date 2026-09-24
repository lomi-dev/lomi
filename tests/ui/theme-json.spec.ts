import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

const manifest = {
  version: 2,
  name: "JSON test",
  common: { tokens: { "--radius-control": "8px" } },
};
const text = (editor: Locator) =>
  editor
    .locator(".cm-line")
    .allTextContents()
    .then((lines) => lines.join("\n"));

async function open(page: Page) {
  await mockDesktop(page, false);
  await page.addInitScript((manifest) => {
    localStorage.setItem(
      "test-theme-manifests",
      JSON.stringify({ custom: manifest }),
    );
  }, manifest);
  await page.goto("/?window=settings&page=themes");
  await page.getByRole("button", { name: "Edit theme", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "Edit JSON", exact: true }).click();
  const editor = dialog.getByRole("textbox", {
    name: "Theme JSON",
    exact: true,
  });
  await expect(editor).toBeFocused();
  return { dialog, editor };
}

test("theme JSON uses the code editor, formats with undo, searches and saves through theme commands", async ({
  page,
}) => {
  const { dialog, editor } = await open(page);
  await expect
    .poll(() => editor.locator("span[class]").count())
    .toBeGreaterThan(0);
  await expect(dialog.locator(".cm-lineNumbers")).toBeVisible();
  await expect(dialog.locator(".cm-foldGutter")).toBeVisible();
  const compact = JSON.stringify({ ...manifest, name: "Edited JSON" });
  await editor.fill(compact);
  await dialog
    .getByRole("button", { name: "Format JSON", exact: true })
    .click();
  await expect
    .poll(() => text(editor))
    .toBe(JSON.stringify(JSON.parse(compact), null, 4));
  await editor.press("Control+z");
  await expect.poll(() => text(editor)).toBe(compact);
  await dialog.getByRole("button", { name: "Redo", exact: true }).click();
  await expect
    .poll(() => text(editor))
    .toBe(JSON.stringify(JSON.parse(compact), null, 4));
  await editor.press("Control+f");
  await dialog
    .getByRole("textbox", { name: "Find", exact: true })
    .fill("Edited JSON");
  await dialog
    .getByRole("textbox", { name: "Find", exact: true })
    .press("Enter");
  await expect(dialog.locator(".cm-searchMatch")).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  await expect(editor).toBeFocused();
  await editor.press("Control+s");
  await expect(dialog.getByRole("status")).toContainText("Theme saved");
  expect(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("test-theme-manifests")!).custom.name,
    ),
  ).toBe("Edited JSON");
  await dialog
    .getByRole("button", { name: "Show controls", exact: true })
    .click();
  await expect(dialog.getByLabel("Name", { exact: true })).toHaveValue(
    "Edited JSON",
  );
  await dialog.getByLabel("Name", { exact: true }).fill("Changed in controls");
  await dialog.getByRole("button", { name: "Edit JSON", exact: true }).click();
  await expect.poll(() => text(editor)).toContain("Changed in controls");
  await editor.press("Control+z");
  await expect.poll(() => text(editor)).toContain("Edited JSON");
  await editor.press("Control+z");
  await expect.poll(() => text(editor)).toBe(compact);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter((call: any) =>
        [
          "read_editor_file",
          "save_editor_file",
          "watch_editor_files",
          "start_terminal",
        ].includes(call.command),
      ),
    ),
  ).toHaveLength(0);
});

test("invalid JSON remains editable and live preferences keep the draft and undo history", async ({
  page,
}) => {
  const { dialog, editor } = await open(page);
  const invalid = '{"version":1,"name":';
  await editor.fill(invalid);
  await dialog
    .getByRole("button", { name: "Format JSON", exact: true })
    .click();
  await expect(dialog.getByRole("alert")).toBeVisible();
  await expect.poll(() => text(editor)).toBe(invalid);
  await dialog
    .getByRole("button", { name: "Show controls", exact: true })
    .click();
  await expect(editor).toBeVisible();
  await expect.poll(() => text(editor)).toBe(invalid);
  await editor.press("Control+s");
  await expect(dialog.getByRole("alert")).toBeVisible();
  await expect.poll(() => text(editor)).toBe(invalid);
  await page.evaluate(async () => {
    localStorage.setItem(
      "test-editor-preferences",
      JSON.stringify({ version: 1, tabSize: 8, insertSpaces: false }),
    );
    await (window as any).__nativeTest.emitEvent("editor-preferences-changed");
    localStorage.setItem(
      "test-keybindings",
      JSON.stringify({ version: 1, bindings: { saveFile: "Ctrl+Alt+KeyS" } }),
    );
    await (window as any).__nativeTest.emitEvent("keybindings-changed");
  });
  await expect(editor).toHaveCSS("tab-size", "8");
  await expect.poll(() => text(editor)).toBe(invalid);
  await editor.press("Control+z");
  await expect.poll(() => text(editor)).toBe(JSON.stringify(manifest, null, 2));
  const compact = JSON.stringify({ ...manifest, name: "Tab indents" });
  await editor.fill(compact);
  await dialog
    .getByRole("button", { name: "Format JSON", exact: true })
    .click();
  await expect
    .poll(() => text(editor))
    .toBe(JSON.stringify(JSON.parse(compact), null, "\t"));
  await editor.press("Control+Alt+s");
  await expect(dialog.getByRole("status")).toContainText("Theme saved");
  await editor.fill(invalid);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Keep editing", exact: true }).click();
  await expect.poll(() => text(editor)).toBe(invalid);
});

test("JSON editor scrolls within the modal in both appearances at the minimum window size", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "dark" });
  const { dialog, editor } = await open(page);
  await page.setViewportSize({ width: 560, height: 420 });
  for (const appearance of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: appearance });
    await expect(page.locator("html")).toHaveAttribute(
      "data-appearance",
      appearance,
    );
    await expect
      .poll(() =>
        dialog.locator(".cm-editor").evaluate((editor) => {
          const reference = document.createElement("span");
          reference.style.color = "var(--editor-foreground)";
          editor.append(reference);
          const matches =
            getComputedStyle(editor).color ===
            getComputedStyle(reference).color;
          reference.remove();
          return matches;
        }),
      )
      .toBe(true);
    await expect(
      dialog.getByRole("button", { name: "Format JSON", exact: true }),
    ).toBeInViewport();
    await expect(
      dialog.getByRole("button", { name: "Save theme", exact: true }),
    ).toBeInViewport();
    expect((await editor.boundingBox())!.width).toBeGreaterThan(200);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`theme-json-${appearance}.png`),
    });
  }
});
