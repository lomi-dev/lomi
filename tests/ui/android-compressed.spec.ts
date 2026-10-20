import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockAndroid } from "./android-mock";
import {
  newAndroidTab,
  newProject,
  newSession,
  type TerminalTab,
} from "../../src/model";

async function compressedFixture(
  page: Page,
  holdFirstDecode: boolean,
  options: {
    shared?: boolean;
    browser?: boolean;
    host?: string;
    disableCompression?: boolean;
    rle32?: boolean;
    corruptRLE?: boolean;
  } = {},
) {
  let session;
  if (options.shared) {
    const project = newProject("/project", "local:bash");
    const tab = project.workspaces[0].tabs[0] as TerminalTab;
    const first = newAndroidTab(
      "12345678-1234-4567-8123-123456789abc",
      "Shared phone",
    );
    const second = newAndroidTab(
      "12345678-1234-4567-8123-123456789abc",
      "Shared phone",
    );
    tab.layout = {
      type: "split",
      id: crypto.randomUUID(),
      axis: "horizontal",
      ratio: 0.5,
      first,
      second,
    };
    tab.activePaneId = first.id;
    session = {
      ...newSession(),
      projects: [project],
      activeProjectId: project.id,
    };
  }
  await mockDesktop(page, false, session);
  await mockAndroid(page, true);
  await page.addInitScript(
    ({
      holdFirstDecode,
      host,
      disableCompression,
      rle32,
      corruptRLE,
      browser,
    }) => {
      const desktop = window as any;
      if (browser) desktop.isTauri = false;
      desktop.__androidTest.state.host = host;
      desktop.__compressedPackets = [];
      desktop.__compressedDraws = 0;
      desktop.__compressedUploads = [];
      const upload = WebGLRenderingContext.prototype.texSubImage2D;
      WebGLRenderingContext.prototype.texSubImage2D = function (
        ...args: any[]
      ) {
        if (
          this.canvas instanceof HTMLCanvasElement &&
          this.canvas.classList.contains("android-screen")
        )
          desktop.__compressedUploads.push({
            canvas: this.canvas,
            kind: args.length === 9 ? "rgba" : "canvas",
            source: args.length === 9 ? args[8] : args[6],
          });
        return (upload as any).apply(this, args);
      };
      const draw = WebGLRenderingContext.prototype.drawArrays;
      WebGLRenderingContext.prototype.drawArrays = function (...args) {
        if (
          this.canvas instanceof HTMLCanvasElement &&
          this.canvas.classList.contains("android-screen")
        )
          desktop.__compressedDraws++;
        return draw.apply(this, args);
      };
      const put = CanvasRenderingContext2D.prototype.putImageData;
      CanvasRenderingContext2D.prototype.putImageData = function (
        ...args: any[]
      ) {
        if (
          this.canvas instanceof HTMLCanvasElement &&
          this.canvas.classList.contains("android-screen")
        ) {
          desktop.__compressedUploads.push({
            canvas: this.canvas,
            kind: "2d-rgba",
            source: args[0].data,
          });
          desktop.__compressedDraws++;
        }
        return (put as any).apply(this, args);
      };
      const copy = CanvasRenderingContext2D.prototype.drawImage;
      CanvasRenderingContext2D.prototype.drawImage = function (...args: any[]) {
        if (
          this.canvas instanceof HTMLCanvasElement &&
          this.canvas.classList.contains("android-screen")
        ) {
          desktop.__compressedUploads.push({
            canvas: this.canvas,
            kind: "2d-canvas",
            source: args[0],
          });
          desktop.__compressedDraws++;
        }
        return (copy as any).apply(this, args);
      };
      const NativeDecoder = window.DecompressionStream;
      window.DecompressionStream = class extends NativeDecoder {
        constructor(format: CompressionFormat) {
          super(format);
          if (disableCompression)
            throw Error("Deflate intentionally unavailable");
          const getReader = this.readable.getReader.bind(this.readable);
          Object.defineProperty(this.readable, "getReader", {
            value: () => {
              const reader = getReader();
              const decodeNumber = (desktop.__decodeReaders ?? 0) + 1;
              desktop.__decodeReaders = decodeNumber;
              const read = reader.read.bind(reader);
              const cancel = reader.cancel.bind(reader);
              reader.read = async () => {
                const value = await read();
                if (
                  (holdFirstDecode || desktop.__holdDecode === decodeNumber) &&
                  !desktop.__decodeHeld &&
                  !value.done
                ) {
                  desktop.__decodeHeld = true;
                  await new Promise<void>((resolve) => {
                    desktop.__resumeDecode = resolve;
                  });
                }
                return value;
              };
              reader.cancel = (reason) => {
                desktop.__resumeDecode?.();
                return cancel(reason);
              };
              return reader;
            },
          });
        }
      };
      const invoke = desktop.__androidInvoke;
      desktop.__androidInvoke = async (command: string, args: any) => {
        if (command === "android_subscribe_frames") {
          desktop.__compressedRequested = args.compressedFrames;
          desktop.__rle32Requested = args.rle32Frames;
          const receive = args.frames.onmessage;
          args.frames.onmessage = (raw: ArrayBuffer) => {
            desktop.__lastRawAndroidFrame = raw.slice(0);
            if (!args.compressedFrames && !(rle32 && args.rle32Frames)) {
              receive(raw);
              return;
            }
            void (async () => {
              const header = new DataView(raw);
              const length = header.getUint32(64, true);
              const useRLE = rle32 && args.rle32Frames;
              let encoded: Uint8Array;
              if (useRLE) {
                const pixels = new Uint8Array(raw, 68, length);
                if (!pixels.every((value, i) => value === pixels[i % 4]))
                  throw Error("RLE fixture requires its uniform test pixels");
                encoded = new Uint8Array(8);
                new DataView(encoded.buffer).setUint32(
                  0,
                  corruptRLE ? 0xffffffff : 0x80000000 + length / 4,
                  true,
                );
                encoded.set(pixels.subarray(0, 4), 4);
              } else
                encoded = new Uint8Array(
                  await new Response(
                    new Blob([raw.slice(68, 68 + length)])
                      .stream()
                      .pipeThrough(new CompressionStream("deflate")),
                  ).arrayBuffer(),
                );
              const bytes = new ArrayBuffer(
                Math.max(1024, 68 + encoded.length),
              );
              new Uint8Array(bytes, 0, 68).set(new Uint8Array(raw, 0, 68));
              new DataView(bytes).setUint32(60, useRLE ? 2 : 1, true);
              new DataView(bytes).setUint32(64, encoded.length, true);
              new Uint8Array(bytes, 68, encoded.length).set(encoded);
              const packet = {
                epoch: Number(header.getBigUint64(24, true)),
                released: 0,
              };
              desktop.__compressedPackets.push(packet);
              const transfer = (bytes as any).transfer?.bind(bytes);
              Object.defineProperty(bytes, "transfer", {
                value: (size: number) => {
                  packet.released++;
                  return transfer ? transfer(size) : new ArrayBuffer(size);
                },
              });
              receive(bytes);
            })().catch((reason) => {
              desktop.__compressedFixtureError = String(reason);
            });
          };
        }
        return invoke(command, args);
      };
    },
    {
      holdFirstDecode,
      browser: options.browser ?? false,
      host: options.host ?? "linux_x86_64",
      disableCompression: options.disableCompression ?? false,
      rle32: options.rle32 ?? false,
      corruptRLE: options.corruptRLE ?? false,
    },
  );
  await page.goto("/");
  if (options.browser) {
    // The browser workbench does not restore the native session fixture.
    await page.evaluate(async () => {
      const runtime = await import("/src/android/runtime.ts");
      const { newAndroidTab } = await import("/src/model.ts");
      const deviceId = "12345678-1234-4567-8123-123456789abc";
      for (let i = 0; i < 2; i++) {
        const host = document.createElement("div");
        host.style.cssText = "position:fixed;inset:0;width:300px;height:600px";
        const input = document.createElement("textarea");
        document.body.append(host, input);
        runtime.mount(
          newAndroidTab(deviceId, "Browser phone"),
          host,
          input,
          () => {},
          false,
        );
      }
    });
    return;
  }
  if (options.shared) return;
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page
    .getByRole("menuitem", { name: "New android symulator", exact: true })
    .click();
}

