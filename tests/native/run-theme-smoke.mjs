import { mkdtemp, mkdir, cp, readFile, writeFile } from "node:fs/promises";
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
await cp(
  process.argv[2] ??
    "/Applications/Visual Studio Code.app/Contents/Resources/app/extensions/theme-defaults",
  join(directory, "vscode-defaults"),
  { recursive: true },
);
const config = join(directory, "config.json");
await writeFile(config, JSON.stringify({ identifier }));
console.log(
  `Native theme artifacts: ${directory}\nIsolated application data: ${appData}`,
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
    env: { ...process.env, LOMI_THEME_SMOKE_DIRECTORY: directory },
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
  const deadline = Date.now() + 180000;
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
    if (error.code !== "ESRCH") throw error;
  }
  await writeFile(join(directory, "native.log"), log);
}
