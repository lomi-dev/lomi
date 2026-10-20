import { spawn, execFileSync } from "node:child_process";
import { mkdir, readFile, writeFile, access } from "node:fs/promises";
import { homedir } from "node:os";
import { resolve, join } from "node:path";
import assert from "node:assert/strict";

// Build separately with native-smoke,tauri/custom-protocol and the exact
// --config identity. This runner never builds, calls pnpm, or invokes a shell.
const args = process.argv.slice(2);
const option = (name) => {
  const index = args.indexOf(name);
  if (index < 0 || !args[index + 1]) throw Error(`Required option: ${name}`);
  return resolve(args[index + 1]);
};
const binary = option("--binary");
const config = JSON.parse(await readFile(option("--config"), "utf8"));
const directory = option("--directory");
const identifier = config.identifier;
assert.match(
  identifier ?? "",
  /^dev\.lomi\.desktop-smoke-[a-zA-Z0-9.-]+$/,
  "Supply the unique isolated identifier used at binary BUILD time",
);
const base =
  process.platform === "darwin"
    ? join(homedir(), "Library/Application Support")
    : process.platform === "win32"
      ? process.env.APPDATA
      : process.env.XDG_DATA_HOME || join(homedir(), ".local/share");
assert(base, "Platform app-data directory is unavailable");
const appData = join(base, identifier);
// Refuse reuse before writing any profile or project fixture.
await access(binary);
for (const path of [directory, appData]) {
  let exists = true;
  try {
    await access(path);
  } catch (error) {
    if (error.code === "ENOENT") exists = false;
    else throw error;
  }
  assert(!exists, `Refusing to replace existing fixture/profile: ${path}`);
}
await mkdir(directory, { recursive: true });
await mkdir(appData, { recursive: true });
await writeFile(
  join(appData, "agent-control-preferences.json"),
  JSON.stringify({ version: 1, autoStart: false, yoloMode: false }),
);
const root = join(
  directory,
  process.platform === "win32"
    ? "project with spaces"
    : "project with spaces \\ literal",
);
await mkdir(root);
const gitEnv = Object.fromEntries(
  Object.entries(process.env).filter(
    ([key]) => !key.toUpperCase().startsWith("GIT_"),
  ),
);
Object.assign(gitEnv, {
  GIT_CONFIG_GLOBAL: join(directory, "absent-gitconfig"),
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_TERMINAL_PROMPT: "0",
});
const git = (...args) =>
  execFileSync("git", ["-C", root, ...args], { encoding: "utf8", env: gitEnv });
const encode = (text, encoding) => {
  if (encoding === "utf8") return Buffer.from(text);
  if (encoding === "utf8-bom")
    return Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(text)]);
  const bytes = Buffer.from(text, "utf16le");
  return encoding === "utf16-le"
    ? Buffer.concat([Buffer.from([0xff, 0xfe]), bytes])
    : Buffer.concat([Buffer.from([0xfe, 0xff]), bytes.swap16()]);
};
const fixtures = [
  [
    "utf8.txt",
    "utf8",
    "Zażółć 日本語 😀\r\nsecond\r\n",
    "Saved żółć 日本語 😀\r\nsecond\r\n",
  ],
  ["utf8-bom.txt", "utf8-bom", "BOM żółć\r\n", "Saved BOM żółć\r\n"],
  ["utf16le.txt", "utf16-le", "UTF16 żółć 😀\r\n", "Saved UTF16 żółć 😀\r\n"],
  ["utf16be.txt", "utf16-be", "UTF16 żółć 😀\r\n", "Saved UTF16 żółć 😀\r\n"],
];
for (const [file, encoding, original] of fixtures)
  await writeFile(join(root, file), encode(original, encoding));
await writeFile(
  join(root, "display.txt"),
  "Desktop fixture rendered in native CodeMirror\n",
);
await writeFile(join(root, "conflict.txt"), "original conflict\r\n");
await writeFile(join(root, "dirty-native.txt"), "");
await writeFile(join(root, "tracked.txt"), "Initial Git line\n");
git("init", "-b", "main");
for (const [key, value] of [
  ["user.name", "Desktop Smoke"],
  ["user.email", "desktop-smoke@example.test"],
  ["commit.gpgsign", "false"],
  ["core.hooksPath", join(directory, "disabled-hooks")],
  ["core.autocrlf", "false"],
])
  git("config", "--local", key, value);
