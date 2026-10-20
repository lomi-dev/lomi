import { api, errorMessage } from "../api";
import { reportAndroidError } from "./service";
import { touchPoint, type AndroidFrame } from "./frame";
import type { AndroidRenderer } from "./canvas";
import type { Status } from "./types";
import { TextCommitAssembler, wellFormedInputText } from "./text-commit";

export type InputEvent =
  | { type: "key"; key: string; down: boolean }
  | { type: "navigation"; key: "GoBack" | "GoHome" | "AppSwitch" | "Power" }
  | {
      type: "touch";
      identifier: number;
      x: number;
      y: number;
      phase: "down" | "move" | "up";
    }
  | {
      type: "text";
      action: "compose" | "commit" | "finish" | "delete";
      text: string;
    }
  | { type: "paste"; text: string }
  | { type: "rotate"; quarterTurns: number }
  | { type: "settings" };
interface Focus {
  id: string;
  deviceId: string;
  generation: string;
  epoch: number;
  commits: TextCommitAssembler;
}
interface Owner extends Focus {
  lease: string;
  sequence: number;
}
interface Pending {
  focus: Focus;
  event: InputEvent;
}
let desired: Focus | undefined;
let owner: Owner | undefined;
let epoch = 0;
let draining: Promise<void> | undefined;
const queue: Pending[] = [];
let bytes = 0;
const cost = (event: InputEvent) =>
  "text" in event ? new TextEncoder().encode(event.text).length : 0;

function send(owner: Focus, input: object) {
  return api<{ lease: string | null; sequence: number }>("android_input", {
    deviceId: owner.deviceId,
    generation: owner.generation,
    input,
  });
}
async function drain() {
  if (draining) return draining;
  draining = Promise.resolve().then(async () => {
    try {
      for (;;) {
        if (owner && owner.epoch !== desired?.epoch) {
          const previous = owner;
          await send(previous, { type: "blur", lease: previous.lease });
          owner = undefined;
        }
        if (desired && owner?.epoch !== desired.epoch) {
          const focus = desired;
          const reply = await send(focus, { type: "focus", viewId: focus.id });
          if (!reply.lease)
            throw new Error(
              "Android did not grant input focus. Focus the phone again.",
            );
          owner = { ...focus, lease: reply.lease, sequence: 0 };
          continue;
        }
        const pending = queue.shift();
        if (!pending) break;
        bytes -= cost(pending.event);
        if (!owner || pending.focus.epoch !== owner.epoch) continue;
        await send(owner, {
          type: "send",
          lease: owner.lease,
          sequence: ++owner.sequence,
          event: pending.event,
        });
      }
    } catch (reason) {
      desired?.commits.reset();
      desired = undefined;
      queue.length = 0;
      bytes = 0;
      // A lost reply can follow a delivered down event. Always release the lease;
      // never replay text or infer that a failed invoke rolled back guest input.
      if (owner) {
        const previous = owner;
        owner = undefined;
        await send(previous, { type: "blur", lease: previous.lease }).catch(
          () => {},
        );
      }
      reportAndroidError(errorMessage(reason));
    } finally {
      draining = undefined;
    }
  });
  return draining;
}
export function focusInput(id: string, deviceId: string, generation: string) {
  if (
    desired?.id === id &&
    desired.deviceId === deviceId &&
    desired.generation === generation
  )
    return;
  desired?.commits.reset();
  desired = {
    id,
    deviceId,
    generation,
    epoch: ++epoch,
    commits: new TextCommitAssembler(),
  };
  queue.length = 0;
  bytes = 0;
  void drain();
}
export async function releaseInput(viewId?: string) {
  if (viewId && desired?.id !== viewId && owner?.id !== viewId) return;
  desired?.commits.reset();
  desired = undefined;
  queue.length = 0;
  bytes = 0;
  await drain();
}
export async function pasteInput(id: string, read: () => Promise<string>) {
  const focus = desired;
  if (!focus || focus.id !== id) return;
  focus.commits.reset();
  const text = await read();
  // Native clipboard reads can finish after a tab switch or a new focus lease.
  if (desired === focus) enqueueInput(id, { type: "paste", text });
}
export function enqueueInput(id: string, event: InputEvent) {
  if (!desired || desired.id !== id) return;
  // Non-text input cannot connect two halves from unrelated editing actions.
  if (event.type !== "text" || event.action !== "commit")
    desired.commits.reset();
  if ("text" in event)
    event = { ...event, text: wellFormedInputText(event.text) };
  const size = cost(event);
  const last = queue[queue.length - 1];
  if (
    event.type === "touch" &&
    event.phase === "move" &&
    last?.event.type === "touch" &&
    last.event.phase === "move" &&
    last.event.identifier === event.identifier &&
    last.focus.epoch === desired.epoch
  ) {
    last.event = event;
    return;
  }
  if (
    event.type === "text" &&
    event.action === "compose" &&
    last?.event.type === "text" &&
    last.event.action === "compose"
  ) {
    bytes -= cost(last.event);
    queue.pop();
  }
  if (queue.length >= 64 || bytes + size > 32768 || size > 16384) {
    void releaseInput();
    reportAndroidError(
      "Android input exceeded its queue limit. Gesture release was requested; focus the phone and retry with less text.",
    );
    return;
  }
  queue.push({ focus: desired, event });
  bytes += size;
  void drain();
}

