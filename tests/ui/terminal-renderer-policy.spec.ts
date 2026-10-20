import { expect, test } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

test("native DOM policy avoids WebGL from first attachment and retains font and input behavior", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const state = window as any;
    state.__webglAttempts = 0;
    state.__fontLoads = 0;
    const getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (
      kind: string,
      ...args: any[]
    ) {
      if (kind === "webgl" || kind === "webgl2") state.__webglAttempts++;
      return (getContext as any).call(this, kind, ...args);
    } as typeof getContext;
    const load = FontFaceSet.prototype.load;
    FontFaceSet.prototype.load = function (...args) {
      state.__fontLoads++;
      return load.apply(this, args);
    };
  });
  await page.route("**/src/main.tsx", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      body: `
        const nativeInvoke = window.__TAURI_INTERNALS__.invoke;
        window.__TAURI_INTERNALS__.invoke = async (command, args) => {
          const result = await nativeInvoke(command, args);
          if (command === "write_terminal") window.__nativeTest.emit(args.id, args.data);
          return command === "app_info" ? { ...result, terminalRenderer: "dom" } : result;
        };
      ${await response.text()}`,
    });
  });
  const addonRequests: string[] = [];
  page.on("request", (request) => {
    if (/addon[-_]webgl/.test(request.url())) addonRequests.push(request.url());
  });
  let release!: () => void;
  const fonts = new Promise<void>((resolve) => {
    release = resolve;
  });
  let waitingForFont = false;
  await page.route("**/JetBrainsMono-BoldItalic.woff2", async (route) => {
    waitingForFont = true;
    await fonts;
    await route.continue();
  });
  try {
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect.poll(() => waitingForFont).toBe(true);
    await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "0");
    expect(
      await page.evaluate(() =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ),
      ),
    ).toEqual([]);
    expect(await page.evaluate(() => (window as any).__webglAttempts)).toBe(0);
  } finally {
    release();
  }
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  const id = (await page
    .locator("[data-pane-id]")
    .getAttribute("data-pane-id"))!;
  await expect.poll(() => buffer(page, id)).toContain("bash $ ");
  const initial = await page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(id)!;
    (window as any).__policyRuntime = runtime;
    return {
      renderer: runtime.getSnapshot().renderer,
      sessionId: runtime.sessionId,
      fonts: document.fonts.status,
      fontLoads: (window as any).__fontLoads,
    };
  }, id);
  expect(initial.renderer).toBe("DOM");
  expect(initial.fonts).toBe("loaded");
  expect(initial.fontLoads).toBeGreaterThan(0);
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.type("dom first");
  await expect.poll(() => buffer(page, id)).toContain("dom first");
  await expect(page.locator(".xterm-rows")).toContainText("dom first");

  await page.keyboard.press("Control+Shift+t");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  const beforeReturn = await page.evaluate(() => (window as any).__fontLoads);
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
  const returned = await page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    const runtime = runningTerminal(id)!;
    return {
      sameRuntime: runtime === (window as any).__policyRuntime,
      renderer: runtime.getSnapshot().renderer,
      sessionId: runtime.sessionId,
      webglAttempts: (window as any).__webglAttempts,
      fontLoads: (window as any).__fontLoads,
    };
  }, id);
  expect(returned).toEqual({
    sameRuntime: true,
    renderer: "DOM",
    sessionId: initial.sessionId,
    webglAttempts: 0,
    fontLoads: beforeReturn,
  });
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.type(" retained");
  await expect.poll(() => buffer(page, id)).toContain("dom first retained");
  await expect(page.locator(".xterm-rows")).toContainText("dom first retained");
  expect(addonRequests).toEqual([]);
});
