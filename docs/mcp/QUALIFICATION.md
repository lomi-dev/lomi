# MCP qualification

Host: macOS 27.0 (26A428), Apple M3 ARM64. Baseline HEAD and local changes: IMPLEMENTATION-STATUS.md.

## Scoped file artifact import/export (2026-09-24)

**lVi0Ip PASS**,12 checks, catalog72, macOS ARM64. The real stdio helper,
Settings enrollment, broker, main file-operation guard and native writer imported
and exported binary Unicode/CRLF, empty content and an opaque file with an .apk
extension. Rewriting each source after staging did not alter the exported bytes.
Exact retries reused import/export receipts. Changed requests conflicted; existing
files and symlinks remained intact. Stale parent revisions, foreign workspaces,
secret paths and oversize imports were denied. Normal host/wrapper exit0 and
private app-data removal are confirmed. Evidence: `/tmp/lomi-mcp-artifact-native.log`,
`lomi-mcp-control-lVi0Ip/artifact-files.json`. The actual Settings permission
screenshot and separate mocked-WebKit screenshot were inspected.

Core60 PASS (one existing opt-in ignored), helper wire8, WebKit permission1,
TypeScript and production workspace check PASS. The new schema5 migration test
opens actual version4 image/APK metadata, preserves their bytes/classification
and adds empty/opaque files. Final broker65 additionally includes a full4MiB
import/export, APK-only grant refusal, native one-use/forged-ACK checks,
cancellation and exported Android PNG source revocation, including receipt reads.
All-target MCP and MCP-probe Clippy passed with warnings denied after removing
one needless borrow. Final broker/lint logs use `/tmp/lomi-mcp-artifact-*-final.log`.

This qualifies controlled project artifact import/export. It does not qualify
browser upload/download, Android installation of arbitrary generic files,
other platforms or the full P6 fault/performance matrix.

## Expanded native browser logs (2026-09-24)

**a6K0Te PASS**,11 checks, catalog71, macOS ARM64. Real helper/Settings/broker/
WKWebView evidence: `/tmp/lomi-mcp-browser-logs-native4.log` and
`lomi-mcp-control-a6K0Te/browser-expanded-logs.json`. Normal host/wrapper exit0,
private app-data removal and browser close are confirmed. Five console levels,
Unicode, primitive/opaque rejection reasons, retained error coverage, pagination,
kind-bound cursors, page tampering, overflow/gaps, malformed Unicode, foreign
workspace denial and navigation invalidation passed. The native-console baseline
confirms the collector adds no object coercion; the native engine can still
format objects itself. The public catalog remains71 tools.

Actual WebKit and WKWebView mark genuine unhandled Promise events isTrusted=false.
The collector therefore reports eventTrusted without treating it as authenticity,
and includes synthetic PromiseRejectionEvent reports. Plain reasons are captured;
objects are omitted. The page-world console/Promise collector has no native IPC,
file access, grant or evaluation tool. DOM and error dispatch remain isolated.
Coverage is main-frame-only and partial; replacement console methods can bypass
later capture. These messages never confirm authorized operation completion.

Collector3, real WebKit rejection1, existing frame WebKit4, permission WebKit1,
helper wire8, TypeScript and broker64 PASS. Final all-target MCP package and
MCP-probe Clippy PASS with warnings denied. Initial DgBgy6 failed a fixture that
mixed engine coercion with collector coercion. LWDj7U and r1BPLE failed the
incorrect trusted-only Promise assumption. Those runs are not counted as passes.
The corrected assertions are exercised by a6K0Te. Browser transfers and P6 remain
open; this result does not qualify those paths or another operating system.

## Same-origin native browser frames (2026-09-24)

**KjoDju PASS**,13 checks, catalog71, macOS ARM64. The actual stdio helper,
broker and private WKContentWorld observed main/child/nested documents, filled
Unicode, dispatched keys only to the current focused frame, clicked exactly once
across retry, and scrolled only the requested child viewport. Replacing/removing
a frame invalidated its old references. A parent overlay blocked input at the
actual projected target point. Opaque/srcdoc/hidden contents and private field
values remained omitted. Page-world constructor/global replacement did not change
the isolated dispatcher. Native browser close and isolated app-data removal passed;
host/wrapper exit0. Evidence: `/tmp/lomi-mcp-browser-frames-native3.log` and
`lomi-mcp-control-KjoDju/browser-frames.json`. The nested-frame screenshot from the
same rendering path in Uy20vL was inspected.

Four WebKit adapter tests additionally cover real cross-origin navigation,
frame/node/byte/depth budgets and transformed ancestor refusal. Core59 + broker64,
TypeScript, wire5, all-target MCP Clippy and the final scroll compatibility unit
passed. MCP-probe and final all-target MCP Clippy passed with warnings denied. Native Uy20vL failed at the initially
missing viewportRef DTO field; Cjx0vi failed the fixture's final close because its
request omitted browserGeneration. Their cleanup passed, but neither is a full
qualification pass. No advanced log/transfer or P6 completion is implied.

## Android layout native qualification (2026-09-24)

Native **ErnuOt PASS** (`/tmp/lomi-mcp-android-layout-native3.log`), catalog71,
14 checks and normal host/wrapper exit0 with isolated app-data cleanup. The real
private emulator preserved its generation across duplicate views, focus,
docking, pane moves, tab reordering and transfer of a mixed Android/PTY tab.
The retained zoom survived; the PTY kept its native session. Closing one shared
view did not stop Android. Last-view panel/workspace/project closure confirmed
native exit, preserved device metadata, and exact retries reused receipts.
A dirty editor Cancel preceded Stop and retained the running phone; Discard
allowed project close and Stop. The closed PTY was absent from terminal_contexts.
K0tPJi's mixed Android/PTY and dirty-close screenshots were visually inspected;
ErnuOt ran the same rendering/close paths after a fixture-only assertion repair.

Shared Chat closure regression **0ovWOt PASS**, catalog71 and54 checks,
`/tmp/lomi-mcp-android-layout-chat-regression.log`: normal host/wrapper exit0,
isolated app-data cleanup, actual local fixture generations and existing Chat
panel/workspace/project close behavior. No paid provider request. Native
project/browser/PTY close regression **wGLMu7 PASS**, catalog71, normal exit0 and
isolated app-data cleanup, covers dirty Cancel/Save/Discard, MCP cancellation,
real idle PTYs, hidden WKWebView cleanup, exact retry and a retained project.
Wire5 passed at `/tmp/lomi-mcp-android-layout-wire-all.log`; native Android76
passed (17 opt-in tests ignored). Production workspace, all-target MCP packages
and final MCP-probe Clippy passed with warnings denied. The additional application
all-target check found nine existing test-only lints in android/input.rs,
chat/preferences.rs, chat/store.rs, files/editor.rs, themes.rs and updater.rs.
All six files are unchanged from HEAD; these lints remain recorded for P6.
This increment is ready for its milestone commit/push.
No advanced browser/terminal or P6 qualification is claimed by this Android run.

Current broker64 PASS (`/tmp/lomi-mcp-android-layout-broker2.log`) includes shared
last-view detection, missing stop scope on whole-project closure, native barrier
lifetime, exact replay, workspace transfer with shared source views, preserved
generation, input revocation and human takeover. A slow native callback proves
that operation reads/cancel remain responsive and a forged early close ACK is
refused. Existing WebKit workspace/Chat17 PASS; model179 + AI17, TypeScript,
frontend build and MCP-probe Clippy PASS. Formatting/diff checks passed.

Native bMi7lr did not pass: the fixture supplied a scale factor to the existing
percentage-based zoom API. It reached native boot and shared-view focus, then
failed its zoom setup assertion. Host exit0 and isolated app-data cleanup were
confirmed. The fixture now supplies125%; production zoom code was unchanged.
K0tPJi reached all successful native close paths, then failed the final fixture
assertion because terminal_contexts returns a map, not an array. Normal exit0
and private app-data cleanup were confirmed; its Android/PTY and dirty-close
screenshots were inspected. The corrected fixture asserts absence of the exact
closed PTY generation. The corrected ErnuOt completed proof and cleanup were inspected as recorded
above.

## Android management selected module (2026-09-24)

Native **7dOf1P PASS** (`/tmp/lomi-mcp-android-setup-native2.log`): catalog71,
23 checks, normal host/wrapper exit0 and isolated app-data cleanup. The real
stdio helper enrolled through Settings and used the existing licensed private
SDK. Inventory/catalog/plan revision checks, main-caller refusal and exact
provider consent preceded actual Platform-Tools37.0.1 installation. Only terms
whose digests matched the previously accepted private-SDK record were approved.
No fresh Java bootstrap is claimed from this run.

The fixture created a real AVD, modified desired hardware, wiped its owned
sentinel, deleted the AVD and verified that the previous qualification device
was unchanged. Exact retries reused receipts. Wipe/delete required separately
typed Settings confirmation. Native tool rollback, removal and reinstall also
passed. Removing an image used by the original device was refused. Malformed
preferences were restored from a valid backup and reset only after typed
confirmation; immutable copies preserved the corrupt originals. The original
preference-file baseline was restored. Recovery/owned-cache cleanup passed.
The approval screenshot was inspected. Earlier OzbXGl passed19 before the
additional maintenance scenarios.

Core2 covers foreign plans/devices, empty initial setup grants, exact terms,
revision conflicts, creator-only grants, preflight-before-replay regressions,
decline, active cancellation and workspace removal. Native preflight/guard2
covers name/generation checks, metadata fingerprints, recovery digest changes
and a refusal while holding the actual manager mutation gate. Full broker61,
Android units75 (17 opt-in tests ignored), WebKit UI2, TypeScript and frontend
build, workspace/probe Clippy and wire5 passed. The later error-mapping-only
change passed native3: actual disk-space refusal maps to RESOURCE_EXHAUSTED,
busy mutation exclusion to TARGET_BUSY, and revocation retains its typed error.
All device mutations currently require a stopped target. Device metadata
requires a valid backup and cannot reset; preference metadata can reset.
These results do not qualify remaining Android mixed-layout or broader P6 tests.

## Chat AI selected module (2026-09-24)

Native **iyLq7U PASS** (`/tmp/lomi-mcp-chat-close-native2.log`): 68-tool catalog,
54 checks and five generations using only the local fixture provider. Both host
and wrapper exited normally and isolated app data was removed. The run covers
all seven Chat tools, permission/privacy checks, exact send approval, durable
request identity/replay, stop/checkpoints, paginated text exports, mixed-layout
focus/docking/movement/transfer, and panel/workspace/project closure. A failed
draft flush produced an outcome_unknown/unknown receipt and preserved the active
response; exact retry returned that receipt without cancelling it. Successful
closure at every level checkpointed the exact request and preserved the next
draft. Panel/workspace closure retained the unrelated PTY. The preceding qVY6Fd
run also passed54, and its closing screenshot was inspected.

Full broker59, native Chat34, model179 + AI runtime17, TypeScript, production
workspace check, WebKit Chat/draft/send UI14, workspace Clippy with warnings
denied, helper wire5 and production frontend build passed. Native cleanup.json
confirms hostExited=true, appDataRemoved=true and exitCode=0.

Native gVdzJU (47 checks) also passed before adding close qualification. Earlier
native Ed6xlT/bvy4Sf/SIBYbP runs reproduced render starvation: the elapsed-time
leading throttle rendered each token when unchanged history was expensive.
Commit `12af2935eed70f12563c98b03091b71df37119f8`, pushed to origin/main,
uses a trailing task for message publication and memoizes unchanged Markdown.
The parser retains every chunk; final status publishes immediately. Three model
regressions cover batching, final/error publication and unsubscribe/remount.
The long-response UI fixture now bounds its producer; it passed in WebKit and
Chromium. Failed diagnostic runs are not qualification passes.

The following sections retain the evidence and limitations of earlier increments.

## Chat AI send increment (2026-09-24; in progress)

- IMPLEMENTED_UNVERIFIED until native evidence below is recorded. Catalog66
  connects a native exact-context plan, one-use approval and commit, preallocated
  durable request/message IDs and the retained Chat SDK. Provider/model target
  reads require chat.send in addition to the selected conversation's chat.read.
- Native chat30 PASS (`/tmp/lomi-mcp-chat-send-context-tests.log`), including
  rejection before credential access and credential rotation invalidating a plan
  without consuming the draft or dispatching the provider.
- Full broker55 PASS (`/tmp/lomi-mcp-chat-send-broker-all.log`). Send regression
  covers denial, stale native preflight, exact approval hash, one-use commit,
  forged ACK rejection and durable retry. It found and fixed a premature Running
  transition at UI claim. Read regression now refuses unrequested send metadata
  and denies explicit metadata requests lacking chat.send before native access.
- UI3 PASS (`/tmp/lomi-mcp-chat-send-ui.log`): Cancel focus/Escape and exact
  preview, invalidation after human input, retained SDK and next human draft
  during delayed native ACK, native refusal restoring the draft without dispatch.
  The 900×600 approval screenshot was inspected; details and actions scroll into
  view and all attachment descriptors remain available.
- TypeScript and mcp-probe cargo check PASS. Native **3w7ndI PASS**, 31 checks,
  catalog66, one native generation through the local fixture provider; host and
  wrapper exit0, private app-data removed. BNLJwM was a fixture race: polling
  treated AwaitingUser as settled before the approval dialog completed. The
  fixture now waits for dialog dismissal first. Read/open/draft regressions,
  decline with unchanged draft, native context change rejecting the approved
  stale plan, retained SDK stream and next human draft, exact send replay and
  native cancellation checkpoint all passed. Approval screenshot inspected;
  stream screenshot was of the other active tab, so it is not visual stream
  evidence. Retained hidden stream assertions did pass. UI regression14 PASS.
  The lost-approval regression also checks the actual receipt DB before startup
  recovery; revocation settles an uncommitted send as cancelled/none. A later
  admission-failure repair preserves reserved IDs while recording rejection,
  with exact stale-domain replay tested. No paid provider qualification is
  claimed. Fixture bundle selection requires
  the debug-only mcp-probe feature and private send profile/environment.

