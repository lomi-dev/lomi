import { readFile, writeFile, copyFile, mkdir } from "node:fs/promises";
import { resolve, join } from "node:path";
import {
  productIcons,
  contextProductIcons,
} from "../src/theme/product-icons.ts";

// Pass an extracted lucide-static@1.41.0 package. No network or package scripts run.
const source = process.argv[2];
if (!source)
  throw Error(
    "Usage: node --experimental-strip-types scripts/update-builtin-icon-themes.mjs <lucide-static package directory>",
  );
const output = resolve(import.meta.dirname, "../themes/icons");
await mkdir(output, { recursive: true });
const info = JSON.parse(await readFile(join(source, "font/info.json"), "utf8"));
const iconDefinitions = {};
const glyphs = {};
for (const [name, ids] of Object.entries(productIcons)) {
  const key = name
    .replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)
    .replace(/^-/, "")
    .replace(/([a-z])(\d)/g, "$1-$2");
  if (!info[key]) throw Error(`Missing Lucide icon: ${key}`);
  glyphs[name] = {
    fontCharacter: String.fromCodePoint(
      parseInt(info[key].encodedCode.slice(1), 16),
    ),
  };
  for (const id of ids) iconDefinitions[id] ??= glyphs[name];
}
for (const [id, name] of Object.entries(contextProductIcons))
  iconDefinitions[id] = glyphs[name];
await writeFile(
  join(output, "product.json"),
  JSON.stringify(
    {
      fonts: [
        {
          id: "lomi-lucide",
          src: [{ path: "lucide.woff2", format: "woff2" }],
          weight: "normal",
          style: "normal",
        },
      ],
      iconDefinitions,
    },
    null,
    2,
  ) + "\n",
);
await copyFile(join(source, "font/lucide.woff2"), join(output, "lucide.woff2"));
await copyFile(join(source, "LICENSE"), join(output, "LICENSE.txt"));
for (const name of ["file", "folder", "folder-open"])
  await copyFile(
    join(source, `icons/${name}.svg`),
    join(output, `${name}.svg`),
  );
await writeFile(
  join(output, "file.json"),
  JSON.stringify(
    {
      iconDefinitions: {
        file: { iconPath: "file.svg" },
        folder: { iconPath: "folder.svg" },
        open: { iconPath: "folder-open.svg" },
      },
      file: "file",
      folder: "folder",
      folderExpanded: "open",
      usesCurrentColor: true,
    },
    null,
    2,
  ) + "\n",
);
