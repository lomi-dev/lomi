# MCP implementation decisions

Source of scope: `../../../lomi-mpc-docs/PROMPT.md` and its referenced contracts.

- D01: Keep a Cargo workspace under src-tauri, with GUI-free crates under src-tauri/crates and the existing src-tauri/Cargo.lock. Default member remains the app; MCP checks explicitly cover the workspace. This preserves existing Tauri commands and pre-existing lockfile changes.
- D02: Selected SDK is published rmcp 3.4.0 (Apache-2.0), downloaded with cargo info on 2026-09-23; schemars 1.2.2 (MIT). Pinned after source/API inspection and independent process tests. No HTTP, sampling, task or cache capabilities.
- D03: TLS 1.3 mutual authentication with rustls 0.23.45, tokio-rustls 0.26.5 and rcgen 0.14.10 passed the UDS/replay probe. The initially tested 0.23.44 was replaced after the advisory scan found RUSTSEC-2026-0285; authentication tests passed again. Early data, resumption and tickets are disabled. Session-only keys stay in RAM. Peer trust anchors must be supplied by an approved pairing, never accepted from a descriptor or peer itself. Enrollment pins the broker public certificate from the native Settings-generated configuration and requires explicit approval of the ephemeral helper certificate. The mode is session-only, with no keyring persistence or silent plaintext fallback. A new broker instance needs a new generated configuration.
- D04: Default routing is “prefer Lomi”. Project cwd is not a shell sandbox; browser origin grants are not network isolation; the client sandbox does not constrain the GUI broker.
- D05: Chat UI control remains in P5.6. Chat generation runtime keeps its existing no-tools boundary. No paid provider call is authorized by test fixture execution.
- D06: Native automation must use registered child views and existing resource owners. Qualification fixtures are opt-in and excluded from release.
- D07: All five optional P7 extensions remain NOT_SELECTED. Each needs a separate product decision; no empty infrastructure is added.

- D08: WKWebView isolated callAsyncJavaScript is the candidate macOS callback adapter; Tauri eval_with_callback does not await Promise results. Synthetic JS input is explicitly classified; trusted/native input remains unqualified.
- D09: The pre-existing lockfile package reorder was retained. The only existing package version replaced is rustls 0.23.44 → 0.23.45 for the confirmed advisory. New crates use the same lockfile and target directory.
- D10: Native receipts use SQLite WAL/FULL, fullfsync on macOS, one owner lock, private Unix files and durable transitions before dispatch. Recovery cancels undispatched work and marks dispatched work outcome_unknown. Restart invalidates retry epochs; the store never replays actions. The current conservative quota retains all receipts and refuses new ones at 4096. Schema 2 adds authorized workspace references and typed durable results. Native rename admission/claim/ACK now integrates receipts with grants. Maintenance and Windows ACLs remain unfinished.
- D11: Android macOS ARM64 bootstrap uses the versioned official CLI URL instead of mutable `latest`. The downloaded bytes match the existing pinned SHA-256 and size; tool versions and platform qualification flags are unchanged.

- D12: The first production mutation is workspace rename, which exercises the operation bridge without implicitly starting a shell. UI commands are closed enums with native one-time claims; Workbench remains authoritative. The native broker reserves before queueing, persists Running before claiming, validates the published result before completion, and marks lost running ACKs outcome_unknown. Native tests of terminal-specific leases and process control are separate gates.

- D13: Pairing status includes only the current helper’s public request ID. The Settings approval must match that ID. Native Codex qualification found two simultaneous helper requests; selecting the first same-name request is unreliable. This public correlator is not a token or evidence of agent identity.

