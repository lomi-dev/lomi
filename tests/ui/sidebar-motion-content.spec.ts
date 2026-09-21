import { expect, test, type Page } from "@playwright/test";
import { newPane, newProject, newSession } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";
import { defaultTerminalPreferences } from "../../src/terminal-preferences";

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
    for (const [index, id] of ids.entries()) {
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
    }
    const state = ((window as any).__contentMotion = {
      captures: [] as Animation[][],
      starts: 0,
      skips: 0,
    });
    const start = document.startViewTransition.bind(document);
    document.startViewTransition = (update) => {
      state.starts++;
      const transition = start(update);
      const skip = transition.skipTransition.bind(transition);
      transition.skipTransition = () => {
        state.skips++;
        skip();
      };
      void transition.ready
        .then(() => {
          const animations = document
            .getAnimations()
            .filter((animation) =>
              (animation.effect as KeyframeEffect).pseudoElement?.startsWith(
                "::view-transition",
              ),
            );
          for (const animation of animations) {
            animation.pause();
            animation.currentTime = 90;
          }
          state.captures.push(animations);
        })
        .catch(() => {});
      return transition;
    };
  }, ids);
  for (const id of ids)
    await expect.poll(() => buffer(page, id)).toContain("LAST PROMPT>");
  await expect(page.locator(".terminal-title").first()).toHaveText(
    "Working terminal 1",
  );
  return ids;
}

async function waitMotion(page: Page, count: number) {
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__contentMotion.captures.length),
    )
    .toBe(count);
}

async function finishMotion(page: Page) {
  await page.evaluate(() => {
    for (const animation of (window as any).__contentMotion.captures.at(-1))
      animation.finish();
  });
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

for (const renderer of ["WebGL", "DOM"]) {
  test(`${renderer}: wrapped text stays live and titles stay centered through the last sidebar animation frame`, async ({
    page,
  }, info) => {
    const ids = await prepare(page, renderer);
    for (const [index, action] of ["open", "close"].entries()) {
      await page
        .getByRole("button", { name: /^Toggle source control/ })
        .click();
      await page.mouse.move(10, 10);
      await waitMotion(page, index + 1);
      const centers = await page
        .locator(".terminal-title-box")
        .evaluateAll((titles) =>
          titles.map((title) => {
            const pane = title.closest(".split-child") as HTMLElement;
            const center = (element: HTMLElement) => {
              const style = getComputedStyle(
                document.documentElement,
                `::view-transition-group(${element.style.viewTransitionName})`,
              );
              return (
                new DOMMatrixReadOnly(style.transform).m41 +
                parseFloat(style.width) / 2
              );
            };
            return { pane: center(pane), title: center(title as HTMLElement) };
          }),
        );
      for (const center of centers)
        expect(Math.abs(center.pane - center.title)).toBeLessThan(1);
      const stage = page.locator(".terminal-layout");
      const beforeOutput = await stage.screenshot();
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
        path: info.outputPath(`${action}-live.png`),
      });
      expect(
        await changedPixels(page, beforeOutput, afterOutput),
      ).toBeGreaterThan(0.01);
      await page.evaluate(() => {
        for (const animation of (window as any).__contentMotion.captures.at(-1))
          animation.currentTime = 179.999;
      });
      const lastFrame = await stage.screenshot({
        path: info.outputPath(`${action}-last-frame.png`),
      });
      await finishMotion(page);
      await expect(page.locator("html")).not.toHaveClass(/moving-panes/);
      const settled = await stage.screenshot({
        path: info.outputPath(`${action}-settled.png`),
      });
      expect(await changedPixels(page, lastFrame, settled)).toBeLessThan(0.015);
    }
  });
}

test("sidebar changes requested halfway through motion finish continuously and keep the latest target", async ({
  page,
}) => {
  await prepare(page);
  const source = page.getByRole("button", { name: /^Toggle source control/ });
  await source.click();
  await waitMotion(page, 1);
  const before = await page.locator(".terminal-layout").screenshot();
  await source.evaluate((button) => {
    button.click();
    button.click();
    button.click();
  });
  const after = await page.locator(".terminal-layout").screenshot();
  expect(await changedPixels(page, before, after)).toBeLessThan(0.001);
  expect(
    await page.evaluate(() => ({
      starts: (window as any).__contentMotion.starts,
      skips: (window as any).__contentMotion.skips,
    })),
  ).toEqual({ starts: 1, skips: 0 });
  await finishMotion(page);
  await waitMotion(page, 2);
  await finishMotion(page);
  await expect(page.locator("html")).not.toHaveClass(/moving-panes/);
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: /^Toggle file explorer/ }).click();
  await waitMotion(page, 3);
  await finishMotion(page);
  await expect(page.locator("html")).not.toHaveClass(/moving-panes/);
  await source.click();
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => (window as any).__contentMotion.starts),
  ).toBe(3);
});

test("switching tabs cancels pending sidebar motion without rendering the old tab again", async ({
  page,
}) => {
  await prepare(page);
  const original = await page.getByRole("tab").getAttribute("id");
  const source = page.getByRole("button", { name: /^Toggle source control/ });
  await source.click();
  await waitMotion(page, 1);
  await source.click();
  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(page.getByRole("tab", { selected: true })).not.toHaveAttribute(
    "id",
    original!,
  );
  await expect(page.locator("html")).not.toHaveClass(/moving-panes/);
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
  expect(
    await page.evaluate(() => (window as any).__contentMotion.starts),
  ).toBe(1);
  await expect(
    page.getByRole("complementary", { name: "Source Control", exact: true }),
  ).toHaveCount(0);
});

test("a command finishing during sidebar motion hides its heading without interrupting the transition", async ({
  page,
}) => {
  const ids = await prepare(page);
  const pane = page.locator(`[data-pane-id="${ids[0]}"]`);
  await page.getByRole("button", { name: /^Toggle source control/ }).click();
  await waitMotion(page, 1);
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
  await expect(page.locator("html")).toHaveClass(/moving-sidebars/);
  expect(
    await pane
      .locator(".terminal-title-box")
      .evaluate((element) => element.style.viewTransitionName),
  ).not.toBe("");
  await finishMotion(page);
  await expect(page.locator("html")).not.toHaveClass(/moving-panes/);
});
