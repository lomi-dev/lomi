import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errorMessage, native } from "../api";
import type { AndroidTab } from "../model";
import { AndroidCanvas } from "./canvas";
import {
  decodeFrame,
  generationBytes,
  releaseFrame,
  previewSize,
  type AndroidFrame,
} from "./frame";
import { androidSnapshot, observeAndroid, refreshAndroid } from "./state";
import { changeAndroid, reportAndroidError } from "./service";
import { attachInput, releaseInput } from "./view-input";
import {
  boundedZoom,
  relativeZoom,
  screenSize,
  zoomStep,
  type ScreenZoom,
} from "./zoom";

interface View {
  id: string;
  deviceId: string;
  host: HTMLElement;
  input: HTMLTextAreaElement;
  onFocus: () => void;
  overview: boolean;
  zoom: ScreenZoom;
  visible: boolean;
  renderer?: AndroidCanvas;
  generation?: string;
  resize: ResizeObserver;
  cleanupInput?: ReturnType<typeof attachInput>;
  cleanupViewport?: () => void;
  metadata?: Pick<AndroidFrame, "width" | "height" | "rotation">;
}
interface Stream {
  generation: string;
  epoch: number;
  size: string;
  active: boolean;
  frame: number;
  pending?: ArrayBuffer;
  channel: Channel<ArrayBuffer>;
}
const views = new Map<string, View>();
const streams = new Map<string, Stream>();
const visited = new Map<string, string>();
const viewSettings = new Map<
  string,
  { deviceId: string; zoom: ScreenZoom; left: number; top: number }
>();
const restarting = new Set<string>();
const starts = new Map<
  string,
  { promise: Promise<void>; cancelled: boolean }
>();
const stops = new Map<string, Promise<void>>();
let descriptors: AndroidTab[] = [];
let stopEvents: (() => void) | undefined;
let nativeEvents: (() => void) | undefined;
let viewEpoch = 0;
let mainVisible = true;
let scheduled = 0;
const errors = new Map<string, string>();
const observers = new Set<() => void>();
export function subscribeRuntime(callback: () => void) {
  observers.add(callback);
  return () => {
    observers.delete(callback);
  };
}
export function runtimeError(id: string) {
  return errors.get(id) ?? "";
}
function error(id: string, message: string) {
  errors.set(id, message);
  notify();
}
function notify() {
  for (const observer of observers) observer();
}

export function launchPending(viewId: string, deviceId: string) {
  return starts.has(deviceId) || visited.get(viewId) !== deviceId;
}
export function stopPending(deviceId: string) {
  return stops.has(deviceId);
}

