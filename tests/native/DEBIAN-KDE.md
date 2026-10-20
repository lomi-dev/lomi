# Debian KDE qualification — 2026-10-20

The final integrated candidate passed the recorded native KDE Wayland and X11
checks without injected WebKit/Mesa workarounds. The user confirmed smooth
operation of the final installed DEB on the physical X11 display.
These results do not establish support for every Linux graphics stack or replace
the published v0.6.1 assets.

| Host component          | Recorded version                          |
| ----------------------- | ----------------------------------------- |
| Distribution / libc     | Debian 13.7 x86_64 / glibc 2.41           |
| Desktop / sessions      | KDE Plasma 6.3.6 / Wayland and native X11 |
| GTK / WebKitGTK         | 3.24.49 / 2.54.0                          |
| Graphics                | Mesa 25.0.7, Nouveau, NVIDIA RTX 3060 Ti  |
| Secret Service provider | GNOME Keyring 48.0-1                      |

Production source baseline: `2aa1be6db554487ee45cbaa38f545e5573cf96c6`.
Final application code: `75c055daac29cad90f8a1e3e9e05dd1b5337c2ab`. The staged Git tree matched all 1,450
files in the final build manifest, including the unchanged Android source.
Packaging-only candidate A changes only `@tauri-apps/cli` from 2.11.4 to exact
2.12.3; production application code and graphics defaults remain unchanged.
Source verification checked 1,450 files without mismatches. Diagnostic fixtures
use separate builds and identifiers; their probes are absent from normal releases.
Final source additionally introduces the scoped Nouveau X11 shared-memory default
described below. Its prebuild verification checked 1,450 files with a seven-file
overlay and zero mismatches; this report is a separate subsequent documentation edit.

## Candidate A: Wayland installers and AppImage

- Public v0.6.1 DEB: clean APT installation **before development tools were
  installed**, packaged runtime checks, visible GUI, AT-SPI and normal tray Quit
  passed.
- Public v0.6.1 AppImage and baseline source bundled with CLI 2.11.4: blank GUI
  and WebKit process crash. The public artifact's WebKitWebProcess core records
  SIGSEGV in `libgallium-25.0.7`. Their GTK hook forces `GDK_BACKEND=x11`.
- The same production code rebundled with CLI 2.12.3: AppImage packaged runtime,
  visible GUI, AT-SPI and normal tray Quit passed. The user confirmed smooth
  typing, rapid tabs/panes, plaintext paste and scrolling on the physical KDE
  display. This is an observed packaging fix on this host, not proof of every
  graphics failure's underlying cause.
- The extracted-artifact backend guard rejected the old hook and accepted the
  candidate. Caller checks preserved unset, empty, `x11`, `wayland`, `x11,wayland`
  and custom `GDK_BACKEND` values.
- Candidate DEB upgrade preserved all 28 recorded profile files before first
  launch. `dpkg --verify` was clean; the installed executable hash matched the
  DEB payload. Runtime, GUI, AT-SPI, secondary launch and guarded tray Quit passed.

Candidate A SHA-256 values (not final integrated artifacts):

```text
DEB       aeb310ad2e196149cf060f4d08348158d771e40ecd5730e7e932c1a9e8da5e2f
AppImage  f3ccd55d2fa865035145238fb2ed6b1c0720ea9088d91da7f69cc986542b7688
```

## Candidate A: Wayland fixture coverage

- `latency-wayland-01`: passed 327 software input/render samples, including
  one/eight-terminal 30 FPS workloads, retained/visible panes, pane switching
  and input while a real session-save lock was held for three seconds. Aggregate
  final-key-to-render callback p95 was 23 ms, maximum 76 ms; no WebGL context loss.
- `instance-wayland-02`: passed 20 secondary launches, pre-readiness activation,
  two hide/reopen cycles, retained PTYs, AT-SPI, scratch credential round trip,
  guarded quit, fresh-PTY session restoration and contended runtime-owner handoff.
- `desktop-wayland-01`: exercise, restart and external verification passed for
  CodeMirror retention, UTF-8/BOM/UTF-16 and CRLF preservation, stale-save conflict,
  native watches, Explorer mutations/containment, local Git operations, editor
  settings, notification read state and session persistence.
- `plugin-artifacts-wayland/lomi-native-CAiGmo`: normal and safe-startup probes
  passed for trust gating, actual lazy ESM/assets, pane movement/state retention,
  PTY input, Explorer watches, themes and child-browser geometry/state/IPC denial.
- Focused native unit tests: 46 passed (graphics 6, renderer 2, terminal 23,
  instance 8, files 7). Application and AI runtime tests: 306 + 45 passed,
  zero failures in `evidence/unit-tests.log`.

