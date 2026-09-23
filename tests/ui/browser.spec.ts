import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("browser tabs dock beside terminals, retain pages, hide behind menus and restore", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.keyboard.press("Control+Shift+d");
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New browser" }).click();
  await expect(
    page.getByRole("heading", { name: "Browse the web" }),
  ).toBeVisible();
  const address = page.getByRole("combobox", { name: "Web address" });
  await address.fill("localhost:3000");
  await address.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...(window as any).__nativeTest.browsers.values()]
          .filter((browser) => browser.visible)
          .map((browser) => browser.url),
      ),
    )
    .toEqual(["http://localhost:3000/"]);
  await address.fill("https://example.com");
  await address.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...(window as any).__nativeTest.browsers.values()].map(
          (browser) => browser.url,
        ),
      ),
    )
    .toEqual(["https://example.com/"]);
  const terminal = page.getByRole("tab", { name: "Terminal", exact: true });
  await terminal.click();
  const browser = page.getByRole("tab", { name: "Browser", exact: true });
  const source = (await browser.boundingBox())!;
  const target = (await page.locator(".terminal-layout").boundingBox())!;
  await page.mouse.move(
    source.x + source.width / 2,
    source.y + source.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    target.x + target.width - 20,
    target.y + target.height / 2,
    { steps: 10 },
  );
  await expect(page.locator(".tab-merge-preview")).toBeVisible();
  await page.mouse.up();
  await expect(page.getByRole("tab")).toHaveCount(1);
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  await expect(address).toHaveValue("https://example.com/");
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...(window as any).__nativeTest.browsers.values()].map(
          (browser) => browser.visits,
        ),
      ),
    )
    .toEqual([2]);
  await page.getByRole("button", { name: /^New tab/ }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...(window as any).__nativeTest.browsers.values()].some(
          (browser) => browser.visible,
        ),
      ),
    )
    .toBe(false);
  await page.keyboard.press("Escape");
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...(window as any).__nativeTest.browsers.values()].every(
          (browser) => browser.visible,
        ),
      ),
    )
    .toBe(true);
  await page.setViewportSize({ width: 800, height: 420 });
  await expect(address).toBeInViewport();
  await expect(
    page.getByRole("button", { name: "Close browser panel" }),
  ).toBeInViewport();
  await page.screenshot({ path: "test-results/browser-split-minimum.png" });
  await expect
    .poll(() =>
      page.evaluate(() => {
        const tabs = JSON.parse(localStorage.getItem("test-session") ?? "null")
          ?.projects[0].workspaces[0].tabs;
        return { count: tabs?.length, url: tabs?.[0]?.layout?.second?.url };
      }),
    )
    .toEqual({ count: 1, url: "https://example.com/" });
  await page.reload();
  await expect(address).toHaveValue("https://example.com/");
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  await page.getByRole("button", { name: "Close browser panel" }).click();
  await expect(page.locator(".browser-pane")).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.browsers.size))
    .toBe(0);
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  expect(errors).toEqual([]);
});

test("local servers refresh, filter and open in the current browser with mouse or keyboard", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    (window as any).__nativeTest.localWebServers = [
      "http://localhost:3000",
      "http://localhost:5173",
    ];
  });
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New browser" }).click();
  const address = page.getByRole("combobox", { name: "Web address" });
  const list = page.getByRole("listbox", { name: "Local web servers" });
  await expect(list.getByRole("option")).toHaveCount(2);
  await list
    .getByRole("option", { name: "http://localhost:3000", exact: true })
    .click();
  await expect(address).toHaveValue("http://localhost:3000/");
  await expect(list).toBeHidden();
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...(window as any).__nativeTest.browsers.values()]
          .filter((browser) => browser.visible)
          .map((browser) => browser.url),
      ),
    )
    .toEqual(["http://localhost:3000/"]);

  await address.click();
  await expect(list.getByRole("option")).toHaveCount(2);
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...(window as any).__nativeTest.browsers.values()].some(
          (browser) => browser.visible,
        ),
      ),
    )
    .toBe(false);
  await page.setViewportSize({ width: 800, height: 420 });
  await expect(list).toBeInViewport();
  await page.screenshot({ path: "test-results/browser-local-servers.png" });
  await address.fill("5173");
  await expect(list.getByRole("option")).toHaveCount(1);
  await address.press("ArrowDown");
  await expect(list.getByRole("option")).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await address.press("Enter");
  await expect(address).toHaveValue("http://localhost:5173/");
  await expect(list).toBeHidden();

  await address.click();
  await page.evaluate(() => {
    (window as any).__nativeTest.localWebServers = ["http://localhost:3001"];
  });
  await expect(list.getByRole("option")).toHaveText(["http://localhost:3001"], {
    timeout: 8000,
  });
  await address.press("ArrowUp");
  await address.press("Escape");
  await expect(address).toBeFocused();
  await expect(list).toBeHidden();
  const calls = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "local_web_servers",
      ).length,
  );
  await page.waitForTimeout(5100);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "local_web_servers",
        ).length,
    ),
  ).toBe(calls);
  await address.fill("https://example.com");
  await expect(list.getByRole("status")).toHaveText("No matching servers.");
  await address.press("Enter");
  await expect(address).toHaveValue("https://example.com/");
  await expect(page.getByRole("tab")).toHaveCount(2);
});

