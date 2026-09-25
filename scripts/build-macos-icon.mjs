import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "darwin") {
  throw new Error(
    "Compiling the macOS icon requires macOS and Xcode 26 or later.",
  );
}

const projectRoot = fileURLToPath(new URL("../", import.meta.url));
const iconSource = path.join(projectRoot, "public", "Lomi.icon");
const outputDirectory = path.join(
  projectRoot,
  "src-tauri",
  "target",
  "macos-icon",
);
const fallbackIcon = path.join(projectRoot, "src-tauri", "icons", "icon.icns");
const config = JSON.parse(
  readFileSync(
    path.join(projectRoot, "src-tauri", "tauri.macos.conf.json"),
    "utf8",
  ),
);

function writeIfChanged(destination, contents) {
  if (!existsSync(destination) || !readFileSync(destination).equals(contents)) {
    writeFileSync(destination, contents);
  }
}

mkdirSync(outputDirectory, { recursive: true });
// Compile in isolation so a failed build cannot reuse an older asset catalog.
const stagingDirectory = mkdtempSync(path.join(outputDirectory, "compile-"));
try {
  execFileSync(
    "xcrun",
    [
      "actool",
      iconSource,
      "--compile",
      stagingDirectory,
      "--app-icon",
      "Lomi",
      "--output-partial-info-plist",
      path.join(stagingDirectory, "icon-info.plist"),
      "--platform",
      "macosx",
      "--minimum-deployment-target",
      config.bundle?.macOS?.minimumSystemVersion ?? "10.13",
      "--target-device",
      "mac",
      "--output-format",
      "human-readable-text",
      "--errors",
      "--warnings",
    ],
    { stdio: "inherit" },
  );

  const macosMajorVersion = Number(
    execFileSync("sw_vers", ["-productVersion"], { encoding: "utf8" })
      .trim()
      .split(".")[0],
  );
  if (!Number.isInteger(macosMajorVersion)) {
    throw new Error(
      "Could not determine the macOS version for icon validation.",
    );
  }
  // macOS 15 assetutil cannot inspect the icon stacks compiled by Xcode 26.
  const canInspectLayers = macosMajorVersion >= 26;
  if (canInspectLayers) {
    const catalog = JSON.parse(
      execFileSync(
        "xcrun",
        ["assetutil", "--info", path.join(stagingDirectory, "Assets.car")],
        { encoding: "utf8" },
      ),
    );
    for (const appearance of [
      "NSAppearanceNameAqua",
      "NSAppearanceNameDarkAqua",
      "ISAppearanceTintable",
    ]) {
      if (
        !catalog.some(
          (asset) =>
            asset.Name === "Lomi" &&
            asset.AssetType === "IconImageStack" &&
            asset.Appearance === appearance,
        )
      ) {
        throw new Error(
          `Compiled Lomi icon is missing its layered ${appearance} appearance.`,
        );
      }
    }
  }
  for (const name of ["Assets.car", "Lomi.icns", "icon-info.plist"]) {
    writeIfChanged(
      path.join(outputDirectory, name),
      readFileSync(path.join(stagingDirectory, name)),
    );
  }
  writeIfChanged(
    fallbackIcon,
    readFileSync(path.join(stagingDirectory, "Lomi.icns")),
  );
  console.log(
    canInspectLayers
      ? "macOS icon: default, dark, and clear/tinted layers compiled."
      : "macOS icon: Assets.car and Lomi.icns compiled; layer inspection unavailable on this macOS version.",
  );
} finally {
  rmSync(stagingDirectory, { recursive: true, force: true });
}
