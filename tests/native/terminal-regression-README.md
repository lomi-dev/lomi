# Native macOS and Windows terminal regression

This opt-in fixture packages the real Workbench with an early diagnostic entry.
Build only with `native-smoke`; do not enable `android-probe`, ship this entry,
or reuse the normal application identifier. Choose a fresh build identifier
for each run, such as `dev.lomi.terminal-regression-20261020-macos-a`.

Write an absolute configuration override:

```json
{
  "identifier": "dev.lomi.terminal-regression-20261020-macos-a",
  "build": {
    "beforeBuildCommand": "pnpm ai:prepare && pnpm --filter @lomi-dev/remote-terminal-runtime build && pnpm exec vite build --config tests/native/terminal-regression-vite.config.ts"
  },
  "bundle": { "createUpdaterArtifacts": false }
}
```

Build and copy the executable to a stable path before further builds:

```sh
pnpm tauri build --debug --no-bundle --features native-smoke --config /absolute/regression.conf.json -- --locked
node tests/native/run-terminal-regression.mjs --binary /absolute/lomi --config /absolute/regression.conf.json --directory /absolute/new-artifacts
```

On macOS, build with `--bundles app` instead of `--no-bundle`, preserve the
entire `.app` at its stable path, and pass its `Contents/MacOS/lomi` executable
to the runner.

Run from the logged-in native graphical session. Windows requires the fixture
window in the foreground. Stop builds before measuring. The runner rejects existing artifact
and application-data directories. It uses the platform application-data root
with the exact build identifier and verifies that path inside the native app.
It does not override HOME or edit shell profiles; existing product shell startup
files still run. It selects local zsh on macOS and local PowerShell on Windows.
Node is the runner's absolute `process.execPath`, copied only as a path into the
fixture configuration; the workload script lives in the disposable project.

The exercise phase checks 25 seconds of ordinary shell typing, real shortcut
handlers opening 2/4/8 tabs, 2/4/8 visible split panes, all fifteen native PTY
PIDs, first-runtime/PID retention, tab switching, and eight concurrent finite
20-second ANSI workloads. Each measured marker must arrive exactly through
xterm onData and appear in parser/render callbacks. Every ANSI input sample
requires all eight same-token/PID READY markers and no DONE markers. The finite
programs record exact input, frames and duration; normal completion and retired
workload processes are required. The runner requires per-stage final-character
request-to-render p95 below 100 ms and maximum at most 250 ms.

Ordinary input waits for shell integration's prompt marker. Every Return must
produce a different live prompt marker before the next ordinary sample, proving
that the shell executed the command. Two startup warmup samples per launch are
retained in the report but excluded from steady-state timing gates.

Windows exercise quit calls the actual window close request; macOS exercise
quit emits the application's `lomi-quit-requested` event. The fixture writes a
ready receipt and never exits the app itself. Workbench must save and destroy
the main window. The runner observes normal process exit and verifies eight
saved tabs/fifteen panes, relaunches, activates all restored tabs, verifies fresh
PTY PIDs for the same pane IDs, sends native input, and quits through Workbench
again. A failure-only timeout targets the runner's own ChildProcess handle;
there is no blanket process termination or profile cleanup.
On macOS, a native close/hide/reopen cycle also verifies all retained PTYs and
the original terminal runtime before quitting. After each guarded quit, all
fifteen owned PTY PIDs must retire before the next launch.

Artifacts include run identity, native logs, ready receipts with samples and
stage/PID records, workload JSON, and runner-verified phase receipts. A failed
frontend assertion records the active checkpoint and error before requesting
product quit. Artifacts and isolated profiles remain available for review.

