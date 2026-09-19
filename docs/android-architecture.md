# Local Android architecture and qualification

The user workflow is described in [Android phones](android.md). The native
fixture guide is [tests/native/ANDROID.md](../tests/native/ANDROID.md). Historical
planning documents and raw trial reports are kept outside the source repository.
This guide records the maintained contract and the scope of completed tests.

## Ownership and persistence

Android is a built-in tab/pane with a device UUID and no shell or PTY. Device
identity, domain view identity and process generation are distinct. The domain
layout owns references; Dockview and React mounts do not own the emulator.
`src/android/service.ts` loads the retained runtime lazily. AndroidPane and
AndroidSettingsPage are separate lazy entry points. An unused Android backend
starts no processes, timers or SDK scans.

Rust owns the SDK, private Java, AVDs, ports, credentials and process handles.
One interprocess directory lock protects metadata, installation and runtime.
Preferences and devices use separate validated, revisioned, versioned files,
atomic publication and explicit recovery of preserved corrupt data. Session
files contain descriptors, never live PIDs, ports, credentials or pixel buffers.
Restored phones start only when visited; missing devices remain repairable.

One actor per device coalesces Start and retains its Child until confirmed exit.
Boot preparation is queued. A failed or timed-out Stop retains ownership and
blocks another Start. Docking and workspace changes preserve the process.
Closing the final view runs existing guards, stops the phone, then changes the
current domain; it never restores an obsolete session snapshot.

Window close, updater installation and plugin restart share
`src/application-close.ts`: freeze new Android starts/mutations, run existing
guards, save the current session, settle installation, stop phones, then exit.
Cancellation before Stop leaves phones running; cancellation after a partial
Stop preserves descriptors and stopped devices without restarting them.
Every path keeping the application open releases the preparation barrier only
after operations settle. Settings closure does not cancel an accepted install.

## Toolchain and installation

The last official catalog check was on 2026-09-19. The qualified macOS ARM64
combination uses Android Emulator 37.1.11 (build 15917651), Platform Tools 37.0.1,
Command-Line Tools 23.0, direct Android CLI 1.0.16261425 and private Eclipse
Temurin JRE 21.0.12.1+1. `src-tauri/android-toolchain.json` pins bootstrap sizes,
SHA-256 hashes, URLs and per-host qualification. A mutable download URL does
not permit different bytes; a mismatch requires an explicit toolchain update.

The installer reads Google's official repository catalogs, shows provider terms
before consent, verifies downloads and installed artifacts, bounds extraction,
and journals directory/manifest publication. Cancellation leaves recoverable
owned staging; one previous working tool copy is preserved. Images referenced
by any device are immutable, including while stopped. Revision conflicts are
explicit; recording a revision in JSON does not pin mutable files. Maintenance
bounds logs/cache/staging without deleting AVD data or the CLI's cache lock inode.

The CLI uses `--no-metrics`. Android SDK/user/emulator/AVD homes, Java user.home
and the ADB endpoint are isolated per managed directory. No global PATH,
JAVA_HOME, shell profile, .androidrc, external SDK or external AVD is modified.
Device creation invokes private Java's AvdManagerCli. Host virtualization
preparation can require an administrator; the application reports requirements
without promising to change BIOS, KVM permissions or Windows features itself.

Profiles come from the installed SDK; images come from compatible stable
catalog entries. Minor APIs, including decimal installed ApiLevel values, sort
numerically and must agree with separate minor metadata. Multiple tags preserve
Google/AOSP variants and 16 KB page requirements. Foldable, resizable and tablet
profiles remain excluded. A profile describes virtual hardware, not a physical
phone's complete components or proprietary manufacturer software.

## Transport and input

The checked-in [protobuf provenance](../src-tauri/android-proto/README.md)
identifies the exact protocol from Emulator 37.1.11. Rust uses local authenticated
gRPC with per-instance ES256 credentials and exact RPC audiences; secrets never
reach webviews. The selected emulator's Tink validator rejects the `typ: JWT`
header. The tested binary returned UNIMPLEMENTED for the experimental RTC
service, so the application uses no assumed WebRTC backend or external bridge.

One screenshot source and one binary Tauri Channel per device serve all visible
views. Frame v4 contains process UUID, independent subscription epoch, sequence,
dimensions and rotation. The queue retains one unacknowledged IPC frame and one
latest native frame; old frames are discarded. A two-second missing ACK cancels
the source. Stale ACK/unsubscribe cannot affect a new epoch. Hidden views and
native main-window minimization cancel transfer at the source without stopping
guest apps. GPU resources belong only to visible canvases, outside React state.

Source limits are 921,600 RGBA pixels, 1280 on either edge and 30 FPS. Hardware
LCD dimensions are separate: at most 4096 per edge and 8,294,400 total pixels.
The largest visible canvas retains source resolution; smaller secondary
framebuffers may match their physical viewport. Fit and 25–300% zoom/scroll are
retained per domain view. Detached or zero-sized hosts cannot overwrite saved
scroll positions during cleanup. 100% means preview pixels; actual-size wording
is offered only when the whole hardware display fits the source budget.
Explicit PNG screenshots use a separate bounded full-resolution response.

One ordered native focus lease controls keys, touches and composition across
all devices. Blur releases held input. Application shortcuts, ordinary form
fields and dialogs keep their roles. Direct Unicode/composition use the
explicitly disclosed [bundled IME](../src-tauri/android-input/README.md), which
has no network permission. Intentional Paste transfers UTF-8, sets the guest
clipboard and invokes Paste in order; typing never changes the clipboard.
The native APK and screenshot actions use system file dialogs. End users do not
need Android build tools to build the embedded, integrity-checked IME.

