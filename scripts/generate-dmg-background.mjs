import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "@playwright/test";

const projectRoot = fileURLToPath(new URL("../", import.meta.url));
const source = path.join(projectRoot, "src-tauri", "dmg", "background.html");
const outputDirectory = path.dirname(source);
const output = path.join(outputDirectory, "background.png");

mkdirSync(outputDirectory, { recursive: true });

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({
    viewport: { width: 720, height: 480 },
    deviceScaleFactor: 1,
  });
  await page.goto(pathToFileURL(source).href, { waitUntil: "load" });
  await page.evaluate(async () => {
    await Promise.all([
      document.fonts.load("700 32px Manrope"),
      document.fonts.load("400 16px Manrope"),
      document.fonts.load("500 12px Manrope"),
    ]);
    await document.fonts.ready;
  });

  const isManropeReady = await page.evaluate(() =>
    document.fonts.check("700 32px Manrope"),
  );
  if (!isManropeReady) {
    throw new Error(
      "Manrope did not load; refusing to render the fallback font.",
    );
  }

  await page.screenshot({ path: output, animations: "disabled" });
} finally {
  await browser.close();
}

console.log(`Generated ${path.relative(projectRoot, output)} (720x480, 1x).`);