test("server discovery handles empty lists, failures and late responses after dismissal", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New browser" }).click();
  const address = page.getByRole("combobox", { name: "Web address" });
  const list = page.getByRole("listbox", { name: "Local web servers" });
  await expect(list.getByRole("status")).toHaveText(
    "No local HTTP servers found.",
  );
  await address.press("Escape");
  await page.evaluate(() => {
    (window as any).__nativeTest.localWebServersError =
      "Cannot read local listeners.";
  });
  await address.click();
  await expect(list.getByRole("status")).toHaveText(
    "Cannot read local listeners.",
  );
  await address.press("Escape");
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.localWebServersError = "";
    native.localWebServers = ["http://localhost:3000"];
    native.localWebServersDelay = 400;
    native.localWebServersProgress = [];
  });
  await address.click();
  await expect(list.getByRole("status")).toHaveText(
    "No local HTTP servers found.",
  );
  await address.press("Escape");
  await page.waitForTimeout(500);
  await expect(list).toBeHidden();
  await address.click();
  await expect(list.getByRole("option")).toHaveText("http://localhost:3000");
  await address.press("Tab");
  await expect(list).toBeHidden();
});

test("server results stream before scan completion and stay cached across browser tabs", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.localWebServers = ["http://localhost:3000", "http://localhost:5173"];
    native.localWebServersProgress = ["http://localhost:3000"];
    native.holdLocalWebServers = true;
  });
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New browser" }).click();
  const address = page.getByRole("combobox", { name: "Web address" });
  const list = page.getByRole("listbox", { name: "Local web servers" });
  await expect(list.getByRole("option")).toHaveText(["http://localhost:3000"]);
  expect(
    await page.evaluate(
      () => (window as any).__nativeTest.localWebServersCompleted,
    ),
  ).toBe(0);
  await list.getByRole("option").click();
  await expect(address).toHaveValue("http://localhost:3000/");
  await address.click();
  await expect(list.getByRole("option")).toHaveText(["http://localhost:3000"]);
  await address.press("Escape");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New browser" }).click();
  await expect(list.getByRole("option")).toHaveText(["http://localhost:3000"]);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "local_web_servers",
        ).length,
    ),
  ).toBe(1);
  await page.screenshot({ path: "test-results/browser-streamed-servers.png" });
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.holdLocalWebServers = false;
    native.finishLocalWebServers();
  });
  await expect(list.getByRole("option")).toHaveText(
    ["http://localhost:3000", "http://localhost:5173"],
    { timeout: 7000 },
  );

  await address.press("Escape");
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.localWebServers = ["http://localhost:4000"];
    native.localWebServersProgress = [];
    native.localWebServersDelay = 600;
  });
  await address.click();
  await expect(list.getByRole("option")).toHaveText([
    "http://localhost:3000",
    "http://localhost:5173",
  ]);
  await expect(list.getByRole("status")).toHaveCount(0);
  await expect(list.getByRole("option")).toHaveText(["http://localhost:4000"]);
});

test("late browser state cannot erase a newer navigation error", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New browser" }).click();
  const address = page.getByRole("combobox", { name: "Web address" });
  await address.fill("https://example.com");
  await address.press("Enter");
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.browsers.size))
    .toBe(1);
  await page.evaluate(async () => {
    const native = (window as any).__nativeTest;
    const browser = [...native.browsers.values()][0] as any;
    browser.revision = "9007199254740993";
    browser.error = "Navigation blocked by agent browser permissions.";
    await native.emitEvent("browser-page", { ...browser });
    await native.emitEvent("browser-page", {
      ...browser,
      revision: "9007199254740992",
      error: "",
    });
  });
  await expect(page.getByRole("alert")).toHaveText(
    "Navigation blocked by agent browser permissions.",
  );
  await page.setViewportSize({ width: 850, height: 500 });
  await expect(page.getByRole("alert")).toHaveText(
    "Navigation blocked by agent browser permissions.",
  );
  await page.screenshot({ path: "test-results/browser-navigation-denied.png" });
  await page.evaluate(async () => {
    const native = (window as any).__nativeTest;
    const browser = [...native.browsers.values()][0] as any;
    browser.revision = "9007199254740994";
    browser.error = "";
    await native.emitEvent("browser-page", { ...browser });
  });
  await expect(page.getByRole("alert")).toHaveCount(0);
});
