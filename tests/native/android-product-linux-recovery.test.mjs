import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = await readFile(
  new URL("./android-product-linux-recovery.mjs", import.meta.url),
  "utf8",
);
const embedded = source.match(/"cold-start",\s*`([\s\S]*?)`,\s*240000,/)[1];
const script = embedded
  .replaceAll("${JSON.stringify(deviceId)}", '"phone"')
  .replaceAll("${JSON.stringify(before.device.name)}", '"Phone"')
  .replaceAll("${JSON.stringify(runtime.generation)}", '"old-generation"');

async function run(options = {}) {
  const calls = [];
  let timer,
    paneQueries = 0,
    cleared = false,
    menuPane,
    sleeps = 0,
    foreignDraw;
  class Context {
    constructor(canvas) {
      this.canvas = canvas;
    }
    putImageData() {
      calls.push("original-put");
      if (options.putFailure) throw Error("Original put failed");
    }
    drawImage() {
      calls.push("original-draw");
      if (options.drawFailure) throw Error("Original draw failed");
    }
    getImageData() {
      throw Error("Recovery must not read bitmap pixels");
    }
  }
  const originalPut = Context.prototype.putImageData;
  const originalDraw = Context.prototype.drawImage;
  const canvas = (id) => ({
    width: 321,
    height: 571,
    isConnected: true,
    paneId: id,
    classList: { contains: (name) => name === "android-screen" },
    closest() {
      return { dataset: { androidPaneId: this.paneId } };
    },
    getClientRects: () => [1],
  });
  const primary = canvas("primary"),
    secondary = canvas("secondary");
  primary.context = new Context(primary);
  secondary.context = new Context(secondary);
  const canvases = [primary, secondary];
  const pane = (width, element, actual) => ({
    actual,
    getAttribute: () => "Phone",
    getClientRects: () => [1],
    getBoundingClientRect: () => ({ width }),
    querySelector(selector) {
      if (selector === ".android-screen") return element;
      return {
        disabled: false,
        click: () => {
          menuPane = this;
        },
      };
    },
    element,
  });
  const panes = [pane(700, primary, false), pane(321, secondary, true)];
  if (options.ambiguousWidth)
    panes[1].getBoundingClientRect = () => ({ width: 700 });
  const menu = {
    get textContent() {
      return menuPane.actual ? "Fit to panel" : "Actual size (1:1)";
    },
    disabled: false,
    getClientRects: () => [1],
    closest: () => ({
      dispatchEvent() {
        calls.push("escape-menu");
      },
    }),
    click() {
      menuPane.actual = !menuPane.actual;
      Object.assign(
        menuPane.element,
        menuPane.actual
          ? { width: 720, height: 1280 }
          : { width: 321, height: 571 },
      );
      calls.push(menuPane === panes[0] ? "primary-mode" : "secondary-mode");
    },
  };
  const draw = () => {
    primary.context.putImageData({ width: 720, height: 1280 }, 0, 0);
    if (!options.noSecondaryDraw)
      secondary.context.drawImage(
        options.wrongSource ? { width: 720, height: 1280 } : primary,
        0,
        0,
        321,
        571,
      );
  };
  const state = () => ({
    statuses: [
      {
        deviceId: "phone",
        generation: "new-generation",
        phase: "running",
        processAlive: true,
      },
    ],
    streams: [
      {
        deviceId: "phone",
        generation: "new-generation",
        phase: "streaming",
        ipcFrames: 1,
      },
    ],
  });
  const env = {
    CanvasRenderingContext2D: Context,
    document: {
      querySelectorAll(selector) {
        if (selector === ".android-pane") {
          paneQueries++;
          if (options.mountRace && paneQueries === 1) return [];
          return options.onePane ? panes.slice(0, 1) : panes;
        }
        if (selector === ".android-screen") return canvases;
        return [menu];
      },
    },
    KeyboardEvent: class {},
    setTimeout(callback, ms) {
      assert.equal(ms, 225000);
      timer = callback;
      return 1;
    },
    clearTimeout(id) {
      assert.equal(id, 1);
      cleared = true;
    },
  };
  const execute = vm.runInNewContext(
    `(async (invoke,sleep,wait)=>{${script}})`,
    env,
  );
  let result, error;
  try {
    result = await execute(
      async (command) => {
        if (command === "android_start")
          return { processAlive: true, generation: "new-generation" };
        assert.equal(command, "android_state");
        if (options.changedIdentity && sleeps >= 2)
          secondary.paneId = "replacement";
        return state();
      },
      async () => {
        sleeps++;
        draw();
        if (sleeps === 2 && options.foreignHook) {
          foreignDraw = function () {};
          Context.prototype.drawImage = foreignDraw;
        }
        if (sleeps === 2 && options.expired) timer();
      },
      async (predicate) => {
        for (let attempt = 0; attempt < 3; attempt++) {
          const value = await predicate();
          if (value) return value;
          draw();
        }
        throw Error("Mock wait timed out");
      },
    );
  } catch (value) {
    error = value;
  }
  assert.equal(Context.prototype.putImageData, originalPut);
  assert.equal(
    Context.prototype.drawImage,
    options.foreignHook ? foreignDraw : originalDraw,
  );
  assert.equal(cleared, true, "observer deadline timer cleared");
  return { result, error, calls, paneQueries };
}

