# Local MCP development preview

This is an implementation preview, not the completed v1 release. The current
native evidence is from macOS on Apple Silicon. See [implementation status](IMPLEMENTATION-STATUS.md)
for unfinished work and [qualification](QUALIFICATION.md) for the exact tests,
versions and limitations. A working connection does not qualify every tool or
another operating system.

The user narrowed this delivery on 2026-09-23: further theme/plugin MCP work,
application lifecycle tools and distribution/installers are excluded. Completed
modules are delivered as ordinary commits on origin/main, without tags or releases.

## Run and pair

From the application repository, install the pinned dependencies as described in
the main [README](../../README.md). Build the matching helper before starting the
desktop application:

```sh
cargo build --manifest-path src-tauri/Cargo.toml --locked -p lomi-mcp
pnpm tauri dev
```

1. Open the project in Lomi, then open **Settings → Agent control**.
2. Select **Enable for this Lomi session**.
3. Copy **MCP JSON configuration for this running instance** into the local MCP
   client's supported configuration interface. The generated command points to
   `lomi-mcp` next to the application executable. Its arguments identify this
   instance and its public broker identity. The JSON itself grants no access.
4. Start the client's MCP connection. Call `lomi_status` and match its
   `pairingRequestId` with the pending request displayed in Lomi.
5. Select the intended workspace and explicitly check any additional existing
   workspaces from the same project. Enable the required permissions and choose
   **Approve session**. The client name is only a label; match the request ID.
6. Call `lomi_workspace_list`, then `lomi_connect` with the approved workspace's
   ID. Keep the returned retry epoch for mutations on that connection.

Access ends when the connection is revoked, Lomi restarts, or the workspace view
is re-registered. Use fresh generated configuration after restarting Lomi or
disabling control. Do not reuse an endpoint from an old diagnostic run.

Codex can require its own approval before sending a modifying MCP call, in
addition to Lomi's pairing and operation approvals. A noninteractive
`approval_policy = "never"` does not grant that approval: the client can reject
the call before it reaches Lomi. Use interactive client approval, or explicitly
approve only the intended tools for a controlled noninteractive workflow with
`mcp_servers.<server>.tools.<tool>.approval_mode = "approve"`. This does not
expand Lomi permissions or remove native confirmation dialogs. See the
[official Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp).

The pairing form selects one existing project and its explicitly checked
workspaces. Workspace creation, when authorized, adds the newly created workspace
to that connection's grant. A project folder does not grant access to every existing
workspace. Project closure requires a separate permission and access to all of its
current workspaces; a new unapproved workspace blocks closure.

## Choose a terminal shell

Enable **Allow terminal creation, command execution and output reads**, then
select **Approved terminal shell**: the discovered system Bash or Zsh. Each
connection receives exactly one profile and its current revision.
`lomi_terminal_create` defaults to that profile; an explicit `profileId` must
match it. Changing that profile's definition invalidates the old approval. Reconnect
and approve the new profile to change shells.

Shell initialization still uses your normal configuration and account access.
The project working directory is not a filesystem or network sandbox. Automatic
`lomi_terminal_run` requires a confirmed idle prompt. On macOS Bash 3.2, an
existing DEBUG trap or debugger configuration is preserved; Lomi then refuses
automatic run readiness because it cannot safely install its preexec hook.
Explicit input remains subject to the terminal lease and ordered input sequence.
Human input revokes the lease. A prompt redraw after resizing cannot make
partially entered input eligible for automatic execution. Silence and shell EOF
without a completion marker do not imply a successful command.

## Import and export file artifacts

Pair with **Allow reading project files** and **Allow importing project files as
artifacts** to import an opaque file of0–4MiB using `lomi_artifact_import` with
`kind: file`. Supply its exact byte length and SHA-256 plus the usual revision
and retry fields. The completed private copy retains its original source
classification; changing the source afterward does not change that copy.
`lomi_artifact_read` returns its metadata, without exposing a private disk path.
APK imports retain their separate permission, validation and512MiB limit.

To save an artifact, also select **Allow creating project files and folders**
and **Allow exporting artifacts to new project files**. Call
`lomi_artifact_export` with the artifact ID/hash, a new project-relative path,
the parent revision returned by `lomi_files_list`, and the revision/retry fields.
The export limit is4MiB. Existing files and links are preserved. Source access
still applies: a closed or human-taken browser/Android source cannot be exported.
Poll the operation to confirm completion; reuse the exact original request for
retry. An uncertain outcome requires inspecting the destination before new work.

