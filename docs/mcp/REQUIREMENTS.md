# MCP requirements traceability

Current scope override (user instruction, 2026-09-23): further theme/plugin MCP
work, application close/restart/update MCP tools, and distribution/installers/
releases are EXCLUDED_BY_USER. Earlier rows and notes retain their historical
status; unfinished items in those domains are no longer acceptance requirements.
Chat AI, Android management, remaining browser/terminal/layout integration and
functional/safety/performance tests remain required. Publish completed modules
as ordinary commits to origin/main, with no tags or releases.

The source contracts remain normative. Native57 and earlier evidence predates the multiple-project binding refactor; native58 regression is tracked in IMPLEMENTATION-STATUS.md. A tested initial slice does not qualify an unfinished wider domain. A TODO contract is not exposed as a tool. Before implementation, each row must link concrete input/output schemas, read/mutation scopes, target resolution, readiness, limits, retry/cancellation, typed errors, adapter and executed tests. Shared requirements come from chapters 02, 03, 08, 11, 14 and audit A01–A13.

| Tool                         | Source / stage  | Contract and adapter                                                                                                                 | Evidence                                                                                                                | State                                                            |
| ---------------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `lomi_operation_get`         | 03 / P1–P5      | Protocol control.rs; broker.rs / broker/operations.rs                                                                                | Core UDS, process schemas, native Settings/Codex                                                                        | IN_PROGRESS                                                      |
| `lomi_status`                | 03 / P1–P5      | Protocol control.rs; broker.rs / broker/operations.rs                                                                                | Core UDS, process schemas, native Settings/Codex                                                                        | IN_PROGRESS                                                      |
| `lomi_connect`               | 03 / P1–P5      | Protocol control.rs; broker.rs / broker/operations.rs                                                                                | Core UDS, process schemas, native Settings/Codex                                                                        | IN_PROGRESS                                                      |
| `lomi_workspace_list`        | 03 / P1–P5      | Protocol control.rs; broker.rs / broker/operations.rs                                                                                | Core UDS, process schemas, native Settings/Codex                                                                        | IN_PROGRESS                                                      |
| `lomi_workspace_create`      | 03 / P1–P4/P5   | Protocol control.rs; broker/panels.rs, operations.rs and broker.rs; Workbench bridge                                                 | Native app/stdio and scoped core tests; full domain variants pending                                                    | IN_PROGRESS                                                      |
| `lomi_workspace_update`      | 03 / 07 / P5.3  | Closed rename/select/close DTOs; per-action scopes, exact revision and durable ancestor receipts                                     | UDS/native56 YWPPVZ; six real runtime close cycles, dirty-buffer guards                                                 | IN_PROGRESS — Android/chat/plugin descendants pending            |
| `lomi_workspace_close`       | 03 / 07 / P5.3  | Provided by lomi_workspace_update action=close; no separate alias exposed                                                            | See workspace close contract and native56 YWPPVZ                                                                        | IN_PROGRESS — initial resource kinds VERIFIED                    |
| `lomi_panel_list`            | 03 / P1–P4/P5   | Protocol control.rs; broker/panels.rs, operations.rs and broker.rs; Workbench bridge                                                 | Native app/stdio and scoped core tests; full domain variants pending                                                    | IN_PROGRESS                                                      |
| `lomi_panel_focus`           | 03 / P1–P4/P5   | Protocol control.rs; broker/panels.rs, operations.rs and broker.rs; Workbench bridge                                                 | Native app/stdio and scoped core tests; full domain variants pending                                                    | IN_PROGRESS                                                      |
| `lomi_panel_control`         | 03 / P1–P4/P5   | Closed claim/release DTO; bounded Settings-only approval and native existing-PTY attachment                                          | Native ordinary user terminal retains PID/output; deny, reclaim, release and manual takeover                            | IN_PROGRESS                                                      |
| `lomi_panel_move`            | 03 / 07 / P5.3  | Reorder/dock/move and same-project cross-workspace tab transfer; both grants, exact resource identities                              | Model/UDS/wire and native56 YWPPVZ; same PTY/form/buffer/Undo; original run retry preserved                             | IN_PROGRESS — Android/chat/plugin transfer pending               |
| `lomi_panel_close`           | 03 / P1–P4/P5   | Protocol control.rs; broker/panels.rs, operations.rs and broker.rs; Workbench bridge                                                 | Native app/stdio and scoped core tests; full domain variants pending                                                    | IN_PROGRESS                                                      |
| `lomi_operation_cancel`      | 03 / P1–P5      | Protocol control.rs; broker.rs / broker/operations.rs                                                                                | Core UDS, process schemas, native Settings/Codex                                                                        | IN_PROGRESS                                                      |
| `lomi_events_read`           | 03 / P1–P4/P5   | Protocol control.rs; broker/panels.rs, operations.rs and broker.rs; Workbench bridge                                                 | Native app/stdio and scoped core tests; full domain variants pending                                                    | IN_PROGRESS                                                      |
| `lomi_artifact_read`         | 03 / P1–P4/P5   | Immutable PNG source classification and native authority; protocol artifact.rs, broker/artifacts.rs and private Store                | Native ggnnxP: exact bytes, geometry, owner/takeover; quota and recovery tests                                          | IN_PROGRESS                                                      |
| `lomi_artifact_import`       | 03 / 06 / P4    | ArtifactImportInput/ArtifactImported; files.read + artifact.import; immutable bounded APK staging via broker/imports.rs              | Native55 Im0Xl9; hash/identity/quota/ownership/retry tests                                                              | IN_PROGRESS — broader artifact types pending                     |
| `lomi_diagnostics`           | 03 / P1–P5      | Protocol control.rs; broker.rs / broker/operations.rs                                                                                | Core UDS, process schemas, native Settings/Codex                                                                        | IN_PROGRESS                                                      |
| `lomi_terminal_create`       | 05 / P2         | Closed control.rs DTO; broker/terminals.rs, terminal_runs.rs, terminal_input.rs; native terminal.rs and retained terminal-runtime.ts | Native app/stdio, input, targeted interrupt/cancel and core lease tests                                                 | IN_PROGRESS                                                      |
| `lomi_terminal_run`          | 05 / P2         | Closed control.rs DTO; broker/terminals.rs, terminal_runs.rs, terminal_input.rs; native terminal.rs and retained terminal-runtime.ts | Native app/stdio, input, targeted interrupt/cancel and core lease tests                                                 | IN_PROGRESS                                                      |
| `lomi_terminal_read`         | 05 / P2         | Closed control.rs DTO; broker/terminals.rs, terminal_runs.rs, terminal_input.rs; native terminal.rs and retained terminal-runtime.ts | Native app/stdio, input, targeted interrupt/cancel and core lease tests                                                 | IN_PROGRESS                                                      |
| `lomi_terminal_input`        | 05 / P2         | Closed control.rs DTO; broker/terminals.rs, terminal_runs.rs, terminal_input.rs; native terminal.rs and retained terminal-runtime.ts | Native app/stdio, input, targeted interrupt/cancel and core lease tests                                                 | IN_PROGRESS                                                      |
| `lomi_terminal_interrupt`    | 05 / P2         | Closed control.rs DTO; broker/terminals.rs, terminal_runs.rs, terminal_input.rs; native terminal.rs and retained terminal-runtime.ts | Native app/stdio, input, targeted interrupt/cancel and core lease tests                                                 | IN_PROGRESS                                                      |
| `lomi_browser_open`          | 04 / P2–P3/P5.7 | Closed DTO, broker/browsers.rs, atomic BrowserControl, native browser.rs and retained Workbench                                      | Native visible WKWebView, exact-origin approval, deduplication and pre-request redirect denial; hidden creation pending | IN_PROGRESS                                                      |
| `lomi_browser_navigate`      | 04 / P2–P3/P5.7 | Owned generation, lease, exact origins; native commit/load                                                                           | Native navigate, deduplication, denied redirect                                                                         | IN_PROGRESS                                                      |
| `lomi_browser_snapshot`      | 04 / P2–P3/P5.7 | Bounded isolated-world top-level semantic DOM, private values omitted                                                                | Native React/Unicode, stale refs and scope denial                                                                       | IN_PROGRESS                                                      |
| `lomi_browser_click`         | 04 / P2–P3/P5.7 | Snapshot-bound synthetic DOM click, durable retry                                                                                    | Native form, no duplicate click, replaced node denied                                                                   | IN_PROGRESS                                                      |
| `lomi_browser_fill`          | 04 / P2–P3/P5.7 | Text/textarea/select/contenteditable, value length only                                                                              | Native React Unicode, select/contenteditable, expired dispatch                                                          | IN_PROGRESS                                                      |
| `lomi_browser_key`           | 04 / P2–P3/P5.7 | Focused top-level synthetic key events; no native default action                                                                     | Native focused listener, wrong-focus denial, retry                                                                      | IN_PROGRESS                                                      |
| `lomi_browser_scroll`        | 04 / P2–P3/P5.7 | Top-level viewport CSS deltas, snapshot/lease                                                                                        | Native scroll position and retry                                                                                        | IN_PROGRESS                                                      |
| `lomi_browser_wait`          | 04 / P2–P3/P5.7 | Bounded DOM text/element/URL/load polling                                                                                            | Native async result, timeout, fresh snapshot                                                                            | IN_PROGRESS                                                      |
| `lomi_browser_screenshot`    | 04 / P2–P3/P5.7 | Explicit browser.capture_composite; native WK snapshot, bounded producer, PNG and immutable artifact                                 | Native ggnnxP and visual inspection; further frame/geometry/fault matrix pending                                        | IN_PROGRESS                                                      |
| `lomi_browser_logs`          | 04 / P2–P3/P5.7 | browser.read; document-start main-frame errors, 64 entries, scoped cursor; native agent_dom and broker/browser_logs                  | Native dmOW6D: error capture, private-world separation, overflow/pagination, foreign workspace and takeover             | IN_PROGRESS                                                      |
| `lomi_android_list`          | 03/06/08, P4    | Selected managed-device metadata; no runtime startup                                                                                 | Native `1A9KWF/android-list.json`, UDS device/scope/revoke guards                                                       | VERIFIED (metadata only)                                         |
| `lomi_android_open`          | 03/06 / P4      | Selected device + workspace/panel.create/android.read; strict revision/retry; actual retained manual-start panel                     | UiAction::CreateAndroid / Workbench / Android runtime                                                                   | VERIFIED: cplPJo; no boot, one panel on retry, persistence/UI    |
| `lomi_android_start`         | 03/06 / P4      | Selected device/panel, android.control + read, revision/retry; ready requires guest/RPC/ADB/IME; 180s bound                          | Manager/actor with revocable queued dispatch and device authority; native completion                                    | VERIFIED: H8Q7b3; native readiness, retry; queue tests PASS      |
| `lomi_android_stop`          | 03/06 / P4      | Selected device/panel/exact owned generation; android.control + read; revision/retry; confirmed exit; no implicit force              | Same managed actor; native generation/authority checks, retained failed stop                                            | VERIFIED: H8Q7b3; native confirmed exit, stale generation, retry |
| `lomi_android_snapshot`      | 06 / P4         | AndroidSnapshotInput; android.observe/control/read; owned generation, bounded hierarchy; broker/android_capture.rs                   | Native55 Im0Xl9 and UDS scope/identity tests                                                                            | IN_PROGRESS — broader P4/P6 matrix pending                       |
| `lomi_android_screenshot`    | 06 / P4         | AndroidScreenshotInput/Artifact; android.capture/control/read; bounded native PNG, source authority                                  | Native55 Im0Xl9; artifact reread, geometry and generation tests                                                         | IN_PROGRESS — broader P4/P6 matrix pending                       |
| `lomi_android_input`         | 06 / P4         | AndroidInput; android.interact/control/read; RAM lease, sequences, native focus and human takeover                                   | Native55 Im0Xl9 input/rotation PASS                                                                                     | IN_PROGRESS — two-view/host matrix pending                       |
| `lomi_android_install_apk`   | 06 / P4         | AndroidInstallInput; android.install/control/read + files.read; immutable APK and exact Settings approval                            | Native55 Im0Xl9; U0qTez signature mismatch preserves app                                                                | IN_PROGRESS — fault/guest-space matrix pending                   |
| `lomi_android_launch`        | 06 / P4         | AndroidLaunchInput; android.launch/control/read; selected packages and explicit launch receipt                                       | Native55 Im0Xl9 and scoped UDS tests                                                                                    | IN_PROGRESS — broader P4/P6 matrix pending                       |
| `lomi_android_logcat`        | 06 / P4         | AndroidLogcatInput; android.logs/control/read; selected package, PID/UID main buffer, bounded cursor                                 | Native55 Im0Xl9 and scoped UDS tests                                                                                    | IN_PROGRESS — broader P4/P6 matrix pending                       |
| `lomi_android_setup_plan`    | 06 / P4/P5.5    | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_android_setup_apply`   | 06 / P4/P5.5    | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_android_device_manage` | 06 / P4/P5.5    | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_project_open`          | 07 / P5.3       | ProjectOpenInput/ProjectOpened; project.open + workspace.write/panel.create; Settings root approval; project_open.rs                 | UDS44/UI4/wire5; native58 DJziSo and full58 H2IkCx PASS with normal cleanup                                             | IN_PROGRESS                                                      |
| `lomi_project_close`         | 07 / P5.3       | ProjectCloseInput/ProjectClosure; project.close + all workspace grants + ancestor scopes; broker/project_close.rs and shared bridge  | UDS42/UI3/model7/wire5; focused native57 5ranoM; full regression pending                                                | IN_PROGRESS — initial resource kinds VERIFIED on macOS ARM64     |
| `lomi_editor_open`           | 07 / P5.1       | EditorOpenInput/EditorOpened or EditorPreviewed; files.read + editor.read + panel.create/focus; shared runtime                       | Full native57 88IOPg, shared dirty documents and text/image/Markdown previews                                           | IN_PROGRESS — broader P5/P6 matrix pending                       |
| `lomi_editor_read`           | 07 / P5.1       | EditorReadInput/EditorText; editor.read + files.read; shared buffer and canonical source proof                                       | Full native57 88IOPg, core/UDS/UI                                                                                       | IN_PROGRESS — broader P5/P6 matrix pending                       |
| `lomi_editor_apply_edits`    | 07 / P5.1       | EditorEditsInput/EditorEdited; editor.write/read + files.read; one revision-bound UTF-16 undo transaction                            | Full native57 88IOPg, core/UDS/model/UI                                                                                 | IN_PROGRESS — broader P5/P6 matrix pending                       |
| `lomi_editor_save`           | 07 / P5.1       | EditorSaveInput/EditorSaved; files.mutate/read + editor.write/read; pinned atomic save and native receipt                            | Full native57 88IOPg; real full-disk failed Save retains RAM                                                            | IN_PROGRESS — wider encoding/fault matrix pending                |
| `lomi_files_list`            | 07 / P5.1       | FilesListInput; files.read; descriptor-relative directory, bounded revision-bound pages                                              | Full native57 88IOPg, core/UDS root and cursor tests                                                                    | IN_PROGRESS — broader P5/P6 matrix pending                       |
| `lomi_files_read`            | 07 / P5.1       | FilesReadInput/FileText; files.read; pinned file, bounded decoded disk text and exact revision                                       | Full native57 88IOPg, core/UDS                                                                                          | IN_PROGRESS — wider encoding/fault matrix pending                |
| `lomi_files_search`          | 07 / P5.1       | FilesSearchInput; files.read; bounded directory traversal, secret policy and owner-scoped cursors                                    | Full native57 88IOPg, core/UDS                                                                                          | IN_PROGRESS — broader P5/P6 matrix pending                       |
| `lomi_files_mutate`          | 07 / P5.1       | FilesMutateInput/FilesMutated; files.mutate/read plus create/rename/trash; native atomic writes and shared aliases                   | Full native57 88IOPg; create/move/rename/Trash; real ENOSPC recovery                                                    | IN_PROGRESS — lifecycle/fault matrix pending                     |
| `lomi_git_status`            | 07 / P5.2       | GitStatusInput; git.read + files.read; pinned repository, redacted bounded revision-bound status                                     | Full native57 88IOPg, core/UDS                                                                                          | IN_PROGRESS — wider Git matrix pending                           |
| `lomi_git_diff`              | 07 / P5.2       | GitDiffInput; git.read + files.read; explicit repository/path/comparison and bounded revision                                        | Full native57 88IOPg, core/UDS                                                                                          | IN_PROGRESS — wider Git matrix pending                           |
| `lomi_git_history`           | 07 / P5.2       | GitHistoryInput; git.read + files.read; pinned start commit and bounded history pages                                                | Full native57 88IOPg, core/UDS                                                                                          | IN_PROGRESS — wider Git matrix pending                           |
| `lomi_git_mutate`            | 07 / P5.2       | GitMutateInput/GitMutated; seven typed operations, operation scopes and exact UI/native approval; git_mutate.rs                      | Full native57 88IOPg: stage/unstage/commit/fetch/push/discard/pull incl rebase and conflict                             | IN_PROGRESS — wider Git matrix pending                           |
| `lomi_settings_read`         | 07 / P5.4       | SettingsReadInput/SettingsSnapshot, global settings.read, broker/settings_read.rs and retained provider bridge                       | core57/UDS46/protocol9/wire5/UI2; native60 qqYreQ snapshots/recovery/cleanup PASS                                       | VERIFIED                                                         |
| `lomi_settings_update`       | 07 / P5.4       | SettingsUpdateInput/SettingsUpdated; settings.read+write, exact Settings decision; closed editor/terminal/shortcut patches           | Editor0GkHVO/fullfCqMdY, terminalu6PGm8 native PASS; shortcuts GJqZd0 native12 + terminal9/editor6 PASS; themes pending | IN_PROGRESS                                                      |
| `lomi_settings_open`         | 07 / P5.4       | SettingsOpenInput/SettingsOpened; settings.open; broker/settings.rs, native settings_window shared request                           | core57/UDS45/protocol9, wire5/UI1; native59 KzW0fU nine pages and cleanup PASS                                          | VERIFIED                                                         |
| `lomi_plugins_list`          | 07 / P5.4       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_plugin_open`           | 07 / P5.4       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_chat_list`             | 07 / P5.6       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_chat_open`             | 07 / P5.6       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_chat_read`             | 07 / P5.6       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_chat_draft`            | 07 / P5.6       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_chat_send`             | 07 / P5.6       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_chat_stop`             | 07 / P5.6       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_chat_export`           | 07 / P5.6       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_app_prepare_close`     | 07 / P5.7       | TODO (not exposed)                                                                                                                   | TODO                                                                                                                    | TODO                                                             |
| `lomi_git_commit`            | 07 / P5.2       | GitCommitInput; git.read + files.read; bounded full commit details                                                                   | Full native57 88IOPg, core/UDS                                                                                          | IN_PROGRESS — wider Git matrix pending                           |
| `lomi_git_remotes`           | 07 / P5.2       | GitRemotesInput; git.read + files.read; credential-redacted remote metadata                                                          | Full native57 88IOPg, core/UDS                                                                                          | IN_PROGRESS — wider Git matrix pending                           |
| `lomi_git_open`              | 07 / P5.2       | GitOpenInput/GitOpened; git.read + files.read; explicit diff/commit shared views; panel.create/focus                                 | Full native57 88IOPg, core/UDS                                                                                          | IN_PROGRESS — wider Git matrix pending                           |

