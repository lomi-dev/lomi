# Completion notification retest

The user requested a new full verification run on 2026-10-13. This receipt
records that run separately from the initial implementation validation. All
backend integration databases were disposable local databases. No live migration,
deployment, store publication, cloud provisioning, commit or push was performed.

## Completed automated checks

| Area                | Check                                                                                          | Result                                                                                                        |
| ------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Backend             | Typecheck and build with `bun --no-env-file`                                                   | Passed                                                                                                        |
| Backend             | Full suite before the worker configuration fix, with restricted-role/schema fixtures           | 262 passed, 0 failed, 4,414 assertions, 35 files                                                              |
| Backend             | Latest full suite after the worker configuration fix                                           | 268 passed, 1 failed, 4,451 assertions, 36 files; existing 32-pane capacity test hit its five-second deadline |
| Backend             | Worker configuration, push dispatch/receipts and health focused tests                          | 31 passed; minimal restricted-role configuration passed                                                       |
| Backend worker      | Actual minimal-config entrypoint, startup and 65-second health probe, shutdown                 | HTTP 200 twice, SIGTERM exit 0; no identity-service secret or OAuth configuration required                    |
| Backend             | Explicit media receiver suite not auto-discovered by Bun                                       | 4 passed, 0 failed, 15 assertions                                                                             |
| Database            | Fresh disposable database, all 12 migrations, seven push tables                                | Passed                                                                                                        |
| Database            | Remote/push grants, migration status, worker readiness and actual privilege probes             | Passed; worker credential reads and schema creation denied                                                    |
| Desktop             | `pnpm check`, `pnpm test`, `pnpm build`                                                        | Passed; 246 frontend and 45 AI tests                                                                          |
| Desktop Rust        | Full workspace tests                                                                           | 948 passed, 0 failed, 56 ignored                                                                              |
| Desktop Rust        | Workspace/all-target check and required default Clippy                                         | Passed                                                                                                        |
| Desktop Rust        | Owned workspace/all-target Clippy, excluding unchanged vendored Snow                           | Passed after moving the notification helper impl before the test module                                       |
| Desktop native      | Actual isolated legacy notification/PTY smoke, `--inbox-only`                                  | Passed, 14 checks; this does not qualify new managed Tauri banners or OS presentation                         |
| Mobile              | Full `pnpm check` after the constants fix                                                      | Passed; 265 tests, lint, both typechecks, renderer/CSP, crypto provenance, config and release hygiene         |
| Mobile browser      | Exact `pnpm renderer:browser` assertions, actual agent-browser 0.21.0 and fresh Chrome session | Passed                                                                                                        |
| Mobile dependencies | Expo Doctor 1.20.4                                                                             | 20/21 passed; native duplicate check passed, eight existing patch-version warnings remain                     |

Broader `cargo clippy --workspace --all-targets -- -D warnings` reports 17 lint
errors in unchanged vendored Snow tests. The owned-target and required default
commands pass. Full Prettier continues to report the three unchanged
remote-terminal-runtime files listed in the initial validation receipt.

The backend Remote suites passed all 56 tests in a separate run. The latest full
run failed the existing 32-pane capacity test, as did its immediate targeted
retry. After all task-owned compilers/UI work ended, the exact test passed
unchanged: one passed, zero failed, 272 assertions, 661.78 ms within its original
five-second deadline. All 269 distinct backend tests therefore have passed in
this verification, while the full-run result remains 268 passed and one failed;
there was no subsequent uninterrupted full-suite run. The final exact-test log is
`/tmp/lomi-notifications-retest-backend-capacity-quiet-final.log`.
Disposable rate-limit fixtures were reset before the full and final targeted
runs to avoid previous runs affecting counter comparisons. Assertions and
deadlines were not weakened.

## Desktop UI rerun

The full single-worker UI run completed with 721 passed, 24 failed and one
skipped. All 24 failures were repeated with the original assertions/deadlines and
retained traces: 22 passed and two terminal-layout cases still failed. After the
reviewed asynchronous persistence assertion fix described below, both remaining
cases passed in a final 17.2-second run. Coverage across those runs is 745
distinct passed scenarios and one skipped; this is not an uninterrupted green
full run. The three new completion-notification Settings scenarios passed. The
pre-existing generated plugin fixture was preserved and restored with its
original bytes, inodes and modes after all three runs.

Full-run contexts are retained under
`/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-ui-full-failure-evidence-nk4ag1v_`;
retry traces are under
`/tmp/lomi-notifications-retest-desktop-ui-retry-results`.
The final two-case log is
`/tmp/lomi-notifications-retest-desktop-ui-layout-retry.log`.

## Native builds

- Mobile `CARGO_BUILD_JOBS=2 pnpm crypto:test`: **passed**, 24 Rust tests
  (five module, two core unit, twelve crypto and five workspace tests), none
  failed or ignored. This is separate from the provenance checks in `pnpm check`.
- Android development Debug APK, arm64-v8a only: **passed**, Gradle exit 0,
  `BUILD SUCCESSFUL` in 1m56s, 558 tasks (526 executed, 32 up-to-date).
  `:app:assembleDebug` used two workers, no persistent daemon, JDK 17 and a
  bounded 1,536 MiB JVM heap. Native preparation also passed. Push gate was OFF;
  no Android device or existing emulator was used for install/delivery testing.
  The 98,147,987-byte APK is
  `mobile-app/android/app/build/outputs/apk/debug/app-debug.apk`, with ID
  `dev.lomi.mobile.dev`, compile SDK 36/minimum SDK 24 and the Rust crypto library
  included. SHA-256:
  `bbfb2f009033d10005b02d11627679d9e5beb2366a37bd900426e1671eef1914`.