- D14: Agent terminal creation uses native-generated panel/runtime IDs and a one-use spawn ticket. Settings selects one discovered `/bin/zsh` or `/bin/bash` profile per connection; its immutable revision is captured in the grant and rechecked at native spawn and attachment. The caller cannot select another shell through `profileId`. Workbench creates the actual tab and the retained xterm runtime owns its existing binary Channel. Only managed PTYs use nonblocking I/O; ordinary terminals retain their previous behavior. Bash before 4.4 uses a guarded DEBUG preexec hook because it has no PS0. An existing DEBUG trap, functrace or extdebug configuration remains untouched and suppresses automatic prompt readiness on that version; explicit input still requires its existing lease and input sequence.
- D15: Terminal leases and input ACKs remain in RAM. Durable create receipts omit leases; authorized receipt reads hydrate the current lease from the live native owner. Revoke/disconnect detaches the 1 MiB fan-out buffer and releases its global observer slot without killing a human-visible shell. Manual input revokes the write lease before waiting for the native writer lock.
- D16: The native terminal writer checks authenticated helper ancestry using macOS socket peer PID and libproc, rejects unresolved ancestry, and protects recognized foreground agent CLIs. This is a domain operation guard, not an OS sandbox. Agent-created terminals receive their initial lease from the native start ticket. Existing terminals require a bounded, expiring Settings-only claim approval; attachment validates the original profile, process ancestry, drained parser ACKs and generation without restarting the PTY. Native manual input and the Take control button revoke the lease; release cannot kill or type into the terminal.

- D17: Workspace creation starts with a scratch editor and grants only its creator access to the new workspace. Focus must account for every runtime in the selected tab; the present adapter rejects lazy/unqualified runtimes. Clean panel close uses a one-use native commit, retains editor guards, refuses active or human-owned terminals, and creates an empty scratch editor when needed without implicitly spawning a replacement shell. Replay resolves the durable receipt even after the original panel is gone.
- D18: Native stop increments an atomic authorization epoch before any policy or SQLite lock. Each native terminal also carries a connection-lifetime token. Writer admission waits at most 25 ms for transient lock contention, checks atomic revocation during that wait and at every write boundary, and fails closed if unavailable. Deferred receipt settlement and observer cleanup cannot restore old tokens. The private IPC transport serializes requests and monitors EOF during an active storage worker; pipelining is rejected. Main/Settings commands that can wait for policy/storage run in blocking workers, leaving the native stop path independent.

- D19: Each observed PTY retains two 512 KiB rings (normalized output and raw bytes) within the original 1 MiB budget. Raw data is returned only for the explicit diagnostic mode and is base64-encoded. Command reads stop at that command's recorded end cursor. Screen reads query the retained xterm through a bounded, one-use main-only request; the native broker revalidates workspace, scope, connection, epoch, generation and producer sequence before disclosure. They report parser lag and never acknowledge PTY flow control on xterm's behalf.

- D20: Browser origin approvals use the same URL validator as native browser navigation. Grants contain canonical scheme/host/effective-port origins, bounded to sixteen entries; paths, userinfo, wildcard hosts and internal app origins are rejected. Each approved browser connection gets a distinct random profile identity. Visible browser open uses these RAM-only approvals and a one-use durable-operation ticket. Restored automation descriptors preserve profile/generation and refuse creation without a fresh ticket; they never fall back to shared browsing. Popups and downloads remain denied in automated profiles. Network isolation remains none.

- D21: Browser control checks the atomic policy epoch and connection token directly in the native navigation delegate, without the broker, receipt or UI mutex. A trusted main address/history action can transfer that isolated panel to human navigation, permanently invalidating the agent lease. Native creation and the UI ACK revalidate the approved workspace, generation, profile and one-use ticket. Lease values are hydrated from RAM and overwritten to null before receipt storage.

## D22 — native DOM projection and synthetic actions

Production reads and click/fill use only the registered child WKWebView, a retained named WKContentWorld and a constant script with NSDictionary data arguments. Native callbacks are the response channel. No public evaluate, page-world reply handler or generic child IPC is added. A bounded top-level DOM projection omits all frame contents and form values, reports truncation, and retains at most 500 references for one snapshot. Identity binds panel, native generation, navigation, document, snapshot, frame and origin; a later snapshot or URL change expires old references. Actions recheck connected element identity and hit testing. Input mode is `synthetic_dom`; trusted gestures/default keyboard behavior are not inferred. One native DOM operation per browser, three-second dispatch expiry and a per-operation cancellation permit bound late work. Receipts contain result metadata, not input text or live references. A native proof distinguishes a known pre-effect rejection from an unknown outcome.

