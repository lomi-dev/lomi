import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function prepare(
  page: Page,
  platform: "linux" | "windows" | "macos" = "linux",
) {
  await mockDesktop(page, true, undefined, undefined, {}, platform);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    (window as any).__nativeTest.update = {
      rid: 901,
      currentVersion: "0.1.0",
      version: "0.2.0",
      body: "Faster terminals.\n<svg onload=alert('unsafe')> remains plain text.",
      rawJson: {},
    };
    (window as any).__nativeTest.calls.length = 0;
  });
}

async function trigger(page: Page) {
  await page.evaluate(
    () => void (window as any).__nativeTest.emitEvent("check-for-updates"),
  );
  await expect(
    page.getByRole("dialog", { name: "Software update" }),
  ).toBeVisible();
}

async function calls(page: Page, command: string) {
  return page.evaluate(
    (command) =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === command,
      ),
    command,
  );
}

for (const instruction of [
  "yay -Syu lomi-bin",
  "flatpak update",
  "Download the latest package from GitHub Releases and reinstall it with your distribution’s package manager, or replace your AppImage.",
]) {
  test(`Linux startup update provides external instructions: ${instruction.slice(0, 16)}`, async ({
    page,
  }, testInfo) => {
    await prepare(page);
    await page.evaluate(
      (instruction) =>
        ((window as any).__nativeTest.updateInstruction = instruction),
      instruction,
    );
    const dialog = page.getByRole("dialog", { name: "Software update" });
    await expect(dialog).toBeVisible({ timeout: 7000 });
    await expect(dialog).toContainText("0.1.0 → 0.2.0");
    await expect(dialog).toContainText(instruction);
    await expect(dialog.locator("svg[onload]")).toHaveCount(0);
    await expect(
      dialog.getByRole("button", { name: "Update now" }),
    ).toHaveCount(0);
    await dialog.getByRole("button", { name: "Copy instructions" }).click();
    expect(
      (await calls(page, "plugin:clipboard-manager|write_text"))[0].args.text,
    ).toBe(instruction);
    await dialog.getByRole("button", { name: "GitHub Releases" }).click();
    expect((await calls(page, "plugin:opener|open_url"))[0].args.url).toContain(
      "lomi/releases/latest",
    );
    await page.setViewportSize({ width: 800, height: 420 });
    await page.screenshot({ path: testInfo.outputPath("linux-update.png") });
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    expect(await calls(page, "plugin:updater|download")).toHaveLength(0);
    expect(await calls(page, "plugin:resources|close")).toHaveLength(1);
  });
}

test("About requests a check in the workspace; concurrent checks share the request and errors can retry", async ({
  page,
  context,
}) => {
  await prepare(page);
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.update = null;
    mock.updateCheckDelay = 700;
    mock.updateCheckError = "Network unavailable";
  });
  const settings = await context.newPage();
  await mockDesktop(settings);
  await settings.goto("/?window=settings&page=about");
  await settings.getByRole("button", { name: "Check for updates" }).click();
  const dialog = page.getByRole("dialog", { name: "Software update" });
  await expect(dialog).toContainText("Checking for updates");
  await trigger(page);
  await expect(dialog.getByRole("alert")).toContainText("Network unavailable");
  expect(await calls(page, "check_app_update")).toHaveLength(1);
  expect(await calls(settings, "check_app_update")).toHaveLength(0);
  await page.evaluate(
    () => ((window as any).__nativeTest.updateCheckError = ""),
  );
  await dialog.getByRole("button", { name: "Try again" }).click();
  await expect(dialog).toContainText("Lomi is up to date.");
});

for (const platform of ["windows", "macos"] as const) {
  test(`${platform} verifies download before protecting edits and saving the session before installation`, async ({
    page,
  }, testInfo) => {
    await prepare(page, platform);
    await page.getByRole("button", { name: "README.md", exact: true }).click();
    await page.locator(".cm-content").fill("unsaved update work");
    await page.evaluate(() => {
      const mock = (window as any).__nativeTest;
      mock.failFileSave = true;
      mock.updateDownloadDelay = 1000;
      mock.updateContentLength = null;
    });
    await trigger(page);
    const dialog = page.getByRole("dialog", { name: "Software update" });
    await dialog.getByRole("button", { name: "Update now" }).click();
    await expect(dialog.getByRole("progressbar")).not.toHaveAttribute("value");
    await page.keyboard.press("Escape");
    await page.evaluate(
      () =>
        void (window as any).__nativeTest.emitEvent("tauri://close-requested"),
    );
    await expect(dialog).toBeVisible();
    expect(await calls(page, "plugin:window|destroy")).toHaveLength(0);
    const guard = page.getByRole("dialog", {
      name: "Save changes before closing?",
    });
    await expect(guard).toBeVisible();
    await guard
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(guard.getByRole("alert")).toContainText("Disk is full");
    expect(await calls(page, "plugin:updater|install")).toHaveLength(0);
    expect(await calls(page, "reset_terminals")).toHaveLength(0);
    await guard.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(
      dialog.getByRole("button", { name: "Update now" }),
    ).toBeVisible();
    await page.evaluate(
      () => ((window as any).__nativeTest.failFileSave = false),
    );
    await dialog.getByRole("button", { name: "Update now" }).click();
    await guard
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(dialog).toContainText("Update installed.");
    const order = await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.map(
          (call: any) => call.command,
        ) as string[],
    );
    expect(order.lastIndexOf("save_session")).toBeLessThan(
      order.indexOf("plugin:updater|install"),
    );
    expect(order.indexOf("reset_terminals")).toBeLessThan(
      order.indexOf("plugin:updater|install"),
    );
    expect(order.indexOf("plugin:updater|install")).toBeLessThan(
      order.indexOf("restart_after_update"),
    );
    expect(await calls(page, "plugin:updater|download")).toHaveLength(1);
    await page.screenshot({
      path: testInfo.outputPath(`${platform}-update.png`),
    });
  });
}