Chapter 07 additional contracts: theme import, plugin install/enable/update/uninstall, artifact export and update/restart preparation are TODO protected UI flows; no generic dispatch is permitted.

### Native screenshot and artifact increment

`lomi_browser_screenshot` / `BrowserScreenshotInput` → `Data::Artifact` + standard MCP PNG; native `agent_capture.rs`, core `broker/artifacts.rs` / `artifacts.rs`; explicit `browser.capture_composite`; stable workspace/panel/generation/navigation, native renderability, 64–1600 width and 16 KiB–3 MiB byte limit. Read-only capture has no mutation retry receipt. Timeout does not free the native callback producer permit early. `lomi_artifact_read` / `ArtifactReadInput` rechecks the original immutable source classification, current scope/profile/generation/origin and lease authority, workspace, expiry, file identity and hash. Native fixture `ggnnxP` and artifact store tests verify the implemented paths; the broader P3/P6 fault and platform matrices remain IN_PROGRESS.

### Browser log and focus increment

`lomi_browser_logs`: BrowserLogsInput → BrowserLogs; browser.read; existing owned workspace/panel/generation, top-level approved origin, 1–64 entries, 64-entry per-document retention and 256 UTF-16-unit messages. Read-only, no mutation receipt. Cursor binds generation/navigation/sequence and expires on navigation. Dispatch/callback/disclosure require current source authority; timeout retains native callback capacity. CURSOR_EXPIRED, SCOPE_DENIED, CONTROL_REVOKED and target/generation errors are explicit. No console/network/Promise rejection coverage is claimed. Native `dmOW6D`/`TWNWAc`, wire schemas and core scope-denial test are executed evidence; advanced P5 logs remain pending.

