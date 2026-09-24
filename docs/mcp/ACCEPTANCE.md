# Selected MCP acceptance record

Status: **VERIFIED — selected source scope, macOS ARM64** on 2026-09-24.
P0–P6 are complete for the explicitly selected scope. The actual-model gate has
20 distinct tasks with three complete passes each. P7, further theme/plugin MCP,
application lifecycle MCP and distribution remain NOT_SELECTED.
This record summarizes the detailed, dated evidence in
[QUALIFICATION.md](QUALIFICATION.md), including failed attempts.

## Scope and host

The selected delivery is the source implementation on macOS ARM64, with 74 MCP
tools. Further theme/plugin MCP work, application close/restart/update tools,
distribution, installers, releases and tags are NOT_SELECTED by the user.
Ordinary application close guards remain part of integrity testing. P7 and
strict routing are NOT_SELECTED. Windows, Linux and macOS Intel are unqualified.

Host: Apple M3, macOS 27.0 (26A428), native WKWebView, Bash and Zsh. Application
0.4.0, helper/protocol crates 0.1.0, control API 1.0, rmcp 3.4.0. The actual-model
profile is Codex CLI 0.156.1, gpt-6-sol, medium effort, prefer-Lomi. Tests use fresh
native app data and client sessions, with ordinary competing client tools still
available. No private client configuration was written and no paid Chat provider
request was made. Chat generations use the local test provider.

Android uses the licensed, isolated API 36 AOSP ARM64 revision 2 device,
720×1280, Host GPU, cold boot, 2 CPUs, 2560 MiB RAM and 6 GiB data. Shared ADB,
unrelated emulators and user shell profiles are not test fixtures.

## Product observations

VERIFIED in this table identifies the recorded observation on the selected host.
It does not replace the three-run routing gate below.

| ID  | State                     | Executed evidence and boundary                                                                                                                                                                                                                                           |
| --- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| E01 | VERIFIED                  | EFPfJF: actual Codex descendant of the origin PTY starts a separate Lomi execution PTY; native page/PNG and origin protection observed. Fixed-profile repetitions are recorded separately.                                                                               |
| E02 | VERIFIED                  | Native WKWebView action/snapshot/PNG in 1tJWXL; actual-model form QDQtUj, 0SEf7i and MF5HV5 each produced exactly one Unicode submission and normal cleanup.                                                                                                             |
| E03 | VERIFIED                  | Actual-model m7oy2s, KnKa7X and oKImyU launched external Playwright; native PID ancestry and PNG confirmed Chromium, and the final answer distinguished it from a Lomi browser panel. All recorded child PIDs exited.                                                    |
| E04 | VERIFIED                  | Actual-model SOE0wa, F9ocAD and 9QoRB6 built/imported/approved/installed/launched the fixture APK and submitted Unicode once. Actual guest UI, same device/generation and panel screenshot confirmed the result.                                                         |
| E05 | VERIFIED                  | Native retained layout in 1tJWXL, YWPPVZ and ErnuOt: PTY/browser/Android identities survive selected moves; Android zoom and shared source views survive transfer.                                                                                                       |
| E06 | VERIFIED                  | 1tJWXL editor-read.json rejects a stale revision. editor-edits.json records failed / none / REVISION_CONFLICT, unchanged buffer and a single undo restoring the original text. Existing UI save-race tests preserve later human edits.                                   |
| E07 | VERIFIED                  | KoF9rh: two real helper processes with identical claimed clientInfo require independent Settings approvals. Distinct workspace PTYs retain correct output despite focus on the other workspace; the second client continues after the first disconnects.                 |
| E08 | VERIFIED                  | ErnuOt opens two real Android views of one generation and preserves shared ownership on close. Native editor-open/read/edit evidence retains document identity and undo; the separate shared-buffer UI test covers simultaneous workspace aliases with mocked transport. |
| E09 | VERIFIED                  | KoF9rh running command, 6im2p3 browser POST/click and wYdLH1 partial real APK stream: disconnect records durable unknown effects, late ACK cannot fabricate success, and a newly approved helper cannot replay the old operation.                                        |
| E10 | VERIFIED, selected subset | wGLMu7, ErnuOt and 0ovWOt preserve dirty Cancel/Save/Discard, busy/origin guards, Chat checkpoints and last-view Android Stop. Application lifecycle MCP tools and installer/update qualification are NOT_SELECTED.                                                      |
| E11 | VERIFIED                  | Full native 1tJWXL revokes during a main-renderer hang; broker tests revoke while storage is locked. Native authorization blocks further input independently of the renderer/storage worker.                                                                             |
| E12 | VERIFIED                  | NHdHGE/TjYIpJ: two actual application processes, saved layout, fresh PTY/broker/retry epoch, one retained file effect and no command replay. Reconnect requires manual reapproval; no transparent resumption is claimed.                                                 |

