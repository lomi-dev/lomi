import assert from "node:assert/strict";
import test from "node:test";
import {
  isWindowsPath,
  joinFilePath,
  normalizePath,
  parentPath,
  pathBasename,
} from "../src/file-path.ts";

test("Unix path operations preserve literal backslashes and drive-shaped filenames", () => {
  const root = "/tmp/project\\name";
  for (const relative of ["a\\b.txt", "C:\\file.txt", "trailing\\"]) {
    assert.equal(normalizePath(relative, root), relative);
    assert.equal(pathBasename(relative, root), relative);
    assert.equal(joinFilePath(root, relative), `${root}/${relative}`);
    assert.equal(parentPath(relative, root), "");
  }
  assert.equal(joinFilePath(root, "dir/file.txt"), `${root}/dir/file.txt`);
  assert.equal(parentPath(`${root}/a\\b.txt`), root);
  assert.equal(pathBasename(`${root}/a\\b.txt`), "a\\b.txt");
  assert.equal(isWindowsPath(root), false);
});

test("Windows relative paths inherit drive, UNC and verbatim root separators", () => {
  for (const root of [
    "C:\\repo",
    "C:/repo",
    "\\\\?\\C:\\repo",
    "\\\\server\\share\\repo",
    "\\\\?\\UNC\\server\\share\\repo",
    "//server/share/repo",
  ]) {
    assert.equal(isWindowsPath(root), true);
    const separator = root.includes("\\") ? "\\" : "/";
    const expected = `${root}${separator}src${separator}file.ts`;
    assert.equal(joinFilePath(root, "src\\file.ts"), expected);
    assert.equal(joinFilePath(root, "src/file.ts"), expected);
    assert.equal(normalizePath("src\\file.ts", root), "src/file.ts");
    assert.equal(parentPath("src\\file.ts", root), "src");
    assert.equal(pathBasename("src\\file.ts", root), "file.ts");
  }
});

test("Windows canonical prefixes compare equally to restored drive and UNC paths", () => {
  assert.equal(normalizePath("\\\\?\\C:\\repo\\file.txt"), "C:/repo/file.txt");
  assert.equal(
    normalizePath("\\\\?\\UNC\\server\\share\\file.txt"),
    "//server/share/file.txt",
  );
  assert.equal(
    normalizePath("//?/unc/server/share/file.txt"),
    "//server/share/file.txt",
  );
  assert.equal(
    normalizePath("/repo/\\\\?\\C:\\literal"),
    "/repo/\\\\?\\C:\\literal",
  );
});
