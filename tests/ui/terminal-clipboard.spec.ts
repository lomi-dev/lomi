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

async function clipboard(page: Page, delayed = false) {
  await page.evaluate(async (delayed) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(
      document.querySelector<HTMLElement>("[data-pane-id]")!.dataset.paneId!,
    )!;
    await new Promise<void>((resolve) =>
      runtime.terminal.write("\x1b[?2004h", resolve),
    );
    const native = (window as any).__TAURI_INTERNALS__;
    const invoke = native.invoke;
    (window as any).__clipboardReads = [];
    native.invoke = async (command: string, args: any) => {
      if (command !== "paste_terminal_clipboard") return invoke(command, args);
      (window as any).__clipboardReads.push(args.id);
      if (delayed)
        await new Promise<void>((resolve, reject) => {
          (window as any).__finishClipboard = resolve;
          (window as any).__failClipboard = reject;
        });
      return "'/private/clipboard images/image-test.png' ";
    };
  }, delayed);
}

const pasted = "\x1b[200~'/private/clipboard images/image-test.png' \x1b[201~";

for (const [platform, shortcuts] of [
  ["macos", ["Meta+v", "Meta+Shift+v"]],
  ["windows", ["Control+v", "Control+Shift+v", "Shift+Insert"]],
  ["linux", ["Control+Shift+v"]],
] as const) {
  test(`${platform} pastes image paths once without submitting`, async ({
    page,
  }) => {
    await mockDesktop(page, false, undefined, undefined, {}, platform);
    await page.goto("/");
    await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
    await clipboard(page);
    let count = 0;
    for (const shortcut of shortcuts) {
      await page.keyboard.press(shortcut);
      await expect.poll(() => input(page)).toBe(pasted.repeat(++count));
    }
    const reads = await page.evaluate(() => (window as any).__clipboardReads);
    expect(reads).toHaveLength(shortcuts.length);
    expect(new Set(reads).size).toBe(1);
  });
}

test("native menu paste uses image contents instead of the webview text representation", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await clipboard(page);
  await page.locator(".xterm-helper-textarea").evaluate((textarea) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", "https://example.test/image.png");
    textarea.dispatchEvent(
      new ClipboardEvent("paste", {
        clipboardData,
        bubbles: true,
        cancelable: true,
      }),
    );
  });
  await expect.poll(() => input(page)).toBe(pasted);
  expect(
    await page.evaluate(() => (window as any).__clipboardReads.length),
  ).toBe(1);
  await page.keyboard.press("Control+Shift+i");
  await page
    .getByRole("textbox", { name: "Command input" })
    .evaluate((textarea) => {
      textarea.dispatchEvent(
        new ClipboardEvent("paste", { bubbles: true, cancelable: true }),
      );
    });
  expect(
    await page.evaluate(() => (window as any).__clipboardReads.length),
  ).toBe(1);
});

test("image preparation reserves its position before typing and Enter", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await clipboard(page, true);
  await page.keyboard.press("Control+Shift+v");
  await expect
    .poll(() => page.evaluate(() => !!(window as any).__finishClipboard))
    .toBe(true);
  await page.keyboard.type("describe this");
  await page.keyboard.press("Enter");
  expect(await input(page)).toBe("");
  await page.evaluate(() => (window as any).__finishClipboard());
  await expect.poll(() => input(page)).toBe(pasted + "describe this\r");
});

test("native image paste runs once between preceding and following terminal input", async ({
  page,
}) => {
  await mockDesktop(page, false, undefined, undefined, {}, "macos");
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await page.evaluate(() => {
    const native = (window as any).__TAURI_INTERNALS__;
    const invoke = native.invoke;
    (window as any).__inputOrder = [];
    native.invoke = async (command: string, args: any) => {
      if (command === "paste_terminal_clipboard") {
        (window as any).__inputOrder.push("native image");
        return null;
      }
      if (command === "write_terminal") {
        (window as any).__inputOrder.push(args.data);
        if (args.data === "a")
          await new Promise<void>((resolve) => {
            (window as any).__releaseInput = resolve;
          });
      }
      return invoke(command, args);
    };
  });
  await page.keyboard.type("a");
  await expect
    .poll(() => page.evaluate(() => !!(window as any).__releaseInput))
    .toBe(true);
  await page.keyboard.press("Meta+v");
  await page.keyboard.type("b");
  expect(await page.evaluate(() => (window as any).__inputOrder)).toEqual([
    "a",
  ]);
  await page.evaluate(() => (window as any).__releaseInput());
  await expect
    .poll(() => page.evaluate(() => (window as any).__inputOrder))
    .toEqual(["a", "native image", "b"]);
  expect(await input(page)).toBe("ab");
});

test("a failed clipboard read reports the error and releases later input", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await clipboard(page, true);
  await page.keyboard.press("Control+Shift+v");
  await expect
    .poll(() => page.evaluate(() => !!(window as any).__failClipboard))
    .toBe(true);
  await page.keyboard.type("still typing");
  await page.evaluate(() =>
    (window as any).__failClipboard("Cannot save clipboard image: disk full"),
  );
  await expect(page.getByRole("alert")).toContainText(
    "Cannot save clipboard image: disk full",
  );
  await expect.poll(() => input(page)).toBe("still typing");
});

test("closing a terminal during image preparation cannot paste into its replacement", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await clipboard(page, true);
  await page.keyboard.press("Control+Shift+v");
  await expect
    .poll(() => page.evaluate(() => !!(window as any).__finishClipboard))
    .toBe(true);
  await page.keyboard.press("Control+w");
  const dialog = page.getByRole("dialog", { name: "Close running processes?" });
  await expect(dialog).toContainText("Terminal input is still pending");
  await dialog
    .getByRole("button", { name: "Close anyway", exact: true })
    .click();
  await page.evaluate(() => (window as any).__finishClipboard());
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
  expect(await input(page)).toBe("");
});
