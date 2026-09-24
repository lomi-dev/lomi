// Sequential qualification reuse; never an installer or release entry point.
import { createHash } from "node:crypto";
import { readFile, writeFile, rename } from "node:fs/promises";
import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { createServer } from "vite";

const root = resolve(import.meta.dirname, "../..");
const binary = resolve(root, "src-tauri/target/debug/lomi");
const expected = process.env.LOMI_MCP_PREBUILT_SHA256;
if (
  !/^[a-f0-9]{64}$/.test(expected ?? "") ||
  !process.env.LOMI_MCP_BATCH_IDENTIFIER
)
  throw Error("Prebuilt reuse requires the preceding batch's binary identity");
const actual = createHash("sha256")
  .update(await readFile(binary))
  .digest("hex");
if (actual !== expected)
  throw Error("Native binary changed during the routing batch");
const server = await createServer({
  root,
  server: { host: "127.0.0.1", port: 1444, strictPort: true },
});
let app;
const events = [];
const record = async (event, fields = {}) => {
  events.push({ event, at: Date.now(), ...fields });
  const path = resolve(
    process.env.LOMI_MCP_CONTROL_PROBE_DIRECTORY,
    "prebuilt-process.json",
  );
  await writeFile(
    path + ".tmp",
    JSON.stringify({ sha256: actual, events }, null, 2),
  );
  await rename(path + ".tmp", path);
};
process.on("SIGTERM", () => app?.kill("SIGTERM"));
try {
  await server.listen();
  app = spawn("sh", ["scripts/run-macos-dev.sh", binary], {
    cwd: root,
    env: process.env,
    stdio: "inherit",
  });
  const closed = new Promise((resolve, reject) => {
    app.once("error", reject);
    app.once("close", resolve);
  });
  await record("native-start", { pid: app.pid });
  const code = await closed;
  await record("native-close", { code, signal: app.signalCode });
  process.exitCode = code ?? 1;
} finally {
  await server.close();
  await record("vite-closed");
}
