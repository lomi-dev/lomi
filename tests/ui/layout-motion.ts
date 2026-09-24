import type { Page } from "@playwright/test";

declare global {
  interface Window {
    __layoutMotion?: {
      records: Array<{
        animation: Animation;
        element: Element;
        duration: number;
      }>;
    };
  }
}

export async function captureLayoutMotion(page: Page) {
  await page.evaluate(() => {
    const records: NonNullable<Window["__layoutMotion"]>["records"] = [];
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (keyframes, options) {
      const animation = animate.call(this, keyframes, options);
      if (
        typeof options === "object" &&
        options !== null &&
        options.id === "lomi-layout-motion"
      ) {
        const duration =
          typeof options.duration === "number" ? options.duration : 0;
        records.push({ animation, element: this, duration });
        animation.pause();
        animation.currentTime = duration / 2;
      }
      return animation;
    };
    window.__layoutMotion = { records };
  });
}

export async function layoutMotionCount(page: Page) {
  return page.evaluate(() => window.__layoutMotion?.records.length ?? 0);
}

export async function layoutMotionRecords(page: Page, start = 0) {
  return page.evaluate(
    (start) =>
      (window.__layoutMotion?.records ?? []).slice(start).map((record) => ({
        id: record.animation.id,
        duration: record.duration,
        currentTime: record.animation.currentTime,
        playState: record.animation.playState,
        tagName: record.element.tagName,
        className:
          record.element instanceof HTMLElement ? record.element.className : "",
        paneId: record.element
          .closest<HTMLElement>("[data-pane-id]")
          ?.getAttribute("data-pane-id"),
      })),
    start,
  );
}

export async function finishLayoutMotion(page: Page, start = 0) {
  await page.evaluate((start) => {
    for (const { animation } of window.__layoutMotion?.records.slice(start) ??
      [])
      if (animation.playState === "paused" || animation.playState === "running")
        animation.finish();
  }, start);
}
