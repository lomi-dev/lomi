import { expect, test, type Page } from "@playwright/test";
import { newPane, newProject, newSession, panes } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";
import {
  captureLayoutMotion,
  finishLayoutMotion,
  layoutMotionCount,
  layoutMotionRecords,
} from "./layout-motion";

const panels = [
  { id: "files", name: "file explorer", key: "e" },
  { id: "git", name: "source control", key: "g" },
  { id: "workspaces", name: "workspaces", key: null },
] as const;

async function paneGeometry(page: Page) {
  return page
    .locator(".dock-pane-host > .split-child")
    .evaluateAll((elements) =>
      elements.map((element) => {
        const rect = element.getBoundingClientRect();
        return {
          x: rect.x,
          width: rect.width,
          height: rect.height,
          layoutWidth: (element as HTMLElement).offsetWidth,
          layoutHeight: (element as HTMLElement).offsetHeight,
        };
      }),
    );
}

async function resizeCounts(page: Page, ids: string[], start: number) {
  return page.evaluate(
    async ({ ids, start }) => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      const calls = (window as any).__nativeTest.calls.slice(start);
      return ids.map((id) => {
        const sessionId = runningTerminal(id)?.sessionId;
        return calls.filter(
          (call: any) =>
            call.command === "resize_terminal" && call.args.id === sessionId,
        ).length;
      });
    },
    { ids, start },
  );
}

async function dockHostGeometry(page: Page) {
  return page.locator(".dock-pane-host").evaluateAll((elements) =>
    elements.map((element) => {
      const rect = element.getBoundingClientRect();
      return {
        width: rect.width,
        height: rect.height,
        layoutWidth: (element as HTMLElement).offsetWidth,
        layoutHeight: (element as HTMLElement).offsetHeight,
      };
    }),
  );
}

async function twoFrames(page: Page) {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
}

