import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  layoutPanes,
  newAgentTaskTab,
  newAndroidTab,
  newBrowserTab,
  newChatTab,
  newPane,
  newProject,
  newSession,
  newTab,
  splitPane,
  type LayoutPane,
  type Tab,
} from "../../src/model";
import { buffer, mockDesktop } from "./desktop";
import { mockAndroid } from "./android-mock";
import { mockChats } from "./chat-mock";

test.use({ colorScheme: "dark" });

const kinds = [
  "terminal",
  "file",
  "browser",
  "android",
  "chat",
  "agent-task",
  "plugin",
] as const;

function panel(kind: (typeof kinds)[number]): LayoutPane {
  switch (kind) {
    case "terminal":
      return newPane("/project/second");
    case "file":
      return {
        type: "file",
        id: crypto.randomUUID(),
        title: "README.md",
        root: "/project",
        relative: "README.md",
      };
    case "browser":
      return newBrowserTab("https://example.com/");
    case "android":
      return newAndroidTab("12345678-1234-4567-8123-123456789abc");
    case "chat":
      return newChatTab("pane-tabs-chat");
    case "agent-task":
      return newAgentTaskTab("pane-tabs-task");
    case "plugin":
      return {
        type: "plugin",
        id: crypto.randomUUID(),
        title: "Saved plugin",
        owner: "local.fixture",
        viewType: "local.fixture.view",
        stateVersion: 1,
        state: { draft: "retained" },
      };
  }
}

async function setup(
  page: Page,
  kind: (typeof kinds)[number] = "terminal",
  destination?: Tab | Tab[],
  single = false,
) {
  const project = newProject("/project", "local:bash");
  const workspace = project.workspaces[0];
  const source = workspace.tabs[0];
  if (source.type !== "terminal") throw new Error("Expected terminal tab");
  source.title = "Source";
  const moved = single ? layoutPanes(source.layout)[0] : panel(kind);
  if (!single)
    source.layout = splitPane(
      source.layout,
      source.activePaneId,
      "horizontal",
      moved,
    );
  if (destination)
    workspace.tabs.push(
      ...(Array.isArray(destination) ? destination : [destination]),
    );
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  if (kind === "android") await mockAndroid(page, true);
  if (kind === "chat") await mockChats(page);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toHaveCount(
    !single && kind === "terminal" ? 2 : 1,
  );
  await expect
    .poll(() => buffer(page, source.activePaneId))
    .toContain("bash $ ");
  if (moved.type === "terminal")
    await expect.poll(() => buffer(page, moved.id)).toContain("bash $ ");
  return { source, moved, workspace };
}

async function grab(page: Page, pane: LayoutPane) {
  await page.keyboard.down("Control");
  const selector = {
    terminal: `[data-pane-id="${pane.id}"] .terminal-title-box`,
    file: `[data-file-pane-id="${pane.id}"] [data-pane-drag-handle]`,
    browser: `[data-browser-pane-id="${pane.id}"] [data-pane-drag-handle]`,
    android: `[data-android-pane-id="${pane.id}"] [data-pane-drag-handle]`,
    chat: `[data-chat-pane-id="${pane.id}"] [data-pane-drag-handle]`,
    "agent-task": `[data-task-pane-id="${pane.id}"] [data-pane-drag-handle]`,
    plugin: `[data-plugin-pane-id="${pane.id}"] [data-pane-drag-handle]`,
  }[pane.type];
  const handle = page.locator(selector);
  await expect(handle).toBeVisible();
  const point = await handle.evaluate((element) => {
    const bounds = element.getBoundingClientRect();
    for (let y = bounds.top + 2; y < bounds.bottom - 2; y += 3)
      for (let x = bounds.left + 2; x < bounds.right - 2; x += 3) {
        const target = document.elementFromPoint(x, y);
        if (
          target &&
          element.contains(target) &&
          !target.closest("button, input, textarea, select")
        )
          return { x, y };
      }
    return null;
  });
  expect(point, `${pane.type} has a draggable heading`).not.toBeNull();
  await page.mouse.move(point!.x, point!.y);
  await page.mouse.down();
}

