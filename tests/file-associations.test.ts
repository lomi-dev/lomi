import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const project = new URL("../", import.meta.url);
const readProjectFile = (path: string) =>
  readFileSync(new URL(path, project), "utf8");

function plistArray(xml: string, key: string): string[] {
  const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(
    `<key>${escapedKey}<\\/key>\\s*<array>([\\s\\S]*?)<\\/array>`,
  ).exec(xml);
  assert.ok(match, `missing plist array ${key}`);
  return [...match[1].matchAll(/<string>(.*?)<\/string>/g)].map(
    ([, value]) => value,
  );
}

test("macOS advertises text and data as alternate document types", () => {
  const plist = readProjectFile("src-tauri/Info.plist");
  const documentType =
    /<key>CFBundleDocumentTypes<\/key>\s*<array>\s*<dict>([\s\S]*?)<\/dict>/.exec(
      plist,
    );
  assert.ok(documentType, "missing CFBundleDocumentTypes entry");
  assert.match(
    documentType[1],
    /<key>CFBundleTypeRole<\/key>\s*<string>Editor<\/string>/,
  );
  assert.match(
    documentType[1],
    /<key>LSHandlerRank<\/key>\s*<string>Alternate<\/string>/,
  );
  assert.deepEqual(plistArray(documentType[1], "LSItemContentTypes"), [
    "public.text",
    "public.data",
  ]);

  const extensions = new Set(
    plistArray(documentType[1], "CFBundleTypeExtensions"),
  );
  const imagePreview = readProjectFile("src/image-preview.ts");
  const previewExtensions = [
    ...imagePreview.matchAll(/^\s+([a-z0-9]+): "image\//gm),
  ].map(([, extension]) => extension);
  for (const extension of previewExtensions)
    assert.ok(
      extensions.has(extension),
      `missing image extension ${extension}`,
    );
  for (const extension of [
    "ts",
    "tsx",
    "rs",
    "py",
    "jsonc",
    "yaml",
    "md",
    "gitignore",
  ])
    assert.ok(extensions.has(extension), `missing text extension ${extension}`);

  const prepareScript = readProjectFile("scripts/prepare-macos-dev.mjs");
  assert.match(prepareScript, /path\.join\(tauriDirectory, "Info\.plist"\)/);
  assert.match(prepareScript, /\.\.\.nativeInfo/);
});

test("Linux desktop entry accepts multiple text, data and image files", () => {
  const config = JSON.parse(readProjectFile("src-tauri/tauri.conf.json"));
  const templatePath = "desktop/lomi.desktop";
  assert.equal(config.bundle.linux.deb.desktopTemplate, templatePath);
  assert.equal(config.bundle.linux.rpm.desktopTemplate, templatePath);

  const template = readProjectFile(`src-tauri/${templatePath}`);
  assert.match(template, /^Exec=\{\{exec\}\} %F$/m);
  assert.match(template, /^StartupWMClass=\{\{exec\}\}$/m);
  const mimeTypeLine = /^MimeType=(.*)$/m.exec(template)?.[1];
  assert.ok(mimeTypeLine, "missing Linux MIME declarations");
  const mimeTypes = new Set(mimeTypeLine.split(";"));
  for (const mimeType of [
    "text/plain",
    "text/markdown",
    "text/x-python",
    "application/octet-stream",
    "application/x-zerosize",
    "application/json",
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/svg+xml",
    "image/avif",
    "image/tiff",
  ])
    assert.ok(mimeTypes.has(mimeType), `missing Linux MIME type ${mimeType}`);
});

test("Windows installers add an explicit generic Open with action", () => {
  const config = JSON.parse(readProjectFile("src-tauri/tauri.conf.json"));
  assert.equal(config.bundle.fileAssociations, undefined);
  assert.equal(
    config.bundle.windows.nsis.installerHooks,
    "windows/file-associations.nsh",
  );
  assert.deepEqual(config.bundle.windows.wix.fragmentPaths, [
    "windows/file-associations.wxs",
  ]);
  assert.deepEqual(config.bundle.windows.wix.componentRefs, [
    "LomiFileAssociations",
  ]);

  const nsis = readProjectFile("src-tauri/windows/file-associations.nsh");
  assert.match(
    nsis,
    /Software\\Classes\\Applications\\lomi\.exe\\shell\\open\\command/,
  );
  assert.match(nsis, /Software\\Classes\\\*\\shell\\dev\.lomi\.desktop\.open/);
  assert.match(nsis, /"Open with Lomi"/);
  assert.match(nsis, /Function un\.RemoveLomiFileActions/);
  assert.match(nsis, /DeleteRegKey \/ifempty/);

  const wix = readProjectFile("src-tauri/windows/file-associations.wxs");
  assert.match(
    wix,
    /Key="Software\\Classes\\Applications\\lomi\.exe\\shell\\open\\command"/,
  );
  assert.match(
    wix,
    /Key="Software\\Classes\\\*\\shell\\dev\.lomi\.desktop\.open"/,
  );
  assert.match(wix, /Value="Open with Lomi"/);
  assert.doesNotMatch(wix, /Action="createAndRemoveOnUninstall"/);
  assert.doesNotMatch(wix, /Root="HKCU"/);
  assert.match(wix, /Root="HKLM"/);
  assert.doesNotMatch(wix, /Key="Software\\Classes\\\.[^"]+"/);
});