## Native KDE X11 controls

The session was an actual KDE X11 login (session 15), independently verified.
These controls used unchanged application binaries with explicit process
environment settings. They precede final source integration.

- Baseline instance startup crashed WebKit in `libgallium`. A single Kopper
  override, Skia CPU rendering, disabled compositing, and Skia CPU plus disabled
  compositing also crashed; they did not resolve this native X11 failure.
- `WEBKIT_DISABLE_DMABUF_RENDERER=1` alone passed the complete instance lifecycle:
  20 secondary launches, credentials, AT-SPI, restore, owner-lock handoff and normal
  cleanup. However, `latency-x11-dmabuf-01` failed after 271/327 samples: an
  eight-terminal ANSI workload expired, with a recorded maximum of 153 ms.
- Adding disabled compositing selected DOM terminals. Control `-02` failed the
  one-terminal ANSI stage at p95 exactly 100 ms. Adding Skia CPU rendering as well
  (`-03`) failed the eight-terminal stage at p95 105 ms. Acceptance thresholds
  were unchanged. None is added as a Nouveau X11 default.
- `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` alone passed `instance-x11-shm-07`:
  20 secondary launches, scratch credentials, AT-SPI (five nodes, one frame child),
  restoration with fresh PTYs, real owner-lock contention with an additional
  254.356 ms hold, guarded exit and normal cleanup without forced retirement.
- The same single-switch control passed `latency-x11-shm-control-04`: all 327
  samples, aggregate final-key-to-render callback p95 23 ms and maximum 42 ms.
  The ordinary 30-second/120-sample stage had p95 18 ms, maximum 24 ms; the sample
  during a real three-second session-save hold was 18 ms. One-terminal ANSI had
  p95 22 ms, maximum 23 ms; the eight-terminal stage had p95 37 ms, maximum 42 ms.
  Nine finite 20-second/30 FPS workloads completed normally with exact input-byte
  checks. The run exercised 15 PTYs and eight tabs/panes, retained the original
  shell PID, had eight WebGL contexts with zero losses, and retired owned processes.

The successful SHM control retains WebGL and permits accelerated compositing. It is still
a software-proxy result; physical keyboard-to-KWin glyph presentation is not proved.

## Final integrated candidate: native KDE X11

The new default sets only `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1`, requiring nonempty
`DISPLAY` and any resolved Nouveau `renderD` driver binding. With a nonempty
`WAYLAND_DISPLAY`, the first exact `GDK_BACKEND` token must be `x11`; padded tokens,
Wayland-first lists and wildcards do not qualify. Without a Wayland display, an
unset backend or exact `x11`/`*` fallback token can qualify. The default Wayland
path remains unchanged; detection does not prove WebKit selected the bound GPU.

Any caller-supplied FORCE_SHM value or any presence of
`WEBKIT_DISABLE_DMABUF_RENDERER` suppresses the new default, including `0`, empty
and non-Unicode values. No new CPU-rendering, compositing-disable or
DMA-BUF-disable default is added. `lib.rs` and `terminal_renderer.rs` were restored
to baseline; the original compositing-only terminal selection remains in place.

