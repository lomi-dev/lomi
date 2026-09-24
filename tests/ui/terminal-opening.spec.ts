import { expect, test } from "@playwright/test";
import { buffer, mockDesktop } from "./desktop";

for (const mode of ["WebGL", "DOM", "reduced motion"] as const) {
  test(`${mode}: new terminals reveal their first output without replaying the entrance on tab switches`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({
      reducedMotion: mode === "reduced motion" ? "reduce" : "no-preference",
    });
    await mockDesktop(page, false);
    await page.addInitScript((mode) => {
      const desktop = window as any;
      desktop.__nativeTest.terminalOutputDelay = 60000;
      desktop.__entrances = [];
      const animate = HTMLElement.prototype.animate;
      HTMLElement.prototype.animate = function (...args) {
        const animation = animate.apply(this, args);
        if (this.classList.contains("terminal-host")) {
          const duration = Number(animation.effect?.getTiming().duration ?? 0);
          animation.pause();
          animation.currentTime = duration / 2;
          desktop.__entrances.push({ animation, duration });
        }
        return animation;
      };
      if (mode === "DOM") {
        const getContext = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (
          kind: string,
          ...args: any[]
        ) {
          return kind === "webgl2"
            ? null
            : (getContext as any).call(this, kind, ...args);
        } as typeof getContext;
      }
    }, mode);
    await page.goto("/");
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__nativeTest.sessions.size),
      )
      .toBe(1);
    const first = (await page
      .locator("[data-pane-id]")
      .getAttribute("data-pane-id"))!;
    const host = page.locator(".terminal-host");
    await expect(host).toHaveCSS("opacity", "0");
    await expect(page.locator(".xterm-helper-textarea")).toBeFocused();
    await page.keyboard.insertText("input before prompt");
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as any).__nativeTest.calls
            .filter((call: any) => call.command === "write_terminal")
            .map((call: any) => call.args.data)
            .join(""),
        ),
      )
      .toBe("input before prompt");
    await page.evaluate(() => {
      const desktop = (window as any).__nativeTest;
      desktop.emit(
        [...desktop.sessions.keys()][0],
        "\x1b]133;A\x07[project]$ \x1b]133;B\x07",
      );
    });
    await expect.poll(() => buffer(page, first)).toContain("[project]$ ");
    if (mode === "reduced motion") {
      await expect(host).toHaveCSS("opacity", "1");
      expect(
        await page.evaluate(() => (window as any).__entrances.length),
      ).toBe(0);
    } else {
      await expect
        .poll(() => page.evaluate(() => (window as any).__entrances.length))
        .toBe(1);
      expect(
        await page.evaluate(() => (window as any).__entrances[0].duration),
      ).toBeLessThanOrEqual(100);
      const opacity = Number(
        await host.evaluate((host) => getComputedStyle(host).opacity),
      );
      expect(opacity).toBeGreaterThan(0);
      expect(opacity).toBeLessThan(1);
      await host.screenshot({
        path: testInfo.outputPath(`terminal-opening-${mode}.png`),
      });
      await page.evaluate(() =>
        (window as any).__entrances[0].animation.finish(),
      );
      await expect(host).toHaveCSS("opacity", "1");
    }
    await page.evaluate(() => {
      (window as any).__nativeTest.terminalOutputDelay = 10;
    });
    await page.keyboard.press("Control+Shift+t");
    await expect(page.getByRole("tab")).toHaveCount(2);
    await page.getByRole("tab", { name: "Terminal", exact: true }).click();
    await expect(
      page.locator(`[data-pane-id="${first}"] .terminal-host`),
    ).toHaveCSS("opacity", "1");
    await expect(page.locator(".xterm-helper-textarea")).toBeFocused();
    const returned = await page.evaluate(async (id) => {
      const { runningTerminal } = await import("/src/terminal-runtime.ts");
      const runtime = runningTerminal(id)!;
      return {
        animations: (window as any).__entrances.filter(
          ({ animation }: { animation: Animation }) =>
            (animation.effect as KeyframeEffect).target === runtime.host,
        ).length,
        active: runtime.host.getAnimations().length,
        renderer: runtime.getSnapshot().renderer,
        starts: (window as any).__nativeTest.calls.filter(
          (call: any) =>
            call.command === "start_terminal" &&
            call.args.request.id === runtime.sessionId,
        ).length,
      };
    }, first);
    expect(returned).toEqual({
      animations: mode === "reduced motion" ? 0 : 1,
      active: 0,
      renderer: mode === "DOM" ? "DOM" : "WebGL",
      starts: 1,
    });
  });
}
