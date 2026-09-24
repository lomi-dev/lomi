import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function openAgentControlSettings(page: Page) {
  await mockDesktop(page, false, null);
  await page.addInitScript(() => {
    const desktop = window as any;
    const saved = localStorage.getItem("test-agent-control-startup");
    desktop.__nativeTest.agentControlStartup = saved
      ? { supported: true, ...JSON.parse(saved) }
      : {
          supported: true,
          autoStart: true,
          yoloMode: false,
          error: null,
        };
    localStorage.setItem(
      "test-agent-control-startup",
      JSON.stringify(desktop.__nativeTest.agentControlStartup),
    );
  });
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Preferences", exact: true }).click();
}

async function yoloWrites(page: Page) {
  return page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter(
        (call: { command: string }) =>
          call.command === "agent_control_set_yolo_mode",
      )
      .map((call: { args: { enabled: boolean } }) => call.args.enabled),
  );
}

test("cancelling YOLO confirmation with buttons, Escape, or backdrop never saves", async ({
  page,
}) => {
  await openAgentControlSettings(page);
  const yolo = page.getByRole("switch", { name: /YOLO mode/i });
  const confirmation = page.getByRole("alertdialog", {
    name: "Enable YOLO mode?",
  });
  const cancel = confirmation.getByRole("button", { name: "Cancel" });

  await yolo.click();
  await expect(yolo).not.toBeChecked();
  await expect(cancel).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(confirmation).toHaveCount(0);
  await expect(yolo).toBeFocused();

  await yolo.click();
  await confirmation.getByRole("button", { name: "Close dialog" }).click();
  await expect(confirmation).toHaveCount(0);

  await yolo.click();
  await page.mouse.click(12, 12);
  await expect(confirmation).toHaveCount(0);

  await yolo.click();
  await cancel.click();
  await expect(confirmation).toHaveCount(0);
  expect(await yoloWrites(page)).toEqual([]);
  await expect(yolo).not.toBeChecked();
});

test("YOLO requires confirmation, stays open while saving, and disables without a dialog", async ({
  page,
}) => {
  await openAgentControlSettings(page);
  const yolo = page.getByRole("switch", { name: /YOLO mode/i });
  const automaticStart = page.getByRole("switch", {
    name: /Start MCP server when Lomi opens/,
  });
  const confirmation = page.getByRole("alertdialog", {
    name: "Enable YOLO mode?",
  });

  await expect(automaticStart).toBeChecked();
  await yolo.click();
  await expect(yolo).not.toBeChecked();
  await expect(confirmation).toBeVisible();
  await expect(confirmation).toContainText(
    "Local MCP clients will pair automatically",
  );
  await expect(confirmation).toContainText("terminal commands");
  await expect(confirmation).toContainText("chat messages");
  expect(await yoloWrites(page)).toEqual([]);
  await page.screenshot({
    path: "test-results/agent-control-yolo-confirmation.png",
  });

  await page.evaluate(() => {
    (window as any).__nativeTest.agentControlYoloModeSaveDelay = 1000;
  });
  const enable = confirmation.getByRole("button", {
    name: "Enable YOLO mode",
  });
  await enable.click();
  await expect(
    confirmation.getByRole("button", { name: "Saving…" }),
  ).toBeDisabled();
  await expect(
    confirmation.getByRole("button", { name: "Cancel" }),
  ).toBeDisabled();
  await expect(
    confirmation.getByRole("button", { name: "Close dialog" }),
  ).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.mouse.click(12, 12);
  await expect(confirmation).toBeVisible();
  expect(await yoloWrites(page)).toEqual([true]);

  await expect(confirmation).toHaveCount(0);
  await expect(yolo).toBeChecked();
  await expect(yolo).toBeFocused();
  await expect(automaticStart).toBeChecked();

  await page.reload();
  await page.getByRole("tab", { name: "Preferences", exact: true }).click();
  const savedYolo = page.getByRole("switch", { name: /YOLO mode/i });
  const savedAutomaticStart = page.getByRole("switch", {
    name: /Start MCP server when Lomi opens/,
  });
  await expect(savedYolo).toBeChecked();
  await expect(savedAutomaticStart).toBeChecked();
  await savedYolo.uncheck();
  await expect(savedYolo).not.toBeChecked();
  await expect(savedAutomaticStart).toBeChecked();
  await expect(
    page.getByRole("alertdialog", { name: "Enable YOLO mode?" }),
  ).toHaveCount(0);
  expect(await yoloWrites(page)).toEqual([false]);

  await page.reload();
  await page.getByRole("tab", { name: "Preferences", exact: true }).click();
  await expect(
    page.getByRole("switch", { name: /YOLO mode/i }),
  ).not.toBeChecked();
  await expect(
    page.getByRole("switch", {
      name: /Start MCP server when Lomi opens/,
    }),
  ).toBeChecked();
});

test("a failed YOLO save stays in the dialog and can be retried", async ({
  page,
}) => {
  await openAgentControlSettings(page);
  const yolo = page.getByRole("switch", { name: /YOLO mode/i });
  await page.evaluate(() => {
    (window as any).__nativeTest.failAgentControlYoloModeSave = true;
  });

  await yolo.click();
  const confirmation = page.getByRole("alertdialog", {
    name: "Enable YOLO mode?",
  });
  await confirmation.getByRole("button", { name: "Enable YOLO mode" }).click();
  await expect(confirmation.getByRole("alert")).toContainText(
    "YOLO mode settings are unavailable",
  );
  await expect(confirmation).toBeVisible();
  await expect(yolo).not.toBeChecked();
  expect(await yoloWrites(page)).toEqual([true]);

  await page.evaluate(() => {
    (window as any).__nativeTest.failAgentControlYoloModeSave = false;
  });
  await confirmation.getByRole("button", { name: "Enable YOLO mode" }).click();
  await expect(confirmation).toHaveCount(0);
  await expect(yolo).toBeChecked();
  expect(await yoloWrites(page)).toEqual([true, true]);
});