## Native boundaries and shutdown

The global trusted-app caller guard excludes browser child webviews. Commands
extract Window because one window can contain several webviews. Settings owns
management; main owns runtime/input and workspace mutations. Both trusted views
may read state and Stop. Rust resolves deviceId to owned paths/processes; no
arbitrary command, PID, AVD path or gRPC address is accepted from frontend code.
Settings sends expiring, acknowledged open intents to main and cannot recreate
a closed setup target in another workspace.

ADB operations use a bounded native smart-socket connection with device and
private generation checks on that same transport. An ordinary ADB client can
replace an incompatible server after preflight, so banning `kill-server` alone
is insufficient. The backend never replaces or kills a shared server. Cold Stop
performs guarded sync and Android shutdown before reaping; gRPC shutdown alone
lost unsynced guest data in the initial trial. Recovery checks executable,
creation/boot identity, owned AVD and authenticated instance, never PID alone.

Windows requires separate qualification of launcher and QEMU child identities:
the Windows launcher can spawn QEMU while Unix preserves PID through exec.
Process-tree ownership, discovery, recovery and cleanup must all be verified
before enabling that host. No distribution currently claims Windows/Linux or
Intel macOS Android support; their installation/create/wipe/open/Start gates
remain closed. Read, Stop and recovery stay available.

## Native qualification

The [Windows preflight on 2026-09-19](android-windows-qualification.md) is NO-GO:
the Windows 10 x64 host reports unavailable WHPX. It records independent
regression checks and portability corrections, not emulator qualification.
Windows remains gated until native lifecycle, image, input and resource tests
pass on an accelerated host.

Results below were obtained on Apple M3 / macOS 27.0 ARM64, Host GPU, cold boot.
They are not evidence for other hosts, all catalog images, or Quick Boot.

- AOSP API 36 ARM64 r2, Small Phone 720×1280: integrated 30-minute trial,
  two shared views, 28.03 FPS per view, +0.2993 CPU cores, 103.32 MB/s IPC,
  +0.01561 active GPU seconds/second.
- Google Play API 37.2 ARM64 r5, Android 17, Pixel 10 Pro XL 1344×2992,
  480 DPI, 2560 MiB guest RAM, two CPU cores, 4 GiB data, 16 KB pages:
  real Settings download/create, APK installation, full-resolution PNG,
  Polish text, Japanese composition, explicit UTF-8/emoji Paste, four
  rotations and 16 Fit/zoom cases. Maximum measured mapping error: one guest
  pixel. Native input-to-presented-image latency: p95 114 ms over 50 events.
- The modern guest completed 1800.005 visible seconds with two shared views
  at 24.13 FPS each, +0.3232 CPU cores, 70.52 MB/s IPC and +0.03237 GPU active
  s/s. This preceded a viewport cleanup correction; it is not a thirty-minute
  measurement of the later executable.
- After that correction, a 120.097-second two-view regression achieved
  28.44 FPS each, +0.4661 CPU cores, +62.59 MiB application footprint,
  83.40 MB/s IPC and +0.02910 GPU active s/s. Source: 574×1277.
  The independent zoom/scroll retention, smaller framebuffer input, workspace,
  minimize/resume, lazy session restoration and persistent data tests passed.
- Android 17 plus an independent AOSP phone: 60.082 seconds at 28.36/28.26 FPS,
  +0.5851 CPU cores, 187.02 MB/s IPC and +0.03646 GPU active s/s. Input stayed
  with the active device. Both processes stopped on application close; the
  independently owned ADB server survived until its own driver stopped it.

All identified application/webview/GPU/networking/PTY processes and the guest,
helpers and ADB were included. CPU deltas conservatively sum nonnegative group
deltas. GPU active-interval deltas are not percentages of GPU core utilization.
The modern guest/helper mean physical footprint was about 7–8.3 GiB, including
emulator/graphics allocations, not just configured guest RAM. Different warmed
terminal/WebKit baselines can produce negative net application memory deltas;
those are not claimed as negative intrinsic transport costs.

One-phone budgets are +0.5 CPU cores, +128 MiB application/webview memory,
111 MB/s IPC and 24–30 FPS under animation. The recorded Mac GPU budget is
+0.10 active s/s per independent phone; another host needs a defined comparable
metric. Two views share one phone's transport budget. Thirty minutes must not
show sustained memory growth; p95 input-to-image target is 150 ms. Record
source/framebuffer/CSS/DPR separately and stop builds during measurements.

Actual product trials also covered ACK timeout/reconnect/stale ACK, interprocess
locking, native docking, last-view Stop/reopen with data, fresh installation,
installer interruption, shared-server ADB races and close/plugin-restart paths.
The final source baseline passed 404 UI tests with mocked commands, 121 model
and 17 AI runtime tests, 172 Rust tests (16 opt-in tests ignored in the ordinary
suite), TypeScript, formatting, Cargo check/clippy/fmt and frontend/desktop
builds. Native fixtures provide separate evidence; mocks do not prove a host.
Local macOS app/DMG packaging was verified with ad hoc signatures only, without
Developer ID, notarization or publication.

Historical modern native binary SHA-256 after the viewport fix:
`88f60d19b76c757f13a83c5e4865982afc009e8df8e029924986dcc6e8e80ee7`.
These measurements predate repository-only relocation of the IME sources and
comment/documentation cleanup; they are not a new native trial of that build.
