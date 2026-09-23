import { expect, test } from "@playwright/test";
import {
  newProject,
  newSession,
  openFileTab,
  updateFile,
  fileTabs,
} from "../../src/model";
import { mockDesktop } from "./desktop";

function sessionFor(relative: string) {
  const project = newProject("/project", "default");
  let session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  session = openFileTab(
    session,
    project.workspaces[0].id,
    "/project",
    relative,
  );
  const tab = fileTabs(session).find((tab) => tab.relative === relative)!;
  session = updateFile(session, tab.id, (file) => ({
    ...file,
    previewView: "preview",
    agentPreview: true,
  }));
  return { session, tab };
}

test("restored agent image panels fail closed and use only their staged derivative", async ({
  page,
}) => {
  const { session, tab } = sessionFor("pixel.png");
  await mockDesktop(page, false, session);
  await page.goto("/");
  await expect(page.getByRole("alert")).toContainText(
    "Reopen this preview through Agent control",
  );
  await page.evaluate(async (id) => {
    const path = "/src/agent-preview.ts";
    const preview = await import(path);
    const canvas = document.createElement("canvas");
    canvas.width = 2;
    canvas.height = 3;
    const context = canvas.getContext("2d")!;
    context.fillStyle = "red";
    context.fillRect(0, 0, 2, 3);
    const image = {
      kind: "image",
      dataBase64: canvas.toDataURL().split(",")[1],
      mimeType: "image/png",
      width: 2,
      height: 3,
      originalWidth: 4000,
      originalHeight: 6000,
    };
    preview.stageAgentPreview(id, { body: image, assetPermit: null }).commit();
  }, tab.id);
  await expect(page.locator(".image-canvas img")).toBeVisible();
  expect(
    await page
      .locator(".image-canvas img")
      .evaluate((image: HTMLImageElement) => [
        image.naturalWidth,
        image.naturalHeight,
      ]),
  ).toEqual([2, 3]);
  await expect(
    page.getByRole("button", { name: "Reload image from disk" }),
  ).toHaveCount(0);
  await expect(page.getByTitle("Preview pixels")).toBeVisible();
  await page.evaluate(async () => {
    const path = "/src/agent-preview.ts";
    (await import(path)).retainAgentPreviews([]);
  });
  await expect(page.getByRole("alert")).toContainText(
    "Reopen this preview through Agent control",
  );
  const reads = await page.evaluate(() =>
    (window as any).__nativeTest.calls.filter((call: any) =>
      ["read_image_file", "read_markdown_image"].includes(call.command),
    ),
  );
  expect(reads).toEqual([]);
});

test("agent Markdown uses its permit for embedded images and keeps external images as links", async ({
  page,
}, testInfo) => {
  const { session, tab } = sessionFor("README.md");
  await mockDesktop(page, false, session);
  await page.addInitScript(() => {
    const native = (window as any).__nativeTest;
    native.editorFiles["/project/README.md"] = {
      content:
        "# Scoped preview\n\n![Local](pixel.png)\n![Secret](.env.png)\n![Tracker](https://example.invalid/tracker.png)",
      revision: "preview",
      encoding: "utf8",
      readOnly: false,
    };
    const bridge = (window as any).__TAURI_INTERNALS__;
    const invoke = bridge.invoke;
    bridge.invoke = async (command: string, args: any) => {
      if (command === "agent_control_preview_asset") {
        native.calls.push({ command, args });
        if (args.permitId !== "scoped-permit" || args.relative !== "pixel.png")
          throw Error("SCOPE_DENIED");
        const canvas = document.createElement("canvas");
        canvas.width = 32;
        canvas.height = 32;
        const context = canvas.getContext("2d")!;
        context.fillStyle = "#6bb799";
        context.fillRect(0, 0, 32, 32);
        return {
          mimeType: "image/png",
          dataBase64: canvas.toDataURL().split(",")[1],
        };
      }
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Scoped preview" }),
  ).toBeVisible();
  await expect(page.locator(".markdown-image-error")).toHaveCount(2);
  await page.evaluate(async (id) => {
    const path = "/src/agent-preview.ts";
    (await import(path))
      .stageAgentPreview(id, {
        body: { kind: "text" },
        assetPermit: "scoped-permit",
      })
      .commit();
  }, tab.id);
  await expect(
    page.locator('.markdown-preview img[alt="Local"]'),
  ).toBeVisible();
  await expect(page.locator(".markdown-image-error")).toHaveText(
    "Secret (image unavailable)",
  );
  await expect(
    page.getByRole("link", { name: "Image: Tracker" }),
  ).toBeVisible();
  const reads = await page.evaluate(() =>
    (window as any).__nativeTest.calls.filter((call: any) =>
      [
        "agent_control_preview_asset",
        "read_markdown_image",
        "read_image_file",
      ].includes(call.command),
    ),
  );
  expect(reads.map((read: any) => read.command)).toEqual([
    "agent_control_preview_asset",
    "agent_control_preview_asset",
  ]);
  expect(reads.map((read: any) => read.args)).toEqual([
    { permitId: "scoped-permit", relative: "pixel.png" },
    { permitId: "scoped-permit", relative: ".env.png" },
  ]);
  await page.screenshot({
    path: testInfo.outputPath("agent-markdown-preview.png"),
  });
});
