import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("context polling stays single-flight, retries failures and ignores unmounted results", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.clock.install();
  await page.route("**/src/main.tsx", (route) =>
    route.fulfill({
      contentType: "text/javascript",
      body: `
        import React from "/node_modules/.vite/deps/react.js";
        import ReactDOM from "/node_modules/.vite/deps/react-dom_client.js";
        import App from "/src/App.tsx";
        import { initializeAppearance } from "/src/theme/runtime.ts";
        import "/src/styles.css";
        const state = window.__contextTest = {
          requests: [], active: 0, maximum: 0, observations: [],
        };
        const invoke = window.__TAURI_INTERNALS__.invoke;
        window.__TAURI_INTERNALS__.invoke = (command, args) => {
          if (command !== "terminal_contexts") return invoke(command, args);
          ++state.active;
          state.maximum = Math.max(state.maximum, state.active);
          return new Promise((resolve, reject) => {
            state.requests.push({ resolve, reject });
          }).finally(() => { --state.active; });
        };
        initializeAppearance();
        state.root = ReactDOM.createRoot(document.getElementById("root"));
        state.root.render(React.createElement(App));
      `,
    }),
  );
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(async () => {
    const modulePath = "/src/terminal-runtime.ts";
    const { runningTerminal } = await import(modulePath);
    const id = document
      .querySelector("[data-pane-id]")!
      .getAttribute("data-pane-id")!;
    const runtime = runningTerminal(id)!;
    const state = (window as any).__contextTest;
    state.sessionId = runtime.sessionId;
    const observe = runtime.observeForegroundProgram.bind(runtime);
    runtime.observeForegroundProgram = (value: string | null) => {
      state.observations.push(value);
      observe(value);
    };
  });

  await page.clock.runFor(4000);
  expect(
    await page.evaluate(() => (window as any).__contextTest.requests.length),
  ).toBe(1);
  expect(await page.evaluate(() => (window as any).__contextTest.maximum)).toBe(
    1,
  );

  await page.evaluate(() => {
    (window as any).__contextTest.requests[0].reject(
      new Error("Slow scan failed"),
    );
  });
  await page.clock.runFor(1000);
  expect(
    await page.evaluate(() => (window as any).__contextTest.requests.length),
  ).toBe(2);
  await page.evaluate(() => {
    const state = (window as any).__contextTest;
    state.requests[1].resolve({
      [state.sessionId]: {
        cwd: "/project/current",
        foregroundProgram: "current",
      },
    });
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).__contextTest.observations))
    .toEqual(["current"]);

  await page.clock.runFor(1000);
  expect(
    await page.evaluate(() => (window as any).__contextTest.requests.length),
  ).toBe(3);
  await page.evaluate(async () => {
    const state = (window as any).__contextTest;
    state.root.unmount();
    state.requests[2].resolve({
      [state.sessionId]: { cwd: "/project/stale", foregroundProgram: "stale" },
    });
    // Flush the invoke result and its observers after React effect cleanup.
    await new Promise<void>((resolve) => queueMicrotask(resolve));
    await new Promise<void>((resolve) => queueMicrotask(resolve));
  });
  await page.clock.runFor(3000);
  expect(
    await page.evaluate(() => ({
      observations: (window as any).__contextTest.observations,
      requests: (window as any).__contextTest.requests.length,
      active: (window as any).__contextTest.active,
      maximum: (window as any).__contextTest.maximum,
    })),
  ).toEqual({ observations: ["current"], requests: 3, active: 0, maximum: 1 });
});
