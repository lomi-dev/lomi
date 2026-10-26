import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import {
  addWorkspace,
  newProject,
  newSession,
  openFileTab,
} from "../../src/model";
import { mockDesktop } from "./desktop";

async function waitForWorkbench(page: Page) {
  const ready = page
    .getByRole("button", { name: /^New tab/ })
    .or(page.getByRole("button", { name: "Open folder or repository" }));
  await expect(ready).toBeVisible({ timeout: 15_000 });
}

test("cold-start OS file requests open after session restoration", async ({
  page,
}) => {
  await page.addInitScript(
    (paths) => {
      localStorage.setItem("test-external-files", JSON.stringify(paths));
    },
    ["/project/cold-start.txt"],
  );
  await mockDesktop(page);
  await page.goto("/");

  await expect(
    page.getByRole("tab", { name: "cold-start.txt", exact: true }),
  ).toBeVisible({ timeout: 15_000 });
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.externalFileRequests.length,
      ),
    )
    .toBe(0);
});

test("one native event opens multiple requested files", async ({ page }) => {
  await mockDesktop(page);
  await page.goto("/");
  await waitForWorkbench(page);

  await page.evaluate(
    (paths) => {
      (window as any).__nativeTest.openExternalFiles(paths);
    },
    ["/project/first.txt", "/project/second.txt"],
  );

  await expect(
    page.getByRole("tab", { name: "first.txt", exact: true }),
  ).toBeVisible();
  const second = page.getByRole("tab", { name: "second.txt", exact: true });
  await expect(second).toBeVisible();
  await expect(second).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".cm-content")).toBeVisible();
});

test("a lost ack response retries the queue without replaying an opened file", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await waitForWorkbench(page);
  await page.evaluate(async () => {
    const desktop = window as any;
    const native = desktop.__TAURI_INTERNALS__;
    const invoke = native.invoke.bind(native);
    let lostAckResponse = false;
    native.invoke = async (command: string, args: any) => {
      const result = await invoke(command, args);
      if (command === "external_files_ack" && !lostAckResponse) {
        lostAckResponse = true;
        throw new Error("Simulated lost acknowledgement response");
      }
      return result;
    };
    await desktop.__nativeTest.openExternalFiles(["/project/retried.txt"]);
  });

  const tab = page.getByRole("tab", { name: "retried.txt", exact: true });
  await expect(tab).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "external_files_ack",
          ).length,
      ),
    )
    .toBe(1);
  const pendingQueries = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "external_files_pending",
      ).length,
  );
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "external_files_pending",
          ).length,
      ),
    )
    .toBeGreaterThan(pendingQueries);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "resolve_external_file",
          ).length,
      ),
    )
    .toBe(1);

  await page.evaluate(() => {
    (window as any).__nativeTest.openExternalFiles([
      "/project/after-lost-ack.txt",
    ]);
  });
  await expect(
    page.getByRole("tab", { name: "after-lost-ack.txt", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.externalFileRequests.length,
      ),
    )
    .toBe(0);
});

test("a new event during resolution is serialized and opens both files", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await waitForWorkbench(page);
  await page.evaluate(async () => {
    const desktop = window as any;
    const mock = desktop.__nativeTest;
    const native = desktop.__TAURI_INTERNALS__;
    const invoke = native.invoke.bind(native);
    mock.externalFileResolveDelay = 1200;
    mock.activeExternalFileResolutions = 0;
    mock.maxExternalFileResolutions = 0;
    native.invoke = async (command: string, args: any) => {
      if (command !== "resolve_external_file") return invoke(command, args);
      mock.activeExternalFileResolutions += 1;
      mock.maxExternalFileResolutions = Math.max(
        mock.maxExternalFileResolutions,
        mock.activeExternalFileResolutions,
      );
      try {
        return await invoke(command, args);
      } finally {
        mock.activeExternalFileResolutions -= 1;
      }
    };
    await mock.openExternalFiles(["/project/first-during-resolve.txt"]);
  });

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "resolve_external_file",
          ).length,
      ),
    )
    .toBe(1);
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.externalFileResolveDelay = 0;
    await mock.openExternalFiles(["/project/second-during-resolve.txt"]);
  });

  const first = page.getByRole("tab", {
    name: "first-during-resolve.txt",
    exact: true,
  });
  const second = page.getByRole("tab", {
    name: "second-during-resolve.txt",
    exact: true,
  });
  await expect(first).toBeVisible();
  await expect(second).toBeVisible();
  await expect(second).toHaveAttribute("aria-selected", "true");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.maxExternalFileResolutions,
      ),
    )
    .toBe(1);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.externalFileRequests.length,
      ),
    )
    .toBe(0);
});

