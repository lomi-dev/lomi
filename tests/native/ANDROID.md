# Android native verification

The production backend and lazy Android UI are included in normal builds. The
additional commands and drivers in this directory require `android-probe` and
must never be enabled in distributed builds. Native qualification covers the
recorded Apple M3/macOS ARM64 and Windows 11 Pro/x64 hosts described below.
Linux and other Mac architectures remain unverified.

## Linux prerequisites and pending qualification

The recorded Linux preflight host is Ubuntu 26.04.1 x86_64, kernel 7,
Mesa 26.0.8, GNOME Wayland and an NVIDIA RTX 3060 Ti bound to Nouveau.
The caller can read/write `/dev/kvm`, and `KVM_GET_API_VERSION` returns 12.
These are prerequisites, not native Android qualification. Linux's production
toolchain qualification flag remains **false**: comparable GPU evidence remains
unresolved. The observed thirty-minute memory requirement was accepted with the
finite-duration caveats recorded below.
Individual functional, input, recovery and resource trials are recorded below.
A successful application startup or host preflight does not change that status.

The application now starts on this host. The source startup policy in
`src-tauri/src/linux_graphics.rs` applies a process-local
`LIBGL_KOPPER_DISABLE=true` default only for a Wayland-capable backend with an
actual Nouveau-bound DRM render node. It selects Mesa's DRI presentation path
while preserving WebKit DMA-BUF and GPU rendering. Existing user values,
including empty values, take precedence; an explicit X11-only backend does not
receive this default. The existing proprietary-NVIDIA Wayland
`__NV_DISABLE_EXPLICIT_SYNC` policy remains separate and preserves user
overrides. No global environment, shell profile, driver or desktop-session
change is required by this source policy. Startup success does not establish
frame rate, input delivery or emulator GPU-budget compliance.

The NVIDIA Wayland startup policy also defaults
`WEBKIT_FORCE_DMABUF_RENDERER=1`, unless the caller supplied any value. Ubuntu's
WebKitGTK 2.52.6 package includes `disable-nvidia-dmabuf.patch`: its NVIDIA
blacklist otherwise disables accelerated compositing before checking GTK GL
support. An ordinary native profile with matching Ubuntu debug symbols confirmed
both WebGL and Skia GPU-to-CPU readbacks in this fallback. The package's explicit
opt-in restores its normal EGL/GTK/DMA-BUF capability checks; the existing
explicit-sync policy remains independent. An explicit X11-only backend does not
receive this new default. Five native startup-policy tests passed. Performance
results below predate this new default unless explicitly identified otherwise;
the new candidate still requires native qualification.

For the same loaded-NVIDIA-module Wayland candidate, startup now also defaults
`WEBKIT_SKIA_ENABLE_CPU_RENDERING=1` and `WEBKIT_DISABLE_COMPOSITING_MODE=1` before
GUI threads start. These are process-wide WebKit page-rendering/compositing
defaults, not Android-only settings or a claim that all GPU work is disabled.
Each preserves every existing caller value, including empty, `0` and non-Unicode
values; explicit X11-only selection and an absent/empty `WAYLAND_DISPLAY` receive
neither default. The DMA-BUF opt-in, Nouveau Kopper and explicit-sync policies
remain independent. No public WebView policy command is required at startup.
Linux qualification remains false. The integrated build07 long-duration CPU,
frame-rate and memory-budget results below are complete, and the unhooked
thirty-minute memory observation was accepted with caveats. Comparable GPU
evidence remains unresolved.

Linux's built-in Lomi theme can now give the actual WebView an opaque
background when its shell covers the viewport with opaque color, square
corners and no uncertain transparency effects. Custom themes keep the
transparent WebView. Native theme synchronization clears that background
before replacement or restored preview styles become active; subsequent
updates share the theme queue and reject stale generations. This changes the
WebView background, not the native window's transparent startup behavior.
The native Linux check observed the production opaque/transparent round trip
without forcing opacity. Theme and appearance checks passed in Chromium and
WebKit, including preview cancellation racing another preview.

Native Linux x86_64 now explicitly requests lossless RLE32, encoding 2 in the
existing v4 packet with its 68-byte header. Little-endian u32 tokens use the high
bit for repeats and the lower 31 bits for pixel count: repeats of at least four
equal pixels carry one RGBA word; literal tokens carry their RGBA words. The
payload must be at most 7/8 of raw RGBA and produce a smaller wire packet, or
encoding 0 sends raw pixels. Generation, epoch, token and output bounds,
truncation, cancellation and packet/output aliasing checks protect reusable
stream-owned scratch storage. The Linux primary uses Canvas2D putImageData with
a clamped view of frame pixels; secondary views drawImage from the full-source
primary canvas. Other hosts retain WebGL.

The integrated production-renderer build07 completed a 1800.003-second isolated
debug trial with two views sharing one 720×1280 source. All 155 product and three
fixture source pins matched. The product manifest was
`b7d6516deaae129bcf7b36eccf2a9a669949c593dcd0dfdd1a50d7a34fd9239c`;
the 339,688,520-byte binary was
`5bbc5ceebb525c65a30a1bdce289a37b7d850f02a1c4aeeccc46c44a5d2499aa`,
and the executed build receipt was
`965fe536ed885c81f799d0fa821f3c80b357c2802aeea34ede45f49e69456631`.
Each view recorded 53,721 frames at 29.8449502584 FPS; the shared source recorded
110.022454089 logical MB/s. Visible-minus-hidden CPU was +0.4894263594 cores
combined (+0.551621 application, −0.062195 guest), below the +0.5-core budget. Application private
resident memory averaged 17,850,618 bytes above baseline; its peak was
25,549,718 bytes above the baseline mean, below 128 MiB. The separate pretrial
native-input check recorded 50 samples with p95 and maximum both 138 ms.

This is not a no-growth result. First-to-last 60-second memory means increased
10.1337 MB (native application +0.333 MB, WebKit +9.799 MB); the last 600 seconds
had a +0.324594 MiB/min slope. After hiding, the mean remained 3.349 MB above
hidden baseline. Instrumentation retained arrays containing 214,882 numbers;
no memory has been subtracted for that retention. Raw evidence is retained outside Git under
`production-1800-07`, with growth analysis under `memory-analysis-07`.
Comparable GPU evidence remains unresolved and Linux stays `qualified: false`.

