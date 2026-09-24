import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { newPane, newProject, newSession, splitPane } from "../../src/model";
import { mockDesktop } from "./desktop";

async function openSplit(page: Page, ratio = 0.5) {
  await page.addInitScript(() => {
    const saved = JSON.parse(
      localStorage.getItem("test-keybindings") ?? '{"version":1,"bindings":{}}',
    );
    localStorage.setItem(
      "test-keybindings",
      JSON.stringify({ ...saved, focusFollowsPointer: true }),
    );
  });
  const project = newProject("/project", "local:bash");
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  const firstId = tab.activePaneId;
  const second = newPane("/project/second");
  tab.layout = splitPane(tab.layout, firstId, "horizontal", second);
  if (tab.layout.type === "split") tab.layout.ratio = ratio;
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  const first = page.locator(`[data-pane-id="${firstId}"]`);
  const hovered = page.locator(`[data-pane-id="${second.id}"]`);
  await first.click();
  await hovered.hover();
  await expect(hovered.locator(".xterm-helper-textarea")).toBeFocused();
  return { first, hovered };
}

async function calls(page: Page, command: string) {
  return page.evaluate(
    (command) =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === command,
      ),
    command,
  );
}

for (const shortcut of ["Control+d", "Control+Shift+d"]) {
  test(`${shortcut} splits the hovered terminal in pointer focus mode`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({
      colorScheme: shortcut === "Control+d" ? "dark" : "light",
    });
    const { first, hovered } = await openSplit(page);
    const originalBounds = await first.boundingBox();
    const hoveredBounds = (await hovered.boundingBox())!;
    await page.keyboard.type("pwd");
    await expect
      .poll(async () =>
        (await calls(page, "write_terminal"))
          .map((call: any) => call.args.data)
          .join(""),
      )
      .toBe("pwd");
    await expect(hovered.locator(".xterm-helper-textarea")).toBeFocused();

    await page.keyboard.press(shortcut);
    await expect(page.locator("[data-pane-id]")).toHaveCount(3);
    for (const [key, value] of Object.entries((await first.boundingBox())!))
      expect(
        Math.abs(value - originalBounds![key as keyof typeof originalBounds]),
      ).toBeLessThanOrEqual(1);
    const added = page.locator("[data-pane-id]").last();
    const addedBounds = (await added.boundingBox())!;
    if (shortcut === "Control+d") {
      expect(addedBounds.x).toBeGreaterThan(hoveredBounds.x);
      expect(addedBounds.height).toBeCloseTo(hoveredBounds.height, 3);
    } else {
      expect(addedBounds.y).toBeGreaterThan(hoveredBounds.y);
      expect(addedBounds.width).toBeCloseTo(hoveredBounds.width, 3);
    }
    await expect
      .poll(async () =>
        (await calls(page, "start_terminal")).map(
          (call: any) => call.args.request.cwd,
        ),
      )
      .toEqual(["/project", "/project/second", "/project/second"]);
    expect(
      (await calls(page, "write_terminal"))
        .map((call: any) => call.args.data)
        .join(""),
    ).toBe("pwd");
    await page.screenshot({ path: testInfo.outputPath("hovered-split.png") });
  });
}

test("split shortcuts fall back to the active terminal after the pointer leaves the panes", async ({
  page,
}) => {
  const { first, hovered } = await openSplit(page);
  await first.click();
  const originalBounds = (await first.boundingBox())!;
  const hoveredBounds = await hovered.boundingBox();
  await page.locator(".sidebar-heading").hover();
  await page.keyboard.press("Control+d");
  await expect(page.locator("[data-pane-id]")).toHaveCount(3);
  for (const [key, value] of Object.entries((await hovered.boundingBox())!))
    expect(
      Math.abs(value - hoveredBounds![key as keyof typeof hoveredBounds]),
    ).toBeLessThanOrEqual(1);
  expect((await first.boundingBox())!.width).toBeLessThan(originalBounds.width);
});

test("a hovered terminal without room does not split the larger active terminal", async ({
  page,
}) => {
  const { first, hovered } = await openSplit(page, 0.75);
  const originalBounds = await first.boundingBox();
  const hoveredBounds = await hovered.boundingBox();
  await page.keyboard.press("Control+d");
  await expect(page.locator(".pane-limit-notice")).toContainText("No room");
  await expect(page.locator("[data-pane-id]")).toHaveCount(2);
  for (const [key, value] of Object.entries((await first.boundingBox())!))
    expect(
      Math.abs(value - originalBounds![key as keyof typeof originalBounds]),
    ).toBeLessThanOrEqual(1);
  for (const [key, value] of Object.entries((await hovered.boundingBox())!))
    expect(
      Math.abs(value - hoveredBounds![key as keyof typeof hoveredBounds]),
    ).toBeLessThanOrEqual(1);
  await expect(hovered.locator(".xterm-helper-textarea")).toBeFocused();
  expect(await calls(page, "write_terminal")).toHaveLength(0);
});

for (const key of ["w", "q"]) {
  test(`Ctrl+${key} closes the hovered terminal and preserves the surviving shell`, async ({
    page,
  }, testInfo) => {
    if (key === "q")
      await page.addInitScript(() => {
        localStorage.setItem(
          "test-keybindings",
          JSON.stringify({
            version: 1,
            bindings: { closeTerminal: "Ctrl+KeyQ" },
          }),
        );
      });
    const { first, hovered } = await openSplit(page);
    await expect
      .poll(async () => (await calls(page, "start_terminal")).length)
      .toBe(2);
    const started = await calls(page, "start_terminal");
    const hoveredSession = started.find(
      (call: any) => call.args.request.cwd === "/project/second",
    ).args.request.id;
    await page.keyboard.down("Control");
    await page.keyboard.down(key);
    await expect(hovered).toHaveCount(0);
    await expect(first.locator(".xterm-helper-textarea")).toBeFocused();
    await page.keyboard.down(key);
    await page.keyboard.up(key);
    await page.keyboard.up("Control");
    await expect(page.locator("[data-pane-id]")).toHaveCount(1);
    await expect
      .poll(async () =>
        (await calls(page, "close_terminal")).map((call: any) => call.args.id),
      )
      .toEqual([hoveredSession]);
    expect(await calls(page, "start_terminal")).toEqual(started);
    expect(await calls(page, "write_terminal")).toHaveLength(0);
    await page.keyboard.type("pwd");
    await expect
      .poll(async () =>
        (await calls(page, "write_terminal"))
          .map((call: any) => call.args.data)
          .join(""),
      )
      .toBe("pwd");
    await page.screenshot({ path: testInfo.outputPath("hovered-close.png") });
  });
}

for (const outside of [".sidebar-heading", ".split-divider"]) {
  test(`Ctrl+W closes the active terminal when the pointer is over ${outside}`, async ({
    page,
  }) => {
    const { first, hovered } = await openSplit(page);
    await page.locator(outside).hover();
    await first.click();
    await page.locator(outside).hover();
    await page.keyboard.press("Control+w");
    await expect(first).toHaveCount(0);
    await expect(hovered.locator(".xterm-helper-textarea")).toBeFocused();
    await expect(page.locator("[data-pane-id]")).toHaveCount(1);
    expect(await calls(page, "write_terminal")).toHaveLength(0);
  });
}
