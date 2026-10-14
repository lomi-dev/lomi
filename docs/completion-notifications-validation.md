# Completion notification validation

Local implementation and review completed on 2026-10-13. No live migration,
provisioning, deployment, publication, commit or push was performed.

The later [full retest receipt](completion-notifications-retest.md) records the
user-requested repeat, broader workspace tests and native build follow-up.

## Executed checks

| Area         | Command / evidence                                                                              | Result                                                               |
| ------------ | ----------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| Backend      | `bun --no-env-file run check`, build                                                            | Passed                                                               |
| Backend      | Full `bun --no-env-file test` with explicit isolated DB and restricted-role URLs                | 262 passed, 0 failed, 4,414 assertions                               |
| Database     | Fresh disposable PostgreSQL17.8 cluster; 12 migrations; real API/worker role probes             | Passed                                                               |
| Database     | Read-only inspector against isolated staging-named DB, exact migration/column/index/role guards | 7 passed                                                             |
| Mobile       | `pnpm lint`, `pnpm typecheck`, `pnpm config:check`, `pnpm release:check`                        | Passed                                                               |
| Mobile       | `pnpm test` with fake Expo/native adapters                                                      | 265 passed                                                           |
| Desktop      | `pnpm check`, `pnpm test`, `pnpm build`                                                         | Passed                                                               |
| Desktop Rust | `cargo test --manifest-path src-tauri/Cargo.toml --locked`                                      | 692 passed, 52 existing ignored                                      |
| Desktop Rust | Cargo check, fmt check, Clippy with `-D warnings`                                               | Passed                                                               |
| Desktop UI   | Full Playwright suite                                                                           | 733 passed, 12 failed, 1 skipped initially                           |
| Desktop UI   | Repeat the 12 failed scenarios with one worker                                                  | All 12 passed; total covered 745 passed, 1 skipped                   |
| Formatting   | Changed frontend/tests/docs Prettier check, all repository whitespace checks                    | Passed                                                               |
| Formatting   | Full `pnpm format:check`                                                                        | Fails on three untouched, pre-existing remote-terminal-runtime files |

The first combined backend run had four timing failures while iOS compilation
and the full UI suite loaded the host. Its final full run with unchanged backend
code and quieter host passed all 262 tests with the normal test timeout. The
capacity cases also completed below one second individually. No server timeout,
authorization rule or test contract was weakened.

The initial default UI fixture build rejected an existing unowned generated
output. It was preserved, the fixture was built into an empty directory for the
suite, and the original generated artifacts were restored afterward. No fixture
source or ownership marker was forged to bypass the helper.

Full formatting reports only these unchanged files:
`packages/remote-terminal-runtime/src/index.mjs`,
`packages/remote-terminal-runtime/src/model.mjs`, and
`packages/remote-terminal-runtime/tests/ipc.test.mjs`. They were kept outside the
notification change.

## Meaningful coverage

| Requirement                                                                  | Evidence                                                                                |
| ---------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Result and intent share transaction; rollback/retry/dedup                    | Runtime/Chat store tests and notification ledger tests                                  |
| Managed settlement, Continue correlation, failed/cancelled/history exclusion | Existing runtime suites plus successful/settled checkpoint qualification tests          |
| Chat final-save retry without UI subscriber                                  | Checkpoint rollback/retry and retained backend/store tests                              |
| Independent four channel combinations                                        | Separate admission/ledger tests, Settings local-only test with cloud OFF                |
| Same restored Session after process generation reset; new Session exclusion  | Durable owner/process tests and retirement/consent fences                               |
| Local OFF/logout before versus after enqueue; lost claim and late ACK        | Production claim/enqueue helper with real SQLite CAS and injected plugin callback       |
| Checking grace, focus-read failure, cloud hang independence                  | Native dispatcher policy/grace and separate-consumer regressions                        |
| HTTP bounded errors/origin, pre/post-request owner fencing                   | Fake HTTP tests for status/code/Retry-After, redirects and account change               |
| Known denial while Offline; renewed credential/new owner preservation        | Delayed HTTP denial auth tests                                                          |
| Multi-device snapshot and nonretroactive consent/source/receive epochs       | Real PostgreSQL ingestion and delivery tests                                            |
| Rebind after actual logout; unknown first register ACK                       | Backend session-revocation trigger plus proof reconciliation and mobile lifecycle tests |
| Expired access versus revoked MobileSession/dependencies                     | Worker owner tests without short-token requirement                                      |
| Token rotation versus old DeviceNotRegistered or receipt timeout             | Attempt snapshot/sequence CAS regressions                                               |
| Unknown send, lease crash, per-item error, provider cooldown/circuit         | Fake Expo integration and two-worker shared429 tests                                    |
| Minimal worker role and required grants                                      | Actual SET ROLE probes deny auth tokens, credential hashes, identity accounts and DDL   |
| Cold/warm response dedup, bindingId reinstall fence, no Control              | Mobile service/intent tests and protected informational route                           |
| Concurrent receiver OFF and stale desktop refresh                            | Registration409 reread and reversed native refresh result tests                         |

Independent reviewer passes examined the actual three-repository diff and
regressions. All material findings were fixed, including DTO mismatch, history
bounds, cross-account reconciliation, late receipt fencing, shared provider
backoff, independent local emission, refresh ordering and worker progress.
Final auth and mobile deltas received separate read-only rechecks with no
remaining material finding.

## Native and external acceptance still unqualified

- iOS native preparation succeeded, including ExpoNotifications, local crypto and
  privacy modules. The local Debug binary failed final arm64 linking in Expo Dev
  Launcher: undefined `_OBJC_CLASS_$_RCTPackagerConnection`, xcodebuild code65.
  No app was installed. The SDK or unrelated dependency pins were not upgraded.
- The new isolated iOS simulator failed boot with code60: `launchd_sim` failed to
  bind/respond. It was removed and task-owned QA processes cleaned up. Existing
  simulator credentials were not used. Native Account UI/motion validation was
  therefore not completed.
- Physical iPhone and Android push, signed staging/release binary taps, Android
  force-stop, APNs/FCMv1 provisioning and Expo enhanced-security delivery: not run;
  no credentials/provisioning/device acceptance was authorized or supplied.
- Local OS banners in signed packaged macOS, installed Windows and Linux with a
  notification service: not run. Rust plugin enqueue and mocked focus tests do
  not establish OS display. No desktop click/action guarantee is made.
- Docker and production PostgreSQL18.6/Bun1.4.2: not run on this host. Local
  backend qualification used PostgreSQL17.8 and Bun1.3.14.
- Ordinary CLI completion integration remains disabled. Codex0.160.0/0.162.1
  structured hook feasibility was source-verified; real per-launch authenticated
  adapter/setup/approval/cancel/duplicate qualification was not performed. Claude
  2.1.287 lacks a verified stable start/completion turn join.

Managed Agent Runtime and persisted Chat are the implemented core sources.
The notification screen currently stays desktop-only because neither source has
an authorized Remote result projection. There is no terminal/Control fallback.
Expo tickets/receipts and Tauri plugin success remain best-effort pipeline
evidence, not exactly-once or a delivery SLA.
