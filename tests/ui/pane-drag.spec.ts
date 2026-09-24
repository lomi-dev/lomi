import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { newPane, newProject, newSession, splitPane } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";
import {
  captureLayoutMotion,
  finishLayoutMotion,
  layoutMotionCount,
} from "./layout-motion";

async function setup(page: Page, withEditor = false) {
  const project = newProject("/project", "local:bash");
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  const source = tab.activePaneId;
  const target = withEditor
    ? {
        type: "file" as const,
        id: "editor",
        title: "README.md",
        root: "/project",
        relative: "README.md",
      }
    : newPane("/project/target");
  const other = newPane("/project/other");
  tab.layout = splitPane(
    splitPane(tab.layout, source, "horizontal", other),
    other.id,
    "vertical",
    target,
  );
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toHaveCount(withEditor ? 2 : 3);
  await expect.poll(() => buffer(page, source)).toContain("bash $ ");
  return { source, target: target.id, layout: tab.layout };
}

test("moving beside a dirty editor retains its text and history with pointer focus enabled", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(() =>
    localStorage.setItem(
      "test-keybindings",
      JSON.stringify({ version: 1, focusFollowsPointer: true, bindings: {} }),
    ),
  );
  const { source, target } = await setup(page, true);
  const editor = page.locator(".cm-content");
  await expect(editor).toBeVisible();
  const original = await editor.innerText();
  await editor.fill("Keep this unsaved text 🦀");
  const area = (await page
    .locator(`[data-file-pane-id="${target}"]`)
    .boundingBox())!;
  await grab(page, source);
  await page.mouse.move(area.x + area.width - 20, area.y + area.height / 2, {
    steps: 8,
  });
  await expect(page.locator(".pane-drop-preview")).toBeVisible();
  await expect(page.locator(`[data-pane-id="${source}"]`)).toHaveClass(
    /is-active/,
  );
  await page.screenshot({
    path: testInfo.outputPath("pane-drag-editor-dark.png"),
  });
  await page.mouse.up();
  await page.keyboard.up("Control");
  await expect(editor).toHaveText("Keep this unsaved text 🦀");
  await editor.hover();
  await expect(editor).toBeFocused();
  await page.keyboard.press("Control+z");
  await expect.poll(() => editor.innerText()).toBe(original);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter((call: any) =>
        ["save_editor_file", "close_terminal"].includes(call.command),
      ),
    ),
  ).toEqual([]);
});

async function grab(page: Page, source: string) {
  await page.keyboard.down("Control");
  const title = page.locator(`[data-pane-id="${source}"] .terminal-title`);
  await expect(title).toBeVisible();
  const bounds = (await title.boundingBox())!;
  await page.mouse.move(
    bounds.x + bounds.width / 2,
    bounds.y + bounds.height / 2,
  );
  await page.mouse.down();
}

async function savedTab(page: Page) {
  return page.evaluate(
    () =>
      JSON.parse(localStorage.getItem("test-session") ?? "null")?.projects[0]
        .workspaces[0].tabs[0],
  );
}