export function retain(tabs: AndroidTab[]) {
  descriptors = tabs;
  for (const [id, settings] of viewSettings)
    if (
      !tabs.some((tab) => tab.id === id && tab.deviceId === settings.deviceId)
    )
      viewSettings.delete(id);
  for (const id of visited.keys())
    if (!tabs.some((tab) => tab.id === id && tab.deviceId === visited.get(id)))
      visited.delete(id);
  // Domain removal is already guarded; transient mounts only release view resources.
  for (const [id, view] of views)
    if (!tabs.some((tab) => tab.id === id && tab.deviceId === view.deviceId))
      detach(view);
}
export function start(deviceId: string): Promise<void> {
  if (stops.has(deviceId))
    return Promise.reject(
      new Error("Wait for the phone to stop before starting it again."),
    );
  const existing = starts.get(deviceId);
  if (existing) return existing.promise;
  const request = { promise: Promise.resolve(), cancelled: false };
  starts.set(deviceId, request);
  error(deviceId, "");
  request.promise = (async () => {
    try {
      await api("android_start", { deviceId });
      await refreshAndroid();
      schedule();
    } catch (reason) {
      const message = errorMessage(reason);
      if (request.cancelled && /start.*cancelled/i.test(message)) return;
      error(deviceId, message);
      throw reason;
    } finally {
      starts.delete(deviceId);
      notify();
    }
  })();
  return request.promise;
}
export function stop(deviceId: string, force = false): Promise<void> {
  const existing = stops.get(deviceId);
  if (existing) return existing;
  const starting = starts.get(deviceId);
  if (starting) starting.cancelled = true;
  // Also cancel first visits that are still waiting for their metadata refresh.
  for (const tab of descriptors)
    if (tab.deviceId === deviceId) visited.set(tab.id, deviceId);
  const operation = (async () => {
    try {
      await releaseInput();
      await api("android_stop", { deviceId, force });
      await starting?.promise.catch(() => {});
      await refreshAndroid();
    } finally {
      stops.delete(deviceId);
      notify();
    }
  })();
  stops.set(deviceId, operation);
  notify();
  return operation;
}
export async function restart(deviceId: string) {
  if (restarting.has(deviceId)) return;
  restarting.add(deviceId);
  try {
    await stop(deviceId);
    await start(deviceId);
  } finally {
    restarting.delete(deviceId);
  }
}
export function rotation(deviceId: string) {
  return (
    [...views.values()].find(
      (view) => view.deviceId === deviceId && view.metadata,
    )?.metadata?.rotation ?? 0
  );
}
export async function choose(viewId: string, deviceId: string) {
  if (!descriptors.some((tab) => tab.id === viewId)) return;
  const device = androidSnapshot()?.devices?.devices.find(
    (device) => device.id === deviceId,
  );
  if (!device)
    throw new Error(
      "This Android device is unavailable. Refresh Android settings.",
    );
  changeAndroid(viewId, { deviceId, title: device.name });
}
export function viewZoom(id: string): ScreenZoom {
  return viewSettings.get(id)?.zoom ?? "fit";
}
export function setZoom(
  id: string,
  zoom: ScreenZoom,
  anchor?: { x: number; y: number },
) {
  const view = views.get(id);
  if (!view) return;
  const status = androidSnapshot()?.statuses.find(
    (item) => item.deviceId === view.deviceId,
  );
  if (!status?.display || !view.renderer) return;
  const host = view.host.getBoundingClientRect();
  const before = view.renderer.element.getBoundingClientRect();
  const point = anchor ?? {
    x: host.left + host.width / 2,
    y: host.top + host.height / 2,
  };
  const u = Math.max(0, Math.min(1, (point.x - before.left) / before.width));
  const v = Math.max(0, Math.min(1, (point.y - before.top) / before.height));
  view.zoom = zoom === "fit" ? zoom : boundedZoom(zoom);
  view.cleanupInput?.endGesture();
  dimensions(view, status.display);
  const after = view.renderer.element.getBoundingClientRect();
  view.host.scrollLeft += after.left + u * after.width - point.x;
  view.host.scrollTop += after.top + v * after.height - point.y;
  viewSettings.set(id, {
    deviceId: view.deviceId,
    zoom: view.zoom,
    left: view.host.scrollLeft,
    top: view.host.scrollTop,
  });
  for (const observer of observers) observer();
  schedule();
}
function effectiveZoom(view: View) {
  if (view.zoom !== "fit") return view.zoom;
  const display = androidSnapshot()?.statuses.find(
    (item) => item.deviceId === view.deviceId,
  )?.display;
  if (!display) return 100;
  const size = screenSize(
    display,
    view.metadata?.rotation ?? 0,
    { width: view.host.clientWidth, height: view.host.clientHeight },
    devicePixelRatio,
    "fit",
  );
  return (
    ((size.width * size.scale * devicePixelRatio) / size.preview.width) * 100
  );
}
export function stepZoom(id: string, direction: 1 | -1) {
  const view = views.get(id);
  if (view && canZoom(id, direction))
    setZoom(id, zoomStep(effectiveZoom(view), direction));
}
export function canZoom(id: string, direction: 1 | -1) {
  const view = views.get(id);
  return (
    !!view &&
    (direction === 1 ? effectiveZoom(view) < 300 : effectiveZoom(view) > 25)
  );
}
export function reconnect(deviceId: string) {
  error(deviceId, "");
  const stream = streams.get(deviceId);
  if (stream) void disconnect(deviceId, stream);
  for (const view of views.values())
    if (view.deviceId === deviceId) {
      view.cleanupInput?.();
      view.cleanupInput = undefined;
      view.renderer?.dispose();
      view.renderer = undefined;
    }
  schedule();
}
function schedule() {
  if (!scheduled)
    scheduled = window.setTimeout(() => {
      scheduled = 0;
      void synchronize().catch((reason) =>
        reportAndroidError(errorMessage(reason)),
      );
    }, 60);
}