Browser upload and download require their separate permissions described below.

## Upload a file to a page

Enable browser navigation/read/interact and **Allow requesting file uploads to
pages** when pairing. Import the exact file as a private artifact, then use
`lomi_browser_snapshot` to find a visible `file_input` and its frame ID.
`lomi_browser_upload` takes that element/frame/snapshot/navigation, the owned
browser's generation and lease, the artifact ID/hash, a filename without path
components, and the usual layout revision/retry fields. The limit is4MiB.

Review the **Browser upload requests** card in Settings. It shows the exact
destination document/origin/input, filename, size and SHA-256. **Upload this file**
allows that page to read and send those bytes immediately. **Deny upload** or
MCP cancellation before approval leaves the input unchanged. Page text is
untrusted; check the destination and file identity. Snapshot references expire
after60 seconds and also become invalid after navigation or a new snapshot.

Poll the operation for completion. Success confirms synthetic attachment and
input/change events; it does not confirm the server accepted a submission.
Reusing the original retry key never uploads again. A dispatched operation with
an uncertain outcome requires checking the page before starting a new request.
Only same-origin frames are supported. File pickers and arbitrary local paths
are excluded; a changed source file cannot replace the already staged copy.

## Download a browser file

When pairing, enable browser navigation/read and **Allow downloading page files
as private artifacts**. Call `lomi_browser_download` with an owned panel's current
generation/navigation ID, an exact URL on its current origin, `maxBytes` (1 through
4194304), and the usual layout revision/retry fields. The isolated browser session
supplies same-origin cookies. Other origins, URL credentials/fragments and redirects
are refused. The bounded GET has a five-second deadline.

Poll the operation for its artifact ID, byte length and SHA-256. Reading it returns
metadata; saving its bytes uses the separately authorized artifact export above.
Keep the browser source owned and open while accessing that artifact. Exact retry
returns the original operation. A timeout/cancellation after dispatch can leave a
server-side effect, so `outcome_unknown` must not be treated as a safe new request.
This tool does not enable ordinary browser downloads or file pickers.

## Read browser logs

`lomi_browser_logs` requires the browser-read permission, the owned panel and its
current generation. Set `logKind` to `console` or `promise_rejection`; omit it
for the existing JavaScript-error stream. Reuse a cursor only for its original
kind and document. Navigation expires it. Each kind retains64 recent entries;
`dropped` and `hasMore` make gaps and pagination explicit.

Console reports five levels and primitive values. Promise reports preserve the
engine's `eventTrusted` flag, but WKWebView also reports false for genuine
rejections. Synthetic reports are included. Treat every message as untrusted
page content. Objects, stacks, network data and child-frame messages are omitted;
pages replacing console methods can bypass later collection.

## Add another project

Enable **Allow requesting access to new project folders** when pairing. A
`lomi_project_open` request identifies an approved anchor workspace, an absolute
folder, a new workspace name and the usual revision/retry fields. Review its
**Project folder request** in Settings: it shows the canonical folder, inherited
permissions, exact operation/request and expiry. Approval opens one blank editor
and adds access only to that new project/workspace; it starts no shell.

A connection can contain up to sixteen approved project roots. After opening,
use the returned new workspace ID for its resources. When retrieving an operation
by retry key, provide the project ID of its original anchor if the connection has
more than one approved project. Lookup by operation ID remains unambiguous.

## Work with results and permissions

Use explicit workspace/panel IDs and the generations returned by the tools.
Mutation inputs also bind the expected revision and a request key. Poll
`lomi_operation_get` until the operation has settled; receiving an operation ID
does not mean that the requested effect occurred.

Retry the same request with the same epoch, key and exact arguments to retrieve
its existing operation. A changed payload needs a fresh request only after its
earlier effects have been understood. An `outcome_unknown` result must not be
treated as permission to repeat the action. Cancellation does not undo a save,
Git operation, process start or other effect already performed.

Terminal execution runs with the host user's permissions. A project directory
is its starting directory, not an OS sandbox. Terminal control requires its own
permission and a current lease; human takeover revokes the lease. Transferring
an owned running terminal preserves the process and its operation history.

Browser permissions select allowed origins and separate read, input and
composite screenshot access. Browser pages cannot invoke the trusted main or
Settings commands. The native browser is not a Playwright-managed Chromium
instance, and an origin allowlist is not a network sandbox.

Android permissions select a managed device and separate runtime, input,
observation, screenshot and installation access. Opening its panel does not
start it. Input requires the visible selected panel in a focused Lomi window.
SDK setup and licenses remain separate from pairing permissions.

