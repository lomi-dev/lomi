import { mkdtemp, cp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { pluginSDKSpec } from "./plugin-sdk-dependency.mjs";
const root = resolve(import.meta.dirname, "..");
const temporary = await mkdtemp(join(tmpdir(), "lomi-sdk-consumer-"));
const env = {
  ...process.env,
  npm_config_store_dir: join(temporary, "store"),
  npm_config_cache: join(temporary, "cache"),
};
delete env.NODE_PATH;
const run = (args, cwd) => {
  const cli = process.env.npm_execpath;
  const result = spawnSync(
    cli ? process.execPath : "pnpm",
    cli ? [cli, ...args] : args,
    { cwd, env, encoding: "utf8", timeout: 180000, maxBuffer: 8 * 1024 * 1024 },
  );
  if (result.status !== 0)
    throw new Error(result.stdout + result.stderr + String(result.error ?? ""));
};
const project = join(temporary, "independent author żółć");
await cp(join(root, "tests/fixtures/context-plugin"), project, {
  recursive: true,
  filter: (path) =>
    !path
      .split(/[\\/]/)
      .some((part) => ["node_modules", "package", "dist"].includes(part)),
});
const path = join(project, "package.json");
const metadata = JSON.parse(await readFile(path, "utf8"));
metadata.dependencies["@lomi-dev/plugin-sdk"] = await pluginSDKSpec(
  root,
  process.argv[2],
);
await writeFile(path, JSON.stringify(metadata, null, 2));
run(["install", "--ignore-scripts"], project);
run(["exec", "tsc", "--noEmit"], project);
run(["build"], project);
await writeFile(
  join(project, "validate.mjs"),
  `import { validatePackage } from '@lomi-dev/plugin-sdk/package'; import { parsePlugin } from '@lomi-dev/plugin-sdk/manifest'; const { manifest } = await validatePackage('package'); parsePlugin(manifest); console.log(manifest.id);`,
);
const result = spawnSync(process.execPath, ["validate.mjs"], {
  cwd: project,
  env,
  encoding: "utf8",
  timeout: 15000,
});
if (result.status !== 0) throw new Error(result.stdout + result.stderr);
console.log(
  `External SDK consumer passed: ${metadata.dependencies["@lomi-dev/plugin-sdk"]}\nIndependent project: ${project}`,
);
