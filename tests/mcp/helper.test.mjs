import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { resolve } from "node:path";
import { test } from "node:test";
import Ajv from "ajv/dist/2020.js";

for (const version of ["2025-11-25", "2026-07-28"]) {
  test(
    `production helper: ${version} discovery, schemas and no unpaired disclosure`,
    { timeout: 15000 },
    async (t) => {
      const child = spawn(
        resolve(
          `src-tauri/target/debug/lomi-mcp${process.platform === "win32" ? ".exe" : ""}`,
        ),
        [],
        { stdio: ["pipe", "pipe", "pipe"] },
      );
      let id = 0;
      let stderr = "";
      const pending = new Map();
      child.stderr.on("data", (bytes) => {
        stderr += bytes;
      });
      const exit = new Promise((resolve) =>
        child.once("exit", (code) => resolve(code)),
      );
      createInterface({ input: child.stdout }).on("line", (line) => {
        const result = JSON.parse(line);
        assert.equal(result.jsonrpc, "2.0");
        assert.ok(pending.has(result.id));
        pending.get(result.id)(result);
        pending.delete(result.id);
      });
      t.after(async () => {
        child.stdin.end();
        child.kill();
        await exit;
      });
      const meta =
        version === "2026-07-28"
          ? {
              _meta: {
                "io.modelcontextprotocol/protocolVersion": version,
                "io.modelcontextprotocol/clientCapabilities": {},
              },
            }
          : {};
      const call = (method, params) => {
        const requestId = ++id;
        const response = new Promise((resolve) =>
          pending.set(requestId, resolve),
        );
        child.stdin.write(
          `${JSON.stringify({ jsonrpc: "2.0", id: requestId, method, params: { ...params, ...meta } })}\n`,
        );
        return response;
      };
      if (version === "2025-11-25") {
        assert.equal(
          (
            await call("initialize", {
              protocolVersion: version,
              capabilities: {},
              clientInfo: { name: "fixture", version: "1" },
            })
          ).result.serverInfo.name,
          "lomi-mcp",
        );
        child.stdin.write(
          '{"jsonrpc":"2.0","method":"notifications/initialized"}\n',
        );
      } else {
        const result = (await call("server/discover", {})).result;
        assert.equal(result.ttlMs, 0);
        assert.equal(result.cacheScope, "private");
      }
      const catalog = (await call("tools/list", {})).result.tools;
      assert.equal(catalog.length, 61);
      assert.ok(
        JSON.stringify(catalog).length < 700_000,
        "Tool-specific output schemas must stay within the catalog budget",
      );
      const ajv = new Ajv({ strict: false, validateFormats: false });
      const closureReply = {
        controlApiVersion: "1.0",
        status: "ok",
        data: {
          kind: "operation",
          operationId: "operation",
          workspaceId: "workspace",
          state: "succeeded",
          effectState: "complete",
          result: {
            kind: "project_closure",
            workspaceId: "workspace",
            projectId: "project",
            workspaceIds: ["workspace"],
            panelIds: [],
            terminalSessionIds: [],
            closed: true,
          },
        },
      };
      const projectOutput = ajv.compile(
        catalog.find((t) => t.name === "lomi_project_close").outputSchema,
      );
      const operationOutput = ajv.compile(
        catalog.find((t) => t.name === "lomi_operation_get").outputSchema,
      );
      const workspaceOutput = ajv.compile(
        catalog.find((t) => t.name === "lomi_workspace_update").outputSchema,
      );
      assert.ok(
        projectOutput(closureReply),
        JSON.stringify(projectOutput.errors),
      );
      assert.ok(
        operationOutput(closureReply),
        JSON.stringify(operationOutput.errors),
      );
      assert.equal(workspaceOutput(closureReply), false);
      const openReply = structuredClone(closureReply);
      openReply.data.result = {
        kind: "project_opened",
        anchorWorkspaceId: "workspace",
        projectId: "new-project",
        projectPath: "/new-root",
        workspaceId: "new-workspace",
        panelId: "blank-editor",
        name: "New",
        opened: true,
      };
      const openOutput = ajv.compile(
        catalog.find((t) => t.name === "lomi_project_open").outputSchema,
      );
      assert.ok(openOutput(openReply), JSON.stringify(openOutput.errors));
      assert.ok(
        operationOutput(openReply),
        JSON.stringify(operationOutput.errors),
      );
      assert.equal(projectOutput(openReply), false);
      assert.equal(openOutput(closureReply), false);
      const settingsReply = structuredClone(closureReply);
      settingsReply.data.result = {
        kind: "settings_opened",
        workspaceId: "workspace",
        page: "editor",
        requested: true,
      };
      const settingsOutput = ajv.compile(
        catalog.find((t) => t.name === "lomi_settings_open").outputSchema,
      );
      assert.ok(
        settingsOutput(settingsReply),
        JSON.stringify(settingsOutput.errors),
      );
      assert.ok(operationOutput(settingsReply));
      assert.equal(settingsOutput(openReply), false);
      assert.equal(openOutput(settingsReply), false);
      const settingsInput = ajv.compile(
        catalog.find((t) => t.name === "lomi_settings_open").inputSchema,
      );
      const settingsArgs = {
        workspaceId: "workspace",
        page: "editor",
        expectedRevision: "1",
        retryEpoch: "epoch",
        requestKey: "settings",
      };
      assert.ok(settingsInput(settingsArgs));
      assert.equal(
        settingsInput({ ...settingsArgs, page: "credentials" }),
        false,
      );
      assert.equal(settingsInput({ ...settingsArgs, approve: true }), false);
      const updateTool = catalog.find((t) => t.name === "lomi_settings_update");
      assert.equal(updateTool.annotations.readOnlyHint, false);
      const updateInput = ajv.compile(updateTool.inputSchema);
      const updateArgs = {
        workspaceId: "workspace",
        patch: { type: "editor_tab_size", value: 8 },
        expectedRevision: "1",
        expectedSettingsRevision: "a".repeat(64),
        retryEpoch: "epoch",
        requestKey: "update",
      };
      assert.equal(
        updateInput({
          ...updateArgs,
          patch: { type: "terminal_field", field: "appearance.fontSize" },
        }),
        false,
      );
      assert.ok(updateInput(updateArgs), JSON.stringify(updateInput.errors));
      for (const patch of [
        { type: "keybinding_set", action: "saveFile", shortcut: null },
        {
          type: "keybinding_set",
          action: "saveFile",
          shortcut: "Ctrl+Alt+F20",
        },
        { type: "keybinding_reset", action: "saveFile" },
        { type: "keybinds_focus_follows_pointer", value: true },
        { type: "theme_builtin", value: "deepmono" },
        { type: "theme_builtin", value: "lomi" },
        { type: "theme_appearance", value: "system" },
      ])
        assert.ok(
          updateInput({ ...updateArgs, patch }),
          JSON.stringify(updateInput.errors),
        );
      for (const patch of [
        { type: "keybinding_set", action: "saveFile" },
        { type: "keybinding_reset", action: "saveFile", shortcut: null },
        { type: "keybindings_replace", value: {} },
        { type: "theme_builtin", value: "user-package" },
        { type: "theme_appearance", value: "custom" },
      ])
        assert.equal(updateInput({ ...updateArgs, patch }), false);
      assert.ok(
        updateInput({
          ...updateArgs,
          patch: {
            type: "terminal_field",
            field: "appearance.fontSize",
            value: 18,
          },
        }),
      );
      assert.ok(
        updateInput({
          ...updateArgs,
          patch: {
            type: "terminal_field",
            field: "appearance.colors.red",
            value: null,
          },
        }),
      );
      assert.equal(
        updateInput({
          ...updateArgs,
          patch: {
            type: "terminal_field",
            field: "shell.command",
            value: "unsafe",
          },
        }),
        false,
      );
      assert.equal(
        updateInput({
          ...updateArgs,
          patch: {
            type: "terminal_field",
            field: "behavior.scrollback",
            value: { nested: true },
          },
        }),
        false,
      );
      assert.equal(updateInput({ ...updateArgs, approve: true }), false);
      assert.equal(
        updateInput({
          ...updateArgs,
          patch: { type: "editor_tab_size", value: 8, other: true },
        }),
        false,
      );
      assert.equal(
        updateInput({
          ...updateArgs,
          patch: { type: "credentials", value: "private" },
        }),
        false,
      );
      const updateOutput = ajv.compile(updateTool.outputSchema);
      const updatedReply = structuredClone(settingsReply);
      updatedReply.data.result = {
        kind: "settings_updated",
        workspaceId: "workspace",
        section: "editor",
        previousStoredRevision: null,
        storedRevision: "b".repeat(64),
        applied: true,
      };
      assert.ok(
        updateOutput(updatedReply),
        JSON.stringify(updateOutput.errors),
      );
      assert.ok(operationOutput(updatedReply));
      assert.equal(updateOutput(settingsReply), false);
      const settingsReadOutput = ajv.compile(
        catalog.find((t) => t.name === "lomi_settings_read").outputSchema,
      );
      const settingsReadReply = {
        controlApiVersion: "1.0",
        status: "ok",
        data: {
          kind: "settings_snapshot",
          workspaceId: "workspace",
          section: "editor",
          revision: "a".repeat(64),
          readiness: "ready",
          values: { section: "editor", tabSize: 4, insertSpaces: true },
        },
      };
      assert.ok(
        settingsReadOutput(settingsReadReply),
        JSON.stringify(settingsReadOutput.errors),
      );
      assert.equal(settingsReadOutput(settingsReply), false);
      assert.equal(settingsOutput(settingsReadReply), false);
      const settingsReadInput = ajv.compile(
        catalog.find((t) => t.name === "lomi_settings_read").inputSchema,
      );
      assert.ok(
        settingsReadInput({ workspaceId: "workspace", section: "terminal" }),
      );
      assert.equal(
        settingsReadInput({ workspaceId: "workspace", section: "credentials" }),
        false,
      );
      assert.equal(
        settingsReadInput({
          workspaceId: "workspace",
          section: "editor",
          path: "/private",
        }),
        false,
      );
      const workspaceReply = structuredClone(closureReply);
      workspaceReply.data.result = {
        kind: "workspace",
        workspaceId: "workspace",
        name: "Renamed",
      };
      assert.ok(workspaceOutput(workspaceReply));
      assert.ok(operationOutput(workspaceReply));
      assert.equal(projectOutput(workspaceReply), false);
      const updateSchema = ajv.compile(
        catalog.find((t) => t.name === "lomi_workspace_update").inputSchema,
      );
      const selection = {
        action: "select",
        workspaceId: "w",
        expectedRevision: "1",
        retryEpoch: "epoch",
        requestKey: "select",
      };
      assert.ok(updateSchema(selection));
      assert.equal(
        updateSchema({ ...selection, name: "Ambiguous rename" }),
        false,
      );
      assert.equal(updateSchema({ ...selection, action: "execute" }), false);
      for (const tool of catalog) {
        assert.equal(tool.inputSchema.type, "object");
        const validate = ajv.compile(tool.outputSchema);
        if (tool.name === "lomi_android_logcat") {
          assert.equal(
            validate({
              status: "ok",
              controlApiVersion: "1",
              data: {
                kind: "diagnostics",
                connection: "connected",
                uiReady: true,
                nextStep: "",
              },
            }),
            false,
          );
          assert.ok(
            validate({
              status: "ok",
              controlApiVersion: "1",
              data: {
                kind: "android_logcat",
                workspaceId: "w",
                deviceId: "d",
                generation: "g",
                packageName: "org.example.app",
                processId: 123,
                lines: ["message"],
                nextCursor: null,
                truncated: false,
                complete: false,
                scope: "main_process_main_buffer_recent_snapshot",
                gap: "unknown",
              },
            }),
            JSON.stringify(validate.errors),
          );
        }

        const target = {
          workspaceId: "foreign",
          panelId: "missing",
          terminalSessionId: "missing",
        };
        const retry = { retryEpoch: "missing", requestKey: "test" };
        const workspaceMutation = {
          workspaceId: "foreign",
          name: "Renamed",
          expectedRevision: "1",
          ...retry,
        };
        const panelMutation = { ...target, expectedRevision: "1", ...retry };
        const args =
          {
            lomi_connect: { workspaceId: "foreign" },
            lomi_git_diff: {
              workspaceId: "workspace",
              relativePath: "file.txt",
              comparison: "worktree",
            },
            lomi_git_history: { workspaceId: "workspace" },
            lomi_git_commit: {
              workspaceId: "workspace",
              commit: "a".repeat(40),
            },
            lomi_git_remotes: { workspaceId: "workspace" },
            lomi_git_mutate: {
              workspaceId: "workspace",
              operation: "stage",
              paths: ["a.txt"],
              expectedRevision: "1",
              retryEpoch: "epoch",
              requestKey: "request",
            },
            lomi_git_open: {
              workspaceId: "workspace",
              view: { type: "diff", relativePath: "a.txt", staged: false },
              expectedRevision: "1",
              retryEpoch: "epoch",
              requestKey: "request",
            },
            lomi_git_status: { workspaceId: "workspace", limit: 1 },
            lomi_files_mutate: {
              workspaceId: "foreign",
              operation: {
                type: "create",
                relativePath: "new.txt",
                kind: "file",
                expectedParentRevision: "a".repeat(64),
              },
              expectedRevision: "1",
              ...retry,
            },
            lomi_editor_save: {
              workspaceId: "foreign",
              panelId: "missing",
              relativePath: "file.txt",
              documentId: "doc",
              expectedBufferRevision: "doc:0",
              expectedDiskRevision: "a".repeat(64),
              expectedRevision: "1",
              ...retry,
            },
            lomi_editor_open: {
              workspaceId: "foreign",
              relativePath: "file.txt",
              expectedRevision: "1",
              ...retry,
            },
            lomi_editor_apply_edits: {
              workspaceId: "foreign",
              panelId: "missing",
              expectedRevision: "1",
              ...retry,
              relativePath: "file.txt",
              documentId: "doc",
              expectedBufferRevision: "doc:0",
              expectedDiskRevision: "a".repeat(64),
              edits: [{ fromUtf16: 0, toUtf16: 0, insert: "test" }],
            },
            lomi_editor_read: {
              workspaceId: "foreign",
              panelId: "foreign",
              relativePath: "file.txt",
            },
            lomi_files_search: {
              workspaceId: "foreign",
              query: { text: "test" },
            },
            lomi_files_list: { workspaceId: "foreign" },
            lomi_files_read: {
              workspaceId: "foreign",
              relativePath: "example.txt",
            },
            lomi_android_screenshot: {
              workspaceId: "foreign",
              panelId: "foreign",
              deviceId: "00000000-0000-0000-0000-000000000000",
              generation: "00000000-0000-0000-0000-000000000000",
            },
            lomi_android_snapshot: {
              workspaceId: "foreign",
              panelId: "foreign",
              deviceId: "00000000-0000-0000-0000-000000000000",
              generation: "00000000-0000-0000-0000-000000000000",
            },
            lomi_android_input: {
              workspaceId: "foreign",
              panelId: "foreign",
              deviceId: "00000000-0000-0000-0000-000000000000",
              generation: "00000000-0000-0000-0000-000000000000",
              leaseId: "00000000-0000-0000-0000-000000000000",
              inputSequence: "1",
              event: { type: "text", text: "fixture" },
              retryEpoch: "foreign",
            },
            lomi_android_start: {
              workspaceId: "foreign",
              panelId: "foreign",
              deviceId: "00000000-0000-0000-0000-000000000000",
              expectedRevision: "1",
              retryEpoch: "foreign",
              requestKey: "android-start",
            },
            lomi_android_stop: {
              workspaceId: "foreign",
              panelId: "foreign",
              deviceId: "00000000-0000-0000-0000-000000000000",
              generation: "00000000-0000-0000-0000-000000000000",
              expectedRevision: "1",
              retryEpoch: "foreign",
              requestKey: "android-stop",
            },
            lomi_android_open: {
              workspaceId: "foreign",
              deviceId: "00000000-0000-0000-0000-000000000000",
              expectedRevision: "1",
              retryEpoch: "foreign",
              requestKey: "android-open",
            },
            lomi_android_list: { workspaceId: "foreign" },
            lomi_workspace_create: workspaceMutation,
            lomi_workspace_update: workspaceMutation,
            lomi_settings_read: { workspaceId: "workspace", section: "editor" },
            lomi_settings_update: {
              workspaceId: "workspace",
              patch: { type: "editor_tab_size", value: 8 },
              expectedSettingsRevision: "a".repeat(64),
              expectedRevision: "1",
              retryEpoch: "epoch",
              requestKey: "update-settings",
            },
            lomi_settings_open: {
              workspaceId: "workspace",
              page: "editor",
              expectedRevision: "1",
              retryEpoch: "epoch",
              requestKey: "settings-open",
            },
            lomi_project_open: {
              workspaceId: "workspace",
              projectPath: "/new-project",
              name: "New",
              expectedRevision: "1",
              retryEpoch: "epoch",
              requestKey: "project-open",
            },
            lomi_project_close: {
              projectId: "project",
              workspaceId: "workspace",
              expectedRevision: "1",
              retryEpoch: "epoch",
              requestKey: "project-close",
            },
            lomi_panel_list: { workspaceId: "foreign" },
            lomi_events_read: { workspaceId: "foreign" },
            lomi_panel_move: {
              workspaceId: "workspace",
              movement: {
                type: "transfer_tab",
                tabId: "tab",
                targetWorkspaceId: "target",
                beforeTabId: null,
              },
              expectedRevision: "1",
              ...retry,
            },
            lomi_panel_focus: panelMutation,
            lomi_panel_control: { ...target, ...retry, action: "claim" },
            lomi_panel_close: panelMutation,
            lomi_browser_key: {
              workspaceId: "foreign",
              panelId: "missing",
              browserGeneration: "missing",
              navigationId: "missing",
              snapshotId: "missing",
              elementRef: "e1",
              leaseId: "missing",
              key: "Enter",
              ...retry,
            },
            lomi_browser_scroll: {
              workspaceId: "foreign",
              panelId: "missing",
              browserGeneration: "missing",
              navigationId: "missing",
              snapshotId: "missing",
              leaseId: "missing",
              deltaX: 0,
              deltaY: 100,
              ...retry,
            },
            lomi_browser_click: {
              workspaceId: "foreign",
              panelId: "missing",
              browserGeneration: "missing",
              navigationId: "missing",
              snapshotId: "missing",
              elementRef: "e1",
              leaseId: "missing",
              ...retry,
            },
            lomi_browser_fill: {
              workspaceId: "foreign",
              panelId: "missing",
              browserGeneration: "missing",
              navigationId: "missing",
              snapshotId: "missing",
              elementRef: "e1",
              leaseId: "missing",
              text: "Unicode 🙂",
              ...retry,
            },
            lomi_browser_screenshot: {
              workspaceId: "foreign",
              panelId: "missing",
              browserGeneration: "missing",
              navigationId: "missing",
            },
            lomi_android_launch: {
              workspaceId: "foreign",
              panelId: "foreign",
              deviceId: "00000000-0000-4000-8000-000000000001",
              generation: "00000000-0000-4000-8000-000000000002",
              packageName: "org.example.app",
              activity: null,
              expectedRevision: "1",
              ...retry,
            },
            lomi_android_logcat: {
              workspaceId: "foreign",
              panelId: "foreign",
              deviceId: "00000000-0000-4000-8000-000000000001",
              generation: "00000000-0000-4000-8000-000000000002",
              packageName: "org.example.app",
              minPriority: "I",
              limit: 16,
            },
            lomi_android_install_apk: {
              workspaceId: "foreign",
              panelId: "foreign",
              deviceId: "00000000-0000-4000-8000-000000000001",
              generation: "00000000-0000-4000-8000-000000000002",
              artifactId: "artifact",
              sha256: "a".repeat(64),
              retryEpoch: "epoch",
              requestKey: "install",
            },
            lomi_artifact_import: {
              workspaceId: "foreign",
              relativePath: "build/app.apk",
              kind: "android_apk",
              expectedByteLength: 64,
              expectedSha256: "a".repeat(64),
              expectedRevision: "1",
              retryEpoch: "epoch",
              requestKey: "import",
            },
            lomi_artifact_read: {
              workspaceId: "foreign",
              artifactId: "missing",
            },
            lomi_browser_wait: {
              workspaceId: "foreign",
              panelId: "missing",
              browserGeneration: "missing",
              condition: { type: "text", text: "expected" },
            },
            lomi_browser_logs: {
              workspaceId: "foreign",
              panelId: "missing",
              browserGeneration: "missing",
            },
            lomi_browser_snapshot: {
              workspaceId: "foreign",
              panelId: "missing",
              browserGeneration: "missing",
            },
            lomi_browser_navigate: {
              workspaceId: "foreign",
              panelId: "missing",
              browserGeneration: "missing",
              leaseId: "missing",
              url: "http://localhost:3000",
              ...retry,
            },
            lomi_browser_open: {
              workspaceId: "foreign",
              url: "http://localhost:3000",
              expectedRevision: "1",
              ...retry,
            },
            lomi_terminal_create: {
              workspaceId: "foreign",
              cwdRelative: ".",
              title: "Agent",
              expectedRevision: "1",
              ...retry,
            },
            lomi_terminal_run: {
              ...target,
              leaseId: "missing",
              command: "true",
              ...retry,
            },
            lomi_terminal_read: target,
            lomi_terminal_input: {
              ...target,
              leaseId: "missing",
              inputSequence: "1",
              input: { type: "text", text: "test" },
            },
            lomi_terminal_interrupt: {
              ...target,
              leaseId: "missing",
              operationId: "missing",
              ...retry,
            },
            lomi_operation_get: { operationId: "foreign" },
            lomi_operation_cancel: { operationId: "foreign" },
          }[tool.name] ?? {};
        const response = await call("tools/call", {
          name: tool.name,
          arguments: args,
        });
        assert.ok(response.result, JSON.stringify(response));
        assert.ok(
          validate(response.result.structuredContent),
          JSON.stringify(validate.errors),
        );
        const result = response.result.structuredContent;
        if (["lomi_status", "lomi_diagnostics"].includes(tool.name)) {
          assert.equal(result.status, "ok");
          assert.equal(result.data.uiReady, false);
          if (tool.name === "lomi_status") {
            assert.equal(result.data.instanceId, null);
            assert.deepEqual(result.data.capabilities, []);
          }
        } else {
          assert.equal(response.result.isError, true);
          assert.ok(
            ["APP_UNAVAILABLE", "HOST_UNQUALIFIED"].includes(result.code),
          );
        }
        assert.ok(
          !JSON.stringify(result).includes(process.cwd()),
          "No private project path before pairing",
        );
        const invalid = await call("tools/call", {
          name: tool.name,
          arguments: { ...args, approved: true },
        });
        assert.equal(invalid.error.code, -32602);
        if (["lomi_workspace_update", "lomi_project_close"].includes(tool.name))
          assert.equal(tool.annotations.destructiveHint, true);
        assert.equal(
          tool.annotations.readOnlyHint,
          ![
            "lomi_git_mutate",
            "lomi_git_open",
            "lomi_files_mutate",
            "lomi_editor_save",
            "lomi_editor_open",
            "lomi_editor_apply_edits",
            "lomi_android_launch",
            "lomi_android_install_apk",
            "lomi_artifact_import",
            "lomi_android_input",
            "lomi_android_start",
            "lomi_android_stop",
            "lomi_android_open",
            "lomi_browser_click",
            "lomi_browser_fill",
            "lomi_browser_key",
            "lomi_browser_scroll",
            "lomi_browser_navigate",
            "lomi_browser_open",
            "lomi_workspace_update",
            "lomi_settings_update",
            "lomi_settings_open",
            "lomi_project_open",
            "lomi_project_close",
            "lomi_workspace_create",
            "lomi_operation_cancel",
            "lomi_panel_move",
            "lomi_panel_focus",
            "lomi_panel_control",
            "lomi_panel_close",
            "lomi_terminal_create",
            "lomi_terminal_run",
            "lomi_terminal_input",
            "lomi_terminal_interrupt",
          ].includes(tool.name),
        );
      }
      const tooLarge = await call("tools/call", {
        name: "lomi_status",
        arguments: { value: "x".repeat(65536) },
      });
      assert.equal(tooLarge.error.code, -32602);
      child.stdin.end();
      assert.equal(await exit, 0);
      assert.equal(stderr, "");
    },
  );
}