The separate passive follow-up completed 600.004 seconds, from 08:35:16.673 to
08:45:16.676 UTC, without measurement hooks, per-frame timing arrays or measured
pixel reads (zero). Its 768 endpoint pixel reads occurred outside the measured
phase. It recorded 17,932 native packets; that is not draw-FPS qualification.
The independently reviewed offline analysis accepted the evidence. Application
first-to-last 60-second private-resident means rose 3,040,636 bytes: native
−441,482 bytes, WebKit +3,481,927 bytes and network +192 bytes. Full-window OLS
slopes were −0.00259659 MiB/min native, +0.13851594 WebKit and +0.13593543
application; the application's last-300-second slope was +0.25693428 MiB/min.
Minute application means in MB were 372.051, 374.111, 375.301, 375.371, 375.506,
374.138, 373.761, 374.507, 374.902 and 375.092. Periodic variation does not prove
formal 30-minute no-growth. The hidden last-15-second mean stayed 33,176,248
bytes above its own baseline. This used the new post-recovery application and
must not be conflated with the older trial's 3.349 MB hidden delta. Retained
timing arrays were therefore not the only possible cause; that shorter trial
did not confirm thirty-minute no-growth. Raw evidence and accepted analysis are retained outside Git under
`passive-memory-02-execution`. No GPU verdict or platform qualification follows.

The next unhooked follow-up completed 1800.004 seconds on 2026-10-20, from
09:00:54.544 to 09:30:54.548 UTC. The accepted, independently reviewed offline
analysis and raw evidence are retained under `passive-memory-03-execution`.
There were no measurement hooks or timing arrays and zero window pixel reads;
768 endpoint reads remained outside the window. Its 53,824 native packets are
not a draw-FPS qualification. All 155 product and three fixture source pins,
the retained `5bbc5cee` debug binary and `965fe536` build receipt matched in the
driver and latency checks before/after.

Application private-resident memory averaged 373,802,523 bytes against a
346,912,344-byte baseline and peaked at 382,017,536 bytes: mean/peak deltas were
26,890,179/35,105,192 bytes. First-to-last 60-second means rose 5,898,381 bytes
(native +4,309,981; WebKit +1,588,118). The full-window application OLS slope was
+0.13942899 MiB/min, with observed early and within-run rises. Late
observations declined: final-600-second slopes were −0.19119528 MiB/min
application, −0.00066255 native and −0.1906431 WebKit; the final-300-second
application slope was −0.54876221 MiB/min. Minutes 29/30 averaged
373.436/373.451 MB after middle/late values around 375–376 MB. Repeated WebKit
falls in minutes 16/29 have no established GC attribution. The hidden final
15-second application mean was 4,752,466 bytes above its own baseline and
21,785,871 bytes below the visible final-minute mean. No guessed observer bytes
were subtracted. The review accepted the observed thirty-minute requirement:
“No sustained upward memory trend was observed through the end of this
thirty-minute workload.” Minutes 20–28 stayed near 358 MiB before the final two
fell near 356.14 MiB, with native memory flat in the final 600 seconds. These
finite observations establish neither the cause of the early rise nor actual GC
events. The native hidden-after mean remained 8,166,991 bytes (about 7.79 MiB)
above baseline, which could be live objects or allocator retention. There is no
lifetime or repeated-cycle memory bound. Comparable GPU evidence alone remains
the qualification blocker, and Linux stays `qualified: false`.

The post-trial native-input check passed 50 samples, p95 138 ms and maximum
139 ms; `latency-post-memory-03-latency.json` is archived with the trial.
Subsequent normal Stop/Quit reaped the application, guest, both WebKit processes
and its owned private ADB; runtime records were absent and port 15047 closed.
This owned ADB exited normally, without a manual ADB signal. The quiet/awake
leases were released, zero processes remained paused and the inhibitor child
was reaped. These cleanup results do not change qualification or imply that an
owner exception was accepted.

The normal macOS default release build also passed with empty Cargo features
(76,506,336-byte binary, SHA256 prefix `9794c239`); this is build evidence, not new
runtime qualification. Its receipt is retained outside Git as
`normal-macos-release-07.json`. Recovery08's automatic driver stopped on a
transient process-discovery EACCES after relaunch; its failed receipt remains
unchanged (SHA256 `604d2e375d4fc10670900f8d6455b85eb288cd61064b18ba4aa494ff3d8a8970`).
A separately archived manual continuation confirmed Stop and old app/guest
reaping, a fresh guest parented by the replacement application, the retained
marker and a resumed 720×1280 source with 720×1280/321×571 views. Its cold-start
readiness receipt failed; the later resize/source-provenance receipt succeeded,
and these outcomes remain distinct. Independent read-only discovery found one
managed application and one QEMU, verified every protected-process exclusion
against the original role/native pins, and confirmed the original private ADB
identity and exclusive loopback listener inode on 15047 unchanged. The separate
post-recovery latency check completed 50 samples, p95 138 ms and maximum 153 ms.
The continuation proof and raw receipts are retained outside Git under
`recovery-08-continuation`. This does not turn the failed automatic run into a
pass or qualify Linux.

The final normal Linux release build passed using default Tauri
`build --no-bundle`, identifier `dev.lomi.desktop`, and empty binary/library
Cargo feature sets. The ELF64 x86_64 executable is 87,422,144 bytes, SHA256
`36da17857f9764195c2a0e70354bf8c9c97623a91eb0c3f8d4f47ee22a4ded2f`.
All 155 product and three fixture source hashes, plus the retained debug binary,
matched before/after, with no errors or source-byte changes. Group-write bits on
private test paths were narrowed before build guards; no shared paths changed.
The receipt/log are retained outside Git under `normal-linux-release-07-execution`.
Normal Stop/Quit reaped the application, guest and WebKit descendants and left
no runtime record. The orphaned private ADB was retired through its exact pidfd
and socket pins, closing 15047 without touching a shared server. The private
quiet/awake leases were released, the paused-process count was zero and the
inhibitor child was gone. These are build and cleanup results, not Linux runtime
qualification. The later unhooked 1800-second follow-up and its separate cleanup
are recorded above; the observed thirty-minute memory requirement was accepted
with finite-duration caveats. Comparable GPU evidence remains unconfirmed and
Linux remains unqualified.

Subscribers omitting the RLE32 argument retain legacy deflate/raw negotiation.
The deflate fallback builds directly from validated borrowed RGBA into the final
packet, avoiding a full raw staging copy. Its DecompressionStream decoder writes
at most 4096 compressed bytes per native transform while concurrently reading
into bounded, reusable pixel storage. Rejected or canceled decoding waits for
all pending writes and cancellations before releasing the packet. This bounds
native expansion work; native copies and allocator retention still require
whole-application memory measurements. Native WebKit checks rejected overflow,
an expansion bomb, boundary corruption, flush-time
truncation and cancellation, then decoded exact valid pixels after every
failure. These functional decoder/renderer checks do not establish Linux CPU,
GPU or long-duration qualification; its qualification flag remains false.

