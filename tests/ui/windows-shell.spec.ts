import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { newProject, newSession } from "../../src/model";
import { defaultTerminalPreferences } from "../../src/terminal-preferences";
import { chooseOption, mockDesktop } from "./desktop";

async function startedProfiles(page: Page) {
  return page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter((call: any) => call.command === "start_terminal")
      .map((call: any) => call.args.request.profileId),
  );
}

test("Windows shell preference syncs to new tabs while existing and restored panels keep their shell", async ({
  page,
  context,
}, testInfo) => {
  const project = newProject("/project", "local:powershell");
  const saved = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  await mockDesktop(page, false, saved, undefined, {}, "windows");
  await page.goto("/");
  await expect.poll(() => startedProfiles(page)).toEqual(["local:powershell"]);
  const settings = await context.newPage();
  await mockDesktop(settings, false, undefined, undefined, {}, "windows");
  await settings.goto("/?window=settings&page=terminal");
  await settings.getByText("Advanced settings", { exact: true }).click();
  const shell = settings.getByLabel("Default shell", { exact: true });
  await expect(shell).toHaveText("PowerShell");
  await chooseOption(shell, "CMD");
  await expect(settings.getByRole("status")).toHaveText("Saved");
  await expect.poll(() => startedProfiles(page)).toEqual(["local:powershell"]);
  await page.bringToFront();
  await page.locator(".xterm-helper-textarea").focus();
  await page.keyboard.press("Control+Shift+t");
  await expect
    .poll(() => startedProfiles(page))
    .toEqual(["local:powershell", "local:cmd"]);
  await page.keyboard.press("Control+d");
  await expect
    .poll(() => startedProfiles(page))
    .toEqual(["local:powershell", "local:cmd", "local:cmd"]);
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.keyboard.press("Control+d");
  await expect
    .poll(() => startedProfiles(page))
    .toEqual([
      "local:powershell",
      "local:cmd",
      "local:cmd",
      "local:powershell",
    ]);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session")!).projects[0]
            .workspaces[0].tabs[0].layout.type,
      ),
    )
    .toBe("split");
  await settings.reload();
  await settings.getByText("Advanced settings", { exact: true }).click();
  await expect(
    settings.getByLabel("Default shell", { exact: true }),
  ).toHaveText("CMD");
  await settings.getByText("Advanced settings", { exact: true }).click();
  await page.reload();
  await expect
    .poll(() => startedProfiles(page))
    .toEqual(["local:powershell", "local:powershell"]);
  await page.keyboard.press("Control+Shift+t");
  await expect
    .poll(() => startedProfiles(page))
    .toEqual(["local:powershell", "local:powershell", "local:cmd"]);
  const originalTab = page.getByRole("tab", { name: "Terminal", exact: true });
  await originalTab.click();
  await originalTab.click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "Close Others", exact: true })
    .click();
  await expect(page.getByRole("tab")).toHaveCount(1);
  await page.keyboard.press("Control+Shift+w");
  await expect
    .poll(() => startedProfiles(page))
    .toEqual([
      "local:powershell",
      "local:powershell",
      "local:cmd",
      "local:cmd",
    ]);
  await expect(page.locator(".terminal-host")).toHaveCount(1);
  await settings.setViewportSize({ width: 920, height: 680 });
  await settings.screenshot({
    path: testInfo.outputPath("windows-terminal-settings.png"),
  });
  await settings.setViewportSize({ width: 560, height: 420 });
  expect(
    await settings.evaluate(
      () =>
        document.documentElement.scrollWidth <= innerWidth &&
        document.querySelector(".terminal-settings-page")!.scrollWidth <=
          document.querySelector(".terminal-settings-page")!.clientWidth,
    ),
  ).toBe(true);
  await settings.screenshot({
    path: testInfo.outputPath("windows-terminal-settings-small.png"),
  });
});

test("Windows applies a saved CMD default to new projects and workspaces", async ({
  page,
}) => {
  await mockDesktop(page, false, null, undefined, {}, "windows");
  await page.addInitScript(
    (preferences) => {
      localStorage.setItem(
        "test-terminal-preferences",
        JSON.stringify(preferences),
      );
    },
    { version: 1, ...defaultTerminalPreferences, windowsShell: "cmd" },
  );
  await page.goto("/");
  await page
    .getByRole("button", { name: "Open folder or repository", exact: true })
    .click();
  await expect.poll(() => startedProfiles(page)).toEqual(["local:cmd"]);
  await page.getByRole("button", { name: /^Toggle workspaces/ }).click();
  await page
    .getByRole("button", { name: "New workspace", exact: true })
    .click();
  await page.getByRole("textbox", { name: "Name", exact: true }).press("Enter");
  await expect
    .poll(() => startedProfiles(page))
    .toEqual(["local:cmd", "local:cmd"]);
  await expect(page.locator(".workspace-list-item")).toHaveCount(2);
});

for (const platform of ["linux", "macos"] as const)
  test(`${platform} keeps its shell and hides the Windows default`, async ({
    page,
    context,
  }) => {
    await mockDesktop(page, false, undefined, undefined, {}, platform);
    await page.addInitScript(
      (preferences) => {
        localStorage.setItem(
          "test-terminal-preferences",
          JSON.stringify(preferences),
        );
      },
      { version: 1, ...defaultTerminalPreferences, windowsShell: "cmd" },
    );
    await page.goto("/");
    await expect.poll(() => startedProfiles(page)).toEqual(["local:bash"]);
    await page.keyboard.press(
      platform === "macos" ? "Meta+Shift+t" : "Control+Shift+t",
    );
    await expect
      .poll(() => startedProfiles(page))
      .toEqual(["local:bash", "local:bash"]);
    const settings = await context.newPage();
    await mockDesktop(settings, false, undefined, undefined, {}, platform);
    await settings.goto("/?window=settings&page=terminal");
    await expect(
      settings.getByLabel("Font size", { exact: true }),
    ).toBeEnabled();
    await expect(
      settings.getByLabel("Default shell", { exact: true }),
    ).toHaveCount(0);
  });
