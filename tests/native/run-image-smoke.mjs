import {
  mkdtemp,
  mkdir,
  copyFile,
  readFile,
  writeFile,
  open,
} from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { newProject, newSession, openFileTab } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error("This native image smoke runner currently supports macOS.");
const repository = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-image-native-"));
const identifier = `dev.lomi.image-smoke-${Date.now()}`;
const appData = join(homedir(), "Library/Application Support", identifier);
const projectFolder = join(directory, "project");
await mkdir(projectFolder);
await mkdir(appData, { recursive: true });
const png = join(projectFolder, "picture.png");
await copyFile(join(repository, "src-tauri/icons/128x128.png"), png);
await copyFile(
  join(repository, "src-tauri/icons/icon.ico"),
  join(projectFolder, "favicon.ico"),
);
for (const [format, name] of [
  ["jpeg", "photo.jpg"],
  ["bmp", "bitmap.bmp"],
  ["tiff", "scan.tiff"],
])
  execFileSync("sips", [
    "-s",
    "format",
    format,
    png,
    "--out",
    join(projectFolder, name),
  ]);
await writeFile(
  join(projectFolder, "photo.webp"),
  Buffer.from(
    "UklGRiIAAABXRUJQVlA4IBYAAAAwAQCdASoBAAEADsD+JaQAA3AAAAAA",
    "base64",
  ),
);
await writeFile(
  join(projectFolder, "animation.gif"),
  Buffer.from(
    "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
    "base64",
  ),
);
await writeFile(
  join(projectFolder, "vector.svg"),
  '<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="1000"><rect x="100" y="100" width="1400" height="800" rx="100" fill="#737373"/></svg>',
);
await writeFile(
  join(projectFolder, "pixels.ppm"),
  "P3\n2 1\n255\n255 0 0 0 255 0\n",
);
await writeFile(join(projectFolder, "broken.tiff"), "broken");
const large = await open(join(projectFolder, "oversized.png"), "w");
await large.truncate(32 * 1024 * 1024 + 1);
await large.close();
const project = newProject(projectFolder, "local:zsh");
const session = openFileTab(
  { ...newSession(), projects: [project], activeProjectId: project.id },
  project.workspaces[0].id,
  projectFolder,
  "picture.png",
);
await writeFile(join(appData, "session.json"), JSON.stringify(session));
const config = join(directory, "config.json");
const port = Number(process.env.LOMI_IMAGE_SMOKE_PORT ?? 1431);
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand: `pnpm dev --port ${port}`,
      devUrl: `http://127.0.0.1:${port}`,
    },
  }),
);
console.log(`Native image artifacts: ${directory}`);
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
    cwd: repository,
    env: { ...process.env, LOMI_IMAGE_SMOKE_DIRECTORY: directory },
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
  let result;
  for (let i = 0; i < 480; i++) {
    try {
      result = JSON.parse(
        await readFile(join(directory, "result.json"), "utf8"),
      );
    } catch {}
    if (result || child.exitCode !== null) break;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  if (!result) throw Error("Native image smoke did not complete.");
  console.log(JSON.stringify(result, null, 2));
  if (result.stage !== "passed") process.exitCode = 1;
} finally {
  await writeFile(join(directory, "native.log"), log);
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {}
}
