import assert from "node:assert/strict";
import test from "node:test";
import { activateLocale, detectLocale, t, tp } from "./i18n.ts";

test("first supported system language wins, otherwise English", () => {
  assert.equal(detectLocale(["pl-PL", "en-US"]), "pl");
  assert.equal(detectLocale(["fr-FR", "uk_UA"]), "uk");
  assert.equal(detectLocale(["fr-FR", "ja"]), "en");
  assert.equal(detectLocale([]), "en");
});

test("missing translations fall back to English with placeholders", () => {
  activateLocale("pl", { "Add project": "Dodaj projekt" });
  assert.equal(t("Add project"), "Dodaj projekt");
  assert.equal(t("Open {name}", { name: "lomi" }), "Open lomi");
});

test("plural categories follow the active language", () => {
  activateLocale("pl", {
    "{count} files": {
      one: "{count} plik",
      few: "{count} pliki",
      many: "{count} plików",
      other: "{count} pliku",
    },
  });
  const files = (n: number) => tp(n, "{count} file", "{count} files");
  assert.deepEqual([1, 2, 5, 22, 1.5].map(files), [
    "1 plik",
    "2 pliki",
    "5 plików",
    "22 pliki",
    "1.5 pliku",
  ]);
  activateLocale("en", {});
  assert.deepEqual([1, 2].map(files), ["1 file", "2 files"]);
});
