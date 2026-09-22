import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";
import { newPane, newProject, newSession, splitPane } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";

type Platform = "macos" | "linux" | "windows";

async function setup(page: Page, platform: Platform, scale = 2, zoom = 1) {
  const project = newProject("/project", "local:bash");
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  const first = tab.activePaneId;
  const second = newPane("/project/second");
  const third = newPane("/project/third");
  tab.layout = splitPane(
    splitPane(tab.layout, first, "horizontal", second),
    second.id,
    "vertical",
    third,
  );
  await mockDesktop(
    page,
    false,
    { ...newSession(), projects: [project], activeProjectId: project.id },
    undefined,
    {},
    platform,
  );
  await page.addInitScript(
    ({ platform, scale, zoom }) => {
      localStorage.setItem("lomi.zoom.main", String(zoom * 100));
      Object.defineProperty(window, "devicePixelRatio", {
        configurable: true,
        value: platform === "windows" ? scale * zoom : scale,
      });
    },
    { platform, scale, zoom },
  );
  await page.goto("/");
  const ids = [first, second.id, third.id];
  await expect(page.locator(".xterm-screen")).toHaveCount(ids.length);
  for (const id of ids)
    await expect.poll(() => buffer(page, id)).toContain("bash $ ");
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.zoom))
    .toBe(zoom);
  return ids;
}

async function drag(
  page: Page,
  type: "enter" | "over" | "drop" | "leave",
  target: Locator,
  factor = 1,
  paths = ["/external/it's a file żółć.txt", "/external/second.png"],
) {
  const bounds = (await target.boundingBox())!;
  await page.evaluate(
    ({ type, position, paths }) =>
      (window as any).__nativeTest.emitEvent(`tauri://drag-${type}`, {
        position,
        paths,
      }),
    {
      type,
      position: {
        x: (bounds.x + bounds.width / 2) * factor,
        y: (bounds.y + bounds.height / 2) * factor,
      },
      paths,
    },
  );
}

const writes = (page: Page) =>
  page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter((call: any) => call.command === "write_terminal")
      .map((call: any) => call.args),
  );

const terminalSession = (page: Page, id: string) =>
  page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    return runningTerminal(id)!.sessionId;
  }, id);

for (const [platform, scale, zoom] of [
  ["macos", 1, 1],
  ["macos", 2, 1],
  ["macos", 2, 0.8],
  ["macos", 2, 1.4],
  ["linux", 2, 1],
  ["linux", 2, 1.4],
  ["windows", 1, 1],
  ["windows", 2, 1.4],
] as const) {
  test(`${platform} file drops follow all three panes at scale ${scale} and zoom ${zoom}`, async ({
    page,
  }, testInfo) => {
    const ids = await setup(page, platform, scale, zoom);
    const factor = platform === "windows" ? scale * zoom : zoom;
    const pane = (id: string) => page.locator(`[data-pane-id="${id}"]`);
    for (const [index, id] of ids.entries()) {
      const other = pane(ids[(index + 1) % ids.length]);
      await other.locator(".xterm-helper-textarea").focus();
      await drag(page, "enter", other, factor);
      await expect(other).toHaveClass(/drop-target/);
      await drag(page, "over", pane(id), factor);
      await expect(pane(id)).toHaveClass(/drop-target/);
      await expect(page.locator(".drop-target")).toHaveCount(1);
      await expect(other.locator(".xterm-helper-textarea")).toBeFocused();
      if (platform === "macos" && scale === 2 && zoom === 1 && index === 2)
        await page.screenshot({ path: testInfo.outputPath("drop-target.png") });
      await drag(page, "drop", pane(id), factor);
      await expect
        .poll(async () => (await writes(page)).length)
        .toBe(index + 1);
      const sent = (await writes(page))[index];
      expect(sent.id).toBe(await terminalSession(page, id));
      expect(sent.data).toBe(
        "'/external/it'\\''s a file żółć.txt' '/external/second.png' ",
      );
      await expect(pane(id).locator(".xterm-helper-textarea")).toBeFocused();
      await expect(page.locator(".drop-target")).toHaveCount(0);
    }
    const calls = await page.evaluate(() => (window as any).__nativeTest.calls);
    expect(
      calls.filter((call: any) => call.command === "start_terminal"),
    ).toHaveLength(3);
    expect(
      calls.filter((call: any) => call.command === "close_terminal"),
    ).toHaveLength(0);
  });
}

test("leaving or dropping outside terminals clears the target without pasting", async ({
  page,
}) => {
  const ids = await setup(page, "macos");
  const terminal = page.locator(`[data-pane-id="${ids[2]}"]`);
  await drag(page, "enter", terminal);
  await expect(terminal).toHaveClass(/drop-target/);
  await drag(page, "leave", terminal);
  await expect(page.locator(".drop-target")).toHaveCount(0);
  await drag(page, "enter", terminal);
  await drag(page, "drop", page.locator(".file-tree"));
  await expect(page.locator(".drop-target")).toHaveCount(0);
  expect(await writes(page)).toEqual([]);
  await drag(page, "enter", terminal);
  const final = page.locator(`[data-pane-id="${ids[1]}"]`);
  await drag(page, "drop", final);
  await expect
    .poll(async () => (await writes(page)).map((write: any) => write.id))
    .toEqual([await terminalSession(page, ids[1])]);
});

test("Explorer pointer dragging reaches the third terminal on Retina", async ({
  page,
}) => {
  const ids = await setup(page, "macos");
  const file = page.getByRole("button", {
    name: "it's a file.txt",
    exact: true,
  });
  const terminal = page.locator(`[data-pane-id="${ids[2]}"]`);
  const bounds = (await terminal.boundingBox())!;
  await file.hover();
  await page.mouse.down();
  await page.mouse.move(
    bounds.x + bounds.width / 2,
    bounds.y + bounds.height / 2,
    { steps: 8 },
  );
  await expect(terminal).toHaveClass(/drop-target/);
  await page.mouse.up();
  await expect
    .poll(async () => (await writes(page)).map((write: any) => write.id))
    .toEqual([await terminalSession(page, ids[2])]);
  await expect(terminal.locator(".xterm-helper-textarea")).toBeFocused();
});
