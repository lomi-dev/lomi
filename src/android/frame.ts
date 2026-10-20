export const MAX_ANDROID_PIXELS = 720 * 1280;
export const MAX_ANDROID_DISPLAY_EDGE = 4096;
export const MAX_ANDROID_DISPLAY_PIXELS = 3840 * 2160;

/** Bound the source before IPC without changing the guest's screen or density. */
export function previewSize(width: number, height: number) {
  if (![width, height].every((n) => Number.isFinite(n) && n > 0))
    return { width: 1, height: 1 };
  const scale = Math.min(
    1,
    1280 / width,
    1280 / height,
    Math.sqrt(MAX_ANDROID_PIXELS / (width * height)),
  );
  return {
    width: Math.max(1, Math.floor(width * scale)),
    height: Math.max(1, Math.floor(height * scale)),
  };
}
export const ANDROID_FRAME_HEADER = 68;

export interface AndroidFrame {
  epoch: number;
  sequence: number;
  timestampUs: bigint;
  width: number;
  height: number;
  rotation: 0 | 1 | 2 | 3;
  pixels: Uint8Array<ArrayBuffer>;
}

export function generationBytes(generation: string): Uint8Array {
  if (
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(
      generation,
    )
  )
    throw new Error("Invalid Android process generation.");
  return Uint8Array.from(generation.replaceAll("-", "").match(/../g)!, (pair) =>
    parseInt(pair, 16),
  );
}

function frameHeader(
  bytes: ArrayBuffer,
  generation: Uint8Array,
  expectedEncoding: 0 | 1 | 2,
) {
  if (
    !(bytes instanceof ArrayBuffer) ||
    bytes.byteLength < 1024 ||
    bytes.byteLength > ANDROID_FRAME_HEADER + MAX_ANDROID_PIXELS * 4
  )
    throw new Error("Invalid Android binary frame size. Reconnect the panel.");
  const header = new DataView(bytes);
  if (
    header.getUint32(0, true) !== 0x50414253 ||
    header.getUint32(4, true) !== 4 ||
    generation.length !== 16
  )
    throw new Error("Unsupported Android frame protocol. Reconnect the panel.");
  for (let i = 0; i < 16; i++) {
    if (header.getUint8(8 + i) !== generation[i])
      throw new Error("Android frame belongs to an old process.");
  }
  const epoch = Number(header.getBigUint64(24, true));
  const sequence = Number(header.getBigUint64(32, true));
  const width = header.getUint32(48, true);
  const height = header.getUint32(52, true);
  const rotation = header.getUint32(56, true);
  const encoding = header.getUint32(60, true);
  const length = header.getUint32(64, true);
  if (
    !Number.isSafeInteger(epoch) ||
    !Number.isSafeInteger(sequence) ||
    epoch < 1 ||
    sequence < 1 ||
    rotation > 3 ||
    encoding !== expectedEncoding ||
    width > 1280 ||
    height > 1280 ||
    width * height > MAX_ANDROID_PIXELS ||
    (width === 0) !== (height === 0) ||
    (encoding === 0
      ? length !== width * height * 4
      : length < 1 || length > width * height * 4) ||
    Math.max(ANDROID_FRAME_HEADER + length, 1024) !== bytes.byteLength
  )
    throw new Error(
      "Invalid Android frame dimensions or metadata. Reconnect the panel.",
    );
  const padding = new Uint8Array(bytes, ANDROID_FRAME_HEADER + length);
  if (padding.some((value) => value !== 0))
    throw new Error("Invalid Android frame padding. Reconnect the panel.");
  return {
    epoch,
    sequence,
    timestampUs: header.getBigUint64(40, true),
    width,
    height,
    rotation: rotation as AndroidFrame["rotation"],
    length,
  };
}

/** Borrows the binary IPC allocation; no pixel copying or JSON conversion. */
export function decodeFrame(
  bytes: ArrayBuffer,
  generation: Uint8Array,
): AndroidFrame {
  const { length, ...frame } = frameHeader(bytes, generation, 0);
  return {
    ...frame,
    pixels: new Uint8Array(bytes, ANDROID_FRAME_HEADER, length),
  };
}

