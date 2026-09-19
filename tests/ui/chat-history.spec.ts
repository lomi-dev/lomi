import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockChats } from "./chat-mock";

async function setup(page: Page, count = 8) {
  await mockDesktop(page, false);
  await mockChats(page);
  await page.goto("/");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Chat AI", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toBeVisible();
  await page.evaluate((count) => {
    const state = (window as any).__chatTest;
    const template = Object.values(state.conversations)[0] as any;
    for (let i = 0; i < count; i++) {
      const copy = structuredClone(template);
      copy.conversation.id = `history-${i}`;
      copy.conversation.title =
        [
          "Plan aplikacji",
          "Pomysły na interfejs",
          "Analiza bardzo długiego tytułu rozmowy, który powinien mieścić się w panelu",
          "Inny projekt",
        ][i] ?? `Rozmowa ${i}`;
      copy.conversation.pinned = i === 0;
      copy.conversation.updatedAt = i;
      if (i === 3) {
        copy.conversation.origin.projectId = "other-project";
        copy.conversation.origin.workspaceId = "other-workspace";
      }
      state.conversations[copy.conversation.id] = copy;
    }
    localStorage.setItem(
      "chat-conversations",
      JSON.stringify(state.conversations),
    );
  }, count);
  await page.getByRole("button", { name: "Chat history", exact: true }).click();
  await expect(page.locator(".chat-history-list")).toHaveAttribute(
    "aria-busy",
    "false",
  );
  return page.getByRole("complementary", { name: "Chat history" });
}

test("history renames the open conversation and persists pins across reloads", async ({
  page,
}) => {
  const sidebar = await setup(page);
  const current = sidebar.getByRole("button", { name: "Chat AI", exact: true });
  await current.click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "Zmień nazwę", exact: true })
    .click();
  const dialog = page.getByRole("dialog", { name: "Rename conversation" });
  await dialog
    .getByRole("textbox", { name: "Conversation name" })
    .fill("Projekt interfejsu");
  await dialog.getByRole("button", { name: "Save name" }).click();
  await expect(
    page.getByRole("tab", { name: "Projekt interfejsu", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  const renamed = sidebar.getByRole("button", {
    name: "Projekt interfejsu",
    exact: true,
  });
  await renamed.focus();
  await renamed.press("Shift+F10");
  await expect(
    page.getByRole("menuitem", { name: "Zmień nazwę" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(renamed).toBeFocused();
  await renamed.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Przypnij", exact: true }).click();
  await expect(
    sidebar
      .getByRole("region", { name: "Pinned" })
      .getByRole("button", { name: "Projekt interfejsu", exact: true }),
  ).toBeVisible();
  await page.reload();
  await page.getByRole("button", { name: "Chat history", exact: true }).click();
  const pinned = sidebar
    .getByRole("region", { name: "Pinned" })
    .getByRole("button", { name: "Projekt interfejsu", exact: true });
  await expect(pinned).toBeVisible();
  await pinned.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Odepnij", exact: true }).click();
  await expect(
    sidebar
      .getByRole("region", { name: "Recent" })
      .getByRole("button", { name: "Projekt interfejsu", exact: true }),
  ).toBeVisible();
});

test("delete requires confirmation and preserves the conversation on storage failure", async ({
  page,
}) => {
  const sidebar = await setup(page);
  const row = sidebar.getByRole("button", {
    name: "Plan aplikacji",
    exact: true,
  });
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Usuń konwersację" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Delete conversation?",
    exact: true,
  });
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(row).toBeVisible();
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Usuń konwersację" }).click();
  await page.evaluate(() => {
    (window as any).__chatTest.failHistoryAction = "delete";
  });
  await dialog
    .getByRole("button", { name: "Delete conversation", exact: true })
    .click();
  await expect(dialog.getByRole("alert")).toContainText(
    "History fixture failure",
  );
  expect(
    await page.evaluate(
      () => !!(window as any).__chatTest.conversations["history-0"],
    ),
  ).toBe(true);
  await page.evaluate(() => {
    (window as any).__chatTest.failHistoryAction = "";
  });
  await dialog
    .getByRole("button", { name: "Delete conversation", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(row).toHaveCount(0);
  await expect(
    sidebar.getByRole("button", { name: "Pomysły na interfejs", exact: true }),
  ).toBeVisible();
  await sidebar
    .getByRole("button", { name: "Chat AI", exact: true })
    .click({ button: "right" });
  await page.getByRole("menuitem", { name: "Usuń konwersację" }).click();
  await dialog
    .getByRole("button", { name: "Delete conversation", exact: true })
    .click();
  await expect(page.locator(".chat-pane")).toHaveCount(0);
  await expect(page.locator(".xterm-screen")).toHaveCount(1);
  expect(
    await page.evaluate(
      () => Object.keys((window as any).__chatTest.conversations).length,
    ),
  ).toBe(7);
});

test("history paginates all conversations and ignores late search results", async ({
  page,
}) => {
  const sidebar = await setup(page, 55);
  await expect(sidebar.locator(".chat-history-item")).toHaveCount(50);
  await sidebar.getByRole("button", { name: "Load more", exact: true }).click();
  await expect(sidebar.locator(".chat-history-item")).toHaveCount(56);
  await page.evaluate(() => {
    (window as any).__chatTest.holdLists = true;
  });
  const search = sidebar.getByRole("searchbox", {
    name: "Search conversations",
  });
  await search.fill("Plan");
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__chatTest.pendingLists.length),
    )
    .toBe(1);
  await search.fill("Pomysły");
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__chatTest.pendingLists.length),
    )
    .toBe(2);
  await page.evaluate(() => {
    (window as any).__chatTest.pendingLists[1].resolve();
  });
  await expect(sidebar.locator(".chat-history-item")).toHaveCount(1);
  await expect(sidebar.locator(".chat-history-item")).toContainText(
    "Pomysły na interfejs",
  );
  await page.evaluate(() => {
    (window as any).__chatTest.pendingLists[0].resolve();
  });
  await expect(sidebar.locator(".chat-history-item")).toHaveCount(1);
  await expect(sidebar.locator(".chat-history-item")).toContainText(
    "Pomysły na interfejs",
  );
});