test("the drop preview interpolates, retargets from its current position and drops at the latest destination", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({
    colorScheme: "dark",
    reducedMotion: "no-preference",
  });
  const { source, target } = await setup(page);
  const area = (await page
    .locator(`[data-pane-id="${target}"]`)
    .boundingBox())!;
  await grab(page, source);
  await page.mouse.move(area.x + 20, area.y + area.height / 2);
  const preview = page.locator(".pane-drop-preview");
  await preview.evaluate((element) =>
    Promise.all(element.getAnimations().map((animation) => animation.finished)),
  );
  const before = (await preview.boundingBox())!;
  await page.mouse.move(area.x + area.width / 2, area.y + 20);
  const motion = await preview.evaluate((element) => {
    const animations = element.getAnimations();
    for (const animation of animations) {
      animation.pause();
      animation.currentTime = 80;
    }
    return {
      count: animations.length,
      destination: {
        x: parseFloat(element.style.left),
        y: parseFloat(element.style.top),
        width: parseFloat(element.style.width),
        height: parseFloat(element.style.height),
      },
    };
  });
  expect(motion.count).toBeGreaterThan(0);
  const midway = (await preview.boundingBox())!;
  for (const dimension of ["width", "height"] as const) {
    expect(midway[dimension]).toBeGreaterThan(
      Math.min(before[dimension], motion.destination[dimension]),
    );
    expect(midway[dimension]).toBeLessThan(
      Math.max(before[dimension], motion.destination[dimension]),
    );
  }
  await page.mouse.move(area.x + area.width / 2 + 2, area.y + 20);
  expect(await preview.boundingBox()).toEqual(midway);
  await page.screenshot({
    path: testInfo.outputPath("pane-preview-midpoint.png"),
  });
  const next = { x: area.x + area.width - 20, y: area.y + area.height / 2 };
  const retargeted = await preview.evaluate((element, next) => {
    document.dispatchEvent(
      new PointerEvent("pointermove", {
        pointerId: 1,
        clientX: next.x,
        clientY: next.y,
        ctrlKey: true,
      }),
    );
    const rect = element.getBoundingClientRect();
    for (const animation of element.getAnimations()) {
      animation.pause();
      animation.currentTime = 80;
    }
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  }, next);
  for (const dimension of ["x", "y", "width", "height"] as const)
    expect(retargeted[dimension]).toBeCloseTo(midway[dimension], 0);
  await page.mouse.move(next.x, next.y);
  await expect(preview).toHaveAttribute("data-side", "right");
  const destination = await preview.evaluate((element) => ({
    x: parseFloat(element.style.left),
    y: parseFloat(element.style.top),
    width: parseFloat(element.style.width),
    height: parseFloat(element.style.height),
  }));
  await page.mouse.up();
  await page.keyboard.up("Control");
  await expect(preview).toHaveCount(0);
  await expect(async () => {
    const actual = (await page
      .locator(`[data-pane-id="${source}"]`)
      .locator("..")
      .boundingBox())!;
    for (const dimension of ["x", "y", "width", "height"] as const)
      expect(actual[dimension]).toBeCloseTo(destination[dimension], 0);
  }).toPass();
});

test("small movements along a drop-zone boundary keep the preview stable", async ({
  page,
}) => {
  const { source, target } = await setup(page);
  const area = (await page
    .locator(`[data-pane-id="${target}"]`)
    .boundingBox())!;
  await grab(page, source);
  const preview = page.locator(".pane-drop-preview");
  await page.mouse.move(area.x + area.width * 0.2, area.y + area.height * 0.25);
  await expect(preview).toHaveAttribute("data-side", "left");
  for (const offset of [-1, 1, -2, 2]) {
    await page.mouse.move(
      area.x + area.width * 0.25 + offset,
      area.y + area.height * 0.25,
    );
    await expect(preview).toHaveAttribute("data-side", "left");
  }
  await page.mouse.move(area.x + area.width * 0.4, area.y + area.height * 0.25);
  await expect(preview).toHaveAttribute("data-side", "top");
  await page.mouse.move(10, 120);
  await expect(preview).toBeHidden();
  await page.mouse.move(area.x + 20, area.y + area.height / 2);
  await expect(preview).toBeVisible();
  await expect(preview).toHaveAttribute("data-side", "left");
  await page.keyboard.press("Escape");
  await expect(preview).toHaveCount(0);
  await page.mouse.up();
  await page.keyboard.up("Control");
});