async function moveTo(page: Page, target: Locator, edge = false) {
  const bounds = (await target.boundingBox())!;
  await page.mouse.move(
    edge ? bounds.x + 1 : bounds.x + bounds.width / 2,
    bounds.y + bounds.height / 2,
    { steps: 8 },
  );
}

async function release(page: Page) {
  await page.mouse.up();
  await page.keyboard.up("Control");
  await expect(page.locator(".pane-drag-ghost")).toHaveCount(0);
}

async function savedWorkspace(page: Page) {
  return page.evaluate(
    () =>
      JSON.parse(localStorage.getItem("test-session") ?? "null")?.projects[0]
        .workspaces[0],
  );
}

async function terminalCalls(page: Page) {
  return page.evaluate(() =>
    (window as any).__nativeTest.calls.filter((call: any) =>
      ["start_terminal", "close_terminal"].includes(call.command),
    ),
  );
}

for (const kind of kinds) {
  test(`extracting a ${kind} panel creates a tab and retains its identity`, async ({
    page,
  }, testInfo) => {
    const { source, moved } = await setup(page, kind);
    const calls = await terminalCalls(page);
    if (kind === "terminal")
      await page.evaluate(async (id) => {
        const { runningTerminal } = await import("/src/terminal-runtime.ts");
        (window as any).__nativeTest.emit(
          runningTerminal(id)!.sessionId,
          "retained output\r\n",
        );
      }, moved.id);
    let original = "";
    if (kind === "file") {
      const editor = page.locator(".cm-content");
      await expect(editor).toBeVisible();
      original = await editor.innerText();
      await editor.fill("Unsaved tab transfer 🦀");
    }
    await grab(page, moved);
    await moveTo(page, page.getByRole("button", { name: /^New tab/ }));
    await expect(page.locator(".tab-drop-marker")).toBeVisible();
    if (kind === "terminal")
      await page.screenshot({
        path: testInfo.outputPath("extract-panel-preview.png"),
      });
    await release(page);
    await expect(page.getByRole("tab")).toHaveCount(2);
    await expect
      .poll(async () => (await savedWorkspace(page))?.tabs.length)
      .toBe(2);
    const saved = await savedWorkspace(page);
    const extracted = saved.tabs.find((tab: Tab) => tab.id !== source.id);
    expect(extracted.type).toBe(kind);
    expect(kind === "terminal" ? extracted.layout.id : extracted.id).toBe(
      moved.id,
    );
    expect(saved.activeTabId).toBe(extracted.id);
    expect(await terminalCalls(page)).toEqual(calls);
    if (kind === "terminal")
      await expect
        .poll(() => buffer(page, moved.id))
        .toContain("retained output");
    if (kind === "file") {
      const editor = page.locator(".cm-content");
      await expect(editor).toHaveText("Unsaved tab transfer 🦀");
      await editor.click();
      await page.keyboard.press("Control+z");
      await expect.poll(() => editor.innerText()).toBe(original);
    }
    if (kind === "browser")
      expect(
        await page.evaluate(() => (window as any).__nativeTest.browsers.size),
      ).toBe(1);
    if (kind === "android")
      expect(
        await page.evaluate(() => (window as any).__androidTest.stops),
      ).toBe(0);
  });
}

