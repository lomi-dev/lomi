import assert from "node:assert/strict";
import test from "node:test";
import { formatSize } from "./attachment-size.ts";

test("formatSize picks a compact unit", () => {
  assert.equal(formatSize(0), "0 B");
  assert.equal(formatSize(1023), "1023 B");
  assert.equal(formatSize(1536), "1.5 KB");
  assert.equal(formatSize(200 * 1024), "200 KB");
  assert.equal(formatSize(1.4 * 1024 * 1024), "1.4 MB");
  assert.equal(formatSize(10 * 1024 * 1024), "10 MB");
});