## Security and integrity observations

| ID  | State                   | Executed evidence and boundary                                                                                                                                                                                                                                                          |
| --- | ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| S01 | VERIFIED                | Mutual TLS over private UDS, endpoint/handshake/replay tests; KoF9rh separately approves same-name helpers sharing endpoint credentials. Windows SID/named-pipe behavior is unqualified.                                                                                                |
| S02 | VERIFIED                | Core/broker and native domain suites reject foreign workspaces, operations, artifacts, references, cursors and stale leases; revocation also denies completed results.                                                                                                                  |
| S03 | VERIFIED                | Terminal scope/start-ticket tests and native origin/claim/focus tests deny implicit shell startup or input without terminal.execute and exact ownership.                                                                                                                                |
| S04 | VERIFIED                | Concurrent durable-key tests, expiry/recovery tests and E09/E12 actual process faults prevent duplicate dispatch and old-epoch replay. Input sequences deduplicate bytes.                                                                                                               |
| S05 | VERIFIED                | Exact one-use approvals, queued revoke, generation changes and forged/late ACK tests; E09 records uncertainty after real dispatch.                                                                                                                                                      |
| S06 | VERIFIED                | Native URL/redirect/navigation denials precede unauthorized navigation. networkIsolation remains none: origin grants are not an egress firewall or host sandbox.                                                                                                                        |
| S07 | VERIFIED                | Hv3cqb: ordinary browser and two project/client profiles have independent cookies, localStorage, caches and workers. KjoDju covers bounded same-origin frames and foreign-frame denial; screenshots require composite permission.                                                       |
| S08 | VERIFIED, host-specific | Descriptor-pinned file tests reject symlinks, hard links, FIFO/special files and altered identities. Native immutable APK/source-rewrite trials install only the approved copy. Windows reparse/ADS/UNC behavior is unqualified.                                                        |
| S09 | VERIFIED                | Bounded transport/framing, producers, queues, DOM/XML/images and approval queues; native main-hang and broker-storage-lock revoke remain responsive.                                                                                                                                    |
| S10 | VERIFIED                | Artifact reservations, pinned leases and concurrent quota tests; ciGimO actually exhausts a 32 MiB APFS fixture and prevents dispatch without a durable receipt. Native 5Gfkz6 also verifies actual guest-space refusal, immutable package preservation and complete space restoration. |
| S11 | VERIFIED                | Guarded Git reads deny helpers; all seven mutations require exact native approval and code-execution scope. Native hooks, remote/index changes, cancellation and rebase conflicts retain honest outcomes.                                                                               |
| S12 | VERIFIED                | Source classification across files/search/editor/Git/artifacts, private form values and bounded untrusted logs. No raw credentials, arbitrary scripts or active HTML log rendering.                                                                                                     |
| S13 | VERIFIED                | Real SIGKILL/WAL corruption, truncation, old backup, read-only store and schema tests preserve original data and invalidate prior authority; failed durable reservation blocks dispatch.                                                                                                |
| S14 | VERIFIED                | Protocol13/helper2/wire8 and native image checks: closed schemas, supported protocol versions, typed results, UTF-16 boundaries, base64 limits and explicit cursor expiry.                                                                                                              |
| S15 | VERIFIED, prefer-Lomi   | Pairing UI/tool results disclose that the broker uses the host account. Actual-model runs audit competing tools; no OS sandbox or strict-routing guarantee is claimed.                                                                                                                  |
| S16 | VERIFIED                | Real Bash/Zsh REPL/TUI/Unicode/binary output, prompt/human-input races and interrupts; generation-checked private ADB arguments and bounded hierarchy parser tests.                                                                                                                     |

## Performance

