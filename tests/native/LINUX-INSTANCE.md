# Linux instance lifecycle regression

This opt-in fixture runs the real Workbench in packaged WebKitGTK. Build a debug
executable with `native-smoke` and a separate, unique
`dev.lomi.linux-instance-smoke-*` identifier. Never distribute the diagnostic
feature or Vite entry in a normal installer.

Use a configuration override such as:

```json
{
  "identifier": "dev.lomi.linux-instance-smoke-qualification",
  "build": {
    "beforeBuildCommand": "pnpm ai:prepare && pnpm --filter @lomi-dev/remote-terminal-runtime build && pnpm exec vite build --config tests/native/linux-instance-vite.config.ts"
  },
  "bundle": { "createUpdaterArtifacts": false }
}
```

Build on Linux:

```sh
pnpm tauri build --debug --no-bundle --features native-smoke \
  --config /absolute/probe.conf.json -- --locked
```

Retain the executable at an immutable path before another build. Stop compilers
before the trial. In the logged-in graphical session, with Python 3, GI and
Atspi 2.0 available, run:

```sh
node tests/native/linux-instance-run.mjs \
  --binary /absolute/immutable-lomi \
  --config /absolute/probe.conf.json \
  --directory /absolute/new-run-directory \
  --source-revision FULL_COMMIT_ID
```

The directory must not exist. The runner seeds a disposable project with one
Bash tab and two terminal panes, isolates XDG directories and `HISTFILE`, and
preserves `HOME`, shell startup files and caller graphics settings. It records
binary, config and fixture hashes. `--source-revision` records the supplied
binary source identity; `runnerRevision` is independently read from Git and is
`null` in source archives without Git metadata.

The trial checks:

- Four normal secondary launches while the primary is hidden before importing
  the real frontend. They must exit successfully; activation stays pending until
  frontend readiness.
- Two ordinary close-to-background and reopen cycles, each with eight secondary
  launches. Native visibility/focus, exact PTY identities/PIDs, retained JS
  runtime objects and parsed terminal output must survive.
- Real AT-SPI desktop enumeration after the first reopen, including
  `get_process_id()` for the exact primary PID and a visible accessible frame
  with children. Traversal is bounded to four levels and 256 nodes; stale GLib
  objects are tolerated, but missing GI, inaccessible app or primary death fails.
- One native Secret Service scratch credential round trip under the fixture's
  service and token-specific username: set, matching read, delete and confirmed
  absence. Cleanup is attempted after write/read failure. Only status flags are
  recorded; real credentials are never enumerated.
- Product guarded quit, saved pane IDs and retirement of owned processes, then
  restoration of the same profile with fresh PTYs, working input and another
  guarded quit.

Before restore, an owned Python helper holds both real profile owner locks. The
runner requires an atomic `restore-owner-contended.json` receipt from the exact
restored process, emitted on the first actual native lock contention. Only then
it holds the locks for at least another 250 ms, requiring the app to stay alive
without frontend startup. It releases the helper through stdin, requires graceful
helper exit, and completes restoration. `handoff.json` records lock paths,
contention, timestamps, overlap and release; missing contention cannot pass.

`result.json`, per-stage receipts, `accessibility.json`, secondary exit records,
logs and saved sessions retain the evidence. `cleanup.json` records PID/start-time
ownership, survivors and unproven processes. Success requires normal product
shutdown; signals are reserved for failed-run cleanup of verified owned processes.
The native hooks require the exact fixture identifier, token and owned XDG root.

These checks establish software lifecycle, storage handoff and native integration
behavior for the tested host. They do not measure physical presentation or input
latency, qualify every desktop/driver, or establish normal installer behavior.
The AT-SPI helper can also probe an ordinary installed release directly:

```sh
python3 tests/native/linux-instance-accessibility.py PRIMARY_PID
```

## Recorded Fedora qualification: 2026-10-20

Run 02 passed on Fedora 44, GNOME 50.6 on Wayland, WebKitGTK 2.54.1 and
Mesa 26.2.4 with Nouveau on an NVIDIA RTX 3060 Ti. The candidate used production
baseline `26938ea` plus the reviewed Linux reopen and shared-D-Bus changes.
Diagnostic executable SHA-256:
`3d3e776a73d82a85226c26a79c77821e05a7d22e0fb4b759b85011893786665b`.

All 20 secondary launches exited successfully: four before frontend readiness,
then eight in each of two hide/reopen cycles. Primary PID `39789` retained PTYs
`40040` and `40041`, runtime identity and working input. Restore PID `40378`
loaded the same saved panes with fresh PTYs `40567` and `40568`.

The restore emitted an actual native `WouldBlock` contention receipt. Both owner
locks remained held for another 259.816 ms after observation while restore stayed
alive without frontend startup; the helper then released them and exited
gracefully, and restoration completed. Real AT-SPI enumeration verified exact
PID `39789`, one visible frame, two frame children and nine probed nodes, with no
stale-object errors. The scratch credential set/read/delete/absence checks all
passed.

Both guarded quits completed. Cleanup reported `retired: true` and
`forcedCleanup: false`, with no survivors, unproven processes or errors. This is
a diagnostic lifecycle qualification under the limits above; it does not itself
qualify an ordinary installer. Results and receipts are archived locally under
`desktop-audits/2026-10-20/fedora-terminal-qualification/linux-instance-run-02/`
in the parent Lomi project directory, outside release resources.
