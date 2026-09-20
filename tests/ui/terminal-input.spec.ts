import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function input(page: Page) {
  return page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter((call: any) => call.command === "write_terminal")
      .map((call: any) => call.args.data)
      .join(""),
  );
}

test("Windows paste shortcuts use the native clipboard once and preserve bracketed Unicode text", async ({
  page,
}) => {
  await mockDesktop(page, false, undefined, undefined, {}, "windows");
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await page.evaluate(async () => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(
      document.querySelector<HTMLElement>("[data-pane-id]")!.dataset.paneId!,
    )!;
    await new Promise<void>((resolve) =>
      runtime.terminal.write("\x1b[?2004h", resolve),
    );
    const native = (window as any).__TAURI_INTERNALS__;
    const invoke = native.invoke;
    native.invoke = (command: string, args: unknown) => {
      if (command === "paste_terminal_clipboard") {
        (window as any).__nativeTest.clipboardReads =
          ((window as any).__nativeTest.clipboardReads ?? 0) + 1;
        return Promise.resolve("zażółć 🦀\r\nsecond line");
      }
      return invoke(command, args);
    };
  });
  const pasted = "\x1b[200~zażółć 🦀\rsecond line\x1b[201~";
  let count = 0;
  for (const shortcut of ["Control+v", "Control+Shift+v", "Shift+Insert"]) {
    await page.keyboard.press(shortcut);
    await expect.poll(() => input(page)).toBe(pasted.repeat(++count));
  }
  await page.locator(".xterm-helper-textarea").evaluate((textarea) => {
    for (const state of [{ repeat: true }, { isComposing: true, keyCode: 229 }])
      textarea.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "v",
          code: "KeyV",
          ctrlKey: true,
          bubbles: true,
          cancelable: true,
          ...state,
        }),
      );
  });
  await page.keyboard.press("Control+c");
  await expect.poll(() => input(page)).toBe(pasted.repeat(count) + "\x03");
  expect(
    await page.evaluate(() => (window as any).__nativeTest.clipboardReads),
  ).toBe(3);
  await page.keyboard.press("Control+Shift+i");
  const composer = page.getByRole("textbox", { name: "Command input" });
  await expect(composer).toBeFocused();
  await composer.press("Control+v");
  expect(
    await page.evaluate(() => (window as any).__nativeTest.clipboardReads),
  ).toBe(3);
});

test("configured Windows shortcuts take precedence over native paste aliases", async ({
  page,
}) => {
  await mockDesktop(page, false, undefined, undefined, {}, "windows");
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-keybindings",
      JSON.stringify({
        version: 1,
        bindings: { newTerminal: "Ctrl+KeyV", pasteTerminal: "Ctrl+KeyY" },
      }),
    );
  });
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await page.keyboard.press("Control+v");
  await expect(page.locator("[data-pane-id]")).toHaveCount(2);
  expect(await input(page)).toBe("");
  await page.keyboard.press("Control+y");
  await expect.poll(() => input(page)).toBe("clipboard text");
});

test("Linux Ctrl+V remains terminal input", async ({ page }) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await page.keyboard.press("Control+v");
  await expect.poll(() => input(page)).toBe("\x16");
  await page.keyboard.press("Control+Shift+v");
  await expect.poll(() => input(page)).toBe("\x16clipboard text");
});

test("a blocked paste can be cancelled or closed without draining the remaining text", async ({
  page,
}) => {
  await mockDesktop(page, false, undefined, undefined, {}, "windows");
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  const paneId = await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id");
  await page.evaluate(() => {
    const native = (window as any).__TAURI_INTERNALS__;
    const invoke = native.invoke;
    native.invoke = async (command: string, args: unknown) => {
      if (command === "paste_terminal_clipboard") return "x".repeat(50000);
      const result = invoke(command, args);
      if (command === "write_terminal") {
        await new Promise<void>((resolve) => {
          (window as any).__releaseWrite = resolve;
        });
      }
      return result;
    };
  });
  await page.keyboard.press("Control+v");
  await expect.poll(() => input(page)).toHaveLength(16384);
  await page.keyboard.press("Control+w");
  const dialog = page.getByRole("dialog", { name: "Close running processes?" });
  await expect(dialog).toContainText("Terminal input is still pending", {
    timeout: 2000,
  });
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.locator("[data-pane-id]")).toHaveCount(1);
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+w");
  await expect(dialog).toBeVisible({ timeout: 2000 });
  await dialog
    .getByRole("button", { name: "Close anyway", exact: true })
    .click();
  await expect(page.locator(`[data-pane-id="${paneId}"]`)).toHaveCount(0);
  await page.evaluate(() => (window as any).__releaseWrite());
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "close_terminal",
          ).length,
      ),
    )
    .toBe(1);
  expect(await input(page)).toHaveLength(16384);
});

test("resizing coalesces pending native calls and delivers the final dimensions in order", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await page.evaluate(() => {
    const native = (window as any).__TAURI_INTERNALS__;
    const invoke = native.invoke;
    (window as any).__resizes = [];
    native.invoke = async (command: string, args: any) => {
      if (command === "resize_terminal") {
        (window as any).__resizes.push({ cols: args.cols, rows: args.rows });
        await new Promise<void>((resolve) => {
          (window as any).__releaseResize = resolve;
        });
      }
      return invoke(command, args);
    };
  });
  const dimensions = () =>
    page.evaluate(async () => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      const runtime = runningTerminal(
        document.querySelector<HTMLElement>("[data-pane-id]")!.dataset.paneId!,
      )!;
      return { cols: runtime.terminal.cols, rows: runtime.terminal.rows };
    });
  let last = await dimensions();
  for (const width of [1200, 1100, 1000]) {
    await page.setViewportSize({ width, height: 800 });
    await expect.poll(dimensions).not.toEqual(last);
    last = await dimensions();
  }
  const resizes = () => page.evaluate(() => (window as any).__resizes);
  expect(await resizes()).toHaveLength(1);
  await page.evaluate(() => (window as any).__releaseResize());
  await expect.poll(resizes).toHaveLength(2);
  expect((await resizes())[1]).toEqual(last);
  await page.evaluate(() => (window as any).__releaseResize());
});

test("WebGL follows CLI cursor blink changes without moving the cursor", async ({
  page,
}, testInfo) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  const sample = (sequence: string) =>
    page.evaluate(async (sequence) => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      const runtime = runningTerminal(
        document.querySelector<HTMLElement>("[data-pane-id]")!.dataset.paneId!,
      )!;
      const renderer = (runtime.terminal as any)._core._renderService._renderer
        .value;
      if (runtime.getSnapshot().renderer !== "WebGL")
        throw new Error("WebGL unavailable");
      await new Promise<void>((resolve) =>
        runtime.terminal.write(sequence, resolve),
      );
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
      const styles: (string | null)[] = [];
      for (let i = 0; i < 20; i++) {
        styles.push(renderer._model.cursor?.style ?? null);
        await new Promise((resolve) => setTimeout(resolve, 80));
      }
      return styles;
    }, sequence);
  expect(await sample("\x1b[6 q")).toEqual(Array(20).fill("bar"));
  await page.screenshot({
    path: testInfo.outputPath("terminal-steady-cursor.png"),
  });
  const blinking = await sample("\x1b[5 q");
  expect(blinking).toContain("bar");
  expect(blinking).toContain(null);
  expect(await sample("\x1b[4 q")).toEqual(Array(20).fill("underline"));
  const reset = await sample("\x1b[0 q");
  expect(reset).toContain("bar");
  expect(reset).toContain(null);
});