git("add", "--", "tracked.txt");
git("commit", "-m", "Initial local fixture");
const session = {
  version: 5,
  activeProjectId: "desktop-project",
  projects: [
    {
      id: "desktop-project",
      path: root,
      activeWorkspaceId: "desktop-workspace",
      workspaces: [
        {
          id: "desktop-workspace",
          name: "Desktop native smoke",
          activeTabId: "desktop-file",
          tabs: [
            {
              type: "file",
              id: "desktop-file",
              title: "display.txt",
              root,
              relative: "display.txt",
            },
          ],
        },
      ],
    },
  ],
  sidebar: "files",
  sidebarWidth: 250,
  rightSidebar: null,
  rightSidebarWidth: 250,
  sidebarSides: { files: "left", git: "right", workspaces: "left" },
  terminalOverviewSide: "left",
};
await writeFile(join(appData, "session.json"), JSON.stringify(session));
await writeFile(
  join(directory, "run.json"),
  JSON.stringify(
    { binary, identifier, appData, root, platform: process.platform },
    null,
    2,
  ),
);
console.log(`Desktop native smoke artifacts: ${directory}`);
for (const phase of ["exercise", "restore"]) {
  let log = "";
  const child = spawn(binary, [], {
    env: {
      ...gitEnv,
      LOMI_DESKTOP_SMOKE_DIRECTORY: directory,
      LOMI_DESKTOP_SMOKE_IDENTIFIER: identifier,
      LOMI_DESKTOP_SMOKE_APP_DATA: appData,
      LOMI_DESKTOP_SMOKE_PHASE: phase,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const completion = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
  for (const stream of [child.stdout, child.stderr])
    stream.on("data", (data) => {
      log += data;
      process.stdout.write(data);
    });
  const timeout = setTimeout(() => child.kill("SIGTERM"), 120_000);
  try {
    const exit = await completion;
    await writeFile(join(directory, `${phase}-native.log`), log);
    const result = JSON.parse(
      await readFile(join(directory, `${phase}-result.json`), "utf8"),
    );
    console.log(JSON.stringify(result, null, 2));
    assert.equal(
      result.stage,
      "passed",
      `${phase}: ${JSON.stringify(result.data)}`,
    );
    assert.equal(
      exit.code,
      0,
      `${phase} process failed: ${JSON.stringify(exit)}`,
    );
  } finally {
    clearTimeout(timeout);
    if (child.exitCode === null) child.kill();
  }
}
for (const [file, encoding, , expected] of fixtures)
  assert.deepEqual(
    await readFile(join(root, file)),
    encode(expected, encoding),
    `External byte verification failed: ${file}`,
  );
assert.equal(
  await readFile(join(root, "conflict.txt"), "utf8"),
  "external change\r\n",
);
assert.equal(
  await readFile(join(root, "EMPTY.txt"), "utf8"),
  "Explorer copied bytes żółć\r\n",
);
assert.equal(
  await readFile(join(root, "tracked.txt"), "utf8"),
  "Native Git edited żółć\n",
);
assert.equal(
  git("log", "-1", "--format=%ae").trim(),
  "desktop-smoke@example.test",
);
assert.equal(
  git("log", "-1", "--format=%s").trim(),
  "Desktop smoke local commit",
);
assert.equal(git("diff", "HEAD", "--", "tracked.txt"), "");
assert.equal(git("status", "--porcelain", "--", "tracked.txt"), "");
assert.equal(
  await readFile(join(root, "renamed-native.txt"), "utf8"),
  "External renamed document żółć 😀",
);
const preferences = JSON.parse(
  await readFile(join(appData, "editor-preferences.json"), "utf8"),
);
assert.equal(preferences.tabSize, 7);
const savedSession = JSON.parse(
  await readFile(join(appData, "session.json"), "utf8"),
);
assert.equal(savedSession.projects[0].path, root);
const inbox = JSON.parse(
  await readFile(join(appData, "notifications.json"), "utf8"),
);
assert(
  inbox.items.some(
    (item) => item.title === "Desktop native persistence" && item.read,
  ),
);
await writeFile(
  join(directory, "external-verification.json"),
  JSON.stringify(
    {
      stage: "passed",
      checks: [
        "exact UTF-8/BOM/UTF-16 LE/BE Unicode CRLF bytes",
        "conflict external bytes preserved",
        "Explorer copy exact bytes",
        "dirty editor inline rename saves and watches relocated native path",
        "Git author, message, clean committed tracked file",
        "editor settings, session and read notification on disk after two launches",
      ],
    },
    null,
    2,
  ),
);
console.log(
  "Desktop native smoke passed; external bytes and persistence verified. Isolated artifacts retained.",
);
