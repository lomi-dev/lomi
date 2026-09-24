import { expect, test, type Page } from "@playwright/test";
import { newProject, newSession } from "../../src/model";
import { mockDesktop } from "./desktop";

type Renderer = "WebGL" | "DOM";

async function prepare(page: Page, renderer: Renderer) {
  await page.setViewportSize({ width: 1100, height: 620 });
  await page.emulateMedia({
    colorScheme: "dark",
    reducedMotion: "no-preference",
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

  const project = newProject("/project", "local:bash");
  const session = newSession();
  session.sidebar = null;
  session.sidebarSides.git = "left";
  session.projects = [project];
  session.activeProjectId = project.id;
  await mockDesktop(page, true, session);
  await page.goto("/");

  const id = (await page
    .locator(".terminal-pane[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  await expect
    .poll(() =>
      page.evaluate(async (id) => {
        const { runningTerminal } = await import("/src/terminal-runtime.ts");
        return runningTerminal(id)?.getSnapshot();
      }, id),
    )
    .toMatchObject({ status: "running", renderer });

  await page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(id)!;
    runtime.terminal.options.cursorBlink = false;
    const output =
      "\x1b[3J\x1b[2J\x1b[H" +
      Array.from(
        { length: 100 },
        (_, line) =>
          `STATIONARY ${String(line).padStart(3, "0")} short row\r\n`,
      ).join("") +
      Array.from(
        { length: 12 },
        (_, line) =>
          `WRAPPED ${String(line).padStart(2, "0")} ` +
          "abcdefghij".repeat(24) +
          "\r\n",
      ).join("") +
      "\x1b[?25hCURSOR> ";
    await new Promise<void>((resolve) =>
      runtime.terminal.write(output, resolve),
    );

    const state = ((window as any).__terminalStability = {
      runtime,
      renders: 0,
      resizeEvents: [] as { cols: number; rows: number }[],
      startResizes: 0,
      resizeCount() {
        return (window as any).__nativeTest.calls.filter(
          (call: any) =>
            call.command === "resize_terminal" &&
            call.args.id === runtime.sessionId,
        ).length;
      },
      reset() {
        this.renders = 0;
        this.resizeEvents = [];
        this.startResizes = this.resizeCount();
      },
      read(label: string) {
        const terminal = runtime.terminal;
        const buffer = terminal.buffer.active;
        const layout = runtime.host.closest<HTMLElement>(".split-child")!;
        const panel = document.querySelector<HTMLElement>(
          `.terminal-pane[data-pane-id="${id}"]`,
        )!;
        const line = (index: number) =>
          buffer.getLine(index)?.translateToString(true) ?? "";
        const proposed = runtime.fitAddon.proposeDimensions();
        return {
          label,
          cols: terminal.cols,
          rows: terminal.rows,
          proposed: proposed
            ? { cols: proposed.cols, rows: proposed.rows }
            : null,
          buffer: {
            viewportY: buffer.viewportY,
            baseY: buffer.baseY,
            cursorY: buffer.cursorY,
            cursorX: buffer.cursorX,
            top: line(buffer.viewportY),
            cursorLine: line(buffer.baseY + buffer.cursorY),
          },
          transform: getComputedStyle(layout).transform,
          opacity: [layout, panel, runtime.host].map(
            (element) => getComputedStyle(element).opacity,
          ),
          textMotionCount: layout
            .getAnimations()
            .filter((animation) => animation.id === "lomi-layout-motion")
            .length,
          resizeCountSinceAction: this.resizeCount() - this.startResizes,
          resizeEvents: [...this.resizeEvents],
          renders: this.renders,
        };
      },
    });
    runtime.terminal.onResize(({ cols, rows }) =>
      state.resizeEvents.push({ cols, rows }),
    );
    runtime.terminal.onRender(() => state.renders++);
  }, id);
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__terminalStability.read("ready").buffer.cursorLine,
      ),
    )
    .toContain("CURSOR>");
}

