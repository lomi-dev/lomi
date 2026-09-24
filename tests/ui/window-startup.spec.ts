import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("preloaded settings keep their startup background until first shown", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: unknown,
    ) => {
      const result = await invoke(command, args);
      return command === "show_ready_window" ? false : result;
    };
  });
  await page.goto("/?window=settings");
  const calls = (command: string) =>
    page.evaluate(
      (command) =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === command,
        ).length,
      command,
    );
  await expect.poll(() => calls("show_ready_window")).toBe(1);
  expect(await calls("finish_window_startup")).toBe(0);
  await page.evaluate(() =>
    (window as any).__nativeTest.emitEvent("tauri://focus"),
  );
  await expect.poll(() => calls("finish_window_startup")).toBe(1);
  await page.evaluate(() =>
    (window as any).__nativeTest.emitEvent("tauri://focus"),
  );
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(resolve)),
  );
  expect(await calls("finish_window_startup")).toBe(1);
});

test("settings become ready only after subscribing to requested pages", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const desktop = window as any;
    const pending = new Promise<void>((resolve) => {
      desktop.__subscribeSettings = resolve;
    });
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (
        command === "plugin:event|listen" &&
        args.event === "settings-page-changed"
      )
        await pending;
      return invoke(command, args);
    };
  });
  await page.goto("/?window=settings");
  await expect(page.getByRole("heading", { name: "Keybinds" })).toBeVisible();
  const ready = () =>
    page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "show_ready_window",
      ),
    );
  expect(await ready()).toHaveLength(0);
  await page.evaluate(() => (window as any).__subscribeSettings());
  await expect.poll(ready).toHaveLength(1);
  await page.evaluate(() =>
    (window as any).__nativeTest.emitEvent("settings-page-changed", "terminal"),
  );
  await expect(page.getByRole("heading", { name: "Terminal" })).toBeVisible();
});

test("settings handle a first focus before the preload reply arrives", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    const desktop = window as any;
    const pending = new Promise<void>((resolve) => {
      desktop.__releaseReady = resolve;
    });
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: unknown,
    ) => {
      const result = await invoke(command, args);
      if (command === "show_ready_window") {
        await pending;
        return false;
      }
      return result;
    };
  });
  await page.goto("/?window=settings");
  const calls = (command: string) =>
    page.evaluate(
      (command) =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === command,
        ).length,
      command,
    );
  await expect.poll(() => calls("show_ready_window")).toBe(1);
  await page.evaluate(() =>
    (window as any).__nativeTest.emitEvent("tauri://focus"),
  );
  expect(await calls("finish_window_startup")).toBe(0);
  await page.evaluate(() => (window as any).__releaseReady());
  await expect.poll(() => calls("finish_window_startup")).toBe(1);
});