`lomi_panel_focus` adds optional browserGeneration, required to match a browser target. It reuses only an owned started native browser; mixed tabs reject lazy/unqualified peers. Workbench retains authority and existing layout revision/retry/cancellation semantics. Native `TWNWAc` verifies preserved form, stale-generation rejection and exactly-once receipt, plus capture rejection while another tab is selected.

### Owned browser close contract (implementation under verification)

`lomi_panel_close` requires the explicit browserGeneration, current domain revision and durable retry identity for browser targets, plus panel.close/panel.create. Only the requesting session's still-authorized native browser can close; human takeover denies it. The native close callback checks generation/authority and a bounded AppKit deadline, revokes its control, closes the exact Tauri child and removes its input monitor. Workbench then removes the descriptor, selecting an empty scratch file when needed. Late/ambiguous completion is outcome_unknown and never replays the close. Artifact reads lose their source authority. Native qualification for close/retry/human denial is pending.

Owned browser close is now VERIFIED on the recorded macOS ARM64 host: exact browserGeneration + panel.close/panel.create, currently owned live browser only, retained domain close guard, one native AppKit close and durable deduplication; human takeover denies close. Native evidence `MzdU9b/browser-close.json`. Contenteditable omission and stale page-state rejection are verified in the same native run and five browser UI tests. Mixed unqualified runtimes, dirty/busy close and complete workspace lifecycle remain unfinished.

### Workspace selection contract

`lomi_workspace_update` accepts exactly one of the existing rename object (`workspaceId`, `name`, `expectedRevision`, `retryEpoch`, `requestKey`) or a selection object (`action: "select"`, `workspaceId`, `expectedRevision`, `retryEpoch`, `requestKey`). Mixed/unknown arguments are rejected. Workspace discovery includes its saved `activePanelId`. Selection resolves that exact target from the current revision, requires workspace.write + panel.focus at admission and claim, and reuses the same native-generation and lazy-start guards as panel focus. The domain rechecks saved active tab/panel and switches through Workbench without recreating runtimes. Result is the scoped panel operation receipt; retry/cancellation and 30-second UI dispatch budget match panel focus. No fresh shell/device/browser starts are implicit. Missing saved target gives UI_NOT_READY; a lazy terminal gives SCOPE_DENIED; unqualified kinds give UNSUPPORTED_CAPABILITY; changed revision gives REVISION_CONFLICT. Core/UDS tests passed (23 core + 11 UDS + 6 protocol); native selection qualification passed in `oNZ5Gg/workspace-selection.json`.

Hidden browser creation contract (VERIFIED on recorded host, `wfHZKO/browser-hidden.json`): visible=false preserves the domain's active project/workspace/tab. A one-use creation ticket binds visibility as well as URL, generation and profile. A native blank child is hidden before external navigation; the initial hidden layout is 800×600, only for read preparation. It uses the existing isolated native view and permissions, without a new browser process. DOM snapshot/wait remain available; interaction and capture require explicit exact-generation panel focus and actual native visibility. The view and document are retained when revealed. No hidden screenshot readiness is claimed. Cancellation after ticket consumption revokes that browser's authority before any later navigation can pass policy.

### Android inventory contract

`lomi_android_list` input is `{workspaceId}` only. Output is a scoped AndroidDevices result: devicesRevision, hostQualified and at most sixteen devices with deviceId/name/generation/phase/processAlive/display. Requires android.read and explicit device IDs approved in Settings. Native Settings verifies those IDs still exist; the broker binds them to the connection and project/workspaces, and checks scope at admission and disclosure. A typed native request carries the atomic connection/policy permit and a five-second cooperative deadline, checked before and after native metadata work. Runtime status reuses manager.statuses, including recovered process records; it does not boot Java/emulator/ADB. IDs, byte lengths and duplicates are bounded and any unexpected producer device fails closed. No cursor is required because the grant itself is limited to sixteen devices. The read has no receipt, side effect or automatic retry. Missing scope/foreign workspace/revoke/unqualified adapter/storage failure use the closed common errors. The test fixture uses the already accepted isolated directory; no consent or ownership is inferred from environment variables in production.

Android lifecycle contract: `open` returns a durable `android_panel` result (workspaceId/panelId/deviceId) only after Workbench publishes the actual descriptor. The descriptor persists startMode=manual and is never an authorization credential. `start` and `stop` return durable `android_runtime` results (workspaceId/deviceId/generation/ready/stopped) only from native manager completion; renderer ACK cannot assert readiness. Start cannot acquire an already running human generation. Device authority is shared across panels, invalidated by native human actions and connection/policy revocation. Stop never targets a replacement generation and retry after successful stop still resolves its receipt. Input, screenshots, XML, APK and logcat are separate outstanding contracts. Start/stop native proof: H8Q7b3, android-runtime.json; host cleanup confirmed.

| Increment                 | Contract / authorization                                                                                                                                                 | Adapter / limits / cancellation                                                                                                                                                                                                                                                                                    | Evidence                                                                                 |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| P4 `lomi_artifact_import` | `ArtifactImportInput`: workspace, relativePath, android_apk, expected size/hash, domain revision, retry epoch/key; `files.read` + `artifact.import` approved in Settings | Pinned `ProjectDirectory` + `StagedCopy`, native bounded APK inspection, durable native-only receipt; 180 s, 512 MiB per file, shared reservations/leases; cancellation checked per 64 KiB and parser chunk; errors ScopeDenied/RevisionConflict/ArtifactInvalid/ArtifactTooLarge/ResourceExhausted/ControlRevoked | 17 UDS tests incl import lifecycle; real native `lZY77Q/apk-import.json` PASS            |
| P4 APK metadata reread    | Original owner/project/workspace and `files.read`; no binary response or local path                                                                                      | Source classification retained, directory identity rechecked; 24 h expiry, lease-aware cleanup                                                                                                                                                                                                                     | Native cross-workspace denial and unchanged metadata after source rewrite; storage tests |

| P4 `lomi_android_install_apk` | Workspace/panel/device/generation, artifact ID/hash and durable retry key; `android.read`/`android.control`/`android.install` + original `files.read` | Settings-only 120 s one-use decision; private file lease/hash recheck; existing generation-checked actor + guarded ADB streaming; 180 s operation, 2 running/8 total, one per device; no uninstall fallback | Native `Dw6Bc8/apk-install.json`, approval PNG; core approval/cancel/stale/copy-mutation tests and real smart-socket revoke test PASS |

| Increment                | Contract / authorization                                                                                                                                               | Adapter / limits / cancellation                                                                                                                                                                                                                                                                                                                                                                                                     | Evidence                                                                                                   |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| P4 `lomi_android_launch` | AndroidLaunchInput workspace/panel/device/generation/package/optional activity/revision/retry epoch/key; android.read/control/launch + exact Settings-approved package | One-use native claim; existing manager/actor serializes fixed MAIN/LAUNCHER resolution and `am start -W --user 0`; strict package/Java class grammar and quoted guest arguments; 30 s operation / 12 s guest work; uncertain effects never replayed                                                                                                                                                                                 | UDS deny/forgery/retry/conflict/cancel tests PASS; native 1pGhEe/android-apps.json PASS                    |
| P4 `lomi_android_logcat` | AndroidLogcatInput workspace/panel/device/generation/package/minPriority/limit/cursor; android.read/control/logs + exact package                                       | Existing private Guest; unique app UID + current main PID, identity checked before/after; main buffer, recent 256 lines, 64 KiB total, 2048 bytes/line, 1–64 lines/page, 8 s producer deadline, shared global 2/per-device 1 observation; RAM snapshots max 8, opaque cursor TTL 60 s bound to owner/control/target/filter. Incomplete/unknown gaps explicit; expired cursor, unsupported/shared UID, revoked/stale or scope errors | Core cursor/scope/revoke and native UID/UTF-8 budget unit tests PASS; native 1pGhEe/android-apps.json PASS |

### P5.1 disk reads, listing and search contracts

`lomi_files_read`: workspaceId, project-relative path, UTF-16 start offset (default 0), maxChars 2–8192 (default 4096), optional expectedDiskRevision (required beyond the first page). Output explicitly says source=disk and includes path, content slice, exact SHA-256 disk revision, encoding, line-ending classification, total UTF-16 length, next offset and truncation. `files.read` is separately approved in Settings; the pinned root, NOFOLLOW component walk, single-link regular-file and secret-path policy applies. Native decoding reuses the editor's UTF-8/BOM/UTF-16 implementation. No replacement decoding or implicit normalization. File cap 4 MiB, 2 concurrent producers and a 5 s cooperative deadline; read/check/hash happens outside the broker lock, with authority and identity rechecked before disclosure. No buffer access or write is implicit. Invalid boundaries/limits, changed revision, unsupported encoding, secret/link/foreign target, cancellation and quota have closed errors. No mutation receipt or automatic retry.

`lomi_files_list`: workspaceId, relative directory (empty=root), page limit 1–200 and opaque cursor. Directory traversal is descriptor-relative and denies links and secret components. Output contains bounded relative names, entry kinds and byte lengths, never absolute paths. A cursor binds owner/project/workspace/directory and the observed directory revision; changes expire it. A bounded one-directory enumeration does not imply a recursive search. Directory/file search and mutations remain separate following increments, with existing service reuse and guards.

Disk read evidence: core pinned-source cancellation/mutation tests and UDS scope/secret/path/paging/revision/revoke tests PASS; native UTF-8/UTF-16/BOM/line-ending/surrogate-boundary decoder test PASS. Production adapter is `src-tauri/src/files/agent.rs`, reusing `files::editor::decode`; transport adapter is `lomi-control-core/src/broker/files.rs`. The native fixture asserts real disk contents, CRLF, Unicode continuation, revision conflict, secret-path and foreign-workspace denial. Native `xf2Ymh/files-read.json` PASS.

Directory listing contract is implemented in protocol/files.rs, broker/files.rs and project_files.rs, with files.read scope. Enumeration: 10,000 entries/1 MiB metadata, two shared file producers/5 s cooperative deadline. Page: 1–200 entries and 48 KiB. Cursor: opaque per-connection token, workspace/path/revision-bound, retained in the shared maximum-64 discovery cursor map; eviction or directory changes produce CURSOR_EXPIRED. Output explicitly filtered=true, only approved regular files/directories, relative paths and decimal byte lengths. Revoke/root replacement and descriptor identity are checked before disclosure. Unit and UDS directory/filter/change/cursor tests PASS; native run pending.