## Chat AI stop increment (2026-09-24; in progress)

- Catalog67. Direct broker-to-native exact-request cancellation requires
  chat.read/chat.stop and the selected conversation/project. It waits for the
  native terminal checkpoint; no renderer ACK or visible panel is required.
  Read returns the last request's ID, assistant ID and status without provider
  metadata. Completed IDs remain idempotent and cannot target a replacement.
- Native stop units2 PASS: actual fixture generation, foreign/missing IDs,
  cancellation checkpoint, old stop versus replacement generation and storage
  failure retaining RAM with OutcomeUnknown. Core Chat6 PASS, including stop
  scope, native-output binding, request-key conflict and exact replay. TypeScript
  and production workspace check PASS. Native **4aNBY2 PASS**, 35 checks,
  catalog67, two local fixture generations, host/wrapper exit0, private app-data
  removed. Exact active identity, unknown stop refusal, terminal checkpoint and
  old stop replay preserving a new human generation passed. This run revealed
  the sending tab; its actual live stream and next draft screenshot was inspected.

## Chat AI export increment (2026-09-24; in progress)

- Catalog68. Separate chat.export/chat.read with selected conversation/project.
  Exports Markdown/JSON pages from all saved message variants and the persisted
  draft. This is text export, not a private-store backup or an attachment export.
  System instructions, provider metadata, reasoning/non-text parts and attachment
  bytes/names are omitted. No native destination files are written. Clients can
  concatenate pages and verify SHA-256 of the full UTF-8 document before saving.
- Up to 512 messages, 4 MiB raw source/document, 8192 UTF-16 units/48 KiB per
  response and two shared producers/5 s cooperative deadline. Noninitial pages
  require the full document revision; changed history and split scalars fail.
  Oversized input fails without claiming a truncated export is complete.
- Native export1 PASS (privacy, all variants/draft, exact page reconstruction,
  unchanged DB, changed revision and Unicode boundaries); the later 513-message
  refusal assertion subsequently passed in full native chat33. Core Chat7 and TypeScript PASS.
  Wire5 and permission UI1 PASS. Native **BxwjjV PASS**, 41 checks, catalog68,
  two local fixture generations and 23 JSON pages with the exact document hash.
  Five saved messages and a draft, privacy filtering, Markdown, read-only DB,
  exact grants and stale/split pages passed. Native/wrapper exit0; private data
  removed. Probe Clippy found Reply size growth: ChatStopped is now boxed with
  identical JSON; follow-up probe Clippy PASS. Full native chat33, broker57 and
  wire5 PASS on these bytes. Full Chat still needs mixed-layout integration.

## Chat AI open and draft increments (2026-09-24; in progress)

- Open native **ygmgFM PASS**, `/tmp/lomi-mcp-chat-open-native.log`:17 checks,
  catalog64, host/native/wrapper exit0 and private app-data removed. Existing
  view reuse, new native SQLite conversation, creator-only access, durable
  exact retry, retained SDK and human draft, private-ID denial and zero provider
  requests. `chat-open.png` was inspected. Core open1/model2/wire5, TypeScript
  and probe Clippy PASS. Mixed-layout reveal is still explicitly refused.
- Draft native store3 and broker draft1 PASS in
  `/tmp/lomi-mcp-chat-draft-native-unit.log` and
  `/tmp/lomi-mcp-chat-draft-broker.log`. Exact project/panel/conversation,
  scopes, revision conflict, preclaim/repeated-commit/forged-ACK denial,
  native result persistence, no-effect rejection and revocation before commit.
- UI4 PASS, `/tmp/lomi-mcp-chat-draft-ui.log`: opt-in permission/reset,
  ordinary Android approval regression, unsaved/stale draft refusal and human
  typing during a delayed native acknowledgement. The latter waits past the
  normal autosave debounce and verifies the next save uses the new CAS revision.
  These mock-backed UI checks do not qualify native dispatch.
- TypeScript PASS after narrowing mixed-panel selection to Chat descriptors.
  Full broker54 and wire5 PASS (`/tmp/lomi-mcp-chat-draft-broker-all.log`,
  `/tmp/lomi-mcp-chat-draft-wire.log`), catalog65 below the catalog budget.
- Native CswH2K/IlFJDY failed the late-ACK fixture condition: attempted assignment
  to Tauri invoke did not take effect. Diagnostics confirmed the write succeeded
  normally. The probe now wraps only the retained runtime's commit callback,
  delaying its real native result without changing production IPC.
- Native **dtcU69 PASS**, `/tmp/lomi-mcp-chat-draft-native-3.log`:23 checks,
  catalog65, host/native/wrapper exit0 and private app-data removed. Six draft
  assertions include persisted CAS, one-write retry, stale/private refusals,
  human text retained and saved across delayed native acknowledgement, and zero
  provider requests. Read/open regressions and native revocation pass in the
  same run. Settings grant screenshot inspected. Probe Clippy initially found enum growth;
  boxing ChatDraftUpdated leaves wire JSON unchanged. Probe Clippy PASS in
  `/tmp/lomi-mcp-chat-draft-clippy-2.log`; targeted broker1 and wire5 repeated
  PASS afterward. This precedes the in-progress send/context changes.
  Send/stop/export and remaining layout are not implemented yet.

## Chat AI read increment (2026-09-24; in progress)

- Native persisted-history unit2 PASS; exact project/message IDs, Unicode and
  revision paging, omitted system/credential/reasoning metadata.
- Real UDS chat2 PASS; exact conversation grants, empty/scope-denied sessions,
  bounded pages, malformed adapter results and revocation during reads.
- UI2 PASS (picker and existing Agent Control regression), screenshot inspected.
  An initial existing Android assertion needed the new empty chatConversations
  field. No product regression was inferred from that fixture mismatch.
- Production/probe cargo check, TypeScript and wire/process5 PASS. Catalog63
  remains below 700 KB. Initial wire fixtures lacked valid arguments for the two
  new tools; adding their real inputs restored both protocol-version tests.
- Native63tTnf and ymrFrr FAIL: Settings picker initially observed a busy store;
  a later direct native catalog read returned all three expected project entries.
  Settings now waits at most 500 ms and distinguishes busy from storage errors;
  the native fixture deliberately holds the service mutex for 200 ms.
- NativexHWbvi passed history assertions but crashed on synchronous UI revoke.
  The same no-reactor panic was reproduced by a real thread outside Tokio in
  `/tmp/lomi-mcp-native-thread-revoke-before.log`. Broker cleanup now captures
  its owning runtime handle. Full broker52 PASS in
  `/tmp/lomi-mcp-chat-broker-all.log`.
- Native **jMNDbu PASS**, `/tmp/lomi-mcp-chat-read-native-4.log`: all11 checks,
  catalog63, native/wrapper exit0 and app-data cleanup confirmed. Explicit
  Settings selection, project isolation, exact metadata pages, scalar boundaries,
  conversation/message denial, private-field filtering, read-only SQLite,
  revision changes and native synchronous revocation. No provider call.
- Probe Clippy PASS in `/tmp/lomi-mcp-chat-read-clippy.log`. A screenshot review
  found inline conversation labels; each now occupies a row with its ID below.
  The revised two-row screenshot was inspected and UI1 PASS. This CSS-only
  follow-up does not claim a second native qualification. Full Chat remains
  incomplete; five mutation/export tools and layout integration remain.

## Executed

- Read-only inventory: versions, Git, manifests, source contracts — completed 2026-09-23.
- Published crate metadata: rmcp 3.4.0, schemars 1.2.2 — retrieved; not yet a compatibility PASS.

## P0 checks executed on this host

- `pnpm check`: PASS (baseline).
- `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib terminal::tests`: PASS, 2 tests including real PTY UTF-8/resize/exit (baseline; not yet MCP prompt/lease qualification).
- Protocol/core/helper tests: PASS, 4 protocol + 9 core + 2 helper tests. One ignored crash-child entry is executed and killed by its parent recovery test. Includes bounded IPC framing, fragmented UTF-8 stdio, mTLS over real UDS, wrong broker/helper/instance, handshake replay, concurrent retry reservation, canonical argument/generation hashes, unavailable storage, cancellation races, private files and SIGKILL/WAL recovery. These core tests do not qualify broker scopes or production dispatch.
- `node --test tests/mcp/protocol.test.mjs`: PASS, 3 process tests. MCP 2025-11-25 initialize and 2026-07-28 server/discover are distinct lifecycles. Verified closed input/output schemas, standard PNG image, domain error, unknown-field rejection, private zero-TTL discovery and >8 MiB input rejection.
- `node tests/mcp/codex-probe.mjs`: PASS, pinned Codex CLI 0.155.1 app-server starts the actual Rust probe, lists two fixture tools and calls both. Image and typed error received; no model turn/provider call. Other configured MCP servers are disabled only through subprocess overrides (inventory retains disabled entries). No config writes. This does not qualify model routing or image interpretation.
- `node --experimental-strip-types tests/native/run-mcp-browser.mjs`: PASS for the recorded assertions on real `browser-mcp-fixture` WKWebView created by Workbench, plus a candidate isolated child profile. Controlled React form rejects empty input and saves `Zażółć 🙂`; native screenshot inspected visually. Select, contenteditable, canvas, focus, SPA and application-command rejection passed. `isTrusted` is false for synthetic input. A Promise passed to Tauri eval_with_callback is unsupported; WKWebView callAsyncJavaScript in a named isolated content world succeeds and cannot see page globals. Separate profiles isolate cookies/localStorage/cache/service workers. Blocked cross-origin redirect generated zero requests at its target.
- Native Android installer and new retained MCP fixture: PASS, with explicit user consent. Real manager/actor, APK install, Unicode input, UI hierarchy, native PNG and rejected stale lease; confirmed Stop/private ADB cleanup. Details and rerun command: ANDROID-FIXTURE-PLAN.md. Public MCP tools, artifact staging and UI canvas remain unqualified.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace --locked`: PASS, app 199 tests plus the 15 new library tests; opt-in native tests remain ignored in this command. Includes existing native Bash/Zsh/PTY checks, which do not qualify the new MCP prompt/lease semantics.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --locked -p lomi-control-core -p lomi-control-protocol -p lomi-mcp --all-targets -- -D warnings`: PASS.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --locked -- -D warnings`: PASS for production targets.
- Broader workspace Clippy with `--all-targets -D warnings`: FAIL, 8 findings in unchanged existing app tests (`chat/preferences.rs`, `chat/store.rs`, `files/editor.rs`, `themes.rs`, `updater.rs`). New crates pass that same lint mode. Log: `/tmp/lomi-mcp-workspace-clippy.log`. These findings are not reported as a successful release gate.
- New crate Rust formatting, changed fixture/document Prettier formatting, `git diff --check` and post-change `pnpm check`: PASS.
- Cargo audit 0.22.2, RustSec database commit `f7dc4b2860b29978f400fda0aab31cc4dbd21134` (2026-09-22): initial scan found RUSTSEC-2026-0285 in existing rustls 0.23.44. Updated only rustls to 0.23.45 and reran authentication/process tests. Rescan reports 0 vulnerability entries; 7 unmaintained warnings and one glib 0.18.5 unsoundness warning remain in the pre-existing dependency tree. This does not approve a Linux release or constitute the complete P6 dependency review. Reports: `/tmp/lomi-mcp-audit.json` and `/tmp/lomi-mcp-audit-fixed.json`.

Evidence directories (outside Git):

- Native browser: `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-browser-6vNpZB` (`result.json`, `browser.png`, native build log). Earlier failed probe runs are retained for diagnosis, not counted as PASS.
- Codex: `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-codex-wfaZgA/result.json` (latest rerun).
- Android: `/tmp/lomi-android-stage0-mcp-20260923/evidence/`.
- Transient command logs: `/tmp/lomi-mcp-{baseline-check,baseline-rust,wire,core,codex,browser}.log`.

Native product builds use Tauri 2.11.5, Wry 0.55.1, objc2-web-kit 0.3.2 from the existing lockfile. Probe image is 2380×1578 at DPR 2; a production adapter must bound/reduce it to the public image budget. The probe itself is not a product adapter.

## Pending

Full E01–E12 and S01–S16 acceptance remains pending. A production native metadata/connection scenario has passed; it does not complete the broader end-to-end and safety matrix. Existing Android/PTY/browser results qualify only their recorded probes.

P0 overall remains IN_PROGRESS: PTY prompt/lease semantics, full browser navigation/frames/input limits, immutable APK staging and the complete grant/queue matrix still need evidence. The missing Android fixture dependency is resolved by the new isolated SDK/AVD. MCP platform support remains unqualified until its complete scenarios pass. No package has been built, signed, installed or published for MCP.

## P1 executed increments

- Native Settings → production `lomi-mcp` → authenticated broker: PASS on macOS ARM64. Opt-in, generated public-pin configuration, explicit workspace selection, no pre-approval disclosure, main denied pairing authority, foreign ID rejection, input schema rejection, native revoke, clean stdio EOF and disable. Screenshot inspected. Evidence: `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-2mSDk8/`.
- Real Codex CLI 0.155.1 app-server → production helper → native Lomi: PASS for metadata, filtering, connect/retry epoch and foreign workspace denial. Native Settings approved the connection. No provider call or private config write. Evidence: `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-MvxTcW/` and its `codex.log`.
- Core broker integration: PASS, four real UDS/TLS tests including unapproved sessions, wrong pin, UI reload/revoke, cursor revision, read-only scope, durable rename, duplicate key, changed payload conflict, forged/duplicate claim, ACK before matching publication, cancelled queue, and lost running ACK recovery.
- Application-close UI regression: PASS, six Chromium tests, including freeze before Android preparation and resume after cancellation. Initial mock failures were fixed by representing the new native command in the test desktop; native authorization is tested separately.
- `pnpm check` and workspace production Clippy: PASS before the latest mutation increment. Rechecking after changes; all-target Clippy baseline findings above remain distinct.
- The first combined native rename/Codex regression timed out during the Codex phase. A subsequent run passed the entire flow including durable rename/retry: `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-jFiAQT/`. Replacing the fixed approval delay with an explicit assertion-ready marker; final regression is pending.

- Final native rename/receipt/Codex regression: PASS at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-KzUDK6/`. Codex created two pending helper connections (`codex-pairing.json`). The fixture now approves the exact pairingRequestId returned by the client's lomi_status, rather than the first same-name request. This explains the earlier intermittent pairing timeout. Native rename, deduplicated retry and receipt recovery by request key passed.
- Production helper wire checks: PASS for both MCP protocol versions, seven actual tools, closed schemas validated with Ajv, mutation annotations, no pre-pairing resource disclosure, unknown arguments and 64 KiB argument rejection. Together with the original image/error fixture: five process tests.
- Post-mutation workspace Rust tests: PASS, 199 app + 9 core + 4 broker integration + 4 protocol + 2 helper tests. One additional stale UI epoch regression was added afterward and is awaiting its run. Post-change pnpm check and production workspace Clippy PASS.

