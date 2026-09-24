import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { newProject, newSession, openFileTab } from "../../src/model";
import { chooseOption, mockDesktop } from "./desktop";

const indentButton = (page: Page) =>
  page.getByRole("button", {
    name: "Change indentation settings",
    exact: true,
  });
const languageButton = (page: Page) =>
  page.getByRole("button", { name: "Change language mode", exact: true });
const editorText = (page: Page) =>
  page
    .locator(".cm-line")
    .allTextContents()
    .then((lines) => lines.join("\n"));

async function openReadme(page: Page) {
  await mockDesktop(page);
  await page.goto("/");
  await page.getByRole("button", { name: "README.md", exact: true }).click();
  await expect(page.locator(".cm-content")).toBeVisible();
}
async function replaceText(page: Page, text: string) {
  await page.locator(".cm-content").focus();
  await page.keyboard.press("Control+a");
  await page.keyboard.insertText(text);
}
async function selectSize(
  page: Page,
  action:
    "Indent Using Spaces" | "Indent Using Tabs" | "Change Tab Display Size",
  size: number,
) {
  await indentButton(page).click();
  await page
    .getByRole(
      action === "Change Tab Display Size" ? "menuitem" : "menuitemradio",
      { name: new RegExp(`^${action}`) },
    )
    .click();
  await page
    .getByRole("menuitemradio", { name: new RegExp(`^${size}( Current)?$`) })
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".cm-content")).toBeFocused();
}
async function selectLanguage(page: Page, language: string) {
  await languageButton(page).click();
  await page
    .getByRole("searchbox", { name: "Filter languages" })
    .fill(language);
  await page
    .getByRole("menuitemradio", { name: language, exact: true })
    .click();
  await expect(page.locator(".cm-content")).toBeFocused();
}

test("the footer changes indentation inline for the current buffer without changing defaults or losing undo", async ({
  page,
  context,
}) => {
  await openReadme(page);
  await replaceText(page, "    old\nkeep");
  await page.keyboard.press("ArrowLeft");
  await selectSize(page, "Indent Using Spaces", 2);
  await expect(indentButton(page)).toHaveText("Spaces: 2");
  expect(await editorText(page)).toBe("    old\nkeep");
  await expect(page.getByText("Ln 2, Col 4", { exact: true })).toBeVisible();
  await page.keyboard.press("Tab");
  expect(await editorText(page)).toBe("    old\nkee  p");
  await page.keyboard.press("Control+z");
  expect(await editorText(page)).toBe("    old\nkeep");
  await page.keyboard.press("Control+z");
  await expect(page.locator(".cm-content")).toContainText(
    "A text file preview.",
  );
  await expect(page.getByRole("tab", { name: /README.md/ })).not.toContainText(
    "●",
  );
  await page
    .getByRole("button", { name: "it's a file.txt", exact: true })
    .click();
  await expect(indentButton(page)).toHaveText("Spaces: 4");
  await page.getByRole("tab", { name: "README.md", exact: true }).click();
  await expect(indentButton(page)).toHaveText("Spaces: 2");
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter((call: any) =>
        ["open_settings", "save_editor_preferences"].includes(call.command),
      ),
    ),
  ).toEqual([]);
  await expect(page.getByRole("contentinfo")).not.toContainText(
    /\d+ tabs|\d+ terminals|Lomi/,
  );

  const settings = await context.newPage();
  await mockDesktop(settings);
  await settings.goto("/?window=settings&page=editor");
  await chooseOption(
    settings.getByRole("combobox", { name: "Tab size" }),
    "8 spaces",
  );
  await expect(settings.getByRole("status")).toHaveText("Saved");
  await expect(indentButton(page)).toHaveText("Spaces: 2");
  await indentButton(page).click();
  await page
    .getByRole("menuitem", { name: "Use Default Indentation", exact: true })
    .click();
  await expect(indentButton(page)).toHaveText("Spaces: 8");
});

