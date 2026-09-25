import type { Locator, Page } from "@playwright/test";

export async function dragPreviewDivider(
  page: Page,
  divider: Locator,
  deltaX: number,
) {
  const bounds = await divider.boundingBox();
  if (!bounds) throw new Error("Preview divider is not visible.");
  const x = bounds.x + bounds.width / 2;
  const y = bounds.y + bounds.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + deltaX, y, { steps: 8 });
  await page.mouse.up();
}

export async function storedPreviewRatio(page: Page, relative: string) {
  return page.evaluate((relative) => {
    const session = JSON.parse(localStorage.getItem("test-session") ?? "{}");
    const findFile = (node: any): any => {
      if (!node || typeof node !== "object") return undefined;
      if (node.type === "file")
        return node.relative === relative ? node : undefined;
      if (node.type === "terminal") return findFile(node.layout);
      if (node.type === "split")
        return findFile(node.first) ?? findFile(node.second);
      return undefined;
    };
    for (const project of session.projects ?? [])
      for (const workspace of project.workspaces ?? [])
        for (const tab of workspace.tabs ?? []) {
          if (tab.type === "file" && tab.relative === relative)
            return tab.previewRatio;
          const file = findFile(tab);
          if (file) return file.previewRatio;
        }
    return undefined;
  }, relative);
}