`lomi_files_search` contract: FilesSearchInput `{workspaceId, relativeDirectory="", query:{text,caseSensitive,wholeWord,regex,include,exclude},limit=100,cursor}`; `files.read` required. Literal/regex query 1–1024 bytes; patterns 4096 bytes each, same glob parser as UI. Returns FileSearchMatch with relative path, exact disk SHA-256, one-based line/UTF-16 column/length, bounded preview and previewStartUtf16. Result declares per_file_snapshot consistency, explicit_patterns_and_secret_paths filtering, limited/skipped/filesScanned and nextCursor. No implicit editor-buffer reads, global ignore files, symlinks, hard links or special files. 4 MiB/file, 32 MiB total input, 10,000 visited entries, depth 32, 256 matches/128 KiB retained, 200 matches/48 KiB per page, 15-second cooperative deadline. Lines over 32 KiB count as skipped. Deadline/revoke errors disclose no partial results. Cursors bind owner/workspace/directory/query/pinned root/policy, TTL 60 seconds, at most eight snapshots across the broker. Disk changes do not rewrite the immutable cached observations; each result carries its observed revision. Scope, root identity and authority are rechecked on every page; revocation drops caches. Native matching shares `files/search.rs` functions; search cancellation does not touch the human search generation. Status IN_PROGRESS pending checks and native proof.

Editor buffer read contract (native verified): `lomi_editor_read` targets workspaceId/panelId/relativePath with optional documentId and expectedBufferRevision; paging after offset zero requires both identities. Requires both editor.read and files.read, a current file panel and a safe regular source in the pinned project. The retained shared EditorDocument supplies its document UUID, monotonic buffer revision, disk revision, exact dirty state, conflict, encoding and underlying line-ending policy. Content is the CodeMirror LF-normalized buffer with explicit source=buffer and UTF-16 positions; it is never substituted with disk text. Start/max obey scalar boundaries (2–8192 units/page). The main-only typed request/reply binds a random request ID and UI epoch with a two-second revocable wait. Validate scope, root, path, panel, document/revision and bounded response again before disclosure. Track the actual canonical path of each disk load so a refresh through a secret alias cannot become a buffer-read bypass. Opening and saving remain separate guarded operations; this read never creates panels or lazily reads disk.

Editor mutation contract (native verified): `lomi_editor_apply_edits` takes workspaceId/panelId/relativePath/documentId, expectedBufferRevision, expectedDiskRevision, expectedRevision (layout), retryEpoch/requestKey and 1–64 ordered edits `{fromUtf16,toUtf16,insert}`. All offsets refer to the same LF-normalized CodeMirror buffer, half-open, nonoverlapping, without ambiguous coincident insertions or split surrogate pairs. Insertions use LF, valid Unicode and at most 64 KiB aggregate UTF-8. Requires editor.write + editor.read + files.read; does not grant disk writes. Resolve the existing loaded shared document; check all ranges/revisions/source provenance before one CodeMirror transaction isolated in undo history. No range is applied after a validation failure. Result is bounded document/old-and-new-revision metadata, not copied buffer text. Reserve durable receipt/dedup key before dispatch; never replay a RAM mutation after uncertain ACK/restart. Claim and completion recheck granted workspace/panel/path/scopes. A finite dispatch deadline prevents stale renderer queues from applying edits. `lomi_editor_save` remains a separate files.mutate operation with both buffer and disk revisions and descriptor-safe native atomic writing; no force overwrite.

Editor open contract (next increment): `lomi_editor_open` takes workspaceId/relativePath, expected layout revision and retryEpoch/requestKey. Requires files.read + editor.read + panel.create + panel.focus. Select the approved workspace and reuse `openFileTab`, which reveals the existing matching panel or creates one. Prepare text through a native one-use receipt-bound read of the pinned file (4 MiB disk cap, supported editor encoding, bounded internal transfer). Stage this data into the existing lazy editor service before publishing the domain so component mounting cannot fall back to a different pathname read. Preserve an already loaded shared document, dirty state and undo; verify its actual source before reporting success. Return only panel/document/revision/dirty metadata. Retry retrieves the original receipt, never opens a second panel. Partial UI failure becomes outcome_unknown when publication might already have happened. Image-only previews require a separate bounded adapter/qualification; no false claim that decoding a binary file opens a text editor successfully.

P5.1 save contract (implementation in progress): `lomi_editor_save` targets the exact workspace/panel/relative path/document UUID, expected buffer and disk revisions, expected layout revision, retry epoch and request key. Requires files.read + editor.read + editor.write + separately granted files.mutate. The shared document supplies one captured text state; no force overwrite or arbitrary path/content input is exposed to MCP. Native code validates the grant and one-use operation, uses the existing encoding rules and a descriptor-relative atomic replacement shared with ordinary editor saves. Output binds saved buffer revision and resulting disk SHA; later user edits remain dirty. Any failure after replacement is an uncertain effect, never retried as new work. Local provider/editor fixtures must cover Unicode/encoding/endings, stale disk/buffer, parent/leaf replacement links, permissions, cancellation before rename, exact retry and undo retained after save. Not yet exposed.

Next file-mutation increment contract: `lomi_files_mutate` starts with a closed `create` variant for an empty regular file or directory inside the same approved project. Explicit workspace, project-relative destination, expected parent directory revision from `files_list`, expected domain revision and retry key are required. `files.read` and `files.mutate` are separate grants. Native creation uses a pinned NOFOLLOW directory and O_EXCL/mkdirat, never overwrites an existing entry, creates private 0600 files/0700 directories, fsyncs the new entry and parent, and does not remove a published entry after uncertainty. The editor handles later content writes. Main uses the same file-operation busy guard and shared editor pause before a receipt-bound one-use native commit. Result contains bounded relative-path/type metadata. Rename/move/trash remain required follow-on variants and are not yet exposed. No arbitrary operation dispatcher, cross-project move or force overwrite is implied by this first variant.

### File creation native PASS; regular-file rename/move in qualification — 2026-09-23

Native48 `lomi-mcp-control-iD03oY` PASSED (`/tmp/lomi-mcp-files-create-native.log`), including explicit files.create approval, creation of a private directory and nested empty file, same-receipt retries and refusal to overwrite user-written bytes at an occupied destination. Full editor/browser/PTY/Android/Codex regression passed; cleanup removed fixture app data and stopped private resources. Core40+UDS28+protocol9, wire5, UI2, pnpm check and production Clippy PASS for this increment.

`lomi_files_mutate` now also implements `rename` and `move` for regular single-link files up to4MiB, within the same approved project. Additional scope files.rename (separate checkbox) is required along with files.mutate/files.read; creation/save grants do not imply it. Both variants require source expectedDiskRevision SHA-256 plus destination expectedParentRevision from files_list. Rename takes one newName; move takes targetRelativePath. Pinned parents/NOFOLLOW stable source snapshots + rustix1.1.4 renameat_with(NOREPLACE) prevent destination clobber; no cross-device copy/delete fallback. Existing source inode/permissions survive. The main Workbench reuses applyFileChange/relocateEditorFiles while holding the existing file-operation busy guard and pausing disk refresh/saves; dirty buffers/undo survive. Native canonical source provenance is carried across model aliases (/var versus /private/var) to keep subsequent editor read/save authorized. Directory rename/move and trash remain TODO, explicitly absent from the exposed contract.

Foundation3tests, core41+UDS29+protocol9, wire5 and three permission/explorer UI tests PASS; TypeScript PASS. Full native current run `/tmp/lomi-mcp-files-move-native.log` (exec session56610): checks dirty editor rename, cross-directory move, unchanged disk bytes, same-document read through new paths, retry after source disappears, undo and clean close. Inspect this result and screenshot before advancing. Native fixture active means do not edit src frontend until it exits.

Primary API evidence: https://docs.rs/rustix/1.1.4/rustix/fs/fn.renameat_with.html and installed1.1.4 fs/at.rs + backend/libc/fs/types.rs map NOREPLACE to macOS RENAME_EXCL. macOS ARM64 was tested; Linux implementation is compiled by conditional source but not host-qualified.

Directory rename/move next contract: explicit `rename_directory` and `move_directory` variants avoid overloading expectedDiskRevision with a directory observation. Both require files.read/files.mutate/files.rename and expectedDirectoryRevision from files_list of the source, plus expectedParentRevision from files_list of the destination parent; rename_directory takes newName, move_directory takes targetRelativePath. Revisions describe bounded immediate directory entries, not a recursive snapshot of file contents. Native pinned-parent exclusive rename preserves every descendant without reading bytes, deleting or copying. Deny root/secret/link paths, destination inside the source, collisions and cross-device fallback. Main applies the existing folder relocation to all retained aliases and canonical descendant source paths while file refresh/writes are paused; dirty text/history survive. Operation/input limits remain30s and10000 entries per observed directory. Directory trash remains a separate unimplemented approval/guarded flow.

### P5.1 addendum — guarded Trash and preview contract (supersedes earlier gaps)

Tool48 `lomi_files_mutate` now includes Trash with files.read + files.mutate +
files.trash, exact source/parent revisions and the existing dirty-document guard.
Dirty buffers produce awaiting_user; Cancel and MCP cancellation preserve them,
Save cancels deletion and requires a fresh request, Discard authorizes the exact
native plan. Recovery stages original inodes with a durable journal before the
macOS NSFileManager Trash operation. Native `R6zQsO` verified all four choices,
real system Trash bytes/inode, exact replay, editor cleanup, Android input and
rotation. Recovery Settings exposes only the fixed private recovery folder.
Completed-journal retention, full-disk injection and broader lifecycle matrices
remain open; no automatic unresolved-data deletion is implemented.

Tool46 `lomi_editor_open` now accepts optional `presentation` (editor default,
preview, split); text, Markdown and SVG share retained buffers. Raster images
require preview and return editor_previewed with diskRevision plus original and
preview dimensions, no documentId. D34 defines bounded first-frame PNG conversion
and scoped embedded-image permits. Unknown presentations/unsupported combinations
fail before dispatch. Open uses the existing requestKey/retryEpoch/layoutRevision,
30s deadline, files.read/editor.read/panel.create/panel.focus, pinned NOFOLLOW
source and source SHA. Bodies are native-to-main only. AVIF is unsupported.