export function attachInput(
  view: {
    id: string;
    deviceId: string;
    input: HTMLTextAreaElement;
    host: HTMLElement;
    renderer?: Pick<AndroidRenderer, "element">;
    metadata?: Pick<AndroidFrame, "width" | "height" | "rotation">;
    onFocus: () => void;
  },
  status: Status,
) {
  const input = view.input,
    canvas = view.renderer!.element;
  const controller = new AbortController();
  const options = { signal: controller.signal };
  let composing = false;
  let compositionCommit: string | undefined;
  let touch: { id: number; x: number; y: number; second: boolean } | undefined;
  let wheel = 0;
  let wheelPosition: { clientX: number; clientY: number } | undefined;
  const keys = new Set<string>();
  const send = (event: InputEvent) => enqueueInput(view.id, event);
  const resetText = () => {
    if (desired?.id === view.id) desired.commits.reset();
  };
  const focused = () =>
    document.activeElement === input && !document.querySelector("dialog[open]");
  const focus = () => {
    resetText();
    view.onFocus();
    focusInput(view.id, view.deviceId, status.generation!);
  };
  const keyboardFocus = () => {
    input.focus({ preventScroll: true });
    focus();
  };
  input.value = " ";
  input.setSelectionRange(1, 1);
  const clear = () => {
    input.value = " ";
    input.setSelectionRange(1, 1);
  };
  const endTouch = () => {
    wheelPosition = undefined;
    if (!touch) return;
    const previous = touch;
    touch = undefined;
    send({
      type: "touch",
      identifier: 0,
      x: previous.x,
      y: previous.y,
      phase: "up",
    });
    if (previous.second)
      send({
        type: "touch",
        identifier: 1,
        x: status.display![0] - 1 - previous.x,
        y: status.display![1] - 1 - previous.y,
        phase: "up",
      });
    if (canvas.hasPointerCapture(previous.id))
      canvas.releasePointerCapture(previous.id);
  };
  const release = () => {
    resetText();
    clearTimeout(wheel);
    wheel = 0;
    wheelPosition = undefined;
    const previous = touch;
    touch = undefined;
    if (previous && canvas.hasPointerCapture(previous.id))
      canvas.releasePointerCapture(previous.id);
    keys.clear();
    composing = false;
    compositionCommit = undefined;
    void releaseInput(view.id);
  };
  input.addEventListener("focus", focus, options);
  input.addEventListener("blur", release, options);
  canvas.addEventListener("focus", keyboardFocus, options);
  input.addEventListener(
    "compositionstart",
    () => {
      resetText();
      composing = true;
      compositionCommit = undefined;
    },
    options,
  );
  input.addEventListener(
    "compositionupdate",
    (event) => {
      if (focused())
        send({ type: "text", action: "compose", text: event.data ?? "" });
    },
    options,
  );
  input.addEventListener(
    "compositionend",
    (event) => {
      resetText();
      composing = false;
      compositionCommit = event.data ?? "";
      if (focused())
        send({ type: "text", action: "commit", text: compositionCommit });
      clear();
      queueMicrotask(() => {
        compositionCommit = undefined;
      });
    },
    options,
  );
  input.addEventListener(
    "beforeinput",
    (event) => {
      if (
        !focused() ||
        composing ||
        event.isComposing ||
        event.inputType === "insertFromComposition"
      ) {
        resetText();
        return;
      }
      event.preventDefault();
      if (
        event.inputType === "insertText" &&
        event.data !== compositionCommit
      ) {
        const text =
          desired?.id === view.id
            ? desired.commits.append(event.data ?? "")
            : "";
        if (text) send({ type: "text", action: "commit", text });
      } else {
        resetText();
        if (
          event.inputType === "deleteContentBackward" &&
          !keys.has("Backspace")
        )
          send({ type: "text", action: "delete", text: "" });
      }
      clear();
    },
    options,
  );
  input.addEventListener(
    "paste",
    (event) => {
      resetText();
      event.preventDefault();
      if (focused())
        send({
          type: "paste",
          text: event.clipboardData?.getData("text/plain") ?? "",
        });
    },
    options,
  );
  input.addEventListener(
    "keydown",
    (event) => {
      if (
        event.defaultPrevented ||
        event.isComposing ||
        composing ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        event.key.length !== 1
      )
        resetText();
      if (
        !focused() ||
        event.defaultPrevented ||
        event.isComposing ||
        composing
      )
        return;
      if (
        event.key === "Tab" &&
        !event.ctrlKey &&
        !event.altKey &&
        !event.metaKey
      )
        return;
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "v")
        return;
      const control = [
        "Enter",
        "Escape",
        "Backspace",
        "Delete",
        "ArrowLeft",
        "ArrowRight",
        "ArrowUp",
        "ArrowDown",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "Tab",
      ].includes(event.key);
      if (control) {
        event.preventDefault();
        for (const [key, pressed] of [
          ["Shift", event.shiftKey],
          ["Control", event.ctrlKey],
          ["Alt", event.altKey],
          ["Meta", event.metaKey],
        ] as const) {
          if (pressed && !keys.has(key)) {
            keys.add(key);
            send({ type: "key", key, down: true });
          }
        }
        if (keys.has(event.key))
          send({ type: "key", key: event.key, down: false });
        keys.add(event.key);
        send({ type: "key", key: event.key, down: true });
      } else if (
        (event.ctrlKey || event.metaKey) &&
        event.key.length === 1 &&
        /^[\x20-\x7e]$/.test(event.key)
      ) {
        event.preventDefault();
        const modifier = event.ctrlKey ? "Control" : "Meta";
        send({ type: "key", key: modifier, down: true });
        send({ type: "key", key: event.key, down: true });
        send({ type: "key", key: event.key, down: false });
        send({ type: "key", key: modifier, down: false });
      }
    },
    options,
  );
  input.addEventListener(
    "keyup",
    (event) => {
      if (keys.delete(event.key)) {
        event.preventDefault();
        send({ type: "key", key: event.key, down: false });
      }
    },
    options,
  );
  const point = (
    event: { clientX: number; clientY: number },
    clamp: boolean,
  ) =>
    view.metadata && status.display
      ? touchPoint(
          canvas.getBoundingClientRect(),
          view.metadata,
          status.display,
          event.clientX,
          event.clientY,
          clamp,
        )
      : undefined;
  canvas.addEventListener(
    "pointerdown",
    (event) => {
      if (
        event.button !== 0 ||
        !event.isPrimary ||
        event.ctrlKey ||
        event.metaKey
      )
        return;
      const position = point(event, false);
      if (!position) return;
      event.preventDefault();
      endTouch();
      keyboardFocus();
      canvas.setPointerCapture(event.pointerId);
      touch = { ...position, id: event.pointerId, second: event.altKey };
      send({ type: "touch", identifier: 0, ...position, phase: "down" });
      if (touch.second)
        send({
          type: "touch",
          identifier: 1,
          x: status.display![0] - 1 - position.x,
          y: status.display![1] - 1 - position.y,
          phase: "down",
        });
    },
    options,
  );
  canvas.addEventListener(
    "pointermove",
    (event) => {
      if (touch?.id !== event.pointerId) return;
      const position = point(event, true);
      if (!position) return;
      Object.assign(touch, position);
      send({ type: "touch", identifier: 0, ...position, phase: "move" });
      if (touch.second)
        send({
          type: "touch",
          identifier: 1,
          x: status.display![0] - 1 - position.x,
          y: status.display![1] - 1 - position.y,
          phase: "move",
        });
    },
    options,
  );
  for (const type of [
    "pointerup",
    "pointercancel",
    "lostpointercapture",
  ] as const)
    canvas.addEventListener(
      type,
      (event) => {
        if (touch?.id === event.pointerId) endTouch();
      },
      options,
    );
  canvas.addEventListener(
    "wheel",
    (event) => {
      if (event.defaultPrevented || event.ctrlKey || event.metaKey) return;
      const position = point(event, false);
      if (!position || (touch && touch.id !== -1)) return;
      event.preventDefault();
      keyboardFocus();
      if (!touch) {
        wheelPosition = { clientX: event.clientX, clientY: event.clientY };
        touch = { ...position, id: -1, second: false };
        send({ type: "touch", identifier: 0, ...position, phase: "down" });
      }
      const factor =
        event.deltaMode === 1
          ? 16
          : event.deltaMode === 2
            ? view.host.clientHeight
            : 1;
      wheelPosition = {
        clientX:
          wheelPosition!.clientX -
          Math.max(-120, Math.min(120, event.deltaX * factor)),
        clientY:
          wheelPosition!.clientY -
          Math.max(-120, Math.min(120, event.deltaY * factor)),
      };
      const bounds = canvas.getBoundingClientRect();
      wheelPosition.clientX = Math.max(
        bounds.left,
        Math.min(bounds.right - 1, wheelPosition.clientX),
      );
      wheelPosition.clientY = Math.max(
        bounds.top,
        Math.min(bounds.bottom - 1, wheelPosition.clientY),
      );
      const destination = point(wheelPosition, true);
      if (destination) {
        Object.assign(touch, destination);
        send({ type: "touch", identifier: 0, ...destination, phase: "move" });
      }
      clearTimeout(wheel);
      wheel = window.setTimeout(() => {
        wheel = 0;
        endTouch();
      }, 100);
    },
    { ...options, passive: false },
  );
  window.addEventListener("blur", release, options);
  return Object.assign(
    () => {
      controller.abort();
      release();
    },
    {
      endGesture: () => {
        clearTimeout(wheel);
        wheel = 0;
        endTouch();
      },
    },
  );
}