let deflateSupported: boolean | undefined;
export function supportsCompressedFrames() {
  if (deflateSupported !== undefined) return deflateSupported;
  try {
    new DecompressionStream("deflate");
    return (deflateSupported = true);
  } catch {
    return (deflateSupported = false);
  }
}

function ownedPixels(
  bytes: ArrayBuffer,
  expected: number,
  output?: Uint8Array<ArrayBuffer>,
): Uint8Array<ArrayBuffer> {
  return output instanceof Uint8Array &&
    output.buffer instanceof ArrayBuffer &&
    output.byteOffset === 0 &&
    output.byteLength === expected &&
    output.buffer.byteLength === expected &&
    output.buffer !== bytes
    ? output
    : new Uint8Array(expected);
}

const rle32LittleEndian =
  new Uint8Array(new Uint32Array([0x01020304]).buffer)[0] === 4;

/** Decode bounded RGBA pixel runs into owned storage, preserving byte order. */
export function decodeRLE32Frame(
  bytes: ArrayBuffer,
  generation: Uint8Array,
  signal: AbortSignal,
  output?: Uint8Array<ArrayBuffer>,
): AndroidFrame {
  signal.throwIfAborted();
  const { length, ...frame } = frameHeader(bytes, generation, 2);
  const pixels = ownedPixels(bytes, frame.width * frame.height * 4, output);
  const payload = new Uint8Array(bytes, ANDROID_FRAME_HEADER, length);
  const input = new DataView(bytes, ANDROID_FRAME_HEADER, length);
  const words = rle32LittleEndian ? new Uint32Array(pixels.buffer) : null;
  let from = 0,
    to = 0,
    tokens = 0;
  while (from < length) {
    if ((tokens++ & 255) === 0) signal.throwIfAborted();
    if (length - from < 4) throw new Error("Truncated Android RLE32 token.");
    const token = input.getUint32(from, true);
    from += 4;
    const count = token & 0x7fffffff;
    if (!count || count > (pixels.length - to) / 4)
      throw new Error("Invalid Android RLE32 count or pixel bounds.");
    if (token & 0x80000000) {
      if (length - from < 4) throw new Error("Truncated Android RLE32 repeat.");
      const end = to + count * 4;
      if (words) words.fill(input.getUint32(from, true), to / 4, end / 4);
      else {
        const a = payload[from],
          b = payload[from + 1],
          c = payload[from + 2],
          d = payload[from + 3];
        for (let i = to; i < end; i += 4) {
          pixels[i] = a;
          pixels[i + 1] = b;
          pixels[i + 2] = c;
          pixels[i + 3] = d;
        }
      }
      to = end;
      from += 4;
    } else {
      const countBytes = count * 4;
      if (countBytes > length - from)
        throw new Error("Truncated Android RLE32 literal.");
      pixels.set(payload.subarray(from, from + countBytes), to);
      from += countBytes;
      to += countBytes;
    }
  }
  if (to !== pixels.length) throw new Error("Truncated Android RLE32 pixels.");
  signal.throwIfAborted();
  return { ...frame, pixels };
}