Agent-opened Markdown/image descriptors retain a deny-by-default marker across
session restoration. Transient bodies/asset permits are not persisted. Missing
permits do not fall back to the ordinary file API. Preview sources consume at most
16 MiB encoded data across 64 retained panel entries; identical pending asset reads
coalesce, with 64 queued reads. Native asset permits expire after15min and bound
64 requests /32MiB source /16MiB encoded output; scope/session/root checks apply
before read and disclosure. Closing the panel releases its permit. External
Markdown images stay links. The source file is never converted or overwritten.

Verified: decoder2, real-UDS asset authorization/root-swap/revoke/limit test,
wire5, pre-existing preview UI13 and new UI2 with restored denial and scoped images.
Native preview qualification is still running; earlier attempt0zyMaR failed on a
fixture tool-name typo, w5NBG7 verified PNG pixels but hit a legitimate revision
conflict during focus restoration after close. The fixture now waits for editor
readiness and a stable layout before each preview mutation; no conflict guard was
relaxed. See IMPLEMENTATION-STATUS for the current native run.

### P5.2 first closed observation — Git status (contract before exposure)

`lomi_git_status`: workspaceId, optional repositoryRelative (default project root),
limit1..200 (default100), optional opaque cursor. Requires both files.read and the
separate git.read grant at acceptance, execution, cursor access and disclosure.
Resolve an exact pinned project/subdirectory; no ancestor repository discovery.
First implementation supports a real .git directory on qualified macOS ARM64;
linked worktrees/external metadata are explicitly unsupported pending qualification.
The closed native reader from D35 enforces a5s child deadline, a single Git process,
2MiB stdout/16KiB stderr caps, immediate revoke/EOF termination and full child reaping.
No caller executable/arguments/env/config values are accepted. Any stderr rejects
results, including exit0 filter failures. Native config/index writes are forbidden.

Output: source=git, consistency=per_command_snapshot, repositoryRelative,
workspaceId, observed Git version, observationRevision (SHA256 of the observation),
changes with project-relative path and index/worktree status, directory marker,
optional original rename path, omittedEntries, and nextCursor. Omit secret/link/
unsupported paths before disclosure. Preserve Unicode; reject malformed/non-UTF8
wire observations instead of inventing target names. Result maximum4096 changes/
1MiB metadata before pagination, page48KiB; at most4 immutable caches for60s.
Cursors bind owner/policy/workspace/project inode/repository and exact observation;
changed query or expiry returns CURSOR_EXPIRED. Read is effect-free, no durable
mutation receipt or caller retry key. It does not stage/commit/fetch/open a view.
Git diff/history/remotes and guarded mutators follow as separately verified increments.

### P5.2 remaining bounded observations — contract before implementation

`lomi_git_diff` takes workspaceId, repositoryRelative (empty means exact project
root), relativePath relative to that repository, comparison (worktree/staged),
startUtf16 (default0), maxChars (default4096,2..8192) and optional
expectedObservationRevision (mandatory after offset0). Returns the literal
Git unified patch, source=git, per_command_snapshot, SHA256 observation revision,
UTF16 offsets/total/next and binary notice. Each page repeats the guarded read;
changed content fails RevisionConflict instead of joining different patches.
The native read ceiling is2MiB and output text per page32KiB. No pathspec magic,
secrets, current symlinks/hardlinks/special files, recursive directory diff or
external helpers. Untracked files have an explicit notice because ordinary Git
diff excludes them; no invented empty comparison verdict. Read-only view opening
is a subsequent separately contracted operation.

`lomi_git_history` takes the same repository identity plus optional full commit
SHA, skip(default0,max10000), limit(default20,1..100). Pagination requires the
returned full startCommit and nextSkip, pinning the starting commit rather than
following a moving HEAD. Each page is at most48KiB, commits carry full IDs, parents,
author name, timestamp and subject. This is a commit traversal observation, not
an atomic working-tree/index snapshot. Unborn HEAD is explicitly unsupported until
a commit exists. `lomi_git_commit` takes an exact full commit SHA and returns that
commit's metadata and exact message (at most64KiB). Blob/tag-to-blob IDs are refused;
no arbitrary Git revision expression or object read API is exposed.

`lomi_git_remotes` returns at most64 config entries with remote name, fetch/push
role, transport, hostname and optional port. Userinfo, path, query and fragment
are never returned. Local paths and unknown helper transports stay redacted; no
credential helper, URL rewrite expansion or network contact occurs. The result
explicitly reports locationRedacted; it is insufficient to authorize a mutation.
An exact native mutation preview will resolve and approve actual remote/ref.

All four reads require files.read plus git.read, the pinned approved workspace
and project, the same fixed executable/OS read guard as status, and live grant
checks before and after. Two shared file producers and one native Git child cap
concurrency;5s child deadline and bounded stdout/stderr. Read retries have no
mutating effect; revocation kills/reaps an in-flight child. macOS ARM64 only;
other hosts return HostUnqualified. No raw stderr/configuration enters replies.
Tests must include real IPC/helper/native execution, Unicode, staging vs working
comparison, pagination under changes, secret/link refusal, full commit identity,
remote credential redaction and grant loss.

#### Commit file observations and historical comparison

Before integrating existing read-only views, extend `lomi_git_commit` with bounded
changed-file metadata (filesSkip default0; filesLimit default100,1..200),
nextFilesSkip and omittedEntries. IDs remain full and immutable. Each row reports
repository-relative literal path, status and Git modes; secret paths, symlink and
submodule modes are omitted. First-parent comparison for merges and empty-tree
comparison for initial commits match the product. Native raw list cap2MiB,4096
entries; encoded public page remains64KiB including exact message and metadata.
Return committer name/email/date from the same commit, without mailmap. No invented
line counts; read-only UI must show that this metadata has no line statistics.

`lomi_git_diff` comparison adds `commit` with a required full `commit` SHA; the
commit field is forbidden for worktree/staged. Historical patches compare the
commit to its first parent (initial commit to empty tree). Same raw-mode denial,
literal path, secret filter, output limits and UTF16 pagination as working/staged.
Refuse standalone blobs. Any helper/network/write remains denied. These are
read-only observations and do not yet open a view or approve a mutation.

Primary semantics checked against https://git-scm.com/docs/git-show and
https://git-scm.com/docs/git-diff-tree; local Git2.54.0 root/merge prototype is
`lomi-mcp-commit-read-uoh3a3gu/observation.json`. No production qualification is
claimed for this extension before its new IPC/native tests run.

### P5.2 native read-only views — contract before implementation

`lomi_git_open` accepts workspaceId, repositoryRelative, view (closed diff with
relativePath/staged, or commit with exact commit SHA), expectedRevision, retryEpoch,
requestKey. Requires files.read/git.read/panel.create/panel.focus. Thirty-second
queued/claimed native preparation uses the same guarded observations before
publishing an existing Diff/Commit view, with no ordinary UI Git read fallback.
Result git_opened identifies workspace/panel/query and SHA256 of prepared data;
ACK requires the exact prepared body identity and current projected panel. Durable
retry never creates a second panel. No Git mutation occurs.

Main-only read permits bind owner/workspace/project descriptor/repository/query/
policy for15min, at most64 permits,64 reads and16MiB encoded output per permit.
Commit file selection may read only that approved immutable commit through the
same path/mode checks; no new executable/options/ref/network authority. Revocation,
root replacement, owner loss and expiry deny fresh reads. Main closes/replaces a
view by releasing its permit. Transient view data is capped at16MiB/64 panels;
restored agentGit descriptors without live permits show unavailable, never call
ordinary git_diff/git_commit_details/git_commit_diff. Native full observations
remain bounded by2MiB Git output and4096 changed-file rows. UI explicitly reports
omitted entries and unavailable line statistics. A user may open a file through
the existing explicit UI action. Agent focus/close may handle its read-only Git
panels with existing revisions and close guards, without starting another runtime.

### P5.2 Git index mutations — contract before exposure

The next `lomi_git_mutate` increment initially exposes only stage/unstage; other
variants enter only when their implementation and approval flows exist. Input:
workspaceId, repositoryRelative, operation with1..64 explicit file paths,
expectedRevision (UI domain), retryEpoch, requestKey. Requires files.read, git.read,
git.write and git.execute; Settings must disclose code execution by repository
hooks/filters/configured helpers. Every request queues a native exact preview and
waits for a main-window user decision. No MCP endpoint accepts that decision.
Native approval expires after120s. Reject unknown actions/options/environment.

Preview binds pinned project/repository, effective user+repository configuration,
HEAD/ref, physical index, references, exact source content+inode/mode/mtime and
captured native environment. Read preview denies helper subprocesses, network and
writes while allowing effective user-config reads for this separately authorized
execution scope; only hashes and nonsecret facts leave native RAM. Each file at
most4MiB, aggregate32MiB, no links/hardlinks/directories/known secret paths; native
index at most32MiB. Two consecutive snapshots must match, and dispatch recomputes
it again against the approved revision. This deliberate limit may reject projects
whose configured read filters cannot run within the read-only preview guard.

Actual mutation retains the existing shared Git mutation_guard and literal
argument construction, normal user identity and configured filters/hooks. It uses
the exact pinned Git executable and directory, without a read-only sandbox;
git.execute is therefore code trust, not restricted filesystem authority. Captured
environment strips caller-discoverable Git directory/index/object/config injection
variables while preserving native author/committer and configured Git SSH/global
config variables. It never accepts environment from a tool request.

Stage is literal `git add -- paths`; unstage restores the index from HEAD, or
removes selected cached entries for unborn HEAD, keeping working files. Each
spawns once after approval. Deadline30s;2MiB stdout/16KiB stderr bounds; raw output
is not an MCP diagnostic. Shared process Tree retains unreaped leader identity
until descendants exit, kills only that owned group after cancel/deadline, and
retains ownership/locks until Stop is confirmed. Detached trusted code remains
outside this guarantee (P7 not selected). Post-spawn failure, cancellation or
missing post-observation must never be replayed or described as rollback.

Foundation evidence alone does not qualify a tool. Required next tests cover
real main approval/cancel, receipt reuse, config/HEAD/index/path changes while
awaiting approval, scope denial, preserved filters/hooks, cancellation, and user
Source Control regression before native tool55 can be marked verified.

### P5.2 commit — implemented, native qualification in progress

