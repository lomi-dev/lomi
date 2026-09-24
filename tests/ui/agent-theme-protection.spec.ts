import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

test("package styles cannot hide Agent control or modal ancestors, including a late theme commit", async ({
  page,
}) => {
  await mockDesktop(page, true);
  await page.addInitScript(() => {
    const desktop = window as any;
    const invoke = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (name: string, args: unknown) =>
      name === "agent_control_state"
        ? { supported: true, broker: null, helperPath: null }
        : invoke(name, args);
  });
  await page.goto("/?window=settings&page=agent-control");
  await expect(
    page.getByRole("heading", { name: "Agent control", exact: true }),
  ).toBeVisible();
  const install = () =>
    page.evaluate(async () => {
      const runtime = await import("/src/theme/runtime.ts" as string);
      const format = await import("/src/theme/format.ts" as string);
      const bundle = {
        id: "hostile-style-fixture",
        raw: JSON.stringify({
          version: 2,
          name: "Hostile style fixture",
          common: {
            styles: {
              "html, body, .settings-layout": {
                opacity: "0",
                "pointer-events": "none",
              },
              "button, dialog": { display: "none" },
            },
          },
        }),
        revision: "a".repeat(64),
        directory: "/fixture",
        migration: [],
        readOnly: false,
      };
      await (
        await runtime.prepareTheme(bundle, format.builtinPreferences)
      ).commit();
    });
  const opacity = () =>
    page.evaluate(() => getComputedStyle(document.body).opacity);
  await install();
  await expect.poll(opacity).toBe("1");
  await expect(
    page.getByRole("button", {
      name: "Start server",
      exact: true,
    }),
  ).toBeVisible();
  await install();
  await expect.poll(opacity).toBe("1");
  await page.getByRole("button", { name: "Themes", exact: true }).click();
  await expect.poll(opacity).toBe("0");
  await page.evaluate(async () => {
    const fixture = await import(
      "/tests/ui/protected-theme-fixture.tsx" as string
    );
    fixture.mountProtectedModal();
  });
  await expect(
    page.getByRole("dialog", { name: "Protected decision" }),
  ).toBeVisible();
  await expect.poll(opacity).toBe("1");
  await install();
  await expect(
    page.getByRole("button", { name: "Reject fixture action" }),
  ).toBeVisible();
  await page.evaluate(async () => {
    const fixture = await import(
      "/tests/ui/protected-theme-fixture.tsx" as string
    );
    fixture.mountProtectedModal();
  });
  await expect(page.locator("dialog[open]")).toHaveCount(2);
  await page
    .getByRole("button", { name: "Reject fixture action" })
    .last()
    .click();
  await expect(page.locator("dialog[open]")).toHaveCount(1);
  await expect.poll(opacity).toBe("1");
  await page.getByRole("button", { name: "Reject fixture action" }).click();
  await expect.poll(opacity).toBe("0");
  await page.evaluate(() =>
    (window as any).__nativeTest.emitEvent(
      "settings-page-changed",
      "agent-control",
    ),
  );
  await expect(
    page.getByRole("heading", { name: "Agent control", exact: true }),
  ).toBeVisible();
  await expect.poll(opacity).toBe("1");
  await page.screenshot({
    path: "test-results/agent-theme-protected-controls.png",
  });
});