test("Linux negotiates deflate frames and acknowledges the decoded source", async ({
  page,
}) => {
  await compressedFixture(page, false);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "android_ack_frame",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
  expect(await page.evaluate(() => (window as any).__compressedRequested)).toBe(
    true,
  );
  expect(
    await page.evaluate(() => (window as any).__compressedFixtureError),
  ).toBeUndefined();
  expect(
    await page.evaluate(() => (window as any).__compressedPackets[0].released),
  ).toBe(1);
  await expect(page.locator(".android-screen")).toBeVisible();
});

test("hiding and replacing a stream cancels a delayed decode without stale draw or ACK", async ({
  page,
}) => {
  await compressedFixture(page, true);
  await expect
    .poll(() => page.evaluate(() => (window as any).__decodeHeld))
    .toBe(true);
  const oldEpoch = await page.evaluate(
    () => (window as any).__compressedPackets[0].epoch,
  );
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "android_ack_frame",
        ).length,
    ),
  ).toBe(0);
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__compressedPackets[0].released),
    )
    .toBe(1);
  await expect
    .poll(() => page.evaluate(() => (window as any).__androidTest.live.size))
    .toBe(0);
  expect(await page.evaluate(() => (window as any).__compressedDraws)).toBe(0);
  await page.getByRole("tab", { name: "Test phone", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "android_ack_frame",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
  const acknowledgements = await page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter((call: any) => call.command === "android_ack_frame")
      .map((call: any) => call.args.epoch),
  );
  expect(acknowledgements).not.toContain(oldEpoch);
  expect(
    await page.evaluate(() => (window as any).__compressedPackets[0].released),
  ).toBe(1);
  expect(
    await page.evaluate(() => (window as any).__compressedFixtureError),
  ).toBeUndefined();
  await expect(page.locator(".android-screen")).toBeVisible();
});

