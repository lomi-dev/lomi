import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  newAndroidTab,
  newProject,
  newSession,
  splitPane,
  type TerminalTab,
} from "../../src/model";
import { mockDesktop } from "./desktop";
import { mockAndroid } from "./android-mock";

const deviceId = "12345678-1234-4567-8123-123456789abc";
const errors = new WeakMap<Page, string[]>();
const phoneControls = [
  "Android Back",
  "Android Home",
  "Android Recent apps",
  "Toggle phone screen",
  "Rotate phone",
  "Install APK…",
  "Save screenshot…",
];

test.beforeEach(async ({ page }) => {
  const messages: string[] = [];
  errors.set(page, messages);
  page.on("pageerror", (error) => messages.push(error.message));
});
test.afterEach(async ({ page }) => expect(errors.get(page)).toEqual([]));

async function openAndroid(page: Page) {
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page
    .getByRole("menuitem", { name: "New android symulator", exact: true })
    .click();
  await expect(page.locator(".android-pane")).toBeVisible();
}

async function fits(locator: Locator) {
  await expect(locator).toBeInViewport();
  expect(
    await locator.evaluate(
      (element) => element.scrollWidth - element.clientWidth,
    ),
  ).toBeLessThanOrEqual(1);
  const bounds = (await locator.boundingBox())!;
  const viewport = locator.page().viewportSize()!;
  expect(bounds.x).toBeGreaterThanOrEqual(0);
  expect(bounds.y).toBeGreaterThanOrEqual(0);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(viewport.width + 1);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(viewport.height + 1);
}

async function untouched(page: Page) {
  expect(
    await page.evaluate(() => ({
      starts: (window as any).__androidTest.startRequests,
      calls: (window as any).__nativeTest.calls.filter((call: any) =>
        [
          "android_install_plan",
          "android_install",
          "android_manage_device",
        ].includes(call.command),
      ),
    })),
  ).toEqual({ starts: 0, calls: [] });
}

for (const mode of ["dark", "light"] as const) {
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 800, height: 420 },
  ]) {
    test(`${mode} unprepared panel offers explicit setup at ${viewport.width}x${viewport.height}`, async ({
      page,
    }, info) => {
      await page.emulateMedia({ colorScheme: mode });
      await page.setViewportSize(viewport);
      await mockDesktop(page, false);
      await mockAndroid(page);
      await page.goto("/");
      await expect(page.locator(".xterm-screen")).toBeVisible();
      await openAndroid(page);
      const pane = page.locator(".android-pane");
      await fits(pane);
      const paneBounds = (await pane.boundingBox())!;
      const screenBounds = (await pane
        .locator(".android-viewport")
        .boundingBox())!;
      expect(screenBounds.y).toBeCloseTo(paneBounds.y, 0);
      expect(screenBounds.height).toBeCloseTo(paneBounds.height, 0);
      const setup = pane.getByRole("button", {
        name: "Set up Android",
        exact: true,
      });
      await fits(setup);
      for (const name of phoneControls)
        await expect(
          pane.getByRole("button", { name, exact: true }),
        ).toBeDisabled();
      await untouched(page);
      await page.screenshot({ path: info.outputPath("android-setup.png") });
      await setup.click();
      await expect(
        page.getByRole("tab", { name: "Settings", exact: true }),
      ).toHaveAttribute("aria-selected", "true");
      await expect(
        page
          .locator(".settings-page")
          .getByRole("heading", { name: "Android", exact: true }),
      ).toBeVisible();
      await untouched(page);
      expect(
        await page.evaluate(
          () =>
            (window as any).__nativeTest.calls.filter(
              (call: any) => call.command === "start_terminal",
            ).length,
        ),
      ).toBe(1);
    });
  }

  test(`${mode} narrow phone keeps close and actions pinned around its scrolling controls`, async ({
    page,
  }, info) => {
    await page.emulateMedia({ colorScheme: mode });
    await page.setViewportSize({ width: 800, height: 420 });
    const project = newProject("/project", "local:bash");
    const tab = project.workspaces[0].tabs[0] as TerminalTab;
    const phone = newAndroidTab(deviceId, "Test phone");
    tab.layout = splitPane(tab.layout, tab.activePaneId, "horizontal", phone);
    tab.activePaneId = phone.id;
    await mockDesktop(page, false, {
      ...newSession(),
      projects: [project],
      activeProjectId: project.id,
    });
    await mockAndroid(page, true);
    await page.goto("/");
    await expect(page.locator(".android-screen")).toBeVisible();
    const pane = page.locator(".android-pane");
    const toolbar = pane.getByRole("complementary", { name: "Phone controls" });
    const close = toolbar.getByRole("button", {
      name: "Close Android panel",
      exact: true,
    });
    const more = toolbar.getByRole("button", {
      name: "Android actions",
      exact: true,
    });
    const middle = toolbar.locator(".android-toolbar-controls");
    await fits(pane);
    await fits(close);
    await fits(more);
    const initial = {
      close: (await close.boundingBox())!,
      more: (await more.boundingBox())!,
    };
    for (const name of phoneControls) {
      const control = toolbar.getByRole("button", { name, exact: true });
      await expect(control).toBeEnabled();
      await control.scrollIntoViewIfNeeded();
      await fits(control);
      await fits(close);
      await fits(more);
    }
    await middle.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    expect((await close.boundingBox())!.y).toBe(initial.close.y);
    expect((await more.boundingBox())!.y).toBe(initial.more.y);
    const middleBounds = (await middle.boundingBox())!;
    expect(initial.more.y).toBeGreaterThanOrEqual(
      middleBounds.y + middleBounds.height - 1,
    );
    await page.screenshot({ path: info.outputPath("android-narrow-rail.png") });
    await more.click();
    await page
      .getByRole("menuitem", {
        name: /^(Preview size \(100%\)|Actual size \(1:1\))$/,
      })
      .click();
    const viewport = pane.locator(".android-viewport");
    expect(
      await viewport.evaluate(
        (element) => element.scrollHeight > element.clientHeight,
      ),
    ).toBe(true);
    await more.click();
    await page
      .getByRole("menuitem", { name: "Fit to panel", exact: true })
      .click();
    await expect
      .poll(() =>
        viewport.evaluate(
          (element) => element.scrollHeight - element.clientHeight,
        ),
      )
      .toBeLessThanOrEqual(1);
    await more.click();
    await expect(
      page.getByRole("menuitem", { name: "Stop", exact: true }),
    ).toBeInViewport();
    await page.keyboard.press("Escape");
    await expect(more).toBeFocused();
    expect(
      await page.evaluate(() => (window as any).__androidTest.startRequests),
    ).toBe(1);
  });
}