for (const side of ["left", "right", "top", "bottom"] as const) {
  test(`Ctrl+title drag docks on the ${side}, preserves mounts and streaming, and restores the layout`, async ({
    page,
  }, testInfo) => {
    const { source, target } = await setup(page);
    const pane = page.locator(`[data-pane-id="${source}"]`);
    await expect
      .poll(() =>
        page.evaluate(async () => {
          const { runningTerminal } = await import("/src/terminal-runtime.ts");
          return [
            ...document.querySelectorAll<HTMLElement>("[data-pane-id]"),
          ].every(
            (element) =>
              runningTerminal(element.dataset.paneId!)?.getSnapshot()
                .renderer === "WebGL",
          );
        }),
      )
      .toBe(true);
    await page.evaluate(async () => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      (window as any).__paneMounts = [
        ...document.querySelectorAll<HTMLElement>("[data-pane-id]"),
      ].map((element) => ({
        element,
        runtime: runningTerminal(element.dataset.paneId!),
        canvases: [
          ...element.querySelectorAll(
            ".xterm-screen > canvas:not(.xterm-link-layer)",
          ),
        ],
      }));
    });
    const area = (await page
      .locator(`[data-pane-id="${target}"]`)
      .boundingBox())!;
    await grab(page, source);
    await page.mouse.move(
      area.x +
        area.width * (side === "left" ? 0.1 : side === "right" ? 0.9 : 0.5),
      area.y +
        area.height * (side === "top" ? 0.1 : side === "bottom" ? 0.9 : 0.5),
      { steps: 8 },
    );
    const preview = page.locator(".pane-drop-preview");
    await expect(preview).toHaveAttribute("data-side", side);
    await expect(preview).not.toHaveClass(/is-blocked/);
    await preview.evaluate((element) =>
      Promise.all(
        element.getAnimations().map((animation) => animation.finished),
      ),
    );
    const expected = await preview.boundingBox();
    await page.evaluate(async (source) => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      (window as any).__nativeTest.emit(
        runningTerminal(source)!.sessionId,
        "\r\nOutput during drag 🦀\r\n",
      );
    }, source);
    await expect
      .poll(() => buffer(page, source))
      .toContain("Output during drag 🦀");
    if (side === "left")
      await page.screenshot({
        path: testInfo.outputPath("pane-drag-preview.png"),
      });
    await page.mouse.up();
    await page.keyboard.up("Control");
    await expect(preview).toHaveCount(0);
    await expect(async () => {
      const actual = (await pane.locator("..").boundingBox())!;
      // Dockview rounds proportional group sizes to CSS pixels.
      for (const dimension of ["x", "y", "width", "height"] as const)
        expect(
          Math.abs(actual[dimension] - expected![dimension]),
        ).toBeLessThanOrEqual(1);
    }).toPass();
    await expect(pane).toHaveClass(/is-active/);
    await expect(pane.locator(".xterm-helper-textarea")).toBeFocused();
    expect(
      await page.evaluate(async () => {
        const { runningTerminal } = await import("/src/terminal-runtime.ts");
        return (window as any).__paneMounts.every(
          ({ element, runtime, canvases }: any) =>
            document.querySelector(
              `[data-pane-id="${element.dataset.paneId}"]`,
            ) === element &&
            runningTerminal(element.dataset.paneId) === runtime &&
            canvases.every(
              (canvas: HTMLCanvasElement, index: number) =>
                element.querySelectorAll(
                  ".xterm-screen > canvas:not(.xterm-link-layer)",
                )[index] === canvas,
            ),
        );
      }),
    ).toBe(true);
    await page.evaluate(async (source) => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      (window as any).__nativeTest.emit(
        runningTerminal(source)!.sessionId,
        "\r\nStill streaming after move\r\n",
      );
    }, source);
    await expect
      .poll(() => buffer(page, source))
      .toContain("Still streaming after move");
    const calls = await page.evaluate(() =>
      (window as any).__nativeTest.calls
        .filter((call: any) =>
          ["start_terminal", "close_terminal", "write_terminal"].includes(
            call.command,
          ),
        )
        .map((call: any) => call.command),
    );
    expect(calls).toEqual([
      "start_terminal",
      "start_terminal",
      "start_terminal",
    ]);
    await expect
      .poll(async () => (await savedTab(page))?.layout.second?.axis)
      .toBe(side === "left" || side === "right" ? "horizontal" : "vertical");
    const saved = await savedTab(page);
    await page.reload();
    await expect(page.locator(".xterm-screen")).toHaveCount(3);
    await expect(pane).toHaveClass(/is-active/);
    expect(await savedTab(page)).toEqual(saved);
    await expect
      .poll(() => buffer(page, source))
      .not.toContain("Still streaming after move");
    if (side === "left")
      await page.screenshot({
        path: testInfo.outputPath("pane-drag-result.png"),
      });
  });
}

