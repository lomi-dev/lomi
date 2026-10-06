import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  realpath,
  chmod,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn, execFileSync } from "node:child_process";

if (process.platform !== "darwin")
  throw Error("This desktop probe currently supports macOS.");
const repository = resolve(import.meta.dirname, "../..");
const directory = await realpath(
  await mkdtemp(join(tmpdir(), "lomi-agent-runtime-desktop-")),
);
const identifier = "dev.lomi.agent-runtime-production-smoke-20261006";
const config = { identifier, build: { devUrl: null } };
const executable =
  process.env.LOMI_AGENT_RUNTIME_SMOKE_BINARY ??
  join(repository, "src-tauri/target/debug/lomi");
const build = async (command, args) => {
  const child = spawn(command, args, {
    cwd: repository,
    env: { ...process.env, TAURI_CONFIG: JSON.stringify(config) },
    stdio: "inherit",
  });
  const code = await new Promise((resolve) => child.once("exit", resolve));
  if (code !== 0) throw Error(`${command} build failed: ${code}`);
};
if (!process.argv.includes("--no-build")) {
  await build("pnpm", ["build"]);
  await build("cargo", [
    "build",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--locked",
    "--features",
    "native-smoke,tauri/custom-protocol",
  ]);
}
console.log(`Native agent runtime evidence: ${directory}`);
const activationHelper = join(directory, "activate-own-app");
execFileSync("swiftc", [
  join(repository, "tests/native/agent-runtime-activate.swift"),
  "-o",
  activationHelper,
]);
const results = [];
for (const failure of [false, true]) {
  const fixture = join(directory, failure ? "retention" : "positive");
  const home = join(fixture, "home");
  const project = join(fixture, "project");
  const appData = join(home, "Library/Application Support", identifier);
  const runtime = join(appData, "agent-runtime");
  await mkdir(runtime, { recursive: true });
  await chmod(runtime, 0o700);
  await mkdir(project, { recursive: true });
  await mkdir(join(fixture, "tmp"), { recursive: true });
  await writeFile(
    join(appData, "agent-control-preferences.json"),
    JSON.stringify({ version: 1, autoStart: false, yoloMode: false }),
  );
  const taskId = "offline-archive-fixture";
  const task = {
    taskId,
    cwd: project,
    title: "Offline archived task",
    cli: null,
    availabilityReason:
      "Synthetic isolated native fixture; no executable account or credentials.",
    model: "",
    reasoningEffort: null,
    revision: 1,
    historyRevision: 1,
    generation: 1,
    state: "archived",
    nextAccountId: "",
    activeAccountId: null,
    activeAttemptId: null,
    statusMessage: "Read-only archive fixture. No inference or credentials.",
    attempts: failure
      ? [
          {
            attemptId: "fixture-attempt",
            operationId: "fixture-operation",
            accountId: "fixture-account",
            authRevision: 1,
            generation: 1,
            input: "",
            continuationMethod: "legacy",
            state: "recovery_required",
            output: "Preserved original partial output",
            nativeRef: null,
            version: null,
            effectsState: "uncertain",
          },
        ]
      : [],
    history: [
      {
        sequence: 1,
        attemptId: failure ? "fixture-attempt" : "offline-original",
        accountId: "fixture-account",
        authRevision: 1,
        kind: "legacy_record",
        state: "archived",
        content: {
          output: "Preserved original partial output",
          state: "recovery_required",
        },
      },
    ],
    grants: [],
    switches: [],
  };
  const tabs = [
    { type: "agent-task", id: "task-a", title: "Archive A", taskId },
    { type: "agent-task", id: "task-b", title: "Archive B", taskId },
    {
      type: "terminal",
      id: "terminal-target",
      title: "Dock target",
      profileId: "local:sh",
      layout: { type: "terminal", id: "terminal-pane", cwd: project },
      activePaneId: "terminal-pane",
    },
  ];
  const session = {
    version: 5,
    projects: [
      {
        id: "project",
        path: project,
        workspaces: [
          {
            id: "workspace",
            name: "Native smoke",
            tabs,
            activeTabId: "task-a",
          },
        ],
        activeWorkspaceId: "workspace",
      },
    ],
    activeProjectId: "project",
    sidebar: "files",
    rightSidebar: null,
    sidebarSides: { files: "left", git: "right", workspaces: "left" },
    terminalOverviewSide: "left",
    sidebarWidth: 250,
    rightSidebarWidth: 250,
  };
  await writeFile(join(appData, "session.json"), JSON.stringify(session));
  await writeFile(join(fixture, "task.json"), JSON.stringify(task));
  execFileSync("python3", [
    "-c",
    `import sqlite3,json,os,sys,subprocess
root,project,failure=sys.argv[1:]
db=sqlite3.connect(root+'/home/Library/Application Support/${identifier}/agent-runtime/runtime.sqlite')
db.executescript('CREATE TABLE metadata(id INTEGER PRIMARY KEY,revision INTEGER NOT NULL); INSERT INTO metadata VALUES(1,0); CREATE TABLE tasks(id TEXT PRIMARY KEY,record TEXT NOT NULL,shell TEXT NOT NULL,directory_identity TEXT NOT NULL); CREATE TABLE process_ownership(task_id TEXT NOT NULL,attempt_id TEXT PRIMARY KEY,account_id TEXT NOT NULL,generation INTEGER NOT NULL,process_group INTEGER,identity TEXT,ownership_marker TEXT NOT NULL,boot TEXT NOT NULL,unknown_tools TEXT NOT NULL,state TEXT NOT NULL); PRAGMA user_version=1; PRAGMA application_id=1280265545;')
task=json.load(open(root+'/task.json')); st=os.stat(project)
db.executescript('CREATE TABLE accounts(id TEXT PRIMARY KEY,record TEXT NOT NULL,binding TEXT NOT NULL); CREATE TABLE checkpoints(task_id TEXT PRIMARY KEY REFERENCES tasks(id),record TEXT NOT NULL); CREATE TABLE receipts(operation_id TEXT PRIMARY KEY,request TEXT NOT NULL,result TEXT NOT NULL); CREATE TABLE outbox(operation_id TEXT PRIMARY KEY REFERENCES receipts(operation_id),task_id TEXT NOT NULL,attempt_id TEXT NOT NULL,generation INTEGER NOT NULL,state TEXT NOT NULL); CREATE TABLE imports(digest TEXT PRIMARY KEY,record TEXT NOT NULL);')
db.execute('INSERT INTO tasks VALUES(?,?,?,?)',(task['taskId'],json.dumps(task),'local:sh',json.dumps([st.st_dev,st.st_ino])))
if failure=='true':
 boot=subprocess.check_output(['/usr/sbin/sysctl','-n','kern.bootsessionuuid'],text=True).strip()
 db.execute('INSERT INTO process_ownership VALUES(?,?,?,?,?,?,?,?,?,?)',(task['taskId'],'fixture-attempt','fixture-account',1,None,None,'a'*64,boot,json.dumps(['native_background_or_lifecycle_unqualified']),'untracked'))
db.commit();db.close()`,
    fixture,
    project,
    String(failure),
  ]);
  await chmod(join(runtime, "runtime.sqlite"), 0o600);
  // Match the normal desktop supervisor: launchd bootstrap requires an unsandboxed
  // caller on the admitted host. Native children retain the held boundary sandbox;
  // an externally sandboxed supervisor fails closed before releasing any child.
  const child = spawn(executable, [], {
    cwd: fixture,
    env: {
      HOME: home,
      TMPDIR: join(fixture, "tmp"),
      PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
      SHELL: "/bin/sh",
      LANG: "en_US.UTF-8",
      LOMI_AGENT_RUNTIME_SMOKE_DIRECTORY: fixture,
      LOMI_AGENT_RUNTIME_PUBLIC_CLAUDE:
        "/private/tmp/lomi-public-cli-help-c2hyiv50/claude",
      ...(failure ? { LOMI_AGENT_RUNTIME_SMOKE_FAILURE: "1" } : {}),
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  await writeFile(join(fixture, "pid"), String(child.pid));
  for (const stream of [child.stdout, child.stderr])
    stream.on("data", (data) => {
      log += data;
    });
  try {
    const deadline = Date.now() + 120000;
    let evidence;
    let activated = false;
    while (Date.now() < deadline) {
      if (!activated) {
        try {
          const activation = execFileSync(
            activationHelper,
            [String(child.pid)],
            { encoding: "utf8" },
          );
          await writeFile(join(fixture, "activation.log"), activation);
          activated = true;
        } catch {}
      }
      const failed = await readFile(join(fixture, "failed.json"), "utf8")
        .then(JSON.parse)
        .catch(() => null);
      if (failed) throw Error(JSON.stringify(failed));
      evidence = await readFile(
        join(fixture, failure ? "retention-passed.json" : "ready-to-quit.json"),
        "utf8",
      )
        .then(JSON.parse)
        .catch(() => null);
      if (failure && evidence) break;
      if (child.exitCode !== null || child.signalCode !== null) break;
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
    if (!evidence) throw Error("Native renderer did not report evidence");
    if (evidence.pid !== child.pid)
      throw Error("Evidence came from unexpected process");
    if (
      failure
        ? child.exitCode !== null || child.signalCode !== null
        : child.exitCode !== 0
    )
      throw Error("Guarded quit/retention process outcome differs");
    const calls = (await readFile(join(fixture, "calls.jsonl"), "utf8"))
      .trim()
      .split("\n")
      .map(JSON.parse);
    if (!failure) {
      const prepare = calls.findIndex(
        (c) => c.command === "agent_runtime_prepare_close",
      );
      const drain = calls.findIndex((c) => c.command === "agent_tasks_drain");
      if (
        prepare < 0 ||
        drain <= prepare ||
        calls[prepare].completed !== true ||
        calls[drain].completed !== true ||
        !calls
          .slice(prepare + 1, drain)
          .some((c) => c.command === "save_session" && c.completed === true)
      )
        throw Error("Normal quit did not complete prepare, save, then drain");
      for (const command of [
        "agent_task_prepare_close",
        "agent_task_close_release",
      ]) {
        const completed = calls.filter((c) => c.command === command);
        if (completed.length !== 1 || completed[0].completed !== true)
          throw Error("Final task close did not complete exactly once");
      }
    } else {
      const closes = calls.filter(
        (c) => c.command === "agent_task_prepare_close",
      );
      const drains = calls.filter((c) => c.command === "agent_tasks_drain");
      if (
        closes.length !== 1 ||
        closes[0].completed !== false ||
        !drains.length ||
        drains.some((c) => c.completed !== false)
      )
        throw Error("Protected task close/drain was not refused natively");
    }
    const counts = JSON.parse(
      execFileSync(
        "python3",
        [
          "-c",
          "import sqlite3,json,sys; c=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True); print(json.dumps({t:c.execute('select count(*) from '+t).fetchone()[0] for t in ['tasks','accounts','outbox','process_ownership']}))",
          join(runtime, "runtime.sqlite"),
        ],
        { encoding: "utf8" },
      ),
    );
    if (counts.accounts || counts.outbox)
      throw Error("Offline archive fixture created account or dispatch");
    const ownedProductionEntry = failure
      ? null
      : JSON.parse(
          await readFile(join(fixture, "owned-production-entry.json"), "utf8"),
        );
    if (!failure && !ownedProductionEntry.passed)
      throw Error("Owned normal entry did not complete");
    results.push({
      ownedProductionEntry,
      fixture,
      outcome: failure
        ? "close-and-quit-refused-view-retained"
        : "normal-quit-exited-zero",
      evidence,
      calls,
      counts,
    });
  } finally {
    if (child.exitCode === null && child.signalCode === null)
      child.kill("SIGTERM");
    await writeFile(join(fixture, "native.log"), log);
  }
}
await writeFile(
  join(directory, "result.json"),
  JSON.stringify(
    {
      passed: true,
      identifier,
      network: "native child outbound denied; supervisor unsandboxed",
      results,
    },
    null,
    2,
  ),
);
console.log(JSON.stringify({ passed: true, directory, results }, null, 2));