Linux publishes completed public JWK files with a same-directory hard link,
without replacing an existing registration. The selected Linux emulator watches
file creation/modification events but does not discover rename-only publication.
The temporary file is fully written and synced before publication; credentials,
JWT renewal and authentication checks remain unchanged.

Native product checks on this isolated host have now exercised installation of
the private tools/JRE and AOSP API 36 x86_64 image, cold boot with persisted guest
data, GTK pointer delivery in all four rotations, blur/touch release, committed
UTF-8 text, native APK selection, a verified 720x1280 guest PNG through Save
screenshot, and ACK-timeout reconnect. These are individual functional checks,
not completed Linux qualification. The second-instance check observed the earlier
Chat AI owner guard; it did not independently exercise the Android directory lock.

The subsequent optimized debug candidate
`cdda5b89ec4b0878154feb3a6fd20fc3d65f1da1beccdaa29dcdca1701bb5bac`
passed cold boot/persisted marker and cancellation during native compressed
720x1280 decoding: no retired-stream draw or ACK, exactly one packet release,
and a fresh subscription after reopening the tab. After the thirty-minute trial,
fifty GTK pointer-to-image samples had p95 and maximum 132 ms. The corrected
latency driver observed the actual 720x1280 texture-upload source separately
from the measured 321x571 framebuffer; DPR was 1. It measures GTK delivery to
the second animation frame after the guest transition, not compositor-global
physical pointer delivery.

Its completed thirty-minute shared-view trial maintained 29.974 FPS and 110.499
decimal MB/s of logical IPC. Both views received 53,955 frames from the same
720x1280 phone/source; their framebuffers were 321x571 and 720x1280. The combined
application/guest CPU increase was 0.45150 cores, below the 0.5-core budget.
Application/WebView private resident memory increased by 41.24 MiB on average;
the visible peak exceeded the baseline mean by 88.22 MiB, below 128 MiB. The
last-ten-minute memory slope was -0.413 MiB/min, with no sustained late growth;
the following hidden-minute mean returned to 341.90 MiB versus a 335.28 MiB
baseline. The fixed offline resource analyzer independently checked the retained
driver-source digest and complete process roster. CPU, memory, FPS, IPC and the
separate latency comparisons passed. An optional scheduler trace had incomplete
task/job attribution and supplies no GPU-budget result. Comparable GPU evidence
remains pending.

The subsequent recovery fix treats a strictly verified Linux zombie as exited
and removes an unchanged stale runtime record when recovered Stop finds the
guest already gone. Runtime record publication and cleanup share a short mutex
so cleanup cannot delete a replacement record. Focused native tests passed using
normal features: Linux identity/runtime suites passed 4 and 7 tests respectively;
Windows passed 5 and 6. Each identity suite also ignored one subprocess fixture.
The fixed optimized debug candidate
`5da8499c4d96f8843f94f387425eed8bac3589211178486d275961c76028eee5`
passed the complete native crash/recovery trial: a pinned app SIGKILL preserved
the guest, the replacement app recovered it, and recovered Stop removed the QEMU
process and runtime record. A verified SDK helper settled a further 446 ms later.
A subsequent fresh-generation cold boot delivered 720x1280 frames and retained
the guest marker; the borrowed private ADB server remained alive as expected.
The report is `linux-recoveryfix-recovery-02-recovery.json`. Earlier failed
trials remain preserved; the first immediate post-Stop process scan did not
record the transient process's identity. The
thirty-minute resource and latency results above belong to the earlier `cdda`
candidate, not this recovery fix.

The recovery-fixed candidate then passed a separate two-minute shared-view
regression: 29.979 FPS, 110.517 decimal MB/s logical IPC and a combined CPU
increase of 0.49680 cores. Application/WebView private resident memory increased
by 42.33 MiB on average; the visible peak exceeded the baseline mean by
78.28 MiB. Fifty new GTK pointer-to-image samples had p95 146 ms and maximum
147 ms, observing the same actual 720x1280 source and 321x571 framebuffer.
The resource report is `linux-recoveryfix-120-01-resource-report.json`; the
latency report is `linux-recoveryfix-latency-02.json`. The normal macOS runtime
suite also passed six tests, with its real-emulator fixture left opt-in.
The final normal-feature Linux Android suite passed 102 tests, with 16 opt-in
or subprocess fixtures ignored; all three graphics-policy tests also passed.
The matching normal Linux release built with `dev.lomi.desktop`, features `[]`
and no fixture environment in 94.56 seconds. Its 86,307,360-byte executable has
SHA-256 `ebfdd9ab9491934f1af53ec2faed44fe37e05c96ae61a6f2ea44d6b58f3423a2`.
The `normal-linux-release-recoveryfix-01.json` receipt verifies 148 product-source
hashes before and after the build; the executable was not installed or launched.
Comparable GPU evidence is still pending.

An isolated off-repository Vulkan timestamp layer and source-pinned NVK kernel
trace subsequently produced a complete two-minute coverage report for the same
recovery-fixed executable. The summed application and guest command-interval
enclosures, including separately bounded driver-private work, were 0.231946 s/s
(`linux-pooled-gpu-05-combined-report.json`). This absolute upper bound exceeds
0.10 s/s and remains inconclusive for actual additional GPU activity: command
intervals can contain waits and stalls. Subtracting hidden-panel upper bounds
would not establish a valid additional-cost result. A later thirty-second
marker-placement diagnostic measured 0.210538 s/s, but lacks full private-work
and helper coverage and cannot qualify the platform
(`linux-prestart-gpu-06-query-only-diagnostic.json`). Neither trial changes the
production qualification flag. These private measurement tools are not shipped.

A later, explicitly authorized host test installed NVIDIA's recommended open
595.99.02 driver and restarted the same Ubuntu machine. WebKitGTK is
2.52.6 and GTK is 3.24.52. The unchanged `5da` candidate cold-booted the phone
with the persisted marker and completed a two-minute, two-view run at
29.893 FPS with a 720x1280 source. Its 3,589 gRPC frames matched the IPC frame
count; there was no excess source acquisition in this run. Fifty subsequent
GTK pointer-to-image samples had p95 140 ms and maximum 153 ms
(`nvidia-latency-01.json`). The subsequent thirty-minute, two-view trial maintained
29.795 FPS and 109.837 decimal MB/s of logical IPC, with 53,633 displayed frames.
Its combined application/guest CPU increase was 0.52760 cores, above the 0.5-core
budget; this is not a CPU pass. Application/WebView private resident memory
increased by 48.34 MiB on average; the visible peak exceeded the baseline mean by
93.87 MiB, within the 128 MiB component budget. The last-ten-minute descriptive
memory slope was +0.365 MiB/min; this alone supplies no sustained-growth verdict.
The independently source-checked report is `nvidia-long-01-resource-report.json`.
Private Nsight Systems 2026.5.1 captures and positive graphics/COPY controls
collected native hardware metrics. The whole-device conservative bound did not
establish compliance with the additional GPU budget. Context attribution is
diagnostic only, and Linux remains unqualified. The driver and profiler changes
belong solely to the authorized test host; the application does not install
them or alter desktop preferences.

