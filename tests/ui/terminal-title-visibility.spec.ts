import { expect, test, type Page } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

async function setup(page: Page) {
  await mockDesktop(page, false);
  await page.goto("/");
  const pane = page.locator("[data-pane-id]");
  await expect(pane.locator(".xterm-screen")).toBeVisible();
  const id = (await pane.getAttribute("data-pane-id"))!;
  await expect.poll(() => buffer(page, id)).toContain("bash $ ");
  await page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(id)!;
    (window as any).__nativeTest.emit(
      runtime.sessionId,
      "\x1b]133;C\x07\x1b]2;Terminal title\x07",
    );
    await new Promise<void>((resolve) => runtime.terminal.write("", resolve));
  }, id);
  return pane;
}

async function pauseClock(page: Page) {
  const now = new Date();
  await page.clock.install({ time: now });
  await page.clock.pauseAt(new Date(now.getTime() + 1000));
}

test("titles default to hidden and a Control tap reveals them for five seconds without resizing", async ({
  page,
}, testInfo) => {
  const pane = await setup(page);
  const heading = pane.locator(".terminal-heading");
  const mount = pane.locator(".terminal-mount");
  const bounds = await mount.boundingBox();
  await expect(heading).toHaveAttribute("aria-hidden", "true");
  await expect(heading).toHaveCSS("opacity", "0");
  await page.screenshot({ path: testInfo.outputPath("titles-hidden.png") });
  await pauseClock(page);
  await page.keyboard.down("Control");
  await expect(heading).toHaveAttribute("aria-hidden", "false");
  await expect(heading).toHaveCSS("opacity", "1");
  await page.clock.runFor(100);
  await page.keyboard.up("Control");
  await page.clock.runFor(4899);
  await expect(heading).toHaveAttribute("aria-hidden", "false");
  expect(await mount.boundingBox()).toEqual(bounds);
  await page.screenshot({ path: testInfo.outputPath("titles-revealed.png") });
  await page.clock.runFor(1);
  await expect(heading).toHaveAttribute("aria-hidden", "true");
  expect(await mount.boundingBox()).toEqual(bounds);
});

test("a Control hold stays visible past five seconds and hides immediately on release", async ({
  page,
}) => {
  const heading = (await setup(page)).locator(".terminal-heading");
  await pauseClock(page);
  for (const duration of [1001, 6000]) {
    await page.keyboard.down("ControlRight");
    await page.clock.runFor(duration);
    // Repeated keydowns must not restart the hold threshold.
    await page.keyboard.down("ControlRight");
    await expect(heading).toHaveAttribute("aria-hidden", "false");
    await page.keyboard.up("ControlRight");
    await expect(heading).toHaveAttribute("aria-hidden", "true");
  }
});

test("another tap restarts the timeout, and blur clears a held Control", async ({
  page,
}) => {
  const heading = (await setup(page)).locator(".terminal-heading");
  await pauseClock(page);
  await page.keyboard.press("Control");
  await page.clock.runFor(4000);
  await page.keyboard.press("Control");
  await page.clock.runFor(1000);
  await expect(heading).toHaveAttribute("aria-hidden", "false");
  await page.clock.runFor(4000);
  await expect(heading).toHaveAttribute("aria-hidden", "true");
  await page.keyboard.down("Control");
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(heading).toHaveAttribute("aria-hidden", "true");
  await page.keyboard.up("Control");
  await page.keyboard.press("Control");
  await expect(heading).toHaveAttribute("aria-hidden", "false");
  await page.clock.runFor(5000);
  await expect(heading).toHaveAttribute("aria-hidden", "true");
});

test("all split titles share the reveal and Control shortcuts still reach the terminal", async ({
  page,
}) => {
  await setup(page);
  await page.keyboard.press("Control+d");
  await expect(page.locator("[data-pane-id]")).toHaveCount(2);
  const headings = page.locator(".terminal-heading");
  await expect(headings.nth(0)).toHaveAttribute("aria-hidden", "false");
  await expect(headings.nth(1)).toHaveAttribute("aria-hidden", "false");
  await expect(page.locator(".terminal-title").nth(1)).toHaveText("/project");
  await pauseClock(page);
  await page.keyboard.press("Control+c");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) => call.command === "write_terminal")
          .map((call: any) => call.args.data)
          .join(""),
      ),
    )
    .toBe("\x03");
  await page.clock.runFor(5000);
  for (const heading of await headings.all()) {
    await expect(heading).toHaveAttribute("aria-hidden", "true");
    await expect(heading).toHaveAttribute("inert", "");
  }
  await expect(
    page.getByRole("button", { name: "Maximize terminal", exact: true }),
  ).toHaveCount(0);
});

test("the setting applies across windows, survives reload and resets to hidden", async ({
  page,
  context,
}, testInfo) => {
  const heading = (await setup(page)).locator(".terminal-heading");
  const settings = await context.newPage();
  await mockDesktop(settings, false);
  await settings.goto("/?window=settings&page=terminal");
  const toggle = settings.getByRole("switch", {
    name: "Always show terminal titles",
  });
  await expect(toggle).toBeEnabled();
  await expect(toggle).not.toBeChecked();
  await toggle.check();
  await expect(settings.getByRole("status")).toHaveText("Saved");
  await expect(heading).toHaveAttribute("aria-hidden", "false");
  await page.bringToFront();
  await pauseClock(page);
  await page.keyboard.down("Control");
  await page.clock.runFor(6000);
  await page.keyboard.up("Control");
  await expect(heading).toHaveAttribute("aria-hidden", "false");
  await page.clock.resume();
  await settings.reload();
  await expect(toggle).toBeChecked();
  await settings.setViewportSize({ width: 920, height: 680 });
  await settings.screenshot({ path: testInfo.outputPath("title-setting.png") });
  await settings
    .getByRole("button", {
      name: "Restore default behavior for Always show terminal titles",
    })
    .click();
  await expect(toggle).not.toBeChecked();
  await expect(heading).toHaveAttribute("aria-hidden", "true");
  await settings.reload();
  await expect(toggle).not.toBeChecked();
  await settings.setViewportSize({ width: 560, height: 420 });
  expect(
    await settings.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await settings.screenshot({
    path: testInfo.outputPath("title-setting-small.png"),
  });
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(1);
});
