#!/usr/bin/env node
// Extracts translation keys from src/ and checks every locale file.
//   node scripts/i18n-check.mjs          report and fail on missing keys
//   node scripts/i18n-check.mjs --keys   print all keys as JSON
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = new URL("..", import.meta.url).pathname;
const src = join(root, "src");
const string = String.raw`"((?:[^"\\]|\\.)*)"`;
const patterns = [
  { re: new RegExp(String.raw`\bt\(\s*${string}`, "g"), kind: "text" },
  { re: new RegExp(String.raw`\bmsg\(\s*${string}`, "g"), kind: "text" },
  {
    re: new RegExp(String.raw`\btp\(\s*[^,]+,\s*${string},\s*${string}`, "g"),
    kind: "plural",
  },
];

function files(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory())
      return name === "locales" ? [] : files(path);
    return /\.(tsx?|mjs)$/.test(name) && !/\.test\./.test(name) ? [path] : [];
  });
}

const keys = new Map();
for (const file of files(src)) {
  const text = readFileSync(file, "utf8");
  for (const { re, kind } of patterns)
    for (const match of text.matchAll(re)) {
      const decode = (value) => JSON.parse(`"${value}"`);
      const key = decode(kind === "plural" ? match[2] : match[1]);
      const entry = keys.get(key) ?? { key, files: new Set() };
      if (kind === "plural") entry.one = decode(match[1]);
      entry.files.add(relative(root, file));
      keys.set(key, entry);
    }
}

if (process.argv.includes("--keys")) {
  console.log(
    JSON.stringify(
      [...keys.values()].map(({ key, one, files }) => ({
        key,
        ...(one ? { one } : {}),
        files: [...files],
      })),
      null,
      2,
    ),
  );
  // exitCode instead of exit(): exit() truncates large output written to a pipe.
} else {
  const placeholders = (text) =>
    [...text.matchAll(/\{(\w+)\}/g)]
      .map((m) => m[1])
      .sort()
      .join();
  const localeDir = join(src, "locales");
  let failed = false;
  for (const name of readdirSync(localeDir).filter((n) =>
    n.endsWith(".json"),
  )) {
    const dictionary = JSON.parse(readFileSync(join(localeDir, name), "utf8"));
    const missing = [...keys.keys()].filter((key) => !(key in dictionary));
    const unused = Object.keys(dictionary).filter((key) => !keys.has(key));
    // Plural forms may omit {count} where the form covers a single number.
    const withoutCount = (text) =>
      placeholders(text)
        .split(",")
        .filter((name) => name && name !== "count")
        .join();
    const broken = Object.entries(dictionary).flatMap(([key, value]) =>
      (typeof value === "string"
        ? [[value, placeholders]]
        : Object.values(value).map((text) => [text, withoutCount])
      )
        .filter(
          ([text, compare]) => keys.has(key) && compare(text) !== compare(key),
        )
        .map(() => key),
    );
    console.log(
      `${name}: ${keys.size - missing.length}/${keys.size} translated` +
        (unused.length ? `, ${unused.length} unused` : "") +
        (broken.length ? `, ${broken.length} with wrong placeholders` : ""),
    );
    for (const key of missing.slice(0, 10)) console.log(`  missing: ${key}`);
    for (const key of [...new Set(broken)].slice(0, 10))
      console.log(`  placeholders: ${key}`);
    if (missing.length || broken.length) failed = true;
  }
  process.exitCode = failed ? 1 : 0;
}
