import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

const toggle = (page: Page, panel: "files" | "git") =>
  page.getByRole("button", {
    name:
      panel === "files" ? /^Toggle file explorer/ : /^Toggle source control/,
  });
const sidebar = (page: Page, panel: "files" | "git") =>
  page.getByRole("complementary", {
    name: panel === "files" ? "Explorer" : "Source Control",
    exact: true,
  });
const saved = (page: Page) =>
  page.evaluate(() =>
    JSON.parse(localStorage.getItem("test-session") ?? "null"),
  );
async function move(
  page: Page,
  panel: "files" | "git",
  side: "left" | "right",
) {
  await toggle(page, panel).click({ button: "right" });
  const menu = page.getByRole("menu", {
    name:
      panel === "files"
        ? "Explorer panel position"
        : "Source Control panel position",
  });
  await expect(menu).toBeInViewport();
  await menu
    .getByRole("menuitemradio", { name: `Panel on the ${side}` })
    .click();
  await expect(menu).toHaveCount(0);
  await expect(sidebar(page, panel)).toHaveAttribute("data-side", side);
}
async function waitForLayoutMotion(page: Page) {
  await page.evaluate(async () => {
    const animations = document
      .getAnimations()
      .filter((animation) => animation.id === "lomi-layout-motion");
    await Promise.all(
      animations.map((animation) => animation.finished.catch(() => undefined)),
    );
  });
}
async function setup(page: Page, repository = true) {
  await mockDesktop(page, repository);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
}

test("opposite panels stay open together, relocate their controls and preserve the running terminal", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await setup(page);
  const host = page.locator("[data-pane-id]");
  const id = (await host.getAttribute("data-pane-id"))!;
  await host.evaluate((element) => {
    (window as any).__sidebarTerminalHost = element;
  });
  await move(page, "git", "right");
  await waitForLayoutMotion(page);
  await expect(sidebar(page, "files")).toBeVisible();
  await expect(sidebar(page, "git")).toBeVisible();
  const explorer = (await sidebar(page, "files").boundingBox())!;
  const git = (await sidebar(page, "git").boundingBox())!;
  const terminal = (await host.boundingBox())!;
  expect(explorer.x + explorer.width).toBeLessThanOrEqual(terminal.x + 1);
  expect(git.x).toBeGreaterThanOrEqual(terminal.x + terminal.width - 1);
  expect((await toggle(page, "files").boundingBox())!.x).toBeLessThan(50);
  expect((await toggle(page, "git").boundingBox())!.x).toBeGreaterThan(1200);
  expect(
    await host.evaluate(
      (element) => element === (window as any).__sidebarTerminalHost,
    ),
  ).toBe(true);
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.emit([...native.sessions.keys()][0], "\r\nBOTH SIDEBARS OPEN\r\n");
  });
  await expect.poll(() => buffer(page, id)).toContain("BOTH SIDEBARS OPEN");
  await page.keyboard.press("Control+Shift+e");
  await expect(sidebar(page, "files")).toHaveCount(0);
  await expect(sidebar(page, "git")).toBeVisible();
  await page.keyboard.press("Control+Shift+e");
  await page.keyboard.press("Control+Shift+g");
  await expect(sidebar(page, "files")).toBeVisible();
  await expect(sidebar(page, "git")).toHaveCount(0);
  await page.getByTitle("Show source control", { exact: true }).click();
  await expect(sidebar(page, "git")).toBeVisible();
  await expect.poll(async () => (await saved(page))?.rightSidebar).toBe("git");
  await page.reload();
  await expect(sidebar(page, "files")).toHaveAttribute("data-side", "left");
  await expect(sidebar(page, "git")).toHaveAttribute("data-side", "right");
  await page.screenshot({ path: "test-results/opposite-sidebars.png" });
});

test("moving a visible source control panel preserves its draft and same-side panels switch", async ({
  page,
}) => {
  await setup(page);
  await toggle(page, "git").click();
  const message = page.getByRole("textbox", {
    name: "Commit message",
    exact: true,
  });
  await message.fill("preserve this commit draft\nwith its exact text");
  await move(page, "git", "right");
  await expect(message).toHaveValue(
    "preserve this commit draft\nwith its exact text",
  );
  await toggle(page, "files").click();
  await move(page, "files", "right");
  await expect(sidebar(page, "git")).toHaveCount(0);
  await expect(sidebar(page, "files")).toBeVisible();
  await toggle(page, "git").click();
  await expect(sidebar(page, "files")).toHaveCount(0);
  await move(page, "files", "left");
  await expect(sidebar(page, "files")).toBeVisible();
  await expect(sidebar(page, "git")).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter((call: any) =>
        ["git_commit", "git_stage", "close_terminal"].includes(call.command),
      ),
    ),
  ).toEqual([]);
});