test("panel motion leaves the workbench still, streams output and resizes each PTY once", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({
    colorScheme: "dark",
    reducedMotion: "no-preference",
  });
  const { source, target } = await setup(page);
  await captureLayoutMotion(page);
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const { runningTerminal } = await import("/src/terminal-runtime.ts");
        const panes = [
          ...document.querySelectorAll<HTMLElement>("[data-pane-id]"),
        ];
        if (
          !panes.every(
            (pane) =>
              runningTerminal(pane.dataset.paneId!)?.getSnapshot().renderer ===
              "WebGL",
          )
        )
          return false;
        (window as any).__paneMounts = panes.map((element) => ({
          element,
          runtime: runningTerminal(element.dataset.paneId!),
          canvases: [
            ...element.querySelectorAll(
              ".xterm-screen > canvas:not(.xterm-link-layer)",
            ),
          ],
        }));
        return true;
      }),
    )
    .toBe(true);
  const callStart = await page.evaluate(
    () => (window as any).__nativeTest.calls.length,
  );
  const motionStart = await layoutMotionCount(page);
  const stage = page.locator(".terminal-layout");
  const before = await stage.boundingBox();
  const area = (await page
    .locator(`[data-pane-id="${target}"]`)
    .boundingBox())!;
  await grab(page, source);
  await page.mouse.move(area.x + area.width - 20, area.y + area.height / 2, {
    steps: 8,
  });
  await page.mouse.up();
  await expect.poll(() => layoutMotionCount(page)).toBeGreaterThan(motionStart);
  const motion = await page.evaluate((start) => {
    const records = (window as any).__layoutMotion.records.slice(start);
    const animations = records.map(({ animation, element, duration }: any) => {
      const style = getComputedStyle(element);
      const matrix =
        style.transform === "none"
          ? new DOMMatrixReadOnly()
          : new DOMMatrixReadOnly(style.transform);
      const frames = (animation.effect as KeyframeEffect).getKeyframes();
      return {
        id: animation.id,
        duration,
        currentTime: Number(animation.currentTime),
        playState: animation.playState,
        className: (element as HTMLElement).className,
        opacity: style.opacity,
        containsTerminalPane: [
          ...document.querySelectorAll<HTMLElement>(
            ".terminal-pane[data-pane-id]",
          ),
        ].some((pane) => element === pane || element.contains(pane)),
        x: matrix.e,
        y: matrix.f,
        scaleX: Math.hypot(matrix.a, matrix.b),
        scaleY: Math.hypot(matrix.c, matrix.d),
        frames: frames.map((frame) => {
          const transform = String(frame.transform ?? "none");
          const frameMatrix =
            transform === "none"
              ? new DOMMatrixReadOnly()
              : new DOMMatrixReadOnly(transform);
          return {
            hasScale: "scale" in frame || /scale\s*\(/i.test(transform),
            opacity: String(frame.opacity),
            x: frameMatrix.e,
            y: frameMatrix.f,
          };
        }),
      };
    });
    const geometry = [
      ...document.querySelectorAll<HTMLElement>(".dock-pane-host"),
    ].map((host) => ({
      paneId: host.querySelector<HTMLElement>("[data-pane-id]")?.dataset.paneId,
      left: host.offsetLeft,
      top: host.offsetTop,
      width: host.offsetWidth,
      height: host.offsetHeight,
    }));
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
          .filter((animation) => animation.id === "lomi-layout-motion").length,
      }));
    return { animations, geometry, terminalTargets };
  }, motionStart);
  expect(motion.animations.length).toBeGreaterThan(0);
  expect(
    motion.animations.every((animation) => !animation.containsTerminalPane),
  ).toBe(true);
  expect(
    motion.terminalTargets.every(
      (target) => target.motionCount === 0 && target.transform === "none",
    ),
  ).toBe(true);
  for (const animation of motion.animations) {
    expect(animation.id).toBe("lomi-layout-motion");
    expect(animation.duration).toBeGreaterThan(0);
    expect(animation.duration).toBeLessThanOrEqual(120);
    expect(animation.currentTime).toBeCloseTo(animation.duration / 2, 0);
    expect(animation.playState).toBe("paused");
    expect(["split-child", "split-divider"]).toContain(animation.className);
    if (animation.className === "split-child") {
      expect(animation.opacity).toBe("1");
      expect(animation.frames.every((frame) => frame.opacity === "1")).toBe(
        true,
      );
    }
    expect(animation.scaleX).toBeCloseTo(1, 2);
    expect(animation.scaleY).toBeCloseTo(1, 2);
    expect(Math.abs(animation.x)).toBeLessThanOrEqual(8.1);
    expect(Math.abs(animation.y)).toBeLessThanOrEqual(8.1);
    expect(animation.frames.length).toBeGreaterThanOrEqual(2);
    expect(animation.frames.every((frame) => !frame.hasScale)).toBe(true);
    for (const frame of animation.frames) {
      expect(Math.abs(frame.x)).toBeLessThanOrEqual(8.1);
      expect(Math.abs(frame.y)).toBeLessThanOrEqual(8.1);
    }
  }
  expect(await stage.boundingBox()).toEqual(before);
  await page.evaluate(async (source) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    (window as any).__nativeTest.emit(
      runningTerminal(source)!.sessionId,
      "\r\nStill streaming during motion\r\n",
    );
  }, source);
  await expect
    .poll(() => buffer(page, source))
    .toContain("Still streaming during motion");
  await page.screenshot({
    path: testInfo.outputPath("pane-motion-midpoint.png"),
  });
  expect(
    await page.evaluate(async () => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      return (window as any).__paneMounts.every(
        ({ element, runtime, canvases }: any) =>
          document.querySelector(
            `[data-pane-id="${element.dataset.paneId}"]`,
          ) === element &&
          runningTerminal(element.dataset.paneId) === runtime &&
          getComputedStyle(element).opacity === "1" &&
          canvases.every(
            (canvas: HTMLCanvasElement, index: number) =>
              element.querySelectorAll(
                ".xterm-screen > canvas:not(.xterm-link-layer)",
              )[index] === canvas,
          ),
      );
    }),
  ).toBe(true);
  await finishLayoutMotion(page, motionStart);
  await expect
    .poll(() =>
      page.evaluate(
        (start) =>
          (window as any).__layoutMotion.records
            .slice(start)
            .every((record: any) => record.animation.playState === "idle"),
        motionStart,
      ),
    )
    .toBe(true);
  expect(
    await page.evaluate(
      (expectedGeometry) =>
        expectedGeometry.every((expected: any) => {
          const host = [
            ...document.querySelectorAll<HTMLElement>(".dock-pane-host"),
          ].find(
            (element) =>
              element.querySelector<HTMLElement>("[data-pane-id]")?.dataset
                .paneId === expected.paneId,
          );
          return (
            host?.offsetLeft === expected.left &&
            host?.offsetTop === expected.top &&
            host?.offsetWidth === expected.width &&
            host?.offsetHeight === expected.height
          );
        }),
      motion.geometry,
    ),
  ).toBe(true);
  const resizes = await page.evaluate(
    (start) =>
      (window as any).__nativeTest.calls
        .slice(start)
        .filter((call: any) => call.command === "resize_terminal")
        .map((call: any) => call.args.id),
    callStart,
  );
  expect(resizes.length).toBeGreaterThan(0);
  expect(resizes.length).toBeLessThanOrEqual(3);
  expect(new Set(resizes).size).toBe(resizes.length);
  await page.keyboard.up("Control");
});

