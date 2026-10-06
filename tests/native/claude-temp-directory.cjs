"use strict";
// Evaluate only pinned public temp utilities; never initialize or launch Claude.
const fs = require("node:fs"),
  path = require("node:path"),
  crypto = require("node:crypto"),
  assert = require("node:assert/strict");
const artifact = process.argv[2];
if (!artifact)
  throw Error(
    "Usage: node tests/native/claude-temp-directory.cjs <owned-public-Claude-2.1.287-artifact>",
  );
assert.equal(process.version, "v22.22.3");
assert.equal(process.platform, "darwin");
const fd = fs.openSync(
  artifact,
  fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK,
);
function pinned(start, end, expected) {
  const bytes = Buffer.alloc(end - start);
  assert.equal(fs.readSync(fd, bytes, 0, bytes.length, start), bytes.length);
  assert.equal(
    crypto.createHash("sha256").update(bytes).digest("hex"),
    expected,
  );
  return bytes.toString("utf8");
}
let source;
try {
  const metadata = fs.fstatSync(fd);
  assert(metadata.isFile());
  assert.equal(metadata.size, 227827120);
  const hash = crypto.createHash("sha256"),
    buffer = Buffer.alloc(1024 * 1024);
  for (let position = 0; position < metadata.size;) {
    const count = fs.readSync(
      fd,
      buffer,
      0,
      Math.min(buffer.length, metadata.size - position),
      position,
    );
    assert(count > 0);
    hash.update(buffer.subarray(0, count));
    position += count;
  }
  assert.equal(
    hash.digest("hex"),
    "6eab8333fe2121553100d8f40bfada384a3e989b94f947e18ba6677a6fcb41ea",
  );
  source = pinned(
    178417752,
    178420263,
    "d6a3249d4665ae0fc32124c7c190fab2106af6c17719ead1d6a7a70deea390c9",
  );
  pinned(
    178419905,
    178420186,
    "2b82e88d6ead77e34b834d75d8ae693d2e685658e7637934d891807155bbcc49",
  );
  pinned(
    178420186,
    178420263,
    "7b4180b393a575b01fb447c41892ba874d66235d10d2993ce05ad84e2426ff18",
  );
} finally {
  fs.closeSync(fd);
}
class HostCache {
  constructor(make) {
    this.make = make;
    this.values = new WeakMap();
  }
  of(host) {
    if (!this.values.has(host)) this.values.set(host, this.make());
    return this.values.get(host);
  }
}
function evaluateHelpers(env, mkdir) {
  const host = {};
  const bindings = {
    a: env,
    V: HostCache,
    U: () => ({ host }),
    Ut: () => false,
    p: path.join,
    d: mkdir,
    F: fs.openSync,
    h: fs.constants,
    E: (error) => error.code,
    k: fs.lstatSync,
    P: Error,
    L: fs.fstatSync,
    H: fs.fchmodSync,
    x: fs.closeSync,
    ne: () => {
      throw Error("Unexpected root-container exception");
    },
  };
  return new Function(
    ...Object.keys(bindings),
    source + "\nreturn {NS,hl,nEe,UYo};",
  )(...Object.values(bindings));
}
if (process.argv[3] === "--held") {
  const alias = process.env.CLAUDE_CODE_TMPDIR,
    physical = process.argv[4];
  assert(
    alias && physical,
    "Held mode requires CLAUDE_CODE_TMPDIR and explicit canonical physical temp",
  );
  assert.equal(fs.realpathSync(physical), physical);
  assert.equal(fs.realpathSync(alias), physical);
  const wrapper = path.dirname(alias);
  assert.equal(fs.lstatSync(wrapper).uid, process.getuid());
  assert.equal(fs.lstatSync(wrapper).mode & 0o777, 0o700);
  assert.equal(Buffer.byteLength(alias), 33);
  const attempts = [];
  const helpers = evaluateHelpers(
    { CLAUDE_CODE_TMPDIR: alias },
    (target, options) => {
      attempts.push(target);
      return fs.mkdirSync(target, options); // Real kernel policy, no synthetic denial.
    },
  );
  const child = helpers.hl();
  assert(Buffer.byteLength(child) <= 44);
  assert.equal(helpers.UYo(), alias);
  assert.equal(helpers.nEe(), child);
  assert.deepEqual(attempts, [child]);
  assert.equal(
    fs.realpathSync(child),
    path.join(physical, `claude-${process.getuid()}`),
  );
  const token = crypto.randomBytes(8).toString("hex"),
    data = path.join(child, `owned-held-data-${token}`),
    replacement = path.join(physical, `owned-replacement-${token}`),
    escapeName = `owned-escape-link-${token}`,
    escape = path.join(physical, escapeName);
  const denied = (action) =>
    assert.throws(action, (error) => ["EPERM", "EACCES"].includes(error.code));
  try {
    fs.writeFileSync(data, "owned-held-temp", { flag: "wx", mode: 0o600 });
    assert.equal(
      fs.readFileSync(
        path.join(physical, `claude-${process.getuid()}`, path.basename(data)),
        "utf8",
      ),
      "owned-held-temp",
    );
    denied(() => fs.unlinkSync(alias));
    fs.symlinkSync(physical, replacement);
    denied(() => fs.renameSync(replacement, alias));
    denied(() =>
      fs.writeFileSync(`/private/tmp/lomi-unrelated-temp-${token}`, "denied", {
        flag: "wx",
      }),
    );
    denied(() =>
      fs.mkdirSync(`/tmp/claude-${process.getuid()}/lomi-denied-${token}`, {
        recursive: true,
      }),
    );
    fs.symlinkSync(
      path.resolve(physical, "../..", "outside-owned-canary"),
      escape,
    );
    const aliasEscape = path.join(alias, escapeName);
    denied(() => fs.readFileSync(aliasEscape));
    denied(() => fs.writeFileSync(aliasEscape, "denied-escape-write"));
    assert.equal(fs.realpathSync(alias), physical);
    console.log(
      JSON.stringify({
        passed: true,
        heldKernelPolicy: true,
        checks: 8,
        sourceExtractionNotFullCLIQualification: true,
        credentials: false,
        network: false,
        cliLaunch: false,
      }),
    );
  } finally {
    fs.rmSync(data, { force: true });
    fs.rmSync(replacement, { force: true });
    fs.rmSync(escape, { force: true });
  }
  process.exit(0);
}
assert.equal(
  process.argv.length,
  3,
  "Only explicit --held <physical-temp> is supported",
);
const physical = fs.realpathSync(
    fs.mkdtempSync("/private/tmp/lomi-claude-temp-source-long-baseline-"),
  ),
  wrapper = "/private/tmp/la" + crypto.randomBytes(8).toString("hex"),
  alias = path.join(wrapper, "t"),
  sharedAttempts = [];
