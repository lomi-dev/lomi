// Regenerates src-tauri/crates/lomi-stt/catalog.json from @handy-computer/models 0.2.3.
// Usage: node scripts/stt-catalog.mjs [--check] [path/to/catalog.json]
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const TARBALL =
  "https://registry.npmjs.org/@handy-computer/models/-/models-0.2.3.tgz";
const OUTPUT = fileURLToPath(
  new URL("../src-tauri/crates/lomi-stt/catalog.json", import.meta.url),
);
const IDS = [
  "parakeet-ultra",
  "parakeet-unified-en-0.6b",
  "canary-1b-v2",
  "whisper-large-v3-turbo",
  "whisper-large-v3",
  "whisper-small",
  "silero-vad-v6.2",
];
const LICENSES = new Set(["apache-2.0", "mit", "cc-by-4.0"]);

async function loadSource(path) {
  if (path) return JSON.parse(readFileSync(path, "utf8"));
  const dir = mkdtempSync(join(tmpdir(), "stt-catalog-"));
  try {
    const response = await fetch(TARBALL);
    assert.ok(response.ok, `Download failed: ${response.status}`);
    const archive = join(dir, "models.tgz");
    writeFileSync(archive, Buffer.from(await response.arrayBuffer()));
    execFileSync("tar", ["-xzf", archive, "-C", dir]);
    return JSON.parse(
      readFileSync(join(dir, "package", "catalog.json"), "utf8"),
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const seconds = (factor, what) => {
  assert.ok(Number.isFinite(factor) && factor > 0, `Missing ${what}`);
  return Math.round((30 / factor) * 10) / 10;
};

function build(source) {
  return IDS.map((id) => {
    const model = source.models.find((m) => m.id === id);
    assert.ok(model, `Missing model: ${id}`);
    const vad = id === "silero-vad-v6.2";
    assert.equal(model.role, vad ? "vad" : "asr", `Role of ${id}`);
    const file = model.files?.[vad ? "F32" : model.quant];
    assert.ok(file, `Missing file for ${id}`);
    assert.match(model.revision, /^[0-9a-f]{40}$/, `Revision of ${id}`);
    assert.match(file.sha256, /^[0-9a-f]{64}$/, `sha256 of ${id}`);
    assert.ok(
      Number.isSafeInteger(file.size) && file.size > 0,
      `Size of ${id}`,
    );
    assert.match(file.filename, /^[^/\\]+\.gguf$/, `Filename of ${id}`);
    assert.ok(
      LICENSES.has(model.license),
      `License of ${id}: ${model.license}`,
    );
    const entry = {
      id,
      name: model.name,
      role: model.role,
      repository: model.repository,
      revision: model.revision,
      filename: file.filename,
      size: file.size,
      sha256: file.sha256,
      license: model.license,
      languages: model.languages,
      streaming: model.capabilities.streaming,
      translate: model.capabilities.translate,
      languageDetection: model.capabilities.languageDetection,
    };
    if (vad) return entry;
    const result = source.results?.[id];
    assert.ok(result, `Missing results for ${id}`);
    entry.errorPl = result.accuracy?.pl?.error ?? null;
    entry.errorEn = result.accuracy?.en?.error ?? null;
    entry.cpuSecondsPer30s = seconds(result.performance?.cpu, `cpu of ${id}`);
    entry.gpuSecondsPer30s = seconds(
      result.performance?.accelerated,
      `accelerated of ${id}`,
    );
    return entry;
  });
}

const args = process.argv.slice(2);
const check = args.includes("--check");
const output = `${JSON.stringify(build(await loadSource(args.find((a) => !a.startsWith("--")))), null, 2)}\n`;
if (check) {
  assert.equal(readFileSync(OUTPUT, "utf8"), output, "catalog.json is stale");
} else {
  writeFileSync(OUTPUT, output);
}
