import { Channel, invoke } from "@tauri-apps/api/core";

const control = (action) => invoke("android_probe_control", { action });
const report = (name, data) => invoke("android_probe_report", { name, data });
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let canvas,
  second,
  epoch,
  channel,
  measurement,
  skipAck = false;
let received = 0;
let latency;
let trackPixels = false;
let lastPixel;
let rendererGeneration = 0;
let decoder = "bitmap";
let surface = "2d";
let composited = false;
let fit = true;
let rotation = 0;
let guestDisplay;
let activeTouch;
let frameError;
let drawStage;
let desiredStream;
let streamCommands = Promise.resolve();
let streamRevision = 0;
const completedStats = {
  grpcFrames: 0,
  grpcBytes: 0,
  ipcFrames: 0,
  ipcBytes: 0,
  encodeMicros: 0,
};
const visibilityChanges = [];
const imageDecoder = new Image();
const gpuSurfaces = new WeakMap();
const latencyPixel = document.createElement("canvas");
latencyPixel.width = latencyPixel.height = 1;
function testPixel() {
  const reader = latencyPixel.getContext("2d", { willReadFrequently: true });
  reader.drawImage(
    canvas,
    Math.floor(canvas.width / 2),
    Math.floor(canvas.height * 0.65),
    1,
    1,
    0,
    0,
    1,
    1,
  );
  return reader.getImageData(0, 0, 1, 1).data[0];
}
const root = document.createElement("section");
root.style.cssText =
  "position:fixed;inset:42px 0 22px;z-index:99;background:var(--color-background);color:var(--color-background-text);display:flex;flex-direction:column;overflow:auto";
const label = document.createElement("p");
label.textContent = "Android native transport probe — preparing";
const screens = document.createElement("div");
const keyboard = document.createElement("textarea");
keyboard.setAttribute("aria-label", "Android text input");
keyboard.style.cssText =
  "position:absolute;left:2px;top:2px;width:1px;height:1px;opacity:0.01;resize:none";
let composing = false;
const inputEvents = [];
for (const name of [
  "compositionstart",
  "compositionupdate",
  "compositionend",
  "beforeinput",
  "input",
  "keydown",
]) {
  keyboard.addEventListener(name, (event) => {
    inputEvents.push({
      type: event.type,
      data: event.data,
      inputType: event.inputType,
      key: event.key,
      composing: event.isComposing,
    });
    if (inputEvents.length > 200) inputEvents.shift();
  });
}
keyboard.addEventListener("compositionstart", () => {
  composing = true;
});
keyboard.addEventListener("compositionupdate", (event) =>
  enqueue({ kind: "ime", action: "compose", text: event.data ?? "" }),
);
keyboard.addEventListener("compositionend", (event) => {
  composing = false;
  enqueue({ kind: "ime", action: "commit", text: event.data ?? "" });
  keyboard.value = "";
});
keyboard.addEventListener("beforeinput", (event) => {
  if (
    event.isComposing ||
    composing ||
    event.inputType === "insertFromComposition"
  )
    return;
  event.preventDefault();
  if (event.inputType === "insertText")
    enqueue({ kind: "ime", action: "commit", text: event.data ?? "" });
  else if (event.inputType === "deleteContentBackward")
    enqueue({ kind: "ime", action: "delete", text: "" });
});
screens.style.cssText =
  "display:flex;gap:16px;overflow:auto;min-height:0;flex:1;align-items:flex-start";