test("Windows download failures preserve the workspace and offer manual download or retry", async ({
  page,
}, testInfo) => {
  await prepare(page, "windows");
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  await page.locator(".cm-content").fill("work during an interrupted download");
  await page.evaluate(() => {
    (window as any).__nativeTest.updateDownloadError =
      "error decoding response body";
  });
  await trigger(page);
  const dialog = page.getByRole("dialog", { name: "Software update" });
  await dialog.getByRole("button", { name: "Update now" }).click();
  await expect(dialog.getByRole("alert")).toContainText(
    "Could not download or verify the update",
  );
  await expect(dialog.getByRole("alert")).toContainText(
    "error decoding response body",
  );
  expect(await calls(page, "reset_terminals")).toHaveLength(0);
  expect(await calls(page, "plugin:updater|install")).toHaveLength(0);
  expect(await calls(page, "check_app_update")).toHaveLength(1);
  await dialog.getByRole("button", { name: "GitHub Releases" }).click();
  expect((await calls(page, "plugin:opener|open_url"))[0].args.url).toContain(
    "lomi/releases/latest",
  );
  await page.setViewportSize({ width: 800, height: 420 });
  await page.screenshot({
    path: testInfo.outputPath("windows-download-error.png"),
  });
  await page.evaluate(() => {
    (window as any).__nativeTest.updateDownloadError = "";
  });
  await dialog.getByRole("button", { name: "Update now" }).click();
  const guard = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await expect(guard).toBeVisible();
  expect(await calls(page, "plugin:updater|download")).toHaveLength(2);
  for (const call of await calls(page, "plugin:updater|download")) {
    expect(call.args.timeout).toBeUndefined();
  }
  await guard.getByRole("button", { name: "Cancel", exact: true }).click();
  await dialog.getByRole("button", { name: "Later" }).click();
  await expect(page.locator(".cm-content")).toHaveText(
    "work during an interrupted download",
  );
});

test("signature and session-save failures prevent installation; restart failures retry only the restart", async ({
  page,
}) => {
  await prepare(page, "macos");
  await page.evaluate(
    () =>
      ((window as any).__nativeTest.updateDownloadError = "Invalid signature"),
  );
  await trigger(page);
  const dialog = page.getByRole("dialog", { name: "Software update" });
  await dialog.getByRole("button", { name: "Update now" }).click();
  await expect(dialog.getByRole("alert")).toContainText("Invalid signature");
  expect(await calls(page, "reset_terminals")).toHaveLength(0);
  expect(await calls(page, "plugin:updater|install")).toHaveLength(0);
  await page.evaluate(() => {
    (window as any).__nativeTest.updateDownloadError = "";
    (window as any).__nativeTest.failSave = true;
  });
  await dialog.getByRole("button", { name: "Update now" }).click();
  await expect(dialog.getByRole("alert")).toBeVisible();
  expect(await calls(page, "plugin:updater|install")).toHaveLength(0);
  await page.evaluate(() => {
    (window as any).__nativeTest.failSave = false;
    (window as any).__nativeTest.updateRestartError = "Restart unavailable";
  });
  await dialog.getByRole("button", { name: "Update now" }).click();
  await expect(dialog.getByRole("alert")).toContainText(
    "The update is installed",
  );
  await dialog.getByRole("button", { name: "Later" }).click();
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  await page.locator(".cm-content").fill("work after a failed restart");
  await trigger(page);
  await dialog.getByRole("button", { name: "Restart now" }).click();
  const guard = page.getByRole("dialog", {
    name: "Save changes before closing?",
  });
  await guard.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await calls(page, "restart_after_update")).toHaveLength(1);
  await page.evaluate(
    () => ((window as any).__nativeTest.updateRestartError = ""),
  );
  await dialog.getByRole("button", { name: "Restart now" }).click();
  await guard
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(dialog).toContainText("Update installed.");
  expect(await calls(page, "plugin:updater|install")).toHaveLength(1);
  expect(await calls(page, "restart_after_update")).toHaveLength(2);
});