test("2D cold start proves shared source, keeps only widest Actual and secondary Fit", async () => {
  const { result, error, calls } = await run();
  assert.ifError(error);
  assert.equal(result.views, 2);
  assert.deepEqual(
    Array.from(result.viewDetails, (view) => Array.from(view.framebuffer)),
    [
      [720, 1280],
      [321, 571],
    ],
  );
  assert.deepEqual(
    Array.from(result.viewDetails, (view) => view.sizeMode),
    ["actual", "fit"],
  );
  assert.match(
    result.sourceProof,
    /source\/draw provenance only, no bitmap readback or native input assertion/,
  );
  assert.ok(calls.includes("primary-mode") && calls.includes("secondary-mode"));
});
test("a same-sized foreign canvas cannot prove secondary provenance", async () => {
  assert.match((await run({ wrongSource: true })).error.message, /timed out/);
});
test("primary source alone cannot prove both views", async () => {
  assert.match(
    (await run({ noSecondaryDraw: true })).error.message,
    /timed out/,
  );
});
test("a failing original primary upload is never counted and hooks restore", async () => {
  assert.match(
    (await run({ putFailure: true })).error.message,
    /Original put failed/,
  );
});
test("a failing original secondary draw is never counted and hooks restore", async () => {
  assert.match(
    (await run({ drawFailure: true })).error.message,
    /Original draw failed/,
  );
});
test("foreign hook replacement is preserved and rejects successful recovery", async () => {
  assert.match(
    (await run({ foreignHook: true })).error.message,
    /hook ownership changed/,
  );
});
test("deadline restores hooks and rejects further source proof", async () => {
  assert.match(
    (await run({ expired: true })).error.message,
    /exceeded its deadline/,
  );
});
test("retired pane identity cannot satisfy source proof", async () => {
  assert.match(
    (await run({ changedIdentity: true })).error.message,
    /identities changed/,
  );
});
test("one pane or ambiguous widest pane fails explicitly", async () => {
  assert.match((await run({ onePane: true })).error.message, /timed out/);
  assert.match(
    (await run({ ambiguousWidth: true })).error.message,
    /unambiguous widest/,
  );
});

