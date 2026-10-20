import type { AndroidRenderer } from "./canvas";
import type { AndroidFrame } from "./frame";

/** Owns only a Linux view's CPU-backed canvas, never borrowed frame buffers. */
export class LinuxCanvas2D implements AndroidRenderer {
  readonly element = document.createElement("canvas");
  private readonly context: CanvasRenderingContext2D;
  private disposed = false;
  private lost = false;
  private readonly onLost: (event: Event) => void;

  constructor(onError: (message: string) => void) {
    this.element.className = "android-screen";
    this.element.setAttribute("aria-label", "Android phone screen");
    this.element.tabIndex = 0;
    const context = this.element.getContext("2d", {
      alpha: false,
      willReadFrequently: true,
    });
    if (!context)
      throw new Error(
        "Android 2D graphics are unavailable. Reconnect the panel.",
      );
    this.context = context;
    this.onLost = (event) => {
      event.preventDefault();
      this.lost = true;
      if (!this.disposed)
        onError("Android 2D context was lost. Reconnect the panel.");
    };
    this.element.addEventListener("contextlost", this.onLost);
  }

  draw(
    frame: AndroidFrame,
    source?: HTMLCanvasElement,
    target = { width: frame.width, height: frame.height },
  ) {
    const { context, element } = this;
    if (this.disposed || this.lost || context.isContextLost?.())
      throw new Error("Android 2D context was lost. Reconnect the panel.");
    if (!frame.width || !frame.height) {
      context.clearRect(0, 0, element.width, element.height);
      return;
    }
    if (
      ![frame.width, frame.height, target.width, target.height].every(
        (size) => Number.isInteger(size) && size > 0 && size <= 1280,
      ) ||
      frame.width * frame.height > 921600 ||
      target.width > frame.width ||
      target.height > frame.height
    )
      throw new Error("Invalid Android 2D dimensions.");
    if (element.width !== target.width || element.height !== target.height) {
      element.width = target.width;
      element.height = target.height;
    }
    if (source) {
      if (
        source === element ||
        source.width !== frame.width ||
        source.height !== frame.height
      )
        throw new Error(
          "Android 2D source is not the full-resolution primary.",
        );
      context.imageSmoothingEnabled = true;
      context.drawImage(source, 0, 0, target.width, target.height);
    } else {
      if (
        target.width !== frame.width ||
        target.height !== frame.height ||
        frame.pixels.byteLength !== frame.width * frame.height * 4 ||
        !(frame.pixels.buffer instanceof ArrayBuffer)
      )
        throw new Error("Invalid Android 2D primary pixels.");
      // This clamped view aliases the borrowed packet/decoded buffer. ImageData
      // and putImageData consume it synchronously; nothing retains it after draw.
      const pixels = new Uint8ClampedArray(
        frame.pixels.buffer,
        frame.pixels.byteOffset,
        frame.pixels.byteLength,
      );
      context.putImageData(
        new ImageData(pixels, frame.width, frame.height),
        0,
        0,
      );
    }
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.element.removeEventListener("contextlost", this.onLost);
    this.element.width = this.element.height = 0;
    this.element.remove();
  }
}
