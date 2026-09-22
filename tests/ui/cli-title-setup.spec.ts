import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

const config = {
  cli: "codex",
  path: "/home/user/.codex/config.toml",
  revision: "original",
};

async function setup(page: Page) {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate((config) => {
    (window as any).__nativeTest.cliTitleSetup = config;
  }, config);
}

async function detect(page: Page, cliRunning = true) {
  await page.evaluate((cliRunning) => {
    const state = (window as any).__nativeTest;
    for (const id of state.sessions.keys())
      state.terminalContexts[id] = {
        cwd: "/project",
        foregroundProgram: cliRunning ? "codex" : null,
        titleCli: cliRunning ? { cli: "codex", pid: 123 } : null,
      };
  }, cliRunning);
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

test("detects Codex and writes only after consent without restarting the PTY", async ({
  page,
}, testInfo) => {
  await setup(page);
  await page.keyboard.type("echo codex");
  await expect.poll(() => calls(page, "terminal_contexts")).not.toHaveLength(0);
  expect(await calls(page, "inspect_cli_titles")).toHaveLength(0);
  await detect(page);
  const dialog = page.getByRole("dialog", {
    name: "Enable Codex terminal titles?",
  });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText(config.path);
  await expect(dialog.getByRole("button", { name: "Not now" })).toBeFocused();
  expect(await calls(page, "enable_cli_titles")).toHaveLength(0);
  await page.setViewportSize({ width: 800, height: 420 });
  await expect(
    dialog.getByRole("button", { name: "Allow changes" }),
  ).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("codex-consent.png") });
  await page.evaluate(() => {
    (window as any).__nativeTest.cliTitleSaveDelay = 300;
  });
  await dialog.getByRole("button", { name: "Allow changes" }).click();
  await expect(dialog.getByRole("button", { name: "Not now" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText(/Codex title settings are ready/)).toBeVisible();
  const saved = await calls(page, "enable_cli_titles");
  expect(saved).toHaveLength(1);
  expect(saved[0].args).toEqual({
    path: config.path,
    revision: config.revision,
    id: expect.any(String),
    process: { cli: "codex", pid: 123 },
  });
  expect(await calls(page, "start_terminal")).toHaveLength(1);
  expect(await calls(page, "close_terminal")).toHaveLength(0);
  expect(await calls(page, "write_terminal")).toEqual(
    expect.not.arrayContaining([
      expect.objectContaining({
        args: expect.objectContaining({ data: "\r" }),
      }),
    ]),
  );
});

test("defers to existing dialogs and does not repeat a declined request for the same config", async ({
  page,
}) => {
  await setup(page);
  await page.getByRole("tab", { name: "Terminal", exact: true }).dblclick();
  const rename = page.getByRole("dialog", { name: "Rename tab" });
  await expect(rename).toBeVisible();
  await detect(page);
  await expect.poll(() => calls(page, "terminal_contexts")).not.toHaveLength(0);
  expect(await calls(page, "inspect_cli_titles")).toHaveLength(0);
  await rename.getByRole("button", { name: "Cancel" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Enable Codex terminal titles?",
  });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Not now" }).click();
  await page.keyboard.press("Control+Shift+t");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await detect(page);
  await expect.poll(() => calls(page, "inspect_cli_titles")).toHaveLength(2);
  await expect(dialog).toHaveCount(0);
  expect(await calls(page, "enable_cli_titles")).toHaveLength(0);
});

test("skips configured titles and requires renewed consent after a save conflict", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.cliTitleSetup = null;
  });
  await detect(page);
  await expect.poll(() => calls(page, "inspect_cli_titles")).toHaveLength(1);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.keyboard.press("Control+Shift+t");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate((config) => {
    const state = (window as any).__nativeTest;
    state.cliTitleSetup = config;
    state.cliTitleError =
      "Codex configuration changed. Check the settings again.";
  }, config);
  await detect(page);
  const dialog = page.getByRole("dialog", {
    name: "Enable Codex terminal titles?",
  });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Allow changes" }).click();
  await expect(dialog.getByRole("alert")).toContainText(
    "configuration changed",
  );
  await page.evaluate((config) => {
    const state = (window as any).__nativeTest;
    state.cliTitleSetup = { ...config, revision: "external-edit" };
    state.cliTitleError = "";
  }, config);
  await dialog.getByRole("button", { name: "Check again" }).click();
  await expect(
    dialog.getByRole("button", { name: "Allow changes" }),
  ).toBeVisible();
  expect(await calls(page, "enable_cli_titles")).toHaveLength(1);
  await dialog.getByRole("button", { name: "Allow changes" }).click();
  await expect(dialog).toHaveCount(0);
  expect((await calls(page, "enable_cli_titles"))[1].args.revision).toBe(
    "external-edit",
  );
});