Dirty editor closures use the existing Save/Discard/Cancel dialog. Saving during
a workspace-close request keeps the workspace open and reports partial effects;
read the new state before requesting closure again. New text entered while a
native close is pending remains protected. Closing never deletes the project
directory. Protected origin terminals and busy or human-controlled resources
can prevent closing their workspace.

Git mutations require their concrete Lomi approval dialog, in addition to the
granted scopes. The dialog identifies the intended operation and the relevant
paths, revisions or remote/ref. Permission to read Git metadata does not grant
permission to execute repository hooks or helpers.

## Stop and recover

Use **Stop agent control** in Settings to revoke sessions and reject pending
requests. The native Lomi menu also offers stopping access if the workspace view
is unresponsive. Revocation does not assert that already running commands have
exited; inspect their actual terminal or device state before taking further
action. Disabling agent control also requires new configuration for a later
connection.

For an interrupted Trash operation, use **Settings → Agent control → Show
recovery folder**. An operation's `entry` folder contains retained data and its
`plan.json` records the original destination. A `completed.json` record means
publication to system Trash completed. Restore to an unused name so newer work
is not overwritten. Keep recovery records until their contents are reconciled.

| Result                                        | Next step                                                                                      |
| --------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `PAIRING_REQUIRED`                            | Match and approve the pending request in Settings.                                             |
| `SCOPE_DENIED`                                | Inspect the granted permissions; do not work around them through another resource.             |
| `TARGET_NOT_FOUND`                            | Refresh the authorized workspace/panel list; the ID may be closed, moved or outside the grant. |
| `STALE_GENERATION`                            | Inspect the current resource generation before any new mutation.                               |
| `REVISION_CONFLICT`                           | Inspect the operation's effect state, then refresh the relevant domain or buffer revision.     |
| `CONTROL_REVOKED`                             | Re-establish explicitly approved control if still wanted.                                      |
| `HOST_UNQUALIFIED` / `UNSUPPORTED_CAPABILITY` | Consult the qualification matrix; another successful tool does not qualify this path.          |
| `outcome_unknown`                             | Preserve the receipt and inspect actual resources. Do not automatically replay the effect.     |

## Local verification

`pnpm test:mcp` exercises the actual helper wire contracts. `pnpm test:mcp:control`
runs the opt-in isolated native fixture on its supported host and records its
artifact directory. That fixture is not part of a release build. Its result and
cleanup records must both be reviewed. Android is only included when its
explicit isolated fixture has been configured; a default native pass does not
constitute a new Android qualification.

Client routing and functional/safety/performance tests remain in the selected
scope. Installer distribution, packaged upgrades and release publication are
excluded from this delivery. Use the local development commands above.

## Set up and manage Android

In Agent control, enable Android reads and either setup or device management.
These permissions can start with no selected device. Existing devices still
need explicit selection; creating a device adds only that creator's new ID.
Runtime, input, APK installation and screenshot scopes remain separate.

- `lomi_android_setup_plan` with `action.type=inventory` reports installed
  packages, supported profiles, granted device configurations and recovery choices.
  `catalog` pages use `offset`, `limit` (1–32) and `expectedCatalogRevision` after
  page one; `refresh=true` is allowed only at offset0.
- `prepare` uses that `catalogRevision`, exact `packages:[{id,revision}]` and
  `prepareTools`. It returns a plan ID/revision, component sources/checksums,
  download sizes and required license digests. No SDK archive is downloaded yet.
- `lomi_android_setup_apply` takes that `planId`/`planRevision`, workspace,
  retry epoch and request key. Settings opens with the exact plan and full terms.
  Check each license yourself and approve. The original operation continues;
  poll `lomi_operation_get`, retaining the same key after uncertain completion.

`lomi_android_device_manage` uses a closed action: `create`, `modify`, `wipe`,
`delete`, `recover`, `cleanup`, `restore_metadata`, `remove_package` or
`rollback_package`. Each action requires its own Settings approval. Device
changes bind `expectedDevicesRevision` and, for an existing device, its exact ID
and `generation` (including null when none exists). Stop that device first.
Create/modify select an installed profile and bounded hardware configuration.
Modify preserves the image and data-partition size; changed hardware applies on
the next start. Changing the image requires a new device.

