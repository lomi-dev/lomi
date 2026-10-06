import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

const beforeHash = "a".repeat(64);
const content = "# Project\nA text file preview.\n";
async function setup(page: Page, readOnly = false) {
  await mockDesktop(page, true, undefined, undefined, {
    "/project/README.md": {
      content,
      revision: beforeHash,
      encoding: "utf8",
      readOnly,
    },
  });
  await page.goto("/");
}
async function openReadme(page: Page) {
  await page.getByRole("button", { name: "README.md", exact: true }).dblclick();
  await expect(page.locator(".cm-content")).toBeVisible();
}

test("native project approval rejects actual dirty text and releases its opening gate", async ({
  page,
}) => {
  await setup(page);
  await openReadme(page);
  await page.locator(".cm-content").focus();
  await page.keyboard.insertText("unsaved");
  const result = await page.evaluate(async (hash) => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const doc = runtime.documents()[0];
    let failure = "accepted";
    try {
      await runtime.freezeNativeProject("/project");
    } catch (error) {
      failure = (error as Error).message;
    }
    return { failure, dirty: doc.dirty, frozen: doc.nativeOperationFrozen };
  }, beforeHash);
  expect(result).toEqual({
    failure:
      "Save or close unsaved project editors before approving this native operation.",
    dirty: true,
    frozen: false,
  });
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "true",
  );
  await page
    .getByRole("button", { name: "it's a file.txt", exact: true })
    .dblclick();
  await expect(page.locator(".cm-content")).toContainText("Hello, 🦀!");
});

test("native project fence blocks bypass edits and opens until its last idempotent release", async ({
  page,
}) => {
  await setup(page);
  await openReadme(page);
  const result = await page.evaluate(async (hash) => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const doc = runtime.documents()[0];
    const first = await runtime.freezeNativeProject("/project");
    const second = await runtime.freezeNativeProject("/project");
    (window as any).__projectFence = second;
    const failures: string[] = [];
    for (const attempt of [
      () => doc.readAgentBuffer({}),
      () => doc.applyAgentEdits({}, "/project/README.md"),
      () =>
        runtime.openDocument({
          type: "file",
          id: "frozen",
          root: "/project",
          relative: "README.md",
          title: "README.md",
        }),
      () =>
        runtime.openDocument({
          type: "file",
          id: "new",
          root: "/project",
          relative: "new.txt",
          title: "new.txt",
        }),
    ]) {
      try {
        await attempt();
        failures.push("accepted");
      } catch (error) {
        failures.push((error as Error).message);
      }
    }
    // This bypasses CodeMirror transaction filters; the dispatch guard must
    // still reject the change, including callers outside the agent API.
    doc.view.dispatch({
      changes: { from: 0, insert: "bypass" },
      filter: false,
    });
    first.release();
    first.release();
    return {
      failures,
      text: doc.state.doc.toString(),
      frozen: doc.nativeOperationFrozen,
    };
  }, beforeHash);
  expect(result.failures).toEqual([
    "TARGET_BUSY",
    "TARGET_BUSY",
    "TARGET_BUSY",
    "TARGET_BUSY",
  ]);
  expect(result.text).toBe(content);
  expect(result.frozen).toBe(true);
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "false",
  );
  await page.evaluate(() => {
    (window as any).__nativeTest.editorFiles["/project/README.md"] = {
      content: "effect committed\n",
      revision: "b".repeat(64),
      encoding: "utf8",
      readOnly: false,
    };
    (window as any).__projectFence.release();
  });
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "true",
  );
  await expect(page.locator(".cm-content")).toContainText("effect committed");
});

test("native project freeze waits for an already opening target and freezes the loaded buffer", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.fileReadDelays["README.md"] = 500;
  });
  await page.getByRole("button", { name: "README.md", exact: true }).dblclick();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls.some(
          (call: any) =>
            call.command === "read_editor_file" &&
            call.args.relative === "README.md",
        ),
      ),
    )
    .toBe(true);
  const proof = await page.evaluate(async (hash) => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const fence = await runtime.freezeNativeProject("/project");
    (window as any).__projectFence = fence;
    return runtime
      .documents()
      .map((doc: any) => ({ frozen: doc.nativeOperationFrozen }));
  }, beforeHash);
  expect(proof).toEqual([{ frozen: true }]);
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "false",
  );
  await page.evaluate(() => (window as any).__projectFence.release());
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "true",
  );
});

test("native project release preserves original disk read-only permissions", async ({
  page,
}) => {
  await setup(page, true);
  await openReadme(page);
  await page.evaluate(async (hash) => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const fence = await runtime.freezeNativeProject("/project");
    fence.release();
    fence.release();
  }, beforeHash);
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "false",
  );
});

