import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = await readFile(
  new URL("./android-product-linux-visible.mjs", import.meta.url),
  "utf8",
);
const measurement = source.slice(
  source.indexOf("async function measureVisible("),
  source.indexOf("\nconst tab = "),
);

async function run(options = {}) {
  let now = 0,
    count = 0,
    reads = 0,
    calls = 0;
  const window = {};
  class Canvas {
    constructor(paneId, width, height) {
      Object.assign(this, { paneId, width, height, isConnected: true });
      this.classList = { contains: (name) => name === "android-screen" };
    }
    closest() {
      return { dataset: { androidPaneId: this.paneId } };
    }
    getBoundingClientRect() {
      return { width: this.width, height: this.height };
    }
  }
  class Image {
    constructor(data, width, height) {
      Object.assign(this, { data, width, height });
    }
  }
  class Context {
    constructor(canvas) {
      this.canvas = canvas;
      this.color = 0;
      canvas.context = this;
    }
    putImageData(image) {
      if (options.putFailure) throw Error("Original put failed");
      this.color = image.data[0];
    }
    drawImage(source) {
      if (options.drawFailure) throw Error("Original draw failed");
      this.color = source.context.color;
    }
    getImageData() {
      reads++;
      if (options.inputEvidence !== false)
        throw Error("Measured phase must not read bitmap pixels");
      if (options.readFailure) throw Error("Bitmap unavailable");
      return { data: Uint8ClampedArray.of(this.color, 0, 0, 255) };
    }
    getContextAttributes() {
      return {
        alpha: false,
        willReadFrequently: !options.wrongAttributes,
      };
    }
  }
  const primary = new Canvas("primary", 720, 1280);
  const secondary = new Canvas("secondary", 280, 498);
  const canvases = [primary, secondary];
  const primaryContext = new Context(primary),
    secondaryContext = new Context(secondary);
  const originalPut = Context.prototype.putImageData,
    originalDraw = Context.prototype.drawImage;
  if (options.installFailure)
    Object.defineProperty(Context.prototype, "drawImage", {
      value: originalDraw,
      writable: false,
    });
  if (options.listenerFailure)
    window.chrome = {
      webview: {
        addEventListener() {},
        removeEventListener() {
          throw Error("Listener cleanup failed");
        },
      },
    };
  const foreignDraw = function () {};
  const foreignOwner = { name: "foreign" };
  const pixels = new Uint8ClampedArray(720 * 1280 * 4);
  const image = new Image(pixels, 720, 1280);
  const invoke = async () => ({
    streams: [
      {
        phase: "streaming",
        deviceId: "phone",
        generation: options.replaceGeneration && count ? "other" : "generation",
        epoch: options.replaceEpoch && count ? 8 : 7,
        ipcFrames: count,
        ipcBytes: count * pixels.length,
        grpcFrames: count,
        grpcBytes: count * pixels.length,
      },
    ],
  });
  const sleep = async (milliseconds) => {
    now += milliseconds;
    calls++;
    for (let frame = 0; frame < 3; frame++) {
      count++;
      pixels.fill(options.still ? 37 : count % 2 ? 37 : 211);
      if (!options.stale) primaryContext.putImageData(image, 0, 0);
      if (!options.missingSecondary || frame !== 2)
        secondaryContext.drawImage(primary, 0, 0, 280, 498);
    }
    if (options.cancel) window.__androidProductBenchmark.cancel();
    if (options.detach) secondary.isConnected = false;
    if (options.foreignDraw && calls === 4)
      Context.prototype.drawImage = foreignDraw;
    if (options.foreignOwner && calls === 4)
      window.__androidProductBenchmark = foreignOwner;
  };
  const context = {
    CanvasRenderingContext2D: Context,
    HTMLCanvasElement: Canvas,
    ImageData: Image,
    Uint8ClampedArray,
    window,
    document: {
      querySelectorAll: () => canvases,
      visibilityState: "visible",
    },
    getComputedStyle: () => ({ visibility: "visible" }),
    devicePixelRatio: 1,
    performance: { now: () => now },
    Date,
  };
  const measure = vm.runInNewContext(`(${measurement})`, context);
  let result, error;
  try {
    result = await measure(invoke, sleep, {
      deviceId: "phone",
      generation: "generation",
      seconds: 0.4,
      views: 2,
      name: "test",
      requireShared: false,
      inputEvidence: options.inputEvidence !== false,
    });
  } catch (reason) {
    error = reason;
  }
  assert.equal(Context.prototype.putImageData, originalPut);
  assert.equal(
    Context.prototype.drawImage,
    options.foreignDraw ? foreignDraw : originalDraw,
  );
  assert.equal(
    window.__androidProductBenchmark,
    options.foreignOwner ? foreignOwner : undefined,
  );
  return { result, error, reads };
}

