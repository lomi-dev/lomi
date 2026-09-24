# MCP implementation status

Updated: 2026-09-24. Scope: PROMPT.md in the sibling lomi-mpc-docs directory,
as narrowed explicitly by the user on 2026-09-23. The current delivery excludes
further theme/plugin MCP work, application close/restart/update MCP tools, and
distribution/installers/releases. Completed work remains recorded below; its
historical TODO lists do not override this scope change. Chat AI, Android
management, advanced browser/terminal work, remaining layout integration and
functional/safety/performance tests remain included. P7 is NOT_SELECTED.

Delivery: commit and push each completed module to origin/main, then push the
final remaining changes and explicitly report completion. No tags, GitHub releases,
installer publication or private client-configuration writes.

## Baseline

- App 0.4.0, initial HEAD `7d4b9ac5fbe94889b09b5a527055defd08047cca`; baseline follow-up HEAD `4403883ebe583188f79dfe8d7886d2422e49bfcd` (external desktop icon change, preserved).
- Pre-existing modification: `src-tauri/Cargo.lock` (relocation of the lomi package entry; no version changes). Preserve it.
- macOS 27.0 (26A428), Apple M3, ARM64; Rust/Cargo 1.98.1, Node 22.22.3, pnpm 11.25.0, Codex CLI 0.155.1.
- Initial free disk: approximately 18 GiB. Reuse the existing Cargo target.
- Read repository AGENTS.md and all 18 source documents plus PROMPT.md. No nested AGENTS.md found.
- Existing native tests are baseline evidence only; they do not qualify new MCP adapters.

## Milestones

| Stage | State        | Acceptance                                                                                                |
| ----- | ------------ | --------------------------------------------------------------------------------------------------------- |
| P0    | VERIFIED     | Selected SDK/client, native WKWebView, Bash/Zsh PTY, managed Android and authenticated IPC on macOS ARM64 |
| P1    | VERIFIED     | Authenticated broker, exact grants, durable receipts, scoped UI bridge and native revoke                  |
| P2    | VERIFIED     | Workspace, retained PTY and native browser end-to-end; client routing repetitions tracked under P6        |
| P3    | VERIFIED     | Native browser actions, observations, screenshots, immutable artifacts and profile isolation              |
| P4    | VERIFIED     | Selected managed Android build/import/install/launch/input path, protected generations and cleanup        |
| P5.1  | VERIFIED     | Scoped disk/editor operations, shared buffers, conflicts, previews and real full-disk preservation        |
| P5.2  | VERIFIED     | Guarded Git observations/views and all seven typed mutations with exact native approval                   |
| P5.3  | VERIFIED     | Selected project/workspace/panel lifecycle, mixed layout and Chat/Android descendants                     |
| P5.4  | VERIFIED     | Selected Settings open/read/editor/terminal/shortcut writes; further themes/plugins NOT_SELECTED          |
| P5.5  | VERIFIED     | Android setup, device management and protected recovery on the qualified host                             |
| P5.6  | VERIFIED     | Seven Chat AI tools and retained layout; local provider fixture, no paid request                          |
| P5.7  | VERIFIED     | Advanced browser/PTY; application close/restart/update tools NOT_SELECTED                                 |
| P6    | IN_PROGRESS  | Functional/safety/performance evidence recorded; finish actual-model repetitions and final reconciliation |
| P7    | NOT_SELECTED | No optional extension selected                                                                            |

VERIFIED applies only to the selected macOS ARM64 contracts and their documented
limits. It does not qualify another operating system, a strict routing profile,
or distribution. See REQUIREMENTS.md for all 74 tools and QUALIFICATION.md for
executed evidence, retained failures and per-profile cleanup.

## Current work

Native dual-client isolation, real application restart, in-flight command/click/
APK disconnects and product browser-profile separation have passed with normal
cleanup. Their artifacts and scope are recorded in QUALIFICATION.md. The focused
ADB guard regression and workspace all-target mcp-probe Clippy also passed.
The fixed-profile actual-model matrix remains IN_PROGRESS: 58 complete passes
and 19 complete three-run rows are recorded. Source-frozen batch p0GALB finished
20/20 with normal cleanup. All 65 recorded attempts match the effective profile.
The final two APK repetitions are running with the isolated Android fixture;
application/helper/test sources remain frozen until that batch ends. The complete
acceptance record is in ACCEPTANCE.md; final validation, documentation
reconciliation and the final push remain pending.

Stale Android layout descriptions and the native pre-session storage-refusal
parser are fixed and verified in dcaaa703bee9c6892e04b8ba13e8279be6806e83 on
origin/main. Native 5Gfkz6, two parser regressions, eight wire tests, workspace
all-target mcp-probe Clippy, formatting/syntax, catalog and link checks passed.

## Milestone history

These dated observations retain prior failures and then-current next steps.
They do not replace the current work or the explicitly narrowed delivery scope.

Native in-flight browser click and APK transfer disconnects passed at 6im2p3
and wYdLH1, completing the run/install/click observations of E09. Both preserve
durable uncertainty and reject replay by a newly approved helper. Cleanup
passed, including the isolated Android guest/private ADB. The fresh REPL run
vFL2iy passed normally, bringing routing to 26/60 complete passes and five
complete rows. A product-level two-project browser storage isolation probe is
running to strengthen the earlier engine/profile evidence. Final checks and
the remaining routing repetitions are still required.

Native dual-client isolation E07 and terminal application restart E12 passed
with normal cleanup (KoF9rh, NHdHGE/TjYIpJ). Disconnecting a helper inside a
running native command preserved its sole side effect and retained terminal,
recorded durable uncertainty, and left the other client functional. Browser
click/APK install disconnect trials remain required. Fixed-profile routing is
25/60 complete passes across 20 tasks; a fresh REPL trial is diagnosing the
previous fixture-exit failure. No full-system completion is claimed.

File mutation now identifies an invalid domain revision as REVISION_CONFLICT
and describes its decimal workspace source in the tool/schema. Actual-model
rename trials passed 3/3, including recovery from the originally mistaken hash.
Core/broker/protocol/helper and independent wire checks passed in the final
recorded run; all-target probe Clippy passed. Four routing rows have complete
3/3 passes. REPL has three successful task observations, but the third fixture
exit required termination and remains a separate cleanup failure. The exact
cause will be diagnosed before counting that whole trial as passed.

Manual reconnect N8154I passed two fresh actual-client sessions with a single
preserved file effect, distinct approvals and denial of the old receipt.
The expanded batch additionally passed terminal interrupt/binary output,
Unicode contenteditable, same-panel navigation and Git review. REPL needs a
fixture expected-tool correction; file rename exposed a misleading revision
error and needs a product correction. See QUALIFICATION.md and routing-matrix.json.
Twenty distinct task fixtures are now defined; three matrix rows have all
three complete passes. Dual native clients, in-flight disconnect and native
application restart acceptance remain under active audit.

Expanded routing trials now pass terminal-exit, files-search, browser-select
and external-playwright with independent native postconditions. Scope-denied
has three fixed-profile passes (Nh2EEe, sL1uSU, oCvt22), making three complete
rows of the required 20×3 matrix. The preferences and external-browser cleanup
failures were fixture commands left active at the existing busy-close guard;
verified fixture-only Ctrl+C after model observation now permits normal exit.
Final browser-select nAUFba and external-playwright m7oy2s exited 0, with all
recorded external processes absent. Both screenshots and model explanations
were reviewed. See QUALIFICATION.md for failed attempts and corrective evidence.
At that milestone nineteen task fixtures were defined.
Unexecuted definitions and historical failures do not count as matrix passes.

Throughput milestone `a021adc192a6c797967fc4de6775f417656fd28e` was
pushed to origin/main and verified. Fixed-profile unavailable-client routing
passed 3/3 (pMc8pe, ccgHM8, Pm72d8), with explicitly reported unavailability,
zero competing actions and normal cleanup. Exact-binary reuse with fresh
app/client sessions was exercised in the latter two runs. Two of 20 matrix
tasks now have all three executions; remaining tasks and final acceptance
reconciliation are still open.

Two-workspace milestone `877e3ec28aa9ad4af80374c9fc635bb5aa192d30` was
pushed to origin/main and verified. The subsequent terminal throughput run
fhyMJU passed 20 alternating measured pairs after three paired warmups:
ordinary median 238.5 ms, MCP-observed median 248 ms (+3.98%, threshold 10%).
Both parsed the identical 2 MiB payload through retained real PTYs/xterm and
returned to the shell. Cleanup passed. The complete fixed-profile model
matrix and final acceptance reconciliation remain open.

Android latency milestone `7091bcbeb957a99e40813346f1591442d6ccc9e3`
was pushed to origin/main and verified. Two-workspace actual-model routing
then passed 3/3 (u7KQSR, IemlCy, YitjKG), with gpt-6-sol and medium effort
pinned, independently verified retained PTYs/outputs/selection, no competing
actions and complete cleanup. This is one completed row of the 20×3 matrix.
Terminal throughput measurement is now being qualified separately.

Android/import milestone `42ed05a7f1912e9df1a5a0cf3269e7024b6b1e66` was
pushed to origin/main and verified remotely. The subsequent full native run
1tJWXL passed with 74 tools, 91 primary checks and required Android input.
Its 50-sample input-to-presentation proxy measured p95 136 ms, maximum 144 ms
against the 150 ms budget. Host, guest and private ADB cleanup passed.
See QUALIFICATION.md for the endpoint, raw evidence and measurement limits.
The fixed-profile model matrix, terminal throughput and final acceptance
reconciliation remain open. No distribution or tag is planned.

The WAL recovery fix was pushed to origin/main as
`c73058cd557ca424cbff1c4a2fab04eed563041c` and verified remotely.
The final storage-fault audit found SQLite silently discarding a corrupted
committed WAL suffix. A bounded read-only checksum preflight now refuses startup
and preserves the original DB/WAL. The reproducer, read-only storage, valid WAL
reuse and the full core/broker suites pass (68 + 68). See QUALIFICATION.md for
the actual failure and correction. Actual-model Android qualification is being
rerun after fixing XML entity decoding in its independent native oracle;
the first trace showed successful installation and one Unicode submission,
but that failed oracle is not recorded as a complete pass. Final APK trial
SOE0wa passed with 39 actual-model MCP calls, exact build/import/install/launch,
one Unicode submission, matching native device/panel/generation, screenshot
and graceful cleanup. The intervening mePKbt trial exposed misleading import
revision feedback; the contract/error and reproducing test were corrected.
WFDYwx timed out with configured xhigh effort. The runner now pins and records
medium for subsequent routing trials. All-target workspace probe Clippy and
all 8 helper-wire tests passed. See QUALIFICATION.md for every attempt.

Actual Codex routing milestone `42dfa9411676fba1a2e798b6614db881f9d26e75`
was pushed to origin/main and verified. E01 subsequently passed in EFPfJF:
Codex ran inside a real ordinary Lomi PTY, created a distinct execution PTY,
opened/captured its server in WKWebView, preserved the origin session and
returned exit 0 to it. Process ancestry and native resource IDs were checked
independently; no competing action occurred; host cleanup passed.
The required 20-task × 3-run prefer-Lomi routing matrix remains incomplete,
including APK, two-workspace and reconnect categories. P6 is still open.

P6 recovery and regression milestone was pushed to origin/main as
`6ca72bf4cf71e47d0ab64ac9f990abb04d5b8b1b`; the dependency audit followed as
`6b6330df50d970f3a34764f7b5bd6d09ce338f54`. Both remote SHAs were verified.
Restoring an older
receipt database could previously label a queued row cancelled/none despite a
later completed effect. Startup now conservatively marks every unfinished row
outcome_unknown/unknown; a fresh instance still invalidates all old retry epochs.
The reproducing restore test and corrupt-file preservation test pass. A real
32 MiB APFS ENOSPC run now covers SQLite reservation failure, no partial receipt,
preserved history, invalid old retry and recovery after freeing space, alongside
atomic save/Trash preservation. Final artifacts: lomi-mcp-full-disk-ciGimO.

Workspace Rust: 377 passed, 23 opt-in entries ignored before the recovery change;
updated core 64 + broker 68 passed afterward. Model 179 + AI 17, wire 8,
TypeScript and all-target workspace Clippy passed. WebKit full regression found
stale Lomi-theme expectations, a terminal-start race and browser-specific focus
assumptions. Those test fixtures were corrected without product UI changes. The
second full run passed 499, skipped 1 and exposed two remaining test issues;
the final complete terminal-title/agent-git files passed all 22 after correction.
No unresolved assertion from either full run remains. This is combined evidence,
not a claim that the final tree ran all UI cases in one pass. The dependency
audit is recorded in QUALIFICATION.md, including remaining advisory warnings.
P6 performance, full native regression and model-driven routing remain open.

The isolated production broker completed a 600-second authenticated idle
connection test: 0.000116% of one CPU core, below the 1% budget, with the same
connection still usable afterward. The separate native 30-minute run
`lomi-mcp-control-VzCKYx` passed: status/panels/admission/snapshot/PNG p95
2.805/0.957/10.522/17.065/195.574 ms; normal exit 0 and private app-data
cleanup confirmed. Native app/helper RSS showed no sustained growth in the
recorded run; its exact scope excludes renderer/emulator/model processes.
See QUALIFICATION.md for samples, budgets, the earlier failed short diagnostic
and remaining throughput/Android-latency checks. This milestone was pushed as
`6e3ffca9b29dfd522af9b8d670fd063490c36eca`; remote main verified.
Full native regression `lomi-mcp-control-ifJdyT` passed: catalog 74, 91 named
primary checks, Android input RUN, normal host exit 0, private app-data removal,
guest/private ADB stopped without forced termination. Log:
`/tmp/lomi-mcp-p6-native-full.log`. Native screenshot inspected.
`tests/mcp/codex-routing.mjs` now has initial actual-model evidence;
the unavailable-app case passed in three fresh sessions at
`/tmp/lomi-mcp-routing-unavailable-{shg1rw,LTw9Ck,Ybvnln}`:
Codex 0.156.1 / gpt-6-sol
selected lomi_status, observed app_unavailable and stopped without competing
actions. Shell/unified execution remained enabled; hooks/apps/delegation were
disabled. Native development-server case G1u0SB passed with independently
verified PTY/WKWebView/PNG and clean teardown. Its predecessor QB2GgW failed
because Codex's client approval policy blocked terminal creation before Lomi;
case-specific subprocess approval overrides corrected the fixture. Form case
dzL2Kq passed with one exact Unicode POST, native DOM/PNG and normal cleanup;
the first form attempt OndJrH omitted terminal-create client approval and failed
before dispatch. Fix-test crfwhe passed: Lomi-observed failure, editor change,
unchanged tests, successful rerun and independent native Node exit 0; no
competing action and clean teardown. Scope-denied IFyupE passed with an actual
terminal-create SCOPE_DENIED, an explicit model refusal and no competing action.
Final probe Clippy passed, including the exact-tool refusal check
(`/tmp/lomi-mcp-routing-clippy-final2.log`). Rust/JS syntax and formatting passed.
No native host or model trial remains running. Source is unfrozen.
The complete origin-terminal, APK, two-workspace and reconnect model scenarios
remain open. These initial categories are not reliable-routing qualification.
The intended
profile is prefer-Lomi, with private client configuration unchanged and hooks,
notifications, other MCP servers, apps and delegation disabled for the subprocess.

Browser upload was committed and pushed to origin/main as
`da9a26e849a682432a8ac945840c115ccb18c5dd`; remote SHA verified.
Terminal qualification is complete for the selected macOS ARM64 host:
**uw9501 Bash PASS** and **Gn0P6D Zsh PASS**, 13 checks each, catalog 74,
normal host/wrapper exit 0 and private app-data removal. Settings selects a
single Bash/Zsh profile per connection, pinned through native start/attachment.
The native fixture confirms real Node REPL, Vim, exact Unicode file content,
rendered alternate-screen text, binary output, a 2 MiB flood with bounded gaps,
xterm ACKs, quiet commands, exit status, targeted interrupt, input/run retries,
EOF uncertainty, manual takeover and Settings-approved reclaim.

Two reproduced product defects were fixed: missing preexec on macOS Bash 3.2,
and prompt redraw restoring readiness over a partial line. Bash preserves
user prompt hooks and DEBUG traps; a conflicting debugger/trap omits automatic
readiness. Both explicit input and existing-PTY attachment now require a fresh
completion boundary before automatic run. Partial text remains protected after
resize and reclaim. Native Settings and Vim screenshots were visually inspected.

Core 62, broker 68, native shell/PTY 9, helper wire 8, WebKit approval 1,
TypeScript, SDK/AI checks, frontend build and production workspace all-target
check passed. All-target MCP and final native-probe Clippy passed with warnings
denied after the screenshot timing correction. No native fixture remains
running. Full MCP remains incomplete: continue P6 system-wide acceptance,
fault/recovery, performance, dependency review and actual client routing.

Android management was committed and pushed to origin/main as
`464ed18da59e5786c3b348e38783d9cc7feec732`; remote SHA verified.

