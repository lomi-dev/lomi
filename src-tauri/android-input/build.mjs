import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
  copyFileSync,
} from "node:fs";
import { join, resolve, relative } from "node:path";

// Maintainer-only build. The desktop application ships the verified APK and
// never invokes Java, Node or Android build tools to build the keyboard.
const args = process.argv.slice(2);
if (args.length !== 6)
  throw Error(
    "Expected JDK home, android.jar, build-tools directory, output directory, keystore and key alias. Set LOMI_INPUT_KEY_PASSWORD separately.",
  );
const [javaHome, androidJar, buildTools, output, key] = args
  .slice(0, 5)
  .map((arg) => resolve(arg));
const alias = args[5];
if (!process.env.LOMI_INPUT_KEY_PASSWORD)
  throw Error("Missing signing key password.");
const env = Object.fromEntries(
  Object.entries(process.env).filter(
    ([key]) => !/^(ANDROID_|ADB_|JAVA_|JDK_|_JAVA_)/.test(key),
  ),
);
env.JAVA_HOME = javaHome;
function run(program, args) {
  const result = spawnSync(program, args, {
    env,
    encoding: "utf8",
    timeout: 120000,
    maxBuffer: 1024 * 1024,
    windowsHide: true,
  });
  if (result.error || result.status !== 0)
    throw result.error ?? Error(`${program}: ${result.stderr}`);
  return result.stdout || result.stderr;
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
const source = import.meta.dirname;
for (const child of ["classes", "dex", "generated"])
  mkdirSync(join(output, child), { recursive: true });
run(join(buildTools, "aapt2"), [
  "compile",
  "--dir",
  join(source, "res"),
  "-o",
  join(output, "resources.zip"),
]);
run(join(buildTools, "aapt2"), [
  "link",
  "-I",
  androidJar,
  "--manifest",
  join(source, "AndroidManifest.xml"),
  "--java",
  join(output, "generated"),
  "-o",
  join(output, "base.apk"),
  join(output, "resources.zip"),
]);
run(join(javaHome, "bin/javac"), [
  "-encoding",
  "UTF-8",
  "--release",
  "8",
  "-classpath",
  androidJar,
  "-d",
  join(output, "classes"),
  ...files(join(source, "src"), ".java"),
  ...files(join(output, "generated"), ".java"),
]);
run(join(javaHome, "bin/java"), [
  "-cp",
  join(buildTools, "lib/d8.jar"),
  "com.android.tools.r8.D8",
  "--release",
  "--min-api",
  "26",
  "--lib",
  androidJar,
  "--output",
  join(output, "dex"),
  ...files(join(output, "classes"), ".class"),
]);
run(join(javaHome, "bin/jar"), [
  "uf",
  join(output, "base.apk"),
  "-C",
  join(output, "dex"),
  "classes.dex",
]);
const apk = join(output, "lomi-input.apk");
run(join(buildTools, "zipalign"), ["-f", "4", join(output, "base.apk"), apk]);
const signer = ["-jar", join(buildTools, "lib/apksigner.jar")];
run(join(javaHome, "bin/java"), [
  ...signer,
  "sign",
  "--ks",
  key,
  "--ks-key-alias",
  alias,
  "--ks-pass",
  "env:LOMI_INPUT_KEY_PASSWORD",
  apk,
]);
const certificate = run(join(javaHome, "bin/java"), [
  ...signer,
  "verify",
  "--verbose",
  "--print-certs",
  apk,
]);
const fingerprints = [
  ...certificate.matchAll(/certificate SHA-256 digest: ([a-f0-9]{64})$/gm),
].map((match) => match[1]);
const [certificateSha256] = [...new Set(fingerprints)];
if (!certificateSha256 || new Set(fingerprints).size !== 1)
  throw Error("Missing or ambiguous verified signing certificate fingerprint.");
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const sources = [
  join(source, "AndroidManifest.xml"),
  ...files(join(source, "res"), ".xml"),
  ...files(join(source, "src"), ".java"),
].sort();
const provenance = {
  version: 1,
  package: "org.lomi.input",
  versionCode: Number(
    readFileSync(join(source, "AndroidManifest.xml"), "utf8").match(
      /android:versionCode="(\d+)"/,
    )[1],
  ),
  minApi: 26,
  targetApi: 36,
  apkSha256: sha256(readFileSync(apk)),
  certificateSha256,
  sources: Object.fromEntries(
    sources.map((file) => [
      relative(source, file).replaceAll("\\", "/"),
      sha256(readFileSync(file)),
    ]),
  ),
  tools: {
    javac: run(join(javaHome, "bin/javac"), ["-version"]).trim(),
    aapt2: run(join(buildTools, "aapt2"), ["version"]).trim(),
    d8: run(join(javaHome, "bin/java"), [
      "-cp",
      join(buildTools, "lib/d8.jar"),
      "com.android.tools.r8.D8",
      "--version",
    ]).trim(),
    androidJarSha256: sha256(readFileSync(androidJar)),
  },
};
copyFileSync(apk, join(source, "lomi-input.apk"));
writeFileSync(
  join(source, "artifact.json"),
  JSON.stringify(provenance, null, 2) + "\n",
);
console.log(JSON.stringify(provenance, null, 2));
