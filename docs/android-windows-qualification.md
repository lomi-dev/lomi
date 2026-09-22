# Windows Android qualification: NO-GO

Trial date: 2026-09-19. Source baseline:
`1a6ed0a5ff3bef23babbad7ed5a56230065d7221`, with the working-tree corrections
described below. This is a blocked Windows preflight and independent regression
run, not a completed Android native qualification. Linux was not tested.

## Host and blocking evidence

| Item                                | Observed value                                              |
| ----------------------------------- | ----------------------------------------------------------- |
| OS                                  | Windows 10 Enterprise LTSC, x64, build 19044                |
| CPU                                 | Intel Xeon E5-2650 v4, 12 cores / 24 logical processors     |
| RAM                                 | 34,195,742,720 bytes installed                              |
| GPU                                 | Radeon RX 580, driver 31.0.12027.9001                       |
| Active display                      | Parsec Virtual Display Adapter 0.45.0.0, 1920 x 1080        |
| System DPI                          | 96; actual application DPR/per-monitor scaling not measured |
| WebView2 installed                  | 153.0.4234.32; Android rendering not exercised              |
| Disk C at preflight                 | 23,591,071,744 bytes free; builds consume additional space  |
| Firmware virtualization / SLAT      | Both reported available by WMI                              |
| Hypervisor                          | WMI HypervisorPresent=false; WHvGetCapability reports false |
| Optional-feature state              | NOT RUN: DISM inspection requires administrator elevation   |
| Inherited Android/JVM configuration | No matching environment names; no user .androidrc           |

The real WHPX capability query returned unavailable. Administrator work and a
restart are required before the accelerated emulator trial can proceed. In
**Turn Windows features on or off**, enable **Windows Hypervisor Platform**,
then restart Windows and rerun preflight in a new evidence directory. If it
still reports unavailable, have the administrator investigate host boot and
virtualization configuration. No Windows feature, firmware setting, global
PATH, Java configuration or user shell profile was changed by this trial.