async function disconnect(deviceId: string, stream: Stream) {
  stream.active = false;
  if (streams.get(deviceId) === stream) streams.delete(deviceId);
  cancelAnimationFrame(stream.frame);
  if (stream.pending) {
    releaseFrame(stream.pending);
    stream.pending = undefined;
  }
  if (stream.epoch)
    await api("android_unsubscribe_frames", {
      deviceId,
      generation: stream.generation,
      epoch: stream.epoch,
    }).catch(() => {
      // Retired epochs cannot fail their replacement; native ACK timeout also
      // cancels an unreachable source within two seconds.
    });
}
function disposeView(view: View) {
  view.visible = false;
  view.cleanupInput?.();
  view.cleanupInput = undefined;
  view.renderer?.dispose();
  view.renderer = undefined;
  view.metadata = undefined;
}
function detach(view: View) {
  if (views.get(view.id) !== view) return;
  views.delete(view.id);
  view.resize.disconnect();
  view.cleanupViewport?.();
  disposeView(view);
  if (!views.size) {
    viewEpoch++;
    stopEvents?.();
    stopEvents = undefined;
    nativeEvents?.();
    nativeEvents = undefined;
    document.removeEventListener("visibilitychange", visibility);
    window.removeEventListener("focus", visibility);
    window.removeEventListener("resize", visibility);
    window.removeEventListener("blur", blur);
    clearTimeout(scheduled);
    scheduled = 0;
    for (const [deviceId, stream] of streams) void disconnect(deviceId, stream);
  } else schedule();
}
function blur() {
  void releaseInput().catch((reason) =>
    reportAndroidError(errorMessage(reason)),
  );
}
function visibility() {
  if (!views.size) return;
  const epoch = viewEpoch;
  void (
    native
      ? Promise.all([
          getCurrentWindow().isVisible(),
          getCurrentWindow().isMinimized(),
        ]).then(([visible, minimized]) => visible && !minimized)
      : Promise.resolve(true)
  )
    .then((visible) => {
      if (!views.size || epoch !== viewEpoch) return;
      mainVisible = visible && document.visibilityState !== "hidden";
      if (!mainVisible) {
        blur();
        for (const [id, stream] of streams) void disconnect(id, stream);
        for (const view of views.values()) disposeView(view);
      }
      schedule();
    })
    .catch((reason) => reportAndroidError(errorMessage(reason)));
}
export function mount(
  tab: AndroidTab,
  host: HTMLElement,
  input: HTMLTextAreaElement,
  onFocus: () => void,
  overview: boolean,
) {
  if (!tab.deviceId) return () => {};
  const old = views.get(tab.id);
  if (old) detach(old);
  const resize = new ResizeObserver(schedule);
  const view: View = {
    id: tab.id,
    deviceId: tab.deviceId,
    host,
    input,
    onFocus,
    overview,
    zoom: viewZoom(tab.id),
    visible: false,
    resize,
  };
  views.set(tab.id, view);
  viewSettings.set(
    tab.id,
    viewSettings.get(tab.id) ?? {
      deviceId: tab.deviceId,
      zoom: view.zoom,
      left: 0,
      top: 0,
    },
  );
  view.cleanupViewport = attachViewport(view);
  resize.observe(host);
  if (!stopEvents) {
    stopEvents = observeAndroid((event) => {
      if (event.kind === "metadata") {
        schedule();
        return;
      }
      if (event.kind === "status") {
        const stream = streams.get(event.value.deviceId);
        if (
          stream &&
          (event.value.phase !== "running" ||
            event.value.generation !== stream.generation)
        )
          void disconnect(event.value.deviceId, stream);
        schedule();
      }
      if (event.kind === "stream") {
        const stream = streams.get(event.value.deviceId);
        if (
          stream?.epoch &&
          event.value.generation === stream.generation &&
          stream.epoch === event.value.epoch &&
          ["hidden", "disconnected"].includes(event.value.phase)
        ) {
          if (event.value.error) error(event.value.deviceId, event.value.error);
          void disconnect(event.value.deviceId, stream);
        }
      }
    });
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("focus", visibility);
    window.addEventListener("resize", visibility);
    window.addEventListener("blur", blur);
    const epoch = viewEpoch;
    if (native)
      void getCurrentWindow()
        .onResized(visibility)
        .then((unlisten) => {
          if (!views.size || epoch !== viewEpoch) unlisten();
          else nativeEvents = unlisten;
        });
  }
  visibility();
  void refreshAndroid().then(() => {
    if (views.get(tab.id) !== view || overview) return;
    if (!androidSnapshot()?.qualified) return;
    if (visited.get(tab.id) !== tab.deviceId) {
      visited.set(tab.id, tab.deviceId!);
      if (!restarting.has(tab.deviceId!))
        void start(tab.deviceId!).catch(() => {});
    }
    schedule();
  });
  return () => detach(view);
}

