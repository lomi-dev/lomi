import { expect, test } from "@playwright/test";

test("Linux 2D consumes borrowed pixels synchronously and scales the full primary", async ({
  page,
}) => {
  await page.goto("/");
  const result = await page.evaluate(async () => {
    const { LinuxCanvas2D } = await import("/src/android/canvas-2d.ts");
    const errors: string[] = [];
    const primary = new LinuxCanvas2D((message: string) =>
      errors.push(message),
    );
    const secondary = new LinuxCanvas2D((message: string) =>
      errors.push(message),
    );
    document.body.append(primary.element, secondary.element);
    const buffer = new ArrayBuffer(720 * 1280 * 4 + 16);
    const pixels = new Uint8Array(buffer, 8, 720 * 1280 * 4);
    const frame = { width: 720, height: 1280, rotation: 0, pixels };
    const OriginalImageData = window.ImageData;
    let alias = false;
    window.ImageData = class extends OriginalImageData {
      constructor(
        data: Uint8ClampedArray<ArrayBuffer>,
        width: number,
        height: number,
      ) {
        super(data, width, height);
        alias =
          data.buffer === buffer &&
          this.data.buffer === buffer &&
          data.byteOffset === 8;
      }
    } as typeof ImageData;
    try {
      for (let i = 0; i < pixels.length; i += 4) {
        const right = (i / 4) % frame.width >= frame.width / 2;
        pixels.set(right ? [0, 0, 255, 255] : [255, 0, 0, 255], i);
      }
      primary.draw(frame);
      pixels.fill(0);
      secondary.draw(frame, primary.element, { width: 180, height: 320 });
      const pixel = (canvas: HTMLCanvasElement, x: number, y: number) =>
        Array.from(canvas.getContext("2d")!.getImageData(x, y, 1, 1).data);
      const colors = [
        pixel(primary.element, 0, 0),
        pixel(secondary.element, 0, 0),
        pixel(secondary.element, 179, 319),
      ];
      const rejects = (draw: () => void) => {
        try {
          draw();
          return false;
        } catch {
          return true;
        }
      };
      const invalid = [
        rejects(() => secondary.draw(frame, secondary.element)),
        rejects(() =>
          secondary.draw(frame, primary.element, { width: 721, height: 320 }),
        ),
        rejects(() => primary.draw({ ...frame, width: 1281 })),
        rejects(() => primary.draw({ ...frame, width: 1280, height: 1280 })),
        rejects(() => primary.draw({ ...frame, pixels: pixels.subarray(4) })),
        rejects(() =>
          primary.draw(frame, undefined, { width: 180, height: 320 }),
        ),
      ];
      primary.draw({ ...frame, width: 1280, height: 720, rotation: 1 });
      secondary.draw(
        { ...frame, width: 1280, height: 720, rotation: 1 },
        primary.element,
        { width: 320, height: 180 },
      );
      const rotated = [
        primary.element.width,
        primary.element.height,
        secondary.element.width,
        secondary.element.height,
      ];
      primary.element.dispatchEvent(
        new Event("contextlost", { cancelable: true }),
      );
      const lost = rejects(() => primary.draw(frame));
      primary.dispose();
      primary.dispose();
      secondary.dispose();
      const disposed = rejects(() => secondary.draw(frame));
      return {
        alias,
        colors,
        invalid,
        rotated,
        lost,
        errors: errors.length,
        disposed,
        released:
          primary.element.width === 0 &&
          primary.element.height === 0 &&
          !primary.element.isConnected,
      };
    } finally {
      window.ImageData = OriginalImageData;
      primary.dispose();
      secondary.dispose();
    }
  });
  expect(result).toEqual({
    alias: true,
    colors: [
      [255, 0, 0, 255],
      [255, 0, 0, 255],
      [0, 0, 255, 255],
    ],
    invalid: Array(6).fill(true),
    rotated: [1280, 720, 320, 180],
    lost: true,
    errors: 1,
    disposed: true,
    released: true,
  });
});