Native input is owned AppKit firstResponder `insertText:replacementRange:` on
macOS and bounded `WM_CHAR` to a verified foreground owned HWND on Windows.
Windows Return uses bounded `VK_RETURN` keydown/up to the same verified focus
instead of a text character message.
macOS shows and focuses its fixture window before waiting for the initial
terminal; the reopen
check explicitly focuses it again. Individual text dispatches target the
verified owned responder without retaking global focus. Every native dispatch records `keyWindow`; final-key
foreground and inactive timings have separate summaries and the same gates.
The isolated macOS fixture stays above other windows to prevent occlusion from
suspending render callbacks during measurements.
Keep it on the currently visible macOS Space and monitor. Always-on-top does
not make a window on another Space visible. WebKit can suspend animation frames
and font-dependent startup there; such a run does not qualify terminal latency.
Inactive committed-text tests qualify this software pipeline, not foreground
keyboard routing or physical presentation.
These are committed-text paths, not physical keyboard events. Tab/split
shortcuts use DOM key events through actual application shortcut handlers.
Parser completion and xterm onRender are software callbacks, not compositor
presentation or input-to-photon measurements. This fixture does not cover
corrupt-session recovery; use the existing session recovery lifecycle UI tests
for that behavior.

## Recorded qualification: 2026-10-20

The final macOS trial used a complete debug `.app` on macOS 27.0.1, Apple
Silicon, after the Remote queue fix. Both the exercise and
restoration phases passed with all sampled final keys in the foreground:

| Stage                           | Samples |   p95 | Maximum |
| ------------------------------- | ------: | ----: | ------: |
| Ordinary shell, 25 seconds      |      71 | 17 ms |   18 ms |
| Eight concurrent ANSI programs  |      24 | 49 ms |   50 ms |
| Input after session restoration |       5 | 15 ms |   15 ms |

The trial also passed rapid 2/4/8-tab and 2/4/8-pane creation, exact input,
fifteen distinct live PTYs, retained runtime/PID checks, native close/hide/reopen,
and guarded quit. Both quit cycles saved eight tabs/fifteen panes and retired
all fifteen owned PTYs. Restoration created fresh shells for the same pane IDs.
This candidate included the existing pane-transfer commit `54b319f`.

Earlier fixture trials exposed focus/occlusion and tab-discovery problems; these
were corrected before the passing run. An earlier eight-program trial exceeded
the p95 gate. Later startup attempts on an inactive Space recorded zero
animation frames and a hidden document despite a non-minimized native window.
The user confirmed the window was on another Space/monitor; the final visible
run passed both phases. Observed Remote offer maximum in that final exercise
was 0.252 ms across 7,004 retained records, with 11 diagnostic records dropped
during export contention. A single passing run does not establish a worst-case latency bound
for every workload or machine. Raw passing and diagnostic trials are retained
in the parent project's local
`desktop-audits/2026-10-20/terminal-cross-platform-regression/` directory.

The optional native-output trace records reader completion, Remote observer
submission and local channel submission separately. It is restricted to the
isolated regression identity, bounded to 16,384 records, and compiled only with
`native-smoke`. Reader-side trace contention drops diagnostic records rather
than blocking terminal output. Trace exports and generated shell output remain
local test artifacts.

The Windows trial used Windows 10 Enterprise LTSC, build 19044, x64 and
WebView2. Both exercise and restoration passed after removing Remote queue
capacity waiting from local PTY producers:

| Stage                           | Samples |     p95 | Maximum |
| ------------------------------- | ------: | ------: | ------: |
| Ordinary shell, 25 seconds      |      48 | 39.6 ms | 40.4 ms |
| Eight concurrent ANSI programs  |      24 | 54.2 ms | 54.6 ms |
| Input after session restoration |       5 | 36.5 ms | 36.5 ms |

The preceding Windows run failed the unchanged ANSI p95 gate at 103.8 ms,
with a maximum of 137.7 ms. Native timestamps isolated 94.1 ms of the slowest
marker response inside the Remote queue offer, before local channel submission.
Observed Remote offer maximum across the full trace fell from 255.289 ms to
0.075 ms after the fix. Trace export contention dropped 21/23 diagnostic records
before/after; these observed maxima are not universal latency bounds.

Successful Remote offers preserve their order. A full bounded observer queue
now immediately fences only the affected session generation using the existing
unavailable-model recovery path. It no longer makes local output or resizing
wait up to 250 ms. A transient full queue can therefore make that session's
Remote model unavailable earlier; local terminal bytes remain intact. The
native PTY regression also verifies exact local output, acknowledgements,
resizing, close and EOF while the Remote consumer never drains its queue.
