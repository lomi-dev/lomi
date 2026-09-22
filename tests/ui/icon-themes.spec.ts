import { expect, test, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { mockDesktop } from "./desktop";

const file = {
  iconDefinitions: {
    file: { iconPath: "file.svg" },
    folder: { iconPath: "folder.svg" },
    open: { iconPath: "open.svg" },
    ts: { iconPath: "ts.svg" },
    light: { iconPath: "light.svg" },
  },
  file: "file",
  folder: "folder",
  folderExpanded: "open",
  fileExtensions: { ts: "ts" },
  light: { fileExtensions: { ts: "light" } },
  hidesExplorerArrows: true,
};
const product = {
  fonts: [{ id: "icons", src: [{ path: "icons.woff2", format: "woff2" }] }],
  iconDefinitions: {
    search: { fontCharacter: "\ue151" },
    refresh: { fontCharacter: "\ue149" },
    close: { fontCharacter: "\ue1b2" },
    gear: { fontCharacter: "\ue154" },
  },
};
const manifests = {
  files: {
    version: 2,
    name: "Portable files",
    iconTheme: { kind: "file", path: "icons.json" },
  },
  product: {
    version: 2,
    name: "Portable interface",
    iconTheme: { kind: "product", path: "icons.json" },
  },
  broken: {
    version: 2,
    name: "Broken font",
    iconTheme: { kind: "product", path: "icons.json" },
  },
  color: {
    version: 2,
    name: "Portable colors",
    appearance: "light",
    common: { tokens: { "--color-background": "#f4f4f4" } },
  },
};
async function install(page: Page, selected = false) {
  await mockDesktop(page);
  await page.addInitScript(
    ({ manifests, file, product, selected }) => {
      if (!localStorage.getItem("test-theme-manifests")) {
        localStorage.setItem("test-theme-manifests", JSON.stringify(manifests));
        localStorage.setItem(
          "test-icon-themes",
          JSON.stringify({
            files: file,
            product,
            broken: {
              ...product,
              fonts: [
                {
                  id: "icons",
                  src: [{ path: "broken.woff2", format: "woff2" }],
                },
              ],
            },
          }),
        );
        localStorage.setItem(
          "test-theme-settings",
          JSON.stringify({
            version: 1,
            active: null,
            appearance: "dark",
            fileIcons: selected ? "files" : null,
            productIcons: selected ? "product" : null,
          }),
        );
      }
    },
    { manifests, file, product, selected },
  );
  const font = await readFile("themes/icons/lucide.woff2");
  await page.route("**/theme-assets/**", (route) => {
    const path = route.request().url();
    if (path.endsWith("broken.woff2"))
      return route.fulfill({ status: 404, body: "missing" });
    if (path.endsWith(".woff2"))
      return route.fulfill({ contentType: "font/woff2", body: font });
    return route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#739ad1" d="M4 2h10l6 6v14H4z"/><path fill="#a9cdf9" d="M14 2v6h6"/></svg>',
    });
  });
}
const category = (page: Page, name: string) =>
  page
    .getByRole("group", { name: "Theme types" })
    .getByRole("button", { name, exact: true });

test("file and interface selections are independent, survive color changes and export their own kind", async ({
  page,
  context,
}) => {
  await install(page);
  await page.goto("/?window=settings&page=themes");
  await category(page, "File icons").click();
  await page.getByRole("button", { name: "Use Portable files theme" }).click();
  await expect(
    page.getByRole("button", { name: "Use Portable files theme" }),
  ).toHaveAttribute("aria-pressed", "true");
  await category(page, "Interface icons").click();
  await page
    .getByRole("button", { name: "Use Portable interface theme" })
    .click();
  await expect(
    page.locator('[data-product-icon="search"]').first(),
  ).toBeVisible();
  expect(
    await page
      .locator('[data-product-icon="search"] text')
      .first()
      .evaluate((el) => getComputedStyle(el).fontFamily),
  ).toContain("Lomi icons");
  const main = await context.newPage();
  await install(main);
  await main.goto("/");
  await expect(
    main.locator('[data-product-icon="gear"]').first(),
  ).toBeVisible();
  await category(page, "Colors").click();
  await page.getByRole("button", { name: "Use Portable colors theme" }).click();
  await expect(main.locator("html")).toHaveAttribute(
    "data-appearance",
    "light",
  );
  const preferences = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("test-theme-settings")!),
  );
  expect(preferences).toMatchObject({
    active: "color",
    fileIcons: "files",
    productIcons: "product",
  });
  await category(page, "Interface icons").click();
  await page
    .getByRole("article", { name: "Portable interface", exact: true })
    .getByRole("button", { name: "Export to VS Code" })
    .click();
  await expect(page.locator(".theme-status")).toContainText(
    "Install from VSIX",
  );
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls
          .filter((c: any) => c.command === "export_vscode_icon_theme")
          .at(-1).args,
    ),
  ).toMatchObject({ id: "product", kind: "product" });
  await page
    .getByRole("button", { name: "Use Lomi interface icons theme" })
    .click();
  await expect(main.locator("[data-product-icon]")).toHaveCount(0);
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("test-theme-settings")!).fileIcons,
    ),
  ).toBe("files");
});

