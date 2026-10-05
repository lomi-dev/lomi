import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, delimiter } from "node:path";
import { fileURLToPath } from "node:url";
import {
  runSecurityRegressions,
  securitySuites,
} from "../scripts/security-regressions.mjs";

test(
  "the native release gate stops on failures and empty test selections",
  {
    skip:
      process.platform === "win32"
        ? "The fake Cargo executable fixture requires a Unix shebang."
        : false,
  },
  () => {
    const directory = mkdtempSync(join(tmpdir(), "lomi-security-gate-"));
    const log = join(directory, "calls.jsonl");
    try {
      writeFileSync(
        join(directory, "cargo"),
        `#!${process.execPath}
const fs = require('node:fs');
fs.appendFileSync(process.env.LOMI_SECURITY_TEST_LOG, JSON.stringify(process.argv.slice(2)) + '\\n');
if (process.env.LOMI_SECURITY_TEST_MODE === 'failure') process.exit(1);
const count = process.env.LOMI_SECURITY_TEST_MODE === 'empty' ? 0 : 1;
console.log('test result: ok. ' + count + ' passed; 0 failed; 0 ignored');
`,
        { mode: 0o700 },
      );
      const run = (mode: string) => {
        writeFileSync(log, "");
        return spawnSync(
          process.execPath,
          [
            fileURLToPath(
              new URL("../scripts/security-regressions.mjs", import.meta.url),
            ),
          ],
          {
            encoding: "utf8",
            timeout: 30000,
            env: {
              ...process.env,
              PATH: directory + delimiter + process.env.PATH,
              LOMI_SECURITY_TEST_LOG: log,
              LOMI_SECURITY_TEST_MODE: mode,
            },
          },
        );
      };
      for (const mode of ["failure", "empty"]) {
        const result = run(mode);
        assert.notEqual(result.status, 0, mode);
        assert.match(
          result.stderr,
          /Security suite failed or selected no tests/,
        );
        assert.equal(
          readFileSync(log, "utf8").trim().split("\n").length,
          1,
          "The failed gate must not advance to later suites.",
        );
      }
      const success = run("success");
      assert.equal(success.status, 0, success.stderr);
      assert.equal(
        readFileSync(log, "utf8").trim().split("\n").length,
        securitySuites(process.platform).length,
      );
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  },
);

test("all platforms gate Remote and authentication without selecting Unix-only control suites on Windows", () => {
  assert.deepEqual(securitySuites("win32"), [
    ["lomi", "files::tests::json_"],
    ["lomi", "auth::"],
    ["lomi", "remote::"],
    ["lomi", "terminal::tests::remote_"],
    ["lomi-remote-crypto", ""],
  ]);
  for (const platform of ["darwin", "linux"]) {
    assert.deepEqual(securitySuites(platform), [
      ["lomi", "files::tests::json_"],
      ["lomi", "auth::"],
      ["lomi", "remote::"],
      ["lomi", "terminal::tests::remote_"],
      ["lomi-control-core", "authentication::tests::"],
      ["lomi-control-core", "project_files::tests::"],
      ["lomi-control-core", "broker::revoke_tests::"],
      ["lomi-remote-crypto", ""],
    ]);
  }
});

test("Windows rejects a failed or empty security suite and never advances past it", async () => {
  const suites = securitySuites("win32");
  for (const failure of ["failed", "empty"]) {
    for (let failureIndex = 0; failureIndex < suites.length; failureIndex++) {
      const selected: string[][] = [];
      await assert.rejects(
        () =>
          runSecurityRegressions("win32", (command: string, args: string[]) => {
            assert.equal(command, "cargo");
            selected.push([
              args[args.indexOf("-p") + 1],
              args.at(-1) === "--tests" ? "" : args.at(-1)!,
            ]);
            const failing = selected.length - 1 === failureIndex;
            return {
              status: failing && failure === "failed" ? 1 : 0,
              stdout: `test result: ok. ${failing && failure === "empty" ? 0 : 1} passed; 0 failed; 0 ignored\n`,
              stderr: "",
            };
          }),
        /Security suite failed or selected no tests/,
      );
      assert.deepEqual(selected, suites.slice(0, failureIndex + 1));
    }
  }
  const selected: string[][] = [];
  await runSecurityRegressions("win32", (_command: string, args: string[]) => {
    selected.push([
      args[args.indexOf("-p") + 1],
      args.at(-1) === "--tests" ? "" : args.at(-1)!,
    ]);
    return {
      status: 0,
      stdout: "test result: ok. 2 passed; 0 failed\n",
      stderr: "",
    };
  });
  assert.deepEqual(selected, suites);
});

test("failed native builds retain terminal diagnostics even with a large compiler log", async () => {
  const stderr =
    "compiler warning\n".repeat(10000) +
    "error: could not compile dependency (signal: 9, SIGKILL)\n";
  await assert.rejects(
    () =>
      runSecurityRegressions("win32", () => ({
        status: 101,
        signal: null,
        stdout: "",
        stderr,
      })),
    (error: Error) => {
      assert.match(error.message, /Exit status: 101; signal: none/);
      assert.match(
        error.message,
        /error: could not compile dependency \(signal: 9, SIGKILL\)/,
      );
      assert.ok(
        error.message.length < 9000,
        "The failure retains a bounded diagnostic tail.",
      );
      return true;
    },
  );
});

test("Windows loader receipts run only for entry-point failures and cannot pass the gate", async () => {
  for (const [platform, status, inspect] of [
    ["win32", 0xc0000139, true],
    ["win32", 101, false],
    ["linux", 0xc0000139, false],
  ] as const) {
    const calls: string[] = [];
    await assert.rejects(
      () =>
        runSecurityRegressions(platform, (command: string, args: string[]) => {
          calls.push(command);
          if (command === "pwsh") {
            assert.ok(args.includes("-NoProfile"));
            assert.ok(
              args[args.indexOf("-TestExecutable") + 1].endsWith(
                "lomi_lib-123.exe",
              ),
            );
            return {
              status: 0,
              stdout: "Import inspection completed\n",
              stderr: "",
            };
          }
          return {
            status,
            stdout: "",
            stderr:
              "Running unittests src/lib.rs (src-tauri/target/debug/deps/lomi_lib-123.exe)\n",
          };
        }),
      /Security suite failed or selected no tests/,
    );
    assert.deepEqual(calls, inspect ? ["cargo", "pwsh"] : ["cargo"]);
  }
});