Wipe/delete include the exact device name in `confirmation`; you must separately
type it in Settings. SDK setup/recovery/cleanup/package maintenance need
`android.setup`; device changes need `android.manage`, all with `android.read`.
Recovery uses the inventory's exact malformed-file digest. Device metadata
requires a valid backup. Only preferences support reset, with typed
`RESET PREFERENCES` confirmation. The corrupt original is preserved. Package
maintenance uses `expectedManifestRevision` and refuses images used by any AVD.

Prepared download plans expire after30 minutes; Settings decisions expire after
10 minutes. Approved native work has a two-hour bound. Closing Settings does not
cancel it. Cancelling the MCP operation, revoking access or closing its workspace
invalidates only its own native work. Uncertain or partial completion is never
replayed automatically. Native progress remains visible in Android settings.

## Rearrange and close Android views

On qualified macOS ARM64 hosts, an agent can focus, dock, rearrange and transfer its running Android views using the
existing panel tools. The selected device needs `android.read` and
`android.control`; moves also need the existing workspace/panel permissions.
A transfer stays within one project and requires both workspace grants. The
same native process and retained viewport survive; input ownership is released
when the selected view changes or the view moves to another workspace. Focus
never starts a stopped or lazy device: use `lomi_android_start` explicitly.

Closing a shared view preserves the phone while another domain view exists.
Closing its last view, workspace or project requires `android.control`, passes
the existing editor guards, and waits for confirmed native Stop. Device files
and apps remain configured. A native admission barrier prevents a new start
until the close receipt settles. Cancelled guards preserve the running device;
a cancellation after native dispatch can leave it stopped with its views still
present. Failed Stop keeps the views and the owned process handle.

Native generation changes or human takeover invalidate queued layout authority.
Keep the same request key after uncertainty; a retry never repeats native Stop.
Operation reads and cancellation remain available while the native stop runs.

## Control selected Chat AI conversations

In **Settings → Agent control**, enable history access and select the exact
conversations to share. A project grant does not share its history. Opening,
creating, changing drafts, sending, stopping and exporting have separate
permissions. Native qualification uses a local test provider; no paid provider
was used to validate these tools.

| Tool               | Behavior                                                                            |
| ------------------ | ----------------------------------------------------------------------------------- |
| `lomi_chat_list`   | List metadata for selected conversations in the approved project.                   |
| `lomi_chat_open`   | Reuse an existing view or create a conversation with `target.type=new`.             |
| `lomi_chat_read`   | Read saved draft/message text and the exact active or last request identity.        |
| `lomi_chat_draft`  | Save draft text locally using the current draft, conversation and domain revisions. |
| `lomi_chat_send`   | Request sending the saved draft through its configured connection and model.        |
| `lomi_chat_stop`   | Stop one exact request ID and wait for its native saved terminal state.             |
| `lomi_chat_export` | Read a Markdown or JSON text export in revision-bound pages.                        |

Open an explicitly shared conversation, then read its draft. For sending, pass
`includeSendTarget=true` to the read call; this additionally requires send
permission. Use the returned connection/model and content revisions plus the
current domain revision. Every send displays an expiring approval in Lomi with
the actual draft, context, system instructions, attachments, model and possible
provider charges. The tool cannot change the configured provider. Approval is
always a human action in Lomi.

Mutation receipts use the normal retry epoch/key. A send receipt reserves its
request/message IDs before contacting the provider. `draftRevision=null` means
the request is only reserved; poll the operation to learn whether it was accepted.
After uncertain completion, keep the same retry key. Stream reconnection never
sends the message again. To stop, pass the exact `requestId` from the receipt or
history read. Retrying an old stop cannot stop a newer response.

Opening a chat already in a split layout selects that existing pane. All revealed
siblings need the appropriate permissions and ready runtimes. Docking, moving,
reordering and transferring a chat within an approved project preserve its live
response and draft. Closing a shared view preserves the response when another
view remains. Closing its last panel, workspace or project additionally requires
`chat.stop` for every affected selected conversation. Existing editor/process
guards run first, then drafts are saved and native responses are stopped and
checkpointed before removing views. Save failures retain the views; a failed
save attempt reports an uncertain effect, so keep the same retry key. No
replacement response can start until the close operation settles. These actions preserve
saved history and never delete a conversation.

Reads and exports contain persisted text, not unsaved typing or every live stream
chunk. They omit credentials, system instructions, provider metadata, reasoning
and attachment bytes. Export also omits attachment names, and explicitly reports
omitted content. It includes all saved message variants and the saved draft, with
limits of 512 messages and 4 MiB. For later pages, pass the initial `revision` as
`expectedRevision`, then concatenate `content` in order. The revision is the
SHA-256 of the complete UTF-8 export. Choose an output file through your client;
the export tool itself writes no file.