| Observation                            | Result                        | Scope                                                                                                                                   |
| -------------------------------------- | ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Warm status / panel list               | p95 2.805 / 0.957 ms          | VzCKYx; excludes model and startup                                                                                                      |
| Mutation admission                     | p95 10.522 ms                 | Excludes human approval                                                                                                                 |
| DOM snapshot / PNG                     | p95 17.065 / 195.574 ms       | Recorded fixture page and geometry                                                                                                      |
| Idle CPU                               | 0.855592% of one core         | App plus helper in the recorded idle phase; browser/emulator/model excluded                                                             |
| Broker and authenticated IPC idle      | 0.000115666% of one core      | Separate ten-minute probe                                                                                                               |
| Long run                               | 1805 s after 120 s warmup     | App RSS fell from 356 to 190 MiB; final ten minutes fluctuated with recurring falls, helper stable                                      |
| Terminal parser throughput             | Median 238.5 / 248 ms, +3.98% | fhyMJU; 20 alternating pairs, 2 MiB; same-build ordinary versus observed PTY, not an old-build or visible-paint comparison              |
| Android input to presented-image proxy | p95 136 ms, max 144 ms        | 1tJWXL; 50 measured plus 3 warmup samples; MCP input to second animation frame after changed guest pixels, not a photodiode measurement |

## Actual-model gate and known limitations

[routing-matrix.json](routing-matrix.json) records all 67 attempts in the declared
profile, including the seven failed attempts. All 20 distinct tasks have at
least three complete passes. A complete pass requires independent native
postconditions, normal host exit and cleanup, not only the model's answer.

| Observation                         | Recorded result                                                                                                             |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Complete passes                     | 60/67 attempts; 20 tasks × 3 complete passes                                                                                |
| Native task postconditions          | 64/67 attempts                                                                                                              |
| Completed model turns               | 67/67 attempts                                                                                                              |
| Selected Lomi                       | 67/67 attempts                                                                                                              |
| Competing actions / silent fallback | 0 / 0 observed                                                                                                              |
| Effective-profile audit             | All 67 match CLI, model, provider, effort, features, instruction sources, private-configuration write count and 74-tool set |

All final answers from the successful repeated trials were reviewed against
native results. Representative browser images and both final Android images
were visually inspected; this does not claim manual inspection of every repeated
image. The successful APK repetitions also cover normal installation after the
storage-refusal parser correction. Native 5Gfkz6 verifies real guest-storage
refusal and preservation of the previously installed package.

Seven earlier attempts remain failed. In particular, REPL 0KCGWi completed its
task but required exit 143; the cause of that fixture shutdown remains unresolved.
Fresh vFL2iy exited normally with bounded exit diagnostics armed, and the REPL
row has three complete passes. No claim is made that the old failure's root
cause was fixed. These historical failures are not counted as successful runs.

The source implementation uses bounded receipts/retry epochs (4096 each) and
refuses admission when the durable budget is full; it does not silently discard
deduplication history. [USAGE.md](USAGE.md) describes this operational limit.
The broker uses the host account, browser networkIsolation is none, and this is
a prefer-Lomi profile. Other host architectures/operating systems, strict routing,
installers, signing, updates and distribution are not qualified by this record.

The dependency audit recorded no known vulnerabilities at execution time, with
separate unmaintained-crate advisories, a Linux-only glib advisory and a low
Windows dev-server esbuild advisory. See the dated inventory in QUALIFICATION.md;
this is not a zero-advisory claim.

## Resources and reproduction

[USAGE.md](USAGE.md) describes source builds, native Settings enrollment, grants,
takeover and recovery. Source and binary fingerprints accompany routing batches.
Local native evidence uses the prefix
`/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-` followed by the
recorded run ID. Evidence is local temporary data; committed reports preserve
assertions and outcomes, not private app data or provider credentials.

The licensed SDK/AVD is retained for qualification at
`/tmp/lomi-android-stage0-mcp-20260923/native-managed-ca5c1615-3555-4833-a00f-e563569bf434`.
Individual finished profiles record normal app exit, private app-data removal
and, where used, stopped owned Android/private ADB or external browser processes.
Final reconciliation found no matching qualification app/helper/server/browser/
emulator processes and no listener on private ADB port 15047. All 60 successful
attempts have normal exit and absent private app-data directories. The licensed
SDK/AVD and its private fixture signing key are retained; logs, screenshots and
fixture projects remain as local temporary evidence. No shared emulator or ADB
was stopped. No installer, release or tag was created.
