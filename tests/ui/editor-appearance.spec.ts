import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { newProject, newSession, openFileTab } from "../../src/model";
import { mockDesktop } from "./desktop";

const source = `// Keep each workspace ready for the next session.
interface Workspace {
  name: string;
  tabs: number;
}

export function describeWorkspace(workspace: Workspace): string {
  const label = "Ready to build";
  const limit = 12;

  if (workspace.tabs > limit) {
    return label;
  }

  return workspace.name;
}
`;

async function openEditor(
  page: Page,
  relative = "workspace.ts",
  content = source,
) {
  const project = newProject("/project", "local:bash");
  const session = openFileTab(
    { ...newSession(), projects: [project], activeProjectId: project.id },
    project.workspaces[0].id,
    "/project",
    relative,
  );
  await mockDesktop(page, false, session, undefined, {
    [`/project/${relative}`]: {
      content,
      revision: "initial",
      encoding: "utf8",
      readOnly: false,
    },
  });
  await page.goto("/");
  await expect(page.locator(".cm-content")).toBeVisible();
  await expect(page.locator(".cm-line span[class]").first()).toBeVisible();
}

async function selectText(page: Page, from: number, to: number) {
  await page.locator(".cm-editor").evaluate(
    async (element, { from, to }) => {
      const moduleUrl = performance
        .getEntriesByType("resource")
        .map((entry) => entry.name)
        .find((url) => url.includes("/@codemirror_view.js"))!;
      const { EditorView } = await import(moduleUrl);
      const view = EditorView.findFromDOM(element)!;
      view.focus();
      view.dispatch({ selection: { anchor: from, head: to } });
    },
    { from, to },
  );
  await expect(page.locator(".cm-selectionBackground").first()).toBeVisible();
}

for (const name of ["Lomi", "DeepMono"] as const)
  for (const appearance of ["dark", "light"] as const) {
    test(`${name} ${appearance} keeps selected syntax readable`, async ({
      page,
    }, testInfo) => {
      await page.emulateMedia({ colorScheme: appearance });
      if (name === "DeepMono")
        await page.addInitScript(() => {
          localStorage.setItem(
            "test-theme-settings",
            JSON.stringify({
              version: 1,
              active: "@builtin-deepmono",
              appearance: "system",
            }),
          );
        });
      await openEditor(page);
      await selectText(page, source.indexOf("export"), source.length);
      const content = page.locator(".cm-content");
      expect(
        await content.evaluate(
          (element) => getComputedStyle(element, "::selection").color,
        ),
      ).toBe(
        await content.evaluate((element) => getComputedStyle(element).color),
      );
      await page.screenshot({
        path: testInfo.outputPath(`editor-${appearance}.png`),
      });

      const styles = await page.locator(".cm-content").evaluate((content) => {
        // Resolve CSS color-mix values through a canvas before measuring contrast.
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = 1;
        const context = canvas.getContext("2d")!;
        const rgba = (color: string) => {
          context.clearRect(0, 0, 1, 1);
          context.fillStyle = color;
          context.fillRect(0, 0, 1, 1);
          return Array.from(context.getImageData(0, 0, 1, 1).data);
        };
        const luminance = (color: number[]) =>
          color.slice(0, 3).reduce((sum, value, index) => {
            const channel = value / 255;
            return (
              sum +
              (channel <= 0.04045
                ? channel / 12.92
                : ((channel + 0.055) / 1.055) ** 2.4) *
                [0.2126, 0.7152, 0.0722][index]
            );
          }, 0);
        const selection = rgba(
          getComputedStyle(
            content
              .closest(".cm-editor")!
              .querySelector(".cm-selectionBackground")!,
          ).backgroundColor,
        );
        const background = rgba(
          getComputedStyle(content.closest(".cm-editor")!).backgroundColor,
        );
        return Array.from(
          content.querySelectorAll(".cm-line, .cm-line span"),
        ).map((element) => {
          const style = getComputedStyle(element);
          const selected = getComputedStyle(element, "::selection");
          const color = rgba(style.color);
          const foreground = rgba(selected.color);
          const contrast = (foreground: number[], against: number[]) => {
            const a = luminance(foreground);
            const b = luminance(against);
            return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
          };
          return {
            text: `${element.textContent}: ${style.color} / ${selected.color}`,
            color,
            nativeBackground: rgba(selected.backgroundColor),
            contrast: Math.min(
              contrast(foreground, selection),
              contrast(foreground, background),
              contrast(color, selection),
              contrast(color, background),
            ),
          };
        });
      });
      for (const style of styles) {
        expect(style.nativeBackground[3], style.text ?? "").toBe(0);
        expect(style.contrast, style.text ?? "").toBeGreaterThanOrEqual(4.5);
      }
      expect(
        new Set(styles.map((style) => style.color.join(","))).size,
      ).toBeGreaterThanOrEqual(5);

      const focused = await page
        .locator(".cm-selectionBackground")
        .first()
        .evaluate((element) => getComputedStyle(element).backgroundColor);
      await page
        .getByRole("button", { name: "Change language mode", exact: true })
        .focus();
      await expect(page.locator(".cm-editor")).not.toHaveClass(/cm-focused/);
      await expect(
        page.locator(".cm-selectionBackground").first(),
      ).not.toHaveCSS("background-color", focused);
    });
  }