async function newDraft(page: Page) {
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New file", exact: true }).click();
  await expect(page.locator(".cm-content")).toBeVisible();
}

test("a native project fence bars untitled Save As before any native write", async ({
  page,
}) => {
  await setup(page);
  await newDraft(page);
  const result = await page.evaluate(async (hash) => {
    const runtimePath = "/src/editor-runtime.ts";
    const runtime = await import(runtimePath);
    const doc = runtime.documents()[0];
    const fence = await runtime.freezeNativeProject("/project");
    let failure = "accepted";
    try {
      await doc.save();
    } catch (error) {
      failure = (error as Error).message;
    }
    const calls = (window as any).__nativeTest.calls.filter(
      (call: any) => call.command === "save_new_editor_file",
    ).length;
    fence.release();
    return { failure, calls };
  }, beforeHash);
  expect(result).toEqual({ failure: "TARGET_BUSY", calls: 0 });
});

test("native project freeze drains an earlier Save As and freezes its newly canonical alias", async ({
  page,
}) => {
  await setup(page);
  await newDraft(page);
  await page.evaluate(
    async ({ hash, text }) => {
      const native = (window as any).__nativeTest;
      native.newFilePath = "/project/README.md";
      const internals = (window as any).__TAURI_INTERNALS__;
      const invoke = internals.invoke;
      let finish!: () => void;
      const pending = new Promise<void>((resolve) => {
        finish = resolve;
      });
      (window as any).__finishEarlierSave = finish;
      internals.invoke = async (command: string, args: unknown) => {
        if (command !== "save_new_editor_file") return invoke(command, args);
        (window as any).__earlierSaveRequested = true;
        await pending;
        const result = await invoke(command, args);
        result.file.revision = hash;
        native.editorFiles["/project/README.md"].revision = hash;
        return result;
      };
      const runtimePath = "/src/editor-runtime.ts";
      const runtime = await import(runtimePath);
      const doc = runtime.documents()[0];
      doc.dispatch({ changes: { from: 0, insert: text } });
      (window as any).__earlierSave = doc.save();
      (window as any).__projectFreezeSettled = false;
      (window as any).__projectFreeze = runtime
        .freezeNativeProject("/project")
        .then((fence: any) => {
          (window as any).__projectFreezeSettled = true;
          (window as any).__projectFence = fence;
          return fence;
        });
    },
    { hash: beforeHash, text: content },
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).__earlierSaveRequested))
    .toBe(true);
  expect(
    await page.evaluate(() => (window as any).__projectFreezeSettled),
  ).toBe(false);
  const result = await page.evaluate(async () => {
    (window as any).__finishEarlierSave();
    await (window as any).__earlierSave;
    const fence = await (window as any).__projectFreeze;
    const runtimePath = "/src/editor-runtime.ts";
    const runtime = await import(runtimePath);
    const doc = runtime.documents()[0];
    const result = {
      documents: runtime.documents().length,
      frozen: doc.nativeOperationFrozen,
      revision: doc.trashRevision().diskRevision,
    };
    fence.release();
    return result;
  });
  expect(result).toEqual({ documents: 1, frozen: true, revision: beforeHash });
});

test("global file-operation and project leases exclude each other even with no loaded editor", async ({
  page,
}) => {
  await setup(page);
  const result = await page.evaluate(async (hash) => {
    const runtimePath = "/src/editor-runtime.ts";
    const runtime = await import(runtimePath);
    const servicePath = "/src/editor-service.ts";
    const service = await import(servicePath);
    const failures: string[] = [];
    const pause = await service.pauseEditorFileOperations();
    const paused = service.editorFileOperationsPaused();
    try {
      await runtime.freezeNativeProject("/project");
    } catch (error) {
      failures.push((error as Error).message);
    }
    try {
      await service.pauseEditorFileOperations();
    } catch (error) {
      failures.push((error as Error).message);
    }
    pause();
    pause();
    const fence = await runtime.freezeNativeProject("/project");
    try {
      await service.pauseEditorFileOperations();
    } catch (error) {
      failures.push((error as Error).message);
    }
    const documents = runtime.documents().length;
    fence.release();
    fence.release();
    const resumed = await service.pauseEditorFileOperations();
    resumed();
    return {
      paused,
      documents,
      failures,
      released: !service.editorFileOperationsPaused(),
    };
  }, beforeHash);
  expect(result).toEqual({
    paused: true,
    documents: 0,
    failures: ["TARGET_BUSY", "TARGET_BUSY", "TARGET_BUSY"],
    released: true,
  });
});