## P0 terminal increment

- Native Zsh 5.9 on macOS ARM64: PASS using the production shell builder and integration scripts in a private fixture configuration (`LOMI_ZDOTDIR`, no user profile edits). Actual PTY prompt, command start, Unicode, exit statuses 0/1/130, a silent running `sleep`, input replay prevention with an on-disk assertion, and manual lease takeover were observed. Command: `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib shell::mcp_qualification -- --ignored --nocapture`. Log: `/tmp/lomi-mcp-terminal-native.log`. The owned shell and child were reaped.
- Core observation/input tests: PASS. Bounded 1 MiB normalized ring, 32-observer ceiling, fragmented Unicode/OSC, oversized escape body, gaps, same-sequence ACK replay, changed payload, sequence gaps, and no lease restoration through OSC. Native nonblocking writes stop on a deadline or revocation even when the peer never drains its buffer. The actual PTY probe also uses the nonblocking reader/writer helper.
- These are prerequisites, not a passed public terminal tool. Product PTY fan-out, main renderer watermark, origin ancestry, UI manual-input race and MCP create/run/read/input/interrupt integration are still pending. `/bin/bash` is 3.2.57; its automatic run/completion behavior needs separate qualification and is not inferred from Zsh.

## P2 terminal increments

- Native create: PASS at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-PkwPVq/`. One real retained terminal, native start ticket, deduplicated create and hydrated RAM lease. Screenshot taken before prompt output was ready; no prompt-render claim is based on that image.
- Native create/run/read: PASS at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-VcdU79/`. Real shell command in the visible xterm, Unicode `Zażółć 🙂`, OSC start/completion and exit 0, on-disk exactly-once assertion after retry. The terminal screenshot was inspected and shows the command and output. Production helper wire suite passed both protocol versions with ten tools and five process tests.
- Native input: PASS at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-69bID8/`. Same input sequence returns its previous ACK, changed payload conflicts, exactly one file append, a silent `sleep` remains running, explicit Ctrl+C and native human input revokes further agent writes. A stricter terminal exit-130 assertion and targeted interrupt/cancel tests were added afterward and are being rerun.
- Core regression: PASS, 12 unit tests and 6 real UDS broker tests (`/tmp/lomi-mcp-p2-all-core.log`). The new native-spawn-ticket test covers missing execute scope, relative path escape, claim-before-start, profile revision mismatch, single-use ticket, UI lease forgery, RAM-only lease hydration and revocation. The process crash fixture still executes from its parent test.
- Production workspace Clippy: PASS after the input/interrupt implementation (`/tmp/lomi-mcp-p2-clippy.log`). Full P2 completion is not claimed: workspace/panel controls, browser/server flow, events, parser watermark and remaining safety scenarios are pending.

- Targeted interruption and cancellation: PASS at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-0iKqJA/`. Explicit key Ctrl+C requires observed exit 130; `lomi_terminal_interrupt` dispatches once to its original command, and replay after a new `sleep` starts does not interrupt that new command. `lomi_operation_cancel` sends its one native interrupt and the original operation settles cancelled with observed exit 130. Human takeover still rejects subsequent input. The actual Codex metadata connection also passed in this run.
- Native PTY origin ancestry regression: PASS, two app terminal tests, including real child PID protection, external application ancestry and fail-closed unknown PID (`/tmp/lomi-mcp-p2-origin-native.log`). This does not by itself cover every wrapper/SSH or indirect close/restart path.
- Updated production helper wire suite: PASS with twelve actual tools, both protocol lifecycles, closed schemas and mutation annotations. Parser stream/ACK watermarks are now implemented and are under native regression.

## P2 workspace, panel and revocation increments

- Native workspace creation, scoped panel discovery, retained hidden PTY and real xterm ACK watermarks: PASS. The event/panel fixture completed at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-fB0yVG/` with seventeen tools. Focus and clean scratch/idle terminal close settle successfully; retry after removal returns the original receipt; a human-owned terminal cannot be closed. Exactly one original human PTY remains after the disposable terminal closes. No replacement shell is started.
- A native fixture initially exposed a spurious revision conflict caused by publishing an unchanged Workbench snapshot twice. Publication now compares the domain/runtime signature and advances the revision only for a changed snapshot. The detecting native scenario passed after the fix.
- Operation event pages are project/workspace/session-scoped, bounded and cursor-based; a foreign workspace is denied. They currently describe durable operation transitions, not all UI events.
- Native stop during a confirmed busy-loop main renderer: PASS. A native event marker proves the loop began; stop and subsequent denied MCP call must complete before the renderer resumes. The atomic-token version also passed at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-8You9S/`.
- Core regression: PASS, 15 unit tests plus 7 UDS broker tests in `/tmp/lomi-mcp-revoke-core.log`. New deterministic tests hold policy, receipt-storage and observation locks during stop, and abort a real authenticated TLS/UDS call while SQLite is blocked. Native authority is revoked before those locks are released, and the queued mutation is never dispatched. The crash child is intentionally invoked by its parent recovery test.
- Helper wire regression: PASS, five process tests covering seventeen tools, both protocol lifecycles, closed schemas, structured errors and annotations (`/tmp/lomi-mcp-p2-wire.log`). TypeScript/SDK/runtime checks passed (`/tmp/lomi-mcp-p2-ts.log`).
- Remaining P2 work includes approved existing-terminal ownership, broader close/focus variants and native browser/server integration. P0–P2 are not marked complete by these increments.

- Async main/Settings adapters and connection-lifetime tokens: full native regression PASS at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-UiCKVb/`.
- Terminal read modes: native PASS at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-Y5IfMI/`. `screen` reads the retained xterm alternate buffer with Unicode and actual parser sequence; `terminal-alternate.png` was visually inspected and matches `terminal-screen.json`. `command` excludes later commands, `raw` returns the original ANSI bytes as base64, and cancellation/restoration returns the terminal to its normal buffer. Scope/generation/epoch/watermark and command-range negative regression is being completed separately.

## P2 existing-terminal approval and current rendering qualification

- The eighteen-tool native fixture passed claim denial, one-use Settings approval, reattachment sequence preservation, explicit release, ordinary Workbench terminal attachment without PID replacement and the Take control button. Actual retained xterm text includes output produced before attachment. Evidence: `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-XXNQmJ/`.
- Visibility qualification is still open: this run reports host opacity 1 and parsed output, but the native WK snapshot contains a blank WebGL canvas. Both the ordinary and alternate-screen images are blank in current runs. A run under `caffeinate -u` did not resolve it (`lomi-mcp-control-7aGRCK`). These images are not counted as visual PASS. The earlier Y5IfMI alternate-screen image was visibly rendered. The subsequent foreground/window/RAF diagnosis below identifies an unavailable native drawing surface.
- The experimental reveal change was reverted after native diagnostics showed `document.visibilityState = hidden`, no document focus and no animation frame within 1.5 seconds even after hiding Settings, showing/unminimizing the main window and focusing the view. Evidence: `lomi-mcp-control-GKqwmn`. Six Chromium UI regressions for WebGL/DOM font readiness, fallback, startup glyphs, hidden GPU teardown and split remount passed (`/tmp/lomi-mcp-reveal-ui.log`). They do not qualify native WebGL painting.
- Canonical browser origin policy tests passed (six protocol tests total). The native browser now shares this validator. Explicit browser origin approval and isolated profile identity allocation are implemented; browser tools are not yet exposed.

- Current core regression: PASS, 17 unit tests, 9 real UDS broker tests and 6 protocol tests (`/tmp/lomi-mcp-browser-grant-core.log`). The revoked-session reconnection test now awaits deferred cleanup before reconnecting; authority is still revoked synchronously. Browser grant tests reject missing/path/wildcard/internal origins without consuming the pending request, then prove canonical approved origins through a real authenticated connection.

## P2 native browser open increment

- PASS at `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-81NHbS/`: production nineteen-tool helper and real Workbench child WKWebView; explicitly approved local origin and separate profile; native creation readiness and RAM-only lease; retry returns the same operation; unapproved origin is rejected; an allowed local URL redirecting to another local port is stopped with zero requests reaching that server. Native `browser-open.png` inspected and shows the fixture text/form with Polish Unicode and emoji. This qualifies the visible-open increment, not P2/P3 completion.
- Core/protocol regression PASS: 18 core unit tests, 10 authenticated broker tests, 6 protocol tests (`/tmp/lomi-mcp-browser-open-tests.log`); one intentionally ignored crash child is executed by its parent. New ticket test rejects pre-claim use, profile substitution and duplicate starts; result metadata is set natively and SQLite stores a null lease. Atomic browser tests cover revoke, disconnect, exact-origin boundaries and irreversible human takeover.
- TypeScript/SDK/runtime checks and 23 model tests PASS (`/tmp/lomi-mcp-browser-ts.log`, `/tmp/lomi-mcp-browser-model.log`). Saved isolated-browser descriptors survive restoration; malformed descriptors throw instead of becoming shared-profile browsers.
- Browser hidden creation, navigation tool, DOM actions/refs, capture/artifacts, manual takeover display, permissions, full profile cleanup and broader failure/lifecycle qualification remain pending.

## 2026-09-23 — native browser navigation and DOM increment

- Native 20-tool run: `lomi-mcp-control-bqligS`, PASS. Native commit/load observed, exactly one HTTP navigation on retry, denied redirect reached zero foreign requests, AppKit pointer and toolbar takeover revoke the lease, native monitors removed.
- Native 21-tool run: `lomi-mcp-control-QPch5v`, PASS. Production isolated-world DOM callback returns bounded semantic text and references through stdio.
- First click trial failed with STALE_SNAPSHOT before input because WebKit deallocated the named content world. A diagnostic showed no retained isolated state. The native adapter now holds a strong WKContentWorld reference, as required by its API lifetime contract; page-world globals remain separate.
- Native 23-tool run: `lomi-mcp-control-2xBPrJ`, recorded stage PASS (the fixture cleanup shell additionally printed exit 143; this does not replace the recorded assertions). React required-field error, Unicode fill, save, synthetic `isTrusted=false`, repeat click deduplication and expired snapshot were asserted. `browser-form-result.json` contains the actual DOM observation. Select/contenteditable and replaced-node rejection added in the next run, pending here.
- Core/protocol regression: 20 core unit tests plus 10 UDS integration and 6 protocol tests PASS; one helper crash-child is intentionally ignored standalone and launched by its parent test. Production workspace Clippy and pnpm check PASS for the DOM/actions increment. Further negative/storage/lifecycle/browser capture qualification remains required.

## 2026-09-23 — 26-tool browser DOM/permission increment

Native report `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-vehgxG/result.json`: PASS. Command log `/tmp/lomi-mcp-dom-final-native.log`. The fixture drives production stdio, Settings approval, retained native PTYs and the actual child WKWebView, plus real Codex discovery. Added assertions: select/contenteditable fill, scoped synthetic key and wrong-focus denial, scroll retry deduplication, asynchronous-text wait, unmatched timeout, SPA reference invalidation, and no delayed fill after a four-second WebContent JS hang. `browser-open.png` inspected: actual React fixture, form, select, editable text and masked password are visible.

The native WKUIDelegate is replaced for automation profiles before their first HTTP request. File chooser, media and motion handlers deny; absent popup/dialog handlers use WebKit cancellation/default behavior. Wry's original media handler was found to grant unconditionally and is no longer inherited. `browser-permissions.json`: delegate attached, media API unavailable, zero media-denial callbacks. This proves installation and absence of the API on this fixture, not a successful real media request/denial qualification. Geolocation/clipboard and the remaining permission/frame matrix remain open.

Failures repaired during this increment: unretained named WKContentWorld lost element references; waiter attempted one extra native read after its deadline; whole-layout revisions caused spurious native browser action conflicts after SPA URL publication. Runtime browser actions now refresh the domain, recheck the exact target/generation, and keep native authorization independent of unrelated layout revisions. Native URL refresh invalidates SPA references. Permission/cancellation deadlines are checked before AppKit dispatch and script entry; accepted effects cannot be undone by cancellation. One native callback per browser remains reserved until completion, including after a caller timeout.

Final regressions: `/tmp/lomi-mcp-dom-final-{wire,tests,clippy,core-clippy,ts,ui}.log`: wire 5/5; core 20 unit (one separate crash helper intentionally ignored) + 10 UDS integration; protocol 6; browser UI 4/4; TypeScript/SDK/AI runtime checks and both Clippy gates PASS. This is a P3 increment, not completion of P3 or full v1.