test("reopening a dirty file keeps its editor buffer and tab", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await waitForWorkbench(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.openExternalFiles(["/project/draft.txt"]);
  });

  const tab = page.getByRole("tab", { name: "draft.txt", exact: true });
  const editor = page.locator(".cm-content");
  await expect(tab).toBeVisible();
  await expect(editor).toBeVisible();
  await editor.focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText("Unsaved external-open draft");

  await page.evaluate(() => {
    (window as any).__nativeTest.openExternalFiles(["/project/draft.txt"]);
  });

  await expect(page.getByRole("tab", { name: /^draft\.txt/ })).toHaveCount(1);
  await expect(editor).toHaveText("Unsaved external-open draft");
});

test("failed external opens notify the user and acknowledge the request", async ({
  page,
}) => {
  await mockDesktop(page);
  await page.goto("/");
  await waitForWorkbench(page);
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.externalFileError = "The requested file is unavailable.";
    await mock.openExternalFiles(["/project/missing.txt"]);
  });

  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.externalFileRequests.length,
      ),
    )
    .toBe(0);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) =>
              call.command === "record_app_notification" &&
              call.args.body === "The requested file is unavailable.",
          ).length,
      ),
    )
    .toBe(1);

  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.externalFileError = "";
    await mock.openExternalFiles(["/project/recovered.txt"]);
  });
  await expect(
    page.getByRole("tab", { name: "recovered.txt", exact: true }),
  ).toBeVisible();
});

test("an empty saved session creates only a file tab for an external open", async ({
  page,
}) => {
  await mockDesktop(page, true, newSession());
  await page.goto("/");
  await page.evaluate(() => {
    (window as any).__nativeTest.openExternalFiles(["/outside/plain-file"]);
  });

  await expect(
    page.getByRole("tab", { name: "plain-file", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const session = JSON.parse(
          localStorage.getItem("test-session") ?? "null",
        );
        return session?.projects[0]?.workspaces[0]?.tabs.map(
          (tab: any) => tab.type,
        );
      }),
    )
    .toEqual(["file"]);
});

test("an existing file in another workspace is activated without duplication", async ({
  page,
}) => {
  const project = newProject("/project", "local:bash");
  const first = project.workspaces[0];
  let saved = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  saved = addWorkspace(saved, "/project", "local:bash", "Second");
  const second = saved.projects[0].workspaces.find(
    (workspace) => workspace.id !== first.id,
  )!;
  saved = openFileTab(saved, second.id, "/project", "src/main.ts");
  saved = {
    ...saved,
    projects: saved.projects.map((candidate) =>
      candidate.id === project.id
        ? { ...candidate, activeWorkspaceId: first.id }
        : candidate,
    ),
  };
  await mockDesktop(page, true, saved);
  await page.goto("/");
  await waitForWorkbench(page);

  await page.evaluate(() => {
    (window as any).__nativeTest.openExternalFiles(["/project/src/main.ts"]);
  });

  await expect(
    page.getByRole("tab", { name: "main.ts", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const session = JSON.parse(
          localStorage.getItem("test-session") ?? "null",
        );
        return session?.projects[0]?.activeWorkspaceId;
      }),
    )
    .toBe(second.id);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const session = JSON.parse(
          localStorage.getItem("test-session") ?? "null",
        );
        return session?.projects[0]?.workspaces.flatMap((workspace: any) =>
          workspace.tabs.filter((tab: any) => tab.type === "file"),
        ).length;
      }),
    )
    .toBe(1);
});

test("Windows external paths reuse a backslash tab under a nested Git root", async ({
  page,
}) => {
  const project = newProject("C:\\Project", "local:pwsh");
  let saved = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  saved = openFileTab(
    saved,
    project.workspaces[0].id,
    "C:\\Project\\packages\\service",
    "src\\main.ts",
  );
  await mockDesktop(page, true, saved, undefined, {}, "windows");
  await page.goto("/");

  await page.evaluate(() => {
    (window as any).__nativeTest.openExternalFiles([
      "C:/Project/packages/service/src/main.ts",
    ]);
  });

  await expect(
    page.getByRole("tab", { name: "main.ts", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const session = JSON.parse(
          localStorage.getItem("test-session") ?? "null",
        );
        const tabs = session?.projects[0]?.workspaces[0]?.tabs ?? [];
        return {
          fileCount: tabs.filter((tab: any) => tab.type === "file").length,
          root: tabs.find((tab: any) => tab.type === "file")?.root,
        };
      }),
    )
    .toEqual({ fileCount: 1, root: "C:\\Project\\packages\\service" });
});
