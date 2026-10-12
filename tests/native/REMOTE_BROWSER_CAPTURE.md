# Ordinary Browser native capture probe

This opt-in fixture qualifies capture experiments on macOS ARM64. It is not
production Remote admission and does not enable BrowserObserve, Agent control,
MCP access, grants, artifact storage, or browser-page application commands.
`remote-visual-probe` is rejected when debug assertions are disabled.

Run from the `lomi` repository root:

```sh
cargo check --manifest-path src-tauri/Cargo.toml --locked --features remote-visual-probe
cargo test --manifest-path src-tauri/Cargo.toml --locked --features remote-visual-probe --lib browser::remote_probe::tests
node tests/native/run-remote-browser-capture.mjs
```

The runner starts one loopback fixture server and a Tauri development process
with a unique application identifier and application-data directory. It writes
the exact evidence directory and isolated application-data path to stdout. Its
controlled main renderer has no product BrowserRuntime; the native harness
creates an ordinary `Page` through the existing `sync_browsers` factory. It then
captures that same native child WKWebView, with `control=None`, throughout the
matrix. Native fixture evaluation reads clocks and separately changes a DOM
color. It does not use privileged page IPC or drive the autonomous RAF loop.
All pages, cookies, input values, screenshots and clocks are synthetic fixture
data. This runner does not capture human browsing or the user's application
windows. Cleanup stops its own process group and removes only its generated
application-data directory after a result is available; evidence is retained.

The fixture tests native retained-hidden transitions for inactive tab/workspace
cases. It does not exercise actual product tab or workspace switching, focus
handoff, overlays or BrowserRuntime ownership. The controlled main renderer
prevents frontend visibility changes from silently invalidating the active
baseline. Each named visibility case checks its actual native state.

The adapter uses global capacity two and per-WebView capacity one. A timeout
receiver cannot release the slot while WebKit still owns the completion block.
The worker also retains the slot through decode, bounded JPEG encoding and
publication checks. TIFF bytes are limited to 8 MiB, decoder allocation to
16 MiB, the longest pixel edge to 1280, total pixels to 921600, and JPEG bytes to
512 KiB. TIFF extraction and copying occur in the native callback; TIFF decode,
alpha compositing and JPEG quality 70 encoding occur on a blocking worker.
Identity, document/content epoch, geometry epoch, native URL, bounds and zoom
checks reject stale results. Geometry epochs also fence resize/hide/show cycles
that return to the same bounds.

The fixture includes timer-driven DOM, a native-controlled DOM color,
RAF-driven DOM color, canvas 2D and WebGL clear colors. WebGL uses
`preserveDrawingBuffer=true`; default non-preserved WebGL buffers are not
qualified. Pixel colors are checked against native fixture clock reads before
and after capture. Autonomous qualification additionally requires RAF counter
advancement and changed pixels across frames. A static correct image cannot
pass as a running animation.

The recorded macOS 27.0.1 ARM64 trial establishes these prototype results:

| Native source configuration                             | Timer DOM           | Autonomous RAF DOM/canvas 2D/WebGL                                                 |
| ------------------------------------------------------- | ------------------- | ---------------------------------------------------------------------------------- |
| Active visible ordinary Page                            | Fresh               | Fresh, advancing pixels                                                            |
| Hidden native view / retained inactive tab or workspace | Fresh               | RAF freezes; repeated pixels                                                       |
| Original main window hidden or minimized                | Fresh               | RAF freezes; repeated pixels                                                       |
| Same WKWebView in non-key offscreen host                | Measured separately | RAF freezes; host is not occlusion-visible                                         |
| Same WKWebView in transparent on-screen host            | Measured separately | Fresh, advancing pixels while original main window is visible, hidden or minimized |

The transparent host is a fixture-owned borderless NSWindow with alpha zero,
`ignoresMouseEvents=true`, and no ability to become the key or main window. It
uses public `orderFront`; the offscreen host uses public `orderBack`. Neither
uses `makeKeyAndOrderFront`, a replacement WKWebView, private visibility APIs,
or a screen-capture permission. The source is retained and reparented with the
same CSS dimensions. The fixture restores its original native parent, frame
and hidden flag, and closes the owned host window. This is a measured candidate
for a retained render host, not an adopted product architecture.