root.append(label, screens, keyboard);
function layoutCanvases() {
  const views = [canvas, second].filter(Boolean);
  if (!views.length) return;
  const availableWidth = Math.max(
    1,
    (screens.clientWidth - Math.max(0, views.length - 1) * 16) / views.length,
  );
  for (const el of views) {
    if (!el.width || !el.height) continue;
    const scale = fit
      ? Math.min(availableWidth / el.width, screens.clientHeight / el.height)
      : 1 / devicePixelRatio;
    el.style.width = `${el.width * scale}px`;
    el.style.height = `${el.height * scale}px`;
  }
}
new ResizeObserver(layoutCanvases).observe(screens);
function geometry() {
  return {
    viewport: { width: innerWidth, height: innerHeight },
    dpr: devicePixelRatio,
    fit,
    rotation,
    surface,
    decoder,
    composited,
    visibility: document.visibilityState,
    views: [canvas, second].filter(Boolean).map((el) => ({
      pixels: { width: el.width, height: el.height },
      bounds: el.getBoundingClientRect().toJSON(),
    })),
  };
}
function createCanvas() {
  const el = document.createElement("canvas");
  el.tabIndex = 0;
  el.setAttribute("aria-label", "Android feasibility probe");
  el.style.cssText =
    "flex:none;margin-inline:auto;object-fit:contain;touch-action:none;outline:none";
  if (composited) el.style.willChange = "transform";
  screens.append(el);
  el.addEventListener("pointerdown", (event) => {
    const point = touchPoint(el, event.clientX, event.clientY, false);
    if (!point) return;
    event.preventDefault();
    releaseTouch();
    keyboard.focus();
    el.setPointerCapture(event.pointerId);
    activeTouch = { element: el, id: event.pointerId, point };
    enqueue({ kind: "touch", ...point, pressure: 1024 });
  });
  el.addEventListener("pointermove", (event) => {
    if (activeTouch?.id !== event.pointerId) return;
    const point = touchPoint(el, event.clientX, event.clientY, true);
    if (!point) return;
    activeTouch.point = point;
    enqueue({ kind: "touch", ...point, pressure: 1024 });
  });
  for (const name of ["pointerup", "pointercancel", "lostpointercapture"])
    el.addEventListener(name, (event) => {
      if (activeTouch?.id === event.pointerId) releaseTouch();
    });
  return el;
}
function context(el) {
  if (surface.startsWith("webgl")) return gpuSurface(el).gl;
  const result =
    surface === "bitmaprenderer"
      ? el.getContext("bitmaprenderer", { alpha: false })
      : el.getContext("2d", {
          alpha: false,
          willReadFrequently: surface === "software",
        });
  if (!result) throw Error(`Canvas ${surface} is unavailable`);
  return result;
}
function releaseGpuSurface(el) {
  const value = gpuSurfaces.get(el);
  if (!value) return;
  value.gl.deleteTexture(value.texture);
  value.gl.deleteBuffer(value.vertices);
  value.gl.deleteProgram(value.program);
  gpuSurfaces.delete(el);
}
function gpuSurface(el) {
  let value = gpuSurfaces.get(el);
  if (value) return value;
  const gl = el.getContext("webgl", {
    alpha: false,
    antialias: false,
    depth: false,
    stencil: false,
    preserveDrawingBuffer: false,
  });
  if (!gl || gl.isContextLost()) throw Error("Canvas WebGL is unavailable");
  const program = gl.createProgram();
  for (const [kind, source] of [
    [
      gl.VERTEX_SHADER,
      "attribute vec2 p; varying vec2 uv; void main(){ gl_Position=vec4(p,0.,1.); uv=vec2((p.x+1.)*.5,(1.-p.y)*.5); }",
    ],
    [
      gl.FRAGMENT_SHADER,
      "precision mediump float; varying vec2 uv; uniform sampler2D frame; void main(){ gl_FragColor=texture2D(frame,uv); }",
    ],
  ]) {
    const shader = gl.createShader(kind);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      const message = gl.getShaderInfoLog(shader);
      gl.deleteShader(shader);
      gl.deleteProgram(program);
      throw Error(message);
    }
    gl.attachShader(program, shader);
    gl.deleteShader(shader);
  }
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const message = gl.getProgramInfoLog(program);
    gl.deleteProgram(program);
    throw Error(message);
  }
  gl.useProgram(program);
  const vertices = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, vertices);
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
    gl.STATIC_DRAW,
  );
  const position = gl.getAttribLocation(program, "p");
  gl.enableVertexAttribArray(position);
  gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.uniform1i(gl.getUniformLocation(program, "frame"), 0);
  value = { gl, program, vertices, texture, width: 0, height: 0 };
  gpuSurfaces.set(el, value);
  return value;
}
function drawGpu(el, pixels, source) {
  const value = gpuSurface(el),
    { gl } = value;
  if (gl.isContextLost()) throw Error("Canvas WebGL context was lost");
  gl.viewport(0, 0, el.width, el.height);
  if (value.width !== el.width || value.height !== el.height) {
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.RGBA,
      el.width,
      el.height,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      null,
    );
    value.width = el.width;
    value.height = el.height;
  }
  if (source)
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, gl.RGBA, gl.UNSIGNED_BYTE, source);
  else
    gl.texSubImage2D(
      gl.TEXTURE_2D,
      0,
      0,
      0,
      el.width,
      el.height,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      pixels,
    );
  gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
}
async function draw(bytes, owner) {
  if (owner !== rendererGeneration) {
    bytes.transfer?.(0);
    return;
  }
  drawStage = "header";
  if (!(bytes instanceof ArrayBuffer))
    throw Error("Frame did not use binary IPC");
  if (bytes.byteLength < 1024 || bytes.byteLength > 9 * 1024 * 1024)
    throw Error("Invalid binary frame size");
  const header = new DataView(bytes);
  if (
    header.getUint32(0, true) !== 0x50414253 ||
    header.getUint32(4, true) !== 3
  )
    throw Error("Invalid frame header");
  const frameEpoch = Number(header.getBigUint64(16, true));
  const sequence = Number(header.getBigUint64(24, true));
  const width = header.getUint32(40, true),
    height = header.getUint32(44, true);
  const payloadLength = header.getUint32(56, true);
  const timestamp = Number(header.getBigUint64(32, true)) / 1000;
  const packetLength = bytes.byteLength;
  const encoding = header.getUint32(52, true);
  const orientation = header.getInt32(48, true);
  if (
    payloadLength === 0 ||
    Math.max(60 + payloadLength, 1024) !== bytes.byteLength ||
    width === 0 ||
    height === 0 ||
    width * height > 1920 * 1080 ||
    encoding > 1 ||
    orientation < 0 ||
    orientation > 3 ||
    (encoding === 0 && payloadLength !== width * height * 4)
  )
    throw Error("Invalid pixels");
  const started = performance.now();
  const blob =
    encoding === 1
      ? new Blob([new Uint8Array(bytes, 60, payloadLength)], {
          type: "image/jpeg",
        })
      : undefined;
  // Blob owns its encoded bytes; raw pixels remain borrowed until putImageData.
  if (blob) bytes.transfer?.(0);
  let bitmap, url, duplicate;
  try {
    drawStage = "decode";
    if (encoding === 0) {
      bitmap = new ImageData(
        new Uint8ClampedArray(bytes, 60, payloadLength),
        width,
        height,
      );
    } else if (decoder === "image") {
      url = URL.createObjectURL(blob);
      imageDecoder.src = url;
      await imageDecoder.decode();
      bitmap = imageDecoder;
    } else bitmap = await createImageBitmap(blob);
    drawStage = "presentation";
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(Error("Android view is not presenting frames")),
        1500,
      );
      requestAnimationFrame(() => {
        clearTimeout(timeout);
        resolve();
      });
    });
    if (owner !== rendererGeneration) return;
    if (rotation !== orientation) releaseTouch();
    rotation = orientation;
    if (bitmap.width !== width || bitmap.height !== height)
      throw Error("Invalid JPEG dimensions");
    duplicate =
      surface === "bitmaprenderer" && second
        ? await createImageBitmap(bitmap)
        : undefined;
    if (owner !== rendererGeneration) return;
    for (const el of [canvas, second]) {
      if (!el) continue;
      if (el.width !== width || el.height !== height) {
        el.width = width;
        el.height = height;
        layoutCanvases();
      }
      if (surface.startsWith("webgl"))
        drawGpu(
          el,
          bitmap.data,
          surface === "webgl-copy" && el !== canvas ? canvas : undefined,
        );
      else if (encoding === 0) context(el).putImageData(bitmap, 0, 0);
      else if (surface === "bitmaprenderer")
        context(el).transferFromImageBitmap(el === canvas ? bitmap : duplicate);
      else context(el).drawImage(bitmap, 0, 0);
    }
  } finally {
    bitmap?.close?.();
    duplicate?.close?.();
    if (url) {
      imageDecoder.src = "";
      URL.revokeObjectURL(url);
    }
    if (encoding === 0) bytes.transfer?.(0);
  }
  if (trackPixels) lastPixel = testPixel();
  if (latency) {
    if (Math.abs(lastPixel - latency.previous) > 200) {
      const sample = latency;
      latency = undefined;
      requestAnimationFrame(() =>
        requestAnimationFrame(() =>
          sample.resolve(performance.now() - sample.started),
        ),
      );
    }
  }
  received++;
  if (measurement) {
    measurement.frames++;
    measurement.bytes += packetLength;
    measurement.drawMs.push(performance.now() - started);
    measurement.times.push(performance.now());
    measurement.frameAgeMs.push(Date.now() - timestamp);
  }
  if (!skipAck)
    void invoke("android_probe_ack", { epoch: frameEpoch, sequence });
}
async function connect(width, height) {
  const owner = ++rendererGeneration;
  frameError = undefined;
  channel = new Channel((bytes) => {
    void draw(bytes, owner).catch((error) => {
      label.textContent = String(error);
      if (owner === rendererGeneration) {
        frameError = String(error);
        void report("frame-error", {
          error: frameError,
          stage: drawStage,
          decoder,
          visibility: document.visibilityState,
          received,
          detached: bytes.detached,
          size: bytes.byteLength,
        });
        void unsubscribe();
      }
    });
  });
  epoch = await invoke("android_probe_subscribe", {
    width,
    height,
    frames: channel,
    encoding: decoder === "rgba" ? "rgba" : "jpeg",
  });
  label.textContent = "Android native transport probe — streaming";
  return epoch;
}
async function disconnect() {
  const stats = await control("unsubscribe");
  if (stats)
    for (const key of Object.keys(completedStats))
      completedStats[key] += stats[key];
}
async function totalStats() {
  const current = await control("stats");
  return {
    ...Object.fromEntries(
      Object.entries(completedStats).map(([key, value]) => [
        key,
        value + (current?.[key] ?? 0),
      ]),
    ),
    error: current?.error ?? null,
  };
}
function releaseCanvases() {
  for (const el of [canvas, second]) {
    if (el) {
      releaseGpuSurface(el);
      el.width = el.height = 0;
    }
  }
}
function reconcileStream() {
  ++rendererGeneration;
  const revision = ++streamRevision;
  const operation = streamCommands
    .catch(() => {})
    .then(async () => {
      if (revision !== streamRevision) return;
      await disconnect();
      if (revision !== streamRevision) return;
      if (desiredStream && document.visibilityState === "visible")
        return connect(desiredStream.width, desiredStream.height);
      releaseCanvases();
    });
  streamCommands = operation;
  return operation;
}
function subscribe(width = 720, height = 1280) {
  desiredStream = { width, height };
  return reconcileStream();
}
function unsubscribe() {
  releaseTouch();
  desiredStream = undefined;
  return reconcileStream();
}
document.addEventListener("visibilitychange", () => {
  const now = performance.now();
  const visible = document.visibilityState === "visible";
  if (!visible) releaseTouch();
  const entry = { at: performance.timeOrigin + now, visible };
  visibilityChanges.push(entry);
  if (visibilityChanges.length > 200) visibilityChanges.shift();
  if (measurement) {
    if (measurement.visible)
      measurement.activeMs += now - measurement.changedAt;
    measurement.changedAt = now;
    measurement.visible = visible;
    measurement.visibility.push(entry);
  }
  void reconcileStream().catch((error) => {
    frameError = String(error);
  });
});
async function input(input) {
  return invoke("android_probe_input", { input });
}
let inputQueue = Promise.resolve();
function enqueue(inputValue) {
  inputQueue = inputQueue
    .then(() => input(inputValue))
    .catch((error) => {
      label.textContent = String(error);
    });
}
function touchPoint(target, clientX, clientY, clamp) {
  if (!guestDisplay) return undefined;
  const bounds = target.getBoundingClientRect();
  const scale = Math.min(
    bounds.width / target.width,
    bounds.height / target.height,
  );
  const left = bounds.left + (bounds.width - target.width * scale) / 2;
  const top = bounds.top + (bounds.height - target.height * scale) / 2;
  if (!Number.isFinite(scale) || scale <= 0) return;
  let u = (clientX - left) / scale / target.width;
  let v = (clientY - top) / scale / target.height;
  if (!Number.isFinite(u) || !Number.isFinite(v)) return;
  if (!clamp && (u < 0 || v < 0 || u >= 1 || v >= 1)) return;
  u = Math.max(0, Math.min(1, u));
  v = Math.max(0, Math.min(1, v));
  const [x, y] = [
    [u, v],
    [1 - v, u],
    [1 - u, 1 - v],
    [v, 1 - u],
  ][rotation];
  // Natural display dimensions belong to this fixed native fixture only.
  return {
    x: Math.min(guestDisplay.width - 1, Math.floor(x * guestDisplay.width)),
    y: Math.min(guestDisplay.height - 1, Math.floor(y * guestDisplay.height)),
  };
}
function releaseTouch() {
  if (!activeTouch) return;
  const touch = activeTouch;
  activeTouch = undefined;
  enqueue({ kind: "touch", ...touch.point, pressure: 0 });
  if (touch.element.hasPointerCapture(touch.id))
    touch.element.releasePointerCapture(touch.id);
}
window.addEventListener("blur", releaseTouch);
document.addEventListener("focusin", (event) => {
  if (![keyboard, canvas, second].includes(event.target)) releaseTouch();
});
function mount() {
  if (canvas) return;
  document.body.append(root);
  canvas = createCanvas();
}
async function execute(task) {
  switch (task.action) {
    case "start": {
      mount();
      const result = await control("start");
      await report("started", result);
      for (let i = 0; i < 240; i++) {
        try {
          const status = await control("status");
          if (status.booted) {
            if (!status.display?.width || !status.display?.height)
              throw Error(
                "The native fixture did not provide display dimensions",
              );
            guestDisplay = status.display;
            label.textContent = "Android native transport probe — running";
            return status;
          }
        } catch {}
        await sleep(500);
      }
      throw Error("Boot or authentication timed out");
    }
    case "subscribe":
      return subscribe(task.width, task.height);
    case "unsubscribe":
      return unsubscribe();
    case "renderer":
      await unsubscribe();
      for (const el of [canvas, second]) {
        if (!el) continue;
        releaseGpuSurface(el);
        el.width = el.height = 0;
        el.remove();
      }
      surface = [
        "2d",
        "software",
        "bitmaprenderer",
        "webgl",
        "webgl-copy",
      ].includes(task.surface)
        ? task.surface
        : "2d";
      composited = task.composited === true;
      canvas = createCanvas();
      if (second) second = createCanvas();
      decoder = ["image", "bitmap", "rgba"].includes(task.decoder)
        ? task.decoder
        : "bitmap";
      if (decoder === "rgba" && surface === "bitmaprenderer") surface = "2d";
      if (surface === "bitmaprenderer") decoder = "bitmap";
      if (surface.startsWith("webgl")) decoder = "rgba";
      context(canvas);
      return subscribe();
    case "present":
    case "large-window":
    case "small-window":
    case "process-info":
      return control(task.action);
    case "hide-views":
      await unsubscribe();
      releaseCanvases();
      root.style.display = "none";
      return geometry();
    case "show-views":
      root.style.display = "flex";
      layoutCanvases();
      return geometry();
    case "fit":
      fit = task.enabled !== false;
      layoutCanvases();
      return geometry();
    case "minimize":
    case "hide":
      await execute({ action: "hide-views" });
      await inputQueue;
      return control(task.action);
    case "auth":
      return control("auth");
    case "rtc":
      return control("rtc");
    case "quit":
      return control("quit");
    case "screenshot":
      return control("screenshot");
    case "input":
      return input(task.input);
    case "rotate": {
      releaseTouch();
      await inputQueue;
      await input({ kind: "rotate", quarterTurns: task.quarterTurns });
      if (desiredStream) {
        const short = Math.min(desiredStream.width, desiredStream.height);
        const long = Math.max(desiredStream.width, desiredStream.height);
        return task.quarterTurns % 2
          ? subscribe(long, short)
          : subscribe(short, long);
      }
      return {};
    }
    case "touch-map": {
      const bounds = canvas.getBoundingClientRect();
      return (
        touchPoint(
          canvas,
          bounds.x + bounds.width * task.u,
          bounds.y + bounds.height * task.v,
          task.clamp === true,
        ) ?? null
      );
    }
    case "tap-relative": {
      const bounds = canvas.getBoundingClientRect();
      const point = touchPoint(
        canvas,
        bounds.x + bounds.width * task.u,
        bounds.y + bounds.height * task.v,
        false,
      );
      if (!point) throw Error("Point is outside the phone image");
      await input({ kind: "touch", ...point, pressure: 1024 });
      await input({ kind: "touch", ...point, pressure: 0 });
      return { point, rotation, geometry: geometry() };
    }
    case "focus-keyboard":
      keyboard.focus();
      return {};
    case "focus-form": {
      const field = document.createElement("input");
      root.append(field);
      field.focus();
      await inputQueue;
      field.remove();
      return { activeTouch: activeTouch?.point ?? null };
    }
    case "letterbox":
      canvas.style.width = "600px";
      canvas.style.height = "600px";
      return geometry();
    case "input-events":
      return inputEvents;
    case "features":
      return {
        guestDisplay,
        visibility: document.visibilityState,
        imageDecoder: typeof ImageDecoder,
        detach: typeof ArrayBuffer.prototype.transfer,
        surface,
        contextAttributes: context(canvas).getContextAttributes?.(),
        received,
        frameError,
        geometry: geometry(),
        visibilityChanges,
        desiredStream,
      };
    case "native-text":
      keyboard.focus();
      await invoke("android_probe_native_text", {
        action: task.operation,
        text: task.text,
      });
      await sleep(150);
      await inputQueue;
      return inputEvents;
    case "native-pointer": {
      const bounds = canvas.getBoundingClientRect();
      await invoke("android_probe_native_pointer", {
        phase: task.phase,
        x: bounds.x + bounds.width * task.u,
        y: bounds.y + bounds.height * task.v,
      });
      await sleep(100);
      await inputQueue;
      return { activeTouch: activeTouch?.point ?? null, rotation };
    }
    case "latency": {
      if (!guestDisplay) throw Error("The guest display is not ready");
      const x = Math.floor(guestDisplay.width / 2);
      const y = Math.floor(guestDisplay.height * 0.65);
      const samples = [];
      trackPixels = true;
      lastPixel = undefined;
      try {
        await input({ kind: "touch", x, y, pressure: 1024 });
        await input({ kind: "touch", x, y, pressure: 0 });
        const ready = performance.now() + 2000;
        while (lastPixel === undefined) {
          if (performance.now() > ready)
            throw Error("No presented frame for latency measurement");
          await sleep(10);
        }
        for (let i = 0; i < 50; i++) {
          const previous = lastPixel;
          let timer;
          const result = new Promise((resolve, reject) => {
            latency = { previous, started: performance.now(), resolve };
            timer = setTimeout(
              () => reject(Error("No guest color transition reached Canvas")),
              2000,
            );
          });
          try {
            await input({ kind: "touch", x, y, pressure: 1024 });
            await input({ kind: "touch", x, y, pressure: 0 });
            samples.push(await result);
          } finally {
            clearTimeout(timer);
            latency = undefined;
          }
          await sleep(100);
        }
        const sorted = [...samples].sort((a, b) => a - b);
        return {
          samples,
          p95: sorted[Math.floor(samples.length * 0.95)],
          max: sorted.at(-1),
          endpoint:
            "second requestAnimationFrame after guest pixel transition is drawn",
        };
      } finally {
        trackPixels = false;
      }
    }
    case "two-views":
      if (!second) second = createCanvas();
      layoutCanvases();
      return {};
    case "one-view":
      if (second) releaseGpuSurface(second);
      if (second) second.width = second.height = 0;
      second?.remove();
      second = undefined;
      layoutCanvases();
      return {};
    case "benchmark": {
      const start = performance.now();
      const visible = document.visibilityState === "visible";
      measurement = {
        frames: 0,
        bytes: 0,
        drawMs: [],
        times: [],
        frameAgeMs: [],
        activeMs: 0,
        changedAt: start,
        visible,
        visibility: [{ at: performance.timeOrigin + start, visible }],
      };
      const nativeBefore = await totalStats();
      const geometryBefore = geometry();
      const requested = task.durationMs ?? 60000;
      const deadline = start + requested + (task.activeTime ? 300000 : 0);
      const activeMilliseconds = () =>
        measurement.activeMs +
        (measurement.visible ? performance.now() - measurement.changedAt : 0);
      while (
        (task.activeTime ? activeMilliseconds() : performance.now() - start) <
        requested
      ) {
        if (performance.now() >= deadline) {
          measurement = undefined;
          throw Error(
            "Visible benchmark duration exceeded its wall-clock limit",
          );
        }
        if (frameError) {
          measurement = undefined;
          throw Error(frameError);
        }
        await sleep(Math.min(250, deadline - performance.now()));
        const pending = await control("instruction");
        if (pending.action === "cancel-benchmark") break;
      }
      const stats = measurement;
      const activeElapsed = activeMilliseconds() / 1000;
      measurement = undefined;
      const elapsed = (performance.now() - start) / 1000;
      const percentile = (values, p) =>
        values.sort((a, b) => a - b)[
          Math.min(values.length - 1, Math.floor(values.length * p))
        ];
      return {
        elapsed,
        activeElapsed,
        visibility: stats.visibility,
        completed:
          (task.activeTime ? activeElapsed : elapsed) >= requested / 1000,
        frames: stats.frames,
        fps: stats.frames / activeElapsed,
        ipcMBs: stats.bytes / elapsed / 1e6,
        activeIpcMBs: stats.bytes / activeElapsed / 1e6,
        drawP95Ms: percentile(stats.drawMs, 0.95),
        frameAgeP95Ms: percentile(stats.frameAgeMs, 0.95),
        gapsP95Ms: percentile(
          stats.times.slice(1).map((n, i) => n - stats.times[i]),
          0.95,
        ),
        nativeBefore,
        native: await totalStats(),
        userAgent: navigator.userAgent,
        dpr: devicePixelRatio,
        geometryBefore,
        geometryAfter: geometry(),
      };
    }
    case "ack-timeout": {
      skipAck = true;
      const stale = await subscribe();
      await sleep(2500);
      const timedOut = await control("stats");
      if (timedOut.error !== "ACK timeout" || timedOut.ipcFrames !== 1)
        throw Error("ACK timeout did not bound IPC");
      const current = await subscribe();
      await sleep(250);
      const before = await control("stats");
      await invoke("android_probe_ack", { epoch: stale, sequence: 1 });
      await sleep(250);
      const afterStale = await control("stats");
      if (before.ipcFrames !== 1 || afterStale.ipcFrames !== 1)
        throw Error("A stale ACK released the current frame");
      skipAck = false;
      await invoke("android_probe_ack", { epoch: current, sequence: 1 });
      await input({ kind: "key", key: "AppSwitch" });
      await sleep(1500);
      const afterCurrent = await control("stats");
      if (afterCurrent.ipcFrames <= 1)
        throw Error("The current ACK did not resume delivery");
      return {
        timedOut,
        stale,
        current,
        before,
        afterStale,
        afterCurrent,
        received,
      };
    }
    case "stop":
      return control("stop");
    case "cancel-benchmark":
      return {};
    default:
      throw Error("Unknown test instruction");
  }
}
(async () => {
  await report("webview", {
    userAgent: navigator.userAgent,
    dpr: devicePixelRatio,
  });
  mount();
  let last = sessionStorage.getItem("lomi.android.probe.last");
  for (;;) {
    try {
      const task = await control("instruction");
      if (task.id && task.id !== last) {
        last = task.id;
        sessionStorage.setItem("lomi.android.probe.last", task.id);
        try {
          const result = await execute(task);
          await report(task.id, { ok: true, result });
        } catch (error) {
          label.textContent = String(error);
          await report(task.id, { ok: false, error: String(error) });
        }
      }
    } catch {}
    await sleep(300);
  }
})();