Extend the closed mutation family only after a working adapter exists. Commit
must identify workspace/repository and the complete explicit staged path set,
exact message, expected UI revision and durable retry key. It requires the same
files.read/git.read/git.write/git.execute grant and one-use main UI decision.
No automatic stage, amend, reset-author, attribution, arbitrary options or remote
operation. A known-secret, symlink/submodule, unmerged or hidden extra staged entry
prevents the commit adapter from approving an incomplete path list. Unborn HEAD
is supported; ongoing merge/rebase/cherry-pick state must not be silently finished.

Preview must bind all staged paths/blob IDs/modes, exact index/HEAD/config/ref
snapshot, effective author/committer identity and message. Preserve the existing
native user's identity and --cleanup=verbatim behavior. The existing Git command
adds one final LF to an unterminated message; show the complete stored message
before approval and preserve all other spaces/newlines/Unicode. Reject NUL,
whitespace-only or oversized messages (stored UTF-8 at most64KiB).

Read-only preparation cannot run hooks or signers. Actual approved execution must
preserve user hooks/filters/signing choices, use the shared repository guard and
owned process tree, and never replay an uncertain result. Hooks are trusted code
and may themselves cause side effects; do not call that OS containment. A successful
receipt requires the actual new commit/parents/message and staged content to match
the approved intent. Changed HEAD/index/config before dispatch fails without
starting commit; observed divergence after a hook runs is uncertain and preserves
all resulting data for inspection. Cancellation retains process ownership until
exit and does not reset the repository.

Required evidence: fixture user identity and multiline Unicode message preservation;
no implicit staging of a modified working file; hidden extra/secret staged changes
rejected; changed index/HEAD/config during approval; real pre-commit and commit-msg
hooks including rejection and mutation; exact retry; native main decision and UI
feedback; existing Source Control behavior. The implementation details of staged
content comparison and commit object verification remain to be written/tested.
Primary sources inspected: https://git-scm.com/docs/git-commit,
https://git-scm.com/docs/git-var, https://git-scm.com/docs/githooks,
https://git-scm.com/docs/git-ls-files. Local message prototype evidence is recorded
in QUALIFICATION.md. This contract is TODO and does not add a public commit tool.

Commit adapter update (2026-09-23): `lomi_git_mutate` now accepts `operation=commit`,
`message` (1..16 KiB, nonblank, no NUL), and the exact complete staged path set.
Stage/unstage reject message. The native plan shows stored text, final-LF notice,
identity and staged object IDs; no automatic staging, amend or attribution. The
sealed executor rechecks approval and preserves hooks/signing configuration. It
verifies raw stored commit text/identity/parents and full staged-object delta,
then requires a stable post-state with no remaining staged changes. Divergence,
failed hook or cancellation after spawn settles as uncertain without rollback
or replay. Receipt includes native commit head and message SHA-256; actual commit
can be read with `lomi_git_commit`. Core hook tests, UDS create/retry/forged-ACK
and UI exact-message/cancel/small-window tests pass; native fixture is running.

## P5.2 fetch — contract before exposure

`lomi_git_mutate(operation=fetch)` will require `git.network` in addition to
files.read/git.read/git.write/git.execute and an exact main-view approval. Input
will name an existing simple remote and full refs/heads branch; paths must be
empty and message absent. It fetches that branch into the corresponding
refs/remotes/<remote>/<branch>, without force, pruning, tags, submodule recursion,
FETCH_HEAD or background maintenance. Other refs, HEAD, index and worktree status must remain unchanged. No worktree
mutation command is dispatched; approved helpers still execute unsandboxed. Preview is local and binds expanded effective remote URL,
configuration, refs and snapshot; credentials never enter MCP, and main displays
a redacted location. Multiple URLs/custom helper protocols are unsupported.
The owned process is cancellable and bounded; any post-spawn failure is uncertain,
never retried automatically. Successful receipt gives the observed local tracking
commit, not an invented guarantee about a remote after connection ends.
Tests must use owned local/bare repositories and a local transport fixture; no
user remote is contacted. Push/pull/discard remain required subsequent adapters.

## P5.2 push — contract before exposure

Push requires git.push and git.network in addition to Git code/write/read access
and exact main-window approval. Input names a configured remote, full target
refs/heads branch, exact source commit equal to current HEAD, and an expected
remote commit (null means create only if absent). Local preview resolves the
push URL without network access and verifies the expected old commit is an
ancestor of source. Grafts/shallow repositories are rejected for this proof.
Dispatch names the immutable source object and target ref; an exact explicit
`--force-with-lease=<target>:<expected>` is used solely as compare-and-swap, after
the independent fast-forward proof. No rewriting history, caller force flag,
implicit tags/mirror/matching refs/submodule pushes or automatic upstream change.
Only new/up-to-date/fast-forward porcelain results for the approved ref can pass;
local post-state is checked and the receipt reports the server acknowledgement,
not a perpetual remote-state guarantee. Any nonzero/cancel/ambiguous post-spawn
result remains unknown and cannot replay. All tests use owned local bare remotes.
Primary semantics: https://git-scm.com/docs/git-push (explicit expected lease and
porcelain flags). The exact-value lease must never use tracking-ref heuristics.

## P5.2 discard — contract before exposure

`lomi_git_mutate(operation=discard)` restores only the exact selected tracked
working-tree paths from their current index entries, preserving staged changes.
It requires git.discard in addition to git.execute/write/read and files.read,
a current layout revision, durable request key and a one-use main-window decision.
The approval lists each current disk SHA/size, indexed blob/mode and full bounded
working diff; the 32 KiB approval budget fails before any effect. Index, HEAD,
refs, config, environment and selected file identities are bound and rechecked.
No reset, clean, autostash, force or conflict resolution is provided. Untracked
files use the existing approved `lomi_files_mutate` Trash flow and its dirty-buffer
guards; this Git branch must never unlink them. Symlinks, special files, hard links,
secret paths, submodules and unmerged index entries remain excluded.

Main pauses existing editor file operations and rejects selected shared buffers
with unsaved changes/conflicts immediately before dispatch. Approved Git preserves
configured code; the existing native process owner and repository guard remain
held through confirmed stop. Post-execution requires unchanged HEAD/index entries/refs/
configuration (Git may refresh index stat metadata) and no remaining selected working diff. Failed/interrupted or
unverifiable post-spawn execution is outcome_unknown, without replay or rollback.
The permission and approval explain that discarded working bytes are not saved
in Git or Trash. Tests must cover partially staged files, missing tracked files,
stale approved content/index, dirty buffers, cancellation and exactly-once retry.

## P5.2 pull — contract before exposure

Pull is a composed native fetch and integration operation, with an explicit
`ff_only` or `rebase` mode, exact remote/full branch, current sourceCommit and
expectedRemoteCommit. The expected remote object must already be available locally
(use the approved fetch first); preview performs no network or configured code.
It requires git.network and a separate git.pull grant plus existing Git write/code
and file-read grants. The native one-use approval shows both commits, remote,
mode, local commits potentially rewritten and bounded affected working paths.
A clean index/worktree, attached branch and no in-progress merge/rebase are required.
No implicit stash, abort/reset, merge resolution or remote publication is provided.

The native plan binds the full graph, config, refs, index and affected file
identities. New incoming files must not collide with existing untracked/ignored
content. Preview bounds affected paths, blobs and replayed history; secret/link/
submodule paths and unqualified graph forms fail before approval. Main additionally
rejects dirty/conflicted shared buffers for affected paths while file I/O is paused.

After the exact fetch, the received commit must equal the approved remote commit.
A changed remote produces a typed partial receipt without integration, requiring a
new request and approval. Integration operates on that immutable commit. Fast-forward
uses `--ff-only --no-autostash --no-overwrite-ignore`. Rebase uses the merge backend,
no autostash/autosquash/update-refs/rebase-merges and no rename inference, with the
replayed commits disclosed. User-configured hooks/filters/signing remain trusted
code under the existing execution permission. Post-checks verify the selected
branch, refs, clean index and resulting ancestry; conflicts remain on disk for the
user and produce a partial receipt. Interrupted/unverifiable effects remain unknown,
never retried, rolled back or automatically resolved. Native process ownership and
repository locking cover both phases and confirmed stop.

Primary command semantics checked 2026-09-23:
https://git-scm.com/docs/git-merge (ignored files default to overwrite),
https://git-scm.com/docs/git-rebase (history rewriting, update-refs, backend and
stash controls), https://git-scm.com/docs/git-pull (fetch plus integration).

## P5.3 panel layout — first contract before exposure

`lomi_panel_move` first adds the existing model's bounded operations within one
approved workspace: reorder a top-level tab, dock a complete compatible tab into
an existing mixed terminal layout, or move a pane within that layout. Each request
names source and target IDs, a closed placement variant, expected layout revision,
retry epoch and request key. There is no arbitrary serialized layout replacement.
Both source and target must resolve in the granted project/workspace at dispatch
and ACK, with their current terminal/browser identities. This requires a separate
panel.move opt-in plus workspace.write. It does not grant terminal execution,
change cwd, open files, start a lazy runtime or close a resource.

The adapter uses moveTab/mergeTabs/movePane and the measured layout constraints.
It preserves the entire existing panel descriptors and retained runtimes, with
one atomic domain update and durable retry. Dirty editor buffers and running PTYs
remain alive; transient Dockview unmount is not domain removal. Refuse a hidden or
unmeasurable docking target, modal, changed source/target/revision or invalid layout
fit before effect. Reordering a tab does not select it or start its contents.
Cross-workspace movement and ancestor/project close are subsequent required P5.3
increments; they are not silently inferred from this permission or marked complete.

The initial panel-move implementation is exposed as tool56. The closed input is
`{workspaceId, movement, expectedRevision, retryEpoch, requestKey}`; movement is
`reorder_tab {tabId,beforeTabId}`, `dock_tab {tabId,targetTabId,side}` or
`move_pane {panelId,targetPanelId,side}`. Side is left/right/top/bottom; null
beforeTabId appends. Output is the standard operation receipt with panel_moved
containing the workspace, exact movement and sorted panel/runtime identities.
All workspace panels are bounded to128 and the plan to48KiB. This is an MCP
operation budget, not a product tab limit. Docking additionally requires
panel.focus, a visible measured terminal layout and already live, qualified
contents; reorder does not select or instantiate lazy contents. Android/chat/
plugin docking remains a required subsequent increment, not qualified by this
initial implementation.