References: [Google's Windows acceleration guide](https://developer.android.com/studio/run/emulator-acceleration)
and [Microsoft's capability API](https://learn.microsoft.com/en-us/virtualization/api/hypervisor-platform/funcs/whvgetcapability).

## Reproduction and evidence

Run [the preflight script](../tests/native/android-windows-preflight.ps1) using
the command in [the native guide](../tests/native/ANDROID.md#windows-preflight).
It saves structured read-only checks, preserves previous runs and does not
install SDK packages or accept provider terms. A PASS on an inventory row means
the information was collected, not that the component is product-qualified.

Local logs and preflight JSON are under
`android-test/trials.local/windows-20260919-01/` (ignored by Git). This directory
also records the source commit, working-tree status and lockfile/toolchain
hashes. Keep raw evidence locally; it may contain machine-specific paths.
No Android tool, Java runtime, image, AVD or provider consent was provisioned.
No Android/ADB process was started, so there is no emulator cleanup to perform.
The existing macOS drivers remain gated and were not run on Windows.

Development tools: Node 24.16.0, pnpm 11.25.0, Cargo/Rust 1.95.0, Visual Studio
2022 MSVC 14.44.35207. Git was found in Visual Studio's Team Explorer bundle;
its directory was added only to the Rust test process PATH. Use `pnpm.cmd` in
PowerShell when the host execution policy blocks the `pnpm.ps1` shim.

## Corrections found by independent checks

- Prettier accepts each checkout's existing line endings, including Windows
  CRLF, without rewriting the repository. Git's hash-sensitive Android
  source/license exceptions remain unchanged.
- Search results normalize native path separators to project-relative `/`
  paths, including on Windows.
- Explorer ignores Windows `Modify(Any)` notifications for content/attribute
  writes; separate creation/removal/rename events still refresh directories.
- HTTP discovery fixtures consume complete request headers before closing,
  preventing Windows TCP resets caused by unread request bytes.
- The consent/stale-plan unit test seeds a pending plan rather than requiring
  host virtualization. Both rejection attempts still run production guards;
  the production acceleration/qualification gates are unchanged.
- Two equivalent boolean expressions were simplified for Rust 1.95 Clippy.
- The literal-ignore fixture uses a Windows-valid bracket filename while
  retaining the additional wildcard-name case on Unix.

## Checks

| Check                                   | Final result                                                              |
| --------------------------------------- | ------------------------------------------------------------------------- |
| pnpm install --frozen-lockfile          | PASS, lockfile unchanged                                                  |
| pnpm check                              | PASS                                                                      |
| pnpm test                               | PASS: 121 model tests and 17 AI runtime tests                             |
| pnpm test:ui --workers=4                | PASS: 404 Chromium tests with mocked native commands                      |
| pnpm build                              | PASS                                                                      |
| cargo check --locked                    | PASS                                                                      |
| cargo test --locked -- --test-threads=4 | PASS: 156 tests, 15 ignored; main/doc tests also passed                   |
| cargo clippy --locked -- -D warnings    | PASS after corrections                                                    |
| cargo fmt --check                       | PASS                                                                      |
| pnpm tauri build --no-bundle            | PASS: optimized Windows executable                                        |
| pnpm tauri build                        | FAIL at updater signing: MSI and NSIS created, private signing key absent |

All Cargo commands above use `--manifest-path src-tauri/Cargo.toml`.
Frontend/document formatting passed using a direct Prettier invocation with
the same arguments as `format:check`, on the final line-ending configuration.
The test suite includes real Windows shell startup, process identity, local TCP,
filesystem watches and owned installer-child exit tests. These component checks
do not establish native desktop UI or emulator behavior. One installer-output
test failed during the default-concurrency rerun and passed with four threads;
the original failure did not include its Outcome. Diagnostic output was added,
but its cause is not established and default-concurrency stability is not claimed.
After builds completed, 20 consecutive isolated repetitions of that test passed
using the same compiled test executable. Their individual logs are retained.

The initial Cargo test run failed with 135 passed, 21 failed and 15 ignored:
16 failures involved Git absent from PATH; the other failures led to the
corrections above. The initial UI run could not launch Chromium; its runtime
was subsequently installed for a separate rerun. Neither initial failure is
counted as passing evidence. The first formatting check failed on CRLF files;
the check passed after LF normalization. The final configuration instead permits
the original checkout endings, avoiding repository-wide working-tree changes.
The final process inventory found no remaining Node, pnpm, Cargo, rustc,
Lomi, emulator/QEMU or ADB process after the runs; no unrelated process
was terminated.

The normal release Cargo fingerprint records `features=[]`: no android-probe,
chat-probe or native-smoke. No updater signing key was requested or created,
and neither installer was installed or published. Package construction is not
an installed-package trial. SHA-256 values:

| Artifact                        | SHA-256                                                            |
| ------------------------------- | ------------------------------------------------------------------ |
| lomi.exe                        | `0f8518a21d81e52f17785616ad64c426c283c6ecd1c56f7c32b4f551c82f8e6d` |
| Lomi_0.3.0_x64_en-US.msi        | `d08627205aa3ebf34771ddd5a2dd4324dc0517b5c112ce6c2bf02c5ef1372fee` |
| Lomi_0.3.0_x64-setup.exe        | `ecdad5605d45373e5eae5f041e91db2cf65209135eb8c2e7f7b5d3a2d569148f` |
| Bundled Windows Node executable | `ba4e6d110e8c1592a1ecd390f6b05f3da124b13871a5be62b341a07a853c6c32` |

IME APK SHA-256, all three manifest-listed source hashes and the vendored
protobuf hash passed. The APK digest is
`4a1dc0861c689f986732172fefee2f33008d534cc6f59931e71afd916cda2bc7`;
protobuf is
`1d62c6bcad5f06621f90ec2bf26c661ba769ccd0f1416b5314d25a68e04eee5f`.
This verifies repository bytes, not the APK signature or Windows emulator
protocol compatibility. The Windows bootstrap manifest remains pinned and
`qualified=false`; no mutable provider binary was substituted.

## Native work not run

| Scope                                                               | Result and reason                                                      |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| Provider catalogs, binary help, SDK/image install                   | NOT RUN: accelerated native trial blocked; no new provider consent     |
| Launcher/QEMU ownership, discovery, recovery, forced/graceful Stop  | NOT RUN: no emulator started                                           |
| Authenticated gRPC, real frame transport, ACK/reconnect             | NOT RUN: no running guest or Windows product fixture                   |
| Unicode, composition, keyboard layout, intentional Paste            | NOT RUN: no guest input path exercised                                 |
| Settings setup, APK/PNG pickers, installation interruption/recovery | NOT RUN: no accepted isolated SDK installation                         |
| Shared views, multiple phones, docking, hide/minimize, persistence  | NOT RUN: no running guest                                              |
| ADB isolation/races, PID reuse, private permissions, Unicode paths  | NOT RUN: no Windows emulator lifecycle trial                           |
| CPU/RAM/GPU/IPC/FPS, input latency, 30-minute visibility            | NOT RUN: no transport; no performance result or GPU threshold claimed  |
| Native PTY/editor/browser/settings/updater regression               | NOT RUN: tests/builds alone do not exercise desktop UI                 |
| Native Chat runtime, credential storage and live APIs               | NOT RUN: no isolated desktop Chat probe; no live API consent/key       |
| Clean-profile installed-package qualification                       | NOT RUN: no qualified Android host; ordinary installation not replaced |
| Linux / X11 / Wayland                                               | NOT RUN: no Linux host                                                 |

Before resuming, enable WHPX and confirm sufficient free space for two images,
AVDs, staging and native builds. Review the exact provider terms for this new
host and obtain consent before installation. Port the opt-in product fixture
to real WebView2 and verify launcher plus QEMU identities before any production
gate change. Define the Windows GPU metric/budget before collecting performance
data, and complete the full native matrix and final-build thirty-minute run.
Do not treat this report or passing mock/unit tests as Windows Android support.