test("formal 2D measurement proves fresh canonical copies with no bitmap reads", async () => {
  const { result, error, reads } = await run();
  assert.equal(error, undefined);
  assert.equal(result.completed, true, result.failure);
  assert.equal(reads, 0);
  assert.equal(result.measuredPhaseReadback, false);
  assert.equal(result.rendererKind, "linux-canvas-2d");
  assert.equal(result.softwareBackingInferenceOnly, true);
  assert.deepEqual(
    Array.from(result.views, (view) => view.uploadKind),
    ["2d-imagedata", "2d-canvas-copy"],
  );
  assert.equal(result.views[0].frames, result.views[1].frames);
  assert.equal(result.views[0].frames, result.counters.ipcFrames);
  assert(result.views.every((view) => view.fingerprintTransitions >= 3));
  assert.equal(result.views[1].sourcePaneId, result.views[0].paneId);
  assert.equal(result.views[1].secondaryBitmapVerified, false);
  assert.equal(
    result.views[1].fingerprintProvenance,
    "canonical-primary-input-inherited-after-successful-drawImage",
  );
});

test("bitmap preflight reads each drawn framebuffer and marks secondary proof", async () => {
  const { result, error, reads } = await run({ inputEvidence: false });
  assert.equal(error, undefined);
  assert.equal(result.completed, true, result.failure);
  assert.equal(reads, result.views[0].frames * 2 * 64);
  assert.equal(result.measuredPhaseReadback, true);
  assert(
    result.views.every(
      (view) => view.fingerprintProvenance === "actual-2d-framebuffer-readback",
    ),
  );
  assert.equal(result.views[1].secondaryBitmapVerified, true);
});

for (const [options, failure] of [
  [{ stale: true }, /fresh canonical/],
  [{ still: true }, /too few changing/],
  [{ missingSecondary: true }, /same canonical frame count/],
  [{ replaceGeneration: true }, /generation or epoch/],
  [{ replaceEpoch: true }, /generation or epoch/],
  [{ detach: true }, /hidden or detached/],
  [{ wrongAttributes: true }, /context attributes/],
  [{ cancel: true }, /cancelled/],
  [{ inputEvidence: false, readFailure: true }, /framebuffer fingerprint/],
])
  test(`2D evidence fails closed and restores hooks: ${JSON.stringify(options)}`, async () => {
    const { result, error } = await run(options);
    assert.equal(error, undefined);
    assert.equal(result.completed, false);
    assert.match(result.failure, failure);
  });

for (const options of [{ putFailure: true }, { drawFailure: true }])
  test(`native draw failure restores measurement ownership: ${JSON.stringify(options)}`, async () => {
    const { error } = await run(options);
    assert.match(error.message, /Original .* failed/);
  });

for (const options of [{ foreignDraw: true }, { foreignOwner: true }])
  test(`foreign hook/owner replacement is preserved and rejected: ${JSON.stringify(options)}`, async () => {
    const { result, error } = await run(options);
    assert.equal(error, undefined);
    assert.equal(result.completed, false);
    assert.match(result.failure, /ownership changed/);
  });

test("partial hook installation failure releases the installed hook and owner", async () => {
  const { error } = await run({ installFailure: true });
  assert.match(error.message, /install.*measurement hooks|read only/);
});

test("listener cleanup failure still restores both prototype hooks and owner", async () => {
  const { error } = await run({ listenerFailure: true });
  assert.match(error.message, /Listener cleanup failed/);
});