Android layout passed native qualification on macOS ARM64: **ErnuOt PASS**,
catalog71,14 checks, normal host/wrapper exit0 and isolated app-data cleanup.
Focus/docking/moves/transfer retain the runtime, native generation, PTY and zoom.
An admitted workspace transfer extends only the existing owner's binding to the
granted destination, preserves shared source views and releases input leases.
Panel/workspace/project last-view close passes editor guards, confirms Stop,
and retains a native start barrier until the receipt settles. Slow native cleanup
runs outside the broker mutex; a separate completion flag rejects early close ACKs.
Native **0ovWOt Chat54 PASS**, catalog71, normal exit0/app-data cleanup, confirms
the shared closure refactor preserves existing Chat behavior. No paid provider calls.

Core3/full broker64, existing WebKit workspace/Chat17, model179 + AI17,
TypeScript, frontend build and wire5 PASS. Native Android76 PASS (17 opt-in
tests ignored). Production workspace, all-target MCP packages and final MCP-probe
Clippy PASS with warnings denied. Native project/browser/PTY close regression
**wGLMu7 PASS**, catalog71, normal exit0 and isolated app-data cleanup.
The extra application all-target Clippy check reports nine pre-existing test-only
lints in android/input.rs, chat/preferences.rs, chat/store.rs, files/editor.rs,
themes.rs and updater.rs; those files are unchanged from HEAD. These remain for
P6 cleanup, not an Android regression. Source is unfrozen; no native app or
emulator remains running.
The mixed Android/PTY and dirty-close screenshots were inspected. First attempts
bMi7lr and K0tPJi failed fixture assertions (percentage zoom and map-shaped
terminal contexts), were corrected, and each cleaned up normally. Their partial
results are not counted as passing runs.

Android layout was committed and pushed to origin/main as
`b7f014874f098a05d85b356acf3679a1f96b9eca`; local and remote SHA verified.

Browser same-origin frame support is natively qualified on macOS ARM64:
**KjoDju PASS**, catalog71,13 checks, normal host/wrapper exit0 and private
app-data removal (`/tmp/lomi-mcp-browser-frames-native3.log`). Nested Unicode
fill/key/click, exact retry, child replacement, detachment, parent occlusion and
frame scroll passed through the actual stdio helper and WKContentWorld. Page-world
constructor/global replacement cannot alter the isolated frame dispatcher.
Snapshots bind bounded frame identities/origins/viewports and element references.
Scroll accepts optional viewportRef (default remains the main viewport).
Cross-origin, opaque/srcdoc, hidden and over-budget frames are explicitly omitted;
transformed ancestor input is refused. WebKit4, core59 + broker64, TypeScript,
wire5 and all-target MCP Clippy passed. Final protocol compatibility test1 passed;
MCP-probe and final all-target MCP Clippy passed with warnings denied. The nested-frame screenshot was inspected.
Uy20vL exposed the missing scroll DTO field; Cjx0vi then passed interactions but
failed fixture cleanup because its close request lacked browserGeneration. Both
were corrected and both exited0 with private app data removed; neither is counted
as a passing run. No native app remains running.

Frames were committed and pushed to origin/main as
`4c95a73226a67186f10a0d6bba426781193dd754`; local and remote SHA verified.

Expanded browser logs are natively qualified on macOS ARM64: **a6K0Te PASS**,
catalog71,11 checks, normal host/wrapper exit0 and isolated app-data removal.
The existing logs tool now selects errors, console or Promise reports, with
independent bounded rings and kind-bound cursors. Console preserves five levels
and primitive Unicode; objects are omitted without added coercion. Page-global
and intrinsic tampering, overflow/gaps, navigation expiry and foreign-workspace
denial passed. WebKit marks genuine unhandled Promise reports isTrusted=false;
the result preserves that flag and explicitly includes synthetic reports.
All messages remain untrusted page content. No new authority enters page JavaScript.
Collector3, actual WebKit rejection1, frame regression4, permission UI1, wire8,
TypeScript, broker64, all-target MCP Clippy and MCP-probe Clippy passed.
Native earlier DgBgy6/LWDj7U/r1BPLE failed fixture/trust assumptions and do not count
as passes; the qualification record explains their corrected expectations.

Expanded logs were committed and pushed to origin/main as
`084a439c5a8cb1a47d1bed73f4e543ce44e510a0`; local and remote SHA verified.

Generic file import and controlled artifact export are natively qualified on
macOS ARM64: **lVi0Ip PASS**, catalog72,12 checks, normal host/wrapper exit0 and
private app-data removal (`/tmp/lomi-mcp-artifact-native.log`). Binary Unicode/
CRLF, empty files, opaque .apk classification, source rewrites, import/export
retries, changed payloads, collision/symlink preservation, stale parents,
foreign workspaces, secret paths and oversize refusal passed. Source is unfrozen.
Both the WebKit and native Settings permission screenshots were inspected.

Generic imports use an explicit new scope; schema5 retains earlier image/APK
records. Export publishes one complete new file under the native writer lock,
preserving original source authority and checking its hash/parent identity.
Core60 (one opt-in ignored), broker65, helper wire8, WebKit permission1,
TypeScript, production workspace check, all-target MCP and MCP-probe Clippy PASS.
Final broker tests add a full4MiB round trip and captured Android PNG export,
including source revocation denial for new exports and stored receipts.
Final broker65 and all-target MCP Clippy PASS; see QUALIFICATION.md for evidence.

File artifacts were committed and pushed to origin/main as
`23422ddc9b70e646b344756f907a18fbca3521a9`; local and remote SHA verified.

Bounded browser download is natively qualified on macOS ARM64: **PvUmgs PASS**,
catalog73,12 checks, normal host/wrapper exit0 and private app-data removal.
Same-origin cookies, opaque binary/empty/full4MiB bytes, exact hashes and project
exports passed. Page-world fetch replacement did not alter private-world bytes.
Redirect, foreign-origin/workspace, stale-document and oversize requests were
refused; dispatched failures report uncertainty. Cancellation, exact retry and
source closure passed. Every server route was requested once, including retries;
the unapproved server received zero requests. The native permission screenshot
was inspected. Core60, broker66, wire8, WebKit2, TypeScript, workspace check,
all-target MCP and MCP-probe Clippy PASS. Opt-in core/full-disk tests remain ignored
in this run, with their separate evidence retained. No native fixture remains running.

Downloads were committed and pushed to origin/main as
`df810b7f26e5f8822e7466bf1c9bd5d575cb73fd`; local and remote SHA verified.

Approved browser upload is natively qualified on macOS ARM64: **vuDLZZ PASS**,
catalog74,12 checks, normal host/wrapper exit0 and isolated app-data removal.
Native Settings approval binds the actual same-origin document/frame/input and
an immutable artifact's filename/size/hash. Actual main/child file inputs and
the test server received binary, empty, Unicode and full4MiB copies; exactly four
POSTs matched their hashes. Source rewrites, page-world File/DataTransfer
replacement, retry, denial, MCP cancel, replaced input and closed destination
were exercised. Denied/cancelled/stale requests produced no extra POST.
Permission and exact-approval screenshots were inspected. Core60/broker67,
wire8, WebKit adapter/frame6 + Settings2, TypeScript and frontend build PASS.
The first core receipt-revocation assertion found and fixed a missing live
destination check; the complete broker67 suite passes after the fix.
Native9eaJeL andYINphg failed test-harness readiness/waiting assumptions, each
with normal cleanup; neither is a full qualification pass.
Final helper wire8, all-target MCP and MCP-probe Clippy PASS with warnings denied;
formatting/diff checks passed. No native fixture remains running.

Next: Bash/Zsh terminal qualification and P6. Full MCP is incomplete.

Chat AI milestone was committed and pushed to origin/main as
`57b05cb46f8ed1896f2b26c3665826a401a96073`; remote SHA verified and the
worktree was clean before Android management work started.

Android management is natively QUALIFIED on macOS ARM64 (catalog71): setup
inventory/catalog/prepare, exact plan apply, create/modify/wipe/delete,
interrupted-operation recovery, metadata recovery, owned-cache cleanup and
package removal/rollback. Native **7dOf1P PASS**, 23 checks, includes actual
Platform-Tools37.0.1 install/reinstall, AVD create/modify/wipe/delete, tool
rollback/removal, malformed preference recovery and typed reset. Original
qualification device and preference-file baseline were preserved. Host/wrapper
exit0 and isolated app-data cleanup confirmed; the approval screenshot was
inspected. Earlier OzbXGl also passed19 before extending maintenance coverage.

Every mutation requires native Settings approval; license acceptance and typed
confirmation are not MCP inputs. The agent supplies the target's exact name for
wipe/delete, and Settings independently requires typing it. Plans bind native
metadata/runtime identities and revisions. Device metadata cannot reset: it
requires a valid backup, matching the existing recovery policy. Cancellation
and workspace removal invalidate only the operation's permit. Full broker61,
Android units75 (17 opt-in tests ignored), WebKit approval/permission2,
TypeScript, frontend build, workspace/probe Clippy and helper wire5 passed. A
subsequent error-mapping-only change passed native3, including actual disk-space
and mutation-gate errors. Android management was pushed as the milestone recorded above.
Android layout is qualified separately in the current-work record above.

### Completed Chat module

Streaming renderer repair was committed and pushed to origin/main as
`12af2935eed70f12563c98b03091b71df37119f8`; remote SHA verified. A trailing
message-view task and memoized unchanged Markdown prevent per-token rendering
from starving native acknowledgements. New model3, WebKit Chat UI9 and
TypeScript PASS. Native **gVdzJU PASS**, 47 checks, catalog68, two local fixture
generations, normal exit and isolated app-data cleanup. The mixed-layout
screenshot was inspected. This includes Chat docking, pane movement, tab reorder,
open/focus of an existing mixed view and round-trip workspace transfer without
replacing the SDK or PTY. Earlier native attempts DcRj84/68CDUU exposed fixture
snapshot/result-size problems; Ed6xlT/bvy4Sf/SIBYbP exposed rendering starvation.
Their results were not counted as passing. Bounded browser tests also corrected
an unintended unlimited long-text producer; WebKit and Chromium then passed.
Speculative listener changes and temporary native renderer diagnostics were removed.

Chat AI is implemented and natively qualified: all seven tools and selected
standalone/mixed layout integration, including panel/workspace/project closure.
Native **iyLq7U PASS**, 54 checks, catalog68, five local fixture generations;
normal host/wrapper exit and isolated app-data cleanup. Existing editor guards
precede draft flush and native stop; last-view detection preserves shared
conversations. A native admission barrier lasts until the broker settles the
layout operation, preventing a replacement response during close. Native
checkpoints precede PTY/browser cleanup; failed saves preserve views. A failed
flush has an uncertain effect receipt, and exact retry never repeats cancellation.
The preceding qVY6Fd closing screenshot was inspected. Full broker59, native
Chat34, frontend model179 + AI17, TypeScript, production workspace check,
WebKit UI14, Clippy with warnings denied, helper wire5 and frontend build PASS.
This milestone includes all selected Chat work; Android management is next.

### Earlier increment records

These records describe intermediate states; the qualified Chat result above
supersedes their incomplete-layout and uncommitted-module notes.

Broker revocation repair was committed and pushed to origin/main as
`73bb60f622575c3bf100b42c0fac52f15ea78259`; the remote SHA was verified. Only
that repair and its regression test were staged. A detached worktree based on
the previous commit tested the exact staged bytes: broker50 PASS. The temporary
worktree was removed after verifying its contents against the new commit.
Chat work remains uncommitted.

Chat open/create is now implemented (catalog64): separate chat.open/create
scopes, exact existing-history grants, native SQLite creation, creator-only read
access, durable retry and native one-use preparation. Internal panel projection
binds each chat view to its conversation ID. The bridge reuses a standalone
view and retained SDK runtime; mixed-layout reveal remains explicitly refused
until sibling lifecycle qualification. Core open1, model2, TypeScript and wire5
PASS. Initial probe compilation needed an explicit fixture Vec<Value> type;
that is corrected. Native **ygmgFM PASS**, 17 checks, catalog64, native and
wrapper exit0, private app-data removed. Native create/reuse/retry preserves the
retained SDK and human draft. Screenshot inspected; probe Clippy PASS.

Chat draft is implemented (catalog65): separate chat.draft permission, exact open
panel/conversation binding, 32 KiB text, conversation/draft/domain revisions,
native draft CAS and durable one-use operation results. The retained runtime
rejects unsaved/recovery text and preserves human typing during native ACK delay,
then saves it against the new draft revision. UI4, TypeScript, native-store3 and
core draft1 PASS. Full broker54 and wire5 PASS. Native **dtcU69 PASS**,
23 checks, catalog65, host/native/wrapper exit0 and private app-data removed.
CswH2K/IlFJDY exposed an ineffective fixture interception of immutable Tauri
invoke; the retained-runtime commit callback now delays the real native ACK.
No production change was needed for that fixture issue. Settings screenshot
inspected. Probe Clippy PASS after boxing the draft receipt result (wire JSON
unchanged); targeted broker1 and wire5 repeated PASS.

Send is implemented through native context preparation, broker, exact UI approval,
retained runtime and helper. Native **3w7ndI PASS**, 31 checks, catalog66, one
local fixture generation, exit0 and private app-data removed. This includes
read/open/draft regressions, decline, native context mutation after preview,
reserved identity, exact replay, retained hidden SDK streaming, next human draft
and ordinary native cancellation. BNLJwM was an AwaitingUser polling race in the
fixture, corrected by awaiting dialog dismissal. Approval screenshot inspected;
stream screenshot showed the other active tab, so no visual stream evidence is
claimed from it. UI regression14, broker55, native chat30 and TypeScript PASS.
Later core regressions fixed lost approval settlement and pre-dispatch admission
failure while preserving reserved IDs; targeted Chat6 PASS after those fixes.

Stop is implemented (catalog67 at its native run). Native **4aNBY2 PASS**, 35
checks and two local fixture generations; native/wrapper exit0 and private
app-data removed. Exact request identity, unknown stop refusal, terminal
checkpoint and old stop replay preserving a replacement generation passed.
The actual sending tab was revealed and its live stream/next-draft screenshot
inspected. Native units2, core Chat6, wire5, permission UI1, production workspace
check and TypeScript PASS. The tool does not depend on a renderer ACK or visible
panel and never registers an unknown cancellation.

Export is implemented (catalog68): chat.read/export, text-only
Markdown/JSON pages of all saved variants and persisted draft, no native file
writes. System instructions, provider metadata, reasoning/non-text parts and
attachment bytes/names are omitted. Limits: 512 messages, 4 MiB raw/document,
8192 UTF-16 units/48 KiB per response, two producers/5 s deadline. Revision is
SHA-256 of the whole UTF-8 document; subsequent pages bind it and reject changes
or split scalars. Native export1, core Chat7, wire5, permission UI1 and TypeScript
PASS. Native **BxwjjV PASS**: 41 checks, catalog68, two fixture requests,
23 JSON export pages reconstructing five messages and a draft with the exact
SHA-256, text-only privacy filtering, Markdown, unchanged DB and stale/split-page
rejection. Host/native/wrapper exit0 and private app-data removed. Full native chat units33 PASS, including the 513-message export refusal.
Full broker57 PASS. Probe Clippy PASS after boxing ChatStopped (unchanged JSON)
and boxing the internal stop admission error. Final wire5 PASS. No native
process is running. Mixed-layout integration remains unfinished;
the full Chat milestone is uncommitted.

Chat AI reads are now wired through Settings-selected exact conversation grants,
protocol DTOs, broker authorization, native SQLite and the helper (catalog63).
No provider call or history initialization occurs through MCP reads. Settings
alone may initialize the history picker after the user explicitly opts in.
Persisted draft/message pages use UTF-16 scalar boundaries and revision hashes;
read results omit system prompts, credentials, provider metadata, reasoning and
attachment bytes. The broker rechecks project membership and permission after
native work. Unit/native store2 and real UDS2 tests PASS; production check and
TypeScript PASS. UI2 PASS after updating the existing Android assertion for the
new empty conversation field; the picker screenshot was inspected. Native attempts 63tTnf/ymrFrr found transient Settings picker lock contention;
the picker now waits at most 500 ms, while MCP reads still fail promptly when
busy. xHWbvi passed history assertions but crashed during synchronous native
revocation: cleanup used ambient Tokio spawn_blocking on the UI thread. A
regression test reproduced the panic. Cleanup now uses the broker's captured
runtime handle; the full broker suite52 PASS. Native **jMNDbu PASS**:
11 history/authorization/revocation assertions, catalog63, native and wrapper
exit0, host exited and private app-data removed. Screenshot inspected; a later
CSS adjustment gives each conversation its own readable row, with UI1 PASS and
screenshot inspected. Probe Clippy PASS. The cleanup fix was separately tested and pushed as recorded above. At that
read qualification, open/draft/send/stop/export remained unimplemented, so the Chat milestone is incomplete and has not been pushed.

First milestone was committed and pushed to origin/main as
`d9b970ea2963bc2e77a46d258a61e83331d0ccf2`; remote SHA was verified and the tree
was clean. No tags/releases were created. Current work: Chat AI control, starting
with bounded persisted history reads and explicit conversation grants. Full Chat
AI is still incomplete; do not expose unimplemented mutation tools or mark the
module complete after read-only support.