An earlier compressed candidate's two-minute shared-view trial maintained
29.972 FPS at 720x1280 with 110.520 decimal MB/s of logical frame IPC. Its combined
application/guest CPU increase was 0.51737 cores, above the 0.5-core budget;
it is not a CPU pass. The scheduler-residence GPU bound was 0.36544 s/s, above
0.10 s/s and inconclusive for actual incremental GPU activity. It cannot be
converted to a pass by subtracting residence upper bounds. These measurements
preceded decoded-buffer reuse and direct RGBA uploads to Linux secondary views.

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
The equivalent elevated PowerShell command is
`Enable-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform -All -NoRestart`;
save work and restart Windows afterward. Preflight records the WHP API HRESULT,
returned BOOL and buffer size separately from optional-feature inspection.
Its native processor-feature checks also work without WMI access, but a false
firmware flag inside a VM or under Hyper-V/VBS does not prove a BIOS setting.
Do not remove the platform guards or reuse the macOS product runner as Windows
evidence. Retain preflight evidence outside Git. The Windows qualification below
separately verified native process ownership, cleanup, rendering and input;
preflight alone does not establish those properties.

Run the Windows launcher ownership regressions without an SDK or hypervisor:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --locked -p lomi-control-core process_tree:: --lib
cargo test --manifest-path src-tauri/Cargo.toml --locked -p lomi --lib android::
```

The process-tree fixture models a launcher waiting for a separate QEMU process.
It checks graceful exit, Force Stop, owner drop and an unrelated live process.
The production Windows launcher is assigned to a Job Object before it resumes;
runtime discovery selects only a member with matching managed executable, AVD
and gRPC port. These subprocess regressions do not qualify emulator boot,
rendering or input.
The release security gate runs the Android unit suite on every platform and
the process-tree regressions on Windows.

### Windows installation prerequisites

The verified private Temurin JRE supplies the CLI's required Visual C++ runtime
DLL fallback through its `bin` directory, prepended only to the child `PATH`.
This also avoids a missing-runtime launch failure (`0xC0000135`). Lomi does not
install global Java or modify the user's `PATH`. External tools receive normal
drive paths such as `C:\Trials\...`; internal containment and ownership checks
retain canonical paths.

The Windows toolchain pins CLI `1.0.16625370` at the
[immutable download URL](https://dl.google.com/android/cli/1.0.16625370/windows_x86_64/android-cli.exe):
87,366,808 bytes, SHA-256
`3782acdbdc2a2f589f81de5b853a74623765d018dd6a91069028a2a8ea6a3554`
([official checksum](https://dl.google.com/android/cli/1.0.16625370/windows_x86_64/android-cli.exe.sha256)).
The previous `1.0.16261425` Windows CLI had a DLL-detach crash (`0xC0000409`),
reported fixed in [Google issue 558351427](https://issuetracker.google.com/issues/558351427)
and the [AWS-LC issue](https://github.com/aws/aws-lc/issues/3540).
The [CLI release notes](https://developer.android.com/tools/agents/android-cli/release-notes)
list the newer releases but do not identify the first build containing that fix.

SDK and image licenses still require explicit user consent to the exact terms.
These pins and path fixes are prerequisites. The recorded native installer and
functional checks below do not qualify Windows on their own.

### Recorded Windows functional verification

The 2026-10-19 isolated Windows 11 Pro x64 trial uses an Intel i5-11400F,
32 GiB RAM, NVIDIA RTX 3060 Ti, WHPX and WebView2 `155.0.4283.62`.
The native Settings installer downloaded and verified the private tools, JRE
and AOSP Android 16/API 36 x86_64 revision 2 image after explicit license
acceptance. The test phone uses 720×1280, Host GPU, two virtual CPUs,
2560 MiB RAM and cold boot. A Windows 10 LTSC host compiled the changes and
ran the native unit suites separately; its unavailable WHPX did not qualify
emulator execution there.

Actual Windows messages and guest UI reads verified touch in all four
rotations, drag, letterbox rejection, shared docking, native APK selection and
a decoded 720×1280 screenshot. Minimize and the application's native Save
dialog released held touch and rejected inactive input. WebView2 child focus
is checked through the owning foreground window and Windows session;
the menu restores its trigger only when appropriate for that dismissal.
Committed text reached the guest exactly as
`Zażółć gęślą jaźń — こんにちは 😀🧪`, including split UTF-16 `WM_CHAR`
surrogates. This verifies committed Unicode text, not Windows IME composition
or physical keyboard layout switching. Fifty native input-to-image samples
had p95 132.8 ms on the earlier debug executable. The final optimized candidate
repeated 50 native samples at p95 114.2 ms (maximum 116.3 ms), using a single
Fit view with 387×688 source/framebuffer/CSS, DPR 1 and a 720×1280 guest.
The native foreground/focus precondition was established before this run;
an earlier unfocused fixture attempt timed out and is not a latency result.
Graceful Stop and guarded
application exit removed the owned runtime, and a guest marker survived cold
restart. Keep raw receipts, executable hashes and reports outside Git.

Real emulator ownership trials covered Force Stop and abrupt owner exit both
while launching and while running on an earlier optimized executable. The final
candidate repeated the running cases: retained native identities confirmed
launcher/QEMU exit, the next launch recovered the same guest marker with a new
generation, and an unrelated benign process stayed alive. Final guarded quit
removed the runtime record and owned tree. The harness then released its scoped
awake requests, removed only its two scheduled tasks, and closed the original
fixture-only ADB through an exactly matched retained handle. Production managers
still preserve borrowed ADB servers; no global `kill-server` was used.

The optimized debug candidate (optimization level 2, assertions enabled,
SHA-256 `709205d92009fe4343ac3ce5417ea836dd9327ddb241003efb84988fc8ee151e`)
completed 1,800 uninterrupted visible seconds with two views of the same phone.
Both views drew 53,987 frames, averaging 29.991 FPS, with 53,445 observed RGB
fingerprint changes each. Native shared-buffer deliveries matched IPC packets
and draws. Source resolution was 720×1280; actual-size and secondary framebuffers
were 720×1280 and 216×384, respectively, at DPR 1. Logical delivered packet
throughput was 110.562 MB/s. The source generation and stream epoch stayed fixed.

Application/WebView private memory averaged 267.68 MiB against a 222.09 MiB
hidden baseline (+45.59 MiB), with a 294.72 MiB peak (+71.42 MiB). Consecutive
five-minute means were 261.12, 267.20, 268.33, 269.37, 269.28 and 270.65 MiB.
The late means remained near the established allocation plateau and below the
+128 MiB budget. The last-ten-minute sampled trend was +0.316 MiB/min; no actual
GC events were instrumented. After hiding the views, private memory returned to 225.61 MiB
by the end of the following minute. This is a thirty-minute observation, not a
lifetime bound.

The original PowerShell CPU aggregate selected an integer `Math.Max` overload
and rounded each process's cumulative CPU seconds. That thirty-minute CPU result
is inconclusive and must not be used for qualification. After fixing the overload
to use doubles, a separate 374.095-second visible tail measured 1.299831 CPU
cores including the guest; the following 60.024-second hidden control measured
0.813923 cores, for +0.485908 cores. Guest swipes continued unchanged across both
phases. This precise paired observation does not repair the earlier CPU data.

A separate 120.116-second final trial verified 3,601 draws per view (29.979 FPS),
3,577 changing-frame fingerprints, matching native shared/IPC deliveries and
the same 720×1280 source, generation and per-view dimensions. Native
`GetProcessTimes` snapshots retained all nine app/WebView/launcher/QEMU identities
and integer 100 ns CPU times. QPC brackets conservatively bounded the active
and hidden intervals; the combined additional CPU rate was 0.492373 cores.

The paired Windows GPU measurement used native PDH `GPU Engine` cumulative busy
timers, with actual type `PERF_100NSEC_TIMER`, a 10 MHz time base and matching
native endpoint-rate calculations. All 18 observed app-GPU/QEMU engine instances
stayed continuous and valid throughout both windows. For each of the app and
emulator groups, summing active process/physical-engine busy time bounds its
active union from above; the largest single baseline engine bounds its baseline
union from below. The active interval enclosed the visible benchmark and a
two-second animated drain; 120 seconds was its conservative denominator. Native
outer QPC barriers gave a 61.000221-second baseline duration upper bound.
Summing both groups' nonnegative differences gave 0.081195 additional busy
seconds per second, within the 0.10 budget. The absolute active engine sum was
0.105510 s/s; it is not an additional-cost result.

This is a recorded sampled-host GPU measurement, not a universal coverage claim.
The other seven held app/browser/renderer/utility/launcher processes exposed no
GPU instances; they are listed in the raw report rather than assigned fabricated
zero counters. Polling at 250 ms can miss wholly unobserved short-lived counter
instances or children. No known instance/status/process gaps occurred in the
measurement windows; initial rate priming was excluded. Keep the native helper
source, executable hash, raw counters, phase barriers and reports outside Git.
Failed static-subpage and diagnostic-timeout trials remain inconclusive and
are not combined with the completed trial.

### Normal production build verification

The native measurements above used an optimized debug fixture with assertions
enabled. Normal production builds were verified separately: macOS ARM64
`pnpm exec tauri build --no-bundle` passed, as did the Windows x64 NSIS build
with the normal `dev.lomi.desktop` identifier and no probe/native-smoke features.
The Windows build used `--bundles nsis --no-sign --ci`; its private packaging
override disabled updater artifacts so no signing key was needed. The latest
recovery-fixed unsigned installer is 40,522,032 bytes, SHA-256
`0faaa310106c8bf228e56bf72f3f64d3848b8bb741bd9892bf94b0e2da75999c`.
The release executable SHA-256 is
`9d34b776061ea421a988d561d5ab38f0d0934ff53c912e7453df1f2306076b7d`.
This repeat build ran on the Windows 10 build host in 322.4 seconds and verified
all 1,401 captured source hashes before and after packaging. The source snapshot
pin is `19474f1a88403e9dbec90bd298d11c3d95abf24f78f913166831bfc262d00fb3`.
Only the local Linux recovery test fixture changed after capture; the remote
snapshot and product build inputs remained unchanged. This is compilation and
packaging evidence, not Windows 10 GUI qualification, and does not replace the
Windows 11 native runtime qualification. The existing debug executable stayed
unchanged. The installer and receipt are retained outside Git under
`/tmp/lomi-windows-recoveryfix-20261020/`, including
`recoveryfix-normal-20261020-summary.json`.

A later Windows native-link regression passed on Windows 10 Enterprise LTSC
21H2, build 19044, on 2026-10-20 from 08:39:12 to 08:43:15 UTC:
`cargo.exe build --locked --offline --release --bin lomi` exited 0. The approved
three-file update covered `src-tauri/src/android/frames.rs` (SHA256 prefix
`307b08d6`), `src-tauri/src/android/commands.rs` (`c57b8179`) and
`src/android/frame.ts` (`abf65b19`); 1,401 source pins were checked before/after,
with the other 1,398 unchanged and no Cargo.toml or Cargo.lock changes. Both
fresh binary and library Cargo fingerprints recorded `features: []`. The linked
executable is 53,531,136 bytes, SHA256
`b8db45cbe123417fba6841e3621b68b397d39dc858ad52e66922bad9c1f8a55e`;
the prior `9d34b776` executable was separately preserved. Receipts, supplemental
fingerprint evidence and the log are retained outside Git under
`/tmp/lomi-final-native-20261020/`. This verifies native release compilation and
linking only: it did not rebuild the frontend bundle or installer, perform a new
GUI runtime qualification, or remove the Windows 10 host's WHP limitation. The
historical Windows 11 native qualification remains separate.

The latest normal macOS ARM64 recovery-fixed executable is 76,313,712 bytes,
SHA-256 `6fa8458b307a858fb9e45d90cbf20598056678589549da2cdf3336efbed0de15`.
Its `pnpm tauri build --no-bundle` receipt is
`/tmp/lomi-macos-normal-build-recoveryfix-20261019.json`; the build used
`dev.lomi.desktop` and features `[]`, with no install or launch.

Frontend type checks and 265 host/45 AI runtime tests passed. Native Android
unit suites passed on macOS (94 passed, 17 ignored) and Windows (89 passed,
16 ignored); Windows process-tree regressions passed separately. After enabling
Windows qualification, the focused manifest/host qualification test passed
again on macOS. Keep build logs, feature receipts and artifact hashes outside
Git with the runtime evidence.

### Windows frame ownership

Windows uses WebView2 SharedBuffer delivery when the JavaScript bridge and
native Environment12/Core17 interfaces are available. Each frame gets a fresh,
bounded read-only mapping of the existing binary packet; there is still only
one unacknowledged frame per stream. The native owner closes after posting.
JavaScript explicitly releases its mapping after all synchronous canvas uploads
and copies, or when canceling a queued draw. A permanent broker also releases
late deliveries for retired subscription tokens. No pixel allocation is reused
while JavaScript may still read it.

Posting awaits the UI callback with a two-second delivery deadline before the
existing frame ACK deadline. Cancellation immediately clears queued pixel data,
including while the UI is stalled. Only missing native SharedBuffer interfaces
select the existing binary-channel fallback; allocation/posting errors disconnect
the stream. macOS and Linux retain binary-channel delivery.

Frame pacing follows a fixed 30 Hz schedule and skips missed ticks. Packet
construction and UI delivery no longer extend every frame period. The source
still retains only its latest waiting frame and waits for each rendered frame's
ACK before sending another. Late timer wakes can shorten the next interval;
this is an average rate target, not a minimum spacing between adjacent frames.

Shared delivery is bound to a main-document generation and the trusted bundled
app source, or the configured development origin in development mode. Main-page
loading invalidates streams and releases input without initializing Android.
The native source and generation are checked immediately before posting. The
recorded native replacement-page regression received zero shared buffers, then
returned to the same phone with a new stream epoch and subscription token.
These ownership checks are separate from the performance measurements above.

### Windows product resource sampling

With the isolated product fixture already running, use Windows PowerShell 5
or later to collect application and WebView2 CPU/RAM evidence outside Git:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/native/android-product-windows-perf.ps1 -ProductDirectory C:\Trials\lomi-android-stage0-UNIQUE\product -DurationSeconds 1800 -EvidenceName windows-visible-UNIQUE -IncludeGpu
```

