"use strict";
// Evaluate only hash-pinned public storage utilities, never CLI initialization.
const fs = require("node:fs"),
  fsp = require("node:fs/promises"),
  path = require("node:path"),
  crypto = require("node:crypto"),
  assert = require("node:assert/strict"),
  cp = require("node:child_process");
const artifact = process.argv[2];
if (!artifact)
  throw Error(
    "Usage: node tests/native/claude-oauth-file-store.cjs <owned-public-Claude-2.1.287-artifact>",
  );
assert.equal(process.version, "v22.22.3");
const fd = fs.openSync(
  artifact,
  fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK,
);
const metadata = fs.fstatSync(fd);
assert(metadata.isFile());
assert.equal(metadata.size, 227827120);
const digest = crypto.createHash("sha256"),
  buffer = Buffer.alloc(1024 * 1024);
for (let position = 0; position < metadata.size;) {
  const n = fs.readSync(
    fd,
    buffer,
    0,
    Math.min(buffer.length, metadata.size - position),
    position,
  );
  assert(n > 0);
  digest.update(buffer.subarray(0, n));
  position += n;
}
assert.equal(
  digest.digest("hex"),
  "6eab8333fe2121553100d8f40bfada384a3e989b94f947e18ba6677a6fcb41ea",
);
function slice(start, end, hash) {
  const data = Buffer.alloc(end - start);
  assert.equal(fs.readSync(fd, data, 0, data.length, start), data.length);
  assert.equal(crypto.createHash("sha256").update(data).digest("hex"), hash);
  return data.toString("utf8");
}
function evaluate(source, bindings, exports) {
  return new Function(
    ...Object.keys(bindings),
    source + "\nreturn {" + exports + "};",
  )(...Object.values(bindings));
}
function imports(source, bindings) {
  let count = 0;
  source = source.replace(
    /import\{([^}]+)\}from"([^"]+)";/g,
    (_, names, module) => {
      count++;
      if (!module.startsWith("/$bunfs/")) {
        const actual = require(module);
        for (const piece of names.split(",")) {
          const [name, alias] = piece.split(" as ");
          bindings[alias || name] = actual[name];
        }
      }
      return "";
    },
  );
  return { source, count };
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
const host = {},
  U = () => ({ host }),
  noop = () => {},
  diagnostics = [];
const atomicBindings = {
  P: Error,
  E: (e) => e.code,
  ee: (ms) => new Promise((r) => setTimeout(r, ms)),
  O: () => "macos",
};
let atomic = imports(
  slice(
    178877684,
    178890134,
    "6739517df0901054d60783f987c65c719211a9e1ebac694ca788aed3272fc34c",
  ),
  atomicBindings,
);
assert.equal(atomic.count, 11);
const { Tn } = evaluate(
  atomic.source.replace(/export\{[^}]+\};\s*\x00?\s*$/, ""),
  atomicBindings,
  "Tn",
);
const cache = evaluate(
  slice(
    180460759,
    180461455,
    "ae14d8771a4a2e4c26e31564eee273ea68bbff324cf96c7edbedfed65f1678bc",
  ),
  { V: HostCache, U },
  "ASt,T9r,fz,iI,tzt",
);
const root = fs.mkdtempSync("/private/tmp/lomi-claude-storage-case-");
fs.chmodSync(root, 0o700);
const helper = path.join(root, "owned-security.cjs");
fs.writeFileSync(helper, "process.exit(Number(process.env.FIXTURE_CODE));\n", {
  mode: 0o500,
});
let readCode = 44,
  writeCode = 1,
  timedOut = false;
