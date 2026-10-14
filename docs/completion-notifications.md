# Completion notifications

Lomi records successful **managed Agent Runtime turns** and **persisted Chat AI
requests** in the owning SQLite transaction. A turn can independently qualify
for a local system alert and an Expo Push notification. Both settings default
to OFF and require a Lomi account; agent-provider accounts do not confer access.

Settings → Account owns the controls. The legacy Terminal notification setting,
terminal attention signals and notification inbox retain their existing behavior.
The new completion channels do not add inbox entries or invoke commands.

See [local validation results](completion-notifications-validation.md) for the
executed checks, native build blockers and external acceptance still required.
The [full retest receipt](completion-notifications-retest.md) records the later
verification run.

## Durable sources

- Managed attempts capture notification ownership on Send and switch/Continue
  admission. A settled Completed checkpoint and completed attempt produce the
  intent within `Store::update_task`, after the existing native settlement.
- Chat captures ownership with durable request admission. The successful
  `completed` checkpoint inserts intents, including a retried final save through
  `Backend::flush`. Connection probes without a conversation are excluded.
- Failed, cancelled, stopped, interrupted and recovery-required work, approval
  requests, imported history, UI EOF and PTY exit do not produce completion.
- Each source database upgrades additively to schema v3. Admission, cloud intents
  and desktop intents are separate from agent input dispatch. Source owners
  serialize reads and compare-and-swap transitions; there is no cross-database
  transaction or second writer.

Cloud keys hash the fixed ordered UTF-8 JSON fields `schemaVersion`, `sourceId`,
`sourceKind`, `logicalTaskId`, `turnId`. Local keys separately include desktop
installation identity, Lomi user, source kind and turn identity. Duplicate final
saves cannot create another intent. Consumed admissions prevent old completion
from becoming a new event after ledger retention cleanup.

Intents expire 15 minutes after the completion commit, with at most 1,000 pending
entries per source/channel and seven-day ledger retention. Task history,
checkpoints, receipts and provider credentials are never notification cleanup
inputs.

## Local system alerts

The account-scoped local preference file is atomic and environment-scoped. It
stores no credentials. `enabled` defaults to false and `backgroundOnly` to true;
CAS revisions reject stale saves and consent epochs fence OFF→ON transitions.

A native owner check requires retained credentials, a previously verified active
user, future Session expiry and no login attempt. SignedIn qualifies. Offline can
qualify with these same retained facts; unverified Offline and Checking cannot.
A ready intent can wait up to ten seconds for Checking to resolve.

The local consumer runs independently of cloud HTTP. Background-only checks the
**main** window; focusing Settings does not substitute for main focus. A focused
main window or failed focus read suppresses the alert permanently. Final
eligibility and plugin enqueue share a short guard with native consent changes
and account retirement. Store claim and acknowledgment are durable CAS operations.
Ready entries for the same restored Session can recover before their deadline;
claimed entries have an unknown outcome after a crash and are not replayed.

The existing Tauri Notification plugin remains pinned at 2.4.0. Its desktop Rust
permission functions report Granted without a real OS permission check and its
show operation returns before asynchronous OS delivery. `plugin-accepted` means
an invocation was accepted, **not displayed**. The Settings test is fixed-template,
login-guarded, rate-limited and uses no HTTP; it bypasses only background-only.
Settings receives check/request permission capabilities, never arbitrary send.
No desktop notification Actions or click navigation are guaranteed.

An alert already enqueued can appear after sign-out. Offline server revocation
cannot be observed immediately; reconnect and Session expiry fence future work.
Use the system notification settings and an actual test to confirm presentation.

## Phone delivery

The native AuthController owns credentials and fixed-origin GET/POST transport.
Typed push errors retain bounded status, code and Retry-After. HTTP is performed
outside source locks and rechecks the same account/Session/process authority.
The cloud dispatcher has bounded retries until the local deadline. Logout,
source retirement or changed consent/source epochs suppress older intents.

