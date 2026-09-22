import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { newProject, newSession, splitPane } from "../../src/model";
import { mockDesktop } from "./desktop";

const icon = readFileSync("src-tauri/icons/icon.ico").toString("base64");

async function setup(page: Page, saved?: unknown) {
  await mockDesktop(page, false, saved);
  await page.addInitScript(
    ({ icon }) => {
      const native = (window as any).__nativeTest;
      const bridge = (window as any).__TAURI_INTERNALS__;
      const invoke = bridge.invoke;
      const canvas = document.createElement("canvas");
      canvas.width = 1600;
      canvas.height = 1000;
      const context = canvas.getContext("2d")!;
      context.fillStyle = "#737373";
      context.fillRect(150, 150, 1300, 700);
      context.fillStyle = "#292929";
      context.fillRect(200, 200, 1200, 600);
      context.fillStyle = "#e5e5e5";
      context.font = "60px monospace";
      context.fillText("Lomi", 350, 525);
      const png = canvas.toDataURL("image/png").split(",")[1];
      native.imageFiles = {
        "picture.PNG": png,
        "photo.jpg": canvas.toDataURL("image/jpeg").split(",")[1],
        "photo.webp": canvas.toDataURL("image/webp").split(",")[1],
        "favicon.ico": icon,
        "animation.gif":
          "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
        "converted.tiff": png,
        "broken.png": btoa("not an image"),
      };
      native.imageError = "";
      native.imageDelays = {};
      bridge.invoke = async (command: string, args: any = {}) => {
        if (command === "list_directory") {
          const entries = await invoke(command, args);
          return args.relative
            ? entries
            : [
                ...entries,
                ...Object.keys(native.imageFiles).map((name) => ({
                  name,
                  relativePath: name,
                  path: `${args.root}/${name}`,
                  isDirectory: false,
                  isSymlink: false,
                })),
              ];
        }
        if (command === "read_image_file") {
          native.calls.push({ command, args });
          const bytes = native.imageFiles[args.relative];
          if (native.imageDelays[args.relative])
            await new Promise((resolve) =>
              setTimeout(resolve, native.imageDelays[args.relative]),
            );
          if (native.imageError) throw new Error(native.imageError);
          if (!bytes) throw new Error("Image no longer exists.");
          return Uint8Array.from(atob(bytes), (char) => char.charCodeAt(0))
            .buffer;
        }
        return invoke(command, args);
      };
    },
    { icon },
  );
  await page.goto("/");
}

async function openImage(page: Page, name = "picture.PNG") {
  await page.getByRole("button", { name, exact: true }).click();
  await expect(page.locator(".image-canvas img")).toBeVisible();
}

test("Explorer opens image formats in reusable file tabs without editor buffers or extra shells", async ({
  page,
}) => {
  await setup(page);
  await expect(page.locator(".xterm-screen")).toBeVisible();
  for (const name of [
    "picture.PNG",
    "photo.jpg",
    "photo.webp",
    "favicon.ico",
    "animation.gif",
    "converted.tiff",
  ]) {
    await openImage(page, name);
    await expect(page.getByRole("tab", { name: new RegExp(name) })).toHaveCount(
      1,
    );
    await page.keyboard.press("Control+s");
    await expect(page.locator(".cm-content")).toHaveCount(0);
  }
  await openImage(page);
  await openImage(page);
  await expect(page.getByRole("tab")).toHaveCount(7);
  const calls = await page.evaluate(() => (window as any).__nativeTest.calls);
  expect(
    calls.filter((call: any) => call.command === "start_terminal"),
  ).toHaveLength(1);
  expect(
    calls.filter((call: any) =>
      ["read_editor_file", "save_editor_file", "close_terminal"].includes(
        call.command,
      ),
    ),
  ).toEqual([]);
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("test-session")))
    .toContain("picture.PNG");
  await page.reload();
  await expect(page.locator(".image-canvas img")).toBeVisible();
  expect(
    await page.evaluate(() => (window as any).__nativeTest.sessions.size),
  ).toBe(0);
});

test("fit and actual size work in both appearances at the minimum window size", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 800, height: 420 });
  await setup(page);
  await openImage(page);
  const image = page.locator(".image-canvas img");
  const viewport = page.locator(".image-viewport");
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme });
    const bounds = (await image.boundingBox())!;
    const area = (await viewport.boundingBox())!;
    expect(bounds.width).toBeLessThanOrEqual(area.width);
    expect(bounds.height).toBeLessThanOrEqual(area.height);
    expect(bounds.x).toBeGreaterThanOrEqual(area.x);
    expect(bounds.y).toBeGreaterThanOrEqual(area.y);
    await page.screenshot({
      path: testInfo.outputPath(`image-${colorScheme}.png`),
    });
  }
  await page.getByRole("button", { name: "100%", exact: true }).click();
  expect((await image.boundingBox())!.width).toBe(1600);
  expect(
    await viewport.evaluate(
      (element) =>
        element.scrollWidth > element.clientWidth &&
        element.scrollHeight > element.clientHeight,
    ),
  ).toBe(true);
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  expect((await image.boundingBox())!.width).toBe(2000);
  await page.getByRole("button", { name: "Zoom out", exact: true }).click();
  expect((await image.boundingBox())!.width).toBe(1600);
  await page.getByRole("button", { name: "Fit", exact: true }).click();
  expect(
    await viewport.evaluate(
      (element) =>
        element.scrollWidth <= element.clientWidth &&
        element.scrollHeight <= element.clientHeight,
    ),
  ).toBe(true);
});

test("restored image panes participate in pointer focus and close without stopping their terminal", async ({
  page,
}) => {
  const project = newProject("/project", "local:bash");
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  tab.layout = splitPane(tab.layout, tab.activePaneId, "horizontal", {
    type: "file",
    id: "image-pane",
    root: "/project",
    relative: "picture.PNG",
    title: "picture.PNG",
  });
  await page.addInitScript(() =>
    localStorage.setItem(
      "test-keybindings",
      JSON.stringify({ version: 1, focusFollowsPointer: true, bindings: {} }),
    ),
  );
  await setup(page, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await expect(page.locator(".image-canvas img")).toBeVisible();
  await page.locator(".xterm-helper-textarea").focus();
  await page.locator(".image-viewport").hover();
  await expect(page.locator(".image-viewport")).toBeFocused();
  await page.keyboard.press("Control+w");
  await expect(page.locator(".image-preview")).toHaveCount(0);
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "close_terminal",
      ),
    ),
  ).toEqual([]);
});

test("decode and read errors can be retried and late reads cannot replace another image", async ({
  page,
}) => {
  await setup(page);
  await page.getByRole("button", { name: "broken.png", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("damaged");
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.imageFiles["broken.png"] = native.imageFiles["picture.PNG"];
  });
  await page.getByRole("button", { name: "Try again", exact: true }).click();
  await expect(page.locator(".image-canvas img")).toBeVisible();
  await page.evaluate(() => {
    (window as any).__nativeTest.imageError =
      "This image exceeds the 32 MiB preview limit.";
  });
  await page
    .getByRole("button", { name: "Reload image from disk", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText("32 MiB");
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    native.imageError = "";
    native.imageDelays["picture.PNG"] = 800;
  });
  await page.getByRole("button", { name: "picture.PNG", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Opening image…" }),
  ).toBeVisible();
  await openImage(page, "favicon.ico");
  await page.waitForTimeout(1000);
  await expect(page.locator(".image-canvas img")).toHaveAttribute(
    "alt",
    "favicon.ico",
  );
  await expect(page.getByRole("alert")).toHaveCount(0);
});
