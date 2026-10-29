import { build } from "esbuild";
import { mkdir, writeFile, readFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
const directory = fileURLToPath(new URL(".", import.meta.url));
const output = new URL(
  "../../src-tauri/resources/ai-runtime/",
  import.meta.url,
);
await mkdir(output, { recursive: true });
for (const name of ["index"]) {
  const outfile = fileURLToPath(new URL(`${name}.cjs`, output));
  const result = await build({
    metafile: true,
    absWorkingDir: directory,
    entryPoints: [`src/${name}.ts`],
    outfile,
    bundle: true,
    platform: "node",
    target: "node24",
    format: "cjs",
    legalComments: "external",
    sourcemap: false,
  });
  if (name === "index") {
    const packages = new Map();
    for (const input of Object.keys(result.metafile.inputs)) {
      if (!input.includes("node_modules")) continue;
      let folder = path.dirname(path.resolve(directory, input));
      while (folder !== path.dirname(folder)) {
        const manifest = await readFile(
          path.join(folder, "package.json"),
          "utf8",
        )
          .then(JSON.parse)
          .catch(() => null);
        if (manifest?.name) {
          const key = `${manifest.name}@${manifest.version}`;
          if (!packages.has(key)) {
            const licenses = (await readdir(folder)).filter((name) =>
              /^(LICENSE|COPYING|NOTICE)(\.|$)/i.test(name),
            );
            const texts = await Promise.all(
              licenses.map((name) => readFile(path.join(folder, name), "utf8")),
            );
            packages.set(
              key,
              `${key} (${manifest.license ?? "See package license"})\n${texts.join("\n")}`,
            );
          }
          break;
        }
        folder = path.dirname(folder);
      }
    }
    await writeFile(
      new URL("THIRD-PARTY-NOTICES", output),
      [...packages.values()].join("\n\n"),
    );
  }
  await writeFile(
    `${outfile}.sha256`,
    createHash("sha256")
      .update(await readFile(outfile))
      .digest("hex") + "\n",
  );
}
