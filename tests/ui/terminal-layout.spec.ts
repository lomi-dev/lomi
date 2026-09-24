import { expect, test } from "@playwright/test";
import { newId, newPane, newProject, newSession, panes } from "../../src/model";
import type { Layout } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";
import {
  captureLayoutMotion,
  finishLayoutMotion,
  layoutMotionCount,
} from "./layout-motion";

test.use({ colorScheme: "dark" });

for (const depth of [0, 3]) {
  test(`splitting and closing preserve ${2 ** depth + 1} terminal mounts and renderers`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({ reducedMotion: "no-preference" });
    const grid = (depth: number): Layout =>
      depth === 0
        ? newPane("/project")
        : {
            type: "split",
            id: newId(),
            axis: depth === 3 ? "horizontal" : "vertical",
            ratio: 0.5,
            first: grid(depth - 1),
            second: grid(depth - 1),
          };
    const project = newProject("/project", "local:bash");
    const tab = project.workspaces[0].tabs[0];
    if (tab.type !== "terminal") throw new Error("Expected terminal tab");
    const closed = newPane("/project");
    tab.layout = {
      type: "split",
      id: newId(),
      axis: "horizontal",
      ratio: 0.25,
      first: closed,
      second: grid(depth),
    };
    tab.activePaneId = closed.id;
    const ids = panes(tab.layout).map((pane) => pane.id);
    await mockDesktop(page, false, {
      ...newSession(),
      projects: [project],
      activeProjectId: project.id,
    });
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toHaveCount(ids.length);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls.filter(
              (call: any) => call.command === "start_terminal",
            ).length,
        ),
      )
      .toBe(ids.length);
    await expect
      .poll(() =>
        page.evaluate(async (ids) => {
          const { runningTerminal } = await import("/src/terminal-runtime.ts");
          return ids.every(
            (id) => runningTerminal(id)?.getSnapshot().renderer === "WebGL",
          );
        }, ids),
      )
      .toBe(true);
    await page.evaluate(async (ids) => {
      const { TerminalRuntime, runningTerminal } =
        await import("/src/terminal-runtime.ts");
      const desktop = window as any;
      desktop.__layoutTest = {
        calls: [] as { method: string; id: string; elapsed: number }[],
        panes: new Map(
          ids.map((id) => [
            id,
            {
              element: document.querySelector(`[data-pane-id="${id}"]`),
              runtime: runningTerminal(id),
              width: document
                .querySelector(`[data-pane-id="${id}"]`)!
                .parentElement!.getBoundingClientRect().width,
              canvases: [
                ...runningTerminal(id)!.host.querySelectorAll(
                  ".xterm-screen > canvas:not(.xterm-link-layer)",
                ),
              ],
            },
          ]),
        ),
      };
      for (const method of ["attach", "detach", "dispose"] as const) {
        const original = TerminalRuntime.prototype[method];
        TerminalRuntime.prototype[method] = function (...args: any[]) {
          const start = performance.now();
          const result = (original as any).apply(this, args);
          desktop.__layoutTest.calls.push({
            method,
            id: this.paneId,
            elapsed: performance.now() - start,
          });
          return result;
        };
      }
      desktop.__nativeTest.emit(
        runningTerminal(ids[1])!.sessionId,
        "\r\nPreserved output 🦀\r\n",
      );
    }, ids);
    await captureLayoutMotion(page);
    await expect
      .poll(() => buffer(page, ids[1]))
      .toContain("Preserved output 🦀");
    const closeStart = await layoutMotionCount(page);
    await page.keyboard.press("Control+w");
    await expect(page.locator("[data-pane-id]")).toHaveCount(ids.length - 1);
    const closeMotion = await page.evaluate((start) => {
      const state = (window as any).__layoutTest;
      const geometry = [...state.panes].slice(1).map(([id, pane]: any) => {
        const panel = pane.element.parentElement as HTMLElement;
        return {
          id,
          left: panel.offsetLeft,
          top: panel.offsetTop,
          width: panel.offsetWidth,
          height: panel.offsetHeight,
          opacity: getComputedStyle(pane.element).opacity,
        };
      });
      const readTransform = (value: string) =>
        value === "none"
          ? new DOMMatrixReadOnly()
          : new DOMMatrixReadOnly(value);
      const animations = (window as any).__layoutMotion.records
        .slice(start)
        .map(({ animation }: { animation: Animation }) => {
          const effect = animation.effect as KeyframeEffect;
          const target = effect.target as HTMLElement;
          const style = getComputedStyle(target);
          const matrix = readTransform(style.transform);
          const translate = style.translate
            .split(/\s+/)
            .filter(Boolean)
            .map((value) => parseFloat(value) || 0);
          const keyframes = effect.getKeyframes().map((frame) => {
            const transform = readTransform(String(frame.transform ?? "none"));
            const frameTranslate = String(frame.translate ?? "none")
              .split(/\s+/)
              .filter(Boolean)
              .map((value) => parseFloat(value) || 0);
            return {
              hasScale: "scale" in frame,
              transformText: String(frame.transform ?? ""),
              opacity: String(frame.opacity),
              x: transform.e + (frameTranslate[0] ?? 0),
              y: transform.f + (frameTranslate[1] ?? 0),
            };
          });
          return {
            id: animation.id,
            currentTime: Number(animation.currentTime),
            playState: animation.playState,
            duration: Number(effect.getTiming().duration),
            targetContainsRetained: [...state.panes]
              .slice(1)
              .some(
                ([, pane]: any) =>
                  pane.element.isConnected &&
                  (target === pane.element || target.contains(pane.element)),
              ),
            keyframes,
            x: matrix.e + (translate[0] ?? 0),
            y: matrix.f + (translate[1] ?? 0),
            scaleX: Math.hypot(matrix.a, matrix.b),
            scaleY: Math.hypot(matrix.c, matrix.d),
          };
        });
      const terminalTargets = [
        ...document.querySelectorAll<HTMLElement>(
          ".dock-pane-host > .split-child",
        ),
      ]
        .filter((element) =>
          element.querySelector(".terminal-pane[data-pane-id]"),
        )
        .map((element) => ({
          transform: getComputedStyle(element).transform,
          motionCount: element
            .getAnimations()
            .filter((animation) => animation.id === "lomi-layout-motion")
            .length,
        }));
      return { geometry, animations, terminalTargets };
    }, closeStart);
    expect(
      closeMotion.animations.every(
        (animation) => !animation.targetContainsRetained,
      ),
    ).toBe(true);
    expect(
      closeMotion.terminalTargets.every(
        (target) => target.motionCount === 0 && target.transform === "none",
      ),
    ).toBe(true);
    for (const animation of closeMotion.animations) {
      expect(animation.id).toBe("lomi-layout-motion");
      expect(animation.playState).toBe("paused");
      expect(animation.currentTime).toBeCloseTo(animation.duration / 2, 0);
      expect(animation.duration).toBeGreaterThan(0);
      expect(animation.duration).toBeLessThanOrEqual(120);
      expect(animation.keyframes.every((frame) => !frame.hasScale)).toBe(true);
      expect(animation.keyframes.every((frame) => frame.opacity === "1")).toBe(
        true,
      );
      expect(
        animation.keyframes.every(
          (frame) => !/scale\s*\(/i.test(frame.transformText),
        ),
      ).toBe(true);
      expect(animation.scaleX).toBeCloseTo(1, 2);
      expect(animation.scaleY).toBeCloseTo(1, 2);
      expect(Math.abs(animation.x)).toBeLessThanOrEqual(8.1);
      expect(Math.abs(animation.y)).toBeLessThanOrEqual(8.1);
      for (const frame of animation.keyframes) {
        expect(Math.abs(frame.x)).toBeLessThanOrEqual(8.1);
        expect(Math.abs(frame.y)).toBeLessThanOrEqual(8.1);
      }
    }
    expect(closeMotion.geometry.every((pane) => pane.opacity === "1")).toBe(
      true,
    );
    await page.evaluate(() => {
      const state = (window as any).__layoutTest;
      const pane = [...state.panes.values()][1] as any;
      (window as any).__nativeTest.emit(
        pane.runtime.sessionId,
        "\r\nStreaming during expansion 🦀\r\n",
      );
    });
    await expect
      .poll(() => buffer(page, ids[1]))
      .toContain("Streaming during expansion 🦀");
    await page.screenshot({
      path: testInfo.outputPath("terminal-close-midpoint.png"),
    });
    await finishLayoutMotion(page, closeStart);
    const closing = await page.evaluate(async (expectedGeometry) => {
      const state = (window as any).__layoutTest;
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      return {
        calls: state.calls,
        preserved: [...state.panes].slice(1).every(([id, pane]: any) => {
          const element = document.querySelector(`[data-pane-id="${id}"]`);
          const canvases = element?.querySelectorAll(
            ".xterm-screen > canvas:not(.xterm-link-layer)",
          );
          return (
            pane.element === element &&
            pane.runtime === runningTerminal(id) &&
            pane.canvases.length === canvases?.length &&
            pane.canvases.every(
              (canvas: HTMLCanvasElement, index: number) =>
                canvas === canvases[index],
            )
          );
        }),
        geometryStable: expectedGeometry.every((expected: any) => {
          const panel = document.querySelector(
            `[data-pane-id="${expected.id}"]`,
          )?.parentElement as HTMLElement | null;
          return (
            panel?.offsetLeft === expected.left &&
            panel?.offsetTop === expected.top &&
            panel?.offsetWidth === expected.width &&
            panel?.offsetHeight === expected.height
          );
        }),
      };
    }, closeMotion.geometry);
    expect(closing.preserved).toBe(true);
    expect(closing.geometryStable).toBe(true);
    await expect
      .poll(() =>
        page.evaluate(
          (start) =>
            (window as any).__layoutMotion.records
              .slice(start)
              .every((record: any) => record.animation.playState === "idle"),
          closeStart,
        ),
      )
      .toBe(true);
    expect(closing.calls.filter((call: any) => call.id !== closed.id)).toEqual(
      [],
    );
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
    await page.evaluate(() => {
      (window as any).__layoutTest.calls = [];
    });
    const splitStart = await layoutMotionCount(page);
    await page.keyboard.press("Control+d");
    await expect(page.locator("[data-pane-id]")).toHaveCount(ids.length);
    await expect
      .poll(() => layoutMotionCount(page))
      .toBeGreaterThan(splitStart);
    const opening = await page.evaluate((start) => {
      const state = (window as any).__layoutTest;
      const panes = [
        ...document.querySelectorAll<HTMLElement>("[data-pane-id]"),
      ];
      const entering = panes.find(
        (pane) => !state.panes.has(pane.dataset.paneId),
      );
      if (!entering) throw new Error("Expected the new pane to be mounted");
      const readTransform = (value: string) =>
        value === "none"
          ? new DOMMatrixReadOnly()
          : new DOMMatrixReadOnly(value);
      const animations = (window as any).__layoutMotion.records
        .slice(start)
        .map(({ animation }: { animation: Animation }) => {
          const effect = animation.effect as KeyframeEffect;
          const target = effect.target as HTMLElement;
          const style = getComputedStyle(target);
          const matrix = readTransform(style.transform);
          const translate = style.translate
            .split(/\s+/)
            .filter(Boolean)
            .map((value) => parseFloat(value) || 0);
          const keyframes = effect.getKeyframes().map((frame) => {
            const transform = readTransform(String(frame.transform ?? "none"));
            const frameTranslate = String(frame.translate ?? "none")
              .split(/\s+/)
              .filter(Boolean)
              .map((value) => parseFloat(value) || 0);
            return {
              hasScale:
                "scale" in frame ||
                /scale\s*\(/i.test(String(frame.transform ?? "")),
              x: transform.e + (frameTranslate[0] ?? 0),
              y: transform.f + (frameTranslate[1] ?? 0),
            };
          });
          return {
            id: animation.id,
            currentTime: Number(animation.currentTime),
            playState: animation.playState,
            duration: Number(effect.getTiming().duration),
            targetContainsEntering:
              target === entering || target.contains(entering),
            targetContainsRetained: [...state.panes.values()].some(
              (pane: any) =>
                pane.element.isConnected &&
                (target === pane.element || target.contains(pane.element)),
            ),
            opacity: Number(style.opacity),
            opacityFrames: effect
              .getKeyframes()
              .filter((frame) => frame.opacity !== undefined)
              .map((frame) => Number(frame.opacity)),
            hasScale: effect
              .getKeyframes()
              .some(
                (frame) =>
                  "scale" in frame ||
                  /scale\s*\(/i.test(String(frame.transform ?? "")),
              ),
            x: matrix.e + (translate[0] ?? 0),
            y: matrix.f + (translate[1] ?? 0),
            scaleX: Math.hypot(matrix.a, matrix.b),
            scaleY: Math.hypot(matrix.c, matrix.d),
            keyframes,
          };
        });
      return {
        animations,
        geometry: [...state.panes]
          .filter(([, pane]: any) => pane.element.isConnected)
          .map(([id, pane]: any) => {
            const panel = pane.element.parentElement as HTMLElement;
            return {
              id,
              left: panel.offsetLeft,
              top: panel.offsetTop,
              width: panel.offsetWidth,
              height: panel.offsetHeight,
            };
          }),
        retainedOpacity: [...state.panes.values()]
          .filter((pane: any) => pane.element.isConnected)
          .map((pane: any) => getComputedStyle(pane.element).opacity),
      };
    }, splitStart);
    const enteringAnimation = opening.animations.find(
      (animation) =>
        animation.targetContainsEntering &&
        animation.opacityFrames[0] === 0 &&
        animation.opacityFrames.at(-1) === 1,
    );
    expect(enteringAnimation).toBeDefined();
    expect(enteringAnimation!.targetContainsRetained).toBe(false);
    expect(enteringAnimation!.duration).toBeGreaterThan(0);
    expect(enteringAnimation!.duration).toBeLessThanOrEqual(120);
    expect(enteringAnimation!.opacity).toBeGreaterThan(0);
    expect(enteringAnimation!.opacity).toBeLessThan(1);
    expect(Math.abs(enteringAnimation!.x)).toBeLessThanOrEqual(8.1);
    expect(Math.abs(enteringAnimation!.y)).toBeLessThanOrEqual(8.1);
    expect(enteringAnimation!.hasScale).toBe(false);
    expect(enteringAnimation!.scaleX).toBeCloseTo(1, 2);
    expect(enteringAnimation!.scaleY).toBeCloseTo(1, 2);
    for (const animation of opening.animations) {
      expect(animation.id).toBe("lomi-layout-motion");
      expect(animation.playState).toBe("paused");
      expect(animation.currentTime).toBeCloseTo(animation.duration / 2, 0);
      expect(animation.duration).toBeGreaterThan(0);
      expect(animation.duration).toBeLessThanOrEqual(120);
      expect(animation.hasScale).toBe(false);
      expect(animation.scaleX).toBeCloseTo(1, 2);
      expect(animation.scaleY).toBeCloseTo(1, 2);
      expect(Math.abs(animation.x)).toBeLessThanOrEqual(8.1);
      expect(Math.abs(animation.y)).toBeLessThanOrEqual(8.1);
      for (const frame of animation.keyframes) {
        expect(frame.hasScale).toBe(false);
        expect(Math.abs(frame.x)).toBeLessThanOrEqual(8.1);
        expect(Math.abs(frame.y)).toBeLessThanOrEqual(8.1);
      }
    }
    expect(opening.retainedOpacity.every((opacity) => opacity === "1")).toBe(
      true,
    );
    await page.screenshot({
      path: testInfo.outputPath("terminal-open-midpoint.png"),
    });
    await finishLayoutMotion(page, splitStart);
    const splitting = await page.evaluate(async (expectedGeometry) => {
      const state = (window as any).__layoutTest;
      return {
        calls: state.calls,
        preserved: [...state.panes].slice(1).every(([id, pane]: any) => {
          const element = document.querySelector(`[data-pane-id="${id}"]`);
          const canvases = element?.querySelectorAll(
            ".xterm-screen > canvas:not(.xterm-link-layer)",
          );
          return (
            pane.element === element &&
            pane.canvases.length === canvases?.length &&
            pane.canvases.every(
              (canvas: HTMLCanvasElement, index: number) =>
                canvas === canvases[index],
            )
          );
        }),
        geometryStable: expectedGeometry.every((expected: any) => {
          const panel = document.querySelector(
            `[data-pane-id="${expected.id}"]`,
          )?.parentElement as HTMLElement | null;
          return (
            panel?.offsetLeft === expected.left &&
            panel?.offsetTop === expected.top &&
            panel?.offsetWidth === expected.width &&
            panel?.offsetHeight === expected.height
          );
        }),
        retainedOpacity: [...state.panes.values()]
          .filter((pane: any) => pane.element.isConnected)
          .every((pane: any) => getComputedStyle(pane.element).opacity === "1"),
      };
    }, opening.geometry);
    expect(splitting.preserved).toBe(true);
    expect(splitting.geometryStable).toBe(true);
    expect(splitting.retainedOpacity).toBe(true);
    await expect
      .poll(() =>
        page.evaluate(
          (start) =>
            (window as any).__layoutMotion.records
              .slice(start)
              .every((record: any) => record.animation.playState === "idle"),
          splitStart,
        ),
      )
      .toBe(true);
    expect(splitting.calls.some((call: any) => call.method === "attach")).toBe(
      true,
    );
    expect(new Set(splitting.calls.map((call: any) => call.id)).size).toBe(1);
    for (const call of splitting.calls) expect(ids).not.toContain(call.id);
    await expect
      .poll(() => buffer(page, ids[1]))
      .toContain("Preserved output 🦀");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls.filter(
              (call: any) => call.command === "start_terminal",
            ).length,
        ),
      )
      .toBe(ids.length + 1);
    await page.screenshot({
      path: testInfo.outputPath("terminal-layout-stable.png"),
    });

    if (depth === 0) {
      const retainedIds = ids.slice(1);
      const expectedLayout = await page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")
            ?.projects[0].workspaces[0].tabs[0].layout,
      );
      const interruptStart = await layoutMotionCount(page);
      await page.keyboard.press("Control+d");
      await expect(page.locator("[data-pane-id]")).toHaveCount(ids.length + 1);
      await expect
        .poll(() => layoutMotionCount(page))
        .toBeGreaterThan(interruptStart);
      const interruptCloseStart = await layoutMotionCount(page);
      await page.keyboard.press("Control+w");
      await expect(page.locator("[data-pane-id]")).toHaveCount(ids.length);
      expect(await layoutMotionCount(page)).toBe(interruptCloseStart);
      await page.screenshot({
        path: testInfo.outputPath("terminal-layout-interrupted-midpoint.png"),
      });
      await finishLayoutMotion(page, interruptStart);
      await expect
        .poll(() =>
          page.evaluate(
            (start) =>
              (window as any).__layoutMotion.records
                .slice(start)
                .every((record: any) => record.animation.playState === "idle"),
            interruptStart,
          ),
        )
        .toBe(true);
      expect(
        await page.evaluate(async (retainedIds) => {
          const { runningTerminal } = await import("/src/terminal-runtime.ts");
          const state = (window as any).__layoutTest;
          return retainedIds.every((id: string) => {
            const pane = state.panes.get(id);
            const element = document.querySelector(`[data-pane-id="${id}"]`);
            const canvases = element?.querySelectorAll(
              ".xterm-screen > canvas:not(.xterm-link-layer)",
            );
            return (
              pane?.element === element &&
              pane.runtime === runningTerminal(id) &&
              pane.canvases.length === canvases?.length &&
              pane.canvases.every(
                (canvas: HTMLCanvasElement, index: number) =>
                  canvas === canvases[index],
              )
            );
          });
        }, retainedIds),
      ).toBe(true);
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window as any).__nativeTest.calls.filter(
                (call: any) => call.command === "close_terminal",
              ).length,
          ),
        )
        .toBe(2);
      expect(
        await page.evaluate(
          () =>
            JSON.parse(localStorage.getItem("test-session") ?? "null")
              ?.projects[0].workspaces[0].tabs[0].layout,
        ),
      ).toEqual(expectedLayout);
    }
  });
}

