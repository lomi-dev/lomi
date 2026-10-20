import assert from "node:assert/strict";
import { test } from "node:test";
import { deflateSync } from "node:zlib";
import {
  ANDROID_FRAME_HEADER,
  decodeCompressedFrame,
  decodeFrame,
  decodeRLE32Frame,
  generationBytes,
  supportsCompressedFrames,
} from "../src/android/frame.ts";

const generation = generationBytes("01234567-89ab-4def-8abc-0123456789ab");
const pixels = Uint8Array.from({ length: 64 * 64 * 4 }, (_, i) => i % 251);
function packet(payload: Uint8Array, width = 64, height = 64, encoding = 1) {
  const bytes = new ArrayBuffer(
    Math.max(1024, ANDROID_FRAME_HEADER + payload.length),
  );
  const header = new DataView(bytes);
  header.setUint32(0, 0x50414253, true);
  header.setUint32(4, 4, true);
  new Uint8Array(bytes, 8, 16).set(generation);
  header.setBigUint64(24, 7n, true);
  header.setBigUint64(32, 123n, true);
  header.setBigUint64(40, 987654n, true);
  header.setUint32(48, width, true);
  header.setUint32(52, height, true);
  header.setUint32(56, 2, true);
  header.setUint32(60, encoding, true);
  header.setUint32(64, payload.length, true);
  new Uint8Array(bytes, ANDROID_FRAME_HEADER, payload.length).set(payload);
  return { bytes, header };
}
const signal = () => new AbortController().signal;

test("deflated frames reuse only exact owned output and allocate safely on size or alias changes", async () => {
  const output = new Uint8Array(pixels.length);
  const first = await decodeCompressedFrame(
    packet(deflateSync(pixels)).bytes,
    generation,
    signal(),
    output,
  );
  assert.equal(first.pixels, output);
  assert.deepEqual(output, pixels);
  const changed = new Uint8Array(pixels.length).fill(71);
  const second = await decodeCompressedFrame(
    packet(deflateSync(changed)).bytes,
    generation,
    signal(),
    output,
  );
  assert.equal(second.pixels, output);
  assert.deepEqual(output, changed);
  const smaller = new Uint8Array(32 * 32 * 4).fill(43);
  const resized = await decodeCompressedFrame(
    packet(deflateSync(smaller), 32, 32).bytes,
    generation,
    signal(),
    output,
  );
  assert.notEqual(resized.pixels, output);
  assert.deepEqual(resized.pixels, smaller);
  for (const invalid of [
    new Uint8Array(pixels.length - 4),
    new Uint8Array(pixels.length + 1).subarray(1),
    new Uint8Array(pixels.length + 1).subarray(0, pixels.length),
  ]) {
    const result = await decodeCompressedFrame(
      packet(deflateSync(pixels)).bytes,
      generation,
      signal(),
      invalid,
    );
    assert.notEqual(result.pixels.buffer, invalid.buffer);
    assert.deepEqual(result.pixels, pixels);
  }
  const tiny = new Uint8Array(16 * 16 * 4).fill(17);
  const aliasedPacket = packet(deflateSync(tiny), 16, 16).bytes;
  const alias = new Uint8Array(aliasedPacket);
  assert.equal(alias.byteLength, tiny.length);
  const saved = alias.slice();
  const nonalias = await decodeCompressedFrame(
    aliasedPacket,
    generation,
    signal(),
    alias,
  );
  assert.notEqual(nonalias.pixels.buffer, aliasedPacket);
  assert.deepEqual(nonalias.pixels, tiny);
  assert.deepEqual(alias, saved);
});