## Native capture and immutable artifacts (28 tools)

PASS: `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-ggnnxP/`, log `/tmp/lomi-mcp-capture-final-native.log`. Host/app/helper versions unchanged from the preceding increment. The real child WKWebView produces standard MCP PNG image content with CSS viewport, DPR, pixel dimensions, capture scales, page zoom, scroll and capture timestamp. Fixture asserts the 1280 pixel default bound, byte limit, exact geometry and byte-for-byte reread. Foreign workspace reads and reads after native pointer takeover are denied. The first capture image in `lomi-mcp-control-S7ujHz/mcp-browser-capture.png` was visually inspected: the saved Unicode React form, select/contenteditable and masked password are visible.

The first run's assertions passed but app-data cleanup failed with ENOTEMPTY while the fixture was still shutting down. That exact leftover isolated root was removed only after its process was confirmed gone. The runner now waits for natural shutdown, escalates only its owned process group when necessary, and removes its app-data root only after child exit. Final evidence `cleanup.json` records `hostExited: true`, `appDataRemoved: true`, `exitCode: 0`, no signal. This verifies fixture app-data cleanup, not yet all OS-managed WebKit profile/cache retention.

Store schema 3 adds bounded artifact reservations: 3 MiB each, 16 MiB per pairing, 64 MiB total, 64 records, two concurrent producers, 24-hour retention. Creation uses private files and fsync before publication; source classification is immutable, reads validate SHA-256 and file identity. New tests cover quota/ownership, partial recovery and expiry, and redirected-directory cleanup refusal. Callback producer permits remain held after caller timeout. Capture on hidden/obscured/changed native views fails closed; full geometry/fault/cross-frame matrix remains open.

The automation host guard requires macOS ARM64 and the macOS 14 isolated-store API. Native setup now checks the actual WKWebsiteDataStore UUID as well as the deny delegate before initial navigation; no default-store fallback is permitted. Only the recorded macOS 27 ARM64 host is qualified so far.

## Bounded error logs, retained browser focus and visibility (29 tools)

PASS native evidence: `lomi-mcp-control-dmOW6D` for logs; `lomi-mcp-control-TWNWAc` additionally covers retained browser focus, stale generation, retry, hidden screenshot rejection and malformed-Unicode normalization. Both cleanup records confirm native exit 0 and removed isolated app data.

The fixed WKUserScript is installed at document start, main frame only, in the strongly retained isolated content world. Genuine ErrorEvents cross into that world; Promise rejection events from the page do not (reproduced in the first test, then recorded as excluded coverage). Console, network, stacks, URLs and child frames are not captured. No page-to-broker reply IPC or public evaluation endpoint is added. The 64-entry ring truncates each message before retaining it, normalizes malformed Unicode, and reports capture start, generation, origin, current navigation, cursor, overflow and partial coverage. Eighty large errors proved bounded overflow and pagination. Page globals and synthetic ErrorEvents cannot replace the collector; all actual error messages remain untrusted page content. Foreign workspace and human-taken browser reads fail; separate core tests deny read without browser.read.

`CgwHUH` failed the new hidden-image negative test: logical tab selection changed while native geometry/hide synchronization was waiting for a suspended main-view RAF. The broker now independently publishes an atomic selection gate for each owned browser, and capture/input checks it before native dispatch (capture also at callback and disclosure). Event-driven browser geometry updates use a microtask while the host document is hidden. `TWNWAc` passes hidden image rejection and returns to the same form without navigation or lost input. This does not qualify full occlusion, platform/DPI and modal-race matrices.

## Native navigation cancellation and durable completion

PASS `lomi-mcp-control-HneLGS`, `/tmp/lomi-mcp-nav-native-completion.log`. A real loopback HTTP fixture writes a partial response and deliberately leaves it open. Cancellation closed the response after 45 ms; the native 15-second deadline closed the second response at 15,054 ms. Both were observed by the HTTP server before fixture shutdown. Receipts report outcome_unknown/unknown because requests already reached the server; retry returns the original operation and creates no second request. Subsequent native navigations succeeded, and old log cursors expired. Full preceding 29-tool assertions also passed; native exit 0 and app-data removal are recorded.

The first run (`vAVa08`) did not settle a timeout operation within the fixture polling bound. Native navigation receipts now complete directly through the broker's existing validated transition path; they do not depend on a main-renderer ACK. The exact original callback delay is not claimed as diagnosed. AppKit dispatch checks the current native permit and deadline immediately before loadRequest. Cancellation/deadline use native stopLoading rather than page JavaScript; a queued stop checks its pending operation and yields to human takeover or a replacement navigation. This does not undo effects already accepted by the server.

Browser UI regression suite: 4/4 PASS (`/tmp/lomi-mcp-final-browser-ui.log`) after event-driven hidden-document geometry synchronization. TypeScript passed. Final Rust/core/wire results are recorded in IMPLEMENTATION-STATUS.md after completion.

## PTY-owned web server and native browser flow

PASS: `lomi-mcp-control-I19XEn`, `/tmp/lomi-mcp-pty-server-native.log`. The native fixture creates an additional Lomi terminal in its approved workspace, waits for the native qualified prompt, and starts `tests/mcp/browser-server.mjs` through lomi_terminal_run. lomi_terminal_read in command mode returns the server's reported URL and matching operation ID. The command remains running after readiness; repeated run returns the same operation. Browser open uses that exact URL, with previously explicit test-origin approval, and all real WK form, Unicode, logs, screenshot, hidden/focus, navigation, cancellation and scope tests run against this server.

`dev-server-provenance.json` records the workspace, panel, terminal generation, lease, operation, observed URL, source/confidence and bounded command output. The URL is reported by the owned command and then confirmed through native browser responses; no local-listener scan is treated as ownership proof. `dev-server-stopped.json` records targeted interrupt followed by actual shell-integration completion/exit 0. Node's recorded PID was absent after the fixture. `cleanup.json` confirms native app exit 0 and app-data removal. HTTP counters are navigation=1, unfinished-response starts=2 and closes=2, forbidden-server requests=0.

This verifies the MCP client → real PTY → native browser path. Full E01 remains IN_PROGRESS: model-driven Codex routing and origin-terminal protection in the complete scenario still need their separate qualification. The React module is served by the isolated fixture Vite server; this does not claim arbitrary framework launch compatibility.

### Owned browser close and stale state (2026-09-23)

Native `lomi-mcp-control-MzdU9b`: PASS, 29 real tools. Exact-generation close, native view/input-monitor removal, deduplicated receipt, refusal after human takeover, peer preservation and artifact denial after close passed (`browser-close.json`). PTY-owned Node server stopped through targeted interrupt; cleanup confirms host exit and isolated app-data removal. Contenteditable values are omitted before and after fill; explicit labels remain. The initial failing runs `6nNwhO` and `XCjJyH` showed native redirect denial but no UI message. Native page revisions and preserved automation-denial state prevent older responses/load-start events from clearing that message. UI 5/5 passed with an explicit reversed-update test above 2^53; screenshot `test-results/browser-navigation-denied.png` inspected. `13hTJA` failed earlier at optimistic terminal focus; a bounded fixture retry only follows proven no-effect revision conflicts and refreshes its observation. No permission check was relaxed. This does not complete the wider P3/P5 lifecycle matrix.

Workspace selection native proof: `oNZ5Gg`, 29 tools, PASS with completed cleanup. Two switches preserve the PTY generation and running dev-server operation. Same-key retries return their original receipt. Broker tests additionally reject missing panel.focus, foreign workspace, stale revision, changed target at claim, lazy terminal startup and success ACK without a focused target. Selection schema accepts the dedicated action and rejects ambiguous rename/select input. Core 23, UDS 11, protocol 6, independent wire 5/5, TypeScript and production workspace Clippy passed.

### Hidden browser and disconnect cleanup (2026-09-23)

`lomi-mcp-control-wfHZKO` PASS, 29 tools, native exit and app-data removal confirmed. Hidden WK child created without selecting it; DOM heading observed through the real helper; capture denied until explicit exact-generation focus; native view then renders and closes without affecting the human view. `browser-hidden.json` records the observations. Prior failed hidden runs were fixture errors (incorrect DOM/DTO fields/discriminator); their cleanup required SIGTERM and completed. `Load` alone does not assert asynchronously rendered React content, so the test uses an element condition.

Core 24 + UDS 11 + protocol 6 passed, browser UI 5/5, TypeScript and production Clippy passed. A blocked-storage regression was sampled at `serve`'s synchronous cleanup waiting on state; cleanup and registration now run off the async executor. The EOF test is strengthened to one Tokio worker and a five-second fail-safe for its artificial blocker; it passed in 0.07 seconds without waiting for the blocker. Owner-lock explicit teardown passed the retained-file-descriptor test and the full artifact recovery suite; the earlier intermittent reopen error is recorded without claiming its precise cause was established.

### Selected Android metadata (2026-09-23)

`lomi-mcp-control-1A9KWF`: PASS, 30 tools, real native Settings device approval, public lomi_android_list over authenticated stdio, one real stopped `MCP qualification` device, foreign workspace denied. Full preceding PTY/browser/hidden/close/Codex assertions passed. Native exited and isolated app data was removed; the consent-checked SDK/AVD remains stopped for reuse. No emulator was started for this metadata test. This qualifies metadata only, not public Android input/start/APK/tools that do not yet exist. Post-run dispatch hardening adds atomic policy/connection/deadline checks; core 24 + UDS 12 + protocol 6 and production Clippy passed. Subsequent native Android runs must exercise that guard.

### Android manual panel and runtime authority (2026-09-23)

`lomi-mcp-control-cplPJo`: PASS, 31 tools, real MCP Android panel creation, selected-device projection, deduplicated receipt, no implicit emulator start. Native exited and isolated app data was removed. This run includes metadata's atomic dispatch guard. A stopped-view screenshot from Playwright was inspected; persistence/manual-Start behavior passed its dedicated UI and model tests. The first attempt supplied an invalid fixture DTO; the second exposed a missing runtime notification that left the native view showing startup. The corrected native run passed.

Start/stop (33 tools): IMPLEMENTED_UNVERIFIED at the native boundary. Rust core 24 + UDS 14 + protocol 6 PASS. Manager 5/5 and actor 1/1 PASS, including revoke while waiting for the boot gate, revoke after actor enqueue, and stale-generation Stop rejection before process effects. Native completion cannot be forged by a renderer ACK. Android Settings permission and manual-start restore UI 2/2 PASS; screenshot inspected. The complete native start/stop fixture is running; do not infer a PASS from the unit/UI results.

`lomi-mcp-control-H8Q7b3`: PASS, 33 tools. Public start reached the real Android manager's Running state only after guest/RPC/ADB/IME preparation, returned its actual generation, and deduplicated the in-flight retry. Stale-generation Stop was rejected; the correct Stop confirmed native process exit and deduplicated its completed receipt. `android-runtime.json` records results; `cleanup.json` confirms host exit 0 and app-data removal. Fixture runtime records are empty after cleanup. This qualifies lifecycle, not agent input/canvas capture/hierarchy/APK staging. The full PTY/browser/Codex fixture also passed. StdIO/schema 5/5, production workspace Clippy and new-crate all-target Clippy passed. No Windows/Linux qualification is inferred.

## Android input and hierarchy increment (2026-09-23)

- Implemented input leases/sequence receipts; core and UDS tests pass. Native input is **unqualified**: `gprft3` shows native focus false, and host IORegistry confirms screen lock. No paid calls or user emulator changes. Unlock and rerun the default native control fixture for Unicode/key/touch/rotation assertions.
- `CCKqQn`: native app, production helper/stdio and real isolated Android UI hierarchy PASS; 35-tool catalog, all preceding browser/PTY/Codex assertions executed. The explicit test-only skip records `androidInputQualification=NOT_RUN_SCREEN_LOCKED`; this is not full P4 acceptance.
- Native snapshot: actual fixture label, omitted editable-field value, node truncation, stale generation and foreign workspace refusal. `/tmp/lomi-mcp-snapshot-native.log`; report `android-snapshot.json` under the fixture evidence directory.
- Core 26, UDS 16, protocol 6, parser 3, stdio 5, permission/input/visibility UI 9 passed. Parser rejects DTD/entities/malformed/deep/oversized data. UI checks default-false observe/interact consent and reset on parent permission removal. Screenshot reviewed.
- Cleanup before and after the native run: exact isolated device stopped, private ADB stopped, no force required; native host exited and isolated application data removed. Original shared ADB/user AVD untouched.

## Android PNG artifacts increment (2026-09-23)

`YtazWR`, `/tmp/lomi-mcp-android-capture-native.log`: PASS for 36-tool executed native assertions, including actual 270×480 / 9,173-byte PNG from a 720×1280 phone, correct transform/scale, exact-byte artifact reread, scope/generation denial and unreadability after Stop. PNG visually inspected. Browser artifacts still pass with the generalized source/geometry union, preserving old JSON. Native input remains NOT_RUN_SCREEN_LOCKED. Device/private ADB cleanup and exited native host/app-data removal confirmed. Core 26, UDS 16, protocol 8, stdio 5, TS, permission UI and both Clippy suites passed. Logs use `/tmp/lomi-mcp-android-capture-*`. Four rotation transforms are unit-tested; actual rotated-image/touch qualification remains outstanding.

