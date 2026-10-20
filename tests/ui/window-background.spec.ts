import { test, expect, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function install(page: Page) {
  await mockDesktop(page);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(() => {
    const host = window as any;
    host.backgroundCommands = [];
    const original = host.__TAURI_INTERNALS__.invoke;
    host.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "sync_theme_window") {
        host.backgroundCommands.push({
          command,
          theme: document.documentElement.dataset.theme,
        });
        if (host.holdNextThemeSync) {
          host.holdNextThemeSync = false;
          await new Promise<void>((resolve) => {
            host.releaseThemeSync = resolve;
          });
        }
      }
      if (command === "sync_theme_webview_background") {
        host.backgroundCommands.push({
          command,
          background: args.background,
          theme: document.documentElement.dataset.theme,
        });
        return;
      }
      return original(command, args);
    };
  });
  await page.route("**/window-background-fixture", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: `<html><body><div id="root"></div>
<script type="module">
import RefreshRuntime from '/@react-refresh';
RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$ = () => {};
window.$RefreshSig$ = () => (type) => type;
window.__vite_plugin_react_preamble_installed__ = true;
</script>
<script type="module" src="/tests/ui/window-background-fixture.tsx"></script></body></html>`,
    }),
  );
  await page.goto("/window-background-fixture");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).backgroundCommands.filter(
            (x: any) => x.background?.join() === "16,17,20",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
}

const preview = {
  id: "custom-opaque",
  raw: JSON.stringify({
    version: 2,
    name: "Custom opaque",
    appearance: "dark",
    common: { tokens: { "--color-background": "#112233" } },
  }),
  revision: "fixture",
  directory: "/private/theme",
};

test("only an opaque, square, viewport-covering shell without uncertain effects is eligible", async ({
  page,
}) => {
  await install(page);
  const results = await page.evaluate(async () => {
    const { opaqueWindowBackground } = await import(
      "/src/window-background.ts" as string
    );
    const shell = document.querySelector<HTMLElement>(".app-shell")!;
    const initial = opaqueWindowBackground();
    const rejected: unknown[] = [];
    for (const patch of [
      { opacity: "0.99" },
      { backgroundColor: "rgba(16,17,20,.99)" },
      { backgroundColor: "color(srgb .06 .07 .08 / .9999)" },
      { borderTopLeftRadius: "0px 1px" },
      { borderBottomRightRadius: "1px 0px" },
      { maskImage: "linear-gradient(black,black)" },
      { clipPath: "inset(0px)" },
      { filter: "blur(0px)" },
      { transform: "translate(0px)" },
      { width: "90%" },
      { borderLeft: "1px solid transparent" },
    ]) {
      Object.assign(shell.style, patch);
      rejected.push(opaqueWindowBackground());
      shell.removeAttribute("style");
    }
    document.body.style.opacity = "0.5";
    rejected.push(opaqueWindowBackground());
    document.body.removeAttribute("style");
    document.documentElement.dataset.theme = "custom-opaque";
    rejected.push(opaqueWindowBackground());
    return { initial, rejected };
  });
  expect(results.initial).toEqual([16, 17, 20]);
  expect(results.rejected).toEqual(Array(13).fill(null));
});

test("custom preview stays transparent and cancel clears before restoring built-in CSS", async ({
  page,
}) => {
  await install(page);
  await page.evaluate(
    (bundle) => (window as any).themeBackgroundFixture.preview(bundle),
    preview,
  );
  await expect(page.locator("html")).toHaveAttribute("data-theme", preview.id);
  expect(
    await page.evaluate(() => (window as any).backgroundCommands.at(-1)),
  ).toMatchObject({ background: null, theme: preview.id });
  await page.evaluate(() => {
    (window as any).backgroundCommands = [];
    return (window as any).themeBackgroundFixture.cancelPreview();
  });
  const commands = await page.evaluate(
    () => (window as any).backgroundCommands,
  );
  expect(commands[0]).toMatchObject({
    command: "sync_theme_window",
    theme: preview.id,
  });
  expect(
    commands.some(
      (x: any) => x.background?.join() === "16,17,20" && x.theme === "lomi",
    ),
  ).toBe(true);
});

test("a superseded queued preview cannot enable a stale opaque background", async ({
  page,
}) => {
  await install(page);
  await page.evaluate((bundle) => {
    const host = window as any;
    host.backgroundCommands = [];
    host.holdNextThemeSync = true;
    host.previewDone = host.themeBackgroundFixture.preview(bundle);
  }, preview);
  await expect
    .poll(() => page.evaluate(() => !!(window as any).releaseThemeSync))
    .toBe(true);
  await page.evaluate(() => {
    const host = window as any;
    host.cancelDone = host.themeBackgroundFixture.cancelPreview();
    host.releaseThemeSync();
  });
  await page.evaluate(() =>
    Promise.all([(window as any).previewDone, (window as any).cancelDone]),
  );
  await expect(page.locator("html")).toHaveAttribute("data-theme", "lomi");
  const backgrounds = await page.evaluate(() =>
    (window as any).backgroundCommands.filter(
      (x: any) => x.command === "sync_theme_webview_background",
    ),
  );
  expect(backgrounds.length).toBeGreaterThan(0);
  expect(
    backgrounds.every(
      (x: any) => x.theme === "lomi" && x.background?.join() === "16,17,20",
    ),
  ).toBe(true);
});

test("a queued cancellation cannot restore CSS over a newer preview", async ({
  page,
}) => {
  await install(page);
  await page.evaluate(
    (bundle) => (window as any).themeBackgroundFixture.preview(bundle),
    preview,
  );
  await page.evaluate(() => {
    const host = window as any;
    host.holdNextThemeSync = true;
    host.cancelDone = host.themeBackgroundFixture.cancelPreview();
  });
  await expect
    .poll(() => page.evaluate(() => !!(window as any).releaseThemeSync))
    .toBe(true);
  await page.evaluate((bundle) => {
    const host = window as any;
    host.nextPreviewDone = host.themeBackgroundFixture.preview({
      ...bundle,
      id: "newer-preview",
    });
    host.releaseThemeSync();
  }, preview);
  await page.evaluate(() =>
    Promise.all([(window as any).cancelDone, (window as any).nextPreviewDone]),
  );
  await expect(page.locator("html")).toHaveAttribute(
    "data-theme",
    "newer-preview",
  );
  await expect(page.locator("[data-theme-held]")).not.toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).backgroundCommands.at(-1)),
  ).toMatchObject({ background: null, theme: "newer-preview" });
  await page.evaluate(() =>
    (window as any).themeBackgroundFixture.cancelPreview(),
  );
  await expect(page.locator("html")).toHaveAttribute("data-theme", "lomi");
});