for (const [cli, name, path] of [
  ["agy", "agy", "/home/user/.gemini/antigravity-cli/settings.json"],
  ["cursor", "Cursor CLI", "/home/user/.cursor/cli-config.json"],
  ["claude", "Claude Code", "/home/user/.claude/settings.json"],
]) {
  test(`requests consent for ${name} after switching CLI in the same terminal`, async ({
    page,
  }, testInfo) => {
    await setup(page);
    await page.evaluate(() => {
      (window as any).__nativeTest.cliTitleSetup = null;
    });
    await detect(page);
    await expect.poll(() => calls(page, "inspect_cli_titles")).toHaveLength(1);
    await page.evaluate(
      ({ cli, path }) => {
        const state = (window as any).__nativeTest;
        state.cliTitleSetup = { cli, path, revision: "new-cli" };
        for (const id of state.sessions.keys())
          state.terminalContexts[id].titleCli = { cli, pid: 456 };
      },
      { cli, path },
    );
    const dialog = page.getByRole("dialog", {
      name: `Enable ${name} terminal titles?`,
    });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(path);
    expect(await calls(page, "enable_cli_titles")).toHaveLength(0);
    if (cli === "agy") {
      await page.setViewportSize({ width: 800, height: 420 });
      await expect(dialog).toContainText("Lomi’s local title formatter");
      const button = await dialog
        .getByRole("button", { name: "Allow changes" })
        .boundingBox();
      expect(button!.y + button!.height).toBeLessThan(420);
      await page.screenshot({ path: testInfo.outputPath("agy-consent.png") });
      await page.clock.install();
    }
    await dialog.getByRole("button", { name: "Allow changes" }).click();
    await expect(dialog).toHaveCount(0);
    if (cli === "agy") {
      const activation = page.getByRole("dialog", {
        name: "Activate agy terminal titles",
      });
      await expect(activation).toContainText("Settings saved.");
      await expect(activation).toContainText("/title on");
      await expect(activation).toContainText(
        "/resume alone does not activate titles",
      );
      await expect(
        activation.getByRole("button", { name: "Got it" }),
      ).toBeFocused();
      await page.clock.fastForward(6000);
      await expect(activation).toBeVisible();
      await page.screenshot({
        path: testInfo.outputPath("agy-activation.png"),
      });
      await activation.getByRole("button", { name: "Got it" }).click();
      await expect(activation).toHaveCount(0);
      expect(await calls(page, "write_terminal")).toHaveLength(0);
    }
    const saved = await calls(page, "enable_cli_titles");
    expect(saved).toHaveLength(1);
    expect(saved[0].args).toEqual({
      id: expect.any(String),
      process: { cli, pid: 456 },
      path,
      revision: "new-cli",
    });
    if (cli !== "agy")
      await expect(
        page.getByText(`${name} title settings are ready.`, { exact: false }),
      ).toBeVisible();
    expect(await calls(page, "start_terminal")).toHaveLength(1);
    expect(await calls(page, "close_terminal")).toHaveLength(0);
  });
}