Read `result.json` before interpreting the runner's exit status. **Exit 1 is
expected when direct background RAF qualification fails even if the transparent
host candidate passes.** `stage: "completed"` means the native matrix completed;
`backgroundContentCandidate: false` describes the direct retained-view failure.
`retainedHostExperiments[].candidate` describes each separate host experiment.
A completed matrix with a failed qualification is not a native crash.
`stage: "failed"` and its `error` describe a fixture failure.

The report distinguishes observations from open qualification:

| Field or evidence                                            | Meaning and limit                                                                                       |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| `cases[].visibilityMatchesCase`                              | Active/hidden/registered bounds and window hide/minimize matched the named native case                  |
| `freshPixels`, `clockFreshPixels`, `autonomousClockAdvanced` | Ordinary DOM pixels matched the stimulus and autonomous timer continued                                 |
| `contentQualification`                                       | Per-surface pixel match, actual changed pixels, and autonomous RAF advancement; read all fields         |
| `metrics.visibility`                                         | Captured source window's hidden, visible, minimized, occlusion-visible, key/main and mouse-ignore flags |
| `originalParentState` / `parentTransitionObserved`           | Original main window's visible/minimized state was checked independently of the host window             |
| `sourceStatePreserved`                                       | Cookie, URL, input draft, JS counter, CSS viewport and DPR matched before/after reparent                |
| `keyAndMainWindowsPreservedAtReparent`                       | Key/main window IDs matched at reparent; this does not qualify first responder or DOM editing focus     |
| `restore`                                                    | Original parent pointer, native frame and hidden flag matched after host teardown                       |
| `captureMillis` / `decodeEncodeMillis`                       | Native snapshot/TIFF transfer and worker decode/JPEG elapsed time, respectively                         |
| `closeQualification`                                         | Explicitly inconclusive: no native submission/identity-removal barrier proves callback ordering         |

First responder, DOM editing focus, selection, IME and editing undo history
remain unqualified. WebKit's reparent path may clear edit commands; preserved
draft text alone is insufficient. End-to-end hit testing is unqualified despite
the recorded mouse-ignore and key/main flags. Actual product UI transitions,
split source identity, navigation/zoom/close callback races, permission dialogs,
menus/drag, app hide/deactivate, lock/sleep, Spaces, fullscreen and multi-display
changes remain open. Video, DRM and cross-origin iframe content are not tested.
The source/host ownership, sharing/revoke fences and complete grant/relay/Worker
pipeline are not enabled by this probe.

Measurements use the unoptimized development build. Recorded worker timing is
not a qualified 5 or 10 FPS rate or a production CPU/GUI/memory budget. Thirty
minute and two hour lifecycle/idle/leak trials remain open. **G1 remains open and
full production Browser background sharing is NO-GO. No limited beta or
production capability is enabled.**

The final machine-readable evidence is
`remote-system-plan/implementation/browser-20261008.json` in the parent project,
with exact native results, source/artifact hashes and copied fixture artifacts
under `implementation/browser-capture-20261008/`. Earlier startup trials with an
already-hidden “active” baseline were withdrawn and are excluded from that
final qualification report.

Relevant primary sources:

- [Apple NSWindow orderFront](<https://developer.apple.com/documentation/appkit/nswindow/orderfront(_:)>) preserves key/main ordering.
- [Apple occlusion visibility](https://developer.apple.com/documentation/appkit/nswindow/occlusionstate-swift.struct/visible) permits transparent windows to be considered visible; it does not guarantee freshness.
- [WebKit native visibility predicate](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/mac/PageClientImplMac.mm#L207) includes native view, window and occlusion visibility.
- [WebKit reparent editing behavior](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/mac/PageClientImplMac.mm#L263) clears edit commands when moving to another window.
