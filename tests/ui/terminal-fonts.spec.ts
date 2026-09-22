import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

for (const renderer of ["WebGL", "DOM"] as const) {
  test(`${renderer} waits for all terminal font faces before fitting and starting the shell`, async ({
    page,
  }, testInfo) => {
    await page.addInitScript(() => {
      const load = FontFaceSet.prototype.load;
      (window as any).__terminalFontLoads = [];
      FontFaceSet.prototype.load = async function (...args) {
        const faces = await load.apply(this, args);
        if (
          !(window as any).__nativeTest?.calls.some(
            (call: any) => call.command === "start_terminal",
          )
        ) {
          (window as any).__terminalFontLoads.push(
            ...faces.map((face) => ({
              family: face.family.replace(/^(["'])(.*)\1$/, "$2"),
              style: face.style,
              weight: face.weight,
              status: face.status,
            })),
          );
        }
        return faces;
      };
    });
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
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    const requested = new Set<string>();
    await page.route(
      /(?:JetBrainsMono-BoldItalic|NotoSansSymbols|NotoSansSymbols2-Regular|SymbolsNerdFontMono-Regular)\.woff2$/,
      async (route) => {
        requested.add(new URL(route.request().url()).pathname);
        await pending;
        await route.continue();
      },
    );
    await mockDesktop(page, false);
    try {
      await page.goto("/", { waitUntil: "domcontentloaded" });
      await expect.poll(() => requested.size).toBe(4);
      await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "0");
      expect(
        await page.evaluate(() =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "start_terminal",
          ),
        ),
      ).toEqual([]);
    } finally {
      release();
    }
    await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls.filter(
              (call: any) => call.command === "start_terminal",
            ).length,
        ),
      )
      .toBe(1);
    const result = await page.evaluate(async () => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      const pane = document.querySelector<HTMLElement>("[data-pane-id]")!;
      const runtime = runningTerminal(pane.dataset.paneId!)!;
      const start = (window as any).__nativeTest.calls.find(
        (call: any) => call.command === "start_terminal",
      ).args.request;
      await new Promise<void>((resolve) =>
        runtime.terminal.write(
          "\x1b[2J\x1b[H" +
            [
              "[woro@woro-home lomi]$ ls",
              "AGENTS.md  package.json  src-tauri  src  tests",
              "Zażółć gęślą jaźń — 0O 1Il {} [] ()",
              "\x1b[1mBold\x1b[0m  \x1b[3mItalic\x1b[0m  \x1b[1;3mBold italic\x1b[0m",
              "┌────────────┐",
              "│ Terminal   │",
              "└────────────┘",
              "Braille: ⠀ ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ⣿",
              "Symbols: ⚙ ✓ ✗ ▶ \u{1fb00}  Nerd: \ue0b0 \uf013 \uf121 \u{f0001}",
            ].join("\r\n"),
          resolve,
        ),
      );
      return {
        renderer: runtime.getSnapshot().renderer,
        // WebKit recreates CSS FontFace objects after xterm injects its styles.
        // Check the completed loads before PTY startup, not replacement objects.
        fonts: [
          ...new Map(
            (window as any).__terminalFontLoads.map((face: any) => [
              `${face.family}/${face.style}/${face.weight}`,
              face.status,
            ]),
          ).values(),
        ],
        initial: { cols: start.cols, rows: start.rows },
        current: { cols: runtime.terminal.cols, rows: runtime.terminal.rows },
        fitted: runtime.fitAddon.proposeDimensions(),
      };
    });
    expect(result.renderer).toBe(renderer);
    expect(result.fonts).toEqual(Array(7).fill("loaded"));
    expect(result.initial).toEqual(result.current);
    expect(result.current).toEqual(result.fitted);
    await page.locator(".terminal-pane").screenshot({
      path: testInfo.outputPath(`terminal-fonts-${renderer}.png`),
    });
    const glyphs = await page.evaluate(async () => {
      const {
        applyTerminalPreferences,
        terminalAppearance,
        themeAppliedEvent,
      } = await import("/src/theme/runtime.ts");
      const { defaultTerminalPreferences } =
        await import("/src/terminal-preferences.ts");
      const updated = new Promise<void>((resolve) =>
        window.addEventListener(themeAppliedEvent, () => resolve(), {
          once: true,
        }),
      );
      applyTerminalPreferences({
        ...defaultTerminalPreferences,
        appearance: { fontFamily: '"Missing, Font", monospace' },
      });
      await updated;
      const family = terminalAppearance().fontFamily!;
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 64;
      const context = canvas.getContext("2d", { willReadFrequently: true })!;
      const pixels = (font: string, text: string, style: string) => {
        context.clearRect(0, 0, 64, 64);
        context.font = `${style} 32px ${font}`;
        context.fillText(text, 8, 48);
        return [...context.getImageData(0, 0, 64, 64).data];
      };
      return {
        family,
        blankBraille: pixels(family, "\u2800", "normal").every(
          (byte) => byte === 0,
        ),
        symbols: ["normal", "bold italic"].flatMap((style) =>
          [
            ["⚙", '"Noto Sans Symbols"'],
            ["⠋", '"Noto Sans Symbols 2"'],
            ["⣿", '"Noto Sans Symbols 2"'],
            ["\u{1fb00}", '"Noto Sans Symbols 2"'],
            ["\uf013", '"Symbols Nerd Font Mono"'],
            ["\u{f0001}", '"Symbols Nerd Font Mono"'],
          ].map(([symbol, fallback]) => {
            const actual = pixels(family, symbol, style);
            const expected = pixels(fallback, symbol, style);
            return (
              actual.some((byte) => byte !== 0) &&
              actual.every((byte, index) => byte === expected[index])
            );
          }),
        ),
      };
    });
    expect(glyphs.family).toBe(
      '"Missing, Font", "JetBrains Mono", "Noto Sans Symbols", "Noto Sans Symbols 2", "Symbols Nerd Font Mono", monospace',
    );
    expect(glyphs.blankBraille).toBe(true);
    expect(glyphs.symbols).toEqual(Array(12).fill(true));
  });
}

test("a font loading failure keeps the terminal usable with a fallback", async ({
  page,
}) => {
  await page.route("**/fonts/**/*.woff2", (route) => route.abort());
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.insertText("fallback input");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) => call.command === "write_terminal")
          .map((call: any) => call.args.data)
          .join(""),
      ),
    )
    .toBe("fallback input");
});
