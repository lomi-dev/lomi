import { expect, test, type Page } from "@playwright/test";
import { newPane, newProject, newSession } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";
import { defaultTerminalPreferences } from "../../src/terminal-preferences";
import {
  captureLayoutMotion,
  finishLayoutMotion,
  layoutMotionCount,
  layoutMotionRecords,
} from "./layout-motion";

test.beforeEach(async ({ page }) => {
  await page.addInitScript((defaults) => {
    localStorage.setItem(
      "test-terminal-preferences",
      JSON.stringify({ version: 1, ...defaults, alwaysShowTitles: true }),
    );
  }, defaultTerminalPreferences);
});

async function prepare(page: Page, renderer = "WebGL") {
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
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  const second = newPane("/project");
  const ids = [tab.activePaneId, second.id];
  tab.layout = {
    type: "split",
    id: "split",
    axis: "horizontal",
    ratio: 0.5,
    first: tab.layout,
    second,
  };
  await mockDesktop(page, true, {
    ...newSession(),
    sidebar: null,
    projects: [project],
    activeProjectId: project.id,
  });
  await page.goto("/");
  await expect
    .poll(() =>
      page.evaluate(
        async ({ ids, renderer }) => {
          const { runningTerminal } = await import("/src/terminal-runtime.ts");
          return ids.every(
            (id) =>
              runningTerminal(id)?.getSnapshot().renderer === renderer &&
              runningTerminal(id)?.getSnapshot().status === "running",
          );
        },
        { ids, renderer },
      ),
    )
    .toBe(true);
  for (const id of ids)
    await expect.poll(() => buffer(page, id)).toContain("bash $ ");
  await page.evaluate(async (ids) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const panes = ids.map((id, index) => {
      const runtime = runningTerminal(id)!;
      runtime.terminal.options.cursorBlink = false;
      (window as any).__nativeTest.emit(
        runtime.sessionId,
        `\x1b]133;C\x07\x1b]2;Working terminal ${index + 1}\x07\x1b[?25l\x1b[2J\x1b[H` +
          Array.from(
            { length: 30 },
            (_, line) =>
              `${line + 1}. A long output line that wraps when Source Control opens: abcdefghijklmnopqrstuvwxyz 0123456789\r\n`,
          ).join("") +
          "LAST PROMPT> ",
      );
      return {
        id,
        runtime,
        element: document.querySelector(`[data-pane-id="${id}"]`),
        canvases: [...runtime.host.querySelectorAll("canvas")],
      };
    });
    (window as any).__contentMotion = { panes };
  }, ids);
  for (const id of ids)
    await expect.poll(() => buffer(page, id)).toContain("LAST PROMPT>");
  await expect(page.locator(".terminal-title").first()).toHaveText(
    "Working terminal 1",
  );
  await captureLayoutMotion(page);
  return ids;
}

async function waitMotion(page: Page, start: number) {
  await expect.poll(() => layoutMotionCount(page)).toBeGreaterThan(start);
}

async function changedPixels(page: Page, before: Buffer, after: Buffer) {
  return page.evaluate(
    async (images) => {
      const pixels = await Promise.all(
        images.map(async (png) => {
          const image = new Image();
          image.src = `data:image/png;base64,${png}`;
          await image.decode();
          const canvas = document.createElement("canvas");
          canvas.width = image.width;
          canvas.height = image.height;
          const context = canvas.getContext("2d")!;
          context.drawImage(image, 0, 0);
          return context.getImageData(0, 0, image.width, image.height).data;
        }),
      );
      if (pixels[0].length !== pixels[1].length)
        throw new Error("Image dimensions changed");
      let changed = 0;
      for (let i = 0; i < pixels[0].length; i += 4)
        if (
          [0, 1, 2].some(
            (channel) =>
              Math.abs(pixels[0][i + channel] - pixels[1][i + channel]) > 20,
          )
        )
          changed++;
      return changed / (pixels[0].length / 4);
    },
    [before.toString("base64"), after.toString("base64")],
  );
}

