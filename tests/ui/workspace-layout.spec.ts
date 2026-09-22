import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { addWorkspace, newSession } from "../../src/model";
import { mockDesktop } from "./desktop";

async function expectWorkspaceFits(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(() => {
        const area = document
          .querySelector(".work-area")!
          .getBoundingClientRect();
        return [
          ...document.querySelectorAll<HTMLElement>(
            ".work-area, .terminal-stage, .terminal-layout, .split-container, .dock-layout, .dock-pane-host",
          ),
        ].flatMap((element) => {
          const bounds = element.getBoundingClientRect();
          element.scrollTo(element.scrollWidth, element.scrollHeight);
          return element.scrollWidth > element.clientWidth ||
            element.scrollHeight > element.clientHeight ||
            element.scrollLeft !== 0 ||
            element.scrollTop !== 0 ||
            bounds.left < area.left - 1 ||
            bounds.right > area.right + 1 ||
            bounds.top < area.top - 1 ||
            bounds.bottom > area.bottom + 1
            ? [
                {
                  name: element.className,
                  width: element.clientWidth,
                  height: element.clientHeight,
                  scrollWidth: element.scrollWidth,
                  scrollHeight: element.scrollHeight,
                  scrollLeft: element.scrollLeft,
                  scrollTop: element.scrollTop,
                  bounds: bounds.toJSON(),
                  area: area.toJSON(),
                },
              ]
            : [];
        });
      }),
    )
    .toEqual([]);
}

for (const sidebars of [1, 2]) {
  test(`workspace fits ${sidebars} sidebars and a live pane after shrinking and enlarging`, async ({
    page,
  }, testInfo) => {
    const session = addWorkspace(
      newSession(),
      "/project",
      "local:bash",
      "First",
    );
    session.sidebar = "workspaces";
    if (sidebars === 2) {
      session.rightSidebar = "git";
      session.sidebarSides.git = "right";
    }
    await mockDesktop(page, true, session);
    await page.goto("/");
    const screen = page.locator(".xterm-screen");
    await expect(screen).toBeVisible();
    const original = await screen.elementHandle();
    await page.evaluate(() => {
      const native = (window as any).__nativeTest;
      native.emit(
        [...native.sessions.keys()][0],
        "scrollback line\r\n".repeat(100),
      );
      document.documentElement.style.setProperty("--stage-padding", "0.3px");
    });
    for (const [width, height] of [
      [800, 420],
      [533, 280],
      [400, 210],
      [400, 160],
      [1440, 900],
    ]) {
      await page.setViewportSize({ width, height });
      await expectWorkspaceFits(page);
      await expect(screen).toBeVisible();
      const pane = await page.locator(".dock-pane-host").boundingBox();
      const stage = await page.locator(".terminal-stage").boundingBox();
      expect(pane!.width).toBeLessThanOrEqual(stage!.width);
      expect(pane!.height).toBeLessThanOrEqual(stage!.height);
      if (width === 400 && height === 210)
        await page.screenshot({
          path: testInfo.outputPath("workspace-zoomed.png"),
        });
    }
    expect(
      await original!.evaluate(
        (element) => element === document.querySelector(".xterm-screen"),
      ),
    ).toBe(true);
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) =>
            ["start_terminal", "close_terminal"].includes(call.command),
          )
          .map((call: any) => call.command),
      ),
    ).toEqual(["start_terminal"]);
    await page.locator(".terminal-pane").evaluate(async (element) => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      runningTerminal(
        (element as HTMLElement).dataset.paneId!,
      )!.terminal.scrollToTop();
    });
    await page.locator(".terminal-pane").hover();
    await page.mouse.wheel(0, 400);
    await expect
      .poll(() =>
        page.locator(".terminal-pane").evaluate(async (element) => {
          const { runningTerminal } = await import("/src/terminal-runtime.ts");
          return runningTerminal((element as HTMLElement).dataset.paneId!)!
            .terminal.buffer.active.viewportY;
        }),
      )
      .toBeGreaterThan(0);
  });
}

