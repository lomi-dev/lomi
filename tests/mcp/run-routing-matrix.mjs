// Sequential fresh native/client sessions with one disposable build identity.
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, readFile, writeFile, open } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { binaryFingerprint, sourceFingerprint } from "./routing-build.mjs";

const cases = (process.argv[2] ?? "").split(",").filter(Boolean);
const repeats = Number(process.argv[3] ?? 3);
if (
  !cases.length ||
  cases.some((name) => !/^[a-z][a-z-]+$/.test(name)) ||
  !Number.isInteger(repeats) ||
  repeats < 1 ||
  repeats > 3
)
  throw Error("Usage: node run-routing-matrix.mjs case,case [1..3]");
const directory = await mkdtemp(join(tmpdir(), "lomi-mcp-routing-matrix-"));
const identifier =
  "dev.lomi.mcp-control-batch-" + randomUUID().replaceAll("-", "");
const root = resolve(import.meta.dirname, "../..");
const sources = await sourceFingerprint(root);
let binary;
const results = [];
console.log("Routing matrix artifacts: " + directory);
const readJson = async (path) => {
  try {
    return JSON.parse(await readFile(path, "utf8"));
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
};
for (const name of cases) {
  for (let repeat = 1; repeat <= repeats; repeat++) {
    if ((await sourceFingerprint(root)) !== sources)
      throw Error(
        "Sources changed during the routing batch; start a fresh batch",
      );
    console.log("Starting " + name + " " + repeat + "/" + repeats);
    const logPath = join(directory, name + "-" + repeat + ".log");
    const log = await open(logPath, "wx", 0o600);
    const child = spawn(
      process.execPath,
      ["--experimental-strip-types", "tests/native/run-mcp-control.mjs"],
      {
        cwd: root,
        env: {
          ...process.env,
          LOMI_MCP_ROUTING_ONLY: name,
          LOMI_MCP_BATCH_IDENTIFIER: identifier,
          ...(binary ? { LOMI_MCP_PREBUILT_SHA256: binary } : {}),
        },
        stdio: ["ignore", log.fd, log.fd],
      },
    );
    const exitCode = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("close", resolve);
    });
    await log.close();
    const firstLines = (await readFile(logPath, "utf8")).slice(0, 8192);
    const artifactDirectory = firstLines.match(
      /Native MCP control artifacts: ([^\r\n]+)/,
    )?.[1];
    if (!artifactDirectory)
      throw Error("Native fixture did not identify its artifacts: " + logPath);
    const report = await readJson(
      join(artifactDirectory, "routing-result.json"),
    );
    const native = await readJson(join(artifactDirectory, "result.json"));
    const cleanup = await readJson(join(artifactDirectory, "cleanup.json"));
    const android = await readJson(
      join(artifactDirectory, "android-cleanup-after.json"),
    );
    const external = await readJson(
      join(artifactDirectory, "external-cleanup.json"),
    );
    const competingActions = report?.competingActions ?? [];
    const row = {
      task: name,
      repeat,
      artifactDirectory,
      exitCode,
      passed:
        exitCode === 0 &&
        native?.stage === "passed" &&
        report?.nativePostconditionsVerified === true,
      taskCompleted: report?.nativePostconditionsVerified === true,
      modelTurnCompleted: report?.modelTurnCompleted ?? false,
      selectedLomi: report?.selectedLomi ?? false,
      competingActions: competingActions.map((item) => ({
        type: item.type,
        id: item.id,
      })),
      silentFallback:
        report?.modelTurnCompleted && !competingActions.length ? false : null,
      prebuilt: Boolean(binary),
      clientVersion: report?.version,
      model: report?.model,
      reasoningEffort: report?.reasoningEffort,
      toolCalls:
        report?.items?.filter((item) => item.type === "mcpToolCall").length ??
        0,
      error: report?.error ?? native?.error ?? null,
      cleanup,
      androidCleanup: android,
      externalCleanup: external,
    };
    results.push(row);
    await writeFile(
      join(directory, "results.json"),
      JSON.stringify(
        { identifier, sourceFingerprint: sources, results },
        null,
        2,
      ),
      { mode: 0o600 },
    );
    console.log(
      name +
        " " +
        repeat +
        ": " +
        (row.passed ? "PASS" : "FAIL") +
        " " +
        artifactDirectory,
    );
    if (
      !cleanup?.hostExited ||
      !cleanup?.appDataRemoved ||
      (external && external.remainingPids.length !== 0) ||
      (android &&
        (android.processAlive !== false || android.privateAdbStopped !== true))
    )
      throw Error(
        "Cleanup must be reconciled before another native session: " +
          artifactDirectory,
      );
    binary ??= await binaryFingerprint(root);
  }
}
process.exitCode = results.every((row) => row.passed) ? 0 : 1;