for (const fallback of [
  "reduced-motion",
  "unsupported",
  "animation-error",
] as const) {
  test(`moving a panel still works with ${fallback}`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.emulateMedia({
      reducedMotion: fallback === "reduced-motion" ? "reduce" : "no-preference",
    });
    const { source, target } = await setup(page);
    await captureLayoutMotion(page);
    await page.evaluate((fallback) => {
      const animate = Element.prototype.animate;
      if (fallback === "unsupported")
        Object.defineProperty(Element.prototype, "animate", {
          configurable: true,
          value: undefined,
        });
      if (fallback === "animation-error")
        Element.prototype.animate = function (keyframes, options) {
          if (
            typeof options === "object" &&
            options !== null &&
            options.id === "lomi-layout-motion"
          )
            throw new Error("Animation creation failed");
          return animate.call(this, keyframes, options);
        };
    }, fallback);
    const area = (await page
      .locator(`[data-pane-id="${target}"]`)
      .boundingBox())!;
    await grab(page, source);
    await page.mouse.move(area.x + 20, area.y + area.height / 2, { steps: 8 });
    if (fallback === "reduced-motion") {
      await page.mouse.move(area.x + area.width / 2, area.y + 20);
      await page.mouse.move(area.x + 20, area.y + area.height / 2);
      expect(
        await page
          .locator(".pane-drop-preview")
          .evaluate((element) => element.getAnimations().length),
      ).toBe(0);
    }
    await page.mouse.up();
    await page.keyboard.up("Control");
    await expect
      .poll(async () => (await savedTab(page))?.layout.second?.first?.id)
      .toBe(source);
    expect(await layoutMotionCount(page)).toBe(0);
    expect(errors).toEqual([]);
  });
}

