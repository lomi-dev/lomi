import { expect, test, type Locator, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockChats } from "./chat-mock";
import { mockAndroid } from "./android-mock";
import { openNotifications } from "./application-notification-helpers";

async function fits(locator: Locator) {
  await expect(locator).toBeInViewport();
  await expect
    .poll(() =>
      locator.evaluate((element) => element.scrollWidth - element.clientWidth),
    )
    .toBeLessThanOrEqual(1);
  await expect
    .poll(() =>
      locator.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        return (
          bounds.left >= -1 &&
          bounds.top >= -1 &&
          bounds.right <= innerWidth + 1 &&
          bounds.bottom <= innerHeight + 1
        );
      }),
    )
    .toBe(true);
}

async function newView(page: Page, name: string) {
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}

const starts = (page: Page) =>
  page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "start_terminal",
      ).length,
  );

for (const mode of ["dark", "light"] as const)
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 800, height: 420 },
  ]) {
    test.describe(`${mode} desktop ${viewport.width}x${viewport.height}`, () => {
      const errors: string[] = [];
      test.beforeEach(async ({ page }) => {
        errors.length = 0;
        page.on("pageerror", (error) => errors.push(error.message));
        await page.setViewportSize(viewport);
        await page.emulateMedia({ colorScheme: mode });
      });
      test.afterEach(() => expect(errors).toEqual([]));

      test("workbench keeps split terminals and editor history across content views", async ({
        page,
      }, info) => {
        await mockDesktop(page);
        await page.goto("/");
        await expect(page.locator(".xterm-screen")).toBeVisible();
        await page.keyboard.press("Control+Shift+d");
        await expect(page.locator(".xterm-screen")).toHaveCount(2);
        await expect.poll(() => starts(page)).toBe(2);
        await fits(page.locator(".terminal-layout"));
        await page.screenshot({ path: info.outputPath("explorer-split.png") });
        await page
          .getByRole("button", { name: "Toggle workspaces", exact: true })
          .click();
        await fits(
          page.getByRole("complementary", { name: "Workspaces", exact: true }),
        );
        await page.screenshot({ path: info.outputPath("workspaces.png") });
        await page
          .getByRole("button", { name: /^Toggle file explorer/ })
          .click();
        await page
          .getByRole("button", { name: "README.md", exact: true })
          .dblclick();
        const editor = page.locator(".cm-content");
        await expect(editor).toBeVisible();
        const original = await editor.textContent();
        await editor.fill("# Release notes\n\nKeep this draft across tabs.");
        await page.getByRole("tab", { name: "Terminal", exact: true }).click();
        await page
          .getByRole("tab", { name: "README.md Unsaved changes", exact: true })
          .click();
        await expect(editor).toContainText("Keep this draft across tabs.");
        await fits(page.locator(".editor-heading"));
        await expect(
          page.getByRole("button", { name: "Save", exact: true }),
        ).toBeInViewport();
        await page.screenshot({ path: info.outputPath("editor-draft.png") });
        await page.getByRole("button", { name: "Undo", exact: true }).click();
        await expect(editor).toHaveText(original!);
        await page
          .getByRole("button", { name: /^Toggle source control/ })
          .click();
        await expect(
          page.getByRole("button", {
            name: "View diff for README.md",
            exact: true,
          }),
        ).toBeVisible();
        await page
          .getByRole("button", { name: "View diff for README.md", exact: true })
          .click();
        await expect(page.locator(".commit-patch")).toContainText("new");
        await fits(page.locator(".working-file-diff"));
        await page.screenshot({ path: info.outputPath("source-diff.png") });
        await page.getByRole("tab", { name: "History", exact: true }).click();
        await expect(
          page.getByText("No commits yet.", { exact: true }),
        ).toBeVisible();
        await page.screenshot({ path: info.outputPath("source-history.png") });
        await page.keyboard.press("Control+Shift+p");
        const commands = page.getByRole("dialog", {
          name: "Commands",
          exact: true,
        });
        await fits(commands);
        await expect(
          commands.getByRole("textbox", { name: "Find command" }),
        ).toBeFocused();
        await page.screenshot({ path: info.outputPath("commands.png") });
        await page.keyboard.press("Escape");
        await expect(commands).toHaveCount(0);
        const inbox = await openNotifications(page);
        await fits(inbox);
        await page.screenshot({ path: info.outputPath("notifications.png") });
        await page.keyboard.press("Escape");
        expect(await starts(page)).toBe(2);
      });

      test("chat composer and history retain a draft while switching views", async ({
        page,
      }, info) => {
        await mockDesktop(page, false);
        await mockChats(page);
        await page.goto("/");
        await expect(page.locator(".xterm-screen")).toBeVisible();
        await newView(page, "Chat AI");
        const input = page.getByRole("textbox", {
          name: "Message",
          exact: true,
        });
        await expect(input).toBeInViewport();
        await fits(page.locator(".chat-pane"));
        await page.screenshot({ path: info.outputPath("chat-empty.png") });
        await input.fill("Release review");
        await input.press("Enter");
        await expect(page.locator(".chat-message-assistant")).toContainText(
          "日本語",
        );
        await expect(
          page.getByRole("button", { name: "Send", exact: true }),
        ).toBeVisible();
        await input.fill("Keep this follow-up 日本語");
        await page.getByRole("tab", { name: "Terminal", exact: true }).click();
        await page
          .getByRole("tab", { name: "Release review", exact: true })
          .click();
        await expect(input).toHaveValue("Keep this follow-up 日本語");
        await expect(input).toBeInViewport();
        await expect(
          page.getByRole("button", { name: "Send", exact: true }),
        ).toBeInViewport();
        await page.screenshot({
          path: info.outputPath("chat-conversation.png"),
        });
        await page
          .getByRole("button", { name: "Chat history", exact: true })
          .click();
        const history = page.getByRole("complementary", {
          name: "Chat history",
          exact: true,
        });
        await fits(history);
        await expect(
          page.getByRole("searchbox", { name: "Search conversations" }),
        ).toBeInViewport();
        await page.screenshot({ path: info.outputPath("chat-history.png") });
        await page.keyboard.press("Escape");
        await expect(input).toHaveValue("Keep this follow-up 日本語");
        expect(await starts(page)).toBe(1);
      });

      test("browser bounds and phone startup controls fit the workbench", async ({
        page,
      }, info) => {
        await mockDesktop(page, false);
        await mockAndroid(page, true);
        await page.addInitScript(() => {
          const phone = (window as any).__androidTest;
          phone.state.preferences.defaultDeviceId = null;
          phone.holdStart = true;
        });
        await page.goto("/");
        await expect(page.locator(".xterm-screen")).toBeVisible();
        await newView(page, "New browser");
        await expect(
          page.getByRole("heading", { name: "Browse the web" }),
        ).toBeVisible();
        await fits(page.locator(".browser-toolbar"));
        await page.screenshot({ path: info.outputPath("browser-empty.png") });
        const address = page.getByRole("combobox", { name: "Web address" });
        await address.fill("localhost:3000");
        await address.press("Enter");
        await expect
          .poll(() =>
            page.evaluate(
              () =>
                [...(window as any).__nativeTest.browsers.values()].filter(
                  (browser: any) => browser.visible,
                ).length,
            ),
          )
          .toBe(1);
        await expect
          .poll(async () => {
            const viewport = await page
              .locator(".browser-viewport")
              .boundingBox();
            const bounds = await page.evaluate(
              () =>
                [...(window as any).__nativeTest.browsers.values()].find(
                  (browser: any) => browser.visible,
                )?.bounds,
            );
            return (
              !!viewport &&
              !!bounds &&
              Math.abs(viewport.x - bounds.x) <= 1 &&
              Math.abs(viewport.y - bounds.y) <= 1 &&
              Math.abs(viewport.width - bounds.width) <= 1 &&
              Math.abs(viewport.height - bounds.height) <= 1
            );
          })
          .toBe(true);
        await newView(page, "New android symulator");
        await expect(
          page.getByRole("heading", { name: "Choose a phone" }),
        ).toBeVisible();
        await fits(page.locator(".android-phone-picker"));
        await expect(
          page.getByRole("button", { name: "Open Test phone", exact: true }),
        ).toBeInViewport();
        await page.screenshot({ path: info.outputPath("phone-picker.png") });
        await page
          .getByRole("button", { name: "Open Test phone", exact: true })
          .click();
        await expect(
          page.getByRole("button", { name: "Cancel start", exact: true }),
        ).toBeInViewport();
        await page.screenshot({ path: info.outputPath("phone-startup.png") });
        await expect
          .poll(() =>
            page.evaluate(
              () => typeof (window as any).__androidTest.finishStart,
            ),
          )
          .toBe("function");
        await page.evaluate(() => (window as any).__androidTest.finishStart());
        await expect(page.locator(".android-screen")).toBeVisible();
        await expect(
          page.getByRole("button", {
            name: "Close Android panel",
            exact: true,
          }),
        ).toBeInViewport();
        await page.screenshot({ path: info.outputPath("phone-running.png") });
        expect(await starts(page)).toBe(1);
      });
    });
  }