Admission/claim/ACK bind the canonical granted project/workspace, domain revision,
source and destination, tab order, panel identities, PTY sessions and browser
identities. Broker checks the published order/membership and preserved runtime
identities before success. Main applies moveTab/mergeTabs/movePane atomically;
it never disposes a runtime because Dockview temporarily unmounted its portal.
A modal or ongoing close/file operation returns TARGET_BUSY; hidden/insufficient
geometry returns PANEL_NOT_RENDERABLE. Unknown/foreign target, lazy runtime,
revoked permission and stale revision fail before mutation. Receipt reservation
precedes dispatch; cancellation or uncertain publication after claim is never
silently replayed. A main OUTCOME_UNKNOWN after applying the move retains unknown
effects. Result disclosure rechecks the move's permissions. Pending operation
uses the existing bounded UI deadline/revoke/cancel lifecycle.

Adapters: protocol/layout.rs, broker/panel_move.rs, agent-layout.ts and the shared
main bridge/model. Tests: tests/agent-layout.test.ts (descriptor/profile retention,
identity, order, scope and fit), tests/broker.rs panel_moves (real UDS, separate
permission, secondary target, publication, forged runtime ACK, stale claim and
retry/unknown), helper schemas and Settings opt-in. Native mixed PTY/browser/dirty
editor docking, undo, modal, hidden target and repeat receipts PASSED in the full
native56 KRWX3H runner; see panel-moves.json and IMPLEMENTATION-STATUS.

## P5.3 ancestor close — contract and initial adapter

Workspace close is an explicit `workspace_update` action with the current
revision and retry key, separate workspace.close permission, and an exact bounded
set of descendants. It removes session descriptors, never the project directory.
Project close must prove approval of every affected workspace; a project root
permission alone is insufficient. Existing terminal origin checks must run before
any descendant effect and cannot be bypassed by a confirmation dialog. Plans bind
all runtime generations and must stop after a concurrent domain change.

The shared Workbench close path includes editor/plugin save-discard-cancel,
chat stop/flush, last Android view shutdown and retained terminal/browser cleanup.
An agent must not bypass these guards or silently approve them. Cancellation after
a confirmed runtime stop preserves the stopped state and reports partial/unknown
work as appropriate; it does not restore an old domain snapshot or claim that no
effect occurred. Closing the active workspace must not implicitly select an
unapproved workspace and start its lazy runtimes.

Ordinary operation lookup requires a live workspace projection. Close has a
narrow, authenticated receipt path after that projection disappears: the original
pairing/project/workspace grant and close scope must remain required, and only the
exact persisted closure receipt may use the retired target. This must not broaden
ordinary files, Git, history, resource or artifact access to a closed/unapproved
workspace. Retry after removal must return the original receipt, including uncertain
effects, instead of resolving a replacement by name or performing another close.
The acceptance tests must include last-workspace close, late ACK, changed children,
foreign secondary workspaces and operation lookup/retry after the target is gone.

The initial adapter accepts `{action:"close", workspaceId, expectedRevision,
retryEpoch, requestKey}` with no caller-selected descendants or confirmation
boolean. It requires workspace.write, workspace.close and panel.close; live PTYs
also require terminal.execute. Native preparation binds at most128 exact panel
identities within48KiB and expires after120seconds. Only files, session-owned Git
views, idle owned PTYs and live owned browsers are currently accepted. Human,
busy or protected-origin terminals are refused before any descendant effect.
Android/chat/plugin descendants and project close remain required P5.3 work.

The shared dirty-document dialog supports Cancel, Save and Discard. Save uses the
existing atomic editor writer but retains the workspace and returns failed/partial
with `workspace_closure.closed=false`; a fresh request must re-evaluate saved
state. A failed save retains RAM. Cancellation during a potentially partial save
is outcome_unknown, never a false no-effect result. A concurrent document/domain
change blocks removal. Closing the selected workspace leaves no active workspace
instead of implicitly starting another workspace's lazy runtime.

The result variant `workspace_closure` contains workspaceId/projectId, exact
panelIds/terminalSessionIds and nullable closed/projectClosed. Native commit
durably records both booleans as null before its first runtime close. A crash,
cancel or failed publication preserves this preparation and unknown effects.
The broker accepts a complete closure only after main publishes the disappearance
of all bound panels and the workspace. One SQLite transaction advances only the
matching prepared result to succeeded/complete and fills the final booleans.
Ordinary receipt result immutability remains unchanged. Retry returns the same
receipt after removal; a new request cannot access the retired workspace.

Adapters: protocol/workspace.rs, broker/workspace_close.rs, the main-only native
commit/pending bridge and shared Workbench/EditorCloseGuard/model transformations.
Real UDS tests cover last-workspace removal, ownership/scope, missing publication,
forged ACK, replay/conflict, unknown native effects and denied post-close reads.
The UI regression covers Cancel/revoke/failed Save/successful Save/fresh close;
the Settings regression covers dependency revocation without restoring consent.
Full native56 YtiSKE PASS: dirty Cancel/MCP cancel/actual Save/Discard, exact
retry, protected ancestor refusal, retired buffer denial and idle owned PTY/browser
close preserving other workspaces/processes. Host exit0/app-data removal passed.
UI mocks separately cover failed Save; they do not replace this native evidence.

## P5.3 cross-workspace transfer — implemented, native preservation evidence

panel_move adds `transfer_tab {tabId,targetWorkspaceId,beforeTabId}` within
the same approved canonical project. Both workspaces must be explicitly granted;
the project grant alone does not authorize the destination. The command must bind
the exact bounded source and destination panel identities/order and current domain
revision. The model moves the retained tab descriptor as one atomic session update,
preserving every pane/profile/buffer/runtime identity. It must not implicitly
activate a replacement lazy shell, browser, device, chat or plugin.

Existing broker publication revokes terminal/browser ownership when a panel
changes workspace. A transfer must instead migrate only the requesting session's
exact native ownership after verifying the complete destination publication and
the still-live operation/grant. It may not broaden ordinary publication into
arbitrary ownership migration. Running terminal command bookkeeping follows the
same generation to the new workspace; old workspace resource calls must fail.
Receipts retain their original idempotency scope, and retries return the original
result even after the source tab disappears. A lost/denied publication remains an
unknown UI effect and must not replay the move. Native tests must prove the same
live PTY and WK form, dirty editor/Undo, foreign destination denial, stale targets,
cancel/late ACK and no lazy runtime starts. Android/chat/plugin lifecycle needs
its own proof before claiming full layout qualification.

Transfer implementation uses `model.transferTab`, agent-layout/agent-control and
`broker/panel_transfer.rs`. Combined source/destination snapshots have the same
128-panel/48KiB ceiling. Ownership migrates once in broker publication, before
ordinary retention, only if the native permit, both explicit workspace grants,
original revision and exact resulting identities/order/focus remain valid. Late
publication after cancel revokes control and yields unknown effects. A success
ACK alone cannot admit migration. Source resource calls fail after transfer;
original receipts retain their retry identity and return no new dispatch.

Initial transferable kinds: files, lazy terminals, live owned PTYs/browsers and
owned Git views. Android/chat/plugin transfer remains required; source panels
outside the moved tab may contain those kinds without being touched. No cross-root
transfer is admitted. Moving a selected tab or populating an empty selected
workspace leaves global selection neutral; hidden moves preserve visible focus.
UDS1 proves foreign destination/cross-project refusal, native ownership/lease
retention, forged ACK refusal, exact retry/conflict, cancelled late publication
and refusal to reuse the consumed permit on an ordinary move. Model3 proves
mixed descriptor/profile preservation and absence of implicit lazy selection.
Native preservation of the live PTY/operation, WK form and dirty editor/Undo
passed in tLJg5N and in the transfer portion of tiONrr. The latter full run failed
later during terminal creation. Its moved-run retry returned the same original
running operation after transfer; fresh old-source reads remained denied. The UDS
regression also verifies cancellation follows the migrated generation. See
QUALIFICATION.md for whole-run status and the workspace viewport race.

## P5.3 project lifecycle — next implementation contract

Project close will use a distinct bounded tool with explicit projectId and anchor
workspaceId, expectedRevision and the standard retry epoch/key. Both IDs must
resolve to the same approved canonical project. Before admitting the request,
every current workspace in that project must be explicitly granted. No projected
workspace may be omitted because the client cannot see it. Bind the complete
workspace/panel/runtime snapshot; run all native terminal-origin checks before
any descendant effect and the shared dirty-document decisions before removal.

The project closes atomically in the shared session model after native preparation,
without deleting its folder and without starting an unapproved fallback project.
Save retains the project and reports partial effects; retry after disappearance
returns the original receipt. A typed project-closure preparation must persist
before the first native close, and its nullable closure state can only become true
after the complete matching domain publication. A failed or cancelled native
preparation retains unknown/partial effects, never a claimed rollback of stopped
processes. Receipt disclosure requires the original owner, project, all affected
workspace grants and close permissions even after the live projection disappears.
The shared resource-kind limitations must remain explicit until Android/chat/plugin
guards are qualified.

Project open needs a separate Settings approval for a new canonical root. The
current single-project Grant must not be expanded by changing its root fields in
response to an arbitrary tool call. Introduce explicit per-project root/workspace
bindings and keep scopes, pinned directory handles, receipts, native tickets and
secondary identifiers tied to the selected binding. Operation lookup by retry key
must be unambiguous across projects; current UI focus must not resolve mutations.
Settings must show the concrete canonical root, initial workspace, inherited or
new scopes, exact request and expiry. A pending request grants no reads or code
execution. Approval and the domain publication must match; cancellation, changed
root identity, revoked session and late ACK must not extend authorization. Open a
neutral editor descriptor so opening a folder alone does not start a shell.

### Domain revision and native ancestor completion

The bridge's control domain signature excludes only editor viewport positions on
actual FileTab descriptors, including mixed layout files. CodeMirror stores those
positions during detach; they do not identify a file or authorize a mutation.
Resource paths, generations, selection, layout, workspace names and arbitrary
plugin state remain in the signature. After claim the bridge retains the latest
equivalent model so cursor persistence cannot be overwritten by an older snapshot.

After native workspace close, removal uses the latest model and verifies the exact
target structure. Unrelated workspace changes are retained. Only transient panel
titles and editor positions may differ on that target; changed paths, children,
automation identities or approved workspace identity conflict. Captured immutable
editor Text values are rechecked, and newly loaded dirty final-view documents
block removal. A conflict after native effects retains the prepared unknown result.
The native diagnostic 19i2LE identified viewport persistence as the terminal-create
race; focused 4gguA9 completed six closures but its reporting wrapper failed, and
focused wNJLEh passed with the corrected wrapper and complete cleanup. Full native
regression for the final source is tracked in IMPLEMENTATION-STATUS.md.

