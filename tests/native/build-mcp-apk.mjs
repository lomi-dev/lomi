import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { createHash } from "node:crypto";

// The runner stages this script and public fixture sources inside the approved
// project. MCP must execute it in Lomi's PTY before the APK can be imported.
const project = import.meta.dirname;
const config = JSON.parse(
  readFileSync(join(project, "android-build.json"), "utf8"),
);
const { javaHome, androidJar, buildTools, fixtureKey } = config;
for (const value of [javaHome, androidJar, buildTools, fixtureKey])
  if (typeof value !== "string" || !value.startsWith("/"))
    throw Error("Missing fixture toolchain path");
const env = Object.fromEntries(
  Object.entries(process.env).filter(
    ([key]) => !/^(ANDROID_|ADB_|JAVA_|JDK_|_JAVA_)/.test(key),
  ),
);
env.JAVA_HOME = javaHome;
env.LOMI_FIXTURE_PASSWORD = "isolated-native-fixture";
function run(program, args) {
  const result = spawnSync(program, args, {
    env,
    stdio: "inherit",
    timeout: 120000,
  });
  if (result.error || result.status !== 0)
    throw result.error ?? Error(`${program} exited ${result.status}`);
}
function files(directory, suffix) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory()
      ? files(join(directory, entry.name), suffix)
      : entry.name.endsWith(suffix)
        ? [join(directory, entry.name)]
        : [],
  );
}
const differentSigner = process.argv.slice(2).includes("--different-signer");
if (process.argv.slice(2).some((arg) => arg !== "--different-signer"))
  throw Error("Unknown fixture build option");
const relativePath = differentSigner
  ? "mcp-incompatible.apk"
  : "mcp-input-test.apk";
const temporary = mkdtempSync(join(project, ".mcp-apk-build-"));
try {
  let signingKey = fixtureKey;
  if (differentSigner) {
    signingKey = join(temporary, "different-fixture.p12");
    run(join(javaHome, "bin/keytool"), [
      "-genkeypair",
      "-keystore",
      signingKey,
      "-storepass:env",
      "LOMI_FIXTURE_PASSWORD",
      "-alias",
      "fixture",
      "-keyalg",
      "RSA",
      "-keysize",
      "2048",
      "-validity",
      "2",
      "-dname",
      "CN=Lomi incompatible test fixture",
    ]);
  }
  for (const name of ["classes", "dex", "generated"])
    mkdirSync(join(temporary, name));
  run(join(buildTools, "aapt2"), [
    "link",
    "-I",
    androidJar,
    "--manifest",
    join(project, "AndroidManifest.xml"),
    "--java",
    join(temporary, "generated"),
    "-o",
    join(temporary, "base.apk"),
  ]);
  run(join(javaHome, "bin/javac"), [
    "-encoding",
    "UTF-8",
    "--release",
    "8",
    "-classpath",
    androidJar,
    "-d",
    join(temporary, "classes"),
    join(project, "InputTest.java"),
    ...files(join(temporary, "generated"), ".java"),
  ]);
  run(join(buildTools, "d8"), [
    "--release",
    "--min-api",
    "26",
    "--lib",
    androidJar,
    "--output",
    join(temporary, "dex"),
    ...files(join(temporary, "classes"), ".class"),
  ]);
  run(join(javaHome, "bin/jar"), [
    "uf",
    join(temporary, "base.apk"),
    "-C",
    join(temporary, "dex"),
    "classes.dex",
  ]);
  const apk = resolve(project, relativePath);
  run(join(buildTools, "zipalign"), [
    "-f",
    "4",
    join(temporary, "base.apk"),
    apk,
  ]);
  run(join(buildTools, "apksigner"), [
    "sign",
    "--ks",
    signingKey,
    "--ks-pass",
    "env:LOMI_FIXTURE_PASSWORD",
    apk,
  ]);
  run(join(buildTools, "apksigner"), ["verify", apk]);
  const bytes = readFileSync(apk);
  console.log(
    "LOMI_APK_RESULT=" +
      JSON.stringify({
        relativePath,
        byteLength: bytes.length,
        sha256: createHash("sha256").update(bytes).digest("hex"),
      }),
  );
} finally {
  rmSync(temporary, { recursive: true });
}
