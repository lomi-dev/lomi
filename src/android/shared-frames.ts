import { ANDROID_FRAME_HEADER, MAX_ANDROID_PIXELS } from "./frame.ts";

export const SHARED_FRAME_PROTOCOL = "lomi.android.frame.v1";

export interface FramePacket {
  bytes: ArrayBuffer;
  release: () => void;
}

/** Release once, including cancellation before the scheduled draw. */
export function ownFrame(
  bytes: ArrayBuffer,
  release: (bytes: ArrayBuffer) => void,
  onError: (reason: unknown) => void,
): FramePacket {
  let released = false;
  return {
    bytes,
    release() {
      if (released) return;
      released = true;
      try {
        release(bytes);
      } catch (reason) {
        onError(reason);
      }
    },
  };
}

interface SharedBufferEvent {
  additionalData: unknown;
  getBuffer: () => ArrayBuffer;
}

export interface SharedFrameBridge {
  addEventListener: (
    name: "sharedbufferreceived",
    listener: (event: SharedBufferEvent) => void,
  ) => void;
  releaseBuffer: (bytes: ArrayBuffer) => void;
}

interface Receiver {
  deviceId: string;
  generation: string;
  receive: (packet: FramePacket) => void;
  onError: (reason: unknown) => void;
}

/** One permanent receiver also releases deliveries for retired subscriptions. */
export class SharedFrameBroker {
  private receivers = new Map<string, Receiver>();

  constructor(bridge: SharedFrameBridge) {
    bridge.addEventListener("sharedbufferreceived", (event) => {
      const data = event.additionalData;
      if (
        !data ||
        typeof data !== "object" ||
        !("protocol" in data) ||
        data.protocol !== SHARED_FRAME_PROTOCOL
      )
        return;
      const receiver =
        "token" in data && typeof data.token === "string"
          ? this.receivers.get(data.token)
          : undefined;
      let packet: FramePacket | undefined;
      try {
        const bytes = event.getBuffer();
        packet = ownFrame(
          bytes,
          (buffer) => bridge.releaseBuffer(buffer),
          (reason) => receiver?.onError(reason),
        );
        if (!receiver) {
          packet.release();
          return;
        }
        if (
          !("deviceId" in data) ||
          data.deviceId !== receiver.deviceId ||
          !("generation" in data) ||
          data.generation !== receiver.generation ||
          !(bytes instanceof ArrayBuffer) ||
          bytes.byteLength < 1024 ||
          bytes.byteLength > ANDROID_FRAME_HEADER + MAX_ANDROID_PIXELS * 4
        )
          throw new Error("Invalid shared Android frame. Reconnect the panel.");
        receiver.receive(packet);
      } catch (reason) {
        packet?.release();
        receiver?.onError(reason);
      }
    });
  }

  subscribe(token: string, receiver: Receiver) {
    if (this.receivers.has(token))
      throw new Error("Android frame subscription already exists.");
    this.receivers.set(token, receiver);
    return () => {
      if (this.receivers.get(token) === receiver) this.receivers.delete(token);
    };
  }
}