async function toggle(page: Page, opening: boolean) {
  const button = page.getByRole("button", { name: /^Toggle source control/ });
  const before = await page.evaluate(() =>
    (window as any).__terminalStability.read("before"),
  );
  const { sync, first } = await button.evaluate(async (element) => {
    const state = (window as any).__terminalStability;
    state.reset();
    (element as HTMLButtonElement).click();
    const sync = state.read("sync");
    const first = await new Promise((resolve) =>
      requestAnimationFrame(() => resolve(state.read("first-raf"))),
    );
    return { sync, first };
  });
  const second = await page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() =>
          resolve((window as any).__terminalStability.read("second-raf")),
        ),
      ),
  );
  await page.waitForTimeout(60);
  const middle = await page.evaluate(() =>
    (window as any).__terminalStability.read("60ms"),
  );
  await page.waitForTimeout(70);
  const end = await page.evaluate(() =>
    (window as any).__terminalStability.read("120ms"),
  );

  expect(sync.cols).toBe(sync.proposed?.cols);
  expect(sync.rows).toBe(sync.proposed?.rows);
  expect(first.cols).toBe(sync.cols);
  expect(first.rows).toBe(sync.rows);
  expect(first.renders).toBeGreaterThan(0);
  expect(first.resizeEvents.length).toBeGreaterThan(0);
  expect(first.resizeCountSinceAction).toBe(1);
  if (opening) expect(sync.cols).toBeLessThan(before.cols);
  else expect(sync.cols).toBeGreaterThan(before.cols);
  for (const frame of [sync, first, second, middle, end]) {
    expect(frame.transform).toBe("none");
    expect(frame.opacity).toEqual(["1", "1", "1"]);
    expect(frame.textMotionCount).toBe(0);
    expect(frame.resizeCountSinceAction).toBe(1);
  }
  return { before, sync, first, second, middle, end };
}

for (const renderer of ["WebGL", "DOM"] as const) {
  test(`${renderer}: sidebar fit reaches the first frame and preserves bottom and scrolled-back text`, async ({
    page,
  }) => {
    test.setTimeout(60_000);
    await prepare(page, renderer);

    for (const viewport of ["bottom", "scrolled-back"] as const) {
      if (viewport === "bottom") {
        await page.evaluate(() =>
          (window as any).__terminalStability.runtime.terminal.scrollToBottom(),
        );
      } else {
        await page.evaluate(() => {
          const terminal = (window as any).__terminalStability.runtime.terminal;
          const buffer = terminal.buffer.active;
          const anchor = Array.from(
            { length: buffer.baseY + buffer.cursorY },
            (_, line) => line,
          ).find((line) =>
            buffer
              .getLine(line)
              ?.translateToString(true)
              .startsWith("STATIONARY 080"),
          );
          if (anchor === undefined)
            throw new Error("Could not find the scrollback anchor");
          terminal.scrollToLine(anchor);
        });
        await expect
          .poll(() =>
            page.evaluate(
              () =>
                (window as any).__terminalStability.read("anchor").buffer.top,
            ),
          )
          .toContain("STATIONARY 080");
      }

      for (const opening of [true, false]) {
        const action = await toggle(page, opening);
        if (viewport === "bottom") {
          expect(action.before.buffer.viewportY).toBe(
            action.before.buffer.baseY,
          );
          expect(action.first.buffer.viewportY).toBe(action.first.buffer.baseY);
          expect(action.first.buffer.cursorLine).toContain("CURSOR>");
        } else {
          expect(action.before.buffer.viewportY).toBeLessThan(
            action.before.buffer.baseY,
          );
          expect(action.before.buffer.top).toContain("STATIONARY 080");
          expect(action.first.buffer.viewportY).toBeLessThan(
            action.first.buffer.baseY,
          );
          expect(action.first.buffer.top).toContain("STATIONARY 080");
        }
        await page.waitForTimeout(140);
      }
    }
  });
}
