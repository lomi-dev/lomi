import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

async function setup(page: Page) {
  await mockDesktop(page, true, undefined, undefined, {
    "/project/README.md": {
      content: "Saved project document\n",
      revision: "a".repeat(64),
      encoding: "utf8",
      readOnly: false,
    },
  });
  await page.goto("/");
  await page.getByRole("button", { name: "README.md", exact: true }).dblclick();
  await expect(page.locator(".cm-content")).toBeVisible();
}

test("native project approval rejects dirty buffers and releases opening fence", async ({
  page,
}) => {
  await setup(page);
  await page.locator(".cm-content").focus();
  await page.keyboard.insertText("unsaved");
  const result = await page.evaluate(async () => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    try {
      await runtime.freezeNativeProject("/project");
      return "accepted";
    } catch (cause) {
      return (cause as Error).message;
    }
  });
  expect(result).toContain("Save or close unsaved project editors");
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "true",
  );
  await page
    .getByRole("button", { name: "it's a file.txt", exact: true })
    .dblclick();
  await expect(page.locator(".cm-content")).toContainText("Hello, 🦀!");
});

test("overlapping native project approvals freeze edits and opens through final release", async ({
  page,
}) => {
  await setup(page);
  const result = await page.evaluate(async () => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const doc = runtime.documents()[0];
    const first = await runtime.freezeNativeProject("/project");
    const second = await runtime.freezeNativeProject("/project");
    (window as any).__nativeFreeze = second;
    doc.view.dispatch({
      changes: { from: 0, insert: "bypass" },
      filter: false,
    });
    let opening = "accepted";
    try {
      await runtime.openDocument({
        type: "file",
        id: "new",
        root: "/project",
        relative: "new.txt",
        title: "new.txt",
      });
    } catch (cause) {
      opening = (cause as Error).message;
    }
    first.release();
    first.release();
    return {
      text: doc.state.doc.toString(),
      opening,
      frozen: doc.nativeOperationFrozen,
    };
  });
  expect(result).toEqual({
    text: "Saved project document\n",
    opening: "TARGET_BUSY",
    frozen: true,
  });
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "false",
  );
  await page.evaluate(() => (window as any).__nativeFreeze.release());
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "true",
  );
  await page
    .getByRole("button", { name: "it's a file.txt", exact: true })
    .dblclick();
  await expect(page.locator(".cm-content")).toContainText("Hello, 🦀!");
});