for (const width of [1440, 800]) {
  test(`workspace switches keep the layout stable at ${width}px`, async ({
    page,
  }, testInfo) => {
    let session = newSession();
    for (const [path, name] of [
      ["/work/short", "First"],
      ["/work/short", "Second"],
      ["/work/plain", "Plain"],
      ["/work/a-longer-project-name", "Third"],
    ])
      session = addWorkspace(session, path, "local:bash", name);
    session.sidebar = "workspaces";
    session.rightSidebar = "git";
    session.sidebarSides.git = "right";
    await page.setViewportSize({ width, height: 600 });
    await mockDesktop(page, true, session);
    await page.addInitScript(() => {
      const native = (window as any).__TAURI_INTERNALS__;
      const invoke = native.invoke;
      native.invoke = async (
        command: string,
        args: Record<string, any> = {},
      ) => {
        if (command === "git_status" || command === "git_repositories") {
          await new Promise((resolve) => setTimeout(resolve, 180));
          const status =
            args.root === "/work/plain"
              ? null
              : { root: args.root, branch: args.root, changes: [] };
          return command === "git_repositories"
            ? {
                repositories: status ? [status] : [],
                errors: [],
                limited: false,
              }
            : status;
        }
        return invoke(command, args);
      };
    });
    await page.goto("/");
    await expect(page.locator(".branch-status")).toBeVisible();
    for (const name of ["First", "Second", "Third", "First"]) {
      await page.evaluate(() => {
        const state = ((window as any).__workspaceGeometry = {
          frames: [] as any[],
        });
        const sample = () => {
          state.frames.push(
            Object.fromEntries(
              [
                ".app-shell",
                ".titlebar",
                ".tab-bar",
                ".work-area",
                ".workspace-list",
                ".terminal-stage",
                ".statusbar",
              ].map((selector) => {
                const element = document.querySelector(selector);
                if (!element) return [selector, null];
                const { x, y, width, height } = element.getBoundingClientRect();
                return [
                  selector,
                  {
                    x,
                    y,
                    width,
                    height,
                    scrollX: element.scrollLeft,
                    scrollY: element.scrollTop,
                  },
                ];
              }),
            ),
          );
          if (state.frames.length < 30) requestAnimationFrame(sample);
        };
        sample();
      });
      const row = page
        .getByRole("navigation", { name: "Workspace list" })
        .getByRole("button", { name: new RegExp(`^${name} `) });
      await row.click();
      await expect(row).toHaveAttribute("aria-current", "true");
      await expect(page.locator(".branch-status")).toHaveText(
        name === "Third" ? "/work/a-longer-project-name" : "/work/short",
      );
      await expect
        .poll(() =>
          page.evaluate(
            () => (window as any).__workspaceGeometry.frames.length,
          ),
        )
        .toBe(30);
      const frames = await page.evaluate(
        () => (window as any).__workspaceGeometry.frames,
      );
      const unique = Object.fromEntries(
        Object.keys(frames[0]).map((selector) => [
          selector,
          [
            ...new Set(
              frames.map((frame: any) => JSON.stringify(frame[selector])),
            ),
          ],
        ]),
      );
      for (const selector of Object.keys(unique))
        expect(
          unique[selector],
          `${name}: ${selector} must stay in place`,
        ).toHaveLength(1);
      await testInfo.attach(`${name}-frames`, {
        body: JSON.stringify(frames),
        contentType: "application/json",
      });
    }
    const sourceControl = page.getByRole("complementary", {
      name: "Source Control",
      exact: true,
    });
    await page.getByRole("button", { name: /^Plain / }).click();
    await expect(sourceControl).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: /^Toggle source control/ }),
    ).toHaveCount(0);
    await page.getByRole("button", { name: /^Third / }).click();
    await expect(sourceControl).toBeVisible();
    await expect(page.locator(".branch-status")).toHaveText(
      "/work/a-longer-project-name",
    );
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls.filter(
              (call: any) => call.command === "start_terminal",
            ).length,
        ),
      )
      .toBe(4);
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls.some(
          (call: any) => call.command === "close_terminal",
        ),
      ),
    ).toBe(false);
    await page.screenshot({
      path: testInfo.outputPath("workspace-layout.png"),
    });
  });
}