APK import qualification (`lomi-mcp-control-lZY77Q`, macOS ARM64): **VERIFIED** for completed private-copy import, native APK container checks, same-key deduplication, expected-hash refusal, source-change stability and workspace-bound metadata. Whole native fixture passed with 37 tools; input explicitly NOT_RUN_SCREEN_LOCKED. Core filesystem/storage/UDS tests cover symlink/hardlink/FIFO/secret-path denial, source identity changes, read-only copy descriptor, deterministic crash cleanup across hard-link publication, reservation quotas, lease-preserving expiry, native-only completion, cancellation and foreign ownership. See `apk-import.json`, `android-cleanup-after.json`, `cleanup.json`; evidence prefix `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/`. APK install/launch and E04 build-to-install remain **TODO**, not inferred from import success. ZIP64/multidisk APKs are outside current bounded parser; Android signatures are not verified by import.

APK install (`lomi-mcp-control-Dw6Bc8`): **VERIFIED** native Settings refusal/one-use approval, main approval denial, exact hash/device/generation binding, source rewrite before dispatch, actual package installer success and observed version codes, no duplicate install on retry. All 38-tool fixture assertions pass except input explicitly NOT_RUN_SCREEN_LOCKED. Native manifest parser tested on the shipped compiled APK, truncation/chunk corruption and cancellation; smart-socket fixture proves revocation interrupts stalled ACK and partial transfer without ADB replacement. Invalid signatures/no-space/native cancelled installation still need their broader fault matrix; no uninstall/reinstall fallback exists. Mock APK installer approval screenshot inspected; native screenshot clipped details due to fixture scrolling and needs a new capture. Native cleanup PASS (no forced stop).

### 2026-09-23 — Native APK build, launch and app logs (40 tools)

`lomi-mcp-control-1pGhEe` PASS on the recorded macOS ARM64 host. Real stdio calls compiled the fixture in a Lomi-owned PTY (native shell completion exit 0), read its result marker through `lomi_terminal_read`, imported the matching private APK, approved installation in Settings, launched `org.lomi.inputtest/.InputTest` via `lomi_android_launch`, observed the actual UI hierarchy and PNG, and read `MCP fixture started: Zażółć 🙂` through the new logcat tool (PID 2051). App UID/PID filtering, one-line cursor pagination, changed-filter denial, foreign-package denial and durable launch/build retries passed. `apk-build.json`, `apk-install.json`, `android-apps.json` and `android-snapshot.json` contain the evidence. APK 12695 bytes, SHA-256 `c206df8eaa3762d6e446039b2e24fb7fdc0489fdce20e9fe3661dbff0a8d8445`. The complete native approval card `apk-install-approval.png` was visually inspected; exact path/hash/device/generation and buttons were visible.

Full E04 remains IN_PROGRESS: input/form assertions require an unlocked native window and were explicitly NOT_RUN_SCREEN_LOCKED. No native focus bypass was added. `cleanup.json` confirms host exit 0/app-data removal; `android-cleanup-after.json` confirms stopped guest, no forced stop and private ADB stopped. Shared ADB PID 83444 was preserved.

Initial `KZ6wwS` failed before pairing because the 40-tool catalog duplicated the entire reply union in every output schema (1,075,481 bytes). Helper output schemas now select each tool's generated Data variants and reachable definitions; catalog measured 279,890 bytes. The schema remains closed and no response limit was raised. Cached catalog, 5/5 wire tests and helper all-target Clippy PASS. Supporting checks: 19 UDS tests, 33 core + 8 protocol tests, one bounded native log/UID test, Settings UI/TypeScript and production Clippy PASS. Follow-up identifier grammar assertions are pending a run.

Native `lomi-mcp-control-U0qTez` PASS: built a second APK with a fresh test-only signer in an MCP-owned PTY, imported its exact hash, requested and approved installation on the same owned generation, and observed Android's real INSTALL_FAILED_UPDATE_INCOMPATIBLE. The durable operation reported failed / effect none / installed=false / previousVersion=1; retry returned the same operation. The existing application then launched through MCP, logged the fixture marker and produced the UI snapshot/PNG. See `apk-signature-rejection.json` and `android-apps.json`; host exit 0, app-data cleanup and stopped guest/private ADB were confirmed. This does not qualify real insufficient-storage behavior, but the bounded INSTALL_FAILED_INSUFFICIENT_STORAGE classifier test passed. Protocol identifier injection cases now pass 9/9. Input remains NOT_RUN_SCREEN_LOCKED.

Native `lomi-mcp-control-xf2Ymh` PASS (41 tools): production `lomi_files_read` returned source=disk, encoding=utf8, lineEndings=cr_lf, exact Unicode/CRLF continuation and stable disk SHA-256. Wrong revision, secret `.env.fixture`, and foreign workspace were denied. Settings disk-read scope was explicit; dependent import/install permissions passed the revised UI test. All previous native checks passed, including actual incompatible-signature rejection and subsequent launch. Input remains NOT_RUN_SCREEN_LOCKED; host/guest/private ADB cleanup confirmed. Evidence: `files-read.json`, `result.json`, `cleanup.json`, `android-cleanup-after.json`.

Directory-list native `9fdcD0` PASS (42 tools): real stdio/main/Settings fixture recorded first/next/all pages and CURSOR_EXPIRED after a host filesystem addition in `files-list.json`. Hidden secret metadata was absent; cleanup exited 0 with no guest/private ADB left running. Core 35, UDS 21, protocol 9, Clippy and stdio 5/5 PASS. Native input still NOT_RUN_SCREEN_LOCKED.

Search native jSmQNw PASS (43 tools), files-search.json: exact UTF-16 matches across CRLF/CR, frozen cursor pages, changed query refused and no secret/foreign-workspace disclosure. Core35/UDS22/protocol9, native search5/5, Clippy, wire5 PASS. Host/fixture cleanup PASS; input NOT_RUN_SCREEN_LOCKED. Editor read follows with core35/UDS23/protocol9, TypeScript and UI2/2 PASS, native pending.

Editor-read native wyT6z7 PASS (44 tools): loaded shared CodeMirror buffer, induced user edit, stale revision rejected, unchanged disk read, exact undo and foreign workspace refused. ONIBpC initially exposed canonical-vs-UI path spelling mismatch, fixed by project ID routing with canonical source-path proof retained. Both runs cleaned host/app data; successful run also stopped guest/private ADB without force. Native input NOT_RUN_SCREEN_LOCKED.

Editor edits native9LKHyq PASS45: actual MCP batch insertion into shared CodeMirror, exact retry/dedup, whole-batch rejection for surrogate split, stable buffer after failure, one undo to original clean text. Native PNG inspected. Whole existing native suite passed except explicitly NOT_RUN_SCREEN_LOCKED input. Cleanup host0/app data removed/guest/private ADB stopped without force. Wire5, production and core/all-target Clippy, core35/UDS24/protocol9, pure editor6 and UI checks PASS.

2026-09-23 continuation: user confirmed the screen is unlocked; native Android input runs now use the default required mode. Editor-open tool46 is fully wired through pinned native preparation, one-use operation, staged shared CodeMirror load and domain openFileTab. TypeScript and UI dirty/shared-open tests passed; full core35 + UDS26 + protocol9 passed after fixing a shutdown race by joining cleanup workers before returning (16 immediate restart cycles included). Native avOmdR failed a fixture field-name assertion, corrected panelId to id; 6rYOCg exposed the expected dirty-indicator debounce after undo, fixture now waits for the actual clean guard; aIoT3w verified open/reopen/retry/shared dirty text/undo/secret denial and visually inspected editor-open.png, then hit a safe no-effect revision conflict in subsequent rename. Fixture now focuses the original editor before closing the added one and only retries a definite no-effect revision conflict with a fresh revision/key, at most3 attempts. Native46 run4 is /tmp/lomi-mcp-editor-open-native-4.log; no overall PASS claim yet.

Atomic save foundation: core atomic_file::replace pins the canonical parent with NOFOLLOW descriptors, hashes/checks the exact current file, stages with openat EXCL, preserves ordinary ownership/mode, rechecks identity/revision, then renameat + parent fsync. A post-rename durability error is explicitly uncertain. This helper is now used by ordinary Unix editor saves; MCP save is NOT exposed yet. Its2 core tests passed (stale revision, executable mode, cancellation and parent replacement link), all6 existing native editor tests passed. Non-Unix code is retained and unqualified. No new dependencies. Pending: finish native46/unlocked input, all-target/wire checks, expose save through operation-bound buffer/revision authorization, all filesystem mutations and remaining P5/P6.

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

### Directory move qualification and event pagination — 2026-09-23

Regular-file native run `Rf7xsu` passed nested editor open, file rename/move, dirty-buffer/source provenance, retry after missing source, undo and clean close, plus the complete Android input form and browser/PTY checks. `files-move.png` was visually inspected. The final fixture check failed because it assumed all workspace operation events fit one100-event page; new file operations exceed this. Fixed fixture to drain up to32 bounded100-event pages, require strictly increasing sequence numbers and exact workspace for every event, require the cancellation receipt, and reach an empty page. The broker limit was not increased and pagination was not bypassed. Full run still needed for final PASS.

Added explicit `rename_directory` / `move_directory` DTOs/native adapter (files.rename grant), using source expectedDirectoryRevision and destination expectedParentRevision, exclusive descriptor-relative rename, no byte recursion/copy, descendant-preserving source provenance updates. Native foundation4tests PASS; protocol/helper wire5 PASS; TypeScript and three permission/dirty-folder/editor UI regressions PASS. Current full native48 run `/tmp/lomi-mcp-directory-move-native.log`, exec86387, tests file then directory renames/moves with the same dirty document, followed by full existing regression and corrected event-pagination check. Do not edit frontend while active. Directory-move core full suite and latest Clippy still to run. Trash and all remaining P5/P6 items remain required.

### Directory native assertions and Android matrix continuation — 2026-09-23

`njQgCi` native run passed all editor save + file/directory create/rename/move assertions, including dirty nested document preserved at `mcp-container/mcp-renamed/final.txt`, exact disk bytes, receipt replay and undo/close. `files-move.png` visually inspected. The run later failed during Android form touch with CONTROL_REVOKED (after successful guest Unicode typing); native protection stayed fail-closed. Cleanup confirmed host exit0, owned emulator stopped and private ADB stopped. Do not report this run as overall PASS. Core42+UDS29+protocol9 PASS (`/tmp/lomi-mcp-directory-move-core-suite.log`), latest production workspace Clippy PASS, wire5/5, UI3 and TypeScript PASS.

Native fixture now captures `failure-ui.json` (focused/visible/minimized, main document visibility/focus/body) and `failure.png` BEFORE teardown to diagnose future revoked focus leases. Also added an explicit Explorer refresh wait before file-move screenshot. New Android matrix assertions test actual Backspace down/up removing a whole emoji, Unicode restoring exact text, quarter-turn1 landscape and0 portrait snapshots/capture geometry. Uses existing MCP lease/sequence4..8 and existing managed input router, no bypass. Fixture helper `Wire::android_packet` settles each exact input receipt. First compile xA85d4 found two moved JSON values; changed capture_args and form_snapshot_args uses to clone. Fresh run `/tmp/lomi-mcp-android-keys-rotation-native-2.log` is active; previous log without-2 is a compile failure, not qualification. No frontend changes until active fixture exits.

### macOS native Trash and Android matrix (2026-09-23; in progress)

The active48-tool run `lomi-mcp-control-0o22LS` has passed the native Trash section: Cancel retained exact dirty buffer and disk; Save cancelled removal and retained newly saved bytes; Discard moved the original bytes and inode into the macOS system Trash and removed the editor panel. Retrying each request returned its original receipt. The unique fixture Trash entry was removed only after exact bytes/inode verification. Evidence: `files-trash.json`, visually inspected `files-trash-dialog.png`. This is a section result; the full run is still in progress and must not yet be labelled PASS.

Android new key tests in `3cAV5z`: Backspace down/up removed exactly one whole emoji and text input restored exact Unicode. A fixture parser correction was required for UIAutomator numeric XML entities. Landscape snapshot/capture passed; next rotation failed because resizing revoked the lease. Fix in Android runtime has UI evidence: held replacement-frame test pauses readiness without revocation; a later dialog still releases the exact lease. Existing dialog/field/hidden-panel revocation tests pass. Full native rotation retry is part of0o22LS.