test("display size changes tab stops independently while inline spaces, literal tabs and dedentation work", async ({
  page,
}) => {
  await openReadme(page);
  await replaceText(page, "\tone\ntwo");
  await selectSize(page, "Indent Using Spaces", 2);
  await selectSize(page, "Change Tab Display Size", 8);
  await expect(indentButton(page)).toHaveText("Spaces: 2");
  await expect
    .poll(() =>
      page
        .locator(".cm-content")
        .evaluate((element) => getComputedStyle(element).tabSize),
    )
    .toBe("8");
  expect(await editorText(page)).toBe("\tone\ntwo");
  await page.keyboard.press("Tab");
  expect(await editorText(page)).toBe("\tone\ntwo  ");
  await selectSize(page, "Indent Using Tabs", 4);
  await expect(indentButton(page)).toHaveText("Tabs: 4");
  await page.keyboard.press("Tab");
  expect(await editorText(page)).toBe("\tone\ntwo  \t");
  await replaceText(page, "first\nsecond");
  await page.keyboard.press("Control+a");
  await page.keyboard.press("Tab");
  expect(await editorText(page)).toBe("\tfirst\n\tsecond");
  await page.keyboard.press("Shift+Tab");
  expect(await editorText(page)).toBe("first\nsecond");
});

test("language selection changes the parser and keeps file identity, text and undo across tab switches", async ({
  page,
}) => {
  await openReadme(page);
  await replaceText(page, '<section class="example">Hello</section>');
  await selectLanguage(page, "HTML");
  await expect(languageButton(page)).toHaveText("HTML");
  await expect
    .poll(() => page.locator(".cm-line span[class]").count())
    .toBeGreaterThan(0);
  expect(await editorText(page)).toBe(
    '<section class="example">Hello</section>',
  );
  await expect(page.getByRole("tab", { selected: true })).toContainText(
    "README.md",
  );
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("tab", { name: /README.md/ }).click();
  await expect(languageButton(page)).toHaveText("HTML");
  await selectLanguage(page, "Plain text");
  await expect(languageButton(page)).toHaveText("Plain text");
  await expect(page.locator(".cm-line span[class]")).toHaveCount(0);
  await page.keyboard.press("Control+z");
  await expect(page.locator(".cm-content")).toContainText(
    "A text file preview.",
  );
  await expect(page.getByRole("tab", { name: /README.md/ })).not.toContainText(
    "●",
  );
  await languageButton(page).click();
  await page
    .getByRole("menuitemradio", { name: "Auto Detect Markdown", exact: true })
    .click();
  await expect(languageButton(page)).toHaveText("Markdown");
  await expect(page.getByRole("tab", { selected: true })).toHaveText(
    "README.md",
  );
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.some(
        (call: any) => call.command === "save_editor_file",
      ),
    ),
  ).toBe(false);
});

test("Lua files detect their language, highlight syntax, indent, comment, and save edits", async ({
  page,
}, testInfo) => {
  const project = newProject("/project", "local:bash");
  const session = openFileTab(
    { ...newSession(), projects: [project], activeProjectId: project.id },
    project.workspaces[0].id,
    "/project",
    "main.LUA",
  );
  const content = [
    "-- Lua sample",
    "local count = 42",
    'local text = "Zażółć 🦀"',
    "--[=[",
    "block comment",
    "]=]",
    "local long = [=[",
    "long string",
    "]=]",
    "if count > 0 then",
    "    print(text)",
    "end",
  ].join("\n");
  await mockDesktop(page, true, session, undefined, {
    "/project/main.LUA": {
      content,
      revision: "initial",
      encoding: "utf8",
      readOnly: false,
    },
  });
  await page.goto("/");
  await expect(languageButton(page)).toHaveText("Lua");
  const token = (text: string) =>
    page.locator(".cm-line span[class]").getByText(text, { exact: true });
  await expect(token("local").first()).toHaveCSS("font-weight", "700");
  await expect(token("block comment")).toHaveCSS("font-style", "normal");
  const color = (text: string) =>
    token(text)
      .first()
      .evaluate((element) => getComputedStyle(element).color);
  // Lomi intentionally shares the info color between keywords and numbers;
  // keyword weight still distinguishes them, while strings use success green.
  expect(await color("local")).toBe(await color("42"));
  expect(await color("42")).not.toBe(await color('"Zażółć 🦀"'));
  expect(await color("long string")).toBe(await color('"Zażółć 🦀"'));
  await page.screenshot({ path: testInfo.outputPath("lua-editor.png") });

  await selectLanguage(page, "Plain text");
  await expect(page.locator(".cm-line span[class]")).toHaveCount(0);
  await selectLanguage(page, "Lua");
  await expect(token("local").first()).toHaveCSS("font-weight", "700");
  expect(await editorText(page)).toBe(content);
  await replaceText(page, "if true then");
  await selectSize(page, "Indent Using Spaces", 2);
  await page.keyboard.press("Enter");
  await page.keyboard.type('print("Lua")');
  expect(await editorText(page)).toBe('if true then\n  print("Lua")');
  await page.keyboard.press("Control+/");
  expect(await editorText(page)).toBe('if true then\n  -- print("Lua")');
  await page.keyboard.press("Control+/");
  await page.keyboard.press("Enter");
  await page.keyboard.type("end");
  const edited = 'if true then\n  print("Lua")\nend';
  expect(await editorText(page)).toBe(edited);
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("tab", { name: /main.LUA/ }).click();
  await expect(languageButton(page)).toHaveText("Lua");
  expect(await editorText(page)).toBe(edited);
  await page.keyboard.press("Control+z");
  expect(await editorText(page)).not.toBe(edited);
  await page.keyboard.press("Control+Shift+Z");
  expect(await editorText(page)).toBe(edited);
  await page.keyboard.press("Control+s");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.editorFiles["/project/main.LUA"].content,
      ),
    )
    .toBe(edited);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const session = JSON.parse(localStorage.getItem("test-session")!);
        const workspace = session.projects[0].workspaces[0];
        return workspace.tabs.find(
          (tab: any) => tab.id === workspace.activeTabId,
        )?.relative;
      }),
    )
    .toBe("main.LUA");
  await page.reload();
  await expect(languageButton(page)).toHaveText("Lua");
  expect(await editorText(page)).toBe(edited);
  await expect(token("if")).toHaveCSS("font-weight", "700");
});

