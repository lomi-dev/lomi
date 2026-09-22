import { spawnSync } from "node:child_process";
import { mkdirSync, readdirSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";

const [javaHome, androidJar, buildTools, output] = process.argv
  .slice(2)
  .map((value) => resolve(value));
if (!javaHome || !androidJar || !buildTools || !output)
  throw Error(
    "Expected JDK home, android.jar, build-tools directory and isolated output directory",
  );
const env = Object.fromEntries(
  Object.entries(process.env).filter(
    ([key]) => !/^(ANDROID_|ADB_|JAVA_|JDK_|_JAVA_)/.test(key),
  ),
);
env.JAVA_HOME = javaHome;
// This fixture key is never used to sign a release or distributed bridge.
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
mkdirSync(output, { recursive: true });
const key = join(output, "fixture.p12");
if (!existsSync(key))
  run(join(javaHome, "bin/keytool"), [
    "-genkeypair",
    "-keystore",
    key,
    "-storepass:env",
    "LOMI_FIXTURE_PASSWORD",
    "-alias",
    "fixture",
    "-keyalg",
    "RSA",
    "-keysize",
    "2048",
    "-validity",
    "365",
    "-dname",
    "CN=Lomi native test fixture",
  ]);
const repository = resolve(import.meta.dirname, "../..");
for (const [name, source] of [
  ["input", "src-tauri/android-input"],
  ["input-test", "tests/native/android-input"],
]) {
  const directory = join(output, name);
  for (const child of ["classes", "dex", "generated"])
    mkdirSync(join(directory, child), { recursive: true });
  const resources = [];
  if (existsSync(join(repository, source, "res"))) {
    run(join(buildTools, "aapt2"), [
      "compile",
      "--dir",
      join(repository, source, "res"),
      "-o",
      join(directory, "resources.zip"),
    ]);
    resources.push(join(directory, "resources.zip"));
  }
  run(join(buildTools, "aapt2"), [
    "link",
    "-I",
    androidJar,
    "--manifest",
    join(repository, source, "AndroidManifest.xml"),
    "--java",
    join(directory, "generated"),
    "-o",
    join(directory, "base.apk"),
    ...resources,
  ]);
  run(join(javaHome, "bin/javac"), [
    "-encoding",
    "UTF-8",
    "--release",
    "8",
    "-classpath",
    androidJar,
    "-d",
    join(directory, "classes"),
    ...files(join(repository, source, "src"), ".java"),
    ...files(join(directory, "generated"), ".java"),
  ]);
  run(join(buildTools, "d8"), [
    "--release",
    "--min-api",
    "26",
    "--lib",
    androidJar,
    "--output",
    join(directory, "dex"),
    ...files(join(directory, "classes"), ".class"),
  ]);
  run(join(javaHome, "bin/jar"), [
    "uf",
    join(directory, "base.apk"),
    "-C",
    join(directory, "dex"),
    "classes.dex",
  ]);
  const apk = join(output, `${name}.apk`);
  run(join(buildTools, "zipalign"), [
    "-f",
    "4",
    join(directory, "base.apk"),
    apk,
  ]);
  run(join(buildTools, "apksigner"), [
    "sign",
    "--ks",
    key,
    "--ks-pass",
    "env:LOMI_FIXTURE_PASSWORD",
    apk,
  ]);
  run(join(buildTools, "apksigner"), [
    "verify",
    "--verbose",
    "--print-certs",
    apk,
  ]);
}