test("both sidebars resize in the correct direction, restore widths and fit the minimum window", async ({
  page,
}) => {
  await setup(page);
  await move(page, "git", "right");
  const left = page.getByRole("separator", {
    name: "Resize sidebar",
    exact: true,
  });
  const right = page.getByRole("separator", {
    name: "Resize right sidebar",
    exact: true,
  });
  await left.press("ArrowRight");
  await right.press("ArrowLeft");
  await expect(left).toHaveAttribute("aria-valuenow", "270");
  await expect(right).toHaveAttribute("aria-valuenow", "270");
  const bounds = (await right.boundingBox())!;
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + 50);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 - 30, bounds.y + 50, {
    steps: 5,
  });
  await page.mouse.up();
  await expect(right).toHaveAttribute("aria-valuenow", "300");
  await expect
    .poll(async () => (await saved(page))?.rightSidebarWidth)
    .toBe(300);
  await page.reload();
  await expect(left).toHaveAttribute("aria-valuenow", "270");
  await expect(right).toHaveAttribute("aria-valuenow", "300");
  await page.setViewportSize({ width: 800, height: 420 });
  await expect(sidebar(page, "files")).toBeVisible();
  await expect(sidebar(page, "git")).toBeVisible();
  const terminal = page.locator(".terminal-stage");
  expect((await terminal.boundingBox())!.width).toBeGreaterThanOrEqual(240);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await expect(
    page.getByRole("textbox", { name: "Commit message", exact: true }),
  ).toBeInViewport();
  await page.screenshot({ path: "test-results/opposite-sidebars-minimum.png" });
});

test("sidebar context menus support keyboard selection, Escape and outside dismissal", async ({
  page,
}) => {
  await setup(page);
  const button = toggle(page, "git");
  await button.focus();
  await button.press("Shift+F10");
  const menu = page.getByRole("menu", {
    name: "Source Control panel position",
  });
  await expect(
    menu.getByRole("menuitemradio", { name: "Panel on the left" }),
  ).toBeFocused();
  await expect(
    menu.getByRole("menuitemradio", { name: "Panel on the left" }),
  ).toHaveAttribute("aria-checked", "true");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(sidebar(page, "git")).toHaveAttribute("data-side", "right");
  await button.press("Shift+F10");
  await expect(
    menu.getByRole("menuitemradio", { name: "Panel on the right" }),
  ).toBeFocused();
  await page.keyboard.press("Control+w");
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(button).toBeFocused();
  await button.click({ button: "right" });
  await page.locator(".terminal-stage").click();
  await expect(menu).toHaveCount(0);
});

test("the explorer can move to the right without a Git repository", async ({
  page,
}) => {
  await setup(page, false);
  await expect(toggle(page, "git")).toHaveCount(0);
  await move(page, "files", "right");
  expect((await toggle(page, "files").boundingBox())!.x).toBeGreaterThan(1300);
  await toggle(page, "files").click();
  await expect(sidebar(page, "files")).toHaveCount(0);
  await expect.poll(async () => (await saved(page))?.rightSidebar).toBe(null);
  await expect
    .poll(async () => (await saved(page))?.sidebarSides.files)
    .toBe("right");
  await page.reload();
  await expect(toggle(page, "files")).toHaveAttribute("aria-pressed", "false");
  await toggle(page, "files").click();
  await expect(sidebar(page, "files")).toHaveAttribute("data-side", "right");
});

test("terminal overview moves without toggling or restarting and restores its position", async ({
  page,
}, testInfo) => {
  await setup(page);
  const button = page.getByRole("button", {
    name: /^Toggle terminal overview/,
  });
  const menu = page.getByRole("menu", {
    name: "Terminal overview panel position",
  });
  const left = menu.getByRole("menuitemradio", { name: "Panel on the left" });
  const right = menu.getByRole("menuitemradio", { name: "Panel on the right" });
  const screen = await page.locator(".xterm-screen").elementHandle();
  await expect.poll(() => saved(page)).not.toBeNull();
  const before = await saved(page);
  expect((await button.boundingBox())!.x).toBeLessThan(400);
  await button.click({ button: "right" });
  await expect(menu).toBeInViewport();
  await expect(left).toBeChecked();
  await expect(button).toHaveAttribute("aria-pressed", "false");
  await right.click();
  await expect(menu).toHaveCount(0);
  expect((await button.boundingBox())!.x).toBeGreaterThan(1300);
  await expect(button).toHaveAttribute("aria-pressed", "false");
  await button.click();
  await expect(page.locator(".terminal-overview")).toBeVisible();
  await button.press("Shift+F10");
  await expect(right).toBeFocused();
  await expect(right).toBeChecked();
  await page.screenshot({ path: testInfo.outputPath("overview-position.png") });
  await page.keyboard.press("Control+Tab");
  await expect(button).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("Escape");
  await expect(button).toBeFocused();
  await button.click();
  await expect(page.locator(".xterm-screen")).toBeVisible();
  expect(
    await screen!.evaluate(
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
  await expect
    .poll(async () => (await saved(page))?.terminalOverviewSide)
    .toBe("right");
  const after = await saved(page);
  expect(after.sidebarSides).toEqual(before.sidebarSides);
  expect(after.sidebar).toBe(before.sidebar);
  expect(after.rightSidebar).toBe(before.rightSidebar);
  await page.reload();
  await expect(button).toBeVisible();
  expect((await button.boundingBox())!.x).toBeGreaterThan(1300);
  await button.press("Shift+F10");
  await expect(right).toBeChecked();
  await page.keyboard.press("Home");
  await page.keyboard.press("Enter");
  expect((await button.boundingBox())!.x).toBeLessThan(400);
  await expect
    .poll(async () => (await saved(page))?.terminalOverviewSide)
    .toBe("left");
});