The read-only sampler writes a new `product/windows-visible-UNIQUE.jsonl` and
refuses overwrite. It verifies the receipt, executable hash and native process
creation identity, discovers WebView2 descendants with Toolhelp snapshots, and
samples every two seconds. CPU cores are process CPU seconds divided by elapsed
wall time; working set and private bytes are separate sums. Emulator, adb and
other descendants are excluded. Exited children retain readable cumulative CPU;
unavailable final reads and discovered child identities are recorded. Processes
that start and exit between snapshots may be missed. Duration accepts 1–3600
seconds and defaults to 1800. The sampler does not drive input, change focus or
start/stop processes. Arrange the visible animation or baseline separately and
pair these samples with framebuffer, FPS and IPC evidence; the sampler alone
does not qualify Windows. Preserve fractional cumulative CPU seconds when
aggregating processes; integer overload selection invalidates precise budgets.

Optional `-IncludeGpu` uses a persistent native PDH query for Windows GPU Engine
utilization. It records verified process/creation identities, raw engine values,
same-engine cohort totals and the busiest engine's fraction. It does not add
utilization across different engines or equate this proxy with macOS GPU active
intervals. Numeric rates require the same retained process identity at both
collection endpoints. Missing counters, invalid rates and new/reused identities
mark evidence incomplete; an unknown interval cannot become a zero measurement.
Initial priming is recorded separately. CPU/GPU samples include UTC timestamps
for joining the visible driver's phase journal.

