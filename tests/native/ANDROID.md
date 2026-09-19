# Android native verification

The production backend and lazy Android UI are included in normal builds. The
additional commands and drivers in this directory require `android-probe` and
must never be enabled in distributed builds. Native qualification currently
covers only the macOS ARM64 host recorded in
[the architecture guide](../../docs/android-architecture.md#native-qualification).
Linux, Windows and other Mac architectures remain unverified.

## Windows preflight

Before downloading Android tools, run the read-only host and artifact checks
from PowerShell with a new evidence directory (the script refuses overwrite):

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/native/android-windows-preflight.ps1 -OutputDirectory android-test/trials.local/preflight-UNIQUE
```

The execution-policy setting applies only to this child process. The script
does not enable Windows features, accept licenses, start Android or modify user
configuration. It records WMI hardware, disk, installed WebView2, system DPI,
the real `WHvGetCapability` result and IME/protobuf hashes. Environment variable
names are recorded without values. Optional-feature inspection needs an
administrator; its failure is recorded separately from the WHPX API result.
System DPI is not a substitute for per-window DPR measurements in WebView2.
Exit 2 means unavailable/unreadable WHPX or an artifact integrity failure.
A successful preflight is not native product qualification.

When WHPX is unavailable, an administrator must enable **Windows Hypervisor
Platform** in Windows Features and restart Windows before repeating preflight.
See [Google's acceleration instructions](https://developer.android.com/studio/run/emulator-acceleration).
Do not remove the platform guards or reuse the macOS product runner as Windows
evidence. The [Windows trial report](../../docs/android-windows-qualification.md)
records the current blockers and independent checks.

The selected transport is authenticated emulator gRPC → Rust → binary Tauri
Channel → retained WebGL Canvas, with source scaling and a 720 × 1280 ceiling.
The original feasibility runner also contains rejected JPEG/2D experiments.
Its results do not replace verification of the integrated product.

The integrated v3 product completed 1,800 uninterrupted visible seconds at
720 × 1280: 28.03 FPS, 0.2993 additional CPU cores including the guest,
103.32 MB/s IPC and 0.01561 s/s additional GPU active intervals. See
the summary in the architecture guide; raw historical reports are kept outside
the source repository.
The memory report includes every identified app/webview process and explains
the higher terminal-view baseline; it does not claim a negative transport cost.

## Integrated product fixture

`run-android-product.mjs` builds the actual workbench and Settings UI with a
unique application identifier and separate app-data/workspace. It reuses only
the isolated `native-managed-*` SDK installed by the native installer trial;
it never uses the user's SimpleBench session, SDK or AVD. The directory owner
lock remains active. The product's Rust runtime owns emulator children and a
private ADB listener on 15047. File pickers open at fixed trial locations in
this feature-only build; selection and all file/guest operations still use
production commands. The fixture never accepts vendor terms.

```sh
node --experimental-strip-types tests/native/run-android-product.mjs \
  /tmp/simplebench-android-stage0-20260918/native-managed-<trial-id>
```

Supply a working `SDKROOT` for the local Xcode installation when necessary.
`--reuse-binary` verifies the previous executable digest, app identifier and
canonical managed directory before reopening that isolated session. Rebuild
after Rust changes; do not overwrite a running fixture binary.

Drive the actual trusted application webview using unique instruction IDs:

```sh
node tests/native/android-product-control.mjs \
  /tmp/simplebench-android-stage0-20260918/product inspect-1 <<'JSON'
{"window":"main","script":"return {text:document.body.innerText,visibility:document.visibilityState};"}
JSON
```

`window` is `main` or `settings`. Optional `processId` restricts an instruction
to one fixture process during the second-instance test; otherwise never run two
product drivers against the same instruction directory. The bounded script receives `invoke`, `click`,
`sleep` and `wait` helpers. Native actions `present`, `hide`, `minimize`, `resize`
and `process-info` remain available when WebKit suspends JavaScript. `present`
changes only the test window's level/workspace visibility. Screenshot and
AppKit text/pointer commands inspect or operate only the fixture's windows.
Never use these drivers against an ordinary user profile. Normal window close
runs the real guards, session save, installer cancellation and device Stop.

`android-product-perf.py` samples the product and its identified WebKit/child
processes, the owned emulator and private ADB separately. It uses a guarded
private smart-socket connection to animate the same guest during hidden and
visible phases, never an ADB client executable. A full run requests 30 visible
minutes between a baseline and a hidden phase. It retains frame counters,
resource samples and incomplete/error outcomes in the isolated `product/`
directory. CPU-heavy builds and other tests should run outside this measurement.

```sh
python3 tests/native/android-product-perf.py \
  /tmp/simplebench-android-stage0-20260918 \
  --device <fixture-device-uuid> --name product-qualification --duration 1800
```

The driver requires the prepared phone to be open as “Native UI phone” with a
“Terminal” tab available. It is development instrumentation, not an application
runtime dependency. Its draw-call timing is not input-to-presentation latency;
that requires the separate controlled guest input test and GPU trace.

Check the actual emulator renderer before comparing results. The driver reads
`SurfaceFlinger` through the guarded fixture transport and records it alongside
the device configuration and executable digest. Managed logs are flushed at Stop;
a log from the previous generation cannot identify a currently running renderer.
The recorded headless macOS `auto` choice selected SwiftShader; the qualified
stage-0 configuration used `host`. Preserve failed trials instead of treating a
different graphics backend as an equivalent baseline. Hardware changes require
Stop and another Start and must be applied through the product's device settings.

With the disposable input-test APK installed through the product's native picker:

```sh
node tests/native/android-product-interaction.mjs ROOT/product DEVICE_UUID UNIQUE
node tests/native/android-product-latency.mjs ROOT/product DEVICE_UUID UNIQUE
node tests/native/android-product-reconnect.mjs ROOT/product DEVICE_UUID UNIQUE
node tests/native/android-product-multidevice.mjs ROOT/product FIRST_UUID SECOND_UUID UNIQUE
```

For native text and the actual host clipboard, compile
`android-clipboard.swift` to `ROOT/android-clipboard` with `xcrun swiftc`, then run
`node tests/native/android-product-text.mjs ROOT/product DEVICE_UUID UNIQUE`.
The helper preserves every clipboard item/type in memory, refuses an oversized
snapshot before changing anything and restores it after the bounded test unless
another application has since changed the clipboard. No saved clipboard contents
are logged or written to disk. The driver checks AppKit insertion/composition
through WKWebView and separately selects the product's **Paste** action.

The interaction runner checks native pointer input at all four rotations, drag
completion, release on a tab switch and letterbox rejection. The latency runner
measures 50 native pointerdown events against the fixture app's alternating
black/white target. It reads the rendered WebGL framebuffer and records the second
animation frame after the observed guest change. This includes the guest response
and presentation scheduling; it is not an IPC acknowledgement time. Run it apart
from CPU/GPU resource sampling because framebuffer readback changes the workload.

The reconnect runner withholds actual product frame acknowledgements until the
native two-second timeout. It then uses the product's Reconnect action, sends an
old epoch's ACK while the new frame is pending, and checks source/IPC counters
before releasing the current ACK. Android must retain its process generation.
The interceptor is test instrumentation and is restored in `finally`; it neither
replaces the backend nor fabricates frames. The test temporarily enlarges the
owned window to avoid source-size changes while an error banner is visible.

The product runner can start with an empty managed directory before the fixture
APK exists, allowing the complete Settings installer flow to be tested first.
APK and input tests still require the separately built `input-test.apk`; put it
in the isolated `product/apk-selection/` directory before opening the native picker.
The fixture build tools are never part of the product installer.

`node tests/native/android-product-install.mjs ROOT/product UNIQUE tools`
drives the exact plus-menu entry and Settings from an empty managed SDK. Repeat
with `image` to select the catalog's AOSP API 36 ARM64 revision 2. The driver
compares every displayed license with the exact hashes of previously recorded
user consent before checking the boxes. Unknown terms stop the trial. Downloads,
Java/CLI execution, package verification and publication all use production code;
progress and the final result are saved under the isolated product directory.

The multi-device runner requires two real phones, each with the input-test APK,
docked into exactly two visible panels. It checks distinct serials/generations,
one stream per phone, and native touch/Unicode focus without changing the other
guest's state. Prepare the panes with the actual UI before invoking it.

`android-product-cancel-close.mjs ROOT/product FIRST_UUID SECOND_UUID UNIQUE`
requests a real main-window close with two running phones. It waits until one
child has exited while the other is still alive, selects **Cancel closing**,
then checks that descriptors remain and stopped phones do not restart. An
explicit Start verifies that the preparation gate was released. Use only the
disposable fixture: if cancellation fails, the actual test application exits.

`android-product-docking.mjs ROOT/product DEVICE_UUID UNIQUE` creates a second
view of the running default fixture phone, then uses AppKit pointer events to
dock both into a new **Android views** terminal tab. It preserves **Terminal**
as the performance baseline. Run the performance driver with `--views 2
--active-tab 'Android views'` to measure shared transfer and additional drawing.
`android-product-shared-input.mjs ROOT/product DEVICE_UUID UNIQUE` verifies
native pointer coordinates in all four rotations of the smaller shared
framebuffer, then checks physical 1:1 size and Fit while retaining one source.
Activate the test window for native input; visibility alone does not grant
keyboard focus.

For the separate two-device input test, the first native APK picker trial is
independent of fixture provisioning: `android-product-prepare-fixture.py ROOT
SECOND_UUID UNIQUE` may install the same disposable APK in the second guest.
It verifies device/generation on the exact smart-socket transport before sending
bytes and never starts or replaces ADB. Its result does not qualify the picker.

For coexistence with an independently owned compatible ADB server, start
`android-product-external-adb.mjs ROOT/product UNIQUE` before opening a phone.
It starts only the server entry point on private port 15047 and retains its own
child handle outside Tauri. After the app and phones have closed, verify that
this listener retains its PID, then create `ROOT/product/UNIQUE-stop` to let the
driver terminate and reap only its child. The fixture never uses port 5037 or
`kill-server`. This supplements, rather than replaces, the server-race tests.

On macOS, `android-window-count.c` uses Quartz's full window list to count only
the explicitly supplied application, emulator and ADB PIDs, including off-screen
windows. Compile it with ApplicationServices in the isolated trial directory.
Check a positive application count alongside zero emulator/ADB counts; do not
publish metadata about unrelated windows.

`node tests/native/android-product-second-instance.mjs ROOT/product UNIQUE`
starts one additional instance of the exact verified fixture binary while an
owned phone is running. It targets commands by process ID, expects the second
Android directory acquisition to fail, checks the first generation and closes
only the additional process. Both fixture builds must include process-targeted
instructions. This driver checks the real OS lock through native commands; it
does not claim to verify the second instance's rendered error presentation.

After CPU/RAM sampling, run a separate product GPU comparison with the current
resource report. The runner verifies process creation identities, traces the
same guest animation with and without the panel, and exports only owned GPU
interval summaries. Full Instruments traces stay in the isolated trial directory.

```sh
python3 tests/native/android-product-gpu.py ROOT --device UUID \
  --resources ROOT/product/UNIQUE-resources.json --name UNIQUE-GPU
```

Use `--views 2 --active-tab 'Android views'` for two visible panes, adding
`--second-device UUID` only for independent phones. The GPU budget is 0.10
active seconds per second per independent phone. The resource inventory must
belong to the current process generation: after Stop/Start, refresh the PID
and OS creation-identity inventory instead of silently reusing an old report.

For resource comparisons, arrange the real docked panes first, leaving a separate
terminal tab for the no-stream baseline. `android-product-perf.py --views 2`
records each visible Canvas and one shared source for a single device. The
report distinguishes uploaded texture/source dimensions from each framebuffer;
a secondary framebuffer may be smaller while sharing the full-size texture. Add
`--second-device UUID` to animate and account for both independent emulators.
`--active-tab TITLE --baseline-tab TITLE` select the prepared tabs. The driver
rejects an unlisted managed emulator instead of attributing its CPU/RAM to the
application. These short comparisons do not replace the thirty-minute trial.

For a supplementary memory comparison, `--fixed-layout-baseline` hides only
the phone viewport hosts using fixture CSS. Their real ResizeObserver cancels
the production sources and releases renderers; the adjacent terminal keeps
exactly the same dimensions. Restoring the styles recreates the live canvases.
This measures transport/rendering overhead with fixed terminal geometry and
does not replace the ordinary tab/workspace visibility tests. Start the driver
on the prepared active tab. It restores the styles when the measurement settles.

## Original stage-0 fixture

`run-android-smoke.mjs` is the separate feasibility prototype. Its small DOM
overlay is measurement instrumentation, independent of workspace lifecycle.
It uses the real emulator, system WKWebView and binary IPC, without an external
emulator window or screencap polling.

Prerequisites in a disposable directory (the recorded run used
`/tmp/simplebench-android-stage0-20260918`):

- An explicitly accepted SDK license, recorded in `evidence/consent.json` as
  `{ "accepted": true, ... }`. Only record a real, scoped acceptance after
  displaying the provider's conditions. The runner never accepts licenses.
- Independently provisioned SDK packages from
  the official repositories, with hashes checked against the selected catalog.
  For the original prototype, tools live under `sdk/`, the AVD named `sb_stage0` under `avd/`, and private
  Android homes under `user/` and `emulator-home/`. Use the documented 1080×1920
  Nexus 5 trial profile; this is a fixture, not a product device catalog.
- `logs/`, `evidence/`, `runtime/`; enough free space for SDK, AVD and builds.
- The normal repository development toolchain. No Android Studio is invoked.
  This probe runner uses Node as a development tool, not an application runtime.

The trial's packages were verified against the official repository XML before
extraction. CLI SDK management and AVD creation were separately exercised with
private Java. The original prototype does not exercise the production installer; use the
managed installer tests and integrated product fixture for those paths.

Run from the repository root:

```sh
node tests/native/run-android-smoke.mjs /tmp/simplebench-android-stage0-20260918
```

`--avd sb_stage0_small` selects a separately provisioned Small Phone AVD in
that same private directory. The probe discovers physical dimensions from
authenticated native status; it never resizes the guest with `wm size`.
`--reuse-binary` is for JS-only fixture changes: the runner verifies the previous
canonical root, repository, owned application identifier and executable SHA-256
before reusing it. Rust changes require a stopped application and a new build.

The runner builds a release executable with a unique application identifier,
creates fresh application data, then starts the native window. It verifies the
four fixed test ports are free before modifying the runtime files. Rust owns
the emulator and private ADB child handles; shutdown uses authenticated gRPC.
The runner registers a newly generated public JWK in the **owned** emulator's
per-PID discovery directory. The selected emulator puts this outside the test
root on macOS; see the report's isolation limitation. Tokens are never passed
to webview JavaScript. No global server on 5037 is used or stopped.
The current runner rotates the key for each emulator PID and renews its 120-second
JWT every 60 seconds. The ignored Rust credential test independently registers
Rust-generated ES256 public keys and checks real audience/expiry/renewal behavior.

The runner makes the fixture root and emulator home private (0700) and protects
any copied ADB key (0600). Android home variables do not isolate the selected
ADB's default key: the emulator can copy an existing user key into its own
home. The shared-server fixture requires that key to exist already and checks
that its fingerprint is unchanged; it never creates or replaces a user key.

To control a running probe, atomically replace `<root>/instruction.json` with
an object containing a **new unique** `id` and an `action`. Read
`<root>/evidence/<id>.json` for the result. Actions:

- `start`: start/join the one test instance and wait for authenticated readiness.
- `auth`, `rtc`: authentication negatives and real RTC service detection.
- `subscribe` with optional `width`/`height` (default 720×1280), `unsubscribe`.
- `benchmark` with `durationMs` and `activeTime: true`: FPS, binary IPC bytes,
  Canvas draw time and source-frame age. Hidden time does not count toward the
  requested visible duration; the extra wall-clock allowance is five minutes.
  `completed: false` is not a successful measurement. Frame age is **not**
  input-to-presentation latency.
- `renderer` with `decoder: "rgba" | "image" | "bitmap"` and
  `surface: "2d" | "software" | "bitmaprenderer" | "webgl" | "webgl-copy"`: select the raw or
  JPEG path and Canvas context, then reconnect. WebGL uses RGBA and one retained
  texture per view. `composited: true` adds a Canvas layer hint for comparisons.
  These options are measurement candidates, not qualified product fallbacks.
  `webgl-copy` uploads raw bytes once and samples the first Canvas in the second
  Canvas texture. Record its additional presentation cost separately.
- `native-text`: AppKit insertion/composition into the real WKWebView and the
  explicitly installed fixture IME. This is separate from the clipboard test.
- `rotate` with `quarterTurns: 0..3`: change the physical orientation through
  authenticated gRPC. Frame orientation metadata determines touch mapping.
- `native-pointer` with `phase: "down" | "drag" | "up"` and normalized Canvas
  coordinates `u`/`v`: send an AppKit mouse event into the owned native window.
  `focus-form` transfers DOM focus to an ordinary input and releases any gesture;
  `letterbox` creates square CSS bounds for the margin rejection check.
- `features`: renderer state and actual document visibility. A locked/occluded
  macOS session suspends RAF and eventually JS timers; it cannot qualify the
  visible performance run. The benchmark fails on a presentation timeout.
- `two-views`, `one-view`: share the same stream/channel across canvases.
- `fit` with `enabled: false`: one source pixel per physical display pixel;
  request the guest's full dimensions with `subscribe` to test true 1:1.
- `hide-views`, `show-views`: remove or show the overlay. Hiding also cancels
  the source subscription and releases Canvas buffers. Showing alone does not
  subscribe; use `subscribe` or `renderer` explicitly.
- `ack-timeout`: intentionally withhold ACK, reconnect, verify the old epoch
  cannot release the new frame, then resume with the correct ACK. It triggers
  a guest navigation event so an otherwise static screen produces a new frame.
- `input` with `input: {kind: "text" | "paste", text: "..."}`, or
  `{kind: "key", key: "GoHome"}`, or `{kind: "touch", x, y, pressure}`.
  Text/Paste are separate tests. An RPC success does not prove guest insertion.
- `screenshot`: one gRPC PNG in `evidence/screen.png`.
- `stop`: gRPC shutdown and wait for the owned emulator to exit; a timeout
  retains the handle for retry. `quit`: after Stop, dispose private ADB and the
  test window. Quit destroys the webview, so its JS report may not be delivered.

Ctrl+C asks the running probe to Stop and then Quit. A failed Stop leaves the
window/process handles available for diagnosis. Do not start another runner or
rebuild its executable while the previous native run remains open. The feature
has emergency Exit cleanup of its disposable child handles; it is not the
production editor/plugin/updater shutdown transaction.

The v9 data-retention trial found that gRPC `SHUTDOWN` alone can lose recent
unsynced guest writes. The ignored `native_guarded_shutdown` test exercises
`android::adb::Guest::request_shutdown`: same-transport identity guard, `sync`
and Android's `svc power shutdown`. An acknowledged ADB disconnect means only
that shutdown was requested. The process owner must still reap its `Child` and
confirm guest data after restart. Do not equate PID existence with liveness:
an exited child can remain a zombie until reaped. The current probe's `stop`
action still tests the emulator RPC; it is not the production Cold Stop policy.

For control when JavaScript is suspended, atomically replace
`<root>/native-control.json` with a new `id` and one of `stop`, `stop-and-quit`,
`reload`, `present`, `large-window`, `small-window`, or `process-info`. Rust
reports to `evidence/native-control.json` without using the webview event loop.
Optional `application` PID and process `generation` must match; stale cleanup
requests cannot stop a new probe instance. Native Stop while the session was
locked passed. `present` preserves the current dimensions. `process-info`
uses read-only WKWebView diagnostics to attribute WebContent/GPU/Networking
PIDs; these private APIs are confined to this test feature.

For a reproducible macOS ARM64 resource measurement, keep the native window
visible and run the developer-only harness from the repository root:

```sh
python3 tests/native/android-perf.py /tmp/simplebench-android-stage0-20260918 \
  --name unique-rgba-run --duration 1800 --decoder rgba --surface 2d
```

Add `--helpers PID...` for system helpers whose ownership was observed at
probe startup. The harness obtains other PIDs from the actual native view,
checks each process start identity, and compiles `android-memory.c` inside the
disposable directory. It records physical footprint as well as RSS and CPU;
RSS alone misses compressed WebContent memory. It settles the animated guest
without a stream for 60 seconds, samples a 60-second baseline, measures the
requested visible duration, and samples another 60 seconds without a stream.
The latter remains an animated guest baseline, not an idle-phone measurement.
Earlier runs have their actual shorter intervals recorded in their evidence.
Use `--views 2` to measure two shared views; `--width`/`--height` select source
dimensions. `--one-to-one` disables CSS Fit but does not itself change the source
resolution. A full-size check must explicitly request the guest's dimensions.

The transient `caffeinate` assertion is tied to the probe and removed on exit.
Cancellation or a failed measurement requests native Stop/Quit for the recorded
application/generation; a stop failure is saved for explicit retry. Full resource
samples checkpoint every ten seconds under `evidence/`. Use unique run names:
the harness preserves previous evidence. Avoid simultaneous builds or other
measurements that introduce memory pressure. Record any tracing intervals
separately and disclose them when interpreting the full run.

`android-gpu.py INTERVALS_XML TOC_XML RESOURCES_JSON OUTPUT_JSON` summarizes
an Instruments `metal-gpu-intervals` export using the measured process groups.
It merges overlapping active intervals per group and uses the duration from
the trace table of contents. It excludes other processes and non-active states.
Compare a stream trace with a separate animated no-stream trace; the sum of
group deltas is a conservative interval metric, not GPU core utilization.
Keep complete traces outside the repository because they contain metadata
about unrelated applications.

The saved performance run used a developer-only raw ADB smart-socket driver
on 15037 to alternate fixed 1000 ms vertical swipes in Android Settings. It
sampled `ps` cumulative CPU time and RSS every 0.5 s for the explicitly listed
application, WebKit, emulator and helper PIDs. This avoids using a normal ADB
client against a potentially replaced/shared server. Full telemetry samples
and screenshots remain in the disposable directory; compact results without
credentials or user data belong in the external trial archive, not source control.

The subsequent five-minute JPEG run includes **physical footprint**, since RSS
alone hides compressed WebContent memory. It passed its short-run budgets;
that historical JPEG trial did not complete thirty visible minutes. See the architecture guide for current transport budgets; this historical
variant is not the production transport.

The dangerous behavior of a normal ADB client is reproduced **only against a
private fake server** by:

```sh
node tests/native/android-adb-race.mjs /tmp/simplebench-android-stage0-20260918
```

This test deliberately changes the server version after preflight, records
`host:kill`, refuses the request, and bounds execution. Exit 0 means the unsafe
client behavior was reproduced; it does not qualify a production ADB path.

The selected emulator's negative case, with a real independently owned ADB
server and a proxy changing its reported version after preflight, is:

```sh
node tests/native/android-emulator-adb-race.mjs /tmp/simplebench-android-stage0-20260918
```

It checks the actual guest connection, authenticated gRPC, absence of
`host:kill`, unchanged server identity and continued server availability after
the emulator exits. Only then does the fixture stop its own test ADB server.
Both scripts use private ports; neither calls or kills the server on 5037.

Rust boundary tests:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --features android-probe android_probe::tests
cargo test --manifest-path src-tauri/Cargo.toml --locked --features android-probe android::
```

The historical fixture IME v2 passed direct Polish text and Japanese composition
using native AppKit input, plus an independent UTF-8 Paste test. The new v3
artifact has separately passed both the Rust/gRPC test and the product AppKit
trial, including the actual native host clipboard and guest Paste action.
Physical keyboard layout switching remains outside the recorded qualification.
Two independent devices passed focus-isolated input; simultaneous input to two
phones is intentionally prohibited. Product focus, rotation, minimization and
the clean Settings installer have native qualification summarized in
[the architecture guide](../../docs/android-architecture.md#native-qualification).
Rust foundations separately test interprocess locking, immutable used images,
interrupted package publication, corrupt metadata preservation and cancellation
of actual child processes. Explicitly ignored tests run the real provider
catalogs, archives, download and owned guest fixture; their required environment
is `SIMPLEBENCH_ANDROID_PROBE_DIRECTORY`. These are complementary checks, not
proof that the Android product or another host platform is ready.

The additional ignored test
`android::artifact::tests::actual_cli_installation_is_verified_before_atomic_publication`
downloads Platform Tools with the Rust downloader, invokes the fingerprinted
Android CLI directly with `--no-metrics`, checks the installed bytes and exact
accepted license against the archive/catalog, publishes the package through
the durable journal, and runs the resulting `adb version`. It uses a temporary
managed directory under the accepted fixture and removes it after completion.
It neither starts an ADB server nor changes the user's SDK. This is a native
component integration test, separate from the clean product installer UI trial.

Probe frame protocol v2 includes an encoding tag (0 = RGBA, 1 = JPEG) and the
payload length in its 60-byte header.
Packets are padded to at least 1024 bytes because Tauri 2.11 converts smaller
Raw messages to JSON arrays. The Canvas decoder reads only the declared
payload, and a 1×1 JPEG boundary test checks the padding. Historical performance
reports used v1; the large-window renderer trials used v2. Prototype frame v3
keeps the same header size and replaces its redundant source-sequence field
with validated orientation metadata. The production frame protocol is v4 and
adds process identity independently of the subscription epoch. Historical
prototype rendering measurements do not qualify the current product binary.

After explicitly provisioning the fixture APKs, run
`python3 tests/native/android-interaction.py ROOT --name UNIQUE` for native
mouse events at four rotations, drag/release, margin rejection, direct Polish
text, Japanese composition, separate UTF-8 Paste and touch-to-presentation
latency. The script controls only the isolated fixture and writes each native
response as evidence. A failure leaves the owned process available for diagnosis;
it does not imply those behaviors have been qualified.

For larger phone profiles, `android-product-zoom.mjs PRODUCT DEVICE_ID UNIQUE`
checks actual native pointer input at Fit/100%/200%/300%, scroll offsets and
four rotations against coordinates reported by the guest test APK. It asserts
the source/framebuffer pixel budget and preserves the process generation.
The interaction, latency and text drivers also accept portrait previews of
larger hardware; coordinate checks use the actual guest display dimensions.
Resource and GPU drivers scale their guest swipes to the actual owned AVD's
display; keep the same viewport and animation for baseline and active samples.

`android-product-view-lifecycle.mjs PRODUCT DEVICE_ID UNIQUE` expects the
isolated fixture's two workspaces and two docked views in `Android views`.
It checks independent zoom/scroll retention, one source after switching back,
source cancellation on native minimization and resumption without restarting
the phone. It does not run concurrently with resource or GPU measurements.

`android-product-last-view.mjs PRODUCT DEVICE_ID UNIQUE` uses those two
references to the default phone. It writes a fixed guest marker, closes each
panel through the UI, checks that only the final close stops the process, and
reopens through the plus menu. A new generation must retain the marker. Run
this after GPU/resource tests, since it intentionally replaces the process.

`android-product-install.mjs PRODUCT UNIQUE image PACKAGE_ID` can select a
specific stable ARM64 image through the real Settings UI. Additional ARM image
terms must have actual user consent recorded outside Git in
`ROOT/modern/image-consent.json` (`accepted`, `licenseId`, `sha256`). The driver
compares the exact displayed text before accepting; do not create this record
without a corresponding user decision. A locked macOS session or a missing
license decision blocks dependent native tests, not the independent checks.
