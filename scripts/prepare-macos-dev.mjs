import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  linkSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  unlinkSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../", import.meta.url));
const tauriDirectory = path.join(projectRoot, "src-tauri");
const config = JSON.parse(
  readFileSync(path.join(tauriDirectory, "tauri.conf.json"), "utf8"),
);
const macConfig = JSON.parse(
  readFileSync(path.join(tauriDirectory, "tauri.macos.conf.json"), "utf8"),
);
const binary = realpathSync(process.argv[2]);
const contents = path.join(
  path.dirname(binary),
  "dev-bundle",
  `${config.productName}.app`,
  "Contents",
);
const resources = path.join(contents, "Resources");
const executable = path.join(contents, "MacOS", path.basename(binary));
const compiledIcons = path.join(tauriDirectory, "target", "macos-icon");

mkdirSync(resources, { recursive: true });
mkdirSync(path.dirname(executable), { recursive: true });
if (existsSync(executable)) unlinkSync(executable);
// A hard link keeps the executable inside the bundle without copying each build.
linkSync(binary, executable);
const helper = path.join(path.dirname(binary), "lomi-mcp");
const bundledHelper = path.join(path.dirname(executable), "lomi-mcp");
if (existsSync(bundledHelper)) unlinkSync(bundledHelper);
if (existsSync(helper)) linkSync(helper, bundledHelper);
copyFileSync(
  path.join(compiledIcons, "Assets.car"),
  path.join(resources, "Assets.car"),
);
copyFileSync(
  path.join(tauriDirectory, "icons", "icon.icns"),
  path.join(resources, "icon.icns"),
);

const nativeInfo = JSON.parse(
  execFileSync(
    "plutil",
    ["-convert", "json", "-o", "-", path.join(tauriDirectory, "Info.plist")],
    { encoding: "utf8" },
  ),
);
execFileSync(
  "plutil",
  ["-convert", "xml1", "-o", path.join(contents, "Info.plist"), "-"],
  {
    input: JSON.stringify({
      CFBundleDevelopmentRegion: "English",
      CFBundleDisplayName: config.productName,
      CFBundleExecutable: path.basename(binary),
      CFBundleIconFile: "icon.icns",
      CFBundleIdentifier: config.identifier,
      CFBundleInfoDictionaryVersion: "6.0",
      CFBundleName: config.productName,
      CFBundlePackageType: "APPL",
      CFBundleShortVersionString: config.version,
      CFBundleVersion: config.version,
      LSMinimumSystemVersion:
        macConfig.bundle?.macOS?.minimumSystemVersion ?? "10.13",
      NSHighResolutionCapable: true,
      ...nativeInfo,
    }),
  },
);

console.log(executable);