test("history stays inside its pane and restores focus and draft at narrow sizes", async ({
  page,
}) => {
  const sidebar = await setup(page);
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Zachowaj mój szkic");
  const pane = await page.locator(".chat-pane").boundingBox();
  const panel = await sidebar.boundingBox();
  const conversation = await page.locator(".chat-conversation").boundingBox();
  await expect.poll(async () => (await sidebar.boundingBox())!.x).toBe(pane!.x);
  expect(panel!.width).toBeLessThan(pane!.width / 2);
  expect(conversation!.x).toBeGreaterThan(panel!.x);
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "dark");
  await page.screenshot({
    path: test.info().outputPath("chat-history-dark.png"),
  });
  await sidebar
    .getByRole("button", { name: "Plan aplikacji", exact: true })
    .click({ button: "right" });
  await page.screenshot({
    path: test.info().outputPath("chat-history-menu.png"),
  });
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 680, height: 500 });
  await page.locator(".chat-history-drawer").evaluate(async (element) => {
    await Promise.all(
      element
        .getAnimations({ subtree: true })
        .map((animation) => animation.finished),
    );
  });
  await expect(input).toBeHidden();
  await expect(sidebar).toBeInViewport();
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  await page.screenshot({
    path: test.info().outputPath("chat-history-narrow-light.png"),
  });
  await sidebar
    .getByRole("searchbox", { name: "Search conversations" })
    .press("Escape");
  await expect(sidebar).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Chat history", exact: true }),
  ).toBeFocused();
  await expect(input).toBeVisible();
  await expect(input).toHaveValue("Zachowaj mój szkic");
});

test("history reverses an interrupted close and respects reduced motion", async ({
  page,
}) => {
  const sidebar = await setup(page);
  const drawer = page.locator(".chat-history-drawer");
  await page.mouse.move(1, 1);
  await expect(sidebar.getByRole("combobox")).toHaveCount(0);
  await expect(
    sidebar.getByRole("button", { name: "Actions for Plan aplikacji" }),
  ).toHaveCSS("opacity", "1");
  await sidebar.getByRole("searchbox").fill("Plan");
  await drawer.evaluate(async (element) => {
    await Promise.all(
      element
        .getAnimations({ subtree: true })
        .map((animation) => animation.finished),
    );
  });
  const motion = await page.evaluate(async () => {
    const drawer = document.querySelector<HTMLElement>(".chat-history-drawer")!;
    const trigger = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Chat history"]',
    )!;
    const frame = () =>
      new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const width = () => drawer.getBoundingClientRect().width;
    const initial = width();
    trigger.click();
    let closing = initial;
    for (let i = 0; i < 20; i++) {
      await frame();
      closing = width();
      if (closing < initial) break;
    }
    const inertWhileClosing = drawer.inert;
    trigger.click();
    await frame();
    await frame();
    await Promise.all(
      drawer
        .getAnimations({ subtree: true })
        .map((animation) => animation.finished),
    );
    return {
      initial,
      closing,
      final: width(),
      inertWhileClosing,
      inertAfterReopen: drawer.inert,
    };
  });
  expect(motion.closing).toBeGreaterThan(0);
  expect(motion.closing).toBeLessThan(motion.initial);
  expect(motion.final).toBe(motion.initial);
  expect(motion.inertWhileClosing).toBe(true);
  expect(motion.inertAfterReopen).toBe(false);
  await expect(sidebar.getByRole("searchbox")).toHaveValue("Plan");
  await expect(sidebar.getByRole("searchbox")).toBeFocused();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await sidebar.getByRole("button", { name: "Close history" }).click();
  await expect(drawer).toHaveCSS("width", "0px");
  await expect(drawer).toHaveCSS("visibility", "hidden");
  await page.getByRole("button", { name: "Chat history", exact: true }).click();
  await expect(sidebar).toBeVisible();
  expect(
    await drawer.evaluate(
      (element) => element.getAnimations({ subtree: true }).length,
    ),
  ).toBe(0);
});