let wrapperCreated = false;
try {
  fs.chmodSync(physical, 0o700);
  fs.mkdirSync(wrapper, { mode: 0o700 }); // Refuse any existing wrapper.
  wrapperCreated = true;
  fs.symlinkSync(physical, alias);
  assert.equal(fs.lstatSync(wrapper).uid, process.getuid());
  assert.equal(fs.lstatSync(wrapper).mode & 0o777, 0o700);
  assert.equal(fs.realpathSync(alias), physical);
  assert.equal(Buffer.byteLength(alias), 33);
  const env = { CLAUDE_CODE_TMPDIR: alias };
  function ownedMkdir(target, options) {
    if (target === `/tmp/claude-${process.getuid()}`) {
      sharedAttempts.push(target);
      throw Object.assign(Error("Owned fixture denies shared temp writes"), {
        code: "EPERM",
      });
    }
    let parent = target;
    while (!fs.existsSync(parent)) parent = path.dirname(parent);
    const canonical = fs.realpathSync(parent);
    assert(canonical === physical || canonical.startsWith(physical + path.sep));
    return fs.mkdirSync(target, options);
  }
  const helpers = evaluateHelpers(env, ownedMkdir);
  const privateChild = helpers.hl();
  assert(Buffer.byteLength(privateChild) <= 44);
  assert.equal(
    fs.realpathSync(privateChild),
    path.join(physical, `claude-${process.getuid()}`),
  );
  assert.equal(fs.statSync(privateChild).uid, process.getuid());
  assert.equal(fs.statSync(privateChild).mode & 0o777, 0o700);
  const data = path.join(privateChild, "owned-data");
  fs.writeFileSync(data, "owned-temp-fixture", { flag: "wx", mode: 0o600 });
  assert.equal(
    fs.readFileSync(
      path.join(physical, `claude-${process.getuid()}`, "owned-data"),
      "utf8",
    ),
    "owned-temp-fixture",
  );
  assert.equal(helpers.UYo(), alias);
  assert.equal(helpers.nEe(), privateChild);
  assert.equal(sharedAttempts.length, 0);
  env.CLAUDE_CODE_TMPDIR = physical;
  assert(Buffer.byteLength(physical) > 44);
  assert.equal(helpers.UYo(), "/tmp");
  assert.equal(
    helpers.nEe(),
    path.join(physical, `claude-${process.getuid()}`),
  );
  assert.deepEqual(sharedAttempts, [`/tmp/claude-${process.getuid()}`]);
  assert.equal(
    helpers.nEe(),
    path.join(physical, `claude-${process.getuid()}`),
  );
  assert.equal(sharedAttempts.length, 1); // Original memoized denied fallback.
  console.log(
    JSON.stringify({
      passed: true,
      checks: 5,
      sourceExtractionNotFullCLIQualification: true,
      checksPassed: [
        "private alias owner and physical data writer",
        "original UYo retains short private alias",
        "original nEe retains short per-uid private path without delegation",
        "long UYo exposes shared /tmp fallback",
        "long nEe catches denied shared writes and memoizes private fallback",
      ],
      credentials: false,
      network: false,
      cliLaunch: false,
      sharedTempWrites: false,
    }),
  );
} finally {
  if (wrapperCreated) fs.rmSync(wrapper, { recursive: true, force: true });
  fs.rmSync(physical, { recursive: true, force: true });
}