for (const rle32 of [false, true])
  test(`Linux uploads one decoded RGBA buffer, copies its primary and reuses scratch (RLE32=${rle32})`, async ({
    page,
  }) => {
    await compressedFixture(page, false, { shared: true, rle32 });
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__compressedUploads.length),
      )
      .toBeGreaterThanOrEqual(2);
    const first = await page.evaluate(() => {
      const desktop = window as any,
        uploads = desktop.__compressedUploads.slice(-2);
      desktop.__firstDecodedPixels = uploads[0].source;
      return {
        kinds: uploads.map((item: any) => item.kind),
        same: uploads[0].canvas === uploads[1].source,
        distinctCanvases: uploads[0].canvas !== uploads[1].canvas,
      };
    });
    expect(first).toEqual({
      kinds: ["2d-rgba", "2d-canvas"],
      same: true,
      distinctCanvases: true,
    });
    await page.evaluate(() => {
      const desktop = window as any,
        raw = desktop.__lastRawAndroidFrame.slice(0),
        header = new DataView(raw);
      header.setBigUint64(32, 2n, true);
      const length = header.getUint32(64, true);
      new Uint8Array(raw, 68, length).fill(93);
      desktop.__androidTest.live
        .get(Number(header.getBigUint64(24, true)))
        .onmessage(raw);
    });
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls.filter(
              (call: any) =>
                call.command === "android_ack_frame" &&
                call.args.sequence === 2,
            ).length,
        ),
      )
      .toBe(1);
    expect(
      await page.evaluate(() => {
        const desktop = window as any,
          uploads = desktop.__compressedUploads.slice(-2);
        return (
          uploads[0].source.buffer === desktop.__firstDecodedPixels.buffer &&
          uploads[0].source[0] === 93 &&
          uploads[1].source === uploads[0].canvas
        );
      }),
    ).toBe(true);
  });

for (const options of [
  { host: "darwin_arm64" },
  { host: "windows_x86_64" },
  { host: "linux_x86_64", browser: true },
  { host: "linux_aarch64" },
  { host: "linux_x86_64", disableCompression: true },
]) {
  test(`secondary uploads preserve host policy with ${options.host} browser=${!!options.browser} codec=${options.disableCompression ? "unavailable" : "available"}`, async ({
    page,
  }) => {
    await compressedFixture(page, false, { shared: true, ...options });
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__compressedUploads.length),
      )
      .toBeGreaterThanOrEqual(2);
    const kinds = await page.evaluate(() =>
      (window as any).__compressedUploads
        .slice(-2)
        .map((upload: any) => upload.kind),
    );
    expect(kinds).toEqual(
      options.host === "linux_x86_64" && !options.browser
        ? ["2d-rgba", "2d-canvas"]
        : ["rgba", "canvas"],
    );
    expect(
      await page.evaluate(() => (window as any).__compressedRequested),
    ).toBe(false);
    expect(await page.evaluate(() => (window as any).__rle32Requested)).toBe(
      options.host === "linux_x86_64" && !options.browser,
    );
  });
}