test("deflated RGBA owns exact pixels and preserves v4 frame identity", async () => {
  assert.equal(supportsCompressedFrames(), true);
  assert.equal(supportsCompressedFrames(), true);
  const { bytes } = packet(deflateSync(pixels));
  const result = await decodeCompressedFrame(bytes, generation, signal());
  assert.deepEqual(result.pixels, pixels);
  assert.notEqual(result.pixels.buffer, bytes);
  assert.equal(result.pixels.byteOffset, 0);
  assert.equal(result.epoch, 7);
  assert.equal(result.sequence, 123);
  assert.equal(result.timestampUs, 987654n);
  assert.equal(result.rotation, 2);
  assert.deepEqual([result.width, result.height], [64, 64]);
  assert.throws(() => decodeFrame(bytes, generation));
  const raw = packet(pixels, 64, 64, 0).bytes;
  assert.equal(decodeFrame(raw, generation).pixels.buffer, raw);
  await assert.rejects(decodeCompressedFrame(raw, generation, signal()));
});

test("deflated frames validate identity, dimensions, counters and wire before constructing a codec", async () => {
  const original = globalThis.DecompressionStream;
  let constructed = 0;
  globalThis.DecompressionStream = class {
    constructor() {
      constructed++;
      throw Error("No invalid header may reach the codec");
    }
  } as unknown as typeof DecompressionStream;
  try {
    for (const [offset, value] of [
      [0, 0],
      [4, 3],
      [48, 1281],
      [52, 0],
      [56, 4],
      [60, 2],
      [64, 0],
      [64, 16385],
    ] as const) {
      const { bytes, header } = packet(deflateSync(pixels));
      header.setUint32(offset, value, true);
      await assert.rejects(decodeCompressedFrame(bytes, generation, signal()));
    }
    for (const offset of [24, 32]) {
      for (const value of [0n, 2n ** 53n]) {
        const { bytes, header } = packet(deflateSync(pixels));
        header.setBigUint64(offset, value, true);
        await assert.rejects(
          decodeCompressedFrame(bytes, generation, signal()),
        );
      }
    }
    const wrong = generationBytes("01234567-89ab-4def-8abc-0123456789ac");
    await assert.rejects(
      decodeCompressedFrame(packet(deflateSync(pixels)).bytes, wrong, signal()),
      /old process/,
    );
    await assert.rejects(
      decodeCompressedFrame(new ArrayBuffer(68), generation, signal()),
    );
    const padded = packet(deflateSync(pixels));
    new Uint8Array(padded.bytes)[padded.bytes.byteLength - 1] = 1;
    await assert.rejects(
      decodeCompressedFrame(padded.bytes, generation, signal()),
      /padding/,
    );
    await assert.rejects(
      decodeCompressedFrame(
        packet(new Uint8Array(1), 1280, 1280).bytes,
        generation,
        signal(),
      ),
    );
    assert.equal(constructed, 0);
  } finally {
    globalThis.DecompressionStream = original;
  }
});

test("deflated frames reject corrupt or truncated streams, wrong decoded lengths and compressed bombs", async () => {
  const encoded = deflateSync(pixels);
  const corrupt = Uint8Array.from(encoded);
  corrupt[corrupt.length - 1] ^= 0xff;
  for (const bytes of [
    packet(corrupt).bytes,
    packet(encoded.subarray(0, -1)).bytes,
  ])
    await assert.rejects(decodeCompressedFrame(bytes, generation, signal()));
  await assert.rejects(
    decodeCompressedFrame(
      packet(deflateSync(pixels.subarray(0, -4))).bytes,
      generation,
      signal(),
    ),
    /truncated/,
  );
  await assert.rejects(
    decodeCompressedFrame(
      packet(deflateSync(new Uint8Array(pixels.length + 4))).bytes,
      generation,
      signal(),
    ),
    /exceeds/,
  );
  await assert.rejects(
    decodeCompressedFrame(
      packet(deflateSync(new Uint8Array(8 * 1024 * 1024))).bytes,
      generation,
      signal(),
    ),
    /exceeds/,
  );
});

test("deflated frames honor cancellation before and during a pending decode", async () => {
  const bytes = packet(deflateSync(pixels)).bytes;
  for (const before of [true, false]) {
    const abort = new AbortController();
    const output = new Uint8Array(pixels.length).fill(83);
    if (before) abort.abort();
    const pending = decodeCompressedFrame(
      bytes,
      generation,
      abort.signal,
      output,
    );
    if (!before) abort.abort();
    await assert.rejects(
      pending,
      (reason: unknown) =>
        reason instanceof DOMException && reason.name === "AbortError",
    );
    assert.ok(output.every((value) => value === 83));
  }
});

