import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

const output =
  "\x1b[?25l\x1b[2J\x1b[H" +
  [
    "\x1b[0mNORMAL TEXT HHHHH",
    "\x1b[37mANSI WHITE HHHHH",
    "\x1b[1;37mBOLD WHITE HHHHH",
    "\x1b[97mBRIGHT WHITE HHHHH",
    "\x1b[38;2;255;255;255mRGB WHITE HHHHH",
    "\x1b[38;5;255mINDEXED WHITE HHHHH",
    "\x1b[2mDIM TEXT HHHHH",
    "\x1b[48;2;10;25;40;38;2;255;255;255mEXPLICIT BACKGROUND HHHHH",
    "\x1b[7mINVERSE TEXT HHHHH",
  ]
    .map((line) => line + "\x1b[0m")
    .join("\r\n");

async function terminal(page: Page, pane: string) {
  return page.evaluate(async (pane) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(pane)!;
    return {
      id: runtime.sessionId,
      renderer: runtime.getSnapshot().renderer,
      foreground: runtime.terminal.options.theme?.foreground,
      background: runtime.terminal.options.theme?.background,
      rows: runtime.terminal.rows,
      cols: runtime.terminal.cols,
    };
  }, pane);
}

async function paintedRows(page: Page, pane: string) {
  const { rows, cols } = await terminal(page, pane);
  const png = await page.locator(".xterm-screen").screenshot();
  return page.evaluate(
    async ({ png, rows, cols }) => {
      const image = new Image();
      image.src = `data:image/png;base64,${png}`;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.width;
      canvas.height = image.height;
      const context = canvas.getContext("2d")!;
      context.drawImage(image, 0, 0);
      // Sample the rendered glyphs, not xterm's configured colors: contrast correction
      // happens in the renderer and can change those colors before they reach the screen.
      return Array.from({ length: 9 }, (_, row) => {
        const top = Math.ceil((row * image.height) / rows);
        const bottom = Math.floor(((row + 1) * image.height) / rows);
        const pixels = context.getImageData(
          0,
          top,
          Math.floor((26 * image.width) / cols),
          bottom - top,
        ).data;
        let darkest = 255;
        let lightest = 0;
        for (let index = 0; index < pixels.length; index += 4) {
          const brightness =
            (pixels[index] + pixels[index + 1] + pixels[index + 2]) / 3;
          darkest = Math.min(darkest, brightness);
          lightest = Math.max(lightest, brightness);
        }
        return { darkest, lightest };
      });
    },
    { png: png.toString("base64"), rows, cols },
  );
}

async function expectLightText(page: Page, pane: string) {
  await expect
    .poll(async () => (await terminal(page, pane)).foreground)
    .toBe("#0b0d0cff");
  await expect
    .poll(async () => {
      const rows = await paintedRows(page, pane);
      return rows.slice(0, 4).map((row) => row.darkest < 60);
    })
    .toEqual([true, true, true, true]);
  const rows = await paintedRows(page, pane);
  expect(rows[4].darkest).toBeLessThan(130);
  expect(rows[5].darkest).toBeLessThan(130);
  expect(rows[6].darkest).toBeLessThan(170);
  expect(rows[7].lightest).toBeGreaterThan(235);
  expect(rows[8].lightest).toBeGreaterThan(230);
  expect((await terminal(page, pane)).background).toBe("#ffffff00");
}

for (const renderer of ["WebGL", "DOM"] as const) {
  test(`${renderer} keeps normal and ANSI text readable after light mode changes and tab switches`, async ({
    page,
  }, testInfo) => {
    if (renderer === "DOM")
      await page.addInitScript(() => {
        const getContext = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (
          kind: string,
          ...args: any[]
        ) {
          return kind === "webgl2"
            ? null
            : (getContext as any).call(this, kind, ...args);
        } as typeof getContext;
      });
    await page.emulateMedia({ colorScheme: "light" });
    await mockDesktop(page);
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    const first = (await page
      .locator("[data-pane-id]")
      .getAttribute("data-pane-id"))!;
    await expect
      .poll(async () => (await terminal(page, first)).renderer)
      .toBe(renderer);
    const before = await terminal(page, first);
    await page.evaluate(
      ({ id, output }) => (window as any).__nativeTest.emit(id, output),
      { id: before.id, output },
    );
    await expect.poll(() => buffer(page, first)).toContain("INDEXED WHITE");
    await expectLightText(page, first);
    await page.evaluate(
      (id) => (window as any).__nativeTest.emit(id, "\x1b]11;?\x07"),
      before.id,
    );
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "write_terminal")
            .map((call: any) => call.args.data)
            .join(""),
        ),
      )
      .toContain("rgb:ffff/ffff/ffff");
    await page.emulateMedia({ colorScheme: "dark" });
    await expect
      .poll(async () => (await terminal(page, first)).foreground)
      .toBe("#f3f4f6ff");
    await expect
      .poll(async () => (await paintedRows(page, first))[0].lightest)
      .toBeGreaterThan(200);
    await page.locator(".xterm-helper-textarea").focus();
    await page.keyboard.press("Control+Shift+t");
    const second = (await page
      .locator("[data-pane-id]")
      .getAttribute("data-pane-id"))!;
    expect(second).not.toBe(first);
    await expect
      .poll(async () => (await terminal(page, second)).renderer)
      .toBe(renderer);
    const other = await terminal(page, second);
    await page.evaluate(
      ({ id, output }) => (window as any).__nativeTest.emit(id, output),
      { id: other.id, output },
    );
    await expect.poll(() => buffer(page, second)).toContain("INDEXED WHITE");
    await page.emulateMedia({ colorScheme: "light" });
    await expectLightText(page, second);
    await page.locator(".tab").first().click();
    await expect(page.locator("[data-pane-id]")).toHaveAttribute(
      "data-pane-id",
      first,
    );
    await expect
      .poll(async () => (await terminal(page, first)).renderer)
      .toBe(renderer);
    await expectLightText(page, first);
    await page.emulateMedia({ colorScheme: "dark" });
    await expect
      .poll(async () => (await terminal(page, first)).foreground)
      .toBe("#f3f4f6ff");
    await expect
      .poll(async () => (await paintedRows(page, first))[0].lightest)
      .toBeGreaterThan(200);
    await page.emulateMedia({ colorScheme: "light" });
    await expectLightText(page, first);
    expect((await terminal(page, first)).id).toBe(before.id);
    expect((await terminal(page, second)).id).toBe(other.id);
    expect(await buffer(page, first)).toContain("INVERSE TEXT");
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "close_terminal",
        ),
      ),
    ).toHaveLength(0);
    await page.locator(".terminal-pane").screenshot({
      path: testInfo.outputPath(`terminal-light-${renderer}.png`),
    });
  });
}