For the fixed visible workload, first arrange one phone in one or two shared
views, with a separate `Terminal` tab. Run the resource sampler separately for
about 2050 seconds with its own fresh evidence name, then start this driver:

```powershell
node tests/native/android-product-windows-visible.mjs C:\Trials\lomi-android-stage0-UNIQUE\product DEVICE_UUID visible-UNIQUE 1800 "Android views" 2 shared
```

Arguments after the evidence name default to 1800 visible seconds, the device
name as the active tab, and one view. Duration accepts 1–1800 seconds; two views
must share the same phone. The driver opens Phone settings through the actual
UI, resolves the guest's Settings homepage and resets only that activity task
through the guarded private transport. This does not clear Settings data or
restart the phone. It then continuously alternates fixed one-second swipes
through the already running private ADB smart socket. Each swipe checks the
guest UUID and private generation on the same transport. It never starts ADB
or stops the phone. A separate three-second changing-frame preflight precedes
the warmup and resource windows; its counters are not added to the main trial.
It hides the panel on `Terminal` for 30 seconds of warmup and a 60-second
baseline, measures the visible tab, records another 60-second hidden baseline,
then restores the requested tab. Keep the window visible and the layout fixed.
Warmup and both hidden baselines check once per second that no production stream
or visible phone canvas has returned. Guarded, bounded activity diagnostics
verify the exact resolved Settings homepage component after task reset and
before and after the visible measurement. Package identity alone is insufficient:
a static Wallpaper subpage can accept swipes without producing the intended
animation. An observed failure on that subpage returned to 29.7 FPS in the same
stream epoch after one native Back action, without restarting the app or phone.
Avoid UIAutomator idle-dependent XML reads during continuous swipe input; they
can time out independently of stream health. Keep the physical display awake
and the fixture window presented for the entire measurement. Recorded trials
held a scoped display/system execution request and released it afterward;
they did not change the system's global power policy.

Fresh `NAME-driver.json`, `NAME-phases.jsonl` and `NAME-frames.json` evidence
records completion, UTC phase boundaries and per-view source/framebuffer/CSS/DPR,
FPS, frame-gap and draw-call p95, plus shared stream IPC/gRPC counter deltas.
Each view also counts changes in fingerprints from 64 spaced RGB pixels of the
existing RGBA upload buffer; no pixels or fingerprint values are exported.
Unsupported upload types are incomplete evidence. A view needs at least three
changes, at least one change per requested visible second overall, and no
five-second interval without a change; repeated identical draws cannot pass.
Use the frame report's UTC start/end for the exact visible resource interval;
the separate sampler should use a different name, such as `resources-UNIQUE`.
Hidden, detached, replaced, resized or stalled views and changed stream
generation/epoch fail the run. Cancellation stops new gestures, closes only
owned sockets/controller helpers, cancels measurement hooks and attempts tab
restoration; an in-flight guest swipe may finish. Partial reports are preserved.
The Node driver does not invoke the PowerShell sampler or collect GPU metrics.
Running either driver is not a native qualification pass by itself.

The final optional argument is `any` (default) or `shared`. Requiring `shared`
counts native shared-buffer event metadata without obtaining another mapping,
and checks its count against primary draws and delivered packets with a
two-frame boundary tolerance. This distinguishes the selected Windows transport
from its binary-channel fallback. IPC byte counters describe delivered logical
packet bytes, including shared-memory payloads; they are not a measurement of
bytes copied across the browser process boundary.