test("decoder feeds at most 4096 compressed bytes per transform and aborts upstream on overflow", async () => {
  const original = globalThis.DecompressionStream;
  const chunks: number[] = [];
  let cancelled = 0;
  let aborted = 0;
  globalThis.DecompressionStream = class {
    readable: ReadableStream<Uint8Array>;
    writable: WritableStream<Uint8Array>;
    constructor() {
      let output: ReadableStreamDefaultController<Uint8Array>;
      this.readable = new ReadableStream({
        start(controller) {
          output = controller;
        },
        cancel() {
          cancelled++;
        },
      });
      this.writable = new WritableStream({
        write(chunk) {
          chunks.push(chunk.length);
          output.enqueue(new Uint8Array(pixels.length + 1));
        },
        abort() {
          aborted++;
        },
      });
    }
  } as unknown as typeof DecompressionStream;
  try {
    await assert.rejects(
      decodeCompressedFrame(
        packet(new Uint8Array(16384)).bytes,
        generation,
        signal(),
      ),
      /exceeds/,
    );
    await new Promise<void>((resolve) => setImmediate(resolve));
    assert.ok(chunks.length > 0);
    assert.ok(chunks.every((size) => size <= 4096));
    assert.ok(chunks.reduce((total, size) => total + size, 0) < 16384);
    assert.equal(cancelled, 1);
    assert.equal(aborted, 1);
  } finally {
    globalThis.DecompressionStream = original;
  }
});

const turn = () => new Promise<void>((resolve) => setImmediate(resolve));
test(
  "writer rejection wakes an independently pending reader and releases both locks",
  { timeout: 5000 },
  async () => {
    const original = globalThis.DecompressionStream,
      error = new Error("write failed");
    let streams:
      | {
          readable: ReadableStream<Uint8Array>;
          writable: WritableStream<Uint8Array>;
        }
      | undefined;
    let cancelled = 0;
    globalThis.DecompressionStream = class {
      readable = new ReadableStream<Uint8Array>({
        cancel() {
          cancelled++;
        },
      });
      writable = new WritableStream<Uint8Array>({
        write() {
          throw error;
        },
      });
      constructor() {
        streams = this;
      }
    } as unknown as typeof DecompressionStream;
    try {
      await assert.rejects(
        decodeCompressedFrame(
          packet(new Uint8Array(8), 16, 16).bytes,
          generation,
          new AbortController().signal,
        ),
        (reason) => reason === error,
      );
      assert.equal(cancelled, 1);
      assert.equal(streams!.readable.locked, false);
      assert.equal(streams!.writable.locked, false);
    } finally {
      globalThis.DecompressionStream = original;
    }
  },
);
test(
  "abort waits for pending write and cancellation to settle before releasing borrowed input",
  { timeout: 5000 },
  async () => {
    const original = globalThis.DecompressionStream;
    let finishWrite!: () => void, finishCancel!: () => void;
    let started = false,
      settled = false,
      aborted = 0;
    let streams:
      | {
          readable: ReadableStream<Uint8Array>;
          writable: WritableStream<Uint8Array>;
        }
      | undefined;
    globalThis.DecompressionStream = class {
      readable = new ReadableStream<Uint8Array>({
        cancel() {
          return new Promise<void>((resolve) => {
            finishCancel = resolve;
          });
        },
      });
      writable = new WritableStream<Uint8Array>({
        write() {
          started = true;
          return new Promise<void>((resolve) => {
            finishWrite = resolve;
          });
        },
        abort() {
          aborted++;
        },
      });
      constructor() {
        streams = this;
      }
    } as unknown as typeof DecompressionStream;
    try {
      const controller = new AbortController();
      const pending = decodeCompressedFrame(
        packet(new Uint8Array(8), 16, 16).bytes,
        generation,
        controller.signal,
      );
      const consumed = pending.then(
        () => {
          settled = true;
        },
        () => {
          settled = true;
        },
      );
      await turn();
      assert(started);
      controller.abort();
      await turn();
      assert.equal(settled, false);
      finishWrite();
      await turn();
      assert.equal(settled, false);
      finishCancel();
      await assert.rejects(
        pending,
        (e: unknown) => e instanceof DOMException && e.name === "AbortError",
      );
      await consumed;
      assert.equal(aborted, 1);
      assert.equal(streams!.readable.locked, false);
      assert.equal(streams!.writable.locked, false);
    } finally {
      globalThis.DecompressionStream = original;
    }
  },
);
test(
  "writer close rejection is consumed and cancels the read side",
  { timeout: 5000 },
  async () => {
    const original = globalThis.DecompressionStream,
      error = new Error("close failed");
    let output!: ReadableStreamDefaultController<Uint8Array>;
    let cancelled = 0;
    globalThis.DecompressionStream = class {
      readable = new ReadableStream<Uint8Array>({
        start(c) {
          output = c;
        },
        cancel() {
          cancelled++;
        },
      });
      writable = new WritableStream<Uint8Array>({
        write() {
          output.enqueue(new Uint8Array(1024));
        },
        close() {
          throw error;
        },
      });
    } as unknown as typeof DecompressionStream;
    try {
      await assert.rejects(
        decodeCompressedFrame(
          packet(new Uint8Array(8), 16, 16).bytes,
          generation,
          new AbortController().signal,
        ),
        (reason) => reason === error,
      );
      assert.equal(cancelled, 1);
    } finally {
      globalThis.DecompressionStream = original;
    }
  },
);

