import { expect, test } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

for (const mode of ["dark", "light"] as const) {
  test(`Lomi ${mode} dialogs retain keyboard guards and fit a compact window`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({ colorScheme: mode });
    await mockDesktop(page);
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__nativeTest.sessions.size),
      )
      .toBe(1);
    const pane = (await page
      .locator("[data-pane-id]")
      .first()
      .getAttribute("data-pane-id"))!;
    await page.evaluate(() => {
      const native = (window as any).__nativeTest;
      const id = [...native.sessions.keys()][0];
      native.busyTerminals = [id];
      native.emit(
        id,
        "\x1b[2J\x1b[H\x1b[1mLomi workspace\x1b[0m\r\n\r\n\x1b[32m✓\x1b[0m Dependencies installed\r\n\x1b[32m✓\x1b[0m Application ready\r\n\r\n\x1b[34m~/workspace\x1b[0m  main\r\n$ pnpm dev\r\n\r\n  Local:   http://localhost:1420/\r\n  Ready in 240 ms\r\n",
      );
    });
    await expect.poll(() => buffer(page, pane)).toContain("Ready in 240 ms");
    await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
    await expect(page.locator(".terminal-pane")).toHaveCSS(
      "background-color",
      mode === "dark" ? "rgb(24, 26, 31)" : "rgb(255, 255, 255)",
    );
    await page.screenshot({
      path: testInfo.outputPath(`terminal-${mode}.png`),
    });
    await page.getByRole("button", { name: "Close window" }).click();
    const dialog = page.getByRole("dialog", { name: "Quit Lomi?" });
    await expect(
      dialog.getByRole("button", { name: "Cancel", exact: true }),
    ).toBeFocused();
    await expect(dialog.locator(".modal-symbol")).toBeVisible();
    await expect(
      dialog.getByRole("button", { name: "Quit anyway" }),
    ).not.toHaveCSS("background-color", "rgb(200, 255, 61)");
    await page.screenshot({ path: testInfo.outputPath(`alert-${mode}.png`) });
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "close_terminal",
        ),
      ),
    ).toHaveLength(0);

    await page.getByRole("button", { name: "README.md", exact: true }).click();
    await page.locator(".cm-content").fill("Keep this unsaved work");
    await page.keyboard.press("Control+w");
    const save = page.getByRole("dialog", {
      name: "Save changes before closing?",
    });
    await expect(
      save.getByRole("button", { name: "Save changes", exact: true }),
    ).toBeFocused();
    await page.screenshot({ path: testInfo.outputPath(`save-${mode}.png`) });
    await page.setViewportSize({ width: 560, height: 420 });
    for (const name of ["Cancel", "Discard changes", "Save changes"]) {
      await expect(
        save.getByRole("button", { name, exact: true }),
      ).toBeInViewport();
    }
    const bounds = await save.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.y).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(560);
    await page.screenshot({
      path: testInfo.outputPath(`save-compact-${mode}.png`),
    });
    await page.keyboard.press("Escape");
    await expect(save).toHaveCount(0);
    await expect(page.locator(".cm-content")).toHaveText(
      "Keep this unsaved work",
    );
  });
}
