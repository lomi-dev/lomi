import assert from "node:assert/strict";
import { test } from "node:test";
import {
  TextCommitAssembler,
  wellFormedInputText,
} from "../src/android/text-commit.ts";

test("committed text preserves BMP and complete astral characters", () => {
  const commits = new TextCommitAssembler();
  const text = "Zażółć gęślą jaźń — こんにちは 🧪😀";
  assert.equal(commits.append(text), text);
  assert.equal(commits.append("!"), "!");
});

test("split Windows UTF-16 units emit each character once and in order", () => {
  const commits = new TextCommitAssembler();
  const text = "A🧪😀にB";
  assert.equal(
    text
      .split("")
      .map((unit) => commits.append(unit))
      .join(""),
    text,
  );
  assert.equal(commits.append(""), "");
  assert.equal(commits.append("prefix\ud83d"), "prefix");
  assert.equal(commits.append("\ude00suffix"), "😀suffix");
});

test("malformed halves are dropped without replacement or joining across BMP", () => {
  const commits = new TextCommitAssembler();
  assert.equal(commits.append("\ude00A\ud83dB\ude00C"), "ABC");
  assert.equal(commits.append("\ud83d"), "");
  assert.equal(commits.append("x\ude00"), "x");
  assert.equal(commits.append("\ud83d\ud83d\ude00"), "😀");
});

test("reset discards an incomplete character and accepts the next complete text", () => {
  const commits = new TextCommitAssembler();
  assert.equal(commits.append("\ud83d"), "");
  commits.reset();
  assert.equal(commits.append("\ude00"), "");
  assert.equal(commits.append("new😀"), "new😀");
  commits.reset();
  assert.equal(commits.append("に"), "に");
});

test("focus scopes and complete paste/composition strings cannot share halves", () => {
  const previous = new TextCommitAssembler();
  const current = new TextCommitAssembler();
  previous.append("\ud83d");
  assert.equal(current.append("\ude00"), "");
  assert.equal(wellFormedInputText("A\ude00🧪B\ud83d"), "A🧪B");
  assert.equal(wellFormedInputText("\ude00"), "");
  assert.equal(wellFormedInputText("😀"), "😀");
});
