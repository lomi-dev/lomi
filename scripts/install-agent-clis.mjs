// Install reviewed npm archives without launching a CLI or lifecycle hook.
// Run with Node >=22.19: node scripts/install-agent-clis.mjs
// Exact transitive versions and npm integrity values live in the sibling manifest.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { constants } from "node:fs";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  open,
  readFile,
  readdir,
  readlink,
  realpath,
  rename,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const manifestBytes = await readFile(
  new URL("./agent-cli-installations.json", import.meta.url),
);
const manifest = JSON.parse(manifestBytes);
// Formatting does not change archive admission. Both digests derive from the
// current exact manifest data, so changing a pin cannot reuse an older receipt.
const digest = createHash("sha256")
  .update(JSON.stringify(manifest))
  .digest("hex");
const legacyDigest = createHash("sha256")
  .update(JSON.stringify(manifest, null, 2) + "\n")
  .digest("hex");
if (
  manifest.schema !== 1 ||
  process.platform !== manifest.platform ||
  process.arch !== manifest.arch
)
  throw new Error(
    "This installer supports only the reviewed macOS arm64 packages.",
  );
const [major, minor] = process.versions.node.split(".").map(Number);
if (major < 22 || (major === 22 && minor < 19))
  throw new Error("The reviewed CLIs require Node >=22.19.0.");
const home = os.homedir();
const base = path.join(home, ".local/share/lomi-router-clis");
const bin = path.join(home, ".local/bin");
const npm = path.join(bin, "npm");
const uid = process.getuid();
process.umask(0o077);