test("deflate corruption and flush truncation across the 4096-byte input boundary reject", async () => {
  const source = new Uint8Array(128 * 128 * 4);
  let random = 0x12345678;
  for (let i = 0; i < 8192; i++) {
    random ^= random << 13;
    random ^= random >>> 17;
    random ^= random << 5;
    source[i] = random & 255;
  }
  const encoded = deflateSync(source);
  assert.ok(encoded.length > 4097 && encoded.length <= source.length);
  const good = await decodeCompressedFrame(
    packet(encoded, 128, 128).bytes,
    generation,
    signal(),
  );
  assert.deepEqual(good.pixels, source);
  const corrupt = Uint8Array.from(encoded);
  corrupt[4096] ^= 0xff;
  for (const payload of [
    corrupt,
    encoded.subarray(0, 4096),
    encoded.subarray(0, 4097),
    encoded.subarray(0, -1),
  ]) {
    await assert.rejects(
      decodeCompressedFrame(
        packet(payload, 128, 128).bytes,
        generation,
        signal(),
      ),
    );
  }
});

function rlePayload(...parts: (number | readonly number[])[]) {
  const bytes: number[] = [];
  for (const part of parts) {
    if (typeof part === "number") {
      const word = new Uint8Array(4);
      new DataView(word.buffer).setUint32(0, part, true);
      bytes.push(...word);
    } else bytes.push(...part);
  }
  return Uint8Array.from(bytes);
}

test("RLE32 mixed pixels preserve channels, identity and exact owned scratch reuse", () => {
  const literal = [1, 128, 255, 37, 77, 23, 9, 4],
    repeat = [31, 200, 17, 99];
  const encoded = rlePayload(2, literal, 0x80000000 + 4094, repeat);
  const expected = new Uint8Array(64 * 64 * 4);
  expected.set(literal);
  for (let i = 8; i < expected.length; i += 4) expected.set(repeat, i);
  const { bytes } = packet(encoded, 64, 64, 2);
  const output = new Uint8Array(expected.length);
  const decoded = decodeRLE32Frame(bytes, generation, signal(), output);
  assert.equal(decoded.pixels, output);
  assert.deepEqual(decoded.pixels, expected);
  assert.deepEqual(
    [decoded.epoch, decoded.sequence, decoded.timestampUs, decoded.rotation],
    [7, 123, 987654n, 2],
  );
  assert.throws(() => decodeFrame(bytes, generation));
  const changed = packet(rlePayload(0x80001000, [9, 8, 7, 6]), 64, 64, 2).bytes;
  assert.equal(
    decodeRLE32Frame(changed, generation, signal(), output).pixels,
    output,
  );
  assert.deepEqual(output.subarray(0, 4), new Uint8Array([9, 8, 7, 6]));
  for (const invalid of [
    new Uint8Array(output.length - 1),
    new Uint8Array(output.length + 1).subarray(1),
    new Uint8Array(output.length + 1).subarray(0, output.length),
  ]) {
    const frame = decodeRLE32Frame(bytes, generation, signal(), invalid);
    assert.notEqual(frame.pixels.buffer, invalid.buffer);
    assert.deepEqual(frame.pixels, expected);
  }
  const tiny = packet(rlePayload(0x80000100, [1, 2, 3, 4]), 16, 16, 2).bytes;
  const saved = new Uint8Array(tiny).slice();
  const frame = decodeRLE32Frame(
    tiny,
    generation,
    signal(),
    new Uint8Array(tiny),
  );
  assert.notEqual(frame.pixels.buffer, tiny);
  assert.deepEqual(new Uint8Array(tiny), saved);
});