function dimensions(view: View, display: readonly [number, number]) {
  const { width, height, scale } = screenSize(
    display,
    view.metadata?.rotation ?? 0,
    { width: view.host.clientWidth, height: view.host.clientHeight },
    devicePixelRatio,
    view.zoom,
  );
  view.host.classList.toggle("is-fit", view.zoom === "fit");
  if (view.renderer) {
    view.renderer.element.style.width = `${width * scale}px`;
    view.renderer.element.style.height = `${height * scale}px`;
  }
  return { width, height, scale: Math.min(1, scale * devicePixelRatio) };
}

function attachViewport(view: View) {
  const events = new AbortController();
  const options = { signal: events.signal };
  let pan: { id: number; x: number; y: number } | undefined;
  const save = () => {
    const settings = viewSettings.get(view.id);
    // Dockview can detach a portal before React cleanup; WebKit then reports
    // zero offsets. Preserve the last visible position across that teardown.
    if (
      settings &&
      view.renderer &&
      view.visible &&
      view.host.isConnected &&
      view.host.clientWidth > 0 &&
      view.host.clientHeight > 0
    ) {
      settings.left = view.host.scrollLeft;
      settings.top = view.host.scrollTop;
    }
  };
  view.host.addEventListener("scroll", save, options);
  view.host.addEventListener(
    "wheel",
    (event) => {
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const factor =
          event.deltaMode === 1
            ? 16
            : event.deltaMode === 2
              ? view.host.clientHeight
              : 1;
        const current = effectiveZoom(view);
        const next = relativeZoom(
          current,
          current *
            Math.exp(
              -Math.max(-100, Math.min(100, event.deltaY * factor)) * 0.01,
            ),
        );
        if (next !== undefined)
          setZoom(view.id, next, { x: event.clientX, y: event.clientY });
      } else if (event.shiftKey && view.zoom !== "fit") {
        event.preventDefault();
        event.stopImmediatePropagation();
        view.cleanupInput?.endGesture();
        view.host.scrollLeft += event.deltaX;
        view.host.scrollTop += event.deltaY;
      }
    },
    { ...options, capture: true, passive: false },
  );
  // WebKit emits native trackpad pinch as gesture events rather than Ctrl+wheel.
  let pinch = 100;
  for (const type of ["gesturestart", "gesturechange", "gestureend"])
    view.host.addEventListener(
      type,
      (raw) => {
        const event = raw as Event & {
          scale: number;
          clientX: number;
          clientY: number;
        };
        event.preventDefault();
        event.stopImmediatePropagation();
        if (type === "gesturestart") pinch = effectiveZoom(view);
        else if (type === "gesturechange" && Number.isFinite(event.scale)) {
          const next = relativeZoom(effectiveZoom(view), pinch * event.scale);
          if (next !== undefined)
            setZoom(view.id, next, { x: event.clientX, y: event.clientY });
        }
      },
      { ...options, capture: true, passive: false },
    );
  view.host.addEventListener(
    "pointerdown",
    (event) => {
      if (event.button !== 1 || !event.isPrimary) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      view.cleanupInput?.endGesture();
      pan = { id: event.pointerId, x: event.clientX, y: event.clientY };
      view.host.setPointerCapture(event.pointerId);
      view.host.classList.add("is-panning");
    },
    { ...options, capture: true },
  );
  view.host.addEventListener(
    "pointermove",
    (event) => {
      if (pan?.id !== event.pointerId) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      view.host.scrollLeft += pan.x - event.clientX;
      view.host.scrollTop += pan.y - event.clientY;
      pan = { id: event.pointerId, x: event.clientX, y: event.clientY };
    },
    { ...options, capture: true },
  );
  const end = () => {
    if (pan && view.host.hasPointerCapture(pan.id))
      view.host.releasePointerCapture(pan.id);
    pan = undefined;
    view.host.classList.remove("is-panning");
  };
  for (const type of ["pointerup", "pointercancel", "lostpointercapture"])
    view.host.addEventListener(type, end, options);
  window.addEventListener("blur", end, options);
  return () => {
    save();
    end();
    events.abort();
  };
}
async function synchronize() {
  const snapshot = androidSnapshot();
  const wanted = new Map<
    string,
    { generation: string; width: number; height: number }
  >();
  for (const view of views.values()) {
    const status = snapshot?.statuses.find(
      (status) => status.deviceId === view.deviceId,
    );
    const rect = view.host.getBoundingClientRect();
    const visible =
      mainVisible &&
      !view.overview &&
      view.host.isConnected &&
      rect.width > 0 &&
      rect.height > 0 &&
      status?.phase === "running" &&
      status.generation &&
      status.display;
    if (!visible || !status?.generation || !status.display) {
      disposeView(view);
      continue;
    }
    view.visible = true;
    if (view.generation !== status.generation) {
      disposeView(view);
      view.generation = status.generation;
      view.visible = true;
    }
    const restoreScroll = !view.renderer;
    if (!view.renderer) {
      try {
        view.renderer = new AndroidCanvas((message) => {
          error(view.deviceId, message);
          disposeView(view);
          const stream = streams.get(view.deviceId);
          if (stream) void disconnect(view.deviceId, stream);
        });
        view.host.appendChild(view.renderer.element);
        view.cleanupInput = attachInput(view, status);
      } catch (reason) {
        error(view.deviceId, errorMessage(reason));
        continue;
      }
    }
    const { width, height, scale } = dimensions(view, status.display);
    if (restoreScroll) {
      const settings = viewSettings.get(view.id);
      view.host.scrollLeft = settings?.left ?? 0;
      view.host.scrollTop = settings?.top ?? 0;
    }
    const size = {
      generation: status.generation,
      ...previewSize(Math.max(1, width * scale), Math.max(1, height * scale)),
    };
    const previous = wanted.get(view.deviceId);
    if (
      !previous ||
      previous.width * previous.height < size.width * size.height
    )
      wanted.set(view.deviceId, size);
  }
  for (const [deviceId, stream] of streams)
    if (!wanted.has(deviceId)) void disconnect(deviceId, stream);
  for (const [deviceId, size] of wanted) {
    const key = `${size.width}x${size.height}`;
    const old = streams.get(deviceId);
    if (old?.generation === size.generation && old.size === key) continue;
    if (errors.get(deviceId)) continue;
    if (old) void disconnect(deviceId, old);
    const generation = generationBytes(size.generation);
    const channel = new Channel<ArrayBuffer>();
    const stream: Stream = {
      generation: size.generation,
      size: key,
      epoch: 0,
      active: true,
      frame: 0,
      channel,
    };
    streams.set(deviceId, stream);
    channel.onmessage = (bytes) => {
      if (!stream.active || streams.get(deviceId) !== stream || !mainVisible) {
        releaseFrame(bytes);
        return;
      }
      if (stream.pending) {
        releaseFrame(bytes);
        error(
          deviceId,
          "Android sent another frame before acknowledgement. Reconnect the panel.",
        );
        void disconnect(deviceId, stream);
        return;
      }
      stream.pending = bytes;
      stream.frame = requestAnimationFrame(() => {
        stream.pending = undefined;
        try {
          const frame = decodeFrame(bytes, generation);
          if (stream.epoch && stream.epoch !== frame.epoch) return;
          stream.epoch = frame.epoch;
          const display = androidSnapshot()?.statuses.find(
            (status) => status.deviceId === deviceId,
          )?.display;
          const targets: {
            view: View;
            width: number;
            height: number;
          }[] = [];
          for (const view of views.values())
            if (view.deviceId === deviceId && view.visible && view.renderer) {
              const rotated = view.metadata?.rotation !== frame.rotation;
              view.metadata = {
                width: frame.width,
                height: frame.height,
                rotation: frame.rotation,
              };
              const size = display ? dimensions(view, display) : null;
              targets.push({
                view,
                width: size
                  ? Math.min(
                      frame.width,
                      Math.max(1, Math.ceil(size.width * size.scale)),
                    )
                  : frame.width,
                height: size
                  ? Math.min(
                      frame.height,
                      Math.max(1, Math.ceil(size.height * size.scale)),
                    )
                  : frame.height,
              });
              if (rotated) schedule();
            }
          // Keep the shared copy source at full resolution. Only secondary
          // framebuffers shrink; a smaller pane must not blur a larger one.
          const primary = targets.reduce<(typeof targets)[number] | undefined>(
            (largest, target) =>
              !largest ||
              target.width * target.height > largest.width * largest.height
                ? target
                : largest,
            undefined,
          );
          if (primary) {
            const source = primary.view.renderer!;
            source.draw(frame);
            for (const target of targets)
              if (target !== primary)
                target.view.renderer!.draw(frame, source.element, target);
          }
          void api("android_ack_frame", {
            deviceId,
            generation: stream.generation,
            epoch: frame.epoch,
            sequence: frame.sequence,
          }).catch((reason) => {
            if (!stream.active || streams.get(deviceId) !== stream) return;
            error(deviceId, errorMessage(reason));
            void disconnect(deviceId, stream);
          });
        } catch (reason) {
          error(deviceId, errorMessage(reason));
          void disconnect(deviceId, stream);
        } finally {
          releaseFrame(bytes);
        }
      });
    };
    void api<number>("android_subscribe_frames", {
      deviceId,
      generation: size.generation,
      size: { width: size.width, height: size.height },
      frames: channel,
    })
      .then((epoch) => {
        stream.epoch = epoch;
        if (!stream.active || streams.get(deviceId) !== stream) {
          void disconnect(deviceId, stream);
          return;
        }
        const state = androidSnapshot()?.streams.find(
          (state) =>
            state.deviceId === deviceId &&
            state.generation === stream.generation &&
            state.epoch === epoch,
        );
        if (state && ["hidden", "disconnected"].includes(state.phase)) {
          if (state.error) error(deviceId, state.error);
          void disconnect(deviceId, stream);
        }
      })
      .catch((reason) => {
        if (!stream.active || streams.get(deviceId) !== stream) return;
        error(deviceId, errorMessage(reason));
        void disconnect(deviceId, stream);
      });
  }
}