for (const side of ["left", "right"] as const) {
  for (const panel of panels) {
    test(`${panel.id} on the ${side} keeps live panes at final size while opening and closing`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize({ width: 1100, height: 620 });
      await page.emulateMedia({
        colorScheme: "dark",
        reducedMotion: "no-preference",
      });
      const project = newProject("/project", "local:bash");
      const tab = project.workspaces[0].tabs[0];
      if (tab.type !== "terminal") throw new Error("Expected terminal tab");
      if (panel.id !== "workspaces")
        tab.layout = {
          type: "split",
          id: "split",
          axis: "horizontal",
          ratio: 0.5,
          first: tab.layout,
          second: newPane("/project"),
        };
      const ids = panes(tab.layout).map((pane) => pane.id);
      const session = newSession();
      await mockDesktop(page, true, {
        ...session,
        sidebar: null,
        sidebarSides: { ...session.sidebarSides, [panel.id]: side },
        projects: [project],
        activeProjectId: project.id,
      });
      await page.goto("/");
      await expect(page.locator(".xterm-screen")).toHaveCount(ids.length);
      await expect
        .poll(() =>
          page.evaluate(async (ids) => {
            const { runningTerminal } =
              await import("/src/terminal-runtime.ts");
            return ids.every(
              (id) =>
                runningTerminal(id)?.getSnapshot().renderer === "WebGL" &&
                runningTerminal(id)?.getSnapshot().status === "running",
            );
          }, ids),
        )
        .toBe(true);
      await page.evaluate(async (ids) => {
        const { runningTerminal } = await import("/src/terminal-runtime.ts");
        (window as any).__sidebarMotion = {
          panes: ids.map((id) => ({
            id,
            runtime: runningTerminal(id),
            element: document.querySelector(`[data-pane-id="${id}"]`),
            canvases: [...runningTerminal(id)!.host.querySelectorAll("canvas")],
          })),
        };
      }, ids);
      await captureLayoutMotion(page);

      for (const opening of [true, false]) {
        const before = await paneGeometry(page);
        const hostsBefore = await dockHostGeometry(page);
        const motionStart = await layoutMotionCount(page);
        const callStart = await page.evaluate(
          () => (window as any).__nativeTest.calls.length,
        );
        if (opening || !panel.key)
          await page
            .getByRole("button", { name: new RegExp(`^Toggle ${panel.name}`) })
            .click();
        else await page.keyboard.press(`Control+Shift+${panel.key}`);
        if (opening)
          await expect
            .poll(() => layoutMotionCount(page))
            .toBeGreaterThan(motionStart);
        const records = await layoutMotionRecords(page, motionStart);
        expect(
          records.every((record) => record.id === "lomi-layout-motion"),
        ).toBe(true);
        expect(
          records.every(
            (record) => record.duration > 0 && record.duration <= 120,
          ),
        ).toBe(true);
        expect(
          records.every(
            (record) =>
              !record.className.split(/\s+/).includes("dock-pane-host"),
          ),
        ).toBe(true);
        expect(
          records.every(
            (record) => !record.className.split(/\s+/).includes("split-child"),
          ),
        ).toBe(true);
        expect(
          await page.evaluate(() =>
            [
              ...document.querySelectorAll<HTMLElement>(
                ".dock-pane-host > .split-child",
              ),
            ]
              .filter((element) =>
                element.querySelector(".terminal-pane[data-pane-id]"),
              )
              .every(
                (element) =>
                  element
                    .getAnimations()
                    .every(
                      (animation) => animation.id !== "lomi-layout-motion",
                    ) && getComputedStyle(element).transform === "none",
              ),
          ),
        ).toBe(true);
        await expect
          .poll(() => resizeCounts(page, ids, callStart))
          .toEqual(ids.map(() => 1));

        const during = await paneGeometry(page);
        const hostsDuring = await dockHostGeometry(page);
        expect(during).toHaveLength(before.length);
        expect(hostsDuring).toHaveLength(hostsBefore.length);
        for (const [index, pane] of during.entries()) {
          expect(Math.abs(pane.width - pane.layoutWidth)).toBeLessThan(1);
          expect(Math.abs(pane.height - pane.layoutHeight)).toBeLessThan(1);
          expect(
            opening
              ? pane.width < before[index].width
              : pane.width > before[index].width,
          ).toBe(true);
        }
        await page.screenshot({
          path: testInfo.outputPath(
            `${opening ? "open" : "close"}-midpoint.png`,
          ),
        });
        await page.evaluate(
          (text) => {
            const state = (window as any).__sidebarMotion;
            for (const pane of state.panes)
              (window as any).__nativeTest.emit(
                pane.runtime.sessionId,
                `\r\n${text}\r\n`,
              );
          },
          `Streaming while ${opening ? "opening" : "closing"}`,
        );
        for (const id of ids)
          await expect
            .poll(() => buffer(page, id))
            .toContain(`Streaming while ${opening ? "opening" : "closing"}`);

        const pausedCount = await layoutMotionCount(page);
        await finishLayoutMotion(page, motionStart);
        await expect.poll(() => layoutMotionCount(page)).toBe(pausedCount);
        const settled = await paneGeometry(page);
        const hostsSettled = await dockHostGeometry(page);
        for (const [index, pane] of settled.entries()) {
          expect(pane.width).toBeCloseTo(during[index].width, 1);
          expect(pane.height).toBeCloseTo(during[index].height, 1);
        }
        for (const [index, host] of hostsDuring.entries()) {
          expect(Math.abs(host.width - host.layoutWidth)).toBeLessThan(1);
          expect(Math.abs(host.height - host.layoutHeight)).toBeLessThan(1);
          expect(host.width).toBeCloseTo(hostsSettled[index].width, 1);
          expect(host.height).toBeCloseTo(hostsSettled[index].height, 1);
        }
      }

      expect(
        await page.evaluate(async () => {
          const { runningTerminal } = await import("/src/terminal-runtime.ts");
          return (window as any).__sidebarMotion.panes.every(
            (pane: any) =>
              runningTerminal(pane.id) === pane.runtime &&
              document.querySelector(`[data-pane-id="${pane.id}"]`) ===
                pane.element &&
              pane.canvases.every(
                (canvas: HTMLCanvasElement, index: number) =>
                  pane.runtime.host.querySelectorAll("canvas")[index] ===
                  canvas,
              ),
          );
        }),
      ).toBe(true);
      expect(
        await page.evaluate(() =>
          (window as any).__nativeTest.calls
            .filter((call: any) =>
              ["start_terminal", "close_terminal"].includes(call.command),
            )
            .map((call: any) => call.command),
        ),
      ).toEqual(ids.map(() => "start_terminal"));
      if (panel.id !== "workspaces") {
        await expect
          .poll(() =>
            page.evaluate(() =>
              (window as any).__nativeTest.calls
                .filter((call: any) => call.command === "save_session")
                .map(
                  (call: any) =>
                    call.args.data.projects[0].workspaces[0].tabs[0].layout
                      .ratio,
                ),
            ),
          )
          .toContain(0.5);
        expect(
          await page.evaluate(() =>
            (window as any).__nativeTest.calls
              .filter((call: any) => call.command === "save_session")
              .map(
                (call: any) =>
                  call.args.data.projects[0].workspaces[0].tabs[0].layout.ratio,
              )
              .every((ratio: number) => ratio === 0.5),
          ),
        ).toBe(true);
      }
    });
  }
}

