import type { Locator, Page } from "@playwright/test";
import { newProject, newSession } from "../../src/model";
import type {
  GitCommitDetails,
  GitCommitDiff,
  GitCommitSummary,
} from "../../src/api";

export interface MockGitHistory {
  commits: GitCommitSummary[];
  details: Record<string, GitCommitDetails>;
  diffs: Record<string, GitCommitDiff>;
}

const project = newProject("/project", "local:bash");
const initialSession = {
  ...newSession(),
  projects: [project],
  activeProjectId: project.id,
};

export async function mockDesktop(
  page: Page,
  repository = true,
  saved: unknown = initialSession,
  gitHistory?: MockGitHistory,
  editorFiles: Record<
    string,
    { content: string; revision: string; encoding: string; readOnly: boolean }
  > = {},
  platform: "linux" | "macos" | "windows" = "linux",
) {
  await page.addInitScript(
    ({ repository, saved, gitHistory, editorFiles, platform }) => {
      Object.defineProperty(navigator, "platform", {
        configurable: true,
        value:
          platform === "macos"
            ? "MacIntel"
            : platform === "windows"
              ? "Win32"
              : "Linux x86_64",
      });
      const callbacks = new Map<number, (value: unknown) => void>();
      let callbackId = 0;
      let repositoryPresent = repository;
      let changes = [
        { path: "README.md", originalPath: null, index: " ", worktree: "M" },
      ];
      const calls: { command: string; args: Record<string, any> }[] = [];
      const browsers = new Map<string, any>();
      const sessions = new Map<string, { output: number; index: number }>();
      const events = new Map<number, { event: string; handler: number }>();
      const emitEvent = async (event: string, payload: unknown = null) => {
        for (const [id, listener] of events) {
          if (listener.event === event)
            await callbacks.get(listener.handler)?.({
              event,
              id,
              payload,
            });
        }
      };
      const emit = (id: string, text: string) => {
        const session = sessions.get(id)!;
        callbacks.get(session.output)?.({
          index: session.index++,
          message: new TextEncoder().encode(text).buffer,
        });
      };
      const desktop = window as any;
      desktop.isTauri = true;
      Object.defineProperty(window.Notification, "permission", {
        configurable: true,
        get: () => "default",
      });
      window.Notification.requestPermission = () =>
        desktop.__TAURI_INTERNALS__.invoke(
          "plugin:notification|request_permission",
        );
      desktop.__nativeTest = {
        update: null,
        updateInstruction: platform === "linux" ? "yay -Syu lomi-bin" : null,
        updateCheckError: "",
        updateDownloadError: "",
        updateInstallError: "",
        updateRestartError: "",
        aboutInfo: {
          platform,
          arch: platform === "macos" ? "aarch64" : "x86_64",
          version: "0.5.0",
          identifier: "dev.lomi.desktop",
        },
        androidPreparation: null,
        androidExitError: "",
        androidExitDelay: 0,
        updateCheckDelay: 0,
        updateDownloadDelay: 0,
        updateContentLength: 100,
        localWebServers: [] as string[],
        localWebServersError: "",
        localWebServersDelay: 0,
        localWebServersProgress: null as string[] | null,
        localWebServersCompleted: 0,
        editorFiles: JSON.parse(
          localStorage.getItem("test-editor-files") ??
            JSON.stringify(editorFiles),
        ),
        fileReadError: "",
        failFileSave: false,
        fileSaveDelay: 0,
        failZoom: false,
        zoomDelay: 0,
        zoom: 1,
        fullscreen: false,
        newFilePath: null as string | null,
        fileReadDelays: {} as Record<string, number>,
        emitEvent,
        calls,
        sessions,
        browsers,
        terminalContexts: {},
        terminalOutputDelay: 10,
        busyTerminals: [] as string[],
        terminalProcessError: "",
        terminalProcessDelay: 0,
        cliTitleSetup: null,
        cliTitleError: "",
        cliTitleSaveDelay: 0,
        cliIntegrationStatuses: {} as Record<string, any>,
        dismissedCliIntegrations: new Map<string, boolean>(),
        cliIntegrationError: "",
        cliIntegrationSaveDelay: 0,
        mcpClients: [] as any[],
        mcpInstallErrors: {} as Record<string, string>,
        agentNotificationSetup: {
          path: "/home/test/.claude/settings.json",
          revision: "initial",
          configured: false,
        },
        agentNotificationSetupError: "",
        agentNotificationPermission: true,
        agentNotificationPermissionDelay: 0,
        agentControlState: {
          supported: false,
          helperPath: null,
          broker: null,
        },
        agentControlStateReads: 0,
        agentControlStartup: (() => {
          const saved = JSON.parse(
            localStorage.getItem("test-agent-control-startup") ??
              JSON.stringify({
                supported: false,
                autoStart: false,
                yoloMode: false,
                error: null,
              }),
          );
          return { yoloMode: false, ...saved };
        })(),
        agentControlStartupCalls: [] as { enabled: boolean }[],
        failAgentControlStartupSave: false,
        agentControlStartupSaveError: "Disk is full",
        agentControlStartupSaveDelay: 0,
        failAgentControlStartupSettingSave: false,
        agentControlStartupSettingSaveError: "Settings are unavailable",
        agentControlStartupSettingSaveDelay: 0,
        failAgentControlYoloModeSave: false,
        agentControlYoloModeSaveError: "YOLO mode settings are unavailable",
        agentControlYoloModeSaveDelay: 0,
        agentControlStartupStartError: "",
        windowFocused: false,
        agentNotifications: [] as unknown[],
        emit,
        failSave: false,
        gitHistory,
        failHistory: false,
        failCommitDetails: false,
        failCommitDiff: false,
        diffDelays: {} as Record<string, number>,
        resolvedDiffs: [] as string[],
        folder: "/chosen folder",
        directoryError: "",
        failKeybindingsSave: false,
        failEditorPreferencesSave: false,
        failTerminalPreferencesSave: false,
        failThemeSave: false,
        themeLoadError: "",
        themeImportError: "",
        setRepository: (value: boolean) => {
          repositoryPresent = value;
        },
      };
      window.addEventListener("storage", (event) => {
        if (event.key === "test-plugins") void emitEvent("plugins-changed");
        if (
          event.key === "test-plugin-request" &&
          !location.search.includes("settings")
        )
          void emitEvent("plugin-close-request", JSON.parse(event.newValue!));
        if (event.key === "test-plugin-finished")
          void emitEvent(
            "plugin-operation-finished",
            JSON.parse(event.newValue!),
          );
        if (
          event.key === "test-update-check" &&
          !location.search.includes("settings")
        )
          void emitEvent("check-for-updates");
        if (event.key === "test-terminal-preferences")
          void emitEvent("terminal-preferences-changed");
        if (
          event.key === "test-agent-notification-setup" &&
          !location.search.includes("settings")
        )
          void emitEvent("agent-notification-setup");
        if (event.key === "test-editor-preferences")
          void emitEvent("editor-preferences-changed");
        if (event.key === "test-keybindings")
          void emitEvent("keybindings-changed");
        if (
          event.key === "test-theme-settings" ||
          event.key === "test-theme-refresh"
        )
          void emitEvent("theme-changed");
      });
      desktop.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      desktop.__TAURI_INTERNALS__ = {
        convertFileSrc(path: string, protocol: string) {
          return `${location.origin}/${protocol}-assets/${path}`;
        },
        metadata: {
          currentWindow: {
            label: location.search.includes("settings") ? "settings" : "main",
          },
          currentWebview: {
            label: location.search.includes("settings") ? "settings" : "main",
          },
        },
        transformCallback(callback: (value: unknown) => void) {
          const id = ++callbackId;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback(id: number) {
          callbacks.delete(id);
        },
        async invoke(command: string, args: Record<string, any> = {}) {
          calls.push({ command, args: JSON.parse(JSON.stringify(args)) });
          if (command === "agent_control_ui_register") return null;
          if (command === "agent_control_startup_state") {
            const mock = desktop.__nativeTest;
            await mock.emitEvent(
              "agent-control-startup-changed",
              mock.agentControlStartup,
            );
            return mock.agentControlStartup;
          }
          if (command === "agent_control_startup_decide") {
            const mock = desktop.__nativeTest;
            mock.agentControlStartupCalls.push(args);
            if (mock.agentControlStartupSaveDelay)
              await new Promise((resolve) =>
                setTimeout(resolve, mock.agentControlStartupSaveDelay),
              );
            if (mock.failAgentControlStartupSave)
              throw new Error(mock.agentControlStartupSaveError);
            if (mock.agentControlStartup.autoStart === null) {
              mock.agentControlStartup = {
                ...mock.agentControlStartup,
                autoStart: args.enabled,
                error:
                  args.enabled && mock.agentControlStartupStartError
                    ? mock.agentControlStartupStartError
                    : null,
              };
              localStorage.setItem(
                "test-agent-control-startup",
                JSON.stringify(mock.agentControlStartup),
              );
            }
            await mock.emitEvent(
              "agent-control-startup-changed",
              mock.agentControlStartup,
            );
            return mock.agentControlStartup;
          }
          if (command === "agent_control_set_auto_start") {
            const mock = desktop.__nativeTest;
            if (mock.agentControlStartupSettingSaveDelay)
              await new Promise((resolve) =>
                setTimeout(resolve, mock.agentControlStartupSettingSaveDelay),
              );
            if (mock.failAgentControlStartupSettingSave)
              throw new Error(mock.agentControlStartupSettingSaveError);
            mock.agentControlStartup = {
              ...mock.agentControlStartup,
              autoStart: args.enabled,
              error: null,
            };
            localStorage.setItem(
              "test-agent-control-startup",
              JSON.stringify(mock.agentControlStartup),
            );
            await mock.emitEvent(
              "agent-control-startup-changed",
              mock.agentControlStartup,
            );
            return mock.agentControlStartup;
          }
          if (command === "agent_control_set_yolo_mode") {
            const mock = desktop.__nativeTest;
            if (mock.agentControlYoloModeSaveDelay)
              await new Promise((resolve) =>
                setTimeout(resolve, mock.agentControlYoloModeSaveDelay),
              );
            if (mock.failAgentControlYoloModeSave)
              throw new Error(mock.agentControlYoloModeSaveError);
            mock.agentControlStartup = {
              ...mock.agentControlStartup,
              yoloMode: args.enabled,
            };
            localStorage.setItem(
              "test-agent-control-startup",
              JSON.stringify(mock.agentControlStartup),
            );
            await mock.emitEvent(
              "agent-control-startup-changed",
              mock.agentControlStartup,
            );
            return mock.agentControlStartup;
          }
          if (command === "agent_control_state") {
            desktop.__nativeTest.agentControlStateReads++;
            return desktop.__nativeTest.agentControlState;
          }
          if (command === "agent_control_closing") {
            desktop.__nativeTest.agentControlClosing = args.closing;
            return;
          }
          if (command.startsWith("chat_") && desktop.__chatInvoke)
            return desktop.__chatInvoke(command, args);
          if (
            (command.startsWith("android_") ||
              command === "save_android_preferences") &&
            command !== "android_exit" &&
            desktop.__androidInvoke
          )
            return desktop.__androidInvoke(command, args);
          if (command === "request_agent_notification_setup") {
            localStorage.setItem(
              "test-agent-notification-setup",
              String(Date.now()),
            );
            return;
          }
          if (command === "inspect_agent_notifications") {
            if (desktop.__nativeTest.agentNotificationSetupError)
              throw desktop.__nativeTest.agentNotificationSetupError;
            return desktop.__nativeTest.agentNotificationSetup;
          }
          if (command === "enable_agent_notifications") {
            if (desktop.__nativeTest.agentNotificationSetupError)
              throw desktop.__nativeTest.agentNotificationSetupError;
            desktop.__nativeTest.agentNotificationSetup.configured = true;
            return;
          }
          if (command === "plugin:notification|is_permission_granted") {
            await new Promise((resolve) =>
              setTimeout(
                resolve,
                desktop.__nativeTest.agentNotificationPermissionDelay,
              ),
            );
            return desktop.__nativeTest.agentNotificationPermission;
          }
          if (command === "plugin:notification|request_permission")
            return desktop.__nativeTest.agentNotificationPermission
              ? "granted"
              : "denied";
          if (command === "notify_agent") {
            const preferences = JSON.parse(
              localStorage.getItem("test-terminal-preferences") ?? "null",
            );
            if (
              desktop.__nativeTest.windowFocused ||
              preferences?.agentNotifications === false
            )
              return false;
            desktop.__nativeTest.agentNotifications.push(args);
            return true;
          }
          if (command === "update_environment")
            return { linuxInstruction: desktop.__nativeTest.updateInstruction };
          if (command === "request_update_check") {
            localStorage.setItem("test-update-check", String(Date.now()));
            return;
          }
          if (command === "check_app_update") {
            await new Promise((resolve) =>
              setTimeout(resolve, desktop.__nativeTest.updateCheckDelay),
            );
            if (desktop.__nativeTest.updateCheckError)
              throw new Error(desktop.__nativeTest.updateCheckError);
            return desktop.__nativeTest.update;
          }
          if (command === "plugin:updater|download") {
            const send = (index: number, message: unknown) =>
              callbacks.get(args.onEvent.id)?.({ index, message });
            send(0, {
              event: "Started",
              data: { contentLength: desktop.__nativeTest.updateContentLength },
            });
            send(1, { event: "Progress", data: { chunkLength: 50 } });
            await new Promise((resolve) =>
              setTimeout(resolve, desktop.__nativeTest.updateDownloadDelay),
            );
            if (desktop.__nativeTest.updateDownloadError)
              throw new Error(desktop.__nativeTest.updateDownloadError);
            send(2, { event: "Finished" });
            return 902;
          }
          if (command === "plugin:updater|install") {
            if (desktop.__nativeTest.updateInstallError)
              throw new Error(desktop.__nativeTest.updateInstallError);
            return;
          }
          if (command === "restart_after_update") {
            if (desktop.__nativeTest.updateRestartError)
              throw new Error(desktop.__nativeTest.updateRestartError);
            return;
          }
          if (command === "android_exit") {
            const action = args.action;
            const mock = desktop.__nativeTest;
            if (action.type === "begin") {
              if (mock.androidPreparation)
                throw new Error("Shutdown is already in progress");
              return (mock.androidPreparation = crypto.randomUUID());
            }
            if (action.preparation !== mock.androidPreparation)
              throw new Error("Stale shutdown preparation");
            if (action.type === "finish") {
              await new Promise((resolve) =>
                setTimeout(resolve, mock.androidExitDelay),
              );
              if (mock.androidExitError) throw new Error(mock.androidExitError);
            } else if (action.type === "resume") {
              mock.androidPreparation = null;
            }
            return null;
          }
          if (command === "local_web_servers") {
            const result = [...desktop.__nativeTest.localWebServers];
            let index = 0;
            try {
              if (desktop.__nativeTest.localWebServersError)
                throw new Error(desktop.__nativeTest.localWebServersError);
              for (const url of desktop.__nativeTest.localWebServersProgress ??
                result)
                callbacks.get(args.onFound.id)?.({
                  index: index++,
                  message: url,
                });
              if (desktop.__nativeTest.holdLocalWebServers)
                await new Promise((resolve) => {
                  desktop.__nativeTest.finishLocalWebServers = resolve;
                });
              else if (desktop.__nativeTest.localWebServersDelay)
                await new Promise((resolve) =>
                  setTimeout(
                    resolve,
                    desktop.__nativeTest.localWebServersDelay,
                  ),
                );
              desktop.__nativeTest.localWebServersCompleted++;
              return result;
            } finally {
              callbacks.get(args.onFound.id)?.({ index, end: true });
            }
          }
          if (command === "sync_browsers") {
            for (const id of browsers.keys())
              if (!args.retained.includes(id)) browsers.delete(id);
            for (const browser of browsers.values()) browser.visible = false;
            for (const slot of args.slots) {
              if (!browsers.has(slot.id))
                browsers.set(slot.id, {
                  id: slot.id,
                  revision: "1",
                  url: slot.url,
                  title: "Browser",
                  loading: false,
                  error: "",
                  download: "",
                  visits: 1,
                });
              Object.assign(browsers.get(slot.id), {
                visible: true,
                bounds: slot.bounds,
              });
            }
            return [...browsers.values()].map(
              ({ id, revision, url, title, loading, error, download }) => ({
                id,
                revision,
                url,
                title,
                loading,
                error,
                download,
              }),
            );
          }
          if (command === "browser_action") {
            const browser = browsers.get(args.id);
            if (!browser) throw new Error("Browser panel is closed.");
            if (args.action.type === "navigate") {
              browser.url = args.action.url;
              browser.visits++;
            }
            if (args.action.type === "reload") browser.visits++;
            browser.revision = String(BigInt(browser.revision) + 1n);
            await emitEvent("browser-page", {
              id: browser.id,
              revision: browser.revision,
              url: browser.url,
              title: browser.title,
              loading: false,
              error: "",
              download: "",
            });
            return;
          }
          if (command === "show_ready_window") return true;
          if (command === "plugin:webview|set_webview_zoom") {
            if (desktop.__nativeTest.zoomDelay)
              await new Promise((resolve) =>
                setTimeout(resolve, desktop.__nativeTest.zoomDelay),
              );
            if (desktop.__nativeTest.failZoom)
              throw new Error("Zoom unavailable");
            desktop.__nativeTest.zoom = args.value;
            return;
          }
          if (command === "resolve_editor_file") return args.relative;
          if (command === "watch_editor_files") return;
          if (command === "watch_explorer_directories") return 2;
          if (command === "save_new_editor_file") {
            if (desktop.__nativeTest.fileSaveDelay)
              await new Promise((resolve) =>
                setTimeout(resolve, desktop.__nativeTest.fileSaveDelay),
              );
            if (desktop.__nativeTest.failFileSave)
              throw { kind: "io", message: "Disk is full" };
            const path = desktop.__nativeTest.newFilePath;
            if (!path) return null;
            const separator = path.lastIndexOf("/");
            const root = path.slice(0, separator);
            const relative = path.slice(separator + 1);
            if (
              args.openFiles.some(
                (file: any) => `${file.root}/${file.relative}` === path,
              )
            )
              throw {
                kind: "openFile",
                message:
                  "This file is already open. Close its editor tabs before replacing it.",
              };
            const file = {
              content: args.content,
              revision: crypto.randomUUID(),
              encoding: "utf8",
              readOnly: false,
            };
            desktop.__nativeTest.editorFiles[path] = file;
            localStorage.setItem(
              "test-editor-files",
              JSON.stringify(desktop.__nativeTest.editorFiles),
            );
            return {
              location: { root, relative },
              file: { ...file, path, relative },
            };
          }
          if (
            command === "read_editor_file" ||
            command === "save_editor_file"
          ) {
            const request =
              command === "read_editor_file" ? args : args.request;
            const key = `${request.root}/${request.relative}`;
            const files = desktop.__nativeTest.editorFiles;
            if (!Object.hasOwn(files, key))
              files[key] = {
                content: request.relative.endsWith(".md")
                  ? "# Project\nA text file preview.\n"
                  : 'fn main() {\n    println!("Hello, 🦀!");\n}\n',
                revision: "initial",
                encoding: "utf8",
                readOnly: false,
              };
            if (command === "read_editor_file") {
              if (desktop.__nativeTest.fileReadError)
                throw {
                  kind: "io",
                  message: desktop.__nativeTest.fileReadError,
                };
              const delay =
                desktop.__nativeTest.fileReadDelays[request.relative];
              if (delay)
                await new Promise((resolve) => setTimeout(resolve, delay));
              const file = files[key];
              if (!file)
                throw { kind: "io", message: "The file no longer exists." };
              return {
                ...file,
                path: key,
                relative: request.relative,
                content:
                  file.revision === args.knownRevision ? null : file.content,
              };
            }
            if (desktop.__nativeTest.fileSaveDelay)
              await new Promise((resolve) =>
                setTimeout(resolve, desktop.__nativeTest.fileSaveDelay),
              );
            if (desktop.__nativeTest.failFileSave)
              throw { kind: "io", message: "Disk is full" };
            const file = files[key];
            if (!file)
              throw { kind: "io", message: "The file no longer exists." };
            if (request.revision !== file.revision)
              throw { kind: "conflict", message: "The file changed on disk." };
            if (file.readOnly)
              throw { kind: "readOnly", message: "This file is read-only." };
            file.content = request.content;
            file.revision += "+saved";
            localStorage.setItem("test-editor-files", JSON.stringify(files));
            await emitEvent("editor-files-changed", [key]);
            return file.revision;
          }
          if (command === "about_info") return desktop.__nativeTest.aboutInfo;
          if (command === "app_info")
            return {
              directory: "/project",
              home: "/home/test",
              platform,
              profiles: [
                ...(platform === "windows"
                  ? ["pwsh", "powershell", "cmd"].map((kind) => ({
                      id: `local:${kind}`,
                      name: kind,
                      kind,
                      program: `${kind}.exe`,
                      distro: null,
                      home: "/home/test",
                    }))
                  : []),
                {
                  id: "local:bash",
                  name: "bash",
                  kind: "bash",
                  program: "/bin/bash",
                  distro: null,
                  home: "/home/test",
                },
              ],
            };
          if (command === "load_session")
            return JSON.parse(
              localStorage.getItem("test-session") ?? JSON.stringify(saved),
            );
          if (command === "load_keybindings")
            return JSON.parse(
              localStorage.getItem("test-keybindings") ?? "null",
            );
          if (command === "load_terminal_preferences")
            return JSON.parse(
              localStorage.getItem("test-terminal-preferences") ?? "null",
            );
          if (command === "load_editor_preferences")
            return JSON.parse(
              localStorage.getItem("test-editor-preferences") ?? "null",
            );
          const themeOwner = (id: string) =>
            JSON.parse(localStorage.getItem("test-plugins") ?? "[]").find(
              (entry: any) => entry.themeIds?.includes(id),
            )?.id;
          const themeBundle = async (id: string, manifest: any) => {
            const { migrateTheme, isBuiltinTheme } = await import(
              location.origin + "/src/theme/format.ts"
            );
            const modern =
              manifest.version === 1
                ? migrateTheme(manifest).manifest
                : manifest;
            const raw =
              JSON.parse(localStorage.getItem("test-theme-raw") ?? "{}")[id] ??
              JSON.stringify(modern, null, 2);
            return {
              id,
              raw,
              iconTheme: JSON.parse(
                localStorage.getItem("test-icon-themes") ?? "{}",
              )[id],
              revision: raw,
              directory: isBuiltinTheme(id) ? "" : `/app/themes/${id}`,
              readOnly: isBuiltinTheme(id) || !!themeOwner(id),
            };
          };
          if (command === "plugin:fs|watch") return 1;
          if (command === "plugin:resources|close") return;
          if (command === "report_plugin_status") return;
          if (command === "list_plugins")
            return {
              directory: "/app/plugins",
              entries: JSON.parse(localStorage.getItem("test-plugins") ?? "[]"),
              safeMode: !!desktop.__nativeTest.pluginSafeMode,
            };
          if (command === "import_plugin") {
            const entry = desktop.__nativeTest.pluginImport;
            if (!entry) throw new Error("Invalid plugin fixture");
            const entries = JSON.parse(
              localStorage.getItem("test-plugins") ?? "[]",
            );
            localStorage.setItem(
              "test-plugins",
              JSON.stringify([
                ...entries.filter((e: any) => e.id !== entry.id),
                entry,
              ]),
            );
            await emitEvent("plugins-changed");
            return entry.id;
          }
          if (command === "enable_plugin") {
            const entries = JSON.parse(
              localStorage.getItem("test-plugins") ?? "[]",
            );
            const entry = entries.find((e: any) => e.id === args.id);
            if (!entry || entry.revision !== args.expected)
              throw new Error("Plugin changed");
            entry.enabled = true;
            entry.trustedRevision = entry.revision;
            localStorage.setItem("test-plugins", JSON.stringify(entries));
            await emitEvent("plugins-changed");
            return;
          }
          if (command === "prepare_plugin") {
            if (location.search.includes("settings"))
              throw new Error("Wrong caller");
            const entry = JSON.parse(
              localStorage.getItem("test-plugins") ?? "[]",
            ).find((e: any) => e.id === args.id);
            if (
              !entry?.enabled ||
              entry.trustedRevision !== args.expected ||
              desktop.__nativeTest.pluginSafeMode
            )
              throw new Error("Not trusted");
            return entry.manifest;
          }
          if (command === "request_plugin_removal") {
            const token = crypto.randomUUID();
            const request = { ...args, token };
            localStorage.setItem(
              "test-plugin-request",
              JSON.stringify(request),
            );
            await emitEvent("plugin-close-request", request);
            return token;
          }
          if (command === "finish_plugin_removal") {
            const request = JSON.parse(
              localStorage.getItem("test-plugin-request")!,
            );
            if (args.approved) {
              let entries = JSON.parse(
                localStorage.getItem("test-plugins") ?? "[]",
              );
              if (request.uninstall)
                entries = entries.filter((e: any) => e.id !== request.id);
              else
                entries.find((e: any) => e.id === request.id).enabled = false;
              localStorage.setItem("test-plugins", JSON.stringify(entries));
              await emitEvent("plugins-changed");
            }
            const result = { token: args.token, approved: args.approved };
            localStorage.setItem(
              "test-plugin-finished",
              JSON.stringify(result),
            );
            await emitEvent("plugin-operation-finished", result);
            return;
          }
          if (command === "duplicate_theme") {
            const manifests = JSON.parse(
              localStorage.getItem("test-theme-manifests") ?? "{}",
            );
            const { builtinTheme, deepmonoTheme, deepmonoThemeId } =
              await import(location.origin + "/src/theme/format.ts");
            manifests.copy =
              args.id === deepmonoThemeId
                ? deepmonoTheme
                : args.id
                  ? manifests[args.id]
                  : builtinTheme;
            localStorage.setItem(
              "test-theme-manifests",
              JSON.stringify(manifests),
            );
            return "copy";
          }
          if (command === "list_themes") {
            const manifests = JSON.parse(
              localStorage.getItem("test-theme-manifests") ?? "{}",
            );
            return {
              directory: "/home/test/.local/share/dev.lomi.desktop/themes",
              themes: Object.entries(manifests).map(
                ([id, value]: [string, any]) => ({
                  id,
                  kind: value.iconTheme?.kind ?? "color",
                  name: value.name ?? id,
                  description: value.description ?? "",
                  author: value.author ?? "",
                  owner: themeOwner(id) ?? null,
                  error: [1, 2].includes(value.version)
                    ? null
                    : "Unsupported theme version",
                }),
              ),
            };
          }
          if (command === "load_theme") {
            const { deepmonoTheme, deepmonoThemeId } = await import(
              location.origin + "/src/theme/format.ts"
            );
            const manifest =
              args.id === deepmonoThemeId
                ? deepmonoTheme
                : JSON.parse(
                    localStorage.getItem("test-theme-manifests") ?? "{}",
                  )[args.id];
            if (!manifest) throw new Error("Theme folder is missing.");
            return themeBundle(args.id, manifest);
          }
          if (command === "load_theme_preferences") {
            if (desktop.__nativeTest.themeLoadError)
              throw new Error(desktop.__nativeTest.themeLoadError);
            const preferences = JSON.parse(
              localStorage.getItem("test-theme-settings") ??
                '{"version":1,"active":null}',
            );
            if (preferences.version !== 1)
              throw new Error(
                "Unsupported theme settings version. The file has been left intact.",
              );
            delete preferences.customCss;
            preferences.appearance ??= "system";
            if (!["system", "light", "dark"].includes(preferences.appearance))
              throw new Error(
                "Invalid color mode. The file has been left intact.",
              );
            const { deepmonoTheme, deepmonoThemeId } = await import(
              location.origin + "/src/theme/format.ts"
            );
            const manifest =
              preferences.active === deepmonoThemeId
                ? deepmonoTheme
                : JSON.parse(
                    localStorage.getItem("test-theme-manifests") ?? "{}",
                  )[preferences.active];
            if (preferences.active && !manifest)
              throw new Error("Theme folder is missing.");
            return {
              preferences,
              theme: manifest
                ? await themeBundle(preferences.active, manifest)
                : null,
              fileIcons: preferences.fileIcons
                ? await themeBundle(
                    preferences.fileIcons,
                    JSON.parse(
                      localStorage.getItem("test-theme-manifests") ?? "{}",
                    )[preferences.fileIcons],
                  )
                : null,
              productIcons: preferences.productIcons
                ? await themeBundle(
                    preferences.productIcons,
                    JSON.parse(
                      localStorage.getItem("test-theme-manifests") ?? "{}",
                    )[preferences.productIcons],
                  )
                : null,
              revision: 1,
              safeMode: false,
            };
          }
          if (command === "save_theme_preferences") {
            if (desktop.__nativeTest.failThemeSave)
              throw new Error("Cannot save theme: Disk is full");
            localStorage.setItem(
              "test-theme-settings",
              JSON.stringify(args.data),
            );
            await emitEvent("theme-changed");
            return;
          }
          if (command === "save_theme_manifest") {
            if (desktop.__nativeTest.failThemeSave)
              throw new Error("Cannot save theme: Disk is full");
            const manifests = JSON.parse(
              localStorage.getItem("test-theme-manifests") ?? "{}",
            );
            if (
              (await themeBundle(args.id, manifests[args.id])).revision !==
              args.expected
            )
              throw new Error(
                "This theme changed on disk. Reopen the editor before saving; your draft is still available.",
              );
            const { readThemeDraft } = await import(
              location.origin + "/src/theme/format.ts"
            );
            manifests[args.id] = readThemeDraft(args.raw);
            localStorage.setItem(
              "test-theme-raw",
              JSON.stringify({
                ...JSON.parse(localStorage.getItem("test-theme-raw") ?? "{}"),
                [args.id]: args.raw,
              }),
            );
            localStorage.setItem(
              "test-theme-manifests",
              JSON.stringify(manifests),
            );
            localStorage.setItem("test-theme-refresh", String(Date.now()));
            await emitEvent("theme-changed");
            return themeBundle(args.id, manifests[args.id]);
          }
          if (command === "refresh_themes") {
            localStorage.setItem("test-theme-refresh", String(Date.now()));
            await emitEvent("theme-changed");
            return;
          }
          if (
            command === "export_vscode_theme" ||
            command === "export_vscode_icon_theme"
          ) {
            if (desktop.__nativeTest.themeExportError)
              throw new Error(desktop.__nativeTest.themeExportError);
            return args.directory + "/lomi-theme.vsix";
          }
          if (command === "import_vscode_themes") {
            if (desktop.__nativeTest.themeImportError)
              throw new Error(desktop.__nativeTest.themeImportError);
            const manifests = JSON.parse(
              localStorage.getItem("test-theme-manifests") ?? "{}",
            );
            const imported = desktop.__nativeTest.vscodeThemes ?? [];
            const ids = imported.map((manifest: unknown, index: number) => {
              const id = `vscode-theme-${index + 1}`;
              manifests[id] = manifest;
              return id;
            });
            localStorage.setItem(
              "test-theme-manifests",
              JSON.stringify(manifests),
            );
            return ids;
          }
          if (command === "import_theme" || command === "create_theme") {
            if (desktop.__nativeTest.themeImportError)
              throw new Error(desktop.__nativeTest.themeImportError);
            const manifests = JSON.parse(
              localStorage.getItem("test-theme-manifests") ?? "{}",
            );
            const id = command === "create_theme" ? "my-theme" : "imported";
            manifests[id] =
              command === "create_theme"
                ? {
                    version: 2,
                    name: "My theme",
                    appearance: "adaptive",
                    common: {},
                  }
                : {
                    version: 1,
                    name: "Imported theme",
                    tokens: { "--radius-control": "12px" },
                  };
            localStorage.setItem(
              "test-theme-manifests",
              JSON.stringify(manifests),
            );
            return id;
          }
          if (["sync_theme_window", "open_themes_folder"].includes(command))
            return;
          if (command === "save_keybindings") {
            if (desktop.__nativeTest.failKeybindingsSave)
              throw new Error("Cannot save shortcuts: Disk is full");
            localStorage.setItem("test-keybindings", JSON.stringify(args.data));
            await emitEvent("keybindings-changed");
            return;
          }
          if (command === "save_terminal_preferences") {
            if (desktop.__nativeTest.failTerminalPreferencesSave)
              throw "Disk is full";
            localStorage.setItem(
              "test-terminal-preferences",
              JSON.stringify(args.data),
            );
            await emitEvent("terminal-preferences-changed");
            return;
          }
          if (command === "save_editor_preferences") {
            if (desktop.__nativeTest.failEditorPreferencesSave)
              throw new Error("Cannot save editor settings: Disk is full");
            localStorage.setItem(
              "test-editor-preferences",
              JSON.stringify(args.data),
            );
            await emitEvent("editor-preferences-changed");
            return;
          }
          if (command === "save_session") {
            if (desktop.__nativeTest.failSave) throw new Error("Disk is full");
            localStorage.setItem("test-session", JSON.stringify(args.data));
            return;
          }
          if (command === "validate_directory") {
            if (desktop.__nativeTest.directoryError)
              throw new Error(desktop.__nativeTest.directoryError);
            return args.path;
          }
          if (command === "list_directory")
            return args.relative
              ? [
                  {
                    name: "main.ts",
                    relativePath: "src/main.ts",
                    path: `${args.root}/src/main.ts`,
                    isDirectory: false,
                    isSymlink: false,
                  },
                ]
              : [
                  {
                    name: "src",
                    relativePath: "src",
                    path: `${args.root}/src`,
                    isDirectory: true,
                    isSymlink: false,
                  },
                  {
                    name: "README.md",
                    relativePath: "README.md",
                    path: `${args.root}/README.md`,
                    isDirectory: false,
                    isSymlink: false,
                  },
                  {
                    name: "it's a file.txt",
                    relativePath: "it's a file.txt",
                    path: `${args.root}/it's a file.txt`,
                    isDirectory: false,
                    isSymlink: false,
                  },
                ];
          if (command === "preview_file")
            return "# Project\nA text file preview.";
          if (command === "git_status")
            return repositoryPresent
              ? { root: args.root, branch: "main", changes }
              : null;
          if (command === "git_repositories")
            return {
              repositories: repositoryPresent
                ? [{ root: args.root, branch: "main", changes }]
                : [],
              errors: [],
              limited: false,
            };
          if (command === "git_history") {
            if (desktop.__nativeTest.failHistory)
              throw new Error("History is unavailable");
            const commits = desktop.__nativeTest.gitHistory?.commits ?? [];
            return {
              commits: commits.slice(args.skip, args.skip + 50),
              tips:
                args.tips ??
                commits.slice(0, 1).map((commit: any) => commit.id),
              hasMore: commits.length > args.skip + 50,
            };
          }
          if (command === "git_commit_details") {
            if (desktop.__nativeTest.failCommitDetails)
              throw new Error("Commit is unavailable");
            const details = desktop.__nativeTest.gitHistory?.details[args.id];
            if (!details) throw new Error("Commit was not found");
            return details;
          }
          if (command === "git_commit_diff") {
            if (desktop.__nativeTest.failCommitDiff)
              throw new Error("Diff is unavailable");
            const diff = desktop.__nativeTest.gitHistory?.diffs[args.path];
            if (!diff) throw new Error("Diff was not found");
            const delay = desktop.__nativeTest.diffDelays[args.path];
            if (delay)
              await new Promise((resolve) => setTimeout(resolve, delay));
            desktop.__nativeTest.resolvedDiffs.push(args.path);
            return diff;
          }
          if (command === "git_stage") {
            changes = changes.map((change) => ({
              ...change,
              index: args.stage ? "M" : " ",
              worktree: args.stage ? " " : "M",
            }));
            return;
          }
          if (command === "git_diff")
            return {
              patch: "@@ -1 +1 @@\n-old\n+new\n",
              truncated: false,
              notice: null,
            };
          if (command === "git_commit") {
            changes = [];
            return;
          }
          if (command === "start_terminal") {
            sessions.set(args.request.id, { output: args.output.id, index: 0 });
            setTimeout(
              () =>
                emit(
                  args.request.id,
                  `\x1b]7;file://localhost${args.request.cwd}\x07\x1b]133;A\x07bash $ \x1b]133;B\x07`,
                ),
              desktop.__nativeTest.terminalOutputDelay,
            );
            return { cwd: args.request.cwd, profileId: args.request.profileId };
          }
          if (command === "terminal_contexts")
            return desktop.__nativeTest.terminalContexts;
          if (command === "busy_terminals") {
            if (desktop.__nativeTest.terminalProcessDelay)
              await new Promise((resolve) =>
                setTimeout(resolve, desktop.__nativeTest.terminalProcessDelay),
              );
            if (desktop.__nativeTest.terminalProcessError)
              throw new Error(desktop.__nativeTest.terminalProcessError);
            return desktop.__nativeTest.busyTerminals.filter((id: string) =>
              args.ids.includes(id),
            );
          }
          if (command === "inspect_cli_titles")
            return desktop.__nativeTest.cliTitleSetup;
          if (command === "enable_cli_titles") {
            await new Promise((resolve) =>
              setTimeout(resolve, desktop.__nativeTest.cliTitleSaveDelay),
            );
            if (desktop.__nativeTest.cliTitleError)
              throw new Error(desktop.__nativeTest.cliTitleError);
            desktop.__nativeTest.cliTitleSetup = null;
            return;
          }
          if (command === "inspect_cli_integrations") {
            const mock = desktop.__nativeTest;
            const configured = mock.cliIntegrationStatuses[args.process.cli];
            const status = configured ?? {
              cli: args.process.cli,
              features: [],
            };
            return {
              ...status,
              features: status.features
                .filter(
                  (feature: any) =>
                    !mock.dismissedCliIntegrations.has(
                      `${args.process.cli}:${feature.feature}`,
                    ),
                )
                .map((feature: any) => ({ ...feature })),
            };
          }
          if (command === "enable_cli_integration") {
            const mock = desktop.__nativeTest;
            if (mock.cliIntegrationSaveDelay)
              await new Promise((resolve) =>
                setTimeout(resolve, mock.cliIntegrationSaveDelay),
              );
            if (mock.cliIntegrationError)
              throw new Error(mock.cliIntegrationError);
            const status = mock.cliIntegrationStatuses[args.process.cli];
            if (status) {
              const feature = status.features.find(
                (item: any) => item.feature === args.feature,
              );
              if (feature) feature.configured = true;
            }
            return `${args.process.cli} ${args.feature} are configured.`;
          }
          if (command === "dismiss_cli_integrations") {
            desktop.__nativeTest.dismissedCliIntegrations.set(
              `${args.cli}:${args.feature}`,
              true,
            );
            return;
          }
          if (command === "inspect_mcp_clients")
            return desktop.__nativeTest.mcpClients.map((client: any) => ({
              ...client,
            }));
          if (command === "install_mcp_client") {
            const mock = desktop.__nativeTest;
            const error = mock.mcpInstallErrors[args.cli];
            if (error) throw new Error(error);
            const client = mock.mcpClients.find(
              (item: any) => item.cli === args.cli,
            );
            if (client) client.configured = true;
            return;
          }
          if (command === "quote_paths")
            return args.paths
              .map((path: string) => "'" + path.replaceAll("'", "'\\''") + "'")
              .join(" ");
          if (
            command === "paste_terminal_clipboard" ||
            command === "plugin:clipboard-manager|read_text"
          )
            return "clipboard text";
          if (command === "plugin:dialog|open")
            return desktop.__nativeTest.folder;
          if (command === "plugin:window|scale_factor") return 1;
          if (command === "plugin:window|is_visible") return true;
          if (command === "plugin:window|is_focused") return true;
          if (command === "plugin:window|is_minimized") return false;
          if (command === "plugin:window|is_fullscreen")
            return desktop.__nativeTest.fullscreen;
          if (command === "plugin:event|listen") {
            const id = ++callbackId;
            events.set(id, { event: args.event, handler: args.handler });
            return id;
          }
          if (command === "plugin:event|unlisten") {
            events.delete(args.eventId);
            return;
          }
          if (command === "plugin:window|close") {
            for (const [id, listener] of events) {
              if (listener.event === "tauri://close-requested")
                await callbacks.get(listener.handler)?.({
                  event: listener.event,
                  id,
                  payload: null,
                });
            }
            return;
          }
          if (
            [
              "chat_close",
              "chat_retain",
              "write_terminal",
              "resize_terminal",
              "acknowledge_terminal",
              "close_terminal",
              "reset_terminals",
              "open_settings",
              "finish_window_startup",
              "plugin:event|unlisten",
              "plugin:window|set_title",
              "plugin:window|destroy",
              "plugin:window|minimize",
              "plugin:window|toggle_maximize",
              "plugin:clipboard-manager|write_text",
              "plugin:opener|open_url",
            ].includes(command)
          )
            return;
          throw new Error(`Unexpected native command: ${command}`);
        },
      };
    },
    { repository, saved, gitHistory, editorFiles, platform },
  );
}

export async function buffer(page: Page, paneId: string) {
  return page.evaluate(async (id) => {
    // Reuse the application's module when Vite adds a version after an update.
    const runtimeUrl = performance
      .getEntriesByType("resource")
      .map((entry) => entry.name)
      .filter((url) => new URL(url).pathname === "/src/terminal-runtime.ts")
      .at(-1);
    const { runningTerminal } = await import(
      runtimeUrl ?? "/src/terminal-runtime.ts"
    );
    const buffer = runningTerminal(id)!.terminal.buffer.active;
    return Array.from({ length: buffer.length }, (_, row) =>
      buffer.getLine(row)?.translateToString(true),
    ).join("\n");
  }, paneId);
}

export async function chooseOption(control: Locator, label: string) {
  await control.click();
  await control
    .page()
    .getByRole("option", { name: label, exact: true })
    .click();
}
