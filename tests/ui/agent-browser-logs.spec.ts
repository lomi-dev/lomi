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
  await page.locator("#rejections").click();
  await page.evaluate(() =>
    dispatchEvent(
      new PromiseRejectionEvent("unhandledrejection", {
        promise: Promise.resolve(),
        reason: "synthetic-rejection",
      }),
    ),
  );
  await expect
    .poll(() =>
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
      ),
    )
    .toMatchObject([
      {
        message: "native-rejection",
        kind: "promise_rejection",
        eventTrusted: false,
      },
      {
        message: "[object omitted]",
        kind: "promise_rejection",
        eventTrusted: false,
      },
      {
        message: "synthetic-rejection",
        kind: "promise_rejection",
        eventTrusted: false,
      },
    ]);
});