## D23 — Native composite PNG and private immutable artifacts

`browser.capture_composite` is an explicit Settings grant, independent of DOM read/input. Capture uses only the registered owned child WKWebView and reserves native producer/storage capacity before producing pixels. Native callback revalidates generation, navigation, authority, URL, bounds and zoom. TIFF conversion is bounded and encoded off AppKit into a bounded PNG writer. Artifacts carry source workspace/panel/generation/profile/origin/scope, immutable image geometry and SHA-256. The public helper emits standard MCP image blocks; private base64 transport is excluded from output schemas and structured metadata. Reads recheck current source authority, including human takeover, before disclosure. Recovery never scans or removes arbitrary files.

## D24 — Browser errors and visibility

Error collection uses a fixed, main-frame-only document-start user script in the private WK content world. Only real ErrorEvents are collected; native qualification showed page Promise rejections are unavailable there. Do not install page-world console/network hooks or claim complete coverage. The bounded ring is fetched through the existing correlated native JavaScript-return channel, never generic Tauri IPC. Scope and current owned source are revalidated before disclosure.

Native hidden flags alone are insufficient when logical tab selection has changed but the renderer has not synchronized its native child. Broker publication therefore updates a lock-independent browser selection flag; input and screenshot require it as well as native renderability. Other tabs remain eligible for authorized DOM/text observation. Event-driven hide/layout synchronization continues even if WebKit suspends main-document animation frames.

## D25 — Native navigation completion and cancellation

A native navigation carries its operation's revocable permit to AppKit. Load dispatch checks permit, deadline, source control and pending operation immediately before WK loadRequest. The native observer checks cancellation independently of page or main JavaScript, uses stopLoading, and persists its result through the broker's validated durable transition path. Main remains the domain dispatcher and refreshes its projection, but no longer supplies the final navigation receipt. A native stop is scoped to the still-pending operation and never targets a replacement operation or human-taken page. Cancellation after dispatch reports unknown effects; no automatic retry is performed.

## D26 — Android references and process authority

MCP panel creation persists startMode=manual in the existing Android descriptor. Ordinary user panels keep their first-visit behavior. A manual descriptor does not grant native access, and restoring it never starts a phone. Android panel IDs are canonical UUIDs accepted by the existing view/input router.

Runtime mutations retain the Workbench command claim, native one-use dispatch and durable operation receipt. Native completion owns boot/stop readiness independently of main-renderer response. A per-device authority binds the authenticated connection and one actual generation; multiple panels cannot create independent process ownership. Manager preparation, actor enqueue and boot checkpoints check atomic authority and the operation deadline. Native human process actions revoke authority; a running human phone cannot be acquired by start. A failed boot retains the owned process for explicit Stop. Timeout/cancellation after native dispatch reports uncertain effects and never replays. Force stop remains the explicit native user flow. The same authority will be integrated with the existing global input router in the next increment.

APK storage decisions (2026-09-23): schema 4 distinguishes images and APKs so an older writer refuses unknown format. Keep legacy image JSON, make image geometry absent for APKs. Fixed reservation-derived staging names permit bounded recovery of interrupted publications; no arbitrary directory cleanup. All artifacts share 2 GiB, 1 GiB per owner/project and 64 files (stricter than the proposed 10,000), with existing image subbudgets retained. Active producers/readers/installers pin both bytes and quota. Import grants pin a directory descriptor and retain source classification. Per-file size 512 MiB; import result exposes only ID/hash/size. Native ZIP central-directory bounds precede parser allocation. APK installation remains a separate explicit user approval of that exact copy. ResourceTypes format reference for subsequent package metadata inspection: [AOSP ResourceTypes.h](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/libs/androidfw/include/androidfw/ResourceTypes.h).

## D27 — Package-scoped Android launch and logs