Current full-domain native regression: **RQy5Jv PASS**
(/tmp/lomi-mcp-theme-protection-full-native.log), 61 tools, native/wrapper exit 0,
host exited and isolated app-data removed. Android NOT_CONFIGURED. This includes
the close-autosave fix below and the new explicit critical-dialog protection.
This is the previous committed baseline, before the current Chat increment.
Frontend model174 + AI runtime17, frontend build and TypeScript PASS. Production
and mcp-probe Clippy (without app test targets) PASS. Workspace Rust tests PASS:
341 passed, 20 intentionally ignored native/crash/full-disk fixtures. Frontend
formatting and workspace Rust formatting PASS. MCP wire/process tests5 PASS
(/tmp/lomi-mcp-milestone-wire.log). First milestone was pushed to origin/main; the current selected module is Chat AI control.
Remaining app-test Clippy baseline findings
are documented in QUALIFICATION.md; no all-target app Clippy pass is claimed.

Critical theme-control protection is natively qualified: **XFBrB5 PASS**
(/tmp/lomi-mcp-theme-protection-native.log), native and wrapper exit 0, isolated
app-data removed. Source is unfrozen. Catalog remains 61. Package layers are
suspended in Agent Control Settings and explicitly protected decision dialogs.
Native proof covers hostile JSON ancestor styles, an external stylesheet, a late
reload during protection, restoration on dialog close, the native menu handler
opening the protected page, unchanged stored preferences and an exactly approved
MCP switch back to Lomi with identical retry. The same run passed nine theme,
nine terminal and six editor cases, retained two PTYs and dirty text/Undo. Native
screenshots were inspected. Android NOT_CONFIGURED.

The first broad Modal guard caused two theme-preview regressions; narrowing it
to critical decisions restored all 25 UI tests. A separate nested-dialog test
passed. TypeScript and mcp-probe cargo check passed; final Clippy is running.
One earlier UI run found an intermittent session-save ordering failure at close.
A targeted test then reproduced two saves during shutdown. The completed fix
pauses debounced autosave before the final save and resumes it on cancellation
or failure; debounce reads the current session. UI close/updater/terminal21 and
TypeScript PASS. Native full-domain regression is running before the first
authorized milestone commit/push. No further lifecycle MCP tools will be added.

Previous increment:

Builtin theme writes are implemented and natively qualified: **lt7OvZ PASS**
(/tmp/lomi-mcp-themes-native.log, LOMI_MCP_SETTINGS_THEMES_ONLY=1), native and wrapper
exit0, private app-data removed. Source is unfrozen. Catalog remains61. Closed theme_builtin choices
Lomi/DeepMono and theme_appearance system/light/dark reuse the Settings approval,
original Themes mutex, shared native version/ID/kind validator, atomic preference
CAS and existing event. Appearance writes refuse a current custom color theme;
safe-mode/recovery refuse all implicit replacement. Unrelated fields/icon
selections survive. No installed/custom theme selection/import/recovery exposed.

Checks PASS: workspace check, TypeScript, native themes11, protocol12 (including
custom-appearance refusal), UI2/screenshot inspected, wire5 below700KB, production
and MCP all-target Clippy, git diff --check. Native fixture adds nine theme cases
with two retained xterms and a dirty original document/Undo, then repeats nine
terminal and six editor cases. All nine theme cases passed, as did the nine terminal and six editor regressions.
The Settings approval and actual light/dark terminal screenshots were inspected.
Android NOT_CONFIGURED. After this slice,
complete CSS-resistant controls and installed/custom/import/recovery, then plugins
and all other required P5/P6 work. Do not finalize v1 here.

Previous increment:

Keyboard shortcut writes are implemented and natively qualified on macOS ARM64. The
closed patches set/disable one available action, reset its stored override, or
change focus-follows-pointer. Main reuses restoreKeybindings with native source
and bounded plugin shortcut metadata, compares retained values/defaults, and
rejects conflicts or changes to unrelated effective bindings. Source access is
bound to the claimed operation/nonce/revision and is Main-only. Settings alone
approves. Native publication uses the original KeybindingsFile mutex and pinned
file CAS while the plugin lifecycle mutex binds the captured definition hash.
Unknown stored descriptors and other JSON fields are preserved; corrupt files
are not recovered implicitly. A disabled package is never enabled or evaluated.

Checks PASS: TypeScript, model3, native keybindings3, UDS49, protocol11, wire5 at
catalog61, UI2 (approval screenshot inspected) and production workspace Clippy.
Evidence: /tmp/lomi-mcp-keybindings-*.log. The combined native probe
**GJqZd0 PASS** (/tmp/lomi-mcp-keybindings-native.log): twelve shortcut scenarios,
nine terminal and six editor regressions, exact retries, unchanged two retained
PTYs/native process contexts, ordinary native/wrapper exit0 and private app-data
removal. The native approval screenshot was inspected. The disabled test package
was imported through the existing Settings command, changed the definition hash,
blocked the pending shortcut write, and was never enabled/evaluated. Android
NOT_CONFIGURED. Source unfrozen; next finish themes/plugins and all remaining
P5/P6. Full v1 remains IN_PROGRESS; no scope has been deferred.

Additional native **IR2YQ0 PASS** (/tmp/lomi-mcp-keybindings-native-2.log) explicitly
creates a dirty document BEFORE shortcut mutations and proves the identical
retained document, exact text and working original Undo afterward. It refocuses
the original terminal through MCP before the visible/hidden runtime checks.
Twelve shortcuts + nine terminal + six editor cases pass with normal exit0 and
app-data removal. MCP all-target Clippy also PASS. Source unfrozen.

Builtin theme selection/appearance implementation is now active: protocol/native
plan/shared validator and main/Settings UI are written; initial workspace check
and TypeScript running. No theme native evidence yet. Installed/custom selection,
verified import/recovery and CSS-resistant approval/status surfaces remain required.

Previous qualified increment:

Terminal preference writes are implemented and natively qualified on macOS ARM64. The
catalog remains 61 tools. `lomi_settings_update` has a closed terminal field enum,
scalar values and explicit nullable appearance resets; shell profile code and
arbitrary keys remain outside the contract. The adapter reuses the existing
terminal validator, file mutex and change event. Editor and terminal writers share
only the pinned preference-source/atomic-publication utility; Settings approval
and section-specific validation remain unchanged. The exact approval shows only
changed terminal values, and explains scrollback trimming when the requested
value decreases.

Checks: core58+1ignored, UDS48, protocol9 plus the new explicit-null test, native
terminal validator/field-preservation test, UI2, final UI approval regression,
TypeScript, wire5/catalog61 under 700 KB and production/MCP all-target Clippy PASS.
One intermediate schema check correctly rejected a required annotation that also
removed null; the final schema preserves required-but-nullable values and the wire
suite passes. Native terminal runs 9wzISd and aE3U51 failed test comparisons:
JSON integer versus float representation, then the existing terminal engine's
RGBA color (`#123456ff`) versus the requested RGB (`#123456`). Both ended with
normal native exit0 and removed private app-data. The fixture now compares numeric
values and the engine's exact RGBA output. These attempts are not qualification.

A publication review also found that retiring an anchor workspace during a native
Settings action did not revoke its operation permit. Publication now invalidates
only affected Settings open/update permits. The new write regression removes the
anchor during application and proves the final guard rejects before file effect;
all 48 UDS tests and final production/MCP all-target Clippy pass. Native KnBYQo completed all seven terminal cases and retained both PTYs, but the
following editor fixture tried Undo on the hidden original document after closure
selected a neutral blank editor. The fixture now explicitly focuses its original
file through MCP before exercising Undo. KnBYQo is FAILED combined evidence with
SIGTERM cleanup and removed private app-data. Corrected combined native **hhAM3F
PASS** (`/tmp/lomi-mcp-settings-terminal-native-4.log`), normal native/wrapper exit0
and app-data removal: seven terminal cases, two retained visible/hidden runtimes
and outputs, and all six editor cases with retained document and Undo.

Final review aligned native font-family/word-separator length limits with the UI's
UTF-16 units. All three native terminal preference tests and production Clippy
pass. Final combined native **u6PGm8 PASS** adds both oversized-Unicode refusal cases
(nine terminal cases plus six editor cases), with ordinary native/wrapper exit0
and private app-data removal (`/tmp/lomi-mcp-settings-terminal-native-5.log`).
No Android in this profile. Source is unfrozen.

Previous qualified baseline:

The current catalog contains 61 tools on macOS ARM64. The current increment is
`lomi_settings_update` for editor tab size and indentation, using explicit global
settings.read/settings.write grants and an exact expiring Settings decision. The
native plan compares retained effective values, validates the stored file and
pins its original byte revision. Approval uses the section lock, atomic create or
replace, a final authority/revision check and the normal change event. The main
queue remains available while approval is pending. Invalid files stay intact;
unknown publication effects retain durable receipts and never replay.

Core58+1ignored/UDS47/protocol9, UI2, wire5/catalog61 (existing 700 KB budget),
final TypeScript and production/MCP all-target Clippy PASS. The UDS write test
covers 12 scenarios, including no-clobber create, replacement, rejection, cancel,
concurrent writes, symlinks, mismatched provider/native plans, revoke before and
during application, unknown effects and exact retry. The approval UI was visually
inspected. The first native61 run X06AUX found a pre-approval recovery failure stuck in
awaiting_user: its failure could not be recorded from that receipt state. The
uncommitted failure now returns to queued before recording failed/none. The new
UDS regression and all 47 UDS tests pass, as do the UI recovery regression and
MCP all-target Clippy. X06AUX is FAILED evidence (forced host cleanup, private
app-data removed). Corrected native61 Settings-update 0GkHVO PASS via
`/tmp/lomi-mcp-settings-update-native-2.log`: six real file/approval/recovery cases,
retained dirty document and Undo, unchanged unrelated preferences/native PTYs,
Main caller rejection, exact retries and normal host exit0/app-data removal.
Full native61 fCqMdY PASS via `/tmp/lomi-mcp-settings-update-full-native.log`,
including all previous file/Git/PTY/browser/layout/lifecycle/Codex cases, normal
exit0 and app-data removal. Android NOT_CONFIGURED. Source is unfrozen.

Preceding focused native60 Settings-read qqYreQ PASS, wrapper/native exit0 and
private app-data cleanup (`/tmp/lomi-mcp-settings-read-native-2.log`): all four
actual preference sections, shortcut pages, real Settings changes, stale
revision, invalid-file preservation and retained dirty editor. Focused native59
Settings-open KzW0fU PASS for all nine sections and exact retries without file,
layout, editor or native-context changes. Native58 project-open DJziSo and
project-close Zdqpsp PASS; full native58 H2IkCx PASS with ordinary cleanup. These
runs did not configure or requalify Android.

Next: implement shortcut/theme preferences and the required protected
import/recovery/plugin flows. Android management, Chat AI, all panel kinds and the
remaining P5/P6 work are still required. Do not finalize v1 here.

Git implements all seven bounded operations, including discard and pull; the
preceding full native55 xSPHym remains separate evidence. Android was
NOT_CONFIGURED in the layout run. Full native55 Im0Xl9 remains Android input and
rotation proof; assisted focus resolved that blocker. The isolated emulator and
private ADB were stopped without force; the user's emulator/shared ADB remain
untouched.

Next increment: `workspace_update` now has an explicit `close` action, separate
workspace.close permission and shared dirty-document guards. Save keeps the
workspace open and returns partial effects; Cancel preserves buffers. Exact
native descendant checks precede any close, including owned idle PTYs, protected
origin checks and owned browser generations. Android/chat/plugin descendants are
still unavailable in this initial close adapter and remain required work.

Prepared closure receipts record null closure booleans before the first native
effect. Only a verified final domain publication can atomically advance that
specific preparation to succeeded/complete. Retired-workspace receipt access is
limited to the original authenticated owner, project, workspace and close scopes;
it does not authorize new resource access. Two real UDS tests passed, including
retry after removal, forged ACK refusal, cancellation and preserved unknown
effects. Final Settings/close UI tests2 and TypeScript passed. Core57+1ignored, UDS40, protocol9, wire5, production and MCP all-target Clippy
passed (`/tmp/lomi-mcp-workspace-close-*.log`). Full native56 **YtiSKE PASS**
confirmed Cancel, MCP cancel, actual Save/retained workspace, Discard/exact retry,
retired buffer denial, protected ancestor refusal and exact idle PTY/browser close.
`workspace-close{,-runtime}.json` record the assertions; the real shared guard and
`mixed-layout-browser.png` were visually inspected. Host exit0 and app-data removal
passed. Android NOT_CONFIGURED in this run; prior Im0Xl9 remains Android evidence.

Cross-workspace `transfer_tab` is implemented; native transfer assertions passed as recorded below.
The shared model preserves complete tab descriptors and neutralizes selection
when a move would activate another lazy runtime. A live claimed operation and
its exact two-workspace publication admit native PTY/browser/Git ownership
migration; ordinary/cancelled moves do not. Existing terminal runs follow the
retained generation. Model3, TypeScript, core57+1ignored, UDS41, protocol9,
wire5 and both Clippy sets passed. The expanded result is boxed internally to
retain bounded enum sizes; JSON is unchanged. Workspace update now truthfully
advertises destructiveHint because it includes guarded close.

Native56 **ZXX06z failed** in the later idle workspace-close regression. The new
transfer assertions passed (workspace-transfer.json): same live PTY/command,
retained native browser form, same dirty buffer/document, exact forward/back
receipts, foreign destination denial and old source resource denial. The guarded
file close cases also passed. Native idle PTY closure then occurred, but the full
workspace result remained outcome_unknown with its prepared null booleans;
failure.png shows the stopped PTY and retained workspace. No repeat was attempted.
Cleanup removed fixture app data and terminated the fixture host with SIGTERM.
This is not a full native PASS. Rerun **tLJg5N PASS**, clean exit0/private app-data
removal; all transfer and workspace-close cases passed. Its attempted JS invoke
wrapper recorded no events (native invoke is not replaceable), so it does not
explain or fix ZXX06z. Removed that ineffective wrapper. A diagnostic stress run
now repeats six fresh idle PTY/browser closures with distinct request keys and
records close commit results plus accepted ACKs in a bounded probe-only native
JSONL file. Production effect accounting remains native-authoritative; a known
post-commit revision conflict still reports unknown effects. TypeScript passed.
Diagnostic **Ik8p9l FAILED**: the first idle PTY/browser workspace closed with
native-close error null and accepted success ACK. Round two failed during
terminal creation with a claimed REVISION_CONFLICT/unknown; it never reached
workspace close. Cleanup terminated the fixture and removed app data. This does
not qualify the six-round run.

Two fixes now have targeted evidence: claim validation compares exact serialized
Session values instead of object references (the publication revision already
uses serialized values), and post-native workspace removal checks the exact
target structure against the latest session, preserving unrelated changes and
ignoring only resource titles/editor viewport positions. New/changed children,
paths, generations and workspace identity still conflict. New text or newly
loaded dirty documents still block removal. UI1, model5 and TypeScript passed;
the UI test edits during an intentionally held native commit and proves RAM is
retained. Original run/interrupt retries now resolve authenticated receipts
before live terminal IDs. The expanded UDS transfer test passed for migrated and
cancelled transfers, conflicting/fresh requests, one dispatch and cancellation
using the migrated run. MCP core all-target Clippy passed.

Full **tiONrr FAILED** again during the second terminal creation (claimed
REVISION_CONFLICT/unknown). The first idle workspace close succeeded with native
error null/accepted ACK. The moved-run retry assertion passed in
workspace-transfer.json (same original operation). Cleanup removed private data
and terminated the host. Value equality alone did not solve the creation race.
A bounded development-only observer now records exact changed model fields;
focused close-only mode skips unrelated earlier tests while retaining native
PTY/browser resources and real pairing. Diagnostic log
**19i2LE FAILED** reproduced the issue with only one changed field:
`session.projects.0.workspaces.2.tabs.0.position` became the default zero editor
position during the native claim. This is actual native evidence of viewport
persistence causing a false domain conflict. Cleanup exited0 and removed app data.

The bridge now excludes only actual FileTab viewport positions (also inside mixed
layouts) from its control domain signature, retaining exact file/resource paths,
selection, layout and arbitrary plugin state. After an accepted claim it uses the
latest equivalent session, preserving those viewport changes. Model6 and
TypeScript passed. Focused native **4gguA9 completed six successful idle
PTY/browser closures** with no unknown effects; remaining trace conflicts were
late browser titles, rejected before effects and handled by fresh revision reads.
Native result passed and cleanup exited0/removed app data. Its new focused-mode
wrapper then failed because it expected the full HTTP test's counter file. That
wrapper now separately validates all six focused close artifacts and the explicit
profile; the full profile's HTTP assertions are unchanged. Overall wrapper PASS
is still pending. Logs `/tmp/lomi-mcp-viewport-{model,native}.log`.
Full core57+1ignored, UDS41, protocol9, Settings/closure UI2, production workspace Clippy, full pnpm check and release frontend build PASS. The development diagnostic observer is absent from generated production JS. Focused wrapper wNJLEh and full native YWPPVZ PASS: six fresh native closes, transferred running-command exact retry, all earlier file/editor/Git/PTY/browser and Codex assertions, exit0 and private app-data cleanup. Android NOT_CONFIGURED; prior Im0Xl9 remains its evidence. Source is no longer frozen. Next: implement project_close plus explicit selection of additional existing workspace grants; then project_open and remaining panel kinds. Logs `/tmp/lomi-mcp-viewport-{stress,full}-final.log`. A development-preview
usage guide is now in USAGE.md; it does not claim complete P6 or a published release.
Next: identify and resolve the intermittent close, requalify, then project
close/open and remaining panel kinds. Project lifecycle contracts are recorded
in REQUIREMENTS.md. The prior terminal-run retry audit finding is fixed with targeted evidence above. Native qualification must still prove the original request returns its receipt after transfer. Earlier plan: move receipt lookup
before current resource resolution with original workspace/scope checks, and prove
this with native server retry while the PTY is in the destination.
All remaining P5 domains, P0–P4 qualification gaps and P6 remain required. This is
not a claim that the complete MCP v1 system is finished.