async function titleCenters(page: Page) {
  return page.locator(".terminal-title-box").evaluateAll((titles) =>
    titles.map((title) => {
      const pane = title.closest(".split-child") as HTMLElement;
      const paneRect = pane.getBoundingClientRect();
      const titleRect = title.getBoundingClientRect();
      return {
        pane: paneRect.left + paneRect.width / 2,
        title: titleRect.left + titleRect.width / 2,
      };
    }),
  );
}

async function paneDimensions(page: Page) {
  return page.locator(".dock-pane-host > .split-child").evaluateAll((panes) =>
    panes.map((pane) => {
      const rect = pane.getBoundingClientRect();
      return {
        width: rect.width,
        height: rect.height,
        layoutWidth: (pane as HTMLElement).offsetWidth,
        layoutHeight: (pane as HTMLElement).offsetHeight,
      };
    }),
  );
}

async function finalFrameGeometry(page: Page, start: number) {
  return page.evaluate((start) => {
    const rect = (element: Element) => {
      const box = element.getBoundingClientRect();
      return { x: box.x, y: box.y, width: box.width, height: box.height };
    };
    const records = (window as any).__layoutMotion.records.slice(start);
    const rows = [...document.querySelectorAll(".xterm-rows > div")].map(rect);
    const effects = records.map(({ animation, element }: any) => {
      const style = getComputedStyle(element);
      const transform = new DOMMatrixReadOnly(style.transform);
      return {
        x: transform.m41,
        y: transform.m42,
        scaleX: Math.hypot(transform.m11, transform.m12),
        scaleY: Math.hypot(transform.m21, transform.m22),
        opacity: Number(style.opacity),
        currentTime: animation.currentTime,
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
          .filter((animation) => animation.id === "lomi-layout-motion").length,
      }));
    const retainedTargets = records.filter(({ element }: any) =>
      (window as any).__contentMotion.panes.some(
        (pane: any) =>
          element === pane.element || element.contains(pane.element),
      ),
    ).length;
    return { rows, effects, terminalTargets, retainedTargets };
  }, start);
}

async function expectTitlesCentered(page: Page) {
  for (const center of await titleCenters(page))
    expect(Math.abs(center.pane - center.title)).toBeLessThan(1);
}

