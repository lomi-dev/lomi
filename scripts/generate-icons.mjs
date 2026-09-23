import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { run } from "@tauri-apps/cli";
import "./build-macos-icon.mjs";

const projectRoot = fileURLToPath(new URL("../", import.meta.url));
const source = path.join(projectRoot, "public", "Lomi.icon");
const icons = path.join(projectRoot, "src-tauri", "icons");
const output = path.join(projectRoot, "src-tauri", "target", "desktop-icons");
const actool = execFileSync("xcrun", ["--find", "actool"], {
  encoding: "utf8",
}).trim();
// Xcode's usr/bin/ictool compiles catalogs; Icon Composer's tool exports PNGs.
const composer = path.resolve(
  path.dirname(actool),
  "../../../Applications/Icon Composer.app/Contents/Executables/ictool",
);
if (!existsSync(composer)) {
  throw new Error(
    "Icon Composer is missing from the selected Xcode installation.",
  );
}
mkdirSync(output, { recursive: true });

function exportImage(rendition, destination, size) {
  execFileSync(
    composer,
    [
      source,
      "--export-image",
      "--output-file",
      destination,
      "--platform",
      "macOS",
      "--rendition",
      rendition,
      "--width",
      String(size),
      "--height",
      String(size),
      "--scale",
      "1",
    ],
    { stdio: "inherit" },
  );
}

const master = path.join(output, "app-icon.png");
exportImage("Default", master, 1024);
const generated = path.join(output, "generated");
await run(["icon", master, "--output", generated]);
// Keep actool's macOS fallback; only publish the Windows/Linux desktop assets.
for (const name of [
  "icon.ico",
  "icon.png",
  "32x32.png",
  "64x64.png",
  "128x128.png",
  "128x128@2x.png",
]) {
  copyFileSync(path.join(generated, name), path.join(icons, name));
}
copyFileSync(path.join(generated, "icon.png"), path.join(icons, "512x512.png"));
copyFileSync(master, path.join(projectRoot, "public", "app-icon.png"));

const previews = path.join(output, "previews");
mkdirSync(previews, { recursive: true });
for (const rendition of [
  "Default",
  "Dark",
  "ClearLight",
  "ClearDark",
  "TintedLight",
  "TintedDark",
]) {
  exportImage(rendition, path.join(previews, `${rendition}.png`), 256);
}
console.log(
  `Desktop icons generated from Lomi.icon. Appearance previews: ${previews}`,
);