test("a delayed reused decode cannot share scratch with its replacement stream", async ({
  page,
}) => {
  await compressedFixture(page, false);
  await expect
    .poll(() => page.evaluate(() => (window as any).__compressedUploads.length))
    .toBeGreaterThan(0);
  await page.evaluate(() => {
    const desktop = window as any;
    desktop.__firstDecodedPixels = desktop.__compressedUploads.at(-1).source;
    desktop.__holdDecode = 2;
    const raw = desktop.__lastRawAndroidFrame.slice(0),
      header = new DataView(raw);
    header.setBigUint64(32, 2n, true);
    new Uint8Array(raw, 68, header.getUint32(64, true)).fill(94);
    desktop.__androidTest.live
      .get(Number(header.getBigUint64(24, true)))
      .onmessage(raw);
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).__decodeHeld))
    .toBe(true);
  const oldEpoch = await page.evaluate(
    () => (window as any).__compressedPackets[0].epoch,
  );
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__compressedPackets[1].released),
    )
    .toBe(1);
  expect(await page.evaluate(() => (window as any).__compressedDraws)).toBe(1);
  await page.getByRole("tab", { name: "Test phone", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__compressedDraws))
    .toBeGreaterThan(1);
  expect(
    await page.evaluate(() => {
      const desktop = window as any;
      return (
        desktop.__compressedUploads.at(-1).source.buffer !==
          desktop.__firstDecodedPixels.buffer &&
        desktop.__firstDecodedPixels[0] === 90
      );
    }),
  ).toBe(true);
  expect(
    await page.evaluate(
      (epoch) =>
        (window as any).__nativeTest.calls.some(
          (call: any) =>
            call.command === "android_ack_frame" &&
            call.args.epoch === epoch &&
            call.args.sequence === 2,
        ),
      oldEpoch,
    ),
  ).toBe(false);
});

test("Linux negotiates RLE32 without browser deflate and releases decoded packets", async ({
  page,
}) => {
  await compressedFixture(page, false, {
    rle32: true,
    disableCompression: true,
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "android_ack_frame",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
  expect(await page.evaluate(() => (window as any).__rle32Requested)).toBe(
    true,
  );
  expect(await page.evaluate(() => (window as any).__compressedRequested)).toBe(
    false,
  );
  expect(
    await page.evaluate(() => (window as any).__compressedPackets[0].released),
  ).toBe(1);
  expect(
    await page.evaluate(() => (window as any).__compressedFixtureError),
  ).toBeUndefined();
  await expect(page.locator(".android-screen")).toBeVisible();
});

test("malformed negotiated RLE32 never draws or acknowledges and releases once", async ({
  page,
}) => {
  await compressedFixture(page, false, { rle32: true, corruptRLE: true });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__compressedPackets[0]?.released),
    )
    .toBe(1);
  expect(await page.evaluate(() => (window as any).__compressedDraws)).toBe(0);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "android_ack_frame",
        ).length,
    ),
  ).toBe(0);
});

test("non-Linux rejects RLE32 that was not negotiated before drawing or ACK", async ({
  page,
}) => {
  await compressedFixture(page, false, { host: "darwin_arm64" });
  await expect
    .poll(() => page.evaluate(() => (window as any).__compressedDraws))
    .toBe(1);
  const before = await page.evaluate(() => {
    const desktop = window as any,
      raw = desktop.__lastRawAndroidFrame;
    const header = new DataView(raw),
      bytes = new ArrayBuffer(1024),
      view = new DataView(bytes);
    new Uint8Array(bytes, 0, 68).set(new Uint8Array(raw, 0, 68));
    view.setBigUint64(32, 2n, true);
    view.setUint32(60, 2, true);
    view.setUint32(64, 8, true);
    view.setUint32(
      68,
      0x80000000 + header.getUint32(48, true) * header.getUint32(52, true),
      true,
    );
    new Uint8Array(bytes, 72, 4).fill(90);
    desktop.__unnegotiatedReleased = 0;
    const transfer = (bytes as any).transfer?.bind(bytes);
    Object.defineProperty(bytes, "transfer", {
      value: (size: number) => {
        desktop.__unnegotiatedReleased++;
        return transfer ? transfer(size) : new ArrayBuffer(size);
      },
    });
    const count = desktop.__nativeTest.calls.filter(
      (call: any) => call.command === "android_ack_frame",
    ).length;
    desktop.__androidTest.live
      .get(Number(header.getBigUint64(24, true)))
      .onmessage(bytes);
    return count;
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).__unnegotiatedReleased))
    .toBe(1);
  expect(await page.evaluate(() => (window as any).__compressedDraws)).toBe(1);
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "android_ack_frame",
        ).length,
    ),
  ).toBe(before);
});
