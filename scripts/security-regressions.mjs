import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(import.meta.dirname, "..");
const unixSuites = [
  ["lomi-control-core", "authentication::tests::"],
  ["lomi-control-core", "project_files::tests::"],
  ["lomi-control-core", "broker::revoke_tests::"],
];

export function securitySuites(platform = process.platform) {
  return [
    ["lomi", "files::tests::json_"],
    ["lomi", "auth::"],
    ["lomi", "remote::"],
    ["lomi", "terminal::tests::remote_"],
    ...(platform === "win32" ? [] : unixSuites),
    ["lomi-remote-crypto", ""],
  ];
}

export async function runSecurityRegressions(
  platform = process.platform,
  run = spawnSync,
) {
  for (const [crate, filter] of securitySuites(platform)) {
    console.log(`Security regressions: ${crate} ${filter}`);
    const result = run(
      "cargo",
      [
        "test",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--locked",
        "-p",
        crate,
        ...(filter ? ["--lib", filter] : ["--tests"]),
      ],
      {
        cwd: root,
        encoding: "utf8",
        timeout: 1800000,
        maxBuffer: 16 * 1024 * 1024,
      },
    );
    // Flush captured Cargo logs before a failure exits the gate.
    await new Promise((resolve, reject) => {
      process.stdout.write(result.stdout ?? "", (error) =>
        error ? reject(error) : resolve(),
      );
    });
    await new Promise((resolve, reject) => {
      process.stderr.write(result.stderr ?? "", (error) =>
        error ? reject(error) : resolve(),
      );
    });
    if (platform === "win32" && result.status === 0xc0000139) {
      const cleanStderr = (result.stderr ?? "").replace(/\x1b\[[0-9;]*m/g, "");
      const executable = cleanStderr.match(
        /Running\s+unittests[^\r\n]*\(([^)\r\n]+\.exe)\)/,
      )?.[1];
      if (executable) {
        console.log(
          "Windows loader failure: inspecting the failed native test executable.",
        );
        const diagnostic = run(
          "pwsh",
          [
            "-NoProfile",
            "-File",
            resolve(root, "scripts/diagnose-windows-native-test.ps1"),
            "-TestExecutable",
            resolve(root, executable),
          ],
          {
            cwd: root,
            encoding: "utf8",
            timeout: 120000,
            maxBuffer: 16 * 1024 * 1024,
          },
        );
        await new Promise((resolve, reject) => {
          process.stdout.write(diagnostic.stdout ?? "", (error) =>
            error ? reject(error) : resolve(),
          );
        });
        await new Promise((resolve, reject) => {
          process.stderr.write(diagnostic.stderr ?? "", (error) =>
            error ? reject(error) : resolve(),
          );
        });
        console.log(
          `Windows loader diagnostic exit status: ${diagnostic.status}; ${diagnostic.error ?? ""}`,
        );
      }
    }
    if (
      result.status !== 0 ||
      !/test result: ok\. [1-9]\d* passed; 0 failed/.test(result.stdout ?? "")
    ) {
      throw new Error(
        `Security suite failed or selected no tests: ${crate} ${filter}\n` +
          `Exit status: ${result.status}; signal: ${result.signal ?? "none"}\n` +
          `${result.error ?? ""}\n` +
          `Final stdout:\n${(result.stdout ?? "").slice(-8192)}\n` +
          `Final stderr:\n${(result.stderr ?? "").slice(-8192)}`,
      );
    }
  }
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  await runSecurityRegressions();
}