Trash backend primary API evidence: [Apple NSFileManager Trash](<https://developer.apple.com/documentation/foundation/filemanager/trashitem(at:resultingitemurl:)>) moves a URL into system Trash. Installed `trash5.2.7/src/macos/mod.rs` explicitly selects `DeleteMethod::NsFileManager`; its default Finder/osascript route is not used by MCP. The library documents that Finder Put Back may be unavailable; manual restoration remains possible. Interrupted staging preserves app-private recovery data. Cross-volume source/staging returns unsupported before rename; other operating systems are not qualified.

The above0o22LS run subsequently PASSED in full. Android quarter-turn1 and0 each returned succeeded on input sequence7/8, snapshots reported the expected rotation and PNG geometry switched landscape/portrait correctly. The lease survived intentional resize. Backspace removed a whole emoji, normalized XML verified exact restored text, and existing form/touch/release assertions passed. `android-cleanup-after.json` confirms processAlive=false, forced=false, privateAdbStopped=true. This verifies these matrix cases on macOS ARM64 only, not the remaining P4 fault/two-view/two-agent matrix or full v1.

## P5.1 native previews and prepared-Trash revoke — 2026-09-23

Native full48-tool run `qZ4ywI` PASS, `/tmp/lomi-mcp-preview-native-4.log`,
exec94075 exit0. Artifacts under
`/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-qZ4ywI`.
macOS27/AppleM3/ARM64 and the previously qualified isolated Android root.

PNG actual canvas pixels preserve alpha (expected browser premultiplication
rounding219 vs220) and2x3 geometry. Two-frame GIF produces the first yellow frame
as static PNG. SVG renders a green64x48 vector; script/onload, external SVG image
and foreignObject image made zero requests to the instrumented loopback server.
Markdown loads approved raster/SVG only, denies .env.png and a link to an otherwise
approved image, and leaves remote image as a link. Preview→edit→split keeps the
same panel/document, dirty buffer, and one-step undo to clean. Receipt retry uses
the original operation. Source bytes are unchanged. No ordinary raw image read
was invoked (`editor-previews.json`); no escaped SVG code observed. The native
split screenshot was inspected. All prior browser/PTy/Codex/Trash/Android input
and rotation assertions also passed. App data removed, host exited, owned Android
process exited gracefully and private ADB stopped; user emulator untouched.

Native fixture corrections:0zyMaR used an incorrect tool name (no preview pass);
w5NBG7 verified PNG then encountered a definite no-effect revision conflict during
post-close editor restoration; LDAVDw passed previews then failed an old assertion
assuming the terminal always occupied the second pagination row. The final test
waits for editor/projection readiness and drains bounded pages by exact panel ID
with duplicate-ID checks. Authorization/receipt conflict guards were preserved.
Closing active panels can create a safe scratch fallback (D17), so ordinal
pagination assumptions are invalid.

Final independent checks: core45PASS+1ignored, UDS32PASS, protocol9PASS
(`/tmp/lomi-mcp-preview-final-core.log`); the opt-in full-disk test is separately
ignored in the ordinary run. Production workspaceClippyPASS; pnpmcheckPASS;
wire5PASS; old previewUI13PASS plus newUI2PASS; decoder2PASS. New UDS coverage:
asset permit forgery/secret/link/root swap/budget/release/revoke, and revoked clean
Trash in Running records Cancelled/effectNone after reopening receipts, preserving
source bytes (`/tmp/lomi-mcp-trash-prepared-revoke-2.log`).

### Actual ENOSPC (32 MiB isolated disk images)

HFS+ `gZXZ2N`: PASS after32747520 filler bytes. APFS `5NZzmp`: PASS after31596544
filler bytes. Each test observed actual ENOSPC, verified failed atomic replacement
preserved original bytes/inode and removed its temporary file; Trash could not
persist its plan and left the source intact. After deleting only the fixture
filler, save and staged Trash completed successfully. `result.json` confirms each
volume detached and its image removed. Logs remain under the matching
`lomi-mcp-full-disk-*` temporary folders; command
`node tests/native/run-mcp-full-disk.mjs` now defaults to APFS, optional explicit
`LOMI_MCP_TEST_FILESYSTEM=HFS+` tests the other filesystem.

Two initial APFS assertions were corrected: the failed-save temporary changed the
parent revision, and a later run showed APFS reclaiming enough space to persist a
small Trash plan even after a large write returned ENOSPC. The test now refreshes
that revision and checks either a definite pre-effect storage failure, or exact
journal/inode preservation after successful staging followed by an injected failed
OS handoff. It does not assume every small write must fail after a large write.
The final APFS run took the actual pre-effect-storage-failure branch. Failed fixture
images4hCHQo/Z4dkzW were detached, inspected, then removed; their logs remain. This
qualifies save/Trash foundations; receipt/artifact/APK full-volume cases remain P6.

Image decoder suite now3PASS: adds2560x1280→1280x640 bounded conversion and a real
JPEG APP1 EXIF orientation6 case producing2x3 instead of3x2. Log
`/tmp/lomi-mcp-preview-image-tests-3.log`. The native prepared transfer reserves128
bytes below8MiB for its optional permit metadata.

### Native Git status — 2026-09-23

PASS49 tools in `lomi-mcp-control-wwumF2/result.json`, log
`/tmp/lomi-mcp-git-status-native.log`. Git2.54.0 AppleGit157, macOS27 ARM64.
Real helper→UDS→native guarded Git returns literal status, bounded immutable pages
while working-tree files change, rejects foreign-workspace cursor and omits .env
and symlink entries. Actual Settings separate git.read grant selected in fixture.
Additional git-status.json retains evidence. Full Android input/rotation, browser,
PTY/Codex, file/editor/preview/Trash regressions passed. The fixture wrapper
stopped its own native app, managed Android and private ADB. No user Git repository
was staged/committed; only the isolated test repository was initialized.
Wire5, status UDS test, Settings UI test, pnpmcheck and workspacecheck PASS.
Further diff/history/remotes tools are in progress and not covered by this binary.

### Native53 guarded Git observations

PASS `lomi-mcp-control-eqiRTa`, `/tmp/lomi-mcp-git-observations-native.log`, exit0.
Git53 paths through production helper, UDS and native app: exact paged UTF16
working patch, staged patch, changed revision refusal, secret-path denial,
commit history/full exact message, redacted remote URL authority. All previous
native cases passed. `git-observations.json` contains actual replies; owned
native app/Android/privateADB stopped by wrapper. macOS27 ARM64, AppleGit157.
Core46/UDS34/protocol9/wire5 PASS. Historical first-parent/root comparisons and
commit file lists were added after this binary; separate real-UDS merge test
PASS `/tmp/lomi-mcp-git-commit-files-test-2.log`. Read-only view tool54 is still
under native qualification; UI2 with zero ordinary Git reads PASS and screenshot
inspected. Broader git.execute mutation code trust and approval are not yet done.

### Native54 read-only Git views

PASS BGgT3s (`/tmp/lomi-mcp-git-open-native.log`, exit0): actual native diff and
commit views show prepared guarded content, commit's selected-file patch reads
through its main-only permit, ordinary git_diff/git_commit_details/git_commit_diff
were called zero times. Both operation retries returned the original receipt;
scoped closes succeeded. Evidence git-views.json plus all prior native regression
artifacts. macOS27 ARM64 only. Wrapper cleaned owned native resources.
UI2PASS with restored descriptors denying ordinary reads and a reviewed light
commit screenshot. Real-UDS test verifies one-use preparation, exact body revision
and panel ACK, secret/unknown permit denial, root replacement and release. Core46,
UDS35, protocol9 and wire5PASS. Git mutation flows are not covered or implemented.

## Git stage/unstage and shutdown ownership (2026-09-23)

Tool55 wire/schema5 PASS (`/tmp/lomi-mcp-git-mutation-wire.log`), core50+1ignored,
UDS36 and protocol9 PASS (`/tmp/lomi-mcp-git-mutation-core-all.log`). The separately
ignored full-disk integration remains qualified by its earlier explicit mounted
volume runs, not by this ordinary invocation. Production workspace lib/bins
Clippy PASS (`/tmp/lomi-mcp-git-mutate-clippy-final.log`); pnpmcheck PASS
(`/tmp/lomi-mcp-git-mutate-ts-final.log`). Git UI3 PASS, including the real bridge
and native-Modal component against mocked commands; Settings/application-close7
PASS (`/tmp/lomi-mcp-git-grants-close-ui.log`). These mocks are not native proof.

Actual native55 increment at `lomi-mcp-control-vmbB5F/git-mutations.json` passed
Cancel, changed bytes -> failed/none/REVISION_CONFLICT, MCP cancel -> cancelled/none,
stage and unstage -> succeeded/complete. Every case first awaited an exact main UI
decision; repeat returned the same operation/state. Index and disk bytes were
observed independently with fixed Git. Native approval screenshot inspected in
dark theme; Chromium approval screenshot inspected in light theme. Full combined
run then stopped at Android input with CONTROL_REVOKED after native focus changed
from true (android-running-ui.json) to false (failure-ui.json). This is NOT a full
native55 PASS. cleanup.json and android-cleanup-after.json confirm owned host exit,
app-data removal, confirmed emulator stop and private ADB cleanup. An earlier
hKMb2c run had a fixture selector mismatch: the product's native HTML dialog was
present, but the fixture matched only explicit role attributes. Corrected the
selector without changing application behavior. Additional Android request-stage
artifacts were added before rerun to distinguish claim from first-input failure.

Shutdown qualification includes a deterministic detached-worker barrier, cancelled
close caller, second close and immediate database reopening with zero retained
Broker Arcs. Native close preparation rejects new native work and waits for an
already detached owner before it can resume. Revoke still passes with policy,
storage and terminal-observer locks deliberately held. Tokio's documented
spawn_blocking non-abortability and JoinHandle destructor completion semantics
explain the ownership fix: https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html
and https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html.

Commit is not exposed yet. Isolated Git2.54.0 prototype at
`/tmp/lomi-mcp-commit-message-probe.json` confirmed --cleanup=verbatim preserves
leading/trailing spaces, Unicode and trailing empty lines, but adds a final LF
when the supplied message lacks one. The commit contract must show that exact
Git-normalized stored message before approval; do not silently promise byte equality
with an unterminated input. No user repository commit was created by this probe.

## Native55 completed with user-assisted focus — 2026-09-23

`LOMI_MCP_WAIT_FOR_FOCUS=1 LOMI_ANDROID_PRODUCT_DIRECTORY=… pnpm test:mcp:control`
passed, log `/tmp/lomi-mcp-git-mutation-native-5.log`, artifact directory
`/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-Im0Xl9`.
The user kept the test window foreground. Actual stage/unstage, cancellation,
stale approval, retry, Android Unicode/emoji input, key operations, rotation and
lease release passed. Full 55-tool fixture and real Codex CLI connection passed.
`cleanup.json`: hostExited=true, appDataRemoved=true, exitCode=0.
`android-cleanup-after.json`: processAlive=false, forced=false, privateAdbStopped=true.
No user emulator/shared ADB was stopped. The added optional human-focus wait
changes fixture timing only; real production focus remains required.

The large-path native UDS approval test additionally passes: >32 KiB preview
returns ResourceExhausted before user approval, leaves index unchanged, records
failed/none and deduplicates retry (`/tmp/lomi-mcp-git-mutation-budget-2.log`).
Commit-preview test passes with full staged-set validation, native identity and
exact Unicode/whitespace message. Execution is not yet exposed. Primary sources:
[git-commit](https://git-scm.com/docs/git-commit),
[git-cat-file](https://git-scm.com/docs/git-cat-file),
[git-diff-tree](https://git-scm.com/docs/git-diff-tree).

## Commit adapter verified — 2026-09-23

- Native core: `/tmp/lomi-mcp-commit-execution-2.log` (4 Git foundation tests):
  initial/ordinary commit, exact staged bytes despite newer working bytes, exact
  message/identity, stale preview, hooks changing message/index, nonzero hook and
  cancelled hook. Post-spawn divergence stays unknown; no rollback.
- Core52+1ignored / UDS36 / protocol9: `/tmp/lomi-mcp-commit-core.log`.
- UDS exact native commit, durable retry and forged-result rejection:
  `/tmp/lomi-mcp-git-commit-uds.log`.
- UI3 including exact readonly message, staged-object list, default Cancel,
  Escape/revoke, approved dispatch and reachable controls at 900x600:
  `/tmp/lomi-mcp-commit-ui.log`; rendered screenshot inspected.
- TypeScript/AI checks, helper5 and all-target MCP Clippy pass:
  `/tmp/lomi-mcp-commit-ts-final.log`, `/tmp/lomi-mcp-commit-wire.log`,
  `/tmp/lomi-mcp-commit-clippy.log`.
- Production native stdio/UI/commit regression PASS:
  `/tmp/lomi-mcp-git-commit-native.log`, `lomi-mcp-control-MIk0rL`;
  `git-mutations.json` includes cancelled commit and successful exact commit.
  Native approval image inspected. Cleanup confirms exit0 and app-data removal.
  Android was not configured for this run; its old summary RUN value is not
  Android evidence. The fixture generator is corrected to NOT_CONFIGURED.
  `Im0Xl9` earlier in this session separately qualifies Android input/rotation.

## Exact fetch verified — 2026-09-23

- `/tmp/lomi-mcp-fetch-foundation.log`: 2 PASS, actual native Git with owned
  source/local/bare remote. Preparation makes no connection; only the selected
  tracking ref is fetched, tags/refmap/prune/FETCH_HEAD are constrained, local
  HEAD/index/status are preserved, changed config is rejected. Controlled SSH
  transport is not invoked in preview and is stopped after revoke in execution.
  Credentials are redacted; helper protocols/ambiguous names/refs are rejected.
- `/tmp/lomi-mcp-fetch-uds.log`: PASS, git.network denial independently of
  git.write/git.execute, exact tracking result, forged-ACK rejection and retry.
- `/tmp/lomi-mcp-fetch-ui.log`: 4 PASS, explicit dependent network grant, reset
  on uncheck, precise remote/ref preview, Cancel and approved dispatch.
  UI/native screenshots inspected.
- TypeScript and all-target MCP Clippy PASS:
  `/tmp/lomi-mcp-fetch-ts-final.log`, `/tmp/lomi-mcp-fetch-clippy.log`.
- Native stdio/main/Settings/real Git regression PASS:
  `/tmp/lomi-mcp-git-fetch-native.log`, `lomi-mcp-control-f4sv4G`;
  cancelled and completed fetch against owned local bare remote. Android
  NOT_CONFIGURED. Cleanup: hostExited=true, appDataRemoved=true, exitCode=0.

Primary [git-fetch reference](https://git-scm.com/docs/git-fetch/2.54.0) was checked
for explicit refspec/refmap, tags, pruning, submodules and maintenance options.
Fetch observes the resulting local tracking commit; it cannot promise the server
will retain that value after the connection. Trusted Git helpers are unsandboxed.

Push native increment (2026-09-23): `lomi-mcp-control-S6Yqf7` PASS, 55 tools.
`git-mutations.json` confirms cancel and exact approved branch publication to the
owned fixture bare remote; all previous native Git/file/editor/browser/PTY cases
passed. `result.json` correctly records Android NOT_CONFIGURED; Im0Xl9 is the
separate Android input proof. `cleanup.json` records host exit0/app-data removal.
Push approval screenshot visually inspected; no real user remote was contacted.
Log: `/tmp/lomi-mcp-git-push-native.log`. Foundation tests additionally preserve
another client's concurrent remote commit and reject non-fast-forward publication.

Discard native oDcSDI: all Git cases (including discard cancel/stale/success),
file/editor/browser/PTY assertions passed. The full run FAILED at final Codex
pairing: external CLI updated to 0.156.1 while the probe pinned 0.155.1, so its
preflight exited before requesting pairing. The misleading renderer timeout was
replaced by waiting on the existing child-exit/approval-ready loop. Host exit0 and
app-data cleanup passed. No Android was configured. Probe pin is updated for a new
0.156.1 qualification; this is not yet a completed client compatibility claim.

Git pull foundation (2026-09-23) PASS: actual fixed Git/owned repositories verify
fast-forward into a previously missing directory, linear rebase preserving local
commits, retained native conflict/index stages, changed remote without integration,
ignored-file collision and dirty/stale-file refusal, and pre-rebase process-group
cancellation. Logs `/tmp/lomi-mcp-pull-foundation-2.log` and
`/tmp/lomi-mcp-pull-uds-2.log` include broker success/partial classification,
forged-ACK refusal, scope denial and no replay. UI4 PASS at
`/tmp/lomi-mcp-pull-ui.log`, native approval UI images inspected. CodeMirror dirty
guards are synchronous and compare the retained canonical disk source. General
snapshot traversal now treats missing safe parent directories as absent targets,
while opening every existing component NOFOLLOW. All-target MCP Clippy PASS at
`/tmp/lomi-mcp-pull-clippy-2.log`; TypeScript PASS at
`/tmp/lomi-mcp-pull-ts-final.log`.

Codex0.156.1 image/typed-error probe PASS (`/tmp/lomi-mcp-codex-01561.log`),
providerCalls0/privateConfigWrites0. The existing app-server/JSON-RPC usage is
checked against installed CLI help and https://learn.chatgpt.com/docs/app-server .
This is separate from the full native control pairing currently running in CnErzk.

All-Git native CnErzk: every mutation scenario PASS, including exact partial
remote-change/conflict pull receipts in `git-pulls.json`. The overall run TIMED OUT
at its240s build-plus-regression deadline during later browser qualification;
Git passed, but the full run is NOT PASS. Build alone took95s while the core suite
ran concurrently. The outer runner budget is now600s; per-operation deadlines are
unchanged. Host process exited on SIGTERM, no fixture host remained, and its exact
private app-data directory was then removed; cleanup.json records manual cleanup.
Repeat runs without concurrent compilation. Full core57+1ignored / UDS37 /
protocol9, helper wire5 and production Clippy PASS in `/tmp/lomi-mcp-all-git-*.log`.

All-Git native repeat xSPHym PASS (55tools, macOS ARM64): cancel/stale/actual
stage/unstage/commit/fetch/push/discard; pull cancel, fast-forward, changed remote
(failed/partial), linear rebase and preserved conflict (failed/partial), all with
exact durable retry. Git approval screenshots visually inspected. Full preceding
file/editor/Trash/preview/PTY/native browser regression and real Codex0.156.1
control pairing passed. Android NOT_CONFIGURED; Im0Xl9 remains separate Android
input evidence. cleanup.json confirms host exit0/appDataRemoved=true. Log:
`/tmp/lomi-mcp-git-pull-native-2.log`. No user remote, client config, paid provider,
existing emulator or shared ADB was mutated by this test.

## Same-workspace panel movements — 2026-09-23

Full native56 `lomi-mcp-control-KRWX3H` PASSED on the recorded macOS ARM64 host.
Production helper/private IPC and Settings opt-in exercised reorder_tab, dock_tab
(browser and dirty file into the running dev-server terminal) and move_pane.
The same terminal session/command kept running, browser generation/form survived,
and shared document ID, unsaved text, buffer revision and undo were preserved.
Every repeated request returned the same receipt. Modal denial recorded
TARGET_BUSY/effect none; a hidden target returned PANEL_NOT_RENDERABLE. Artifact
`panel-moves.json` holds the actual results. `mixed-layout.png` was inspected;
the main WK snapshot does not contain native child-browser pixels. Post-docking
DOM operations, navigation/cancellation and takeover regressions also passed.

The full preceding Git (all seven mutations), editor/files/Trash/previews,
PTY/browser, real Codex0.156.1 and shutdown regression passed. cleanup.json:
hostExited=true, appDataRemoved=true, exitCode=0, signal=null. Android was
NOT_CONFIGURED here; Im0Xl9 remains its separate prior evidence. Log:
`/tmp/lomi-mcp-layout-native-2.log`.

The first attempt czAquR failed in the pre-existing ordinary terminal attachment
fixture, before panel movement. It observed a PTY ID and sent input while shell
startup had not reached a parsed prompt. The fixture now waits up to ten seconds
for that prompt before its simulated human input. Cleanup passed. This change
adds a readiness assertion and does not weaken the output or attachment tests.

Current checks: core57+1ignored, UDS38, protocol9, wire5, model2, Settings UI1,
TypeScript and production/all-target MCP Clippy passed. The UDS case includes
secondary target scope, distinct move permission, forged runtime ACK rejection,
stale native claim, durable retry and unknown effects after uncertain publication.
Same-workspace movements are verified within the explicit initial kind/readiness
limits. Cross-workspace moves, other panel kinds and full P5.3 ancestor lifecycle
remain IN_PROGRESS/TODO as recorded; this does not complete the whole v1 gate.

## Workspace close native qualification — 2026-09-23

`lomi-mcp-control-YtiSKE`, full56-tool macOS ARM64 runner: PASS. The shared native
Save/Discard/Cancel dialog preserved Unicode buffer content on user Cancel and
MCP cancellation. Save wrote the exact bytes and retained the workspace with
partial effects. Discard removed it; exact retries retained all original receipts.
Post-close buffer access was denied. A protected busy/human workspace was refused.
A separate owned idle PTY and WK browser closed successfully while the other two
native PTYs remained. Files on disk and other workspaces remained intact.

Artifacts: workspace-close.json, workspace-close-runtime.json, visually inspected
workspace-close-guard.png and mixed-layout-browser.png. All preceding file/editor/
Trash/preview/Git/PTY/browser and real Codex0.156.1 assertions passed. Android was
NOT_CONFIGURED, not newly qualified. cleanup.json confirms host exit0 and private
app-data removal. Log `/tmp/lomi-mcp-workspace-close-native.log`. Core57+1ignored,
UDS40, protocol9, wire5, TypeScript, UI2 and both Clippy sets passed. The UI set
separately proves failed Save preservation and opt-in dependency revocation.

Workspace closure is qualified for files, owned Git views, idle owned PTYs and
owned native browsers. Android/chat/plugin descendants and project-wide closure
remain required work; this increment does not complete P5.3 or MCP v1.

## Cross-workspace native transfer — partial evidence, full run failed

`lomi-mcp-control-ZXX06z`: transfer assertions passed in workspace-transfer.json,
including exact forward/back receipts, live PTY/command identity, dirty editor
identity/content, retained WK form and denied old-source/foreign-destination reads.
The later guarded file workspace-close cases passed. The idle PTY/browser close
then returned outcome_unknown with prepared null closure flags; failure.png shows
a stopped PTY in the retained workspace. No close retry or inferred success. The
full run is FAILED. Cleanup exited the fixture via SIGTERM and removed its app data.
Trace-only rerun **tLJg5N PASS** with host exit0 and private app-data removal.
All transfer and close assertions passed, but the ineffective JS invoke wrapper
recorded no events, so this was not evidence of a race fix.

Diagnostic **Ik8p9l FAILED** after one successful idle PTY/browser workspace
closure. Native trace confirms error null and accepted success ACK for that
closure. The next terminal-create operation returned claimed revision conflict
with unknown effects, so the fixture did not replay it or reach six closures.
Cleanup terminated the host and removed app data. Log
`/tmp/lomi-mcp-close-race-native.log`.

Targeted regression checks after source changes: model5 PASS (latest-session
removal preserves unrelated state, exact changed descendants refuse; equal value
with new references accepted), UI1 PASS (typing during held native close retains
RAM; failed Save, Cancel, Save then fresh Discard remain guarded), TypeScript
PASS, expanded UDS transfer1 PASS (original run receipt after moved or cancelled
transfer, changed key fingerprint rejected, new execution in old workspace denied,
only one dispatch; cancellation resolves the migrated generation), MCP all-target
Clippy PASS. The rendered post-close UI was inspected. Full native requalification
is active; these targeted checks alone do not resolve the previous native failure.

## Workspace viewport race — native cause and regression

Full run **tiONrr FAILED** during second terminal creation; the first runtime
workspace closed successfully. Its new moved-run retry assertion passed with the
same still-running original operation ID. Cleanup removed app data and terminated
the fixture host. `/tmp/lomi-mcp-close-rebase-native.log`.

Focused diagnostic **19i2LE FAILED** and recorded the exact domain difference in
`domain-conflicts.json`: the new empty FileTab's `position` changed from absent to
zero anchor/head/scroll values during claim. No resource identity changed. Native
cleanup exited0 and removed app data. This confirmed the cause instead of inferring
it from an occasional successful rerun.

The bridge signature now excludes actual file viewport positions, including
mixed layout files, while preserving arbitrary plugin state and every resource,
selection and layout field. After claim it retains the latest equivalent session.
Model6 and TypeScript passed. Focused **4gguA9** has six actual successful owned
PTY/browser closure artifacts and accepted native close ACKs. All other native
PTY contexts remained equal after removal of only the target generation. Its
native result was passed, exit0 and app-data cleanup passed; the outer script
failed on a missing full-profile HTTP counter file. The script has been corrected
to validate the focused profile and all six artifacts separately. That correction
and full native regression still need successful reruns before overall PASS.

Focused wrapper requalification **wNJLEh PASS**: six fresh workspace/native idle
PTY/native hidden browser closures, exact original close receipts on retry,
unchanged other terminal contexts, zero denied-origin requests, native exit0 and
private app-data removal. The script validates the explicit focused profile and
all six artifact records. This profile does not run the unrelated full-domain
suite. `/tmp/lomi-mcp-viewport-stress-final.log`.

Final source checks before full native regression: core57+1ignored, UDS41,
protocol9, model6, Settings/closure UI2, full pnpm check, production workspace
Clippy and frontend build PASS. A separate process-helper test entry is ignored
by design, with its invoking test covered by the core suite. Diagnostic observer
names are absent from generated production JS. No dependency changes.
Full native **YWPPVZ PASS**, exit0 and private app-data cleanup. All six fresh
idle PTY/browser workspace closures succeeded, as did original running-command
receipt replay while transferred, retained browser form and dirty editor/Undo,
protected targets, actual guarded Save/Discard/Cancel and all earlier full-domain
assertions. The mixed layout and separate native browser form screenshots were
visually inspected. Android NOT_CONFIGURED, not a new Android qualification.
`/tmp/lomi-mcp-viewport-full-final.log`. The confirmed viewport race and transfer
retry defect are resolved with these native and regression proofs; full P5.3 and
MCP v1 remain in progress.

### Project closure, native57 (macOS ARM64)

Focused 5ranoM PASS, exit0/hostExited/appDataRemoved. Artifacts under
`/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-5ranoM`:
`project-close.json`, `project-close-native.json`, `project-close-guard.png` and
`project-closed.png`. Explicit Settings approval included both existing project
workspaces; the created guarded workspace was added only by its approved creation.
Native closure removed the project's three workspaces, two exact idle PTY
sessions and owned hidden WK browser. The other project remained, and its default
terminal descriptor did not start. Cancel/MCP cancel preserved dirty text; Save
returned closed=false/partial and retained the project; Discard returned matching
closed=true/complete with durable exact retry. Final native terminal contexts were
empty as expected. Guard and neutral final screenshots visually inspected. The
Settings screenshot captured the top of its page, not its offscreen permission
form, so visual permission evidence remains the separate inspected UI screenshot.
Android NOT_CONFIGURED; this is no new Android qualification.

Core57+1ignored/UDS42/protocol9, UI3, model7, pnpm check, wire5, production workspace
Clippy and MCP all-target Clippy PASS; logs `/tmp/lomi-mcp-project-close-*`.
Full native57 regression 88IOPg PASS, exit0/hostExited/appDataRemoved; all previous
file/editor/Trash/previews/Git/PTY/browser/Codex assertions, transfer retry and six
fresh runtime closure cycles passed. Android NOT_CONFIGURED. Log
`/tmp/lomi-mcp-project-close-full-native.log`.

### Project opening and multiple roots, native58 (macOS ARM64)

Core57+1ignored/UDS44/protocol9, UI4, wire5, TypeScript and MCP all-target Clippy
PASS. The first native UZnAc5 run passed all behavior assertions but needed SIGTERM
at cleanup because its deliberately dirty fixture blocked normal close. This is
not a clean overall PASS. The fixture now saves that exact temporary document
through MCP after retention assertions; the wrapper rejects forced host exit.

Focused DJziSo PASS: rejection, MCP cancellation, root replacement, and two
explicitly approved canonical roots with equal retry keys anchored in different
projects. Pending requests granted no reads; successful publication granted only
the approved roots. The original dirty document identity/text remained unchanged,
and native terminal contexts remained empty. The later fixture cleanup save
succeeded with the exact 33-byte text; host exit0/private app-data cleanup passed.
Native Settings approval and blank editor screenshots were visually inspected.
Log `/tmp/lomi-mcp-project-open-native-2.log`; artifacts under
`/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-DJziSo`.

Focused project-close regression Zdqpsp PASS after the root-binding refactor,
including normal host exit0/private app-data removal. Log
`/tmp/lomi-mcp-project-open-close-regression.log`. Full native58 regression is
running, not yet qualified. These runs do not configure or requalify Android.

Full native58 H2IkCx PASS after the project-binding change: all previous actual
file/editor/Trash/previews/Git/PTY/browser/Codex assertions, retained runtimes and
original retry receipt after transfer, plus six fresh runtime workspace closures.
Wrapper exit0, native exit0, private app-data removal. Log
`/tmp/lomi-mcp-project-open-full-native.log`. Android NOT_CONFIGURED; previous
Android qualification is not promoted to this source without a new emulator run.

### Settings opening, native59 (macOS ARM64)

Focused KzW0fU PASS: all nine enum sections selected in the actual Settings window
through stdio MCP and the trusted native adapter. Every initially hidden window
became visible, and the matching navigation item was observed. Every exact retry
returned its original completed receipt. The original dirty document ID/text,
layout, native terminal contexts and four preference files remained unchanged.
Only after those assertions did MCP save the isolated fixture file for ordinary
close. Native exit0/wrapper exit0/private app-data removal passed. The actual Editor
page and the separate permission-form screenshot were visually inspected.
`/tmp/lomi-mcp-settings-open-native.log`; artifacts under
`/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-KzW0fU`.

Core57+1ignored/UDS45/protocol9, wire5 (59 tools), Settings UI1, TypeScript,
workspace check and production workspace Clippy PASS. The first new UDS permit
test caught that global revoke was not immediately reflected in its local permit;
the Settings permit now checks the global atomic authorization counter and live
session as well as its operation permit. The rerun and full suite passed. This
fix is qualified for the new Settings path; it does not claim a new audit of every
other existing permission path. Full native58 H2IkCx remains the latest unrelated
domain regression. Android NOT_CONFIGURED in the focused Settings run.

### Settings read (catalog60) — qualification active

Core57+1ignored/UDS46/protocol9, wire5 (existing catalog budget), Settings UI2,
TypeScript/workspace check/production Clippy PASS. Permission screenshot inspected.
First native ThP0hB failed during fixture compilation (missing `tauri::Emitter`
import for a test-only event). No application host ran; after verifying the exact
config/session, fixture app data was removed and cleanup.json annotated. Import
fixed; `/tmp/lomi-mcp-settings-read-native-2.log` is running, not yet a PASS.

Native60 qqYreQ PASS on rerun: all four actual provider snapshots; two distinct
shortcut pages with the same revision; foreign workspace denied; four preference
files unchanged by reads. A real Settings command changed editor defaults to
8/tabs, MCP observed the provider and rejected its old revision. Replacing only the
isolated editor-preferences fixture with malformed bytes produced recovery_required
with retained 8/tabs values, preserved exactly those bytes and disclosed no raw
error/path/content. Dirty text/document identity and native terminal contexts were
unchanged. Fixture restoration/save preceded normal exit0/private app-data removal;
wrapper exit0. `/tmp/lomi-mcp-settings-read-native-2.log`, artifacts under
`/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-control-qqYreQ`.

### Settings editor writes, catalog61, in native qualification

Implemented explicit settings.read/write permission, revision-bound editor field
patches, native stored-file plan and exact expiring Settings decision. The plan
uses the existing editor preference validator, mutex and change event; pinned
atomic replacement or no-clobber creation protects native stored revisions.

Core58+1ignored, UDS47, protocol9, wire5/catalog61 under the existing 700 KB budget,
TypeScript, UI2 and production/MCP all-target Clippy PASS. UDS covers 12 write
scenarios with an actual atomic file adapter: create/replace, reject, cancel,
concurrent content, symlink, rejected preparation, provider/plan mismatch, global
revoke before/during write, uncertain publication and exact idempotent retries.
UI verifies scope dependencies, exact plan/decision rendering, main queue release,
stale snapshots and refusal during recovery; approval screenshot inspected.

First native X06AUX FAILED: create, replace, reject, cancel and concurrent-write
checks reached the recovery case, where rejected preparation remained awaiting_user.
The failure recorder excludes that receipt state. Corrected only uncommitted
Settings failures to transition awaiting_user → queued before recording failed/none;
success still requires the native Settings-approved result. Added regression;
all 47 UDS tests and the UI recovery test pass after the fix. First native host
required SIGTERM with the deliberately dirty test buffer; unique private app-data
was removed. This run is not successful qualification.

Corrected native run is tracked in `/tmp/lomi-mcp-settings-update-native-2.log`.

Corrected focused native61 **0GkHVO PASS**, wrapper/native exit0 and unique private
app-data removal. `settings-update.json` proves real create/replace, human reject,
MCP cancel, concurrent-file refusal, corrupt-file refusal, exact retries, retained
dirty document identity/text and working Undo, unrelated files/native contexts
unchanged, and Main unable to approve. The native Settings rendering was inspected
(the request continues below the scroll viewport); the UI-test screenshot includes
the full plan and both decision controls. This profile did not configure Android.

Full native61 **fCqMdY PASS**, `/tmp/lomi-mcp-settings-update-full-native.log`,
ordinary native/wrapper exit0 and unique app-data removal. Previous file/editor,
Git, PTY, browser, transfer/close and real Codex integration cases remain passing
with the Settings changes. Android NOT_CONFIGURED; this is not a new Android
qualification. Settings-specific native behavior is covered by focused 0GkHVO,
qqYreQ and KzW0fU runs.

### Terminal preference writes, catalog61, native qualification active

Added the closed `terminal_field` patch, an explicitly present scalar/null value,
existing native terminal validation and exact typed before/after comparison.
Nullable appearance leaves restore inheritance. Unrelated stored fields remain
unchanged, including optional defaults omitted by older valid files. The shared
preference-source helper reuses pinned atomic publication without moving caller
or section-lock ownership. No shell profile code is accepted or executed.

Core58+1ignored, UDS48, protocol9 plus the new required-null test, native terminal
field/validation test, UI2/final approval regression, TypeScript, wire5/catalog61
under 700 KB and final production/MCP all-target Clippy PASS. The UI screenshot
was inspected and its missing sentence space corrected. The write regression now
has 13 scenarios, including anchor removal during native application. Publishing
a removed Settings anchor revokes the permit before file effect; restored-target
lookup returns the failed/none receipt without replay.

Native 9wzISd FAILED at a fixture comparison of JSON 19 versus 19.0. Native aE3U51
FAILED at a fixture comparison of RGB #123456 versus the existing renderer's
RGBA #123456ff. Both had native exit0 and private app-data removal. Corrected
fixture checks numeric equivalence and exact engine RGBA. Current retry is
`/tmp/lomi-mcp-settings-terminal-native-3.log`; no native PASS is claimed yet.

Native KnBYQo completed seven terminal cases and preserved both original PTYs,
retained xterm instances/output and native process contexts. Its combined editor
regression failed because panel closure correctly selected a neutral blank editor,
while the fixture invoked Undo on the hidden original document. The fixture now
focuses that original panel through MCP before testing Undo. KnBYQo cleanup used
SIGTERM due to its remaining dirty fixture; unique app-data removed. Corrected
combined **hhAM3F PASS**, `/tmp/lomi-mcp-settings-terminal-native-4.log`, native and
wrapper exit0/app-data removal; seven terminal cases and all six editor cases.
The actual Settings terminal-approval screenshot was inspected with both controls
visible and correct spacing. Android NOT_CONFIGURED.

Native font-family and word-separator limits now count UTF-16 units like the
existing TypeScript validator. This prevents surrogate-pair text from producing a
native-successful file rejected by the UI. All three native terminal preference
tests and final production Clippy pass after the alignment. Final combined native
retry adds both oversized-Unicode cases (nine terminal cases plus six editor
cases): `/tmp/lomi-mcp-settings-terminal-native-5.log`.

Final combined **u6PGm8 PASS**, `/tmp/lomi-mcp-settings-terminal-native-5.log`,
normal native/wrapper exit0 and private app-data removal. Nine terminal cases
(create, color, inheritance reset, behavior, invalid number, oversized Unicode
font, oversized Unicode separators, concurrent-file refusal, corrupt-file refusal)
and all six editor cases pass. The same visible/hidden xterm instances, PTY
generations, process contexts and transcript markers survive changes; the original
dirty editor document and Undo also survive. Exact retries return the original
receipts. Android NOT_CONFIGURED. Source unfrozen for the remaining P5.4 work.

### Keyboard shortcut writes (catalog61)

**GJqZd0 PASS**, macOS ARM64, /tmp/lomi-mcp-keybindings-native.log. Twelve
scenarios: create, explicit null disable, reset stored override, pointer focus,
conflict, invalid shortcut, unavailable action, rejection, cancellation, concurrent
file change, changed plugin definitions after preparation and corrupt-file refusal.
Four successes, two cancellations and six failures have the expected complete/none
effects; exact retries retain original receipts. The fixture imports a disabled
package with the existing Settings importer; no plugin code is enabled/evaluated.
The run also repeats nine terminal and six editor cases with the original two
visible/hidden PTY runtimes, process contexts/output and retained editor/Undo.
Native and wrapper exit0; cleanup.json confirms appDataRemoved/hostExited. Native
Settings screenshot inspected. Android NOT_CONFIGURED.

TypeScript, model3, native keybindings3, UDS49, protocol11, wire5/catalog61 below
700 KB, UI2 and production workspace Clippy PASS. MCP all-target Clippy is recorded
separately in /tmp/lomi-mcp-keybindings-core-clippy.log. This qualifies shortcut
writes, not the remaining theme/plugin/Chat/Android-management/lifecycle/P6 scope.

Additional shortcut run **IR2YQ0 PASS**, /tmp/lomi-mcp-keybindings-native-2.log,
explicitly edits the original document before all shortcut cases and verifies its
identity, dirty text and original Undo afterward; two PTYs remain unchanged.
The same twelve/nine/six combined cases and ordinary exit0/app-data cleanup pass.
Final MCP all-target Clippy PASS.

### Builtin theme selection and appearance (catalog61)

**lt7OvZ PASS**, /tmp/lomi-mcp-themes-native.log, macOS ARM64. Native Settings
approval for DeepMono creation, light/dark/system appearance and restoring Lomi;
rejection, cancellation, concurrent-file and recovery refusals all preserve bytes.
Exact retries keep the same receipt. All nine cases plus terminal9/editor6 pass.
The original two visible/hidden xterms apply the computed theme without changing
PTY identity/output/native process contexts. The document made dirty before theme
changes retains exact text, identity and original Undo. Native approval plus real
light/dark screenshots inspected. Normal native/wrapper exit0; private app data
removed. Android NOT_CONFIGURED.

Native theme tests11, protocol12, UI2, wire5/catalog61 below700KB, TypeScript,
workspace check, production/MCP all-target Clippy and diff check PASS. No custom
selection, artifact import, recovery or CSS-isolation qualification is claimed.

### Critical theme controls (catalog61)

**XFBrB5 PASS**, /tmp/lomi-mcp-theme-protection-native.log, macOS ARM64.
An imported data-only fixture hides html/body through JSON styles and buttons/
dialogs through an external stylesheet. Agent Control Settings and a real shared
critical Modal remain readable, including a late native theme reload. Closing the
modal restores the selected CSS without changing preference bytes. Calling the
native macOS menu handler reopens the protected page after ordinary Settings was
hidden; this qualifies handler routing, not an automated OS-menu mouse click.
An exactly approved MCP switch back to Lomi succeeds with identical retry. Two
PTYs, native process contexts/output and the dirty document/original Undo survive.
Nine theme, nine terminal and six editor cases pass in the same native run. Normal
native/wrapper exit0; cleanup confirms host exit and app-data removal. Native
dialog/Settings screenshots inspected. Android NOT_CONFIGURED.

The broad initial Modal protection broke theme previews (two failures); explicitly
selecting critical decisions corrected it. UI regression25 and nested-modal1,
TypeScript, mcp-probe cargo check and production Clippy PASS. Expanded app probe
all-target Clippy exposed 11 native-fixture lint issues and nine existing app-test
issues (test-module placement, unnecessary test borrows, updater read count).
Fixture lint corrections and probe Clippy are being verified; the broader app-test
findings remain recorded for final qualification. The 11 fixture lints were
corrected; mcp-probe Clippy without app test targets PASS in
/tmp/lomi-mcp-theme-protection-probe-clippy-2.log. Further theme/plugin MCP work
was subsequently excluded by the user; this record describes the completed work.

### Pre-push full-domain regression and autosave fix

**RQy5Jv PASS**, /tmp/lomi-mcp-theme-protection-full-native.log, macOS ARM64,
catalog61. The complete default native suite passes, including real PTY, browser,
file/editor/Git, multi-project/layout/close and Settings flows. Normal native and
wrapper exit0; cleanup.json confirms hostExited/appDataRemoved. Android is
NOT_CONFIGURED in this run. The theme-specific protection and retained runtime
cases have separate XFBrB5 evidence above; the full suite does not repeat that
profile.

An intermittent UI close failure was reproduced by a new pending-autosave test:
before the fix, two saves occurred during shutdown. Workbench now cancels the
pending debounce and pauses automatic session saves before the final close save;
cancellation/errors resume them, and debounce captures the current session.
The new regression plus close/updater/terminal tests21 PASS, TypeScript PASS.
Frontend model174 + AI runtime17 and Vite production build PASS before milestone
publication. Vite reports the existing large-chunk warning; no installer or
release is produced.

Pre-push workspace Rust suite PASS: app220 + core58 + broker49 + protocol12 +
helper2 =341 passing tests;20 intentionally ignored native/crash/full-disk fixture
entries. `cargo fmt --all --check` and frontend formatting PASS after formatting
the four changed/new Rust files reported by the check. No behavior changed in that
formatting step. Expanded app-test Clippy's nine recorded baseline findings are
separate from the passing workspace test suite.

MCP real-process/wire tests5 PASS (/tmp/lomi-mcp-milestone-wire.log), including
closed schemas/catalog limits, discovery versions and oversized-line rejection.
