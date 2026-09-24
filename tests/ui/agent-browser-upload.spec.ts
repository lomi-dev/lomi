import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";

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
            interaction: { type: "upload" },
            ...input,
          }),
        ),
      );
    },
    { script, input },
  );
}

test("file upload uses exact same-origin refs, bounded bytes and synthetic events", async ({
  page,
}) => {
  await page.route("**/*", (route) =>
    route.fulfill({
      contentType: "text/html",
      body:
        new URL(route.request().url()).pathname === "/child"
          ? '<label>Child file<input id="file" type="file"></label>'
          : '<label>Main file<input id="file" type="file"></label><iframe src="/child" style="height:240px"></iframe>',
    }),
  );
  await page.goto("http://127.0.0.1:1421/upload");
  await expect(page.frameLocator("iframe").locator("input")).toBeVisible();
  for (const frameId of ["main", "f1"]) {
    const snapshot = await run(page, { action: "snapshot" });
    const ref = snapshot.elements.find(
      (e: any) => e.role === "file_input" && e.frameId === frameId,
    );
    expect(ref.valueLength).toBeNull();
    const base = { elementRef: ref.elementRef, frameId };
    const target = await run(page, { ...base, action: "upload_prepare" });
    expect(target.origin).toBe("http://127.0.0.1:1421");
    const frame =
      frameId === "main"
        ? page.mainFrame()
        : page.frames().find((f) => f.url().endsWith("/child"))!;
    await frame.evaluate(() => {
      (globalThis as any).events = [];
      for (const type of ["input", "change"])
        document
          .querySelector("input")!
          .addEventListener(type, (e) =>
            (globalThis as any).events.push([e.type, e.isTrusted]),
          );
    });
    expect(
      await run(page, {
        ...base,
        action: "interact",
        interaction: { type: "click" },
      }),
    ).toEqual({ error: "UNSUPPORTED_CAPABILITY", noEffect: true });
    expect(
      await run(page, {
        ...base,
        action: "upload",
        documentUrl: target.documentUrl,
        fileName: "Zażółć.bin",
        mediaType: "application/octet-stream",
        byteLength: 5,
        base64: "AP9BDQo=",
      }),
    ).toEqual({ dispatched: true, frameId });
    expect(
      await frame.evaluate(async () => {
        const file = document.querySelector("input")!.files![0];
        return {
          name: file.name,
          bytes: [...new Uint8Array(await file.arrayBuffer())],
          events: (globalThis as any).events,
        };
      }),
    ).toEqual({
      name: "Zażółć.bin",
      bytes: [0, 255, 65, 13, 10],
      events: [
        ["input", false],
        ["change", false],
      ],
    });
    const privateSnapshot = await run(page, { action: "snapshot" });
    expect(JSON.stringify(privateSnapshot)).not.toContain("Zażółć.bin");
    expect(
      await run(page, {
        ...base,
        action: "upload",
        documentUrl: target.documentUrl,
        fileName: "bad/file",
        byteLength: 0,
        base64: "",
      }),
    ).toEqual({ error: "RESOURCE_EXHAUSTED", noEffect: true });
  }
});

test("replacement, occlusion and oversized payloads are refused before file assignment", async ({
  page,
}) => {
  await page.route("**/*", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<label>File<input type="file"></label>',
    }),
  );
  await page.goto("http://127.0.0.1:1421/upload");
  const snapshot = await run(page, { action: "snapshot" });
  const ref = snapshot.elements.find(
    (e: any) => e.role === "file_input",
  ).elementRef;
  const base = {
    action: "upload",
    elementRef: ref,
    frameId: "main",
    documentUrl: page.url(),
    fileName: "empty",
    mediaType: "application/octet-stream",
    byteLength: 0,
    base64: "",
  };
  expect(await run(page, { ...base, byteLength: 4194305 })).toEqual({
    error: "RESOURCE_EXHAUSTED",
    noEffect: true,
  });
  await page.evaluate(() =>
    document.body.insertAdjacentHTML(
      "beforeend",
      '<div id="cover" style="position:fixed;inset:0;z-index:10;background:white"></div>',
    ),
  );
  expect(await run(page, base)).toEqual({
    error: "PANEL_NOT_RENDERABLE",
    noEffect: true,
  });
  await page.evaluate(() => {
    document.querySelector("#cover")!.remove();
    const el = document.querySelector("input")!;
    el.replaceWith(el.cloneNode());
  });
  expect(await run(page, base)).toEqual({
    error: "STALE_SNAPSHOT",
    noEffect: true,
  });
  expect(
    await page
      .locator("input")
      .evaluate((el: HTMLInputElement) => el.files!.length),
  ).toBe(0);
});
