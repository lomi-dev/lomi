# Linux terminal latency qualification

This opt-in fixture loads the actual packaged Workbench and terminal runtime in
WebKitGTK. It requires a debug build with `native-smoke` and the separate
`dev.lomi.terminal-latency-smoke` identifier. Never distribute this feature or its
Vite entry in a normal installer.

Use a configuration override such as:

```json
{
  "identifier": "dev.lomi.terminal-latency-smoke",
  "build": {
    "beforeBuildCommand": "pnpm ai:prepare && pnpm --filter @lomi-dev/remote-terminal-runtime build && pnpm exec vite build --config tests/native/terminal-latency-vite.config.ts"
  },
  "bundle": { "createUpdaterArtifacts": false }
}
```

Build with `pnpm tauri build --debug --no-bundle --features native-smoke --config
/absolute/probe.conf.json -- --locked`. Copy the executable to an immutable path
before the next build. In the logged-in graphical Linux session, run:

```sh
node tests/native/run-terminal-latency.mjs \
  --binary /absolute/immutable-lomi \
  --config /absolute/probe.conf.json \
  --artifacts /absolute/new-run-directory \
  --timeout 180000
```

The artifact directory must not exist. The runner records binary/config hashes
and graphics overrides, seeds a disposable project, isolates XDG application
storage and shell history, and records owned process identities for cleanup.
It never redirects `HOME` or uses the normal Lomi profile. Existing shell startup
files still run, as they do in the product. Stop all builds before measuring.

The workload covers initial input, 30 seconds of single-terminal typing, rapid
creation of 2/4/8 tabs, return to the original PTY, 2/4/8 visible split panes,
and finite 30 FPS ANSI redraw programs in one and eight visible terminals.
Every visited tab must have a native PTY; the first PTY must retain its PID.
Native input uses keypress/release events routed to the owned, active GTK WebKit
widget. Each unique five-character marker has a four-character prefix typed
outside the primary measurement. Primary latency starts immediately before the
final native key dispatch and ends when the complete marker is parsed/rendered.
Whole-marker timing is reported separately and includes intentional 8 ms
intervals between prefix keys. Exact input checks reject lost or duplicated
characters. Workload tokens, PIDs, duration, frame counts and received byte counts
reject stale or early completion evidence.

Each input phase requires p95 below 100 ms and maximum at most 250 ms. The ANSI
programs must complete normally after at least 19.5 seconds, produce at least
27 FPS, and receive the exact expected input. These are fixture criteria, not a
guarantee for every command, machine or Linux distribution. Renderer experiments
can set `LOMI_TERMINAL_LATENCY_RENDERER=dom`; graphics environment overrides are
preserved exactly. Change one variable at a time and use a fresh process/profile.

`result.json` and `checkpoint.json` contain separate input, native dispatch,
channel receipt, parse, render callback, font, context and heartbeat events.
`cleanup.json` must report retirement before comparing subsequent runs.

## Interpretation limits

The key events enter GTK directly, rather than passing through a physical
keyboard or the compositor input stack. Parser completion and xterm `onRender`
are software measurements; neither proves when the glyph reached the screen.
GTK frame timings are not associated with a particular glyph.