Settings approves exact Android package names separately from device control. Launch is a fixed component intent with optional MAIN/LAUNCHER resolution, never a shell/deep-link API; native actor serialization preserves generation and revoke guards. Intent delivery is not a test verdict. Logcat observes only the selected running main process in the main buffer, bounded before disclosure, with unique UID and PID filters. Shared/system identities are refused. A cursor pages an immutable bounded RAM snapshot; it is not a lossless streaming watermark. Gap status remains unknown and completeness false. Sources: [AOSP ActivityManagerShellCommand](https://android.googlesource.com/platform/frameworks/base/+/183f8d21c8c6/services/core/java/com/android/server/am/ActivityManagerShellCommand.java), [AOSP logcat filtering and formats](https://android.googlesource.com/platform/system/logging/+/refs/heads/main/logcat/logcat.cpp). The native test must prove actual command behavior on the managed guest before these adapters are qualified.

## D28 — Descriptor-based project metadata

External disk reads/listing use the already pinned ProjectDirectory and editor text decoding. The UI's pathname-based directory function cannot enforce the MCP NOFOLLOW boundary during replacement races, so metadata enumeration is implemented on the pinned descriptor. Reuse Rustix 1.1.4 (already locked transitively) as a direct Unix-only dependency for safe directory iteration/statat. `Dir::read_from` opens an independent descriptor and handles readdir errors/lifetimes; symlinks are never followed. Source verified against [Rustix v1.1.4 Dir implementation](https://raw.githubusercontent.com/bytecodealliance/rustix/v1.1.4/src/backend/libc/fs/dir.rs). Listing hashes metadata and directory identity for cursor invalidation, distinct from file-content SHA revisions. Known secret paths, links and special files are omitted; hidden names are not an alternate read route. Core file search and buffer adapters must preserve this boundary.

## D29 — bounded project disk search

Reuse the actual `files/search.rs` regex, glob filter and preview/UTF-16 helpers plus `files/editor.rs` decoding; do not invoke the human search command or its global cancellation counter. The MCP traversal uses `ProjectDirectory` descriptors, rejecting links, hard links, secret components and special files. It intentionally does not load global Git excludes or repository ignore files through pathname APIs. The protocol reports `explicit_patterns_and_secret_paths` and exposes the same include/exclude pattern syntax for narrowing a scan. Queries remain line based; lines beyond 32 KiB are skipped and counted. A result is a collection of per-file observations with exact disk hashes, not an atomic project snapshot. Paging retains these immutable observations for at most 60 seconds and does not silently rerun a changed query. Root replacement, lost scope, session end and global revocation invalidate disclosure. No new dependency or external program is used.

### D34 — MCP editor previews and embedded image reads (native verified)

`lomi_editor_open` accepts the closed optional `presentation` enum (`editor`,
`preview`, `split`; default `editor`). Markdown/SVG share the retained editor
buffer and history. Raster images require `preview` and return separate immutable
preview metadata, never a fictitious editable document. Existing scopes, receipt
key, deadline and layout conflict rules apply. Native pinned reads precede panel
publication. Raster derivatives are first-frame, oriented RGBA PNG, at most 1280
pixels per edge / 3 MiB; original file at most 4 MiB / 16 Mi pixels / 64 MiB decoded.
Unsupported decoders (including AVIF) fail explicitly without changing the file.

A transient, bounded per-panel preview source prevents agent-opened image panels
from falling back to ordinary UI path reads. Agent-opened Markdown carries a
native, main-only asset permit bound to the approved session, project directory,
policy epoch and workspace. Every embedded image read repeats authorization and
NOFOLLOW/secret-path checks; permit IDs alone confer no authority. Permits expire
in 15 minutes, are capped at 64, and budget 64 reads / 32 MiB source bytes / 16 MiB
output per permit. External Markdown images remain explicit links. SVG remains
an image element, with native WebKit isolation qualification required. Normal
human-opened previews retain their existing behavior. No preview bodies enter
MCP receipts. None of these choices qualifies a hard OS sandbox.

### D35 — guarded Git reads (status native verified; further observations in progress)

Existing Git commands inherit PATH/environment/configuration and cannot simply
be exported. A macOS27 ARM64 prototype used the root-owned
`/Library/Developer/CommandLineTools/usr/bin/git` (2.54.0 AppleGit157) under an
explicit per-process sandbox-exec profile. This is a narrow Git read guard, not a
Codex execution sandbox or P7 strict-routing claim. Profile denies network,
process-fork, all further executable paths except the exact Git binary, and file
writes except /dev/null; file contents are readable only in the approved canonical
project and required system-library paths. Reading the root directory itself is
required by this macOS runtime; metadata is not wholly sandboxed. Home/global/system
Git config are replaced by a private environment. Unqualified/missing OS guard
must fail closed. The production guard and status adapter are now implemented;
status passed native49 qualification wwumF2.

Prototype fixture `/var/folders/q6/xvq1c0vj24n0cnh7hrysr4k40000gn/T/lomi-mcp-git-read-6qo32l6_`
contains read.sb/result.json/negative-results.json. Status and ordinary raw diff
worked; configured clean filter was denied fork, fsmonitor was explicitly disabled,
external diff/textconv were disabled, and outside include.path was denied without
returning its fixture secret. Git can return exit0 plus stderr after a blocked
filter; the future adapter must reject such a result, never silently report a
converted comparison as successful. Disable system attributes with the documented
GIT_ATTR_NOSYSTEM setting. Normal Git UI/mutations retain user configuration and
still need explicit exact-operation approval + git.execute when code can run.

Implemented foundation checks: child fchdir on the pinned repository descriptor,
bounded concurrent stdout/stderr drains, timeout/revoke/kill/reap, executable and
root identity, secret-path masking, literal paths and closed full commit IDs.
Production tests include a blocking config FIFO, outside includes and helper
execution denial. Linked worktrees remain unsupported. Config/index/HEAD
revisions for mutations, secondary metadata/alternate race qualification and
existing read-only view integration remain open. Full same-UID hostile filesystem
race isolation is not claimed by this narrow read guard. Primary references:
https://git-scm.com/docs/git , https://git-scm.com/docs/git-config ,
https://git-scm.com/docs/git-diff , https://git-scm.com/docs/git-status ; installed
macOS sandbox-exec usage was checked locally. Only this host/prototype is tested.

### D36 — Git mutation execution foundation (implementation in progress)

All seven mutation flows require git.execute plus the domain write scope and an
exact main-window approval. That scope explicitly trusts repository configuration,
hooks, filters and configured helpers to run with the host account's permissions;
it is not a filesystem sandbox. No mutation executes solely because git.read or a
project grant exists. Preserve normal identity, exact commit message and hooks.
No force push, hard reset, caller-supplied executable/options/environment or
implicit conflict resolution. Stage/unstage is the first vertical mutation flow;
only implemented variants may enter the public enum/catalog.

A preview must bind the actual repository descriptor, effective configuration,
HEAD, index, exact paths/content and normalized target remote/ref before dispatch.
Preview reads must themselves prevent helpers/network/writes. Local/global config
identity needed for execution is separate from public Git read disclosure; never
return raw config, credentials or subprocess stderr through MCP. Repeat the
snapshot before a single dispatch; changes invalidate the exact approval. Shared
native Git mutation_guard serializes Lomi; Git's own locks handle other processes.
It cannot promise isolation from arbitrary code the user has explicitly trusted
or same-account filesystem writes after dispatch; post-dispatch uncertainty must
remain explicit and never trigger replay/automatic rollback.

Reuse the existing qualified Android installer's process-group owner as a shared
native process primitive because Git hooks introduce a second real consumer. It
uses waitid(WNOWAIT) to retain the leader PID while checking descendants before
reaping, preventing PGID reuse during cancellation. Keep the Windows Job owner
and original Android installer semantics; qualify shared regressions on this host.
Git needs separate bounded stdout/stderr drains and deadlines; detached code is
not contained without the optional P7 OS sandbox. An unconfirmed child stop must
retain ownership/locks and block conflicting work instead of forgetting a process.