WebKitGTK 2.54 supports [SHM-only transport](https://github.com/WebKit/WebKit/blob/webkitgtk-2.54.0/Source/WebKit/UIProcess/gtk/AcceleratedBackingStore.cpp#L72-L100)
without the explicit [GBM display branch](https://github.com/WebKit/WebKit/blob/webkitgtk-2.54.0/Source/WebKit/WebProcess/glib/WebProcessGLib.cpp#L132-L172),
while permitting GPU compositing. WebKitGTK 2.42/2.48 lack this switch and are not
qualified by these results. Ten graphics policy tests plus the original two renderer tests
passed on Debian, in addition to the 38 baseline focused native tests.

With no injected WebKit/Mesa variables, the frozen final binaries passed:

- `instance-x11-final-08`: 20 secondary launches, retained PTYs, credentials,
  AT-SPI, guarded exits, restored fresh PTYs and real owner-lock contention with
  256.218 ms additional overlap; cleanup retired normally without forced signals.
- `latency-x11-final-05`: all 327 samples, p95 23 ms, maximum 45 ms. The ordinary
  30-second stage had p95 18 ms, maximum 23 ms; the real three-second save-lock
  sample took 20 ms. Eight-terminal ANSI input had p95 42 ms, maximum 45 ms.
  All nine finite workloads completed normally with exact input, eight WebGL
  contexts had zero losses, the original PTY survived, and cleanup retired.
- `desktop-x11-final-01`: both exercise/restore reports and external byte/Git
  assertions passed. `plugin-artifacts-x11-final/lomi-native-luxf57` passed
  normal and safe-startup reports.
- Final DEB installation preserved all 130 recorded profile/project files before
  launch. `dpkg --verify` was clean and its installed executable matched the DEB
  payload. The ordinary application restored project/session v5, font size 15
  and exact `Witaj.txt` bytes; secondary launch retained the primary process.
- Ordinary DEB and direct-FUSE AppImage produced visible compositor screenshots
  and passed AT-SPI. The AppImage's WebKit child inherited SHM from application
  startup while its launcher had no graphics overrides; production tray Quit
  retired all recorded AppImage processes normally. Both packaged runtimes and
  the extracted AppImage backend guard passed.

Final integrated artifact SHA-256 values:

```text
DEB       73edcc255a8957b2bf4053b939746b5eb3be0183eeb41d3209554dfc7d2b00a7
AppImage  83eb8d2d6ad2e491b83fcde78c01da5f8098366cc56280b47d77aaf1d5f6bc7f
```

The user confirmed smooth physical-display typing, rapid tabs/splits, plaintext
paste and scrolling in the final installed DEB on X11. Both ordinary packages
also completed production tray Quit. One tray-property request timed out during
the manually requested session transition; later property reads and guarded Quit
passed, and a captured GTK main-thread stack showed normal GLib polling. This
observation has no established product cause and is retained in the evidence.

Candidate A hashes identify only the earlier packaging comparison.

## Final integrated candidate: native KDE Wayland

This was a new, independently verified Wayland login (session 25), with
`startplasma-wayland`, `kwin_wayland` and `WAYLAND_DISPLAY=wayland-0`.
The same frozen final binaries passed:

- `instance-wayland-final-03`: all 20 secondary launches, credentials, AT-SPI
  (nine nodes, two frame children), retained/restored PTYs, guarded quit and
  real owner-lock handoff with 256.553 ms additional observed contention.
- `latency-wayland-final-02`: all 327 samples, p95 23 ms, maximum 44 ms. The
  ordinary 30-second stage had p95/max 18 ms; input during the real three-second
  save-lock hold took 19 ms. Eight-terminal ANSI input had p95/max 31 ms.
  All nine finite workloads completed normally with exact input-byte checks,
  eight WebGL contexts had zero losses and the original PTY survived.
- `desktop-wayland-final-02`: exercise, restoration and external byte/Git checks.
  `plugin-artifacts-wayland-final/lomi-native-yy5u54`: ordinary and safe startup.
  Owned processes retired without forced cleanup or unproven survivors.
- Ordinary final DEB and direct-FUSE AppImage: visible compositor screenshots,
  AT-SPI and production tray Quit. WebKit children retained the existing Nouveau
  `LIBGL_KOPPER_DISABLE=true` default and had no X11 FORCE_SHM setting. The launcher
  had no injected graphics settings. All recorded processes retired normally.

The ordinary installed DEB was reopened with the preserved `Test Debiana` profile
for continued manual use. Temporary test sleep/screensaver inhibitors were stopped.

## Secret Service recovery

The first instance run, `instance-wayland-01`, failed its scratch credential
round trip. GNOME Keyring advertised `/org/freedesktop/secrets/collection/login`
without exporting its expected methods; metadata access returned `UnknownMethod`.
Replacing and unlocking the existing daemon using the account password through
stdin recovered the service (`Locked=false`). The unchanged instance fixture then
passed as `instance-wayland-02`, including set/read/delete/confirmed absence.

This was environment recovery, separate from the AppImage bundler fix. No provider
reset, keyring deletion or plaintext credential fallback was used. GNOME Keyring
[MR 99](https://gitlab.gnome.org/GNOME/gnome-keyring/-/merge_requests/99.diff)
was an investigation clue; these observations do not prove it explains all
causes of the initial failure.

## Evidence and interpretation limits

The local evidence root is
`desktop-audits/2026-10-20/debian-kde-qualification/` beside the source repository,
including `final-verified-evidence.tar.gz`, the earlier Wayland/X11 archives, the named fixture directories and
`evidence/` installer, host, build, integrity, keyring and test logs. The plugin
runner creates a random child directory; the recorded run is `lomi-native-CAiGmo`.

GTK widget key events, parser/render callbacks and WebKit snapshots are software
proxies. They do not measure physical keyboard-to-display latency or establish
glyph presentation by KWin. Manual plaintext paste was checked; automated Linux
clipboard, image-paste and IME coverage remains absent. The candidate was built on
Debian 13.7, so this trial does not establish an oldest portable build baseline.
Public v0.6.1 still contains the old AppImage until a separate release is published.
Windows and macOS installers have not been rebuilt with the new CLI in this trial.