test("Markdown selection stays readable and matches do not modify the buffer", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "dark" });
  const markdown =
    "# Project notes\n\n1. Prepare the **workspace** and open a terminal.\n2. Keep the workspace ready for the next session.\n\nUse `pnpm dev` to start.\n";
  await openEditor(page, "README.md", markdown);
  const start = markdown.indexOf("workspace");
  await selectText(page, start, start + "workspace".length);
  await expect(page.locator(".cm-selectionMatch")).toHaveCount(1);
  await expect(page.locator(".editor-status")).not.toContainText("Modified");
  await selectText(page, 0, markdown.length);
  await page.screenshot({
    path: testInfo.outputPath("editor-markdown-selection.png"),
  });
  await expect(page.locator(".cm-line").first()).toHaveCSS(
    "color",
    "rgb(243, 244, 246)",
  );
  const heading = page.locator(".cm-line").first();
  expect(
    await heading.evaluate(
      (element) => getComputedStyle(element, "::selection").color,
    ),
  ).toBe("rgb(243, 244, 246)");
  await page.keyboard.insertText("Updated notes");
  await page.keyboard.press("Control+z");
  await expect(page.locator(".cm-content")).toContainText(
    "Prepare the **workspace**",
  );
  await page.setViewportSize({ width: 800, height: 420 });
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Control+f");
  const search = page.locator('.cm-search input[name="search"]');
  await search.fill("");
  await search.pressSequentially("workspace");
  await expect(page.locator(".cm-panels-top .cm-search")).toBeVisible();
  await expect(page.locator(".cm-searchMatch")).toHaveCount(2);
  expect(
    await search.evaluate(
      (element) => getComputedStyle(element, "::selection").backgroundColor,
    ),
  ).not.toBe("rgba(0, 0, 0, 0)");
  await page.screenshot({
    path: testInfo.outputPath("editor-search-minimum.png"),
  });
});

test("custom editor colors apply live without replacing the selection or undo history", async ({
  page,
}) => {
  await openEditor(page);
  const content = page.locator(".cm-content");
  await content.fill("const value = 1;");
  await selectText(page, 6, 11);
  await page.evaluate(async () => {
    const { prepareTheme } = await import("/src/theme/runtime.ts");
    const prepared = await prepareTheme(
      {
        id: "editor-colors",
        directory: "/themes/editor-colors",
        revision: "colors",
        raw: JSON.stringify({
          version: 2,
          name: "Editor colors",
          appearance: "dark",
          common: {
            editor: {
              colors: {
                background: "#172030",
                foreground: "#efe5d0",
                gutterBackground: "#202b3b",
                gutterForeground: "#aabbcc",
                selection: "#354967",
                cursor: "#fedcba",
              },
            },
          },
        }),
      },
      { version: 1, active: "editor-colors", appearance: "dark" },
    );
    await prepared.commit();
  });
  await expect(content).toHaveText("const value = 1;");
  await expect(content).toHaveCSS("color", "rgb(239, 229, 208)");
  await expect(page.locator(".cm-editor")).toHaveCSS(
    "background-color",
    "rgb(23, 32, 48)",
  );
  await expect(page.locator(".cm-gutters")).toHaveCSS(
    "background-color",
    "rgb(32, 43, 59)",
  );
  await expect(page.locator(".cm-gutters")).toHaveCSS(
    "color",
    "rgb(170, 187, 204)",
  );
  await expect(page.locator(".cm-selectionBackground").first()).toHaveCSS(
    "background-color",
    "rgb(53, 73, 103)",
  );
  expect(await page.evaluate(() => window.getSelection()?.toString())).toBe(
    "value",
  );
  await content.press("ArrowRight");
  await expect(page.locator(".cm-cursor").first()).toHaveCSS(
    "border-left-color",
    "rgb(254, 220, 186)",
  );
  await content.press("Control+z");
  await expect(content).toContainText("Keep each workspace ready");
});