for (const targetKind of ["terminal", "file", "browser"] as const) {
  test(`moving a panel into a ${targetKind} tab keeps both panels`, async ({
    page,
  }, testInfo) => {
    const target =
      targetKind === "terminal"
        ? newTab("/project/target", "local:bash", "Destination")
        : ({ ...panel(targetKind), title: "Destination" } as Tab);
    const { source, moved } = await setup(page, "terminal", target);
    await page.locator(`#tab-${target.id}`).click();
    if (target.type === "terminal")
      await expect
        .poll(() => buffer(page, target.activePaneId))
        .toContain("bash $ ");
    await page.locator(`#tab-${source.id}`).click();
    await expect(page.locator(".xterm-screen")).toHaveCount(2);
    const calls = await terminalCalls(page);
    await grab(page, moved);
    await moveTo(page, page.locator(`[data-tab-id="${target.id}"]`));
    await expect(page.locator(".pane-tab-drop-preview")).toBeVisible();
    await expect(page.locator(`#tab-${source.id}`)).toHaveAttribute(
      "aria-selected",
      "true",
    );
    if (targetKind === "terminal") {
      await page
        .locator(".pane-tab-drop-preview")
        .evaluate((element) =>
          Promise.all(
            element.getAnimations().map((animation) => animation.finished),
          ),
        );
      await page.screenshot({
        path: testInfo.outputPath("move-to-tab-preview.png"),
      });
    }
    await release(page);
    await expect(page.getByRole("tab")).toHaveCount(2);
    await expect
      .poll(async () => {
        const saved = await savedWorkspace(page);
        return !!saved?.activeTabId && saved.activeTabId !== source.id;
      })
      .toBe(true);
    const saved = await savedWorkspace(page);
    const destination = saved.tabs.find(
      (tab: Tab) => tab.id === saved.activeTabId,
    );
    expect(destination.type).toBe("terminal");
    const ids = layoutPanes(destination.layout).map((pane) => pane.id);
    expect(ids).toContain(moved.id);
    expect(ids).toContain(
      target.type === "terminal" ? target.activePaneId : target.id,
    );
    expect(await terminalCalls(page)).toEqual(calls);
  });
}

test("extracting at a tab edge inserts the new tab before that tab", async ({
  page,
}) => {
  const target = newTab("/project/target", "local:bash", "Destination");
  const { source, moved } = await setup(page, "terminal", target);
  await grab(page, moved);
  await moveTo(page, page.locator(`[data-tab-id="${target.id}"]`), true);
  await expect(page.locator(".tab-drop-marker")).toBeVisible();
  await release(page);
  await expect(page.getByRole("tab")).toHaveCount(3);
  await expect
    .poll(async () => (await savedWorkspace(page))?.tabs.length)
    .toBe(3);
  const saved = await savedWorkspace(page);
  expect(saved.tabs.map((tab: Tab) => tab.id)).toEqual([
    source.id,
    saved.activeTabId,
    target.id,
  ]);
});

test("moving the only panel into another tab removes its empty source tab", async ({
  page,
}) => {
  const target = newTab("/project/target", "local:bash", "Destination");
  const { moved } = await setup(page, "terminal", target, true);
  await grab(page, moved);
  await moveTo(page, page.locator(`[data-tab-id="${target.id}"]`));
  await release(page);
  await expect(page.getByRole("tab")).toHaveCount(1);
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  await expect
    .poll(async () => (await savedWorkspace(page))?.tabs.length)
    .toBe(1);
  expect(
    layoutPanes((await savedWorkspace(page)).tabs[0].layout).map(
      (pane) => pane.id,
    ),
  ).toContain(moved.id);
  expect(
    (await terminalCalls(page)).filter(
      (call: any) => call.command === "close_terminal",
    ),
  ).toEqual([]);
});

test("the tab strip scrolls while holding a panel near its edge", async ({
  page,
}) => {
  const targets = Array.from({ length: 25 }, (_, index) =>
    newTab("/project", "local:bash", `Destination ${index + 1}`),
  );
  const { moved } = await setup(page, "terminal", targets);
  const strip = page.locator(".tab-strip");
  await grab(page, moved);
  const bounds = (await strip.boundingBox())!;
  await page.mouse.move(
    bounds.x + bounds.width - 4,
    bounds.y + bounds.height / 2,
    { steps: 8 },
  );
  await expect
    .poll(() => strip.evaluate((element) => element.scrollLeft))
    .toBeGreaterThan(200);
  await page.keyboard.press("Escape");
  await release(page);
  await expect(page.getByRole("tab")).toHaveCount(26);
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
});