- iOS development Debug, unsigned arm64 simulator build: **passed**,
  `xcodebuild` exit 0 and `BUILD SUCCEEDED`. The earlier missing
  `RCTPackagerConnection` linker failure is resolved. The artifact is
  `LomiDev.app` under the existing Lomi DerivedData Debug-iphonesimulator
  products. Push gate was OFF; this does not establish APNs delivery, device
  installation or authenticated Account UI qualification.
- Desktop default-feature Debug build:
  `CARGO_BUILD_JOBS=2 pnpm tauri build --debug --no-bundle --no-sign --ci`
  **passed**, producing `src-tauri/target/debug/lomi`. It includes the unchanged
  notification plugin 2.4.0. No signed packaged app, OS banner or click behavior
  was qualified by this build.
- Fresh iPhone 15 Pro Max simulator on iOS 26.5: boot/readiness completed in
  3m06s; the isolated `dev.lomi.mobile.dev` binary installed and its native
  development launcher opened. **Actual Lomi JS/Account UI startup was not
  verified.** Metro initially bound IPv6 only; the bounded IPv4 retry reached
  the OS “Open in Lomi Dev?” confirmation. No simulator input driver was
  installed to accept it, and no JS bundle request was observed. The screenshot
  `/tmp/lomi-notifications-retest-mobile-ios-launch-retry.png` and recording
  `/tmp/lomi-notifications-retest-mobile-ios-smoke.mov` preserve this limit.
  The owned Metro/recording and fresh QA device were cleaned up; existing device
  data was not used.

## Small fixes made during verification

- The push worker now uses a dedicated minimal configuration parser rather than
  the API's complete identity/OAuth parser. This removes an unnecessary runtime
  identity-secret requirement. API validation and receiver ownership predicates
  are unchanged. Seven parser tests, restricted-role dispatch/receipts and a real
  entrypoint probe cover the change; the independent reviewer found no material
  issue.
- Expo CLI aligned root `expo-constants` from 57.0.20 to 57.0.21, removing the
  second native copy introduced by the notification dependency. The pnpm lock
  contains one constants package/snapshot. Expo remains 57.0.26; unrelated
  dependency pins were preserved.
- The Chat notification helper `impl Store` moved before the `cfg(test)` module
  to satisfy all-target Clippy. Its methods and behavior did not change.
- The terminal-layout UI test now polls for the exact saved layout instead of
  asserting synchronously before the existing 400 ms autosave debounce. The
  expected layout, default timeout and all surrounding native/renderer checks
  remain unchanged. Typecheck and formatting passed; independent review found
  no material issue. Both remaining cases passed afterward. The separate initial
  PTY-start timeout did not recur in the final two-case run; no startup or
  admission timeout was changed.
- The native notification smoke runner originally used the same three-minute
  deadline for compilation and application execution. A cold compile exhausted
  it before the app launched. It now has a bounded fifteen-minute build phase
  followed by the original three-minute application phase. Assertions and
  process-group cleanup are unchanged; the reviewer accepted the change.
- The earlier iOS linker failure was traced to a Release React framework in a
  Debug build with a missing React-Core-prebuilt configuration marker. The
  official RN 0.86.3 replacement tool restored the matching Debug archive. Fresh
  Expo development prebuild and CocoaPods generation also selected the correct
  archive, with `RCTPackagerConnection` present. No generated native files were
  hand-edited and no SDK upgrade was made.

An initial disposable Playwright adapter did not reproduce the renderer driver's
timing/input behavior. The unchanged suite then passed using the actual pinned
agent-browser CLI. No assertion or application behavior was weakened.

## Device and environment prerequisites

The connected and unlocked iPhone 15 Pro Max is available to Xcode. It has an
existing production `dev.lomi.mobile` app; the test candidate uses the separate
`dev.lomi.mobile.dev` identity. No production app container, login credential or
Keychain entry was read or changed. No unexpired cached provisioning profile
matches the separate test identity; cloud provisioning is outside this test run.
There is no physical Android available.

The public staging Remote origin in source, `https://remote-staging.lomi.dev`,
does not resolve in DNS on this host (`curl` exit 6). Live/ready/status and push
route probes therefore could not reach a deployed service. The user clarified
that a staging environment is not yet confirmed. Actual Expo delivery requires
deployed API/worker, real project mapping, APNs/FCMv1 provisioning and signed
matching builds. A phone alone does not establish those prerequisites.

Local runtime versions are Bun 1.3.14, PostgreSQL 17.8 and Xcode 26.6 on macOS
27.0.1. Docker and the production Bun 1.4.2/PostgreSQL 18.6 versions are not
available here. Native Windows/Linux notification presentation remains outside
the available host. Managed Runtime and persisted Chat are the core sources;
ordinary CLI completion adapters remain disabled.

Temporary logs use `/tmp/lomi-notifications-retest-*`. The isolated PostgreSQL
cluster was stopped after the final backend test. Task-owned Metro, Xcode,
Gradle and recording processes are gone, and the fresh QA simulator was deleted.
Provider tickets, receipts, plugin
enqueue, native build success and visible OS alerts remain different evidence;
none implies exactly-once or a delivery SLA.
