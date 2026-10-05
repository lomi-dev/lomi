import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import { browserFramePage } from "../mcp/browser-frame-pages.mjs";

test("page-world Promise reports retain primitive reasons and disclose engine trust flags", async ({
  page,
}) => {
  await page.addInitScript({
    content: readFileSync("src-tauri/src/browser/agent-page-logs.js", "utf8"),
  });
  await page.route("**/*", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: browserFramePage(new URL(route.request().url()).pathname),
    }),
  );
  await page.goto("http://127.0.0.1:1421/frames");
  await page.evaluate(() => {
    (window as any).__observedRejections = [];
    addEventListener("unhandledrejection", (event) => {
      (window as any).__observedRejections.push({
        message:
          typeof event.reason === "string" ? event.reason : "[object omitted]",
        kind: "promise_rejection",
        eventTrusted: event.isTrusted,
      });
    });
  });
  await page.locator("#rejections").click();
  await page.evaluate(() =>
    dispatchEvent(
      new PromiseRejectionEvent("unhandledrejection", {
        promise: Promise.resolve(),
        reason: "synthetic-rejection",
      }),
    ),
  );
  const entries = () =>
    page.evaluate(
      () =>
        JSON.parse(
          (window as any).__lomiAgentPageLogsV1(
            JSON.stringify({
              logKind: "promise_rejection",
              after: 0,
              limit: 64,
              origin: location.origin,
              url: location.href,
              deadlineEpochMs: Date.now() + 3000,
            }),
          ),
        ).entries,
    );
  await expect.poll(entries).toHaveLength(3);
  const reports = (await entries())
    .map(({ message, kind, eventTrusted }: any) => ({
      message,
      kind,
      eventTrusted,
    }))
    .sort((a: any, b: any) => a.message.localeCompare(b.message));
  expect(reports.map((entry: any) => entry.message)).toEqual([
    "[object omitted]",
    "native-rejection",
    "synthetic-rejection",
  ]);
  expect(reports).toEqual(
    await page.evaluate(() =>
      (window as any).__observedRejections.sort((a: any, b: any) =>
        a.message.localeCompare(b.message),
      ),
    ),
  );
  expect(
    reports.find((entry: any) => entry.message === "synthetic-rejection")
      .eventTrusted,
  ).toBe(false);
});