test("self and oversized tab targets reject transfers without changing panels", async ({
  page,
}) => {
  const target = newTab("/project", "local:bash", "Crowded");
  for (let index = 0; index < 8; index++)
    target.layout = splitPane(
      target.layout,
      target.activePaneId,
      "horizontal",
      newPane("/project"),
    );
  const { source, moved } = await setup(page, "terminal", target);
  const calls = await terminalCalls(page);
  await grab(page, moved);
  await moveTo(page, page.locator(`[data-tab-id="${source.id}"]`));
  await expect(page.locator(".pane-tab-drop-preview")).toHaveClass(
    /is-blocked/,
  );
  await moveTo(page, page.locator(`[data-tab-id="${target.id}"]`));
  await expect(page.locator(".pane-tab-drop-preview")).toHaveClass(
    /is-blocked/,
  );
  await release(page);
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(page.locator(`#tab-${source.id}`)).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  expect(await terminalCalls(page)).toEqual(calls);
});

test("a pane uses the final release position when dropping on the strip", async ({
  page,
}) => {
  const target = newTab("/project", "local:bash", "Destination");
  const { moved } = await setup(page, "terminal", target);
  await grab(page, moved);
  await moveTo(page, page.getByRole("button", { name: /^New tab/ }));
  await expect(page.locator(".tab-drop-marker")).toBeVisible();
  const bounds = (await page
    .locator(`[data-tab-id="${target.id}"]`)
    .boundingBox())!;
  await page.evaluate(
    ({ x, y }) => {
      document.dispatchEvent(
        new PointerEvent("pointerup", {
          pointerId: 1,
          clientX: x,
          clientY: y,
          ctrlKey: true,
          bubbles: true,
        }),
      );
    },
    { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 },
  );
  await release(page);
  await expect
    .poll(async () => (await savedWorkspace(page))?.activeTabId)
    .toBe(target.id);
  expect(
    layoutPanes((await savedWorkspace(page)).tabs[1].layout).map(
      (pane) => pane.id,
    ),
  ).toContain(moved.id);
  await expect(page.getByRole("tab")).toHaveCount(2);
});

test("Settings remains a utility tab when a panel is dropped on it", async ({
  page,
}) => {
  const { source, moved } = await setup(page);
  await page.getByRole("button", { name: "Open account menu" }).click();
  await page.getByRole("menuitem", { name: /^Settings/ }).click();
  await expect(
    page.getByRole("tab", { name: "Settings", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await page.locator(`#tab-${source.id}`).click();
  const calls = await terminalCalls(page);
  await grab(page, moved);
  await moveTo(page, page.getByRole("tab", { name: "Settings", exact: true }));
  await expect(page.locator(".pane-tab-drop-preview")).toHaveClass(
    /is-blocked/,
  );
  await expect(page.locator(".pane-drag-ghost")).toHaveText(
    "Cannot move panel here",
  );
  await release(page);
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  expect(await terminalCalls(page)).toEqual(calls);
});

for (const cancel of ["Escape", "Control", "blur"] as const) {
  test(`${cancel} cancels a panel transfer to the tab strip`, async ({
    page,
  }) => {
    const { moved } = await setup(page);
    const calls = await terminalCalls(page);
    await grab(page, moved);
    await moveTo(page, page.getByRole("button", { name: /^New tab/ }));
    await expect(page.locator(".tab-drop-marker")).toBeVisible();
    if (cancel === "Escape") await page.keyboard.press("Escape");
    else if (cancel === "Control") await page.keyboard.up("Control");
    else await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await release(page);
    await expect(page.getByRole("tab")).toHaveCount(1);
    await expect(page.locator(".xterm-screen")).toHaveCount(2);
    expect(await terminalCalls(page)).toEqual(calls);
  });
}
