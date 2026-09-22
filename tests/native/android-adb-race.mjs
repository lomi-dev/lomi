// Exercises the real client's automatic kill path only against an isolated fake server.
import { createServer, connect } from "node:net";
import { spawn } from "node:child_process";
import { resolve, join } from "node:path";
import { writeFile, mkdir } from "node:fs/promises";
const root = resolve(process.argv[2] ?? "/tmp/lomi-android-stage0-20260918");
const events = [];
const socketErrors = [];
let version = "0029";
const sockets = new Set();
const server = createServer((socket) => {
  sockets.add(socket);
  socket.on("error", (error) => socketErrors.push(error.code));
  socket.on("close", () => sockets.delete(socket));
  let input = Buffer.alloc(0);
  socket.on("data", (chunk) => {
    input = Buffer.concat([input, chunk]);
    if (input.length > 16384) return socket.destroy();
    if (input.length < 4) return;
    const size = Number.parseInt(input.subarray(0, 4).toString(), 16);
    if (input.length < size + 4) return;
    const command = input.subarray(4, 4 + size).toString();
    events.push(command);
    if (command === "host:version") socket.end(`OKAY0004${version}`);
    else {
      const message =
        command === "host:kill"
          ? "Probe refuses host:kill!"
          : "Unsupported in test";
      socket.end(
        `FAIL${Buffer.byteLength(message).toString(16).padStart(4, "0")}${message}`,
      );
    }
  });
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const { port } = server.address();
const preflight = await new Promise((resolve, reject) => {
  const socket = connect(port, "127.0.0.1", () =>
    socket.write("000Chost:version"),
  );
  let data = "";
  socket.setTimeout(2000, () => socket.destroy(Error("Preflight timeout")));
  socket.on("data", (bytes) => {
    data += bytes;
  });
  socket.on("error", reject);
  socket.on("end", () => resolve(data));
});
// Model replacement after a successful compatibility preflight.
version = "0028";
const env = { ...process.env };
for (const key of Object.keys(env))
  if (/^(ANDROID_|ADB_)/.test(key)) delete env[key];
await mkdir(join(root, "logs/tmp"), { recursive: true });
env.TMPDIR = join(root, "logs/tmp");
env.ANDROID_USER_HOME = join(root, "user");
env.ANDROID_ADB_SERVER_PORT = String(port);
env.ADB_SERVER_SOCKET = `tcp:127.0.0.1:${port}`;
const child = spawn(
  join(root, "sdk/platform-tools/adb"),
  ["-H", "127.0.0.1", "-P", String(port), "devices"],
  { env, stdio: ["ignore", "pipe", "pipe"] },
);
let output = "";
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (chunk) => {
    output = (output + chunk).slice(0, 4096);
  });
const timeout = setTimeout(() => child.kill("SIGTERM"), 3000);
const exit = await new Promise((resolve) =>
  child.on("exit", (code, signal) => resolve({ code, signal })),
);
clearTimeout(timeout);
for (const socket of sockets) socket.destroy();
await new Promise((resolve) => server.close(resolve));
const result = {
  preflight,
  port,
  events,
  socketErrors,
  attemptedAutomaticKill: events.includes("host:kill"),
  exit,
  output,
};
await writeFile(
  join(root, "evidence/adb-server-race.json"),
  JSON.stringify(result, null, 2),
);
console.log(JSON.stringify(result, null, 2));
if (!result.attemptedAutomaticKill) process.exitCode = 1;
