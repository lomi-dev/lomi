import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { join, resolve } from "node:path";

const [appdir] = process.argv.slice(2);
assert.ok(
  appdir,
  "Usage: node scripts/qualify-appimage-backend.mjs EXTRACTED_APPDIR",
);
const root = resolve(appdir);
const hooks = await readdir(join(root, "apprun-hooks"));
assert.ok(
  hooks.includes("linuxdeploy-plugin-gtk.sh"),
  "Expected the extracted linuxdeploy GTK AppRun hook",
);
// Check the known literal and parameter-default shell assignments, rather than
// interpreting arbitrary shell code. Both defaults can force X11 when unset.
const x11Default = String.raw`\$\{GDK_BACKEND:?[=-](?:x11|"x11"|'x11')\}`;
const forcedX11 = new RegExp(
  `(?:^|[\\s;&|])GDK_BACKEND=(?:x11|"x11"|'x11'|${x11Default}|"${x11Default}")(?=$|[\\s;&|])`,
  "m",
);
for (const file of [
  "AppRun",
  ...hooks.map((hook) => join("apprun-hooks", hook)),
]) {
  const source = await readFile(join(root, file), "utf8");
  assert.ok(source.trim(), `${file} must not be empty`);
  const active = source
    .replace(/\\\r?\n/g, "")
    .split(/\r?\n/)
    .filter((line) => !/^\s*#/.test(line))
    .map((line) => line.replace(/\s+#.*$/, ""))
    .join("\n");
  assert.ok(
    !forcedX11.test(active),
    `${file} forces GDK_BACKEND=x11 instead of preserving backend selection`,
  );
}
console.log("AppImage AppRun and GTK hooks preserve backend selection.");