### Project close implementation in qualification

Catalog57 adds `lomi_project_close` with projectId, anchor workspaceId, expectedRevision,
retryEpoch and requestKey. Separate `project.close` requires workspace.close,
workspace.write and panel.close; live PTYs also require terminal.execute.
Every live workspace in that exact granted project/root must be granted, including
empty workspaces. Preparation is bounded to 128 workspaces, 128 panels and 48KiB.
All descendant guards run before effects; unsupported kinds remain preserved.
The typed ProjectClosure records all original workspace/panel/PTY IDs and nullable
closed state. Its successful final publication advances only that exact prepared
record; unknown effects preserve the preparation. Retired receipt access checks all
original workspace grants without exposing live resources in a closed root.
Settings allows explicit selection of additional same-project workspaces, without
selecting them automatically. New unapproved workspaces block project closure.
See IMPLEMENTATION-STATUS.md for tests and outstanding native qualification.

### Project open and multiple root bindings (catalog58, qualification in progress)

`ProjectOpenInput` binds an existing approved anchor workspaceId, an absolute
projectPath, initial workspace name, expectedRevision, retryEpoch and requestKey.
The anchor may be retired: its original project remains the receipt's project key.
A repeat is resolved before canonicalizing its path again. New project/workspace/
editor IDs are broker-generated, not credentials. Opening an already-open canonical
root conflicts; no existing private workspace is enrolled by that request.

A connection holds at most sixteen explicit per-project bindings, each with its
own project ID, canonical root, pinned directory handle and granted workspace IDs.
All bindings inherit the same immutable scopes and browser/Android choices shown
in the connection's original approval and the new folder's Settings request.
Adding a project never expands scopes. File access, native tickets and receipt
keys resolve the binding from explicit target IDs, never current selection.
Operation lookup by ID searches only granted projects; key lookup accepts explicit
projectId and permits omission only while exactly one project is granted. Cross-
project tab transfer remains unsupported and is explicitly rejected.

`project.open` requires workspace.write/panel.create and only permits requesting
Settings approval. The 120-second approval binds canonical root identity, initial
workspace, generated IDs, request key/operation, original scopes and UI revision.
A typed `ProjectOpened(opened=null)` receipt is recorded before dispatch/approval;
queued/awaiting_user/cancelled with effect=none means no project was opened. Approval
transitions awaiting_user to queued; one native commit transitions to running.
Cancellation, revoke, expired work, changed root or late publication cannot add a
grant. Success requires exact domain publication of one new workspace and one file
panel, an active approved native ticket and an unchanged pinned directory. Only
then may the private lifecycle CAS advance the exact prepared result to opened=true
and add that project/workspace binding. Ordinary receipts stay immutable. A lost
post-commit result is unknown; an unacknowledged descriptor grants no agent access.

The UI opens a blank untitled editor and retains existing projects and dirty shared
buffers. No shell profile starts. Settings alone may approve/reject the exact
request; the MCP input has no approval field. New native commands preserve the
trusted main/Settings caller boundary. Seven UDS approval/cancellation/root/ACK
cases and the two-root file/receipt/key/transfer isolation test PASS. UI and native
proof are recorded separately in IMPLEMENTATION-STATUS.md.

### Settings adapter contract (P5.4, implementation and qualification active)

`lomi_settings_open` requires a live explicitly approved workspace anchor and
`settings.open`, independently from file reads and settings mutation. Its closed
page enum is keybinds/themes/plugins/editor/terminal/about/chat-ai/android/agent-control.
The input carries the expected domain revision and the existing durable retry
identity. It requests the existing Settings window/page through the trusted main
bridge, preserves all workspaces/runtimes, and performs no preference write or
code evaluation itself. The typed result records the requested page and confirmed
native window request, not completion of a settings change. Exact retries return
the original receipt. Cancellation before claim has no effect; an ambiguous lost
ACK after dispatch must not be replayed automatically. Every operation lookup
still checks the original owner, anchor and `settings.open` scope.

Subsequent read/update adapters use a separate `settings.read`/`settings.write`
grant for global nonsecret preferences, with section and field-specific DTOs.
They reuse the existing editor/terminal/shortcut/theme validators, file locks and
change events; the UI-only write commands retain their Settings caller checks.
Settings reads do not include chat credentials, Android tokens, plugin source or
arbitrary files. Updates must preserve unrelated fields, use an expected settings
revision, reject malformed/incompatible state without replacement, preserve
retained editor/PTY state and revalidate both before effect and result disclosure.
Theme artifact import/trust and protected control visibility require their own
qualified flow; this contract does not authorize generic CSS or plugin execution.

Settings read uses the current main-window preference providers as the source of
truth for effective nonsecret values. The closed sections are editor, terminal,
keybinds and themes. `settings.read` is independent from opening or changing
Settings and is global in scope, explicitly described during pairing. The snapshot
has an opaque content revision and a ready/recovery_required state; provider errors
never expose raw messages, paths or corrupt file content. Editor values are global
defaults, not per-buffer overrides. Terminal appearance overrides are distinguished
from the selected theme. Theme output contains selection IDs, appearance and safe
mode, never CSS, asset paths or source. Shortcut entries include action IDs and
current/default bindings, bounded to 200 entries per page, with revision required
for subsequent offsets. The bridge hashes the complete bounded snapshot before
pagination. Broker authorization is rechecked after the reply; UI epoch, workspace,
section, bounds and content revision must match. No disk write/recovery is caused
by a read. A later write must additionally compare the native stored revision under
the existing section lock; a runtime snapshot alone is not a disk CAS token.

Settings update starts with the editor-defaults vertical slice, then must expand to
terminal preferences, shortcuts and safe theme selection/import before P5.4 is
complete. The single `lomi_settings_update` tool has a closed patch union, explicit
`settings.read` + `settings.write` grants, a live workspace/expected domain revision,
expected effective-settings revision and the normal durable retry identity. It
prepares a native immutable before/after plan after matching the retained provider
snapshot against the actual stored values. Every change requests an expiring exact
Settings decision. Only Settings can approve it; MCP inputs have no approval flag.
The main queue is released after preparation, while the operation remains
awaiting_user. Approval rechecks the native source revision and live authority;
new preferences are published atomically with the existing editor/terminal locks,
validators and change events. Unrelated fields, editor buffers/Undo and PTYs are
preserved. Invalid existing data requires explicit recovery outside this tool.
Native failure after publication is outcome_unknown and never replayed by retry.
Editor and terminal writes implement this contract with native evidence. Shortcut writes are implemented with native qualification passed; theme writes remain required follow-up work.

Terminal write extension (implemented; macOS ARM64 native qualification passed): keep one closed field per patch,
using the existing terminal validator and provider. Appearance fields support a
null value to restore theme inheritance; colors use a closed color-key set and
#RRGGBB/#RRGGBBAA values. Behavior fields retain the current numeric ranges and
integer requirements, boolean fields and bounded word separators. Fixed Windows
shell selection, notification preference and title visibility are data-only
choices; they do not accept executable paths, arguments or profile source and do
not start/restart a shell. The native plan compares the whole effective terminal
preference value and the stored source revision, then changes only the requested
field under TerminalPreferencesFile. The existing terminal-preferences-changed
provider/event route must update retained visible and hidden xterm instances while
preserving the same PTYs, transcript, selection and input state. Native terminal evidence is recorded separately in QUALIFICATION.md.

Shortcut write extension (implemented, macOS ARM64 native qualification passed): add closed set, disable (explicit
null), reset-to-default and focus-follows-pointer patches. Only builtin or installed
command IDs offered by the existing Keybinds provider are writable; unavailable
saved descriptors remain preserved. A write must not introduce conflicting
shortcuts or change other effective bindings through default fallback/migration.
The existing TypeScript restoreKeybindings validator remains authoritative for
platform defaults, plugin contributions, migration and collision handling; native
validation, the KeybindingsFile mutex and atomic stored-file CAS remain mandatory.

Because effective bindings include defaults and plugin contributions, the write
preflight must read a bounded native source plus an opaque fingerprint of native
plugin shortcut definitions. Main compares normalized native source/defaults with
its retained provider and the requested snapshot revision, validates the one-field
patch, and submits only the affected before/after values with native source proofs.
Main cannot approve. The native plan rechecks source proofs, computes the stored
one-field change itself, and binds application to unchanged plugin definitions
under the existing plugin lock. No plugin code, trust, execution, source paths or
credentials are exposed or changed by this flow. The Settings decision distinguishes
resetting an override from disabling an action, including when their current
resolved values match. The GJqZd0 native evidence and bounded checks are recorded in QUALIFICATION.md.

Theme write extension (builtin slice implemented, macOS ARM64 lt7OvZ qualified): first add closed builtin color-theme
selection (Lomi/DeepMono) and system/light/dark appearance patches. These cannot
introduce third-party CSS/assets; appearance mutation is available only when the
current color theme is builtin. The current provider revision and native byte CAS
must agree, with the original Themes mutex, version/ID/kind validators and change
event. Safe-mode/recovery states are not overwritten implicitly. Unrelated icon
selections and stored fields survive. Native approval and receipt rules are the
same as other Settings patches. Retained xterm/CodeMirror instances must apply the
new computed theme without losing PTYs, text, selections or Undo.

This first slice does not satisfy the complete theme requirement. Selecting
installed/custom packages, verified-artifact import and explicit recovery remain
required. Before allowing agent-introduced CSS, protect authorization/status UI
from package styles and retain the native Stop Agent Control menu. A builtin-only
slice must not be described as full P5.4 completion.

Theme control protection: trusted Agent control Settings and explicitly protected
critical Modal decisions (not the theme editor or ordinary browsing dialogs)
suspend package theme layers in their own native view, including asynchronously
committed stylesheets. Disabling only a dialog selector is insufficient because
package CSS can hide ancestors. Embedded builtin layers remain active. Nested
surfaces use a reference count; closing the last surface restores only the current
connected layers with their requested media. No stored preference or theme bytes
change. The native macOS menu opens the protected Agent control page and retains
Stop Agent Control independently of theme CSS and the main renderer. This covers
readable permission/status controls in Settings and confirmation dialogs; native
qualification must verify hostile ancestor/stylesheet rules, late reload, restore,
existing PTY/editor state and the native entry point before custom theme writes.