The recorded macOS transport is authenticated emulator gRPC → Rust → binary
Tauri Channel → retained WebGL Canvas, with source scaling and a 720 × 1280 ceiling.
The original feasibility runner also contains rejected JPEG/2D experiments.
Its results do not replace verification of the integrated product.

The integrated v3 product completed 1,800 uninterrupted visible seconds at
720 × 1280: 28.03 FPS, 0.2993 additional CPU cores including the guest,
103.32 MB/s IPC and 0.01561 s/s additional GPU active intervals. Raw historical
reports are kept outside the source repository.
The memory report includes every identified app/webview process and explains
the higher terminal-view baseline; it does not claim a negative transport cost.

## Integrated product fixture

`run-android-product.mjs` builds the actual workbench and Settings UI with a
unique application identifier and separate app-data/workspace. It reuses only
the isolated `native-managed-*` SDK installed by the native installer trial;
it never uses the user's Lomi session, SDK or AVD. The directory owner
lock remains active. The product's Rust runtime owns emulator children and a
private ADB listener on 15047. File pickers open at fixed trial locations in
this feature-only build; selection and all file/guest operations still use
production commands. The fixture never accepts vendor terms.

```sh
node --experimental-strip-types tests/native/run-android-product.mjs \
  /tmp/lomi-android-stage0-20260918/native-managed-<trial-id>
```

Supply a working `SDKROOT` for the local Xcode installation when necessary.
The product runner supports macOS ARM64, Windows x64 and Linux x64 and builds
a debug executable with `--debug --no-bundle --features android-probe` under
`src-tauri/target/debug/` (`lomi.exe` on Windows). On Windows, use the same
runner from the repository root with an isolated drive path:

```powershell
node --experimental-strip-types tests/native/run-android-product.mjs C:\Trials\lomi-android-stage0-UNIQUE\native-managed-UNIQUE
```

`--reuse-binary` verifies the previous executable digest, app identifier and
canonical managed directory, plus the debug profile, platform and architecture,
before reopening that isolated session. Rebuild
after Rust changes; do not overwrite a running fixture binary.

### Linux product and measurement drivers

Launch the runner as the ordinary fixture owner inside the verified graphical
session. It inherits the session's GUI environment; do not invent `DISPLAY` or
run the application as root. On Linux, its separate identifier is stored under
the actual absolute `XDG_DATA_HOME`, or `~/.local/share` when that variable is
absent, empty or relative, matching Tauri's data-directory resolution. It
creates the isolated session with `local:bash`; ordinary Lomi app-data is not
the fixture target. The private SDK/workspace and consent requirements above
still apply.

```sh
node --experimental-strip-types tests/native/run-android-product.mjs \
  /tmp/lomi-android-stage0-UNIQUE/native-managed-UNIQUE

node tests/native/android-product-install.mjs \
  /tmp/lomi-android-stage0-UNIQUE/product linux-tools-UNIQUE tools
node tests/native/android-product-install.mjs \
  /tmp/lomi-android-stage0-UNIQUE/product linux-image-UNIQUE image \
  'system-images;android-36;default;x86_64'
```

The Linux installer selects AOSP x86_64, not the Mac ARM64 image. It checks
the exact displayed provider terms against recorded consent before accepting
anything in the UI. Host support in these opt-in drivers does not enable the
production Linux qualification flag.

Prepare a running fixture phone, a **Terminal** tab and the requested visible
phone tab before measuring. For two views, prepare **Android views** with two
panels referencing the same device UUID. Start the independent resource sampler
in one terminal, then the visible driver in another; use fresh evidence names
and stop builds and other test workloads during measurement:

```sh
python3 tests/native/android-product-linux-perf.py \
  /tmp/lomi-android-stage0-UNIQUE/product --device '<fixture-device-uuid>' \
  --duration-seconds 2070 --evidence-name linux-resources-UNIQUE

node tests/native/android-product-linux-visible.mjs \
  /tmp/lomi-android-stage0-UNIQUE/product '<fixture-device-uuid>' \
  linux-visible-UNIQUE 1800 'Android views' 2 any
```

For one view, pass the actual phone-tab label and `1 any`. Linux uses the Tauri
binary channel; the driver does not request Windows SharedBuffer transport.
Native Linux x86_64 explicitly opts into lossless RLE32 encoding 2, retaining
the v4 68-byte header, generation, epoch and sequence checks. Raw encoding 0 is
the fallback when the 7/8 payload threshold or smaller-wire requirement is not
met. Clients without the RLE32 argument retain legacy deflate/raw behavior;
`DecompressionStream("deflate")` support gates that deflate fallback with its
4096-byte input chunks. Both decoders bound output, reject packet aliasing,
cancel on retirement and reuse only their own stream's pixel storage.

Linux now renders the primary through synchronous Canvas2D putImageData and
secondary views through drawImage from that full-resolution primary canvas;
other platforms retain WebGL. The tracked driver and resource analyzer support
this Canvas2D path. Actual bitmap checks use 64 one-pixel readbacks per frame
only during the three-second bitmap preflight. The formal measured phase has
zero readbacks: it checks successful primary ImageData input fingerprints and
successful secondary drawImage(primary) canonical-source provenance, without
claiming a measured secondary bitmap. The analyzer validates preflight and pane
identities while preserving CPU/RAM arithmetic. Software backing remains an
inference; these checks establish no GPU or platform pass. Historical
shared-RGBA/WebGL results describe the earlier renderer. Logical IPC counters
remain the uncompressed packet budget, not compressed wire-byte measurements.
Linux remains `qualified: false`; the completed integrated build07 trial above
meets the recorded CPU, frame-rate and memory budgets, with growth and GPU
questions still open.
The driver resolves the guest's exact Settings homepage, resets its task, checks
the resumed component and runs a separate three-second changing-frame
preflight. It then records 30 seconds of hidden warmup, 60 seconds of hidden
baseline, the requested visible interval, and 60 seconds hidden afterward.
Alternating fixed guest swipes use only the generation-guarded private Smart
ADB listener on 15047; the driver never starts or stops an ADB server. Two views
must share one phone/source stream. Fingerprint changes, fresh uploads/draws,
framebuffer/CSS/DPR, source sequence, generation and epoch are checked; phase
timestamps align these observations with the independent resource samples.
Cancellation restores measurement hooks and attempts to restore the requested
tab without terminating the application or guest.

The Linux sampler pins the receipt's executable SHA-256, application PID,
boot ID, `/proc` start time and executable identity, and separately pins QEMU
to the private runtime record/generation. Integer `utime + stime` ticks divided
by `CLK_TCK` and the monotonic interval produce CPU-core rates; child CPU and
guest time are not added a second time. Application totals include verified
WebKit descendants, traversing sandbox bridges, and exclude QEMU, managed ADB
and unrelated helpers. QEMU CPU/memory is a separate guest cohort. Cgroup paths
are hints, not authorization to include every process in a desktop-session
cgroup. Exited processes retain their last readable CPU count; missing final
reads, discovery failures and unreadable/changed identities remain incomplete.

