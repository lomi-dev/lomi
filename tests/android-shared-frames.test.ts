import assert from "node:assert/strict";
import { test } from "node:test";
import {
  ownFrame,
  SharedFrameBroker,
  SHARED_FRAME_PROTOCOL,
  type FramePacket,
  type SharedFrameBridge,
} from "../src/android/shared-frames.ts";

const identity = {
  protocol: SHARED_FRAME_PROTOCOL,
  token: "subscription",
  deviceId: "device",
  generation: "generation",
};

function fixture(receive: (packet: FramePacket) => void) {
  let listener: Parameters<SharedFrameBridge["addEventListener"]>[1];
  const released: ArrayBuffer[] = [];
  const errors: unknown[] = [];
  const broker = new SharedFrameBroker({
    addEventListener(name, callback) {
      assert.equal(name, "sharedbufferreceived");
      listener = callback;
    },
    releaseBuffer(bytes) {
      released.push(bytes);
    },
  });
  const receiver = {
    deviceId: identity.deviceId,
    generation: identity.generation,
    receive,
    onError: (reason: unknown) => errors.push(reason),
  };
  const dispose = broker.subscribe(identity.token, receiver);
  return {
    broker,
    receiver,
    dispose,
    released,
    errors,
    emit(data: unknown = identity, bytes = new ArrayBuffer(1024)) {
      let reads = 0;
      listener!({
        additionalData: data,
        getBuffer() {
          reads++;
          return bytes;
        },
      });
      return { bytes, reads };
    },
  };
}

test("shared Android frames stay owned through drawing and release once", () => {
  let pending: FramePacket | undefined;
  const f = fixture((packet) => {
    pending = packet;
  });
  const { bytes, reads } = f.emit();
  assert.equal(reads, 1);
  assert.equal(pending?.bytes, bytes);
  assert.deepEqual(f.released, []);
  pending!.release();
  pending!.release();
  assert.deepEqual(f.released, [bytes]);
  assert.deepEqual(f.errors, []);
});

test("retired or unknown subscriptions release late buffers without drawing", () => {
  const f = fixture(() => assert.fail("A retired frame was drawn"));
  f.dispose();
  const first = f.emit();
  const second = f.emit({ ...identity, token: "unknown" });
  assert.deepEqual(f.released, [first.bytes, second.bytes]);
  assert.deepEqual(f.errors, []);
  assert.equal(f.emit({ protocol: "another-feature" }).reads, 0);
});

test("shared frames reject another generation, device or invalid allocation", () => {
  const f = fixture(() => assert.fail("An invalid frame was drawn"));
  const rejected = [
    f.emit({ ...identity, generation: "old" }).bytes,
    f.emit({ ...identity, deviceId: "another" }).bytes,
    f.emit(identity, new ArrayBuffer(68)).bytes,
    f.emit(identity, new ArrayBuffer(68 + 720 * 1280 * 4 + 1)).bytes,
  ];
  assert.deepEqual(f.released, rejected);
  assert.equal(f.errors.length, 4);
});

test("a failed consumer releases its frame and does not poison later delivery", () => {
  let count = 0;
  const f = fixture((packet) => {
    if (count++ === 0) throw new Error("draw failed");
    packet.release();
  });
  const first = f.emit();
  const second = f.emit();
  assert.deepEqual(f.released, [first.bytes, second.bytes]);
  assert.equal(f.errors.length, 1);
  assert.match(String(f.errors[0]), /draw failed/);
});

test("release failures are reported once and ownership cannot be released twice", () => {
  const errors: unknown[] = [];
  let releases = 0;
  const packet = ownFrame(
    new ArrayBuffer(1024),
    () => {
      releases++;
      throw new Error("mapping invalidated");
    },
    (reason) => errors.push(reason),
  );
  packet.release();
  packet.release();
  assert.equal(releases, 1);
  assert.equal(errors.length, 1);
});

test("old cleanup cannot remove a replacement receiver", () => {
  let count = 0;
  const f = fixture((packet) => {
    count++;
    packet.release();
  });
  assert.throws(() => f.broker.subscribe(identity.token, f.receiver));
  f.dispose();
  const replacement = { ...f.receiver };
  const disposeReplacement = f.broker.subscribe(identity.token, replacement);
  f.dispose();
  f.emit();
  assert.equal(count, 1);
  disposeReplacement();
  f.emit();
  assert.equal(count, 1);
});