async function info(file) {
  try {
    return await lstat(file);
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
}
async function directory(file, privateMode = false) {
  const metadata = await info(file);
  if (!metadata) await mkdir(file, { mode: 0o700 });
  const current = await lstat(file);
  if (
    !current.isDirectory() ||
    current.isSymbolicLink() ||
    current.uid !== uid ||
    current.mode & (privateMode ? 0o077 : 0o022)
  )
    throw new Error(`Refusing an unsafe installation directory: ${file}`);
}
// Existing shared parent directories are inspected, never chmodded or replaced.
await directory(path.join(home, ".local"));
await directory(path.join(home, ".local/share"));
await directory(bin);
await directory(base, true);
const resolvedNode = await realpath(process.execPath);
const npmPath = await realpath(npm);
const npmEntry = await lstat(npm);
if (
  (!npmEntry.isSymbolicLink() && !npmEntry.isFile()) ||
  ![0, uid].includes(npmEntry.uid) ||
  (!npmEntry.isSymbolicLink() && npmEntry.mode & 0o022)
)
  throw new Error(`Refusing an unsafe npm entry: ${npm}`);
// Resolve only trusted user/root-owned launch paths, including the npm entry
// and Node interpreter used for the credential-free installer subprocess.
async function admitLauncher(file) {
  for (let ancestor = file; ; ancestor = path.dirname(ancestor)) {
    const metadata = await lstat(ancestor);
    if (
      metadata.isSymbolicLink() ||
      ![0, uid].includes(metadata.uid) ||
      metadata.mode & 0o022 ||
      (ancestor === file ? !metadata.isFile() : !metadata.isDirectory())
    )
      throw new Error(`Refusing an unsafe installer launch path: ${ancestor}`);
    if (ancestor === path.dirname(ancestor)) break;
  }
}
await admitLauncher(resolvedNode);
await admitLauncher(npmPath);
const lockPath = path.join(base, ".install.lock");
const ownerLock = await open(lockPath, "wx", 0o600).catch(() => {
  throw new Error(
    `Another or interrupted installation owns ${lockPath}; preserve it for review.`,
  );
});
const createdLinks = [];
const completed = [];

function localPath(root, relative) {
  if (
    typeof relative !== "string" ||
    path.isAbsolute(relative) ||
    relative
      .split("/")
      .some((part) => part === ".." || part === "." || part === "")
  )
    throw new Error("Invalid path in the installation manifest.");
  return path.join(root, relative);
}
function compatible(values, value) {
  return (
    !values ||
    (!values.includes(`!${value}`) &&
      (values.every((entry) => entry.startsWith("!")) ||
        values.includes(value)))
  );
}
async function regular(file) {
  const metadata = await lstat(file);
  if (!metadata.isFile() || metadata.uid !== uid || metadata.mode & 0o022)
    throw new Error(`Missing or unsafe installation file: ${file}`);
  return metadata;
}
async function readRegular(file) {
  const before = await regular(file);
  const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
  const same = (a, b) =>
    ["dev", "ino", "size", "uid", "mode", "mtimeMs", "ctimeMs"].every(
      (key) => a[key] === b[key],
    );
  try {
    if (!same(before, await handle.stat()))
      throw new Error(`Installation file changed before read: ${file}`);
    const bytes = await handle.readFile();
    if (
      !same(before, await handle.stat()) ||
      !same(before, await regular(file))
    )
      throw new Error(`Installation file changed during read: ${file}`);
    return bytes;
  } finally {
    await handle.close();
  }
}
async function packageDirectory(root, relative) {
  let current = root;
  for (const component of relative.split("/")) {
    current = path.join(current, component);
    await directory(current);
  }
}
async function validate(root, tool) {
  await directory(root, true);
  await regular(path.join(root, "package.json"));
  await regular(path.join(root, "package-lock.json"));
  if (
    !Buffer.from(JSON.stringify(tool.package, null, 2) + "\n").equals(
      await readRegular(path.join(root, "package.json")),
    )
  )
    throw new Error(`Installed package declaration changed: ${tool.cli}`);
  const installedLock = JSON.parse(
    await readRegular(path.join(root, "package-lock.json")),
  );
  if (JSON.stringify(installedLock) !== JSON.stringify(tool.lock))
    throw new Error(`Installed dependency lock changed: ${tool.cli}`);
  for (const [relative, pin] of Object.entries(tool.lock.packages)) {
    if (
      !relative ||
      !compatible(pin.os, process.platform) ||
      !compatible(pin.cpu, process.arch)
    )
      continue;
    if (
      !pin.resolved?.startsWith(manifest.registry) ||
      !pin.integrity?.startsWith("sha512-")
    )
      throw new Error(
        `Dependency lacks official registry integrity: ${relative}`,
      );
    const packageRoot = localPath(root, relative);
    await packageDirectory(root, relative);
    const packageFile = path.join(packageRoot, "package.json");
    await regular(packageFile);
    const installed = JSON.parse(await readRegular(packageFile));
    if (installed.version !== pin.version)
      throw new Error(
        `Missing or mismatched dependency: ${relative}@${pin.version}`,
      );
    if (
      tool.cli === "pi" &&
      installed.name.startsWith("@earendil-works/") &&
      installed.version !== "1.0.1"
    )
      throw new Error(
        `Pi internal dependency is outside its reviewed contract: ${installed.name}`,
      );
  }
  for (const artifact of tool.required) {
    const file = localPath(root, artifact.path);
    await packageDirectory(root, path.dirname(artifact.path));
    const metadata = await regular(file);
    if (
      createHash("sha256")
        .update(await readRegular(file))
        .digest("hex") !== artifact.sha256 ||
      (artifact.executable && !(metadata.mode & 0o100))
    )
      throw new Error(
        `Missing or changed platform/runtime asset: ${artifact.path}`,
      );
  }
}
// Every shipped dependency file is included, rather than guessing the imports
// reachable from an entry point (Pi also dynamically loads runtime chunks).
async function runtimeFiles(root) {
  const files = [];
  async function walk(relative) {
    const absolute = localPath(root, relative);
    const metadata = await lstat(absolute);
    if (metadata.isDirectory()) {
      await directory(absolute);
      for (const name of (await readdir(absolute)).sort())
        await walk(`${relative}/${name}`);
    } else if (metadata.isSymbolicLink()) {
      // npm's local bin links are also recorded and must remain inside the tree.
      if (metadata.uid !== uid)
        throw new Error(`Unsafe runtime link: ${relative}`);
      const target = await readlink(absolute);
      const resolved = path.resolve(path.dirname(absolute), target);
      if (path.isAbsolute(target) || !resolved.startsWith(root + path.sep))
        throw new Error(`Runtime link escapes installation: ${relative}`);
      const canonical = await realpath(absolute);
      if (!canonical.startsWith(root + path.sep))
        throw new Error(
          `Runtime link resolves outside installation: ${relative}`,
        );
      await regular(canonical);
      files.push({ path: relative, link: target });
    } else {
      await regular(absolute);
      files.push({
        path: relative,
        sha256: createHash("sha256")
          .update(await readRegular(absolute))
          .digest("hex"),
        mode: metadata.mode & 0o777,
      });
    }
  }
  await walk("package.json");
  await walk("package-lock.json");
  await walk("node_modules");
  return files;
}
async function compareRuntime(root, expected) {
  if (
    !Array.isArray(expected) ||
    expected.length === 0 ||
    JSON.stringify(await runtimeFiles(root)) !== JSON.stringify(expected)
  )
    throw new Error(
      `Installed runtime closure changed; preserving it: ${root}`,
    );
}
async function receipt(root, tool, files) {
  const temporary = path.join(root, `.lomi-receipt-${process.pid}`);
  const file = await open(temporary, "wx", 0o600);
  try {
    await file.writeFile(
      JSON.stringify({
        schema: 2,
        digest,
        cli: tool.cli,
        version: tool.version,
        files,
      }) + "\n",
    );
    await file.sync();
  } finally {
    await file.close();
  }
  try {
    await rename(temporary, path.join(root, ".lomi-installation.json"));
    const parent = await open(root, "r");
    try {
      await parent.sync();
    } finally {
      await parent.close();
    }
  } finally {
    await rm(temporary, { force: true });
  }
}
async function stageRuntime(tool, useStage) {
  const stage = await mkdtemp(path.join(base, `.${tool.cli}-${tool.version}-`));
  await chmod(stage, 0o700);
  try {
    await writeFile(
      path.join(stage, "package.json"),
      JSON.stringify(tool.package, null, 2) + "\n",
      { mode: 0o600 },
    );
    await writeFile(
      path.join(stage, "package-lock.json"),
      JSON.stringify(tool.lock, null, 2) + "\n",
      { mode: 0o600 },
    );
    const installHome = path.join(stage, ".install-home");
    await mkdir(installHome, { mode: 0o700 });
    for (const config of ["user.npmrc", "global.npmrc"])
      await writeFile(path.join(installHome, config), "", { mode: 0o600 });
    // Forward no credential, shell configuration, npm overrides or agent account data.
    const env = {
      HOME: installHome,
      PATH: `${path.dirname(resolvedNode)}:/usr/bin:/bin:/usr/sbin:/sbin`,
      npm_config_userconfig: path.join(installHome, "user.npmrc"),
      npm_config_globalconfig: path.join(installHome, "global.npmrc"),
      npm_config_cache: path.join(installHome, "cache"),
      npm_config_registry: manifest.registry,
      npm_config_engine_strict: "true",
      npm_config_update_notifier: "false",
      npm_config_ignore_scripts: "true",
    };
    await new Promise((resolve, reject) => {
      const child = spawn(
        resolvedNode,
        [
          npmPath,
          "ci",
          "--ignore-scripts",
          "--include=optional",
          "--no-audit",
          "--no-fund",
        ],
        { cwd: stage, env, stdio: "inherit" },
      );
      child.once("error", reject);
      child.once("exit", (code, signal) =>
        code === 0
          ? resolve()
          : reject(
              new Error(
                `npm installation failed for ${tool.cli}: ${signal ?? code}`,
              ),
            ),
      );
    });
    // node-pty's tarball contains the reviewed prebuild; its helper needs execute
    // permission. Kilo runs directly beside its bundled console/tree-sitter files.
    // esbuild's JS loader resolves its pinned optional platform binary directly.
    for (const artifact of tool.required.filter((item) => item.executable)) {
      const file = localPath(stage, artifact.path);
      await packageDirectory(stage, path.dirname(artifact.path));
      await regular(file);
      await chmod(file, 0o755);
    }
    await validate(stage, tool);
    await rm(installHome, { recursive: true });
    // This baseline comes only from npm's integrity-checked, script-free fresh
    // extraction. Never establish it from the potentially modified live tree.
    await useStage(stage, await runtimeFiles(stage));
  } finally {
    await rm(stage, { recursive: true, force: true });
  }
}
try {
  const plans = manifest.tools.map((tool) => ({
    tool,
    root: path.join(base, `${tool.cli}-${tool.version}`),
    link: path.join(bin, tool.cli),
  }));
  // Fail on every existing foreign or dangling launcher before making changes.
  for (const plan of plans) {
    const expected = localPath(plan.root, plan.tool.entry);
    const link = await info(plan.link);
    if (
      link &&
      (!link.isSymbolicLink() || (await readlink(plan.link)) !== expected)
    )
      throw new Error(
        `Existing launcher is preserved; resolve this conflict explicitly: ${plan.link}`,
      );
    if (await info(plan.root)) {
      await directory(plan.root, true);
      const markerPath = path.join(plan.root, ".lomi-installation.json");
      await regular(markerPath);
      const marker = JSON.parse(await readRegular(markerPath));
      if (
        ![1, 2].includes(marker.schema) ||
        ![digest, legacyDigest].includes(marker.digest) ||
        marker.cli !== plan.tool.cli ||
        marker.version !== plan.tool.version
      )
        throw new Error(
          `Existing installation is not owned by this manifest: ${plan.root}`,
        );
      await validate(plan.root, plan.tool);
      if (marker.schema === 2) {
        await compareRuntime(plan.root, marker.files);
        if (marker.digest !== digest) plan.digestUpgrade = marker.files;
      } else plan.legacy = true;
    } else if (link)
      throw new Error(
        `Owned launcher has no complete installation: ${plan.link}`,
      );
  }
  // Publish launchers only after all three dependency trees and assets are complete.
  for (const plan of plans) {
    if (plan.digestUpgrade) {
      await compareRuntime(plan.root, plan.digestUpgrade);
      await receipt(plan.root, plan.tool, plan.digestUpgrade);
    }
    if (plan.legacy)
      await stageRuntime(plan.tool, async (_stage, files) => {
        await compareRuntime(plan.root, files);
        await receipt(plan.root, plan.tool, files);
      });
    else if (!(await info(plan.root)))
      await stageRuntime(plan.tool, async (stage, files) => {
        await receipt(stage, plan.tool, files);
        if (await info(plan.root))
          throw new Error(
            `Installation destination appeared; preserving it: ${plan.root}`,
          );
        await rename(stage, plan.root);
        const parent = await open(base, "r");
        try {
          await parent.sync();
        } finally {
          await parent.close();
        }
      });
    completed.push(plan);
  }
  for (const plan of completed) {
    const expected = localPath(plan.root, plan.tool.entry);
    if (!(await info(plan.link))) {
      await symlink(expected, plan.link); // exclusive creation: never overwrite
      createdLinks.push({ file: plan.link, target: expected });
    } else if (
      !(await lstat(plan.link)).isSymbolicLink() ||
      (await readlink(plan.link)) !== expected
    )
      throw new Error(
        `Launcher changed during installation; preserving it: ${plan.link}`,
      );
  }
  for (const plan of completed)
    console.log(
      `Installed ${plan.tool.name}@${plan.tool.version}: ${plan.link} -> ${localPath(plan.root, plan.tool.entry)}`,
    );
  console.log(
    "CLI execution and authentication are deferred; package locks and runtime assets are installed.",
  );
} catch (error) {
  // Roll back only launcher links created by this invocation; retain complete
  // owned package trees so a later invocation can safely resume publication.
  for (const link of createdLinks)
    if (
      (await info(link.file))?.isSymbolicLink() &&
      (await readlink(link.file)) === link.target
    )
      await rm(link.file);
  throw error;
} finally {
  await ownerLock.close();
  await rm(lockPath);
}
