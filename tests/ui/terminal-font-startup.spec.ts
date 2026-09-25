import { expect, test as base } from "@playwright/test";
import { mockDesktop } from "./desktop";

const test = base.extend({
  deviceScaleFactor: async ({ browserName }, use) => {
    // Chromium emulates deviceScaleFactor without scaling devicePixelContentBoxSize.
    // Use WebKit to exercise the Retina rendering path.
    await use(browserName === "webkit" ? 2 : 1);
  },
});
test.use({ colorScheme: "dark" });

test("WebGL rebuilds startup glyphs after fonts settle", async ({
  page,
}, testInfo) => {
  await page.addInitScript(() => {
    let pending = false;
    let release!: () => void;
    const ready = new Promise<FontFaceSet>((resolve) => {
      release = () => {
        pending = false;
        resolve(document.fonts);
      };
    });
    const original = Object.getOwnPropertyDescriptor(
      FontFaceSet.prototype,
      "ready",
    )!.get!;
    Object.defineProperty(FontFaceSet.prototype, "ready", {
      get() {
        return pending ? ready : original.call(this);
      },
    });
    // Simulate font invalidation when WebGL replaces xterm's DOM styles:
    // font loads have completed, but early atlas warmup still sees a fallback.
    const getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (
      kind: string,
      ...args: any[]
    ) {
      if (kind === "webgl2" && !(window as any).__releaseFonts) {
        pending = true;
        (window as any).__releaseFonts = release;
      }
      return (getContext as any).call(this, kind, ...args);
    } as typeof getContext;
    (window as any).__fallbackGlyphs = 0;
    const fill = CanvasRenderingContext2D.prototype.fillText;
    CanvasRenderingContext2D.prototype.fillText = function (...args) {
      this.save();
      if (pending) {
        this.font = "10px sans-serif";
        (window as any).__fallbackGlyphs++;
      }
      fill.apply(this, args);
      this.restore();
    };
  });
  await mockDesktop(page, false);
  await page.goto("/");
  await expect
    .poll(() => page.evaluate(() => (window as any).__fallbackGlyphs))
    .toBeGreaterThan(0);
  try {
    // Replace an initialization still waiting for fonts, as docking can do.
    await page.evaluate(async () => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      const pane = document.querySelector<HTMLElement>("[data-pane-id]")!;
      const runtime = runningTerminal(pane.dataset.paneId!)!;
      const container = runtime.host.parentElement!;
      runtime.detach();
      runtime.attach(container);
    });
    await page.locator(".xterm-helper-textarea").focus();
    await page.keyboard.insertText("queued input");
    await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "0");
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls.filter((call: any) =>
          ["start_terminal", "write_terminal"].includes(call.command),
        ),
      ),
    ).toEqual([]);
  } finally {
    await page.evaluate(() => (window as any).__releaseFonts());
  }
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) => call.command === "write_terminal")
          .map((call: any) => call.args.data)
          .join(""),
      ),
    )
    .toBe("queued input");
  const state = await page.evaluate(async () => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const pane = document.querySelector<HTMLElement>("[data-pane-id]")!;
    const runtime = runningTerminal(pane.dataset.paneId!)!;
    const starts = (window as any).__nativeTest.calls.filter(
      (call: any) => call.command === "start_terminal",
    );
    await new Promise<void>((resolve) =>
      runtime.terminal.write("\x1b[?25l\x1b[2J\x1b[HMMMMMMMM", resolve),
    );
    return {
      starts: starts.map((call: any) => ({
        cols: call.args.request.cols,
        rows: call.args.request.rows,
      })),
      cols: runtime.terminal.cols,
      rows: runtime.terminal.rows,
      renderer: runtime.getSnapshot().renderer,
      fontSize: runtime.terminal.options.fontSize!,
      scale: devicePixelRatio,
    };
  });
  expect(state.renderer).toBe("WebGL");
  expect(state.starts).toEqual([{ cols: state.cols, rows: state.rows }]);
  const png = await page.locator(".xterm-screen").screenshot({
    path: testInfo.outputPath("terminal-font-startup.png"),
  });
  const height = await page.evaluate(
    async ({ png, cols, rows }) => {
      const image = new Image();
      image.src = `data:image/png;base64,${png}`;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.width;
      canvas.height = image.height;
      const context = canvas.getContext("2d")!;
      context.drawImage(image, 0, 0);
      const width = Math.floor(image.width / cols);
      const height = Math.floor(image.height / rows);
      const pixels = context.getImageData(0, 0, width, height).data;
      const paintedRows = [];
      for (let y = 0; y < height; y++)
        for (let x = 0; x < width; x++)
          if (pixels[(y * width + x) * 4] > 100) paintedRows.push(y);
      return Math.max(...paintedRows) - Math.min(...paintedRows) + 1;
    },
    { png: png.toString("base64"), cols: state.cols, rows: state.rows },
  );
  // Verify the painted letter, not just xterm's correctly sized cell/cursor.
  expect(height).toBeGreaterThan(state.fontSize * state.scale * 0.5);
});