test("menus support keyboard navigation, Escape, filtering, outside clicks, and the minimum window", async ({
  page,
}) => {
  await openReadme(page);
  await page.setViewportSize({ width: 800, height: 420 });
  await indentButton(page).click();
  await expect(
    page.getByRole("menuitemradio", {
      name: "Indent Using Spaces",
      exact: true,
    }),
  ).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("dialog", { name: "Indent Using Tabs", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("dialog", { name: "Indentation", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(indentButton(page)).toBeFocused();
  await expect(indentButton(page)).toHaveText("Spaces: 4");
  await indentButton(page).click();
  await indentButton(page).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await languageButton(page).click();
  const search = page.getByRole("searchbox", { name: "Filter languages" });
  await expect(search).toBeFocused();
  await search.fill("no matching language");
  await expect(page.getByRole("status")).toContainText(
    "No matching languages.",
  );
  await search.fill("rust");
  await search.press("ArrowDown");
  await expect(
    page.getByRole("menuitemradio", { name: "Rust", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Space");
  await expect(languageButton(page)).toHaveText("Rust");
  await languageButton(page).click();
  const bounds = await page.getByRole("dialog").boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.y).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(800);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(420);
  await page.keyboard.press("Control+w");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await page.locator(".cm-content").click({ position: { x: 5, y: 5 } });
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("a delayed parser cannot override a later language selection", async ({
  page,
}) => {
  await openReadme(page);
  await replaceText(page, "def example():\n    return 1\n");
  let release!: () => void;
  let started!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  const loading = new Promise<void>((resolve) => {
    started = resolve;
  });
  await page.route(/.*@codemirror_lang-python.*\.js.*/, async (route) => {
    started();
    await pending;
    await route.continue();
  });
  try {
    await selectLanguage(page, "Python");
    await loading;
    await selectLanguage(page, "Plain text");
    release();
    await page.evaluate(async () => {
      const url = performance
        .getEntriesByType("resource")
        .map((entry) => entry.name)
        .find((url) => new URL(url).pathname === "/src/editor-languages.ts");
      const languages = await import(url!);
      await languages.loadEditorLanguage(
        languages.editorLanguages.find(
          (language: any) => language.name === "Python",
        ),
      );
    });
    await expect(languageButton(page)).toHaveText("Plain text");
    await expect(page.locator(".cm-content")).not.toHaveAttribute(
      "data-language",
      /.+/,
    );
    await expect(page.locator(".cm-line span[class]")).toHaveCount(0);
    await expect(page.locator(".editor-conflict, .editor-error")).toHaveCount(
      0,
    );
    await selectLanguage(page, "Python");
    await expect
      .poll(() => page.locator(".cm-line span[class]").count())
      .toBeGreaterThan(0);
  } finally {
    release();
  }
});