for (const mode of ["reduced motion", "unsupported"] as const) {
  test(`sidebar changes immediately without effects for ${mode}`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.emulateMedia({
      reducedMotion: mode === "reduced motion" ? "reduce" : "no-preference",
    });
    const session = newSession();
    const project = newProject("/project", "local:bash");
    session.projects = [project];
    session.activeProjectId = project.id;
    session.sidebar = null;
    await mockDesktop(page, true, session);
    await page.goto("/");
    await expect(page.locator(".xterm-screen")).toBeVisible();
    await captureLayoutMotion(page);
    if (mode === "unsupported")
      await page.evaluate(() => {
        const workArea = document.querySelector(".work-area");
        if (!workArea) throw new Error("Expected work area");
        Object.defineProperty(workArea, "animate", {
          configurable: true,
          value: undefined,
        });
      });
    const button = page.getByRole("button", { name: /^Toggle file explorer/ });
    await button.click();
    await expect(
      page.getByRole("complementary", { name: "Explorer" }),
    ).toBeVisible();
    await expect.poll(() => layoutMotionCount(page)).toBe(0);

    const divider = page.getByRole("separator", {
      name: "Resize sidebar",
      exact: true,
    });
    await divider.press("ArrowRight");
    await expect(divider).toHaveAttribute("aria-valuenow", "270");
    await expect(
      page.getByRole("complementary", { name: "Explorer" }),
    ).toBeVisible();
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) =>
            ["start_terminal", "close_terminal"].includes(call.command),
          )
          .map((call: any) => call.command),
      ),
    ).toEqual(["start_terminal"]);
    expect(errors).toEqual([]);
  });
}

test("a paused sidebar animation is canceled on every toggle and the latest target wins", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await captureLayoutMotion(page);
  const button = page.getByRole("button", { name: /^Toggle source control/ });

  await button.click();
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toBeVisible();
  const firstEnd = await layoutMotionCount(page);
  expect(firstEnd).toBeGreaterThan(0);

  await button.click();
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toHaveCount(0);
  const secondEnd = await layoutMotionCount(page);
  expect(
    (await layoutMotionRecords(page, 0))
      .slice(0, firstEnd)
      .every((record) => record.playState === "idle"),
  ).toBe(true);

  await button.click();
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toBeVisible();
  const thirdEnd = await layoutMotionCount(page);
  expect(thirdEnd).toBeGreaterThan(secondEnd);
  expect(
    (await layoutMotionRecords(page, firstEnd))
      .slice(0, secondEnd - firstEnd)
      .every((record) => record.playState === "idle"),
  ).toBe(true);

  await twoFrames(page);
  expect(await layoutMotionCount(page)).toBe(thirdEnd);
  await finishLayoutMotion(page, secondEnd);
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toBeVisible();
  expect(errors).toEqual([]);
});