for (const remount of [false, true])
  test(`waits for font invalidation after the final WebGL font refresh${remount ? " during a remount" : " with the renderer retained"}`, async ({
    page,
  }, testInfo) => {
    await page.addInitScript(() => {
      let pending = false;
      let invalidated = false;
      let releaseReady!: (fonts: FontFaceSet) => void;
      const ready = new Promise<FontFaceSet>((resolve) => {
        releaseReady = resolve;
      });
      const blockedLoads: (() => void)[] = [];
      const originalReady = Object.getOwnPropertyDescriptor(
        FontFaceSet.prototype,
        "ready",
      )!.get!;
      Object.defineProperty(FontFaceSet.prototype, "ready", {
        get() {
          if (pending) return ready;
          const current = originalReady.call(this);
          if (!invalidated && (window as any).__webglContextCreated)
            return current.then((fonts) => {
              invalidated = true;
              pending = true;
              (window as any).__fontInvalidatedAfterReady = true;
              return fonts;
            });
          return current;
        },
      });
      const originalLoad = FontFaceSet.prototype.load;
      FontFaceSet.prototype.load = function (...args) {
        if (!pending) return originalLoad.apply(this, args);
        (window as any).__blockedFontLoads++;
        return new Promise((resolve, reject) => {
          blockedLoads.push(() =>
            originalLoad.apply(this, args).then(resolve, reject),
          );
        });
      };
      (window as any).__blockedFontLoads = 0;
      (window as any).__fallbackGlyphs = [];
      (window as any).__releaseFonts = () => {
        pending = false;
        releaseReady(document.fonts);
        for (const load of blockedLoads.splice(0)) load();
      };
      const getContext = HTMLCanvasElement.prototype.getContext;
      HTMLCanvasElement.prototype.getContext = function (
        kind: string,
        ...args: any[]
      ) {
        if (kind === "webgl2") (window as any).__webglContextCreated = true;
        return (getContext as any).call(this, kind, ...args);
      } as typeof getContext;
      const fill = CanvasRenderingContext2D.prototype.fillText;
      CanvasRenderingContext2D.prototype.fillText = function (...args) {
        this.save();
        if (pending) {
          this.font = "10px sans-serif";
          (window as any).__fallbackGlyphs.push(String(args[0]));
        }
        fill.apply(this, args);
        this.restore();
      };
    });
    await mockDesktop(page, false);
    await page.goto("/");
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__fontInvalidatedAfterReady),
      )
      .toBe(true);
    try {
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window as any).__blockedFontLoads > 0 ||
              (window as any).__nativeTest.calls.some(
                (call: any) => call.command === "start_terminal",
              ),
          ),
        )
        .toBe(true);
      expect(
        await page.evaluate(() =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "start_terminal",
          ),
        ),
      ).toEqual([]);
      await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "0");
      await page.evaluate(async () => {
        const { runningTerminal } = await import("/src/terminal-runtime.ts");
        const pane = document.querySelector<HTMLElement>("[data-pane-id]")!;
        const runtime = runningTerminal(pane.dataset.paneId!)!;
        await new Promise<void>((resolve) => {
          let rendered: { dispose: () => void } | undefined;
          rendered = runtime.terminal.onRender(() => {
            if (
              runtime.terminal.buffer.active
                .getLine(0)
                ?.translateToString(true)
                .includes("MMMM")
            ) {
              rendered?.dispose();
              resolve();
            }
          });
          runtime.terminal.write("\x1b[?25l\x1b[2J\x1b[HMMMMMMMM");
        });
      });
      await expect
        .poll(() =>
          page.evaluate(() => (window as any).__fallbackGlyphs.includes("M")),
        )
        .toBe(true);
      expect(
        await page.evaluate(() =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "start_terminal",
          ),
        ),
      ).toEqual([]);
      await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "0");
      if (remount)
        await page.evaluate(async () => {
          const { runningTerminal } = await import("/src/terminal-runtime.ts");
          const pane = document.querySelector<HTMLElement>("[data-pane-id]")!;
          const runtime = runningTerminal(pane.dataset.paneId!)!;
          const container = runtime.host.parentElement!;
          runtime.detach();
          runtime.attach(container);
        });
      expect(
        await page.evaluate(() =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "start_terminal",
          ),
        ),
      ).toEqual([]);
    } finally {
      await page.evaluate(() => (window as any).__releaseFonts());
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
    const state = await page.evaluate(async () => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      const pane = document.querySelector<HTMLElement>("[data-pane-id]")!;
      const runtime = runningTerminal(pane.dataset.paneId!)!;
      await new Promise<void>((resolve) =>
        runtime.terminal.write("\x1b[?25l\x1b[2J\x1b[HMMMMMMMM", resolve),
      );
      const start = (window as any).__nativeTest.calls.find(
        (call: any) => call.command === "start_terminal",
      );
      return {
        renderer: runtime.getSnapshot().renderer,
        cols: runtime.terminal.cols,
        rows: runtime.terminal.rows,
        fontSize: runtime.terminal.options.fontSize!,
        scale: devicePixelRatio,
        initial: {
          cols: start.args.request.cols,
          rows: start.args.request.rows,
        },
      };
    });
    expect(state.renderer).toBe("WebGL");
    expect(state.initial).toEqual({ cols: state.cols, rows: state.rows });
    const png = await page.locator(".xterm-screen").screenshot({
      path: testInfo.outputPath("terminal-font-post-toggle.png"),
    });
    const height = await page.evaluate(
      async ({ png, cols, rows }) => {
        const image = new Image();
        image.src = `data:image/png;base64,${png}`;
        await image.decode();
        const canvas = document.createElement("canvas");
        canvas.width = image.width;
        canvas.height = image.height;
        const context = canvas.getContext("2d")!;
        context.drawImage(image, 0, 0);
        const width = Math.floor(image.width / cols);
        const height = Math.floor(image.height / rows);
        const pixels = context.getImageData(0, 0, width, height).data;
        const paintedRows = [];
        for (let y = 0; y < height; y++)
          for (let x = 0; x < width; x++)
            if (pixels[(y * width + x) * 4] > 100) paintedRows.push(y);
        return Math.max(...paintedRows) - Math.min(...paintedRows) + 1;
      },
      { png: png.toString("base64"), cols: state.cols, rows: state.rows },
    );
    expect(height).toBeGreaterThan(state.fontSize * state.scale * 0.5);
  });