const operations = [];
function ownedCall(operation, args, options = {}) {
  const label = args?.[0] || operation;
  operations.push(label);
  const code =
    label === "find-generic-password" || operation === "sync-find"
      ? readCode
      : writeCode;
  const result = cp.spawnSync(process.execPath, [helper], {
    env: { HOME: root, FIXTURE_CODE: String(code) },
    input: options.input || "",
    encoding: "utf8",
    timeout: 1000,
  });
  assert.equal(result.status, code);
  return { stdout: result.stdout, code, exitCode: code, timedOut };
}
const adapter = {
  mkdir: (p) => fsp.mkdir(p, { recursive: true, mode: 0o700 }),
  readFile: (p, o) => {
    assert(p.startsWith(root + "/"));
    return fsp.readFile(p, o);
  },
  readFileSync: (p, o) => {
    assert(p.startsWith(root + "/"));
    return fs.readFileSync(p, o);
  },
  unlink: (p) => {
    assert(p.startsWith(root + "/"));
    return fsp.unlink(p);
  },
};
const bindings = {
  ...cache,
  J: require("node:async_hooks").AsyncLocalStorage,
  Z: path.join,
  ne: path.join,
  re: fsp.chmod,
  yb: () => root,
  ae: () => adapter,
  oa: async () => async () => {},
  t: noop,
  y: noop,
  m: noop,
  p: noop,
  l: String,
  E: (e) => e.code,
  Y: JSON.parse,
  _: JSON.stringify,
  F: () => false,
  V: HostCache,
  U,
  cfe: "-credentials",
  jF: () => "owned.synthetic.service",
  Xk: () => "owned.synthetic.account",
  Ke: async (program, args, o) => {
    assert.equal(program, "security");
    return ownedCall("async-find", args, o);
  },
  Nm: async (program, args, o) => {
    assert.equal(program, "security");
    return ownedCall("write", args, o);
  },
  iEe: () => {
    const r = ownedCall("sync-find");
    if (r.code) throw Error("owned helper refused");
    return r.stdout;
  },
  Tn,
};
let core = imports(
  slice(
    180463868,
    180472604,
    "c529bac90493414919c7ff3896d249f3faa732ff3c795e3de2b220e9a8fb6983",
  ),
  bindings,
);
assert.equal(core.count, 4);
const storage = evaluate(
  core.source + "\nvar K;function zn(){if(K)return K;return x(P,T)}",
  bindings,
  "zn,Ka",
);
const oauthCache = {};
const oauth = evaluate(
  slice(
    181258829,
    181261650,
    "4f756274d5c51b6abbd7ea1fe23840edb5d024e1bf53d65f33d92ecb9a43f829",
  ),
  {
    zn: storage.zn,
    uU: (s) => Array.isArray(s) && s.includes("user:inference"),
    F: () => false,
    Rpe: { of: () => oauthCache },
    U,
    i: noop,
    t: noop,
    l: String,
    pu: () => ({}),
    jw: (x) => x,
    aIe: noop,
    Cf: noop,
    Hk: noop,
    Uk: noop,
    _n: noop,
    hwn: async () => {},
    ee: (ms) => new Promise((r) => setTimeout(r, ms)),
  },
  "zQn,WQn",
);
fs.closeSync(fd);
const credentials = path.join(root, ".credentials.json");
const token = (n) => ({
  accessToken: "synthetic-access-" + n,
  refreshToken: "synthetic-refresh-" + n,
  expiresAt: Date.now() + 60000,
  scopes: ["user:inference"],
  clientId: "owned-fixture",
});
(async () => {
  try {
    const first = token(1),
      second = token(2),
      third = token(3);
    assert.equal((await oauth.zQn(first)).success, true);
    assert.equal(
      (await storage.zn().readAsync()).claudeAiOauth.accessToken,
      first.accessToken,
    );
    assert.equal(fs.statSync(credentials).mode & 0o777, 0o600);
    assert.equal(
      await oauth.WQn({
        isCompromised: () => false,
        postedRefreshToken: first.refreshToken,
        refreshedTokens: second,
      }),
      "saved",
    );
    assert.equal(
      (await storage.zn().readAsync()).claudeAiOauth.refreshToken,
      second.refreshToken,
    );
    assert.equal(
      await oauth.WQn({
        isCompromised: () => false,
        postedRefreshToken: first.refreshToken,
        refreshedTokens: third,
      }),
      "adopted_sibling",
    );
    assert.equal(
      (await storage.zn().readAsync()).claudeAiOauth.refreshToken,
      second.refreshToken,
    );
    const before = fs.readFileSync(credentials);
    readCode = 1;
    storage.zn().invalidateCache();
    assert.equal((await oauth.zQn(third)).success, false);
    assert.deepEqual(fs.readFileSync(credentials), before);
    readCode = 44;
    timedOut = true;
    storage.zn().invalidateCache();
    assert.equal((await oauth.zQn(third)).success, false);
    assert.deepEqual(fs.readFileSync(credentials), before);
    timedOut = false;
    assert.equal(
      (
        await storage
          .zn()
          .mutate((value) => ({ ...value, fixturePadding: "x".repeat(3000) }))
      ).success,
      true,
    );
    assert.equal((await oauth.zQn(third)).success, true);
    assert(operations.includes("-i"));
    assert(operations.includes("add-generic-password"));
    const lockedBefore = fs.readFileSync(credentials);
    readCode = 36;
    storage.zn().invalidateCache();
    cache.fz().keychainHoldsItem = true;
    assert.equal((await oauth.zQn(token(4))).success, false);
    assert.deepEqual(fs.readFileSync(credentials), lockedBefore);
    readCode = 44;
    storage.zn().invalidateCache();
    assert.equal(await storage.zn().delete(), true);
    assert.equal(fs.existsSync(credentials), false);
    console.log(
      JSON.stringify({
        artifact: "Claude 2.1.287",
        artifactSha256:
          "6eab8333fe2121553100d8f40bfada384a3e989b94f947e18ba6677a6fcb41ea",
        synthetic: true,
        sourceExtractionNotFullCLIQualification: true,
        checks: [
          "save",
          "read",
          "refresh-CAS",
          "stale-refresh-adoption",
          "private-mode",
          "unknown-read-preservation",
          "timeout-no-fallback",
          "locked-primary-preservation",
          "stdin-and-argv-helper-contract",
          "logout-delete",
        ],
        securityOperations: [...new Set(operations)],
        actualKeychain: false,
        network: false,
      }),
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error.stack);
  process.exitCode = 1;
});