## Open a Settings section

Select **Allow opening Settings sections** when approving the connection. Call
`lomi_settings_open` with an approved live `workspaceId`, a page from its schema,
the current workspace-list `expectedRevision`, and your retry epoch/key. Poll
`lomi_operation_get`; `settings_opened.requested=true` confirms that the native
window accepted the request. Opening may focus Settings and does not change a
preference. The permission does not grant preference/history/credential reads or
writes. Repeat the exact arguments only to retrieve the existing receipt; use a
new request key for an intentional subsequent opening.

## Read application preferences

Approve **Allow reading nonsecret application preferences** independently from
opening Settings. `lomi_settings_read` accepts an approved live `workspaceId` and
`section`: `editor`, `terminal`, `keybinds` or `themes`. Values describe the current
main-window providers. Editor values are global defaults; terminal appearance
values are overrides; theme results omit CSS and paths. No credentials or chat
history are included. The grant covers these application-wide preferences.

For shortcuts, use `limit` (1–200), then `offset=nextOffset` and
`expectedRevision=revision` for the next page. A conflict requires a fresh first
page. `readiness=recovery_required` means the UI retained its last working values
after a settings error; the read does not repair the stored file. The revision is
a runtime snapshot identity and must not be treated as a stored-file write token.

## Request an editor preference change

Approve **Allow reading nonsecret application preferences** and **Allow requesting
application preference changes** when pairing. Read the `editor` section with
`lomi_settings_read`, then call `lomi_settings_update` with that snapshot's
`expectedSettingsRevision`, the current workspace domain `expectedRevision`, and
the connection's `retryEpoch` plus a fresh `requestKey`.

The current closed patches are `{"type":"editor_tab_size","value":8}` (1–16)
and `{"type":"editor_insert_spaces","value":false}`. They change application
editor defaults across projects. Buffer-specific indentation overrides remain.
Settings displays exact before/after values for **Apply change** or **Reject
change**. Each request expires after two minutes. Poll `lomi_operation_get` for
completion; a pending request does not hold the main workbench command queue.

A concurrent stored preference change fails without overwriting it. Invalid
preferences remain preserved for the existing human recovery controls. After a
conflict, read the new snapshot and use a new request key. Retrying the original
request returns its original receipt, including unknown outcomes. It never writes
again. This permission does not include credentials, approval policy or shell
profile code. Terminal fields and keyboard shortcut writes have passed native
qualification on macOS ARM64. Further theme/plugin MCP work is excluded from
this delivery; already implemented builtin theme choices retain their existing
native approvals.

Terminal preference requests use `{"type":"terminal_field","field":"appearance.fontSize","value":18}` with the `terminal` snapshot revision. The field enum is closed and includes appearance, colors, behavior and the existing data-only choices. `value` is required; explicit `null` restores theme inheritance for appearance leaves, for example `appearance.colors.red`. Other terminal fields do not accept null. Existing native ranges, color and string validators apply. A lower `behavior.scrollback` can trim older displayed lines, which the approval explains. Selecting the fixed Windows shell preference does not execute or edit a shell profile.

Keyboard patches use the current `keybinds` snapshot revision and an action ID
returned by that snapshot. `keybinding_set` takes `action` and a required
`shortcut` string; explicit `null` disables that action's shortcut.
`keybinding_reset` removes its stored override. `keybinds_focus_follows_pointer`
takes a boolean `value`. Invalid or conflicting shortcuts and changed action
definitions are rejected; each valid change still needs native Settings approval.

## Interact with a same-origin browser frame

On the qualified macOS ARM64 engine, `lomi_browser_snapshot` includes `frames`
with origin, URL and viewportRef, and each element identifies its frameId.
Use an element's exact snapshot-scoped reference with click/fill/key. To scroll a
frame, pass its viewportRef to `lomi_browser_scroll`; omitting it still scrolls
the main viewport. A fresh snapshot is required after child navigation or replacement.

The adapter traverses at most16 same-origin HTTP(S) frames, with at most4 nested
levels and the same total node/byte budget. Cross-origin, opaque, srcdoc, hidden
and over-budget frames are omitted. Input also requires a visible target through
all ancestor frames; transformed ancestors are currently refused. DOM events are
synthetic, and key events do not promise native default actions. Frame access
does not extend origins or bypass screenshot composite permission.
