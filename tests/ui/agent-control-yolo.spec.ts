import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("YOLO mode saves both states and reports a failed save", async ({
  page,
}) => {
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

  const yolo = page.getByRole("switch", { name: /YOLO mode/i });
  const automaticStart = page.getByRole("switch", {
    name: /Start MCP server when Lomi opens/,
  });
  await expect(yolo).toBeEnabled();
  await expect(yolo).not.toBeChecked();
  await expect(automaticStart).toBeChecked();

  await page.evaluate(() => {
    (window as any).__nativeTest.failAgentControlYoloModeSave = true;
  });
  await yolo.click();
  await expect(page.getByRole("alert")).toContainText(
    "YOLO mode settings are unavailable",
  );
  await expect(yolo).not.toBeChecked();

  await page.evaluate(() => {
    (window as any).__nativeTest.failAgentControlYoloModeSave = false;
  });
  await yolo.check();
  await expect(yolo).toBeChecked();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(automaticStart).toBeChecked();
  await page.screenshot({ path: "test-results/agent-control-yolo.png" });

  await page.reload();
  const savedYolo = page.getByRole("switch", { name: /YOLO mode/i });
  const savedAutomaticStart = page.getByRole("switch", {
    name: /Start MCP server when Lomi opens/,
  });
  await expect(savedYolo).toBeChecked();
  await expect(savedAutomaticStart).toBeChecked();
  await savedYolo.uncheck();
  await expect(savedYolo).not.toBeChecked();
  await expect(savedAutomaticStart).toBeChecked();

  await page.reload();
  await expect(
    page.getByRole("switch", { name: /YOLO mode/i }),
  ).not.toBeChecked();
  await expect(
    page.getByRole("switch", { name: /Start MCP server when Lomi opens/ }),
  ).toBeChecked();
});