## Dependencies and gates

- Production adapters depend on their P0 native evidence.
- Other OS/CPU targets need their own native test host; no support inferred from macOS ARM64.
- Signing/publication require actual release credentials and authorization; local builds can proceed independently.
- Paid model/provider calls and provider license acceptance are not performed implicitly.

## Resume and resources

Next: P5.3 layout/projects and all remaining P5/P6 domains and the
full qualification matrix. No stage is marked complete merely because a vertical
increment passes. During an active native fixture, do not edit src/ (Vite HMR
revokes the fixture's UI epoch). Read the newest entries at the end for exact logs.

The isolated Android SDK/AVD is retained; see ANDROID-FIXTURE-PLAN.md. The
native55 wrapper may start only its `MCP qualification` device/private ADB15047.
Existing user emulators and shared ADB5037 remain untouched. No paid provider call,
user-client configuration write or release publication has been performed.

Baseline backup (outside Git): `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-baseline-fvzi1ppu`.

Browser open now has actual WKWebView/stdio evidence: explicit exact-origin Settings approval, isolated profile identity, one-use native creation ticket, durable deduplication and redirect blocking before any foreign-server request. Native DOM image was inspected at `lomi-mcp-control-81NHbS/browser-open.png`. Native navigation, semantic DOM snapshot and synthetic click/fill have now passed further native increments. Hidden creation and the full browser qualification matrix remain unfinished. Bounded main-frame JavaScript errors are implemented; console and Promise-rejection capture are explicitly unavailable. Native PNG capture and scoped immutable artifact reread have passed. Current native main-window terminal WebGL painting cannot be qualified while WebKit reports the document hidden and suspends animation frames; existing reveal behavior was preserved.

The native 23-tool fixture at `lomi-mcp-control-2xBPrJ` passed a React form through the production helper: empty-field validation, Unicode fill, click, exactly-once retry and a resulting semantic DOM snapshot. Private form values are omitted, page-world globals cannot supply isolated references, and native WKContentWorld is retained across calls (a reproduced lifetime bug was fixed). Select/contenteditable, replaced-node rejection, focus-scoped synthetic key and viewport scroll also passed. Current native DOM input is explicitly `synthetic_dom`; it does not claim trusted gestures. Browser read and interaction are separately opt-in Settings scopes. Native dispatch now checks operation cancellation and deadlines before entering WebKit.

Latest complete native increment: `lomi-mcp-control-vehgxG`, 26 tools, recorded PASS. It includes asynchronous DOM wait, bounded timeout, SPA reference invalidation, a four-second page-JS hang proving a timed-out fill never executes later, native permission-delegate installation, and all preceding PTY/Settings/Codex assertions. The child PNG was visually inspected. Media capture API is unavailable on this WKWebView, so actual media-denial callback qualification remains open (`browser-permissions.json` records zero callbacks). Automation starts at blank, installs deny-by-default UI delegates, then navigates. File chooser, media and motion policy are not inherited from Wry. Native DOM work stays capacity-bound through its callback even after timeout. Browser runtime actions use target generation and refreshed projection rather than unrelated layout revisions.

Latest regressions: `pnpm test:mcp` 5/5, core 20 + UDS 10 + protocol 6, browser UI 4/4, pnpm check, production workspace Clippy and new crates all-targets Clippy PASS. Native evidence and limitation details are in QUALIFICATION.md. Next: screenshot/artefact budgets and scoped access, remaining browser policy/logging qualification, then the remaining P2/P4/P5/P6 work.

The 28-tool native increment `lomi-mcp-control-ggnnxP` passed, including native PNG geometry/budgets, exact-byte artifact reread, foreign-workspace denial and human takeover denial. `cleanup.json` confirms native exit code 0 and isolated app-data removal. The first capture run exposed a fixture shutdown/removal race; the runner now waits for its process to exit before removing only its isolated app-data root. Artifact storage schema is 3, with private files, producer and storage reservations, expiry, hash validation and symlink-safe recovery. macOS automation now requires the native isolated-store API (14+) and verifies the actual store UUID before the first external navigation. This API guard does not qualify untested macOS versions.

Latest native proof: `lomi-mcp-control-TWNWAc`, 29 tools, PASS with clean exit 0 and removed isolated app data. Log capture, scoped cursor pagination, overflow, page-world/synthetic-event isolation, foreign workspace and human takeover denial passed. Promise rejection events do not cross into WK's private world on this host and are explicitly excluded. Messages are bounded and normalize broken surrogate pairs. Owned browser panel focus now preserves page/form state, requires exact browser generation, and deduplicates retries.

The hidden-capture negative test initially exposed a real visibility race (`CgwHUH`): the main WK view can suspend RAF while the native child is still visible. Broker projection now atomically marks browser selection; native input/capture checks it in addition to actual view state. Browser runtime flushes event-driven geometry/hide updates through a microtask when its document is hidden; there is no idle timer. The rerun rejects hidden capture and retains the same page when refocused. Terminal painting behavior is unchanged.

Native navigation cancellation and deadline now passed in `lomi-mcp-control-HneLGS` (29 tools): a real unfinished HTTP response closed in 45 ms after cancellation and at 15,054 ms for the 15-second native deadline. Both operations retain outcome_unknown, do not replay on retry and recover with a subsequent successful navigation. Native receipt completion no longer waits for main-renderer JavaScript. Cleanup confirms exit 0 and removed isolated app data. The broader P0–P6 scope remains unfinished; next implement the terminal-owned dev-server fixture/association and remaining browser lifecycle/mixed-panel flows before P4/P5/P6.

Final checks for the 29-tool navigation/log/focus increment: core 23 passed + 1 ignored subprocess helper, real UDS integration 10 passed, protocol 6 passed; workspace production Clippy and new-crate all-target Clippy passed; TypeScript passed; browser UI 4/4 and independent stdio schemas 5/5 passed. Logs: `/tmp/lomi-mcp-nav-final-{tests,core-clippy,clippy,ts}.log`, `/tmp/lomi-mcp-final-browser-ui.log`, `/tmp/lomi-mcp-logs-wire.log`. Native `HneLGS` includes real Codex connection. Test-only assertion placement was corrected: a load event before native commit does not finish the pending navigation. No user emulator was started; disk remains approximately 12 GiB free. No fixture native process remains.

Next increment in native verification: `tests/mcp/browser-server.mjs` now starts exclusively through lomi_terminal_create/run in the fixture's approved workspace. The runner reserves/releases a port for the explicit test grant, but does not host the allowed page. The fixture reads its exact command block, binds the reported URL to that operation, requires the command to remain running, tests start deduplication, and stops the server with the targeted MCP interrupt. HTTP/DOM/form/screenshot assertions then use that PTY-owned server. PASS: `lomi-mcp-control-I19XEn`, `/tmp/lomi-mcp-pty-server-native.log`; native exit 0 and app-data removal confirmed. The owned Node server exited with shell-observed code 0 after targeted MCP interrupt; its recorded PID is no longer present. This is still not full E01 model-routing qualification.

Latest full native increment is `I19XEn` (29 tools), including the PTY-owned web fixture and all preceding log/focus/cancel/HTTP assertions. Current code checks remain the prior successful Rust/core, UI, TypeScript and wire results; this final fixture-only change passed its native test. Next: remaining P2 workspace select/close and panel move/owned-browser close/hidden lifecycle; full browser qualification matrix; then all P4/P5/P6 domains. No v1 scope has been removed.

Latest native proof: `lomi-mcp-control-MzdU9b` (29 tools), PASS with exited host and removed isolated app data. Owned browser close dispatches to the exact native generation on AppKit, removes its input monitor, deduplicates retries, preserves human-owned peers and denies rereading closed artifacts. Earlier runs `6nNwhO`/`XCjJyH` exposed a missing UI navigation-denial message despite native policy correctly blocking the request. Browser state now carries monotonically increasing decimal revisions; the frontend rejects older updates (including integers above JS's exact Number range). A late load-start callback no longer clears an automation denial. `13hTJA` stopped on a legitimate optimistic focus revision conflict; the fixture now re-inspects and retries at most three times only after a recorded no-effect conflict. It does not retry unknown effects.

The same increment fixes contenteditable value disclosure: snapshot text traversal omits editable contents while retaining explicit labels and semantic references. Native assertions cover initial and filled values. Browser UI 5/5 PASS (`/tmp/lomi-mcp-browser-revision-ui.log`), denial screenshot inspected; TypeScript PASS (`/tmp/lomi-mcp-close-final-ts.log`). Native log: `/tmp/lomi-mcp-browser-close-native-4.log`. Next: workspace select and remaining guarded layout operations, then remaining P3/P4/P5/P6. Full v1 remains unfinished.

Workspace selection is VERIFIED in `lomi-mcp-control-oNZ5Gg`: action=select on workspace_update preserves the exact dev-server PTY/operation across both workspace switches and deduplicates receipts (`workspace-selection.json`). Native cleanup completed. TypeScript, core 23 + UDS 11 + protocol 6, stdio/schema 5/5 and production workspace Clippy passed (`/tmp/lomi-mcp-select-*`). The first native attempt failed compilation due to a fixture JSON/string mismatch; corrected before the successful run. Hidden browser creation is the next increment in qualification; Android public tools and all outstanding v1 work remain required.

Hidden browser creation is VERIFIED in `lomi-mcp-control-wfHZKO` (29 tools), PASS with clean host exit and app-data removal. The hidden WK child preserves the selected human browser, returns semantic DOM at 800×600, denies screenshot before reveal, then reuses that exact generation for focus, capture and close (`browser-hidden.json`, `browser-close.json`). Failed qualification attempts `dL7CDD`, `sWIiqy`, `hNxlTn` exposed fixture mistakes (tab ID on its parent, elements rather than nodes, element wait discriminator); the corrected test waits for React's heading, not just document load. No hidden-image readiness is claimed.

The storage test run `/tmp/lomi-mcp-hidden-core-2.log` had an intermittent reopen refusal. Owner-lock teardown now explicitly unlocks after SQLite closes; a retained-descriptor test verifies immediate reopen and that closing the old descriptor cannot unlock the new owner. Its causal connection to that single refusal remains an inference. A separate hung EOF test had a sampled, confirmed cause: serve cleanup synchronously waited on the policy lock on a Tokio worker (`/tmp/lomi-mcp-core-hang.sample`). Disconnect cleanup now coalesces in a blocking worker with a request revision; atomic authority still revokes first. Enrollment/epoch registration also moves blocking storage/policy work off the executor, with connection rechecks before publication. The stronger single-worker stalled-storage test passed in 0.07s (`/tmp/lomi-mcp-disconnect-single-worker.log`). Core 24 + UDS 11 + protocol 6 passed (`/tmp/lomi-mcp-hidden-core-5.log`); browser UI 5/5 and TypeScript passed. The latest native `wfHZKO` also includes the handshake/cleanup changes. Next: selected-device Android read/open/start/input/artifacts, followed by remaining layout and all other P5/P6 work; full v1 is unfinished.

First public Android increment: `lomi_android_list` is implemented and native-verified in `lomi-mcp-control-1A9KWF` (30 tools), PASS with host exit and isolated app-data removal. Settings explicitly grants android.read for a selected real managed device; project/workspace scope alone does not grant Android access. The helper returns only the selected device's name, generation/phase/display/process status and metadata revision/host qualification. No paths, ADB serials, logs or keys are returned and no emulator starts. `android-list.json` includes actual stopped-device metadata and foreign-workspace refusal. The existing licensed fixture SDK/AVD was reused read-only; the user's emulator/ADB was untouched. Native fixture code can reuse the isolated consent-checked directory under mcp-probe; normal release excludes the override.

Core 24 + real UDS 12 + protocol 6 passed (`/tmp/lomi-mcp-android-list-final-rust.log`), independent helper/schema 5/5 passed, TypeScript and production workspace Clippy passed. After the native run, the metadata dispatcher gained an atomic connection/policy check and five-second deadline before and after its native read; core/UDS checks passed, and the next Android native increment must include it. Device-selection UI passed; its explicit rows were visually inspected (`/tmp/lomi-mcp-android-list-ui-2.log`). P4 is IN_PROGRESS; Android open/start/stop/input/snapshot/capture/install/logs remain to implement. Next concrete step: Android open creates a retained manual-start panel without implicitly invoking Android runtime.start; then durable native start and device-bound ownership through the existing manager/router. Keep all P5/P6 and remaining browser/layout gates in scope.

Android open is VERIFIED in `lomi-mcp-control-cplPJo` (31 tools), with native exit and app-data removal confirmed. The retained panel refers to the selected real device, retries return the same receipt and no emulator starts (`android-open.json`). Manual start survives saved-session restoration; the stopped panel screenshot was inspected. `ypQmpY` failed because the fixture passed revision rather than domainRevision; `SdHidH` exposed a missing runtime observer notification after marking the manual panel visited. Both are corrected. Android panel IDs use canonical UUIDs so they also satisfy the existing input router's view identity contract.

Next Android increment implements start/stop (33 tools) with selected-device android.control, native-only completion, 180-second durable work deadlines, per-device authority and exact-generation Stop. Manager preparation and actor dispatch recheck revocable guards; already-running human phones cannot be acquired. Native human Start/Stop/input revoke agent authority. Failed boots retain their generation for an explicit Stop. No new emulator window or alternative ADB server is introduced. Metadata/open are already native-verified; start/stop are IMPLEMENTED_UNVERIFIED pending the current full native run. Core 24 + UDS 14 + protocol 6, native manager tests 5/5, actor authority test 1/1, TypeScript and permission/manual-open UI 2/2 passed. Logs: `/tmp/lomi-mcp-android-runtime-{rust-2,manager-tests,actor-test,ts,ui}.log`. UI approval screenshot inspected. Current native command reuses LOMI_ANDROID_PRODUCT_DIRECTORY from ANDROID-FIXTURE-PLAN.md; `/tmp/lomi-mcp-android-runtime-native.log` will contain its artifacts path. Remaining P4 input/observations/artifacts/APK and all P5/P6 remain required.

Android start/stop is now VERIFIED in `lomi-mcp-control-H8Q7b3` (33 tools), `/tmp/lomi-mcp-android-runtime-native.log`. The real private device booted to manager readiness, retained one generation on retry, rejected a stale-generation Stop and exited after exact-generation Stop. The completed stop receipt deduplicates even after authority is removed. `android-runtime.json` contains the results. Host exit 0 and isolated app-data removal confirmed; the fixture's runtime-record directory is empty and private ADB was stopped by fixture cleanup. Production workspace Clippy and new-crate all-target Clippy passed (`/tmp/lomi-mcp-android-runtime-{clippy,core-clippy}.log`); stdio/schema 5/5 passed. No fixture process is intentionally left running.

Next: extend the existing native Android input router with human/agent ownership, revocable queued input, device-wide leases, exact payload/sequence deduplication and release on takeover/blur/revoke. Use explicit Settings input consent; preserve native main-window visibility/focus. Qualify real Unicode/touch/keys on the retained isolated phone. Then bounded hierarchy/capture, immutable APK import/install, launch/logcat, and the rest of P4/P5/P6. Public Android input is not yet implemented. Per-device lifecycle authority exists, but it does not itself grant or hold an input lease.

Android input is IMPLEMENTED_UNVERIFIED (34 tools). The device-wide revocable lease reuses the native router, has one in-flight input and 128 retained sequence hashes/results, and returns RAM-only lease IDs. Native completion owns receipts; stored input request keys hash the lease/sequence and never persist the input text or lease. Settings separately opts into android.interact. Human takeover, selection change, window blur and modal/form/pane changes invalidate the lease and release physical input through the native queue. Only a live lease owns the native watcher and DOM observer. Claim/input require selected, rendered phone geometry, a decoded frame and actual native window focus/visibility.

Core 26 + UDS 15 + protocol 6 passed (one ignored subprocess helper), `/tmp/lomi-mcp-input-core-final.log`; TypeScript passed. Permission/manual-open/takeover UI 4/4 and hidden-document/RAF, native minimization, dialog/form/hidden-pane release UI 4/4 passed (`/tmp/lomi-mcp-input-{ui-final,focus-ui}.log`). Native `uPqm9j` failed because hidden WK document state suppressed Android streams. Native window state now governs visibility and hidden-document frame delivery uses a cancellable-by-identity microtask; native minimization still stops the stream. `beSc5Y` confirmed real frames (8 IPC frames), but failed native input claim with OUTCOME_UNKNOWN, so actual guest input is not yet qualified. Its explicit before/after Android cleanup passed, no forced Stop, private ADB stopped, host exited and isolated app data removed. The fixture now waits for device cleanup before publishing its final result, including failures.

Current next step: `/tmp/lomi-mcp-input-native-3.log`, licensed directory from ANDROID-FIXTURE-PLAN.md, qualifies native AppKit focus and records a probe-only claim failure reason. Continue real Unicode/keys/touch/rotation and exact-lease negative tests. Then implement bounded hierarchy/capture, immutable APK import/install/launch/logcat and all remaining P4/P5/P6 work. Full v1 remains unfinished; no requested scope has been removed.

The Android focus failure has a confirmed host cause: IORegistry reports `CGSSessionScreenIsLocked=true`. `gprft3` recorded nativeVisible=true/nativeFocused=false and correctly refused input; cleanup passed. User was asked asynchronously to unlock; no input guard was bypassed. The initial unrestricted hidden-document fallback was tightened: hidden documents render only with real native focus, and locked/minimized windows release streams and GPU buffers. UI tests cover screen lock, minimization, suspended RAF, modal/form/pane release. Actual Unicode/key/touch/rotation input through MCP remains NOT_RUN until the host is unlocked; the event/lease implementation is not accepted on mock evidence.

`lomi_android_snapshot` is now native-VERIFIED in `lomi-mcp-control-CCKqQn` (35 catalog tools). `/tmp/lomi-mcp-snapshot-native.log`: PASS for executed assertions, `androidInputQualification=NOT_RUN_SCREEN_LOCKED`. The explicit probe-only `LOMI_MCP_ANDROID_INPUT_MODE=skip-screen-locked` skips only the focus-dependent input block; default fixture still requires it. Real UI Automator output contains the fixture label and an editable-field omission marker, respects maxNodes=1, and rejects foreign workspace and stale generation. `android-snapshot.json` records those assertions; `android-input-unqualified.json` records the missing qualification. Both before/after cleanup files confirm stopped device/private ADB, no forced exit; `cleanup.json` confirms native host exit 0 and app-data removal.

Hierarchy is separately opt-in (`android.observe`), reads only an owned exact device generation, uses the existing private smart-socket ADB and a generated private guest directory with bounded cleanup, timeout and revoked-authority IO checks. Input cannot pass a shell command or guest filename. The parser caps XML at 256 KiB, depth at 64, parsed nodes at 4096, returned nodes at 500 and serialized output at 48 KiB. DTD, external/unknown entities and malformed XML are rejected; password and editable text values are omitted. Per-device/global producer admission is bounded. Core 26 + UDS 16 + protocol 6, parser 3/3, independent stdio/schema 5/5, TypeScript and UI 9/9 passed (`/tmp/lomi-mcp-snapshot-{core-2,parser-2,wire,ts,ui}.log`). Permission screenshot visually reviewed. Native screenshot/artifact, immutable APK import/install, launch/logcat and the rest of P4/P5/P6 remain required.

Android screenshot is native-VERIFIED in `lomi-mcp-control-YtazWR` (37 tools), `/tmp/lomi-mcp-android-capture-native.log`. PASS for executed assertions; focus-dependent input remains explicitly NOT_RUN_SCREEN_LOCKED. The real guest returned a 270×480 PNG (9,173 bytes) for a 720×1280 hardware display, scale 0.375 and a matching image-to-hardware transform. `android-capture.png` was visually inspected and shows the fixture app. Exact-byte artifact reread, foreign-workspace refusal, stale-generation refusal and denial after managed Stop passed. The existing native browser artifact flow also passed after generalizing the shared store's source and geometry types. Original browser JSON remains readable without a migration/tag change; Android sources require android.capture and exact native ownership. Global capture producer and disk reservations are shared, decode/encode/pixels are bounded and PNG metadata is reconstructed. The native fixture exited 0, removed app data, stopped the device and its private ADB without force.

Final checks for this increment: core 26 + UDS 16 + protocol 8 (one ignored helper), stdio/schema 5/5, TypeScript, permission UI 1/1, production workspace Clippy and new crates all-target Clippy PASS (`/tmp/lomi-mcp-android-capture-{core,wire,ts,ui,clippy,core-clippy}.log`). The new screenshot permission was visually inspected. Quarter-turn rotation input is not native-qualified; protocol affine-transform tests cover all four quarter-turns. Next: immutable project APK import and approval/install, launch/logcat, then remaining P4/P5/P6; do not treat the snapshot/capture success as complete v1.

Next import foundation: `lomi-control-core/src/project_files.rs` now opens the approved project through directory descriptors and uses component-wise `openat(O_NOFOLLOW)` for project-relative files. It rejects traversal, symlinks, hardlinks, directories/FIFOs, known secret paths and oversized files; pins the approved directory identity and detects file mutation. Two filesystem tests and all-target core Clippy passed (`/tmp/lomi-mcp-project-file-{tests,clippy}.log`). This helper is not yet exposed as an MCP tool; immutable staging, durable import operations and APK install approval still need implementation.

APK import increment: `lomi_artifact_import` is now exposed (37 tools). Explicit Settings `files.read` + `artifact.import` pins the approved project directory at grant time. Durable UI work permits a one-use native import; the renderer cannot attest success. Inputs bind project-relative path, APK kind, exact size/hash, expected domain revision and retry key. The private 64 KiB streaming copy closes its writer before validation/publication. Schema 4 adds artifact class; old schema-3 browser/Android JSON remains readable. Reservation-owned `.stage`/`.ready`/`.apk` names support crash recovery without a directory scan. Budgets include active reservations and leases: 2 GiB global, 1 GiB per owner and project, 512 MiB per APK, 64 total files, two producers; image sublimits remain 64/16 MiB. APK rereads return metadata only. Native ZIP preflight bounds central directory to 8 MiB/8192 entries, expanded sizes and manifest to 1 MiB; no extraction or execution, ZIP64/multidisk excluded. Android installation/signature validation remains separate.

Native `lomi-mcp-control-lZY77Q` PASS: production 37-tool stdio helper + native Settings + WKWebView + PTY + isolated emulator. Imported the real 12,695-byte test APK, exact SHA-256 `5ed9daedfbb058b8a42eec463507d1333d06a33f6be122ce4b36d74a109ba48d`; retry returned the same receipt, bad hash was refused, cross-workspace artifact was hidden, changed source left the private copy's metadata unchanged. `apk-import.json` records assertions. Browser/Android screenshot regressions also passed. Android input remains `NOT_RUN_SCREEN_LOCKED`. `android-cleanup-after.json`: stopped phone/private ADB, no force; `cleanup.json`: native host exit 0 and fixture app data removed. No user emulator/shared ADB changes.

Checks: core 33 + UDS 17 + protocol 8 (one ignored crash helper), native APK parser 2/2, stdio/schema 5/5, TypeScript, permission UI 1/1 and production/new-crates Clippy PASS. Logs `/tmp/lomi-mcp-import-{tests,wire-2,ts,ui,clippy,core-clippy,native}.log`, `/tmp/lomi-mcp-apk-{storage-2,storage-clippy,inspect-tests}.log`; permission screenshot visually inspected. Next: Settings-bound APK installation approval and exact-copy native installer, package/activity launch and bounded logcat; input qualification still needs the host unlocked. Continue all remaining P4/P5/P6, not an MVP completion.

Next in progress: APK install adapter and Settings approvals implemented, pending native qualification. `AndroidInstallInput` binds workspace/panel/device/generation and artifact ID/hash, durable retry epoch/key. Native Settings alone consumes an approval (120 s); running installation has 180 s, one per device, at most two running/eight pending. Same private artifact descriptor is quota-pinned, rehashed and rewound; source path is never reopened. Existing manager actor/ADB installer gained revocable guarded dispatch/IO, preserving the ordinary native file-dialog path. No uninstall fallback. Bounded AOSP binary-manifest metadata supplies package/version identity; fixed-package dumpsys reads actual versions, and only bounded INSTALL_* codes leave installer errors. Transport ambiguity stays outcome_unknown. Core 33 + UDS 18 + protocol 8, native APK/installer unit 3/3, 38-tool stdio/schema 5/5, TypeScript, permission/approval UI and production/new-crates Clippy PASS. Latest native evidence still lZY77Q/37 tools; 38-tool install trial is next. New manifest corruption tests and `mcp-probe` check currently running (`/tmp/lomi-mcp-install-{probe-check,manifest-tests}.log`).

APK install native qualification: `lomi-mcp-control-Dw6Bc8` **PASS** with 38 tools. Native Settings denied the first request, denial retry remained cancelled; main could not approve. Settings then approved the concrete hash/device/generation, the source file was changed before installation, and native Android installed the retained private copy: `org.lomi.inputtest`, observed previous/new version code `1`, hash `5ed9daedfbb058b8a42eec463507d1333d06a33f6be122ce4b36d74a109ba48d`. Retry returned the same operation. `apk-install.json` record the result. The native approval PNG was inspected but the auto-scroll clipped the request details; the mock UI PNG shows the complete card. Fixture scrolling is corrected for the next native capture. Browser/PTY/snapshot/capture regressions passed; Android input still NOT_RUN_SCREEN_LOCKED. Phone/private ADB stopped without force, host exited 0 and fixture app data removed. Log `/tmp/lomi-mcp-install-native-2.log`. Earlier `R185U9` stopped before APK on a legitimate optimistic revision conflict while revealing a browser; fixture now records at most three attempts, refreshing/new key only after a definitive failed/no-effect REVISION_CONFLICT, never after uncertain effects.

Latest validation: core 33 + UDS 18 + protocol 8 PASS, native APK/installer 3 tests, compiled-manifest 2 tests, real smart-socket guard 1 test (stalled ACK and mid-transfer revoke close before completion), stdio/schema 5/5, native-probe compile, TypeScript and Settings permission/approval UI PASS. Production and new-crates all-target Clippy PASS (`/tmp/lomi-mcp-install-{core,native-unit,manifest-tests,adb-guard,wire,probe-check,ts-2,ui,clippy-3,core-clippy}.log`). New guard test was added after Clippy; run it in final all-target checks. Next: launch package/activity, bounded package logcat, full E04 build from MCP-owned PTY, unlocked input matrix; then ALL remaining P5/P6. Existing read-only build dependencies found: `/Library/Java/JavaVirtualMachines/zulu-17.jdk` and `/Users/woro/Library/Android/sdk/{build-tools,platforms}`; use compilers without touching the user's SDK/emulator, write all fixture outputs to the approved temporary project. Current fixture still imports a prebuilt APK; do not claim MCP build-to-install yet.

2026-09-23 current increment: `lomi_android_launch` and `lomi_android_logcat` implemented; catalog 40. Separate Settings scopes and a maximum of 16 exact approved package names bind both tools. Launch uses one-use native dispatch, the existing actor and a fixed guest intent operation; receipt confirms intent delivery only. Logcat reads the running main PID and unique app UID, rejects shared/system UIDs, limits the main-buffer tail to 256 lines/64 KiB and explicitly reports incomplete/unknown gaps. Random RAM cursors page one snapshot, bind target/filter/owner and expire after 60 s. No global log dump, arbitrary guest shell, clearing, force-stop or deep link was introduced.

Checks before native execution: core 33 + UDS 19 + protocol 8 PASS; native bounded log/UID test PASS; production workspace and helper/core/protocol all-target Clippy PASS; TypeScript and Settings UI test PASS, rendered package controls visually inspected. Native fixture now builds its test APK through `lomi_terminal_run` in an owned idle PTY, uses build output for hash/size, installs the imported copy, launches through the production tool and checks a Unicode log marker. It reads the local JDK/SDK build tools without modifying their installation and retains the same isolated test signing key. Native `1pGhEe` passed (`/tmp/lomi-mcp-apps-native-2.log`). Build exit 0/hash/size came from the owned MCP PTY; actual package launch and Unicode app log marker, pagination/filter binding and denied foreign package passed. Full native installation approval card was visually inspected. Cleanup confirmed no running fixture guest/private ADB, host exit 0 and isolated app-data removal. First run `KZ6wwS` failed before pairing when repeated output schemas exceeded the test catalog budget; helper now retains generated closed per-tool result variants and only reachable definitions, reducing the catalog from 1,075,481 to 279,890 bytes. The cached catalog and protocol/schema tests pass. Input remains NOT_RUN_SCREEN_LOCKED; CGSSessionScreenIsLocked was reconfirmed true. P4 is not complete, and P5/P6 remain required.

Next concrete work: native mismatched-signature install rejection while retaining the installed app; unlocked Android input/form/rotation/two-view matrix remains blocked only on host focus. Then remaining P2/P3 guards and all P5/P6 domains. No active fixture resources remain after `1pGhEe`; the authorized SDK/AVD is retained and stopped. App-identifier injection assertions passed (protocol 9/9), and insufficient-storage installer-code classification passed. This tests result classification, not real guest disk exhaustion. Mismatched-signature native `U0qTez` PASS (`/tmp/lomi-mcp-signature-native.log`): second APK was actually built in the MCP PTY with a temporary different signer, imported, approved and rejected by Android with INSTALL_FAILED_UPDATE_INCOMPATIBLE / failed / effect none. Retry retained the same failed receipt; the original installed app then launched and logged successfully. Host exit 0, isolated app data removed and fixture guest/private ADB stopped. P5.1 disk-read/list contracts are recorded for the next independent increment; no new file tools are exposed yet.

P5.1 beginning: descriptor-based `ProjectFile::read_bytes` now enforces a 4 MiB allocation ceiling, cooperative checks every 64 KiB and unchanged-file validation. Its mutation/revoke/budget test is running (`/tmp/lomi-mcp-files-foundation.log`). No file tools are exposed yet. Next implement `lomi_files_read` through native editor decoding, then bounded directory listing/search and buffer open/read/edit/save. Settings must explicitly approve general file reads; APK-import permission must not silently expand into generic file/buffer access. Continue remaining P4 native input after host unlock; current screen-lock question is still pending and should not be repeated. Entire v1 P5/P6 scope is still required.

P5.1 disk read implemented: `lomi_files_read` is tool 41, with FilesReadInput/FileText schemas, strict files.read scope, pinned NOFOLLOW project descriptors and bounded native editor decoding. Paging uses exact disk SHA-256 and UTF-16 boundaries while preserving raw line endings. Settings now explicitly grants general disk reads; APK import depends on that visible permission and unchecking it clears dependent import/install grants. Core 34 + UDS 20 + protocol 9 PASS (`/tmp/lomi-mcp-files-core-2.log`); native decoder test, TypeScript, Settings UI, workspace and core/helper all-target Clippy, and stdio tests 5/5 PASS. UI file-permission controls were visually inspected. A revoke test initially expected an error reply after global revoke; it now correctly accepts either CONTROL_REVOKED or the forcibly closed authenticated connection, never successful content. Native 41-tool run is in progress in `/tmp/lomi-mcp-files-read-native.log`.

Next: finish native disk-read proof, then descriptor-safe directory listing/search and full shared editor buffer operations; all remaining P5/P6 and native input qualification are retained in scope. Rustix 1.1.4 is already present transitively; inspect its descriptor-based Dir adapter as a candidate for safe listing before adding any direct dependency. No directory-list implementation or new dependency has been added yet.

Native file-read `lomi-mcp-control-xf2Ymh` PASS (41 tools): `files-read.json` verifies disk source, exact UTF-16 pagination across Polish text/emoji, original CRLF, SHA revision, denied .env and foreign workspace. All prior native APK build/install/signature rejection/launch/log/snapshot/capture and browser/PTY assertions passed. Cleanup confirms host exit 0, isolated app data removed, guest/private ADB stopped; input explicitly NOT_RUN_SCREEN_LOCKED.

Directory listing is now tool 42, pending native qualification. `ProjectDirectory::list` uses Rustix 1.1.4 `Dir::read_from` on an approved descriptor, NOFOLLOW statat, secret/link/hard-link/special-file exclusion and before/after directory identity checks. It bounds enumeration to 10,000 entries/1 MiB and pages to 200 entries/48 KiB; cursors bind owner/workspace/path and listing revision. Rustix was already locked transitively; it is now a pinned direct Unix-only dependency. Core 35 + UDS 21 + protocol 9 PASS (`/tmp/lomi-mcp-files-list-tests.log`). Probe compilation, Clippy and stdio tests are running (`/tmp/lomi-mcp-files-list-*.log`). No fixture process remains active after xf2Ymh.

Directory-list native qualification: `lomi-mcp-control-9fdcD0` PASS with 42 tools (`/tmp/lomi-mcp-files-list-native.log`). `files-list.json` confirms paging, secret/absolute-path exclusion and cursor expiry after an external directory change. Browser/PTY/APK launch/install/signature rejection/logcat/snapshot/capture regressions passed. Input remains explicitly NOT_RUN_SCREEN_LOCKED. Native host exit 0, isolated app data removed, fixture guest/private ADB stopped without force. Listing core 35, UDS 21, protocol 9, Clippy and stdio 5/5 passed.

P5.1 search implementation is in progress as tool 43. Native matcher reuses `files/search.rs` query/filter/preview helpers and editor decoding while all traversal/reads use approved descriptors. Bounded per-file disk observations include SHA-256 and UTF-16 positions; no editor buffer is read. Cursors bind connection/workspace/path/query/pinned root/policy, last at most 60 seconds and retain at most eight 128 KiB batches. The filter policy is explicit include/exclude patterns plus secret-path rejection; Git/global ignore files are not consulted. Implementation/test work is active; no native qualification claim yet. Continue editor open/read/apply/save, file mutations, all P5/P6 and unlocked P4 input afterwards.

Search native qualification `lomi-mcp-control-jSmQNw` PASS with 43 tools (`/tmp/lomi-mcp-search-native.log`). files-search.json confirms Unicode UTF-16 positions, CRLF/CR line normalization, secret exclusion, cursor query binding and foreign-workspace denial. All prior native fixture assertions passed; Android input remains NOT_RUN_SCREEN_LOCKED. Cleanup: host exit 0/app data removed, fixture phone/private ADB stopped without force. Search native matcher tests 5/5, core 35 + UDS22 + protocol9, both Clippy runs and stdio5 passed.

Editor read is implemented as tool 44, pending native qualification. It requires an explicit editor.read checkbox plus files.read and a current file panel. Main reads the existing retained CodeMirror buffer, reporting a document UUID/monotonic buffer revision and tracked canonical disk-load provenance; no implicit open or disk fallback. Core rechecks scope/root/path/identity and revocation around a correlated main-only two-second reply. UTF-16 paging preserves surrogate boundaries; dirty state is computed synchronously, not from the UI debounce. Core35/UDS23/protocol9 and TypeScript PASS. UI dirty-buffer/revision/undo and Settings tests 2/2 PASS; the editor screenshot was visually inspected. Native probe has competing real CodeMirror transaction + disk-versus-buffer + undo assertions and is ready for execution. Next: native proof, then editor open/apply_edits/save, file mutations and remaining P5/P6. No completion claim for full MCP.

Editor native first attempt ONIBpC correctly refused the initial read as TARGET_NOT_FOUND: the native projection canonicalizes `/var/...` to `/private/var/...`, whereas the UI session retains the original spelling. Fixed the typed request to carry the already-authorized project ID; main selects by project/workspace ID and matches the panel root against its UI project, while the broker still compares the actual buffer disk-load source to the canonical approved root/path. The canonical-path proof was not weakened. Targeted UDS test and TypeScript PASS after fix. Retrying `/tmp/lomi-mcp-editor-read-native-2.log`; no frontend changes during that run. Previous run cleanup host exit 0/app data removed. All 44-tool schemas/wire5 and Clippy checks passed before this correction.

Editor-read native retry `lomi-mcp-control-wyT6z7` PASS with 44 tools (`/tmp/lomi-mcp-editor-read-native-2.log`). `editor-read.json`: source buffer vs unchanged disk, real competing CodeMirror transaction, stale buffer revision rejection, stable document ID, undo and cross-workspace denial. Full existing browser/PTY/Android regression passed; input still NOT_RUN_SCREEN_LOCKED. Host exit0/app data removed, fixture guest/private ADB stopped without force. No active fixture after this run.

`lomi_editor_apply_edits` implemented as tool45; native qualification pending. Scope editor.write separately depends on editor.read/files.read. Up to64 sorted/nonoverlapping edits, UTF-16 scalar boundaries, LF Unicode and64KiB inserted bytes, document/buffer/disk/layout expected revisions. All validation precedes one isolated-history CodeMirror transaction, no disk save. Durable receipts, one-use claim/ACK, bounded deadline, strict result identity/counter progression and queued revoke are wired. Core35/UDS24/protocol9, TypeScript, pure editor6/6 and shared runtime UI tests passed. Scope dependency UI passed. Clippy initially caught an oversized receipt enum; EditorEdited is now boxed without changing its wire schema and production Clippy passes. Initial wire failure was a test argument accidentally carrying terminalSessionId into the closed editor schema; corrected, repeat is running. Need finish wire/all-target Clippy and native45 proof before continuing editor open/save and filesystem mutations, then all remaining P5/P6. Do not finish the overall task here.

Native editor edits `lomi-mcp-control-9LKHyq` PASS with 45 tools (`/tmp/lomi-mcp-editor-edits-native.log`): two MCP edits were applied to the real retained CodeMirror buffer once, duplicate request returned the same succeeded receipt, an emoji-splitting second range failed with effect none and left the entire document/revision unchanged, one undo restored the original clean text. `editor-edits.json` records receipts and exact buffers; `editor-edits.png` was visually inspected. Entire prior fixture passed, Android input remains NOT_RUN_SCREEN_LOCKED. Cleanup host exit0/app data removed; guest/private ADB stopped without force. Final wire5 PASS after updating the closed-schema test arguments and its explicit mutation-annotation list; both Clippy suites PASS, unit6/UI/runtime/permission assertions PASS. Core35/UDS24/protocol9 PASS before boxing EditorEdited, then all-target Clippy compiled the boxed tests.

Next editor open/save analysis identified a source-provenance dependency: ordinary UI reads canonicalized a pathname but then reopened it with File::open, allowing a replacement link between these steps. Hardened the existing Unix editor read to walk the resolved absolute path by NOFOLLOW directory FDs, including final file, then recheck inode/mtime/ctime/size/nlink on both the held FD and current named file. Explicit user-selected aliases still resolve normally; MCP separately rejects secret actual-source aliases. Windows remains unchanged/unqualified. Native editor regression tests are running in /tmp/lomi-mcp-editor-source-tests.log. Finish these and add editor open via existing openFileTab/shared buffers, then descriptor-safe save and filesystem mutations; full P5/P6 remain required. No fixture process remains after9LKHyq.

Source-provenance hardening native editor tests6/6 PASS (`/tmp/lomi-mcp-editor-source-tests.log`), including last/intermediate replacement links, explicit user alias to .env, encoding roundtrips, atomic save conflict, executable permissions and new-file protection. Next editor-open DTOs are added in protocol/editor.rs but are not wired or exposed yet. Implement receipt-bound pinned native load + staged shared-editor opening through existing openFileTab; preserve dirty documents and avoid component fallback to a different read. EditorOpenInput/Command/PreparedEditorFile/Opened contract is in REQUIREMENTS. No new dependency. Full next scope remains editor open/save, filesystem mutations and all P5/P6.

2026-09-23 continuation: user confirmed the screen is unlocked; native Android input runs now use the default required mode. Editor-open tool46 is fully wired through pinned native preparation, one-use operation, staged shared CodeMirror load and domain openFileTab. TypeScript and UI dirty/shared-open tests passed; full core35 + UDS26 + protocol9 passed after fixing a shutdown race by joining cleanup workers before returning (16 immediate restart cycles included). Native avOmdR failed a fixture field-name assertion, corrected panelId to id; 6rYOCg exposed the expected dirty-indicator debounce after undo, fixture now waits for the actual clean guard; aIoT3w verified open/reopen/retry/shared dirty text/undo/secret denial and visually inspected editor-open.png, then hit a safe no-effect revision conflict in subsequent rename. Fixture now focuses the original editor before closing the added one and only retries a definite no-effect revision conflict with a fresh revision/key, at most3 attempts. Native46 run4 is /tmp/lomi-mcp-editor-open-native-4.log; no overall PASS claim yet.

Atomic save foundation: core atomic_file::replace pins the canonical parent with NOFOLLOW descriptors, hashes/checks the exact current file, stages with openat EXCL, preserves ordinary ownership/mode, rechecks identity/revision, then renameat + parent fsync. A post-rename durability error is explicitly uncertain. This helper is now used by ordinary Unix editor saves; MCP save is NOT exposed yet. Its2 core tests passed (stale revision, executable mode, cancellation and parent replacement link), all6 existing native editor tests passed. Non-Unix code is retained and unqualified. No new dependencies. Pending: finish native46/unlocked input, all-target/wire checks, expose save through operation-bound buffer/revision authorization, all filesystem mutations and remaining P5/P6.

Unlocked native follow-up: vX2Afk completed editor46, terminal/browser, Android build/install/launch/log/snapshot/capture, then Android claim succeeded with no surviving RAM lease. android-running-ui.json confirms nativeFocused=true and document visible; it also exposed a missing core:window:allow-is-focused permission. Added that narrowly to main only. Input revocation diagnostics are compiled only with mcp-probe. qvni6A stopped earlier at the browser takeover assertion because postEvent only queues AppKit input; native fixture now waits up to2s for the actual registered monitor to consume/revoke before checking denied reads. Runtime browser authority was not weakened. qvni6A cleanup stopped owned resources/app data; host SIGTERM during fixture cleanup, unlike previous normal exit0. Current retry /tmp/lomi-mcp-android-unlocked-native-2.log also builds a form button/result in the guest and submits via MCP touch at snapshot-derived bounds. No form/input PASS yet. Wire46 5/5, production workspace Clippy and new-crates all-target Clippy PASS. Atomic-save additional external-edit/final-link test added; re-run pending. Keep all remaining v1 scope, including tool47save and P5/P6.

Current resume point: catalog still46. Tool47 editor-save backend/DTO/native command are now implemented and compile; the one-use native write / exact persisted ACK / stale disk / missing files.mutate / alias denial test passed (/tmp/lomi-mcp-editor-save-core-test-2.log). Its first test caught a double record_result attempt (receipts are immutable): ACK now validates and retains already-persisted save evidence before transitioning. Save frontend/runtime/Settings/catalog/output schema/tests/native proof are NOT done; do not expose47 yet. Helper now rejects names absent from its effective catalog before Request deserialization, so the in-progress save request is not callable over stdio. atomic_file3 tests PASS, UI native editor6 PASS.

Android diagnosis: FqqRx1 reproduced successful claim with leaseId null; eF7dJX instrumented main invocation and showed blur=[] plus the active Take control badge before cleanup. Thus later native blur logs came from cleanup and do NOT prove why lease enrichment returned null. Temporary debug_assertions logging in broker/terminals.rs now records boolean lease-enrichment conditions; remove after resolving. mcp-probe-only native release diagnostics also exist. Cfk0Ra failed earlier with PANEL_NOT_RENDERABLE because native focus had returned while WK visibility/decoded stream were still settling (android-running-ui.json nativeFocused=true, streams empty). Fixture now awaits actual runtime.agentInputReady(panel,generation) with decoded frame up to8s before claim. Current run /tmp/lomi-mcp-android-unlocked-native-5.log, session5404, default required input mode. The new guest Submit Unicode form is compiled, and snapshot-derived MCP down/up touches + exact result assertions are ready but have not reached execution. Preserve user original emulator/ADB5037. All previous failed fixtures have native cleanup evidence; fullMCP remains incomplete and all P5/P6 are still required.

Android input root cause confirmed in nPZQg3: stack from android_agent_input_blur points to the inputControl event callback itself. Tauri2.11.5 EventTarget::Any listeners receive both separately targeted main/settings emissions (installed primary source manager/mod.rs + event/listener.rs). Runtime also unconditionally revoked on duplicate controlled=true for the same lease. Fixed Android state listener to target its own Webview label and reject stale subscription epochs; runtime now treats identical device/generation/view/lease notification idempotently while still rechecking visibility/guards. All3 UI lease-release tests now first repeat the active notification, assert zero premature blur, then verify exactly one release on dialog/field/hidden panel; PASS /tmp/lomi-mcp-android-duplicate-ui.log. Temporary core/native/JS stack diagnostics removed. Fixture waits for decoded input-ready frame and includes the new submitted Unicode form. Current full native46 retry is /tmp/lomi-mcp-android-unlocked-native-7.log. Full latest core38+UDS27+protocol9 PASS in /tmp/lomi-mcp-editor-save-core-suite.log, TypeScript PASS before save frontend integration.

Save frontend is being prepared independently while native fixture runs: staged files and SHA baseline at /var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-save-ui-tqi5ap4m. Four src files: editor-control.ts DTOs, editor-runtime.ts saveAgent captured state/native IPC/update saved state without undo reset, agent-control.ts editor_save handler, AgentControlSettingsPage.tsx explicit dependent saveFiles checkbox granting files.mutate. NOT copied into repo yet; validate baseline hashes before copying after fixture exit. Need audit staged patch placement, TS/UI checks, catalog47/schema/wire/native save proof. Helper catalog now borrowed cached slice; unknown tools are rejected before enum deserialization (keeps47unexposed until registered).

### Unlocked Android and editor-open native result — 2026-09-23

Full helper/app/native run `lomi-mcp-control-f35ejS` PASSED with all 46 current tools. Log: `/tmp/lomi-mcp-android-unlocked-native-7.log`. Actual Android input lease is non-null, exact Unicode `MCP Zażółć gęślą jaźń 🙂` reaches the guest; duplicate sequence does not type twice, changed payload conflicts, two MCP touch packets submit the observed button, and the guest result is exactly `Submitted: MCP Zażółć gęślą jaźń 🙂`. `android-input.json`, `android-form.json`, guest hierarchy and visually inspected `android-controlled.png` confirm behavior. Release rejects stale lease. Owned emulator process and private ADB stopped; host exit 0, isolated app data removed. User's existing emulator/shared ADB were untouched. This clears the screen-lock dependency and verifies the duplicate-event fix, but does not claim the remaining rotation/keys/two-view/two-agent/fault matrix.

Editor open in this same run passed real shared-buffer open, dirty reuse, retry, undo and clean close. Editor save frontend has now been integrated from the verified staged baseline. UI captured-revision/concurrent-human-edit/undo test PASS; explicit dependent save permission test PASS. TypeScript PASS. Tool 47 `lomi_editor_save` is now registered with closed schemas and correct mutation hints; independent helper wire 5/5 PASS (`/tmp/lomi-mcp-editor-save-wire.log`). Full native47 save/CRLF/dedup/undo run is in progress: `/tmp/lomi-mcp-editor-save-native.log`, artifacts `lomi-mcp-control-shc8fs`. Full pnpm check and production workspace Clippy are also running. P0–P6 remains IN_PROGRESS; continue remaining P5/P6 after this increment.

### Editor save native PASS and file creation next — 2026-09-23

Native47 run `lomi-mcp-control-S6asc3` PASSED (`/tmp/lomi-mcp-editor-save-native-2.log`): real MCP editor save, exact UTF-8/CRLF disk bytes, saved buffer/disk revisions, deduplicated retry, undo retaining saved disk, redo returning clean, and a second revisioned save restoring the disk fixture. Full existing native Android/browser/PTY/Codex assertions also passed. Previous `shc8fs` passed the save assertions and failed only because the later disk-read fixture still expected pre-save contents; corrected by restoring that shared fixture through a second MCP save. Save screenshot visually inspected. pnpm check PASS, production workspace Clippy PASS, wire5/5 PASS, two targeted UI tests PASS. Native host/private emulator cleanup confirmed by fixture.

Next increment `lomi_files_mutate` creation is in progress and NOT catalog-exposed. Closed protocol create variant, pinned-parent empty file/directory creation and one-use broker writer have been added. Two descriptor creation unit tests PASS; core integration check running. Add `files.create` alongside files.mutate/files.read so the existing save checkbox cannot silently grant creation. Shared native result persistence has been extracted from editor-save to support both real writers; rerun core/save tests. Remaining work: native IPC, Workbench file-operation busy/pause integration, explicit creation permission UI, UDS/behavior tests, catalog48/schema/native proof. Rename/move/trash remain required follow-on variants.

### File creation native PASS; regular-file rename/move in qualification — 2026-09-23

Native48 `lomi-mcp-control-iD03oY` PASSED (`/tmp/lomi-mcp-files-create-native.log`), including explicit files.create approval, creation of a private directory and nested empty file, same-receipt retries and refusal to overwrite user-written bytes at an occupied destination. Full editor/browser/PTY/Android/Codex regression passed; cleanup removed fixture app data and stopped private resources. Core40+UDS28+protocol9, wire5, UI2, pnpm check and production Clippy PASS for this increment.

`lomi_files_mutate` now also implements `rename` and `move` for regular single-link files up to4MiB, within the same approved project. Additional scope files.rename (separate checkbox) is required along with files.mutate/files.read; creation/save grants do not imply it. Both variants require source expectedDiskRevision SHA-256 plus destination expectedParentRevision from files_list. Rename takes one newName; move takes targetRelativePath. Pinned parents/NOFOLLOW stable source snapshots + rustix1.1.4 renameat_with(NOREPLACE) prevent destination clobber; no cross-device copy/delete fallback. Existing source inode/permissions survive. The main Workbench reuses applyFileChange/relocateEditorFiles while holding the existing file-operation busy guard and pausing disk refresh/saves; dirty buffers/undo survive. Native canonical source provenance is carried across model aliases (/var versus /private/var) to keep subsequent editor read/save authorized. Directory rename/move and trash remain TODO, explicitly absent from the exposed contract.

Foundation3tests, core41+UDS29+protocol9, wire5 and three permission/explorer UI tests PASS; TypeScript PASS. Full native current run `/tmp/lomi-mcp-files-move-native.log` (exec session56610): checks dirty editor rename, cross-directory move, unchanged disk bytes, same-document read through new paths, retry after source disappears, undo and clean close. Inspect this result and screenshot before advancing. Native fixture active means do not edit src frontend until it exits.

Primary API evidence: https://docs.rs/rustix/1.1.4/rustix/fs/fn.renameat_with.html and installed1.1.4 fs/at.rs + backend/libc/fs/types.rs map NOREPLACE to macOS RENAME_EXCL. macOS ARM64 was tested; Linux implementation is compiled by conditional source but not host-qualified.

Native rename/move first run `ViD07U` exposed a pre-existing nested editor-open bug before rename: fingerprint resource_id used the raw relative path, but receipt IDs disallow slash/Unicode. Changed editor_open resource ID to bounded SHA-256 of the already validated path (same established pattern as artifact import). Entire actual input remains fingerprinted. Updated existing UDS editor-open test to `nested/Zażółć 🙂.txt`; PASS `/tmp/lomi-mcp-nested-editor-open-test.log`. Retry full native48 move now `/tmp/lomi-mcp-files-move-native-2.log`, exec70562; production Clippy queued/running in `/tmp/lomi-mcp-files-move-clippy.log` exec15891. Do not touch frontend while fixture active.

### Directory move qualification and event pagination — 2026-09-23

Regular-file native run `Rf7xsu` passed nested editor open, file rename/move, dirty-buffer/source provenance, retry after missing source, undo and clean close, plus the complete Android input form and browser/PTY checks. `files-move.png` was visually inspected. The final fixture check failed because it assumed all workspace operation events fit one100-event page; new file operations exceed this. Fixed fixture to drain up to32 bounded100-event pages, require strictly increasing sequence numbers and exact workspace for every event, require the cancellation receipt, and reach an empty page. The broker limit was not increased and pagination was not bypassed. Full run still needed for final PASS.

Added explicit `rename_directory` / `move_directory` DTOs/native adapter (files.rename grant), using source expectedDirectoryRevision and destination expectedParentRevision, exclusive descriptor-relative rename, no byte recursion/copy, descendant-preserving source provenance updates. Native foundation4tests PASS; protocol/helper wire5 PASS; TypeScript and three permission/dirty-folder/editor UI regressions PASS. Current full native48 run `/tmp/lomi-mcp-directory-move-native.log`, exec86387, tests file then directory renames/moves with the same dirty document, followed by full existing regression and corrected event-pagination check. Do not edit frontend while active. Directory-move core full suite and latest Clippy still to run. Trash and all remaining P5/P6 items remain required.

### Directory native assertions and Android matrix continuation — 2026-09-23

`njQgCi` native run passed all editor save + file/directory create/rename/move assertions, including dirty nested document preserved at `mcp-container/mcp-renamed/final.txt`, exact disk bytes, receipt replay and undo/close. `files-move.png` visually inspected. The run later failed during Android form touch with CONTROL_REVOKED (after successful guest Unicode typing); native protection stayed fail-closed. Cleanup confirmed host exit0, owned emulator stopped and private ADB stopped. Do not report this run as overall PASS. Core42+UDS29+protocol9 PASS (`/tmp/lomi-mcp-directory-move-core-suite.log`), latest production workspace Clippy PASS, wire5/5, UI3 and TypeScript PASS.

Native fixture now captures `failure-ui.json` (focused/visible/minimized, main document visibility/focus/body) and `failure.png` BEFORE teardown to diagnose future revoked focus leases. Also added an explicit Explorer refresh wait before file-move screenshot. New Android matrix assertions test actual Backspace down/up removing a whole emoji, Unicode restoring exact text, quarter-turn1 landscape and0 portrait snapshots/capture geometry. Uses existing MCP lease/sequence4..8 and existing managed input router, no bypass. Fixture helper `Wire::android_packet` settles each exact input receipt. First compile xA85d4 found two moved JSON values; changed capture_args and form_snapshot_args uses to clone. Fresh run `/tmp/lomi-mcp-android-keys-rotation-native-2.log` is active; previous log without-2 is a compile failure, not qualification. No frontend changes until active fixture exits.

### Android rotation and Trash adapter continuation — 2026-09-23

`WN0dQm` and `7cohe3` native runs failed only at the new post-Backspace assertion: the guest held the exact restored Unicode text, but UIAutomator encoded the emoji as `&#128578;` after key input. Added a fixture XML parser using quick-xml normalized attributes, preserving exact EditText/content-description matching. `3cAV5z` then passed Backspace, text restoration and landscape rotation/capture, and exposed a real lease revocation at the subsequent portrait rotation. `runtime.ts` intentionally replaced the stream on resize and revoked ownership during that replacement. It now pauses readiness until the first decoded replacement frame, retains the same-generation lease for at most5s while the visible/focused context remains valid, and still revokes on actual disconnect, modal, field focus, hidden panel or window blur. UI resize/paused-readiness/modal test PASS plus the3 interruption tests; permissions UI PASS. Full native regression remains required.

Trash has been added to `lomi_files_mutate` with separate files.trash + files.mutate + files.read scope. Native core pins source/parent descriptors, validates file SHA or immediate-directory revision and parent revision, journals a private recovery plan before exclusive same-volume staging, then invokes macOS NSFileManager via the installed trash5.2.7 explicit NsFileManager context. There is no copy/delete fallback, silent rollback or automatic removal of unresolved recovery data. Failure after staging is outcome_unknown and retains the original entry under agent-control/trash-recovery/<operation>/entry/<name>. Recovery metadata quota128 is currently conservative; user-facing recovery/retention management remains to implement before complete acceptance. Regular file bound4MiB; no root/link/hardlink/secret-path removal. Source data and inode are preserved during staging. Foundation3tests PASS, full core45+1ignored/UDS29/protocol9 PASS before the newest UDS test; new UDS Trash decision/replay test PASS.

Dirty-buffer decision is an authenticated main-only two-step native plan, bound to command nonce/uiEpoch/deadline and exact document/buffer/disk revisions. Generic UI ACK cannot approve it. Main claim leaves Trash queued; prepare transitions dirty work to awaiting_user, clean work to running. Only the existing editor dialog event invokes the separate decision command; discard authorizes the exact plan, Cancel cancels without touching disk, Save retains the saved file and cancels this removal so the agent must request the new disk version. Main file-operation lock and existing pause/domain/close guards surround dispatch; buffers created/edited while awaiting are rejected. Stop/cancel/expiry invalidate native approval, but automatic dismissal of a stale dialog is still outstanding. Three existing close/save/discard UI regressions PASS; pnpm check PASS.

Native Trash fixture covers real helper request, main dialog, Cancel preserving dirty buffer, Save preserving newly saved disk, Discard moving exact bytes/inode to system Trash, deduplication and editor panel removal. It uses a unique invocation-named fixture and removes only that exact Trash entry after bytes/inode verification. Also reruns the Android key/rotation fix and all previous48-tool assertions. First run50yNnU failed compilation (Value passed to&str); corrected. Current run `/tmp/lomi-mcp-trash-android-native-2.log`, exec2607. Do not edit frontend until it exits. Latest production Clippy initially found one needless borrow; fixed, rerun still needed. Inspect native result, screenshots and cleanup; then implement pending Trash cancellation/recovery and image/SVG/Markdown previews. All remaining P5/P6 scope stays required; this is not full v1 completion.

Native48 `lomi-mcp-control-0o22LS` PASSED end-to-end (`/tmp/lomi-mcp-trash-android-native-2.log`). This includes all create/file+directory move/save assertions, all3 actual Trash choices and exact system-Trash bytes/inode verification, corrected event pagination, Backspace/Unicode restoration, landscape then portrait rotation with the same input lease and valid capture geometry, and existing browser/PTY/real-Codex assertions. Owned emulator stopped gracefully, private ADB stopped, host fixture exited0. Production workspace Clippy PASS (`/tmp/lomi-mcp-trash-clippy-2.log`); helper-only wire2/2 PASS (full5-test suite still to rerun for final P5 acceptance); pnpm check PASS and close/permission UI regressions PASS.

Follow-up now in qualification: read-only native pending-plan probe lets the existing editor guard dismiss itself when MCP cancels, native Stop revokes, deadline expires or captured document/layout changes. It does not dismiss an active Save until that human save settles. Unmount resolves the guard false. Added Settings-only fixed-path “Show recovery folder” action, available even with broker off; no arbitrary path parameter, no automatic restore/delete. UI explains plan.json/entry/completed.json and preserving newer files during manual recovery. Native fixture now also cancels an awaiting Trash operation through lomi_operation_cancel and requires modal dismissal plus retained dirty buffer/disk. Current run `/tmp/lomi-mcp-trash-cancel-native.log`, exec97818; avoid frontend edits until it exits. `pnpm check` and3 targeted UI regressions running in matching cancel logs. Recovery quota128/retention maintenance and broader fault qualification remain P6 work. Next primary implementation: image/SVG/Markdown preview through scoped pinned reads, then remaining P5 domains.

Native48 cancellation follow-up `lomi-mcp-control-R6zQsO` PASSED (`/tmp/lomi-mcp-trash-cancel-native.log`): awaiting Trash cancelled through MCP dismisses the actual main dialog and retains dirty buffer/disk; user Cancel/Save/Discard still pass, Android form/keys/rotation and all prior native assertions pass. Host wrapper exit0; isolated private resources cleaned. pnpm check +3 close/permission UI regressions PASS. Added native domain-revision rechecks at Trash prepare/approval/commit and pending visibility (after this native run compiled); latest full core45+1ignored, UDS30, protocol9 PASS `/tmp/lomi-mcp-trash-cancel-core-suite.log`, including pending probe assertions. Do not attribute the new revision recheck to the older native binary. Additional recovery UI screenshot/status test and full5-test independent wire suite now running in `/tmp/lomi-mcp-trash-recovery-ui.log` and `/tmp/lomi-mcp-trash-all-wire.log`. No active native fixture after97818exit.

## Active increment — P5.1 previews (2026-09-23)

48 tools remain. Tool46 presentation now implements raster static derivatives,
Markdown/SVG source/preview/split, retained editor reuse, native scoped asset reads
and fail-closed restored descriptors. D34 and REQUIREMENTS specify the contracts.
Native image dependencies add gif0.14.2/color_quant1.1.0 through cargo add while
preserving original feature flags/lock changes. Ordinary human preview behavior
retains its regression tests.

PASS: pnpmcheck; production workspaceClippy (`/tmp/lomi-mcp-preview-clippy.log`);
core45+1ignored, UDS31, protocol9 (`/tmp/lomi-mcp-preview-core.log`), wire5;
decoder2 (`/tmp/lomi-mcp-preview-image-tests-2.log`); old previewUI13 plus newUI2
(`/tmp/lomi-mcp-preview-ui-3.log`). New UI screenshot was inspected. Coalescing
identical pending image reads fixes duplicate work under development StrictMode.

Current native attempt `/tmp/lomi-mcp-preview-native-3.log`, exec78938 includes
Android and all earlier integration assertions. Read result before claiming native
preview PASS. Prior0zyMaR: fixture wrong tool name; priorw5NBG7: PNG actual pixels
PASS, next image request revision-conflicted while the closed panel restored an
editor. Current fixture waits for editor readiness + settled projection revision.
Native sources compile ~70s; do not edit src/ while its Vite/app is running.
A new Rust recovery test for revoked clean prepared Trash is queued separately:
`/tmp/lomi-mcp-trash-prepared-revoke.log`. It verifies the Running→Cancelling→
Cancelled correction in settle_lost yields effect none after reopening receipts.

Next: finish nativepreview qualification, review screenshot, repeat dependent
checks if code changes; continue P5.1 recovery/encoding/fault gaps and P5.2 Git.
Git read-adapter design must cover clean filters as well as fsmonitor/textconv/
external diff and config races. Existing UI git::configured_command uses PATH
and inherited environment, so it cannot be exported directly as a safe MCP read.
No Git adapter/tool was added yet. All P5 including Chat and P6 remain required.

### Native preview result resolved

Fullnative48 PASS `qZ4ywI`, log`/tmp/lomi-mcp-preview-native-4.log`, exec94075 exit0.
See QUALIFICATION for actual pixel/network/editor and cleanup evidence. Latest
pnpmcheckPASS and core45/UDS32/protocol9PASS; clean preparedTrash revokePASS. No
native app/emulator from this fixture remains. Old fixture pagination assumption
was corrected to exact-ID bounded traversal, with no relaxed product guards.

New actual-full-disk fixture: `tests/native/run-mcp-full-disk.mjs` builds a private
32MiB image and runs ignored integration test `tests/full_disk.rs`. The test refuses
the host filesystem or volumes above64MiB. First valid HFS+ runPASS at`gZXZ2N`,
`/tmp/lomi-mcp-full-disk-2.log`: ENOSPC after32747520 filler bytes, atomic save and
Trash plan failed before effect, old bytes/inode intact, no save-temp leak, both
operations succeeded after freeing only fixture filler. The wrapper detaches the
owned image and removes it only on PASS; inspect result.json for final cleanup.
APFS run now in progress in `/tmp/lomi-mcp-full-disk-apfs.log`; check result before
qualification. Native API commands were checked through local hdiutil help; its
macOS27 deprecation warning is not an execution failure.

### ENOSPC and next domain

APFS finalPASS`5NZzmp`, `/tmp/lomi-mcp-full-disk-apfs-3.log`; HFS+PASS`gZXZ2N`.
All owned images detached; successful images removed by wrapper, two failed test
images removed after inspection; logs retained. Decoder3PASS inclEXIForientation
anddownscale. See QUALIFICATION for precise filesystem outcomes and test fixes.
Latest productionClippy `/tmp/lomi-mcp-preview-final-clippy.log` must be read to
confirm completion (exec57295). All native app/emulator fixtures are stopped.
Git remains unexposed. D35 captures a working initial guarded-read prototype and
its required follow-up checks. Continue this foundation and all P5.2–P5.7/P6;
P5.1 broader encodings/retention/lifecycle/fault cases remain open, not hidden.

### P5.2 status native verified; further observations in progress

Native49 PASS wwumF2, `/tmp/lomi-mcp-git-status-native.log`, result.json stage=passed.
`git-status.json` proves exact project-relative paths, omitted .env and symlink,
immutable page across a new working-tree file, foreign-workspace cursor denial,
new observation on fresh read. Full prior Android/browser/PTY/editor/preview/Trash
flow also passed. Owned native app/emulator/privateADB stopped, no active fixture.
Wire5 PASS `/tmp/lomi-mcp-git-status-wire.log`; real UDS status test PASS after
fixing symlink vs missing-file classification; permission UI PASS; pnpmcheck and
workspace all-target check PASS.

Four additional bounded read contracts are defined before implementation:
Git diff/history/commit/remotes. In-progress code adds closed DTOs and guarded
read adapters; url2.5.8 reuses the existing locked URL parser for credential-free
remote authority parsing. New tools are not yet qualified. Run
`/tmp/lomi-mcp-git-observations-test.log` and new wire/native53 qualification before
claiming them verified. Git mutation guards/UI, read-only view integration and all
remaining P5/P6 still required. No user-repository commits or network Git mutations.

### Native53 observations passed; tool54 Git views under qualification

Native53 PASS `eqiRTa`, `/tmp/lomi-mcp-git-observations-native.log`, wrapper4690
exit0. Actual UTF16 patch pagination with Unicode, working/staged distinction,
changed-observation conflict, secret refusal, exact history/commit message and
credential-free remotes all passed; `git-observations.json` retains evidence.
Full earlier Android/browser/PTY/editor/Trash/preview suite passed. No native
fixture remained after this run. Core46+1ignored, UDS34, protocol9 and wire5 PASS.

After that binary: commit file pagination/committer metadata and historical
first-parent/root diff extensions passed real-UDS test
`/tmp/lomi-mcp-git-commit-files-test-2.log`, including a real fixture merge.
Tool54 `lomi_git_open` now has closed DTOs, durable receipt, native one-use
preparation, bounded main-only read permits, original FileDiff/CommitDetails UI
integration, transient cache and persisted fail-closed agentGit marker. Native
followup reads use the fixed guarded Git path. Normal human readers unchanged.
Git-panel close/focus is bound to the creating session; UI projection prunes old
IDs. Error during native preparation settles effect=none before replying.

pnpmcheck and workspacecheck PASS after initial integration; UI2 PASS with
restored denial, staged view rendering, scoped commit diff, no ordinary Git
read calls. Inspected light commit screenshot. UI label subsequently changed
from guarded-view jargon to plain “for this view”; same behavior.

Current native54 `/tmp/lomi-mcp-git-open-native.log`, exec87928; do not edit src/
while it runs. Its fixture includes real Git view rendering, one-use retry, no
ordinary Git reads and scoped close. New UDS view test first found a trailing
slash for empty repositoryRelative; fixed root-path identity, rerun in
`/tmp/lomi-mcp-git-open-core-test-2.log` exec37278. Native54 uses a nested repo
and may predate that empty-root fix; do not claim it qualified that fix. Wire54
first failed solely because the readOnlyHint assertion did not list git_open as
a UI mutation; expectation fixed, rerun still needed. All P5.2 mutations, later
P5 incl Chat and P6 remain open. No user-repo mutation or paid provider request.

### Native54 Git views passed

Full native54 PASS BGgT3s, `/tmp/lomi-mcp-git-open-native.log`, exec87928 exit0.
`git-views.json` shows both existing FileDiff/CommitDetails rendering correct
content, retry same receipt, zero ordinary Git read calls and scoped close, all
through actual helper/native/main. Whole prior suite passed. Wrapper stopped
owned app/emulator/privateADB. Core46+1ignored, UDS35, protocol9 PASS
`/tmp/lomi-mcp-git-open-core-all.log`; wire5 PASS
`/tmp/lomi-mcp-git-open-wire-2.log`. Root empty-path trailing-slash correction
passed UDS `/tmp/lomi-mcp-git-open-core-test-2.log`. Production Clippy found only
new chunks_exact lint; replaced with as_chunks, rerun pending. Native screenshot
capture added for the next combined fixture; current BGgT3s has asserted DOM and
UI-mock light screenshot was inspected. No need to repeat full native only for
this capture addition.

Continue P5.2 all seven Git mutations with git.execute and explicit main UI
approval. Existing `src-tauri/src/git.rs` shared mutation_guard and argument-based
services must be preserved; original readers inherit PATH/config and cannot be
used as MCP read adapters. Implement snapshot/config/index/HEAD/remote checks
before dispatch, dirty-editor guards for pull/discard, exact commit text/identity,
retry/cancel/uncertain effects, real fixture hooks/config changes/local remotes.
Do not execute mutations in the user's actual repository. All later P5 and P6
remain required. No Git mutation tool exists yet.

### Git execute foundation preparation

Latest complete productionClippy PASS `/tmp/lomi-mcp-git-open-clippy-2.log`;
UI old Source Control17 PASS `/tmp/lomi-mcp-git-existing-ui.log` and newGitUI2
PASS `/tmp/lomi-mcp-git-open-final-ui.log`; pnpmcheckPASS. Native54 remains last
full native PASS and no owned native app is running. Fresh HEAD is
4403883ebe583188f79dfe8d7886d2422e49bfcd (desktop icon commit by external work);
its AGENTS diff concerns only icons and is preserved. MCP changes remain dirty.

D36 defines mutation trust and ownership foundation. The existing Android
installer process Tree is being extracted unchanged in behavior into core
process_tree, with Android re-exporting it. windows-sys0.61.2 was already locked;
now explicitly added for the shared Windows Job implementation. Run installer
ownership regressions and core checks before using it in Git. No Git mutation
prototype or exposed mutation tool has been added yet.

### Git execution snapshots and index executor implemented, not exposed

`core/git_execution.rs` now implements captured native Environment, hash-only
Snapshot and sealed IndexPlan for stage/unstage. git_read has a separate internal
execution-preview profile: denies subprocesses/network/writes but permits user
configuration reads, only for a future git.execute-authorized caller. Ordinary
public read calls still use the project-only profile and sanitized environment.
Snapshot binds config/HEAD/refs/index/file bytes and physical identity; two reads
must agree before preview, dispatch repeats against approval. Native stdout/stderr
is bounded/discarded; code execution preserves configured filters and never silently
disables hooks. No mutation tool or grant has been exposed yet.

Shared Tree extraction passed existing installer ownership2 tests +1ignored at
`/tmp/lomi-mcp-shared-process-tree-tests.log`. Snapshot test PASS
`/tmp/lomi-mcp-git-execution-snapshot-test-2.log`: global fixture config changes,
exact file content beyond status, index/HEAD transitions, no raw credential leak,
fsmonitor not run, secret/link/hardlink refusal and cancellation of a config FIFO.
Index executor test PASS `/tmp/lomi-mcp-git-index-executor-test.log`: stale bytes
prevent Git add, actual stage/unstage preserve working bytes, explicitly executed
clean filter produces uppercase staged content, cancellation kills/reaps the
owned filter and sleep child before returning. Fixtures use isolated temp repos.
Source Control now shares only literal index argument construction on qualified
macOS; existing mutation guard/identity/UI behavior retained.

Current checks: `/tmp/lomi-mcp-git-execution-core-all.log` exec(current),
`/tmp/lomi-mcp-git-index-human-regressions.log`,
`/tmp/lomi-mcp-git-index-executor-clippy.log`. Read results. Latest full native is
still tool54 BGgT3s before this foundation refactor; no active native app/emulator.
Next implement broker grant+native plan+exact main UI approval+one dispatch+durable
receipts for tool55 stage/unstage, then all other Git mutations and later P5/P6.

### Shutdown worker ownership fixed; Git stage/unstage end-to-end implemented

Full regression after the execution foundation exposed a real restart race:
`rejected_pin_and_ui_reload_cannot_reuse_authority` reopened the receipt store
while a detached pairing/request worker still held its owner lock. Broker now
registers async timers/observers and blocking workers, closes admission, aborts
and joins async tasks, joins actual blocking completion and then deferred cleanup.
Shutdown itself has a retained owner so cancellation of a close caller does not
detach it. Immediate revoke still does not wait for policy/storage locks.
New deterministic test holds a detached worker, cancels/repeats shutdown, releases
it and verifies no retained Arc before immediate restart. Full core49 PASS +1
ignored, UDS35 PASS, protocol9 PASS: `/tmp/lomi-mcp-shutdown-core-all.log`.

Tool55 `lomi_git_mutate` now has stage/unstage DTOs, separate git.write+git.execute
Settings grants, native one-use approval and shared repository mutation guard.
Main renders exact path/size/hash, repository and HEAD; defaults focus to Cancel,
dismisses revoked/expired plans and preserves file-operation/close guards.
Prepared snapshots are RAM-only. Before-effect rejection and uncertain post-spawn
outcomes are distinct; native evidence is durable before UI ACK and forged success
is rejected. Current native Git workers are tracked through renderer cancellation;
application close revokes and waits for their actual exit before proceeding.
No raw Git stdout/configuration/credentials are returned. Tool annotations declare
code execution/open-world effects. Only stage and unstage are exposed so far;
commit/fetch/pull/push/discard and all later P5/P6 remain required.

UDS exact-approval test PASS `/tmp/lomi-mcp-git-mutation-uds-3.log`: read-only denial,
claim/nonce/one-use binding, user reject, changed bytes/configuration, MCP cancel,
actual stage/unstage, forged ACK refusal, durable replay/conflicting retry, revoke.
UI3 PASS `/tmp/lomi-mcp-git-mutation-ui.log`; actual bridge/Modal Cancel+Escape,
revoke dismissal, one exact approve/commit and no ordinary Git reads in agent
views. Screenshot `test-results/mcp-git-mutation-approval.png` visually inspected.
TypeScript PASS `/tmp/lomi-mcp-git-mutate-ts.log`; production workspace Clippy PASS
`/tmp/lomi-mcp-git-mutate-clippy-2.log` before close-drain addition; wire5 PASS
`/tmp/lomi-mcp-git-mutation-wire.log`. Full native55 currently running:
`/tmp/lomi-mcp-git-mutation-native.log`, exec86883. Do not edit src while its Vite
fixture is active. Fixture now forces private Git configuration (/dev/null global,
no system config) and mutates only the owned temporary repository. Read native
result and confirm wrapper cleanup before further frontend changes.

### Native55 partial proof and rerun

vmbB5F passed all five native Git mutation cases; Git view captures and complete
editor/files/browser/PTY/APK observations also exist. Full run failed at Android
input with CONTROL_REVOKED after main focus true -> false. SDK/emulator/privateADB
cleanup confirmed; no complete native55 PASS yet. Current third run:
`/tmp/lomi-mcp-git-mutation-native-3.log` (read its directory/session output).
Added only fixture Android claim/first-input request-stage evidence; production
input authority remains unchanged. Host screen check now screenLocked=false.
Do not edit src or bring other windows to foreground during its Android phase.

All current core50+1ignored/UDS36/protocol9 PASS; productionClippy PASS and pnpmcheck
PASS; Settings/application-close7 PASS. Exact log paths in QUALIFICATION.md.
Next: settle native55 and cleanup, then all remaining Git variants. Commit-message
probe evidence is `/tmp/lomi-mcp-commit-message-probe.json`; --cleanup=verbatim still
adds a final LF if missing, which must be explicit in the approval contract. Primary
Git docs for commit, var, githooks and ls-files were inspected. No commit adapter or
additional Git enum variant has been implemented. P5/P6 remain unfinished.

### Current handoff — final viewport/transfer qualification

**wNJLEh focused PASS; YWPPVZ full native56 PASS**, both exit0/app-data cleanup.
Six runtime closure artifacts passed in each. Full run retains original running
command receipt after transfer, same PTY/browser/form/editor/Undo and all previous
file/Git/browser/Codex assertions. Android NOT_CONFIGURED in these runs. The mixed
layout and retained native browser form screenshots were visually inspected.
Source is unfrozen; no native fixture remains running. Logs
`/tmp/lomi-mcp-viewport-{stress,full}-final.log`.

Next implementation is project_close and selecting additional existing workspace
grants in Settings. Preserve all preflight/receipt/dirty/native-commit rules in
the existing project lifecycle contract; then project_open, remaining panel kinds
and all other required P5/P6 work. Full MCP is not complete.

### Current implementation — project close (57-tool catalog)

`lomi_project_close` now has a bounded typed input/command/result, all-project
workspace authorization, protected-origin preflight for every descendant, shared
Save/Discard/Cancel UI and atomic latest-session project removal with neutral
selection. A typed nullable preparation precedes native effects; the private
ancestor CAS admits only a matching verified WorkspaceClosure or ProjectClosure.
Ordinary results remain immutable. Historical project receipts require every
original workspace grant and close scope. Project close does not grant root access.
Settings now explicitly selects additional existing workspaces in the same project;
project closure has a separate opt-in and requires all of them. The pending-close
native read runs off the main thread so its dialog timer cannot block the native
browser close callback on the broker mutex.

Targeted UDS project1, UI3 (including shared dirty buffer across two workspaces,
failed Save and edit during native commit), model7, TypeScript and wire5 PASS.
Adding the result exceeded the existing 700KB catalog budget; output schemas now
retain only each mutation tool's real result variants, with full variants for
operation lookup/cancel. Positive and negative typed-result wire tests pass. The
ProjectClosure Rust enum payload is boxed without changing JSON, to keep the
existing enum-size lint. Production/MCP all-target Clippy and full core57+1ignored/UDS42/protocol9
checks PASS after that final internal adjustment.

The native `LOMI_MCP_PROJECT_CLOSE_ONLY=1` profile PASSED at 5ranoM with cleanup.
It enrolls both existing workspaces through Settings, prepares two real idle PTYs
and a hidden WK browser, exercises Cancel/MCP cancel/Save/Discard/exact retries,
checks native contexts, and preserves a separate project/lazy terminal. Guard and
final view images were inspected; the Settings snapshot was above the permission
form (behavior assertions passed). Full native57 regression PASSED at 88IOPg, exit0/app-data cleanup:
`/tmp/lomi-mcp-project-close-full-native.log`. Source is unfrozen.
Project open and Android/chat/plugin descendants remain required, followed by all
remaining P5/P6. This increment does not complete MCP.

### Current implementation — project open (catalog58)

Added separate per-project root/workspace/pinned-directory bindings throughout the
broker; scopes remain immutable and explicitly inherited on each new approval.
Operation key lookup requires projectId once multiple roots are granted; ID lookup
remains limited to granted projects. Cross-project transfer is still denied.
Core57+1ignored/UDS43/protocol9 PASS for the binding refactor, including correct root
reads, key collisions, unapproved clients, rebinding and directory replacement.

`lomi_project_open` now requests an exact Settings approval, opens a neutral scratch
editor, and grants the new root only after native commit and matching publication.
The immutable preparation precedes approval; cancellation/revoke/root replacement
and late ACK cannot extend authority. UDS seven-scenario approval test PASS; wire5
PASS at catalog58 and existing catalog budget. The first UI run exposed a missing
failure ACK in the new wait branch; fixed and the targeted opening test PASS.
Full core/protocol, UI4 and final checks are running. Native
`LOMI_MCP_PROJECT_OPEN_ONLY=1` is implemented, not yet run: reject, MCP cancel,
replaced folder, two approved roots with equal keys, exact retries, original dirty
buffer and unchanged native terminal contexts. Source is currently unfrozen.

Project close native57 5ranoM and full native57 88IOPg precede the root-binding
refactor. Re-run focused open, focused project close, and full native58 after the
current source settles. All remaining P5/P6 scope still applies.

Native58 opening UZnAc5 assertions PASS; cleanup required SIGTERM because the
original dirty test document correctly blocked application exit. The test now
saves that fixture file through MCP after recording retention proof; wrapper PASS
requires normal process exit. Rerun `/tmp/lomi-mcp-project-open-native-2.log`,
exec45240. No production close guard was bypassed. Core57+1ignored/UDS44/protocol9,
UI4, TypeScript and MCP all-target Clippy PASS; actual UI/native permission and
blank-editor screenshots were inspected. Keep source frozen until native finishes.

Project-open focused rerun DJziSo PASS, wrapper exit0 and native cleanup exit0.
Fixture save receipt succeeded/complete and saved exactly 33 bytes after dirty
retention assertions. Focused close Zdqpsp also PASS with normal cleanup. Full58
regression active at `/tmp/lomi-mcp-project-open-full-native.log`, exec32767; source
frozen. Read-only exploration of Settings validators started; no Settings adapter
implementation yet. All other remaining P5/P6 scope is unchanged.

Full native58 H2IkCx PASS with normal cleanup. Source unfrozen; final production
Clippy `/tmp/lomi-mcp-project-open-production-clippy-final.log` running.

### Current implementation — Settings opening (catalog59)

Closed nine-page DTO and separate `settings.open` permission; existing Settings
window/service, approved live workspace anchor and normal durable retry receipts.
The native one-use dispatch permit checks the operation, owner, UI revision,
global policy counter and connected session. It is checked around asynchronous
window preparation. Native completion records the typed result before success;
a forged UI ACK cannot substitute for that completion. No preferences are written.

UDS five-scenario test PASS after it caught an immediate-revoke gap in the first
new permit; the permit now observes the same atomic global authorization counter
as other native adapters. Cases cover scope/foreign anchor denial, queued and
committed cancellation, stale revision, immediate revoke, forged ACK, one-use
commit, exact retry and key conflict. Wire5/UI1/TypeScript/workspace check PASS;
rendered permission control visually inspected. Full core/protocol tests and
production Clippy running. Native `LOMI_MCP_SETTINGS_OPEN_ONLY=1` implemented,
not yet run: all nine pages, hidden window shown, exact retry, retained dirty
buffer, unchanged prefs/layout/PTYs and cleanup save through MCP.

Latest full native evidence remains catalog58 H2IkCx. Next: finish checks, run
focused native59 Settings opening and inspect screenshot. Settings read/update,
themes/plugins/Android management/Chat/lifecycle and all remaining P5/P6 are still
required. This increment does not complete the original request.

Native59 Settings-only KzW0fU PASS, all nine pages and normal exit/cleanup.
Screenshots inspected. Full core57+1ignored/UDS45/protocol9, wire5, UI1, TypeScript,
workspace check and production Clippy PASS. MCP all-target Clippy final check:
`/tmp/lomi-mcp-settings-open-core-clippy.log`. Source unfrozen. Next: Settings
read/update adapters (four nonsecret sections), then the remaining P5/P6 scope.
No preferences adapter code started yet; read-only design notes are in REQUIREMENTS.

### Current implementation — Settings read (catalog60)

New `broker/settings_read.rs` uses the bounded main reply bridge (2 shared read
producers, 16 pending cap, 2s deadline, global revoke/session/epoch/target checks).
`agent-settings.ts` reads existing provider hooks without native file reload or
writes, clones before hash/pagination, caps full hash input at 1 MiB and output at
48 KiB. Closed four-section DTOs omit paths, raw errors, CSS, credentials and chat.
Shortcut total max4096, pages max200, revision required after offset0. Explicit
ready/recovery_required preserves and identifies the last working provider state.

Core57+1ignored/UDS46/protocol9, wire5, UI2, TypeScript/workspace check/production
Clippy PASS (`/tmp/lomi-mcp-settings-read-*.log`). Native profile implemented and
running, `/tmp/lomi-mcp-settings-read-native.log`; source frozen. No Settings write
adapter implemented yet. Latest full-domain native regression is H2IkCx/catalog58.

First native60 ThP0hB failed to compile the test fixture because its new event
emission lacked the `tauri::Emitter` import. No app host ran. The exact private
fixture app data was verified from config/session and removed after exit101;
cleanup.json records this. Added the missing test-only import. Rerun active at
`/tmp/lomi-mcp-settings-read-native-2.log`, exec27521; source frozen.

Native60 qqYreQ PASS: all four actual Settings snapshots, stable shortcut pages,
real Settings editor change, stale-revision denial, corrupt fixture preservation
with recovery_required and no raw error disclosure, same dirty document, unchanged
native contexts and normal exit0/private app-data cleanup. Source unfrozen.
MCP all-target Clippy running at `/tmp/lomi-mcp-settings-read-core-clippy.log`.
Next required increment: Settings writes; no mutation adapter code started yet.
