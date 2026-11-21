import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "playwright";

const projectRoot = fileURLToPath(new URL("../", import.meta.url));
const source = path.join(projectRoot, "src-tauri", "dmg", "background.html");
const outputDirectory = path.dirname(source);
const output = path.join(outputDirectory, "background.tiff");
const scratch = mkdtempSync(path.join(tmpdir(), "lomi-dmg-"));

mkdirSync(outputDirectory, { recursive: true });

const browser = await chromium.launch({ headless: true });
try {
  // Finder picks the 2x representation of a multi-resolution TIFF on Retina displays.
  for (const scale of [1, 2]) {
    const page = await browser.newPage({
      viewport: { width: 720, height: 480 },
      deviceScaleFactor: scale,
    });
    await page.goto(pathToFileURL(source).href, { waitUntil: "load" });
    await page.evaluate(async () => {
      await document.fonts.load('620 32px "Mona Sans"');
      await document.fonts.ready;
    });
    const areFontsReady = await page.evaluate(() =>
      document.fonts.check('620 32px "Mona Sans"'),
    );
    if (!areFontsReady) {
      throw new Error(
        "Mona Sans did not load; refusing to render the fallback font.",
      );
    }
    await page.screenshot({
      path: path.join(
        scratch,
        scale === 1 ? "background.png" : "background@2x.png",
      ),
      animations: "disabled",
    });
    await page.close();
  }
} finally {
  await browser.close();
}

try {
  execFileSync("tiffutil", [
    "-cathidpicheck",
    path.join(scratch, "background.png"),
    path.join(scratch, "background@2x.png"),
    "-out",
    output,
  ]);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}

console.log(
  `Generated ${path.relative(projectRoot, output)} (720x480 with 2x).`,
);