Memory is in bytes: **private resident** is `Private_Clean + Private_Dirty +
Private_Hugetlb` from `smaps_rollup`; PSS, RSS and swap are recorded separately.
This is not Windows private committed memory, and neither memory nor GPU
counters imply equivalence to the recorded Windows or Mac measurements.
Unavailable aggregate samples are unknown, not zero. The raw sampler always
reports `qualification: false`; it does not measure frame latency or qualify a
platform by itself.

Analyze completed, preserved files offline with the driver digest retained
independently when the measurement starts:

```sh
python3 tests/native/android-product-linux-resource-report.py \
  PRODUCT/linux-resources-UNIQUE.jsonl PRODUCT/linux-visible-UNIQUE-driver.json \
  PRODUCT/linux-visible-UNIQUE-phases.jsonl PRODUCT/linux-visible-UNIQUE-frames.json \
  --expected-driver-source-sha256 '<retained-driver-sha256>' \
  --output PRODUCT/linux-resource-report-UNIQUE.json
```

The analyzer checks the complete ordered process roster in every subsequent
sample, native identities, integer CPU totals and phase boundaries. It rejects
missing/unavailable members rather than treating them as idle. CPU comparisons
use strict interior samples and the combined application/guest visible-minus-
baseline mean. Memory summaries retain private resident, PSS, RSS and swap
separately; a last-ten-minute slope requires sufficient visible coverage.
Budget comparisons never qualify GPU, latency or the platform. Its self-tests
run with `--self-test`.

Optional `--include-gpu` reads verified app/guest DRM fdinfo clients,
deduplicates device/client identities, and retains engine nanoseconds and
capacity. Missing engine/capacity keys or unreadable clients are unavailable,
not idle. The recorded Nouveau session exported no usable DRM engine counters;
that absence is not a GPU-budget pass. A separate, optional root collector can
retain scheduler/ioctl evidence while the ordinary-user drivers run:

```sh
sudo python3 tests/native/android-product-linux-gpu-trace.py \
  /tmp/lomi-android-stage0-UNIQUE/product --device '<fixture-device-uuid>' \
  --duration-seconds 2070 --evidence-name linux-gpu-UNIQUE
```

Only this collector runs as root, via `sudo` from the real fixture owner; it
requires the nonzero `SUDO_UID` and validates the caller-owned fixture receipts,
consent, binary and native app/QEMU identities. Its fresh `lomi_android_<uuid>`
tracefs instance uses the monotonic clock and an 8 MiB buffer per CPU. It enables
only the three `gpu_scheduler` queue/run/done events and ioctl entry filtered
to the exact Nouveau EXEC, VM_BIND and legacy GEM_PUSHBUF request numbers.
It does not enable general ioctl/input tracing or write global tracing controls.
Evidence is created exclusively with mode `0600` and assigned to the caller;
cleanup disables and removes only its own instance. Never reuse old evidence
names or run the application, SDK installer or visible driver with `sudo`.

Trace records preserve raw lines, full-width fence identifiers, read times,
best-effort TID/TGID/start/executable/boot identities, task snapshots, event
formats, per-CPU statistics and loss indicators. Observed task/fd identities at
read time do not prove event-time ownership; missing attribution stays unknown.
Queue submitter TIDs are per-job attribution evidence; shared client IDs and
run/done worker contexts do not establish a cohort. Any later analysis of the
per-cohort union of run-to-done residence intervals is separate, conservative
work that must address coverage and lost events. It is not active GPU time or
a calibrated Mac metric. Subtracting paired residence upper bounds does not
produce a valid incremental upper bound. Raw traces make no qualification pass
and perform no baseline subtraction.

Linux input claims require saved live evidence from the owned GTK event-delivery
and committed UTF-8 paths. Available fixture actions alone do not prove those
checks passed. IME preedit/composition and compositor-global input remain
unsupported; GTK delivery is not a claim about a physical Wayland keyboard or
pointer. Wayland OS-window capture is unsupported without a compositor/portal
capture grant. Production **Save screenshot**, which saves the Android guest
image, is a separate operation and requires its own native file-picker and
output verification. None of these fixture additions changes Linux's pending
qualification status.

Drive the actual trusted application webview using unique instruction IDs:

```sh
node tests/native/android-product-control.mjs \
  /tmp/lomi-android-stage0-20260918/product inspect-1 <<'JSON'
{"window":"main","script":"return {text:document.body.innerText,visibility:document.visibilityState};"}
JSON
```

`window` is `main`, including when the Settings tab is active. Optional
`processId` restricts an instruction
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
  /tmp/lomi-android-stage0-20260918 \
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
with `image` to select the host ABI's AOSP API 36 image: ARM64 on the recorded
Mac trial (revision 2), x86_64 on Linux and Windows. The driver
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
`/tmp/lomi-android-stage0-20260918`):

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
node tests/native/run-android-smoke.mjs /tmp/lomi-android-stage0-20260918
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
python3 tests/native/android-perf.py /tmp/lomi-android-stage0-20260918 \
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
that historical JPEG trial did not complete thirty visible minutes. The current
one-phone budgets are +0.5 CPU cores, +128 MiB application/webview memory,
111 MB/s IPC and 24–30 FPS under animation. Two views share one phone’s
transport budget; thirty minutes must not show sustained memory growth, and
the p95 input-to-image target is 150 ms. This historical JPEG variant is not
the production transport.

The dangerous behavior of a normal ADB client is reproduced **only against a
private fake server** by:

```sh
node tests/native/android-adb-race.mjs /tmp/lomi-android-stage0-20260918
```

This test deliberately changes the server version after preflight, records
`host:kill`, refuses the request, and bounds execution. Exit 0 means the unsafe
client behavior was reproduced; it does not qualify a production ADB path.

The selected emulator's negative case, with a real independently owned ADB
server and a proxy changing its reported version after preflight, is:

```sh
node tests/native/android-emulator-adb-race.mjs /tmp/lomi-android-stage0-20260918
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
the clean Settings installer passed native qualification on the recorded macOS
ARM64 host.
Rust foundations separately test interprocess locking, immutable used images,
interrupted package publication, corrupt metadata preservation and cancellation
of actual child processes. Explicitly ignored tests run the real provider
catalogs, archives, download and owned guest fixture; their required environment
is `LOMI_ANDROID_PROBE_DIRECTORY`. These are complementary checks, not
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