/** Inflate into bounded owned storage; no Blob, URL or network path. */
export async function decodeCompressedFrame(
  bytes: ArrayBuffer,
  generation: Uint8Array,
  signal: AbortSignal,
  output?: Uint8Array<ArrayBuffer>,
): Promise<AndroidFrame> {
  signal.throwIfAborted();
  const { length, ...frame } = frameHeader(bytes, generation, 1);
  signal.throwIfAborted();
  const payload = new Uint8Array(bytes, ANDROID_FRAME_HEADER, length);
  const expected = frame.width * frame.height * 4;
  const pixels = ownedPixels(bytes, expected, output);
  const transform = new DecompressionStream("deflate");
  const writer = transform.writable.getWriter();
  const reader = transform.readable.getReader();
  let stopped = false;
  let cancellation: Promise<PromiseSettledResult<unknown>[]> | undefined;
  const stop = (reason?: unknown) => {
    if (stopped) return;
    stopped = true;
    // Cancel the readable first to release transform backpressure, including a
    // pending writer.write. Always retain/consume both cancellation promises.
    cancellation = Promise.allSettled([
      reader.cancel(reason),
      writer.abort(reason),
    ]);
  };
  const cancel = () => stop(signal.reason);
  signal.addEventListener("abort", cancel, { once: true });
  if (signal.aborted) cancel();
  let feedFailed = false;
  let feedFailure: unknown;
  const feeding = (async () => {
    for (let offset = 0; offset < payload.length;) {
      signal.throwIfAborted();
      if (stopped) return;
      // Preserve the native expansion bound: never write more than 4096 bytes.
      const end = Math.min(offset + 4096, payload.length);
      await writer.write(payload.subarray(offset, end));
      offset = end;
    }
    signal.throwIfAborted();
    if (!stopped) await writer.close();
  })().catch((error: unknown) => {
    feedFailed = true;
    feedFailure = error;
    // A rejected writer must also wake a pending reader, even for a codec
    // implementation which does not propagate the writable error itself.
    stop(error);
  });
  let written = 0;
  try {
    for (;;) {
      signal.throwIfAborted();
      const { value, done } = await reader.read();
      signal.throwIfAborted();
      if (done) break;
      if (
        !(value instanceof Uint8Array) ||
        value.length > pixels.length - written
      )
        throw new Error("Android decompressed frame exceeds its pixel bounds.");
      pixels.set(value, written);
      written += value.length;
    }
    await feeding;
    if (feedFailed) throw feedFailure;
    if (written !== pixels.length)
      throw new Error("Android decompressed frame is truncated.");
  } finally {
    signal.removeEventListener("abort", cancel);
    stop();
    // Keep the packet borrowed until every write/close and cancellation settles.
    await feeding;
    await cancellation;
    reader.releaseLock();
    writer.releaseLock();
  }
  signal.throwIfAborted();
  return { ...frame, pixels };
}

/** Coordinates are physical guest pixels in its natural orientation. */
export function touchPoint(
  bounds: { left: number; top: number; width: number; height: number },
  frame: Pick<AndroidFrame, "width" | "height" | "rotation">,
  display: readonly [number, number],
  clientX: number,
  clientY: number,
  clamp: boolean,
): { x: number; y: number } | undefined {
  if (
    !frame.width ||
    !frame.height ||
    display.some(
      (value) =>
        !Number.isInteger(value) ||
        value < 1 ||
        value > MAX_ANDROID_DISPLAY_EDGE,
    ) ||
    display[0] * display[1] > MAX_ANDROID_DISPLAY_PIXELS
  )
    return;
  const scale = Math.min(
    bounds.width / frame.width,
    bounds.height / frame.height,
  );
  if (!Number.isFinite(scale) || scale <= 0) return;
  const left = bounds.left + (bounds.width - frame.width * scale) / 2;
  const top = bounds.top + (bounds.height - frame.height * scale) / 2;
  let u = (clientX - left) / (frame.width * scale);
  let v = (clientY - top) / (frame.height * scale);
  if (
    !Number.isFinite(u) ||
    !Number.isFinite(v) ||
    (!clamp && (u < 0 || v < 0 || u >= 1 || v >= 1))
  )
    return;
  u = Math.max(0, Math.min(1, u));
  v = Math.max(0, Math.min(1, v));
  const point = [
    [u, v],
    [1 - v, u],
    [1 - u, 1 - v],
    [v, 1 - u],
  ][frame.rotation];
  if (!point) return;
  return {
    x: Math.min(display[0] - 1, Math.floor(point[0] * display[0])),
    y: Math.min(display[1] - 1, Math.floor(point[1] * display[1])),
  };
}

/** Release native IPC storage promptly on engines with transferable buffers. */
export function releaseFrame(bytes: ArrayBuffer) {
  const transferable = bytes as ArrayBuffer & {
    transfer?: (size: number) => ArrayBuffer;
  };
  transferable.transfer?.(0);
}
