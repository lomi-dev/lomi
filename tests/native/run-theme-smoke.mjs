import {
  access,
  mkdtemp,
  mkdir,
  cp,
  readFile,
  writeFile,
} from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { newSession, newProject, openFileTab } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error("This native theme smoke runner currently supports macOS.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-theme-native-"));
const identifier = `dev.lomi.theme-smoke-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const projectFolder = join(directory, "project");
await mkdir(projectFolder);
await mkdir(appData, { recursive: true });
await writeFile(join(projectFolder, "source.ts"), "const value = 1;\n");
const project = newProject(projectFolder, "local:zsh");
project.workspaces[0].tabs[0].layout.id = "theme-terminal-pane";
const session = openFileTab(
  { ...newSession(), projects: [project], activeProjectId: project.id },
  project.workspaces[0].id,
  projectFolder,
  "source.ts",
);
await writeFile(join(appData, "session.json"), JSON.stringify(session));
const fixture = join(directory, "fixture");
await mkdir(fixture);
await writeFile(
  join(fixture, "package.json"),
  JSON.stringify({
    name: "theme-smoke",
    publisher: "test",
    contributes: {
      themes: [
        { label: "Native Portable", uiTheme: "vs-dark", path: "theme.json" },
      ],
      iconThemes: [
        {
          id: "portable-files",
          label: "Native file icons",
          path: "icons/file.json",
        },
      ],
      productIconThemes: [
        {
          id: "portable-product",
          label: "Native interface icons",
          path: "icons/product.json",
        },
      ],
    },
  }),
);
await cp(join(root, "themes/icons"), join(fixture, "icons"), {
  recursive: true,
});
await writeFile(
  join(fixture, "theme.json"),
  JSON.stringify({
    name: "Native Portable",
    colors: {
      "editor.background": "#162435",
      "editor.foreground": "#eeddaa",
      "terminal.ansiBlue": "#345678",
      "statusBar.background": "#394a5b",
    },
    tokenColors: [{ scope: "keyword", settings: { foreground: "#abcdef" } }],
    semanticHighlighting: true,
    semanticTokenColors: { variable: "#fedcba" },
  }),
);
let themeSource = process.argv[2];
let themeSourceKind = "supplied-vscode";
if (!themeSource) {
  themeSource =
    "/Applications/Visual Studio Code.app/Contents/Resources/app/extensions/theme-defaults";
  themeSourceKind = "installed-vscode";
  try {
    await access(themeSource);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    themeSource = join(root, "tests/native/fixtures/theme-defaults");
    themeSourceKind = "owned-fixture";
  }
}
await cp(themeSource, join(directory, "vscode-defaults"), { recursive: true });
const config = join(directory, "config.json");
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand:
        "pnpm ai:prepare && pnpm --filter @lomi-dev/remote-terminal-runtime build && pnpm dev --port 1449 --strictPort",
      devUrl: "http://127.0.0.1:1449",
    },
  }),
);
console.log(
  `Native theme artifacts: ${directory}\nIsolated application data: ${appData}\nVS Code theme source (${themeSourceKind}): ${themeSource}`,
);
const child = spawn(
  "pnpm",
  [
    "tauri",
    "dev",
    "--no-watch",
    "--features",
    "native-smoke",
    "--config",
    config,
  ],
  {
    cwd: root,
    env: {
      ...process.env,
      LOMI_THEME_SMOKE_DIRECTORY: directory,
      LOMI_THEME_SMOKE_SOURCE: themeSourceKind,
    },
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let log = "";
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (chunk) => {
    log += chunk;
    process.stdout.write(chunk);
  });
try {
  const deadline = Date.now() + 600000;
  let result;
  while (!result && Date.now() < deadline) {
    try {
      result = JSON.parse(
        await readFile(join(directory, "result.json"), "utf8"),
      );
    } catch (error) {
      if (error.code !== "ENOENT" && !(error instanceof SyntaxError))
        throw error;
    }
    if (!result) {
      if (child.exitCode !== null)
        throw Error(`Tauri exited before reporting: ${child.exitCode}`);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  if (!result) throw Error("Native theme smoke timed out.");
  console.log(JSON.stringify(result, null, 2));
  if (result.stage !== "passed") process.exitCode = 1;
} finally {
  // Tauri dev can retain Vite after app.exit; terminate only this runner's process group.
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch (error) {
    if (error.code !== "ESRCH" && error.code !== "EPERM") throw error;
    if (error.code === "EPERM") child.kill("SIGTERM");
  }
  await writeFile(join(directory, "native.log"), log);
}