test("Ctrl is required; Escape, releasing Ctrl, lost capture, blur and outside drops cancel", async ({
  page,
}) => {
  const { source, target, layout } = await setup(page);
  await page.evaluate(async (source) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    (window as any).__nativeTest.emit(
      runningTerminal(source)!.sessionId,
      "\x1b]133;C\x07\x1b]2;Source title\x07",
    );
  }, source);
  const title = page.locator(`[data-pane-id="${source}"] .terminal-title`);
  await expect(title).toHaveText("Source title");
  const area = (await page
    .locator(`[data-pane-id="${target}"]`)
    .boundingBox())!;
  for (const cancel of [
    "no-control",
    "escape",
    "control-up",
    "pointercancel",
    "lostcapture",
    "blur",
    "outside",
    "self",
  ]) {
    if (cancel === "no-control") {
      await page.keyboard.press("Control");
      await title.hover();
      await page.mouse.down();
    } else await grab(page, source);
    await page.mouse.move(area.x + 20, area.y + area.height / 2, { steps: 5 });
    if (cancel === "no-control")
      await expect(page.locator(".pane-drag-ghost")).toHaveCount(0);
    else await expect(page.locator(".pane-drop-preview")).toBeVisible();
    if (cancel === "escape") await page.keyboard.press("Escape");
    if (cancel === "control-up") await page.keyboard.up("Control");
    if (cancel === "pointercancel")
      await title.dispatchEvent("pointercancel", { pointerId: 1 });
    if (cancel === "lostcapture")
      await title
        .locator("..")
        .evaluate((element) => element.releasePointerCapture(1));
    if (cancel === "blur")
      await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    if (cancel === "outside") await page.mouse.move(10, 120);
    if (cancel === "self") await title.hover();
    await page.mouse.up();
    await page.keyboard.up("Control");
    await expect(
      page.locator(".pane-drag-ghost, .pane-drop-preview"),
    ).toHaveCount(0);
    await expect
      .poll(async () => (await savedTab(page))?.layout)
      .toEqual(layout);
  }
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "write_terminal",
      ),
    ),
  ).toEqual([]);
  await page.keyboard.down("Control");
  await page
    .locator(`[data-pane-id="${source}"]`)
    .getByRole("button", { name: "Maximize terminal", exact: true })
    // Keyboard activation keeps Control held without macOS turning the click into a context menu.
    .press("Space");
  await page.keyboard.up("Control");
  await expect(page.locator("[data-pane-id]")).toHaveCount(1);
  await expect(page.locator(".pane-drag-ghost")).toHaveCount(0);
  await page.getByRole("button", { name: "Restore terminal size" }).click();
  await expect(page.locator("[data-pane-id]")).toHaveCount(3);
});

test("rejects an arrangement that cannot fit and cleans up on tab switches", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 420 });
  const { source, target, layout } = await setup(page);
  const area = (await page
    .locator(`[data-pane-id="${target}"]`)
    .boundingBox())!;
  await grab(page, source);
  await page.mouse.move(area.x + area.width / 2, area.y + area.height - 10, {
    steps: 6,
  });
  await expect(page.locator(".pane-drop-preview")).toHaveText(
    "Not enough room for these panels",
  );
  await page.mouse.up();
  await page.keyboard.up("Control");
  await expect.poll(async () => (await savedTab(page))?.layout).toEqual(layout);
  await grab(page, source);
  await page.mouse.move(area.x + 10, area.y + area.height / 2, { steps: 6 });
  await expect(page.locator(".pane-drop-preview")).toBeVisible();
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(
    page.locator(".pane-drag-ghost, .pane-drop-preview"),
  ).toHaveCount(0);
  await page.mouse.up();
  await page.keyboard.up("Control");
  await expect.poll(async () => (await savedTab(page))?.layout).toEqual(layout);
});