test("switching tabs cancels paused sidebar effects without restoring the old target", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await captureLayoutMotion(page);
  const original = await page.getByRole("tab").getAttribute("id");
  const source = page.getByRole("button", { name: /^Toggle source control/ });
  await source.click();
  await expect.poll(() => layoutMotionCount(page)).toBeGreaterThan(0);
  const openEnd = await layoutMotionCount(page);
  await source.click();
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toHaveCount(0);
  const closeEnd = await layoutMotionCount(page);
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(page.getByRole("tab", { selected: true })).not.toHaveAttribute(
    "id",
    original!,
  );
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
  await twoFrames(page);
  expect(
    (await layoutMotionRecords(page, 0)).every(
      (record) => record.playState === "idle",
    ),
  ).toBe(true);
  expect(await layoutMotionCount(page)).toBe(closeEnd);
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toHaveCount(0);
});

test("native browser panes keep stable final bounds and their ancestors are not animated", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1100, height: 620 });
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const project = newProject("/project", "local:bash");
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  const browser = {
    type: "browser" as const,
    id: "browser-motion-test",
    title: "Browser",
    url: "https://example.com/",
  };
  tab.layout = {
    type: "split",
    id: "browser-split",
    axis: "horizontal",
    ratio: 0.5,
    first: tab.layout,
    second: browser,
  };
  await mockDesktop(page, false, {
    ...newSession(),
    sidebar: null,
    projects: [project],
    activeProjectId: project.id,
  });
  await page.goto("/");
  const browserPane = page.locator(".browser-pane");
  await expect(browserPane).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.browsers.get("browser-motion-test")
            ?.visible,
      ),
    )
    .toBe(true);
  await captureLayoutMotion(page);
  const initialBounds = await page.evaluate(
    () =>
      (window as any).__nativeTest.browsers.get("browser-motion-test").bounds,
  );
  await page.getByRole("button", { name: /^Toggle file explorer/ }).click();
  await expect.poll(() => layoutMotionCount(page)).toBeGreaterThan(0);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const bounds = (window as any).__nativeTest.browsers.get(
          "browser-motion-test",
        )?.bounds;
        return bounds?.width !== undefined && bounds.width > 1;
      }),
    )
    .toBe(true);
  const motionData = await page.evaluate(() => {
    const pane = document.querySelector(".browser-pane")!;
    const slot = document.querySelector(".browser-viewport")!;
    const bounds = (window as any).__nativeTest.browsers.get(
      "browser-motion-test",
    ).bounds;
    const rect = slot.getBoundingClientRect();
    const ancestors = new Set<Element>();
    for (
      let element: Element | null = pane;
      element;
      element = element.parentElement
    )
      ancestors.add(element);
    return {
      animatedAncestor: (window as any).__layoutMotion.records.some(
        (record: any) => ancestors.has(record.element),
      ),
      bounds,
      rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
    };
  });
  expect(motionData.animatedAncestor).toBe(false);
  expect(motionData.bounds.x).toBeCloseTo(motionData.rect.x, 1);
  expect(motionData.bounds.y).toBeCloseTo(motionData.rect.y, 1);
  expect(motionData.bounds.width).toBeCloseTo(motionData.rect.width, 1);
  expect(motionData.bounds.height).toBeCloseTo(motionData.rect.height, 1);
  expect(motionData.bounds.width).toBeLessThan(initialBounds.width);
  await finishLayoutMotion(page);
  const settledBounds = await page.evaluate(
    () =>
      (window as any).__nativeTest.browsers.get("browser-motion-test").bounds,
  );
  expect(settledBounds).toEqual(motionData.bounds);
});