The PNGs are WebKit document snapshots, not GNOME compositor screenshots.
WebKit snapshots repaint flattened layers and read the WebGL display buffer.
They can differ from the buffer being presented, particularly with an
unpreserved WebGL drawing buffer. Missing text in such a PNG must not be treated
as proof of physical display latency. See the
[WebKitGTK snapshot implementation](https://github.com/WebKit/WebKit/blob/webkitgtk-2.52.6/Source/WebKit/UIProcess/API/glib/WebKitWebView.cpp#L5191)
and [canvas readback](https://github.com/WebKit/WebKit/blob/webkitgtk-2.52.6/Source/WebCore/html/HTMLCanvasElement.cpp#L656).
Use compositor capture or a manual observation to establish visible latency.

Normal builds use the ordinary Vite configuration, omit `native-smoke`, and do
not expose fixture input or snapshot commands.

The `steady-session-save-lock` regression stage loads the exact owned session,
types the marker prefix, then holds the fixture's real `SessionFile` mutex on a
blocking worker for three seconds. After lock readiness, it invokes the actual
`save_session` handler without awaiting it and submits the final native key
100 ms later. A synchronous GTK save handler blocks that key for the remaining
lock duration; the asynchronous handler lets input continue while saving waits.
`slowSessionSaves` records readiness and save completion, while
`session-lock-release.json` records the worker's measured hold and release.
The ordinary 100 ms p95 / 250 ms maximum key gates apply to this stage.

The slow-save result is valid only when the final key's host wall-clock timestamp
falls within 2.5 seconds after native lock acquisition, leaving at least 500 ms
of the intended hold, and the successful real save took at least 2.8 seconds.
Otherwise the stage fails as inconclusive. The report records the sample marker,
remaining hold at submission, and `submissionVerifiedDuringHold`; it does not
require the intentionally blocked baseline to render before saving completes.

## Recorded Ubuntu qualification: 2026-10-20

Host: Ubuntu 26.04.1, GNOME 50.1 on Wayland, WebKitGTK 2.52.6,
NVIDIA RTX 3060 Ti with driver 595.99.02. The unmodified production baseline
was commit `6e1f8447f21e242b14a967365fb641891fb4cc0d`; both executables used
the same final diagnostic harness, configuration and default graphics policy.
Builds had finished before either measurement. Profiles and PTYs were isolated.

| Measurement                                                | Baseline | Fixed version |
| ---------------------------------------------------------- | -------: | ------------: |
| Validated input samples                                    |      310 |           330 |
| Key dispatch to render callback, p95                       |    24 ms |         23 ms |
| Key dispatch to render callback, maximum                   |  2923 ms |         39 ms |
| Three-second session-lock stage, render callback           |  2923 ms |          8 ms |
| Actual save duration in that stage                         |  3016 ms |       3011 ms |
| Lock time remaining at key submission                      |  2900 ms |       2900 ms |
| Finite ANSI workloads completed normally                   |        9 |             9 |
| All eight concurrent workloads active during input samples |      Yes |           Yes |
| Owned fixture processes retired                            |      Yes |           Yes |

Every fixed-version phase met the input acceptance criteria, including rapid
creation of 2/4/8 tabs, retained PTYs, 2/4/8 visible panes and eight concurrent
30 FPS ANSI workloads. Input checks found no lost or duplicated characters.
The baseline failed the controlled session-lock phase; an unsolicited
three-second stall was not reproduced during the other phases. The contention
test establishes a concrete event-loop blocking mechanism, rather than proving
that every previously reported stall had that cause.

The normal release DEB was then built without diagnostic features and installed.
The installed executable matched the executable extracted from that DEB;
Tauri patches bundle metadata after the pre-bundling executable is copied, so
its hash differs from the retained pre-bundling copy. Installed runtime checks
passed for resource integrity, bundled Node without PATH, native MCP version,
production runtime handshake, three Custom API formats with and without a fake
key against loopback, and clean runtime shutdown.

The user then confirmed directly on the Ubuntu monitor that ordinary shell
typing and rapid opening of tabs/panes were smooth in the full normal release
application. This trial used a separate manual-test profile. It does not qualify
other Linux distributions or establish a physical per-key latency bound.

Normal DEB SHA-256:
`17b7ee926d621bec12e3d196f19ab387ad31a0e2ea986a0b6a9c673d854c4a95`.
Installed executable SHA-256:
`ab295de7044088559c3808bf17c2f75602ed4d196d66ae2cc3216e6a131b71e0`.
Raw results, process cleanup receipts, build logs, source hashes and installer
receipts are archived locally under
`desktop-audits/2026-10-20/linux-terminal-latency/verified-evidence.tar.gz`
in the parent Lomi project directory. They are not release resources.
