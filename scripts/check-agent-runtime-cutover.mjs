import { readdir, readFile, access } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const retired = [
  "src/router",
  "src-tauri/src/cli_router",
  "scripts/install-router-clis.mjs",
  "scripts/build-router-grok.mjs",
  "scripts/router-cli-installations.json",
  "scripts/router-grok-artifacts.json",
];
const failures = [];
for (const name of retired) {
  try {
    await access(path.join(root, name));
    failures.push(`Retired implementation remains: ${name}`);
  } catch (cause) {
    if (cause.code !== "ENOENT") throw cause;
  }
}

async function inspect(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const filename = path.join(directory, entry.name);
    const relative = path.relative(root, filename).split(path.sep).join("/");
    if (relative === "scripts/check-agent-runtime-cutover.mjs") continue;
    // This frozen decoder preserves historical data only. It is never an owner.
    if (relative.startsWith("src-tauri/src/agent_runtime/migration/")) continue;
    if (entry.isDirectory()) {
      await inspect(filename);
    } else if (/\.(rs|tsx?|m?js|cjs|css|json)$/.test(entry.name)) {
      const source = await readFile(filename, "utf8");
      if (
        /CliRouterService|\bcli_router::|\b(?:cli_router|cli_run|cli_profile|cli_native)_[a-z_]+|cli-router-(?:changed|open-profile)|(?:\.\/|\.\.\/)router\//.test(
          source,
        )
      ) {
        failures.push(`Executable legacy reference: ${relative}`);
      }
    }
  }
}
await inspect(path.join(root, "src"));
await inspect(path.join(root, "src-tauri/src"));
await inspect(path.join(root, "scripts"));
await inspect(path.join(root, "src-tauri/capabilities"));
if (process.argv.includes("--bundle")) {
  await inspect(path.join(root, "dist"));
}
const configuration = await readFile(
  path.join(root, "src-tauri/tauri.conf.json"),
  "utf8",
);
if (configuration.includes("router-grok"))
  failures.push("Retired Grok bundle resource target remains");
if (failures.length) {
  process.stderr.write(`${failures.join("\n")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(
    "Agent runtime cutover source graph passed; legacy execution and IPC are absent.\n",
  );
}