for (const settings of [false, true]) {
  for (const appearance of ["light", "dark"] as const) {
    test(`${settings ? "settings" : "main"} waits for its ${appearance} theme and view before showing`, async ({
      page,
    }, testInfo) => {
      await page.emulateMedia({
        colorScheme: appearance === "light" ? "dark" : "light",
      });
      await mockDesktop(page, false);
      await page.addInitScript((appearance) => {
        localStorage.setItem(
          "test-theme-settings",
          JSON.stringify({ version: 1, active: null, appearance }),
        );
        const desktop = window as any;
        const state = (desktop.__startup = { requested: false, shown: [] });
        const pending = new Promise<void>((resolve) => {
          state.release = resolve;
        });
        const invoke = desktop.__TAURI_INTERNALS__.invoke;
        desktop.__TAURI_INTERNALS__.invoke = async (
          command: string,
          args: any,
        ) => {
          if (command === "load_theme_preferences") {
            state.requested = true;
            await pending;
          }
          if (command === "show_ready_window") {
            const shell = document.querySelector(".app-shell");
            state.shown.push({
              background: shell && getComputedStyle(shell).backgroundColor,
              nativeBackground: args.background,
              appearance: document.documentElement.dataset.appearance,
              settings: shell?.classList.contains("settings-window"),
            });
          }
          return invoke(command, args);
        };
      }, appearance);
      let release!: () => void;
      const pending = new Promise<void>((resolve) => {
        release = resolve;
      });
      let requested = false;
      await page.route(
        settings ? "**/src/SettingsWindow.tsx*" : "**/src/Workbench.tsx*",
        async (route) => {
          requested = true;
          await pending;
          await route.continue();
        },
      );
      const shown = () => page.evaluate(() => (window as any).__startup.shown);
      try {
        await page.goto(settings ? "/?window=settings" : "/");
        await expect
          .poll(() => page.evaluate(() => (window as any).__startup.requested))
          .toBe(true);
        expect(await shown()).toEqual([]);
        await page.evaluate(() => (window as any).__startup.release());
        await expect.poll(() => requested).toBe(true);
        expect(await shown()).toEqual([]);
      } finally {
        release();
      }
      await expect.poll(shown).toEqual([
        {
          background:
            appearance === "light" ? "rgb(247, 248, 243)" : "rgb(16, 17, 20)",
          nativeBackground:
            appearance === "light" ? [247, 248, 243] : [16, 17, 20],
          appearance,
          settings,
        },
      ]);
      await page.screenshot({ path: testInfo.outputPath("first-window.png") });
    });
  }

  test(`${settings ? "settings" : "main"} still shows its fallback when the saved theme fails`, async ({
    page,
  }) => {
    await mockDesktop(page, false);
    await page.addInitScript(() => {
      (window as any).__nativeTest.themeLoadError = "Cannot read theme";
    });
    await page.goto(settings ? "/?window=settings" : "/");
    await expect(page.locator(".app-shell")).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "show_ready_window",
          ),
        ),
      )
      .toHaveLength(1);
  });
}

test("startup keeps its background until the visible webview has rendered", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    localStorage.setItem(
      "test-theme-settings",
      JSON.stringify({ version: 1, active: "glass", appearance: "dark" }),
    );
    localStorage.setItem(
      "test-theme-manifests",
      JSON.stringify({
        glass: {
          version: 1,
          name: "Glass",
          tokens: {
            "--color-background": "#40608080",
            "--opacity-window": "0.75",
          },
        },
      }),
    );
    const desktop = window as any;
    const state = (desktop.__startup = { requested: false });
    const pending = new Promise<void>((resolve) => {
      state.release = resolve;
    });
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: unknown,
    ) => {
      if (command === "show_ready_window") {
        state.requested = true;
        await pending;
      }
      return invoke(command, args);
    };
  });
  await page.goto("/?window=settings");
  await expect
    .poll(() => page.evaluate(() => (window as any).__startup.requested))
    .toBe(true);
  const finished = () =>
    page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "finish_window_startup",
      ),
    );
  expect(await finished()).toHaveLength(0);
  await page.evaluate(() => {
    const state = (window as any).__startup;
    const requestFrame = window.requestAnimationFrame;
    let frames: FrameRequestCallback[] = [];
    window.requestAnimationFrame = (callback) => frames.push(callback);
    state.frame = () => {
      const pending = frames;
      frames = [];
      pending.forEach((callback) => callback(performance.now()));
    };
    state.resume = () => {
      window.requestAnimationFrame = requestFrame;
      frames.forEach((callback) => requestFrame(callback));
    };
    state.release();
  });
  await page.evaluate(() => (window as any).__startup.frame());
  expect(await finished()).toHaveLength(0);
  await page.evaluate(() => (window as any).__startup.frame());
  await expect.poll(finished).toHaveLength(1);
  await page.evaluate(() => (window as any).__startup.resume());
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.find(
          (call: any) => call.command === "show_ready_window",
        ).args.background,
    ),
  ).toEqual([64, 96, 128]);
  await expect(page.locator(".app-shell")).toHaveCSS(
    "background-color",
    "rgba(64, 96, 128, 0.5)",
  );
  await expect(page.locator(".app-shell")).toHaveCSS("opacity", "0.75");
});