test("resource icons follow Explorer folders, file tabs and appearance changes", async ({
  page,
}, testInfo) => {
  await install(page, true);
  await page.goto("/");
  await expect(
    page.locator('.project-tree-heading [data-file-icon="open"]'),
  ).toBeVisible();
  const folder = page.locator('.tree-entry[title="/project/src"]');
  await expect(folder.locator('[data-file-icon="folder"]')).toBeVisible();
  await folder.click();
  await expect(folder.locator('[data-file-icon="open"]')).toBeVisible();
  const fileRow = page.locator('.tree-entry[title="/project/src/main.ts"]');
  await expect(fileRow.locator('[data-file-icon="ts"]')).toBeVisible();
  await expect(folder.locator("svg").first()).toHaveCSS("visibility", "hidden");
  await fileRow.click();
  await expect(page.locator('.tab [data-file-icon="ts"]')).toBeVisible();
  await expect(
    page.locator('.editor-heading [data-file-icon="ts"]'),
  ).toBeVisible();
  await page.evaluate(async () => {
    const data = JSON.parse(localStorage.getItem("test-theme-settings")!);
    data.appearance = "light";
    await (window as any).__TAURI_INTERNALS__.invoke("save_theme_preferences", {
      data,
    });
  });
  await expect(page.locator('.tab [data-file-icon="light"]')).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("icon-workbench.png") });
});

test("invalid fonts leave the last working icons and saved selection intact", async ({
  page,
}, testInfo) => {
  await install(page, true);
  await page.goto("/?window=settings&page=themes");
  await category(page, "Interface icons").click();
  await expect(
    page.locator('[data-product-icon="search"]').first(),
  ).toBeVisible();
  await page.getByRole("button", { name: "Use Broken font theme" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(
    page.locator('[data-product-icon="search"]').first(),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        JSON.parse(localStorage.getItem("test-theme-settings")!).productIcons,
    ),
  ).toBe("product");
  await page.setViewportSize({ width: 600, height: 600 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("icon-settings.png") });
});

test("canceled font staging cannot publish or retain its fonts", async ({
  page,
}) => {
  await install(page, true);
  await page.goto("/?window=settings&page=themes");
  await expect(
    page.locator('[data-product-icon="search"]').first(),
  ).toBeVisible();
  const font = await readFile("themes/icons/lucide.woff2");
  let requested = false;
  await page.route("**/slow.woff2", async (route) => {
    requested = true;
    await new Promise((resolve) => setTimeout(resolve, 200));
    await route.fulfill({ contentType: "font/woff2", body: font });
  });
  const result = await page.evaluate(async (product) => {
    const { prepareTheme, effectiveTheme } =
      await import("/src/theme/runtime.ts");
    const before = effectiveTheme();
    const families = () =>
      [...document.fonts].map((face) => face.family).sort();
    const previous = families();
    const controller = new AbortController();
    const pending = prepareTheme(
      null,
      { version: 1, active: null, appearance: "dark" },
      controller.signal,
      undefined,
      {
        file: null,
        product: {
          id: "slow",
          raw: JSON.stringify({
            version: 2,
            name: "Slow",
            iconTheme: { kind: "product", path: "icons.json" },
          }),
          revision: "1",
          directory: "/slow",
          iconTheme: {
            ...product,
            fonts: [
              { id: "icons", src: [{ path: "slow.woff2", format: "woff2" }] },
            ],
          },
        },
      },
    );
    setTimeout(() => controller.abort(), 50);
    const error = await pending.then(
      (prepared) => {
        prepared.dispose();
        return "Unexpected completion";
      },
      (error) => String(error),
    );
    await new Promise((resolve) => setTimeout(resolve, 300));
    return {
      error,
      unchanged: effectiveTheme().productIcons === before.productIcons,
      previous,
      current: families(),
    };
  }, product);
  expect(requested).toBe(true);
  expect(result.error).toContain("canceled");
  expect(result.unchanged).toBe(true);
  expect(result.current).toEqual(result.previous);
});

test("undecodable current-color masks fall back to a visible built-in file icon", async ({
  page,
}) => {
  await install(page, true);
  await page.addInitScript(() => {
    const data = JSON.parse(localStorage.getItem("test-icon-themes")!);
    data.files.usesCurrentColor = true;
    data.files.iconDefinitions.file.iconPath = "invalid.svg";
    localStorage.setItem("test-icon-themes", JSON.stringify(data));
  });
  await page.route("**/invalid.svg", (route) =>
    route.fulfill({ contentType: "image/svg+xml", body: "invalid SVG" }),
  );
  await page.goto("/");
  await expect(
    page.locator('.tree-entry[title="/project/README.md"] svg.lucide-file'),
  ).toBeVisible();
  await expect(
    page.locator('.tree-entry[title="/project/README.md"] [data-file-icon]'),
  ).toHaveCount(0);
});