test("RLE32 rejects malformed, overflowing, trailing and incomplete pixel runs", () => {
  for (const payload of [
    rlePayload(0, [1, 2, 3, 4]),
    rlePayload(0xffffffff, [1, 2, 3, 4]),
    new Uint8Array([1, 0, 0]),
    rlePayload(0x80001000, [1, 2, 3]),
    rlePayload(4096, [1, 2, 3, 4]),
    rlePayload(0x80000fff, [1, 2, 3, 4]),
    rlePayload(0x80001000, [1, 2, 3, 4], 0x80000001, [5, 6, 7, 8]),
  ])
    assert.throws(() =>
      decodeRLE32Frame(packet(payload, 64, 64, 2).bytes, generation, signal()),
    );
  const encoded = rlePayload(0x80001000, [1, 2, 3, 4]);
  for (const [offset, value] of [
    [0, 0],
    [4, 3],
    [48, 1281],
    [52, 0],
    [56, 4],
    [60, 1],
    [64, 0],
    [64, 16385],
  ] as const) {
    const { bytes, header } = packet(encoded, 64, 64, 2);
    header.setUint32(offset, value, true);
    const output = new Uint8Array(64 * 64 * 4).fill(83);
    assert.throws(() => decodeRLE32Frame(bytes, generation, signal(), output));
    assert(output.every((v) => v === 83));
  }
  const padded = packet(encoded, 64, 64, 2).bytes;
  new Uint8Array(padded)[padded.byteLength - 1] = 1;
  assert.throws(
    () => decodeRLE32Frame(padded, generation, signal()),
    /padding/,
  );
  assert.throws(
    () =>
      decodeRLE32Frame(
        packet(encoded, 64, 64, 2).bytes,
        new Uint8Array(16),
        signal(),
      ),
    /old process/,
  );
  const zeroEpoch = packet(encoded, 64, 64, 2);
  zeroEpoch.header.setBigUint64(24, 0n, true);
  assert.throws(() => decodeRLE32Frame(zeroEpoch.bytes, generation, signal()));
});

test("RLE32 bounds maximum frames and cancellation without returning stale pixels", () => {
  const count = 720 * 1280,
    encoded = rlePayload(0x80000000 + count, [7, 128, 255, 9]);
  const bytes = packet(encoded, 720, 1280, 2).bytes;
  const abort = new AbortController();
  abort.abort();
  const output = new Uint8Array(count * 4).fill(83);
  assert.throws(
    () => decodeRLE32Frame(bytes, generation, abort.signal, output),
    { name: "AbortError" },
  );
  assert(output.every((v) => v === 83));
  const decoded = decodeRLE32Frame(bytes, generation, signal(), output);
  assert.equal(decoded.pixels, output);
  for (let i = 0; i < output.length; i++)
    assert.equal(output[i], [7, 128, 255, 9][i % 4]);
  assert.throws(() =>
    decodeRLE32Frame(
      packet(encoded, 1280, 1280, 2).bytes,
      generation,
      signal(),
    ),
  );
});
