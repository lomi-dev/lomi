import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { browserFramePage } from "../mcp/browser-frame-pages.mjs";

const script = readFileSync("src-tauri/src/browser/agent-dom.js", "utf8");
async function run(page: Page, input: Record<string, unknown>) {
  return page.evaluate(
    async ({ script, input }) => {
      const execute = Object.getPrototypeOf(async function () {}).constructor;
      return JSON.parse(
        await new execute("payload", script)(
          JSON.stringify({
            workspaceId: "workspace",
            panelId: "panel",
            browserGeneration: "generation",
            navigationId: "nav",
            snapshotId: "snapshot",
            maxNodes: 100,
            maxBytes: 16384,
            origin: location.origin,
            url: location.href,
            deadlineEpochMs: Date.now() + 3000,
            ...input,
          }),
        ),
      );
    },
    { script, input },
  );
}
async function setup(page: Page) {
  await page.route("**/*", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: browserFramePage(new URL(route.request().url()).pathname),
    }),
  );
  await page.goto("http://127.0.0.1:1421/frames");
  await expect(
    page.frameLocator("#child").frameLocator("#nested").locator("#save"),
  ).toBeVisible();
}

test("same-origin nested refs retain exact documents and private values stay omitted", async ({
  page,
}) => {
  await setup(page);
  const snapshot = await run(page, { action: "snapshot" });
  expect(snapshot.frames.map((f: any) => f.frameId)).toEqual([
    "main",
    "f1",
    "f2",
  ]);
  expect(snapshot.omittedFrames).toBe(3);
  expect(JSON.stringify(snapshot)).not.toMatch(
    /frame-private|opaque-private|srcdoc-private/,
  );
  const input = snapshot.elements.find(
    (e: any) => e.name === "Nested" && e.role === "textbox",
  );
  expect(input.frameId).toBe("f2");
  expect(
    await run(page, {
      action: "interact",
      elementRef: input.elementRef,
      interaction: { type: "fill", text: "Zażółć 🙂" },
    }),
  ).toMatchObject({ dispatched: true, valueLength: 9 });
  expect(
    await run(page, {
      action: "interact",
      elementRef: input.elementRef,
      interaction: { type: "key", key: "Enter" },
    }),
  ).toMatchObject({ dispatched: true, defaultAction: false });
  const nested = page.frameLocator("#child").frameLocator("#nested");
  await expect(nested.locator("#name")).toHaveAttribute("data-key", "Enter");
  const button = snapshot.elements.find((e: any) => e.name === "Save Nested");
  expect(
    await run(page, {
      action: "interact",
      elementRef: button.elementRef,
      interaction: { type: "click" },
    }),
  ).toMatchObject({ dispatched: true });
  await expect(nested.locator("#result")).toHaveText("Saved: Zażółć 🙂");
  const frame = page
    .frames()
    .find((f) => f.url() === "http://127.0.0.1:1421/nested")!;
  await frame.goto("http://127.0.0.1:1421/nested?replacement");
  expect(
    await run(page, {
      action: "interact",
      elementRef: button.elementRef,
      interaction: { type: "click" },
    }),
  ).toEqual({ error: "STALE_SNAPSHOT", noEffect: true });
});

test("covered, detached and foreign frames never receive input; scroll targets its frame", async ({
  page,
}) => {
  await setup(page);
  const snapshot = await run(page, { action: "snapshot" });
  const button = snapshot.elements.find((e: any) => e.name === "Save Child");
  await page.evaluate(() => {
    const r = document.querySelector("#child")!.getBoundingClientRect();
    const cover = document.createElement("div");
    cover.id = "cover";
    cover.style.cssText = `position:fixed;left:${r.left}px;top:${r.top}px;width:620px;height:80px;z-index:100;background:white`;
    document.body.append(cover);
  });
  expect(
    await run(page, {
      action: "interact",
      elementRef: button.elementRef,
      interaction: { type: "click" },
    }),
  ).toEqual({ error: "PANEL_NOT_RENDERABLE", noEffect: true });
  await page.locator("#cover").evaluate((el) => el.remove());
  expect(
    await run(page, {
      action: "interact",
      elementRef: "f1-viewport",
      interaction: { type: "scroll", deltaX: 0, deltaY: 200 },
    }),
  ).toMatchObject({ dispatched: true, scrollPosition: { y: 200 } });
  expect(await page.evaluate(() => scrollY)).toBe(0);
  const frame = page
    .frames()
    .find((f) => new URL(f.url()).pathname === "/child")!;
  await frame.goto("http://localhost:1421/foreign");
  expect(
    await run(page, {
      action: "interact",
      elementRef: button.elementRef,
      interaction: { type: "click" },
    }),
  ).toEqual({ error: "STALE_SNAPSHOT", noEffect: true });
  const foreign = await run(page, { action: "snapshot" });
  expect(foreign.frames).toHaveLength(1);
  expect(foreign.omittedFrames).toBe(4);
  await page.locator("#child").evaluate((el) => el.remove());
  expect(
    await run(page, {
      action: "interact",
      elementRef: "f1-viewport",
      interaction: { type: "scroll", deltaX: 0, deltaY: 200 },
    }),
  ).toEqual({ error: "STALE_SNAPSHOT", noEffect: true });
});

test("frame traversal shares node, byte, depth and frame budgets", async ({
  page,
}) => {
  await setup(page);
  const small = await run(page, {
    action: "snapshot",
    maxNodes: 2,
    maxBytes: 2048,
  });
  expect(small.truncated).toBe(true);
  expect(small.elements).toHaveLength(2);
  expect(Buffer.byteLength(JSON.stringify(small))).toBeLessThanOrEqual(2048);
  await page.evaluate(async () => {
    document.body.replaceChildren();
    await Promise.all(
      Array.from(
        { length: 25 },
        () =>
          new Promise<void>((resolve) => {
            const frame = document.createElement("iframe");
            frame.src = "/nested";
            frame.onload = () => resolve();
            document.body.append(frame);
          }),
      ),
    );
  });
  const large = await run(page, {
    action: "snapshot",
    maxNodes: 500,
    maxBytes: 49152,
  });
  expect(large.frames).toHaveLength(16);
  expect(large.omittedFrames).toBe(10);
  expect(large.elements.length).toBeLessThanOrEqual(500);
});

test("nested frame depth and transformed ancestor input are bounded", async ({
  page,
}) => {
  await setup(page);
  const snapshot = await run(page, { action: "snapshot" });
  const button = snapshot.elements.find((e: any) => e.name === "Save Child");
  await page
    .locator("#child")
    .evaluate((el) => ((el as HTMLElement).style.transform = "rotate(4deg)"));
  expect(
    await run(page, {
      action: "interact",
      elementRef: button.elementRef,
      interaction: { type: "click" },
    }),
  ).toEqual({ error: "PANEL_NOT_RENDERABLE", noEffect: true });
  await page.goto("http://127.0.0.1:1421/depth/0");
  const deep = await run(page, {
    action: "snapshot",
    maxNodes: 500,
    maxBytes: 49152,
  });
  expect(deep.frames).toHaveLength(5);
  expect(deep.omittedFrames).toBe(1);
  expect(deep.elements.some((e: any) => e.name === "Depth 5")).toBe(false);
});