async function discoveryRace(options = {}) {
  const events = [];
  const deniedPid = 123;
  const nativeError = (code) => Object.assign(Error(code), { code });
  const predicate = vm.runInNewContext(
    source.slice(
      source.indexOf("async function processesMatching(predicate)"),
      source.indexOf("const engine ="),
    ) + "\nprocessesMatching;",
    {
      readdir: async () =>
        [123, 456].map((pid) => ({
          name: String(pid),
          isDirectory: () => true,
        })),
      stat: async () => ({ uid: 1000 }),
      uid: 1000,
      join: (...parts) => parts.join("/"),
      readlink: async (path) => {
        if (path.includes("/123/")) {
          events.push("denied-exe");
          throw nativeError("EACCES");
        }
        return "/fixture/expected";
      },
      protectedSystemProcess: async (pid) => {
        assert.equal(pid, deniedPid);
        events.push("classification-finished");
        return options.protected === true;
      },
      discoveryPins: new Set(options.pinned ? [deniedPid] : []),
      lstat: async () => {
        events.push("recheck-directory");
        if (!options.persistent)
          throw nativeError(options.recheckError ?? "ENOENT");
        return {};
      },
      identity: async (pid) => ({ pid }),
    },
  );
  let result, error;
  try {
    result = await predicate((file) => file === "/fixture/expected");
  } catch (value) {
    error = value;
  }
  return { result, error, events };
}

test("unclassified EACCES followed by confirmed directory disappearance continues discovery", async () => {
  const { result, error, events } = await discoveryRace();
  assert.ifError(error);
  assert.deepEqual(
    Array.from(result, (value) => value.pid),
    [456],
  );
  assert.deepEqual(events, [
    "denied-exe",
    "classification-finished",
    "recheck-directory",
  ]);
});
test("persistent unclassified EACCES remains fatal", async () => {
  const { error } = await discoveryRace({ persistent: true });
  assert.match(error.message, /pid=123, code=EACCES/);
});
test("uncertain recheck errors cannot waive an unclassified EACCES", async () => {
  for (const recheckError of ["EACCES", "ESRCH", "EIO"])
    assert.match(
      (await discoveryRace({ recheckError })).error.message,
      /pid=123, code=EACCES/,
    );
});
test("pinned app, guest or ADB denial remains fatal even if the process could disappear", async () => {
  const { error, events } = await discoveryRace({ pinned: true });
  assert.match(error.message, /pid=123, code=EACCES/);
  assert.deepEqual(events, ["denied-exe", "classification-finished"]);
});
test("existing protected OS process proof still excludes its EACCES without disappearance", async () => {
  const { result, error, events } = await discoveryRace({ protected: true });
  assert.ifError(error);
  assert.deepEqual(
    Array.from(result, (value) => value.pid),
    [456],
  );
  assert.deepEqual(events, ["denied-exe", "classification-finished"]);
});

test("running status can precede mounting both requested recovery canvases", async () => {
  const { result, error, paneQueries } = await run({ mountRace: true });
  assert.ifError(error);
  assert.equal(result.views, 2);
  assert.ok(paneQueries >= 2);
});

test("native resize instruction stays process scoped and precedes cold start", async () => {
  let payload;
  const handlers = {};
  const helper = {
    stdout: {
      on(event, callback) {
        handlers.data = callback;
      },
    },
    stderr: { resume() {} },
    once(event, callback) {
      handlers[event] = callback;
    },
    stdin: {
      once() {},
      end(input) {
        payload = JSON.parse(input);
        handlers.data(JSON.stringify({ ok: true, data: null }));
        handlers.close(0);
      },
    },
    kill() {
      throw Error("Successful mock must not signal helpers");
    },
  };
  const script = source
    .slice(
      source.indexOf("async function control(pin,"),
      source.indexOf("const statusScript ="),
    )
    .replaceAll("import.meta.dirname", '"/source/tests/native"');
  const control = vm.runInNewContext(script + "\ncontrol;", {
    assertIdentity: async (pin) => assert.equal(pin.pid, 99971),
    name: "resize-proof",
    index: 0,
    process: { execPath: "/node" },
    root: "/product",
    repository: "/source",
    join: (...parts) => parts.join("/"),
    spawn: () => helper,
    setTimeout: () => 1,
    clearTimeout() {},
  });
  await control({ pid: 99971 }, "resize", undefined, 15000, "resize");
  assert.deepEqual(payload, {
    processId: 99971,
    window: "main",
    timeoutMs: 15000,
    action: "resize",
    width: 1400,
    height: 950,
  });
  assert.ok(
    source.indexOf('await control(nextApp, "resize"') <
      source.indexOf('"cold-start",'),
  );
});