Process generation is a fence for current asynchronous work, not durable identity.
After a restart only the same safely restored Session/source may reauthorize
pending work; a logout marker or a new Session excludes replay. Intents never
become work for the next account.

The backend snapshots eligible phone installations when accepting an event.
Turning ON or adding a phone later cannot revive suppressed events. Its worker
checks current MobileSession dependencies rather than the short mobile access
token, and does not renew user sessions. Expo is the sole phone gateway. Tickets,
receipts and attempts have separate ledgers; timeout outcomes can be unknown and
retry can duplicate an alert. A receipt confirms provider handoff, not device
presentation or reading.

Lock-screen content is generic. Payloads contain only the fixed type, opaque
notification ID, global binding ID and binding generation. They contain no
prompt, output, task title, project path or Remote credentials. Mobile compares
the full binding identity, resolves the notification with its current account,
and opens a protected information screen. Managed tasks and Chat currently have
no Remote projection, so the result remains in desktop. Push never opens a
terminal, claims Control or sends input.

## Ordinary terminal qualification

Ordinary terminal completion remains disabled. OSC 9/777, terminal silence and
process exit are not completion authorities. Existing legacy hooks retain their
behavior and do not enroll a terminal into these channels.

Source verification found a feasible Codex correlation contract for 0.160.0 and
0.162.1: UserPromptSubmit provides session/turn IDs; the legacy
`agent-turn-complete` notify argument provides matching thread/turn IDs after
Stop hooks have finished their continuation decision. Stop alone is insufficient.
Notify also contains sensitive text that a future adapter must discard.

A qualified adapter still requires explicit setup preserving existing hooks,
per-launch authenticated native IPC, durable admission and live
completion/approval/cancel/duplicate tests. No such live qualification was run;
no unqualified ordinary CLI adapter is enabled. Claude 2.1.287 does not have a
verified documented start/completion join with a stable turn ID.

Verified primary references:

- [Codex start schema 0.160.0](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/hooks/src/events/user_prompt_submit.rs)
- [Codex notify schema 0.160.0](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/hooks/src/legacy_notify.rs)
- [Codex completion ordering](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/session/turn.rs)
- [Claude hooks](https://code.claude.com/docs/en/hooks)
- [Tauri notifications](https://v2.tauri.app/plugin/notification/)

## Staging and production checklist

Prepare the backend using its `PUSH.md`, additive Prisma migrations,
`ops/push-database-permissions.sql`, `ops/push.env.example` and dedicated worker
entrypoint. Apply migrations/grants as an operator, with the gate OFF. The worker
role must not read identity tokens, mobile credential hashes or provider-account
secrets. Readiness must pass for API and worker before canary delivery.

Mobile needs SDK57 `expo-notifications`, a rebuilt native binary and an explicit
environment-specific real Expo project mapping. Android also requires its matching
public Firebase config and notification icon. APNs/FCM and Expo enhanced-security
credentials are provisioned outside these repositories. The existing actual EAS
ID and owner changes are preserved; that does not qualify provisioning or reuse
across environments. OTA remains disabled.

Test both physical iPhone and Android, signed staging/release builds, foreground,
background, cold/warm tap, lock screen, denied/provisional permission, two phones,
rotation, reinstall and account changes. Record tickets/receipts separately from
visible alerts. Android force-stop and OS Focus restrictions are not delivery
promises. Test local banners in a packaged macOS application, installed Windows
application and Linux session with a working notification service.

No live migration, cloud upload, EAS/Firebase/APNs provisioning, deployment,
publication, Git commit or push is part of this implementation task. Until these
external acceptance steps pass, the local code and fake-provider tests establish
pipeline behavior only.

Rollback: close the push gate and stop worker emission. Disable/revoke remains
available. Keep additive tables and old-client compatibility; never delete task
history or rotate account credentials merely to stop notifications. Already
submitted provider/OS alerts cannot be recalled by this switch.