test("a terminal closed before its first frame cancels WebGL and closes its PTY", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
  const id = await page.evaluate(async () => {
    const { terminalFor, closeTerminals } =
      await import("/src/terminal-runtime.ts");
    const id = crypto.randomUUID();
    const runtime = terminalFor(
      { type: "terminal", id, cwd: "/project" },
      {
        id: "local:bash",
        name: "bash",
        kind: "bash",
        program: "/bin/bash",
        distro: null,
        home: "/home/test",
      },
    );
    const container = document.createElement("div");
    container.style.cssText = "width:600px;height:300px";
    document.body.append(container);
    const state = ((window as any).__closedTerminal = {
      rendererLoads: 0,
      sessionId: runtime.sessionId,
      host: runtime.host,
    });
    const load = runtime.terminal.loadAddon.bind(runtime.terminal);
    runtime.terminal.loadAddon = (addon: any) => {
      if ("onContextLoss" in addon) state.rendererLoads++;
      load(addon);
    };
    runtime.attach(container);
    closeTerminals([id]);
    runtime.detach();
    container.remove();
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() =>
        requestAnimationFrame(() => setTimeout(resolve, 0)),
      ),
    );
    return id;
  });
  await expect
    .poll(() =>
      page.evaluate(async (id) => {
        const { runningTerminal } = await import("/src/terminal-runtime.ts");
        const desktop = window as any;
        return {
          runtime: !!runningTerminal(id),
          closed: desktop.__nativeTest.calls.some(
            (call: any) =>
              call.command === "close_terminal" &&
              call.args.id === desktop.__closedTerminal.sessionId,
          ),
          connected: desktop.__closedTerminal.host.isConnected,
          rendererLoads: desktop.__closedTerminal.rendererLoads,
        };
      }, id),
    )
    .toEqual({
      runtime: false,
      closed: true,
      connected: false,
      rendererLoads: 0,
    });
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
});

test("terminal output and shortcuts remain usable when WebGL is unavailable", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (
      kind: string,
      ...args: any[]
    ) {
      if (kind === "webgl2") return null;
      return (getContext as any).call(this, kind, ...args);
    } as typeof getContext;
  });
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
  const first = await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id");
  await page.keyboard.press("Control+d");
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  await page.keyboard.press("Control+w");
  await expect(page.locator("[data-pane-id]")).toHaveCount(1);
  await expect(page.locator("[data-pane-id]")).toHaveAttribute(
    "data-pane-id",
    first!,
  );
  await page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    (window as any).__nativeTest.emit(
      runningTerminal(id!)!.sessionId,
      "\r\nFallback output 🦀\r\n",
    );
  }, first);
  await expect.poll(() => buffer(page, first!)).toContain("Fallback output 🦀");
  await expect(page.locator(".xterm-rows")).toContainText("Fallback output 🦀");
});