for (const renderer of ["WebGL", "DOM"]) {
  test(`${renderer}: wrapped terminal output stays live and titles stay centered through the final sidebar frame`, async ({
    page,
  }, info) => {
    const ids = await prepare(page, renderer);
    for (const [index, action] of ["open", "close"].entries()) {
      const opening = action === "open";
      const motionStart = await layoutMotionCount(page);
      await page
        .getByRole("button", { name: /^Toggle source control/ })
        .click();
      await page.mouse.move(10, 10);
      if (opening) await waitMotion(page, motionStart);
      const records = await layoutMotionRecords(page, motionStart);
      if (opening) expect(records.length).toBeGreaterThan(0);
      expect(
        records.every(
          (record) => record.duration > 0 && record.duration <= 120,
        ),
      ).toBe(true);

      const dimensionsDuring = await paneDimensions(page);
      for (const pane of dimensionsDuring) {
        expect(Math.abs(pane.width - pane.layoutWidth)).toBeLessThan(1);
        expect(Math.abs(pane.height - pane.layoutHeight)).toBeLessThan(1);
      }
      const terminalMotion = await page.evaluate((start) => {
        const panes = (window as any).__contentMotion.panes;
        const records = (window as any).__layoutMotion.records.slice(start);
        const terminalTargets = [
          ...document.querySelectorAll<HTMLElement>(
            ".dock-pane-host > .split-child",
          ),
        ].filter((element) =>
          element.querySelector(".terminal-pane[data-pane-id]"),
        );
        return {
          retainedTargets: records.filter(({ element }: any) =>
            panes.some(
              (pane: any) =>
                element === pane.element || element.contains(pane.element),
            ),
          ).length,
          terminalTargets: terminalTargets.map((element) => ({
            transform: getComputedStyle(element).transform,
            motionCount: element
              .getAnimations()
              .filter((animation) => animation.id === "lomi-layout-motion")
              .length,
          })),
        };
      }, motionStart);
      expect(terminalMotion.retainedTargets).toBe(0);
      expect(
        terminalMotion.terminalTargets.every(
          (target) => target.motionCount === 0 && target.transform === "none",
        ),
      ).toBe(true);
      await expectTitlesCentered(page);

      const stage = page.locator(".terminal-layout");
      const beforeOutput = await stage.screenshot({
        path: info.outputPath(`${action}-midpoint.png`),
      });
      await page.evaluate(async (ids) => {
        const { runningTerminal } = await import("/src/terminal-runtime.ts");
        for (const id of ids)
          (window as any).__nativeTest.emit(
            runningTerminal(id)!.sessionId,
            "\r\nLIVE OUTPUT DURING SIDEBAR MOTION\r\n",
          );
      }, ids);
      for (const id of ids)
        await expect
          .poll(() => buffer(page, id))
          .toContain("LIVE OUTPUT DURING SIDEBAR MOTION");
      const afterOutput = await stage.screenshot({
        path: info.outputPath(`${action}-live-output.png`),
      });
      expect(
        await changedPixels(page, beforeOutput, afterOutput),
      ).toBeGreaterThan(0.01);
      await expectTitlesCentered(page);

      expect(
        await page.evaluate(async () => {
          const { runningTerminal } = await import("/src/terminal-runtime.ts");
          return (window as any).__contentMotion.panes.every(
            (pane: any) =>
              runningTerminal(pane.id) === pane.runtime &&
              document.querySelector(`[data-pane-id="${pane.id}"]`) ===
                pane.element &&
              pane.canvases.every(
                (canvas: HTMLCanvasElement, canvasIndex: number) =>
                  pane.runtime.host.querySelectorAll("canvas")[canvasIndex] ===
                  canvas,
              ),
          );
        }),
      ).toBe(true);

      await page.evaluate((start) => {
        for (const { animation, duration } of (
          window as any
        ).__layoutMotion.records.slice(start))
          animation.currentTime = duration - 0.001;
      }, motionStart);
      const lastFrame = await stage.screenshot({
        path: info.outputPath(`${action}-last-frame.png`),
      });
      const geometryAtLastFrame = await finalFrameGeometry(page, motionStart);
      expect(geometryAtLastFrame.retainedTargets).toBe(0);
      expect(
        geometryAtLastFrame.terminalTargets.every(
          (target) => target.motionCount === 0 && target.transform === "none",
        ),
      ).toBe(true);
      for (const effect of geometryAtLastFrame.effects) {
        expect(Math.abs(effect.x)).toBeLessThan(0.1);
        expect(Math.abs(effect.y)).toBeLessThan(0.1);
        expect(effect.scaleX).toBeCloseTo(1, 3);
        expect(effect.scaleY).toBeCloseTo(1, 3);
        expect(effect.opacity).toBeGreaterThan(0.99);
      }
      const recordCount = await layoutMotionCount(page);
      await finishLayoutMotion(page, motionStart);
      expect(await layoutMotionCount(page)).toBe(recordCount);
      const geometryAfterFinish = await finalFrameGeometry(page, motionStart);
      expect(geometryAfterFinish.retainedTargets).toBe(0);
      expect(
        geometryAfterFinish.terminalTargets.every(
          (target) => target.motionCount === 0 && target.transform === "none",
        ),
      ).toBe(true);
      if (renderer === "DOM") {
        expect(geometryAtLastFrame.rows).toHaveLength(
          geometryAfterFinish.rows.length,
        );
        for (const [rowIndex, row] of geometryAtLastFrame.rows.entries())
          for (const dimension of ["x", "y", "width", "height"] as const)
            expect(row[dimension]).toBeCloseTo(
              geometryAfterFinish.rows[rowIndex][dimension],
              1,
            );
      }
      const settled = await stage.screenshot({
        path: info.outputPath(`${action}-settled.png`),
      });
      if (renderer === "WebGL")
        expect(await changedPixels(page, lastFrame, settled)).toBeLessThan(
          0.015,
        );
      const dimensionsSettled = await paneDimensions(page);
      for (const [paneIndex, pane] of dimensionsSettled.entries()) {
        expect(pane.width).toBeCloseTo(dimensionsDuring[paneIndex].width, 1);
        expect(pane.height).toBeCloseTo(dimensionsDuring[paneIndex].height, 1);
      }
      await expectTitlesCentered(page);
    }
  });
}