test("sleeping, disconnected and stopped phones keep recovery actions reachable", async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await mockDesktop(page, false);
  await mockAndroid(page, true);
  await page.goto("/");
  await openAndroid(page);
  await expect(page.locator(".android-screen")).toBeVisible();
  for (const [phase, action] of [
    ["sleeping", "Wake phone"],
    ["disconnected", "Reconnect"],
  ] as const) {
    await page.evaluate(async (phase) => {
      const fixture = (window as any).__androidTest;
      const value = { ...fixture.state.streams[0], phase, error: null };
      fixture.state.streams = [value];
      await (window as any).__nativeTest.emitEvent("android-changed", {
        kind: "stream",
        value,
      });
    }, phase);
    await fits(
      page
        .locator(".android-pane")
        .getByRole("button", { name: action, exact: true }),
    );
    await page.screenshot({ path: info.outputPath(`android-${phase}.png`) });
  }
  await page
    .getByRole("button", { name: "Android actions", exact: true })
    .click();
  await page.getByRole("menuitem", { name: "Stop", exact: true }).click();
  await fits(
    page
      .locator(".android-pane")
      .getByRole("button", { name: "Start", exact: true }),
  );
  for (const name of phoneControls)
    await expect(
      page
        .locator(".android-toolbar")
        .getByRole("button", { name, exact: true }),
    ).toBeDisabled();
  await page.screenshot({ path: info.outputPath("android-stopped.png") });
  expect(
    await page.evaluate(() => (window as any).__androidTest.startRequests),
  ).toBe(1);
});

test("unsupported and empty-device panels offer settings without starting a phone", async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await mockDesktop(page, false);
  await mockAndroid(page, true);
  await page.addInitScript(() => {
    const state = (window as any).__androidTest.state;
    state.qualified = false;
    state.toolchain.qualified = false;
    state.preferences.defaultDeviceId = null;
    state.devices.devices = [];
  });
  await page.goto("/");
  await openAndroid(page);
  const pane = page.locator(".android-pane");
  await fits(
    pane.getByRole("button", { name: "Android settings", exact: true }),
  );
  await page.screenshot({ path: info.outputPath("android-unsupported.png") });
  await untouched(page);
  await page.evaluate(async () => {
    const state = (window as any).__androidTest.state;
    state.qualified = true;
    state.toolchain.qualified = true;
    await (window as any).__nativeTest.emitEvent("android-changed", {
      kind: "metadata",
    });
  });
  await fits(pane.getByRole("button", { name: "Create a phone", exact: true }));
  await page.screenshot({ path: info.outputPath("android-no-devices.png") });
  await untouched(page);
});
