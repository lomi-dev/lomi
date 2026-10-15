# Completion notifications

Lomi records successful **managed Agent Runtime turns** and **persisted Chat AI
requests** in the owning SQLite transaction. A turn can independently qualify
for a local system alert and an Expo Push notification. Both settings default
to OFF and require a Lomi account; agent-provider accounts do not confer access.

Settings → Notifications owns exactly three switches: “Notify when tasks finish”,
“Only when the main Lomi window is inactive”, and “Push notifications on my phones”.
These settings also govern live terminal agent alerts; CLI integration setup remains
an explicit status-bar action. Local terminal alerts and inbox entries require the
same account opt-in; the legacy Terminal switch no longer gates either channel.
The new completion channels do not add inbox entries or invoke commands.
Phone notifications also forward live agent alerts from ordinary terminal tabs
as a distinct, generic attention event, described below.

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
an invocation was accepted, **not displayed**. Settings provides passive permission
and account notices without test buttons.
Settings receives check/request permission capabilities, never arbitrary send.
No desktop notification Actions or click navigation are guaranteed.

An alert already enqueued can appear after sign-out. Offline server revocation
cannot be observed immediately; reconnect and Session expiry fence future work.
Use system notification settings and a real task completion to confirm presentation.

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

The desktop phone switch controls sending from this computer. ON explicitly
registers or enables its source and, if needed, enables account consent. OFF
disables only this computer's source and preserves account consent and other
computers. Checked means account consent and this nonrevoked source are both ON.
Refresh reads never register a source or opt a computer in. One Settings-only
native command captures the account binding for every request, uses fresh CAS
revisions and distinct operation IDs, and reads confirmed state after partial
failure. Uncertain writes are never automatically retried. Phone receiving is
managed in the phone app.

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

## Ordinary terminal notifications

Live terminal alerts can enter the cloud queue as
`agent.terminal.notification.v1`, with source kind `terminal-notification`.
This forwards an agent notification; it does not certify successful task
completion. Both ordinary OSC 9 and configured Claude OSC 777 alerts qualify.
Codex OSC 9 signals can mean that a response is ready or that the agent needs
approval. The phone shows the neutral fixed message
“Agent wymaga uwagi.” Open desktop Lomi to inspect the terminal.

The native host requires a live owned terminal session and captures the current
account, computer-source and cloud consent at signal receipt. A stable alert ID
deduplicates repeated delivery to the native host. Only opaque IDs and the
receipt timestamp enter the private durable cloud ledger; terminal context,
titles, paths, prompts and output never enter the push payload. Existing queue
limits, expiry, account/session retirement and consent/source epochs still apply.
Enabling notifications later does not replay the inbox or old terminal output.
The terminal ledger retains up to 100,000 small receipt records for seven days,
independently of the 1,000-pending-intent bound. Exhausted receipt capacity reports
an error. Rejected cloud qualification or storage failure does not prevent the
existing local inbox/OS delivery, including an alert parsed after terminal exit.

Settings → Notifications governs these signals. Its first two switches control
local OS delivery, including account qualification and main-window background
policy. The third switch controls phone forwarding independently of the local
switches. Account phone consent, this computer's source and receiving on the
phone must all be enabled for phone delivery. Forwarding does not add another local OS
alert or change CLI configuration. Managed tasks and Chat keep their separate
settled-completion contracts.

Verified ordinary terminal completion remains disabled. OSC 9/777, terminal
silence and process exit are not completion authorities.

## Staging and production checklist

Prepare the backend using its `PUSH.md`, additive Prisma migrations,
`ops/push-database-permissions.sql`, `ops/push.env.example` and dedicated worker
entrypoint. Apply migrations/grants as an operator, with the gate OFF. The worker
role must not read identity tokens, mobile credential hashes or provider-account
secrets. Readiness must pass for API and worker before canary delivery.

Mobile needs SDK57 `expo-notifications`, a rebuilt native binary and an explicit
environment-specific real Expo project mapping. Android also requires its matching
public Firebase config and notification icon. APNs/FCM and Expo enhanced-security
credentials are provisioned outside these repositories. OTA remains disabled.

Test both physical iPhone and Android, signed staging/release builds, foreground,
background, cold/warm tap, lock screen, denied/provisional permission, two phones,
rotation, reinstall and account changes. Record tickets/receipts separately from
visible alerts. Android force-stop and OS Focus restrictions are not delivery
promises. Test local banners in a packaged macOS application, installed Windows
application and Linux session with a working notification service.

Rollback: close the push gate and stop worker emission. Disable/revoke remains
available. Keep additive tables and old-client compatibility; never delete task
history or rotate account credentials merely to stop notifications. Already
submitted provider/OS alerts cannot be recalled by this switch.