test("sidebar toggles update the target immediately, cancel paused effects, and do not queue work", async ({
  page,
}) => {
  await prepare(page);
  const source = page.getByRole("button", { name: /^Toggle source control/ });

  await source.click();
  await expect.poll(() => layoutMotionCount(page)).toBeGreaterThan(0);
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toBeVisible();
  const firstEnd = await layoutMotionCount(page);
  const opened = await page.screenshot();

  await source.click();
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toHaveCount(0);
  await expect.poll(() => layoutMotionCount(page)).toBeGreaterThan(firstEnd);
  const secondEnd = await layoutMotionCount(page);
  expect(
    (await layoutMotionRecords(page, 0))
      .slice(0, firstEnd)
      .every((record) => record.playState === "idle"),
  ).toBe(true);
  const closed = await page.screenshot();
  expect(await changedPixels(page, opened, closed)).toBeGreaterThan(0.01);

  await source.click();
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toBeVisible();
  await expect.poll(() => layoutMotionCount(page)).toBeGreaterThan(secondEnd);
  const latestEnd = await layoutMotionCount(page);
  expect(
    (await layoutMotionRecords(page, firstEnd))
      .slice(0, secondEnd - firstEnd)
      .every((record) => record.playState === "idle"),
  ).toBe(true);

  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  expect(await layoutMotionCount(page)).toBe(latestEnd);
  await finishLayoutMotion(page, secondEnd);
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toBeVisible();
});

test("switching tabs cancels sidebar motion without restoring its prior target", async ({
  page,
}) => {
  await prepare(page);
  const original = await page.getByRole("tab").getAttribute("id");
  const source = page.getByRole("button", { name: /^Toggle source control/ });
  await source.click();
  await expect.poll(() => layoutMotionCount(page)).toBeGreaterThan(0);
  await source.click();
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toHaveCount(0);
  const motionCount = await layoutMotionCount(page);
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(page.getByRole("tab", { selected: true })).not.toHaveAttribute(
    "id",
    original!,
  );
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  expect(
    (await layoutMotionRecords(page, 0)).every(
      (record) => record.playState === "idle",
    ),
  ).toBe(true);
  expect(await layoutMotionCount(page)).toBe(motionCount);
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toHaveCount(0);
});

test("a command finishing during sidebar motion updates its heading and controls live", async ({
  page,
}) => {
  const ids = await prepare(page);
  const pane = page.locator(`[data-pane-id="${ids[0]}"]`);
  await page.getByRole("button", { name: /^Toggle source control/ }).click();
  await expect.poll(() => layoutMotionCount(page)).toBeGreaterThan(0);
  await page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    (window as any).__nativeTest.emit(
      runningTerminal(id)!.sessionId,
      "\x1b]133;D;0\x07",
    );
  }, ids[0]);
  await expect(pane.locator(".terminal-heading")).toHaveCSS("opacity", "0");
  await expect(
    pane.getByRole("button", { name: "Maximize terminal" }),
  ).toHaveCount(0);
  await finishLayoutMotion(page);
});
