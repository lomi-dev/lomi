# Native task desktop smoke

On macOS, from the repository root:

```sh
node tests/native/agent-runtime-desktop.mjs
```

The runner builds current frontend assets and a debug `native-smoke` executable.
Its isolated Tauri config clears the development URL and uses identifier
`dev.lomi.agent-runtime-production-smoke-20261006`. Each fixture uses a fresh
private temporary home and database. The supervisor runs as the normal
unsandboxed Lomi executable: launchd refuses bootstrap from a sandboxed caller.
Native children retain their production sandbox; Claude's Collector has no
outbound grant. Account and credential data are synthetic; real account
credentials, paid requests and CLI inference are not used. Activation targets only the directly spawned application PID;
screenshots target only its native main window. The probe never sends global
keyboard input or inspects another application's accessibility tree.

The passing positive fixture includes an `owned-production-entry` probe that
runs the normal application executable through its early host-worker and
native-runner entry points. It verifies a held `/usr/bin/printf` checkpoint and
owned PTY retirement, including delayed durable helper finalization. Pinned
Claude `auth status` reports false/true/false with exits 1/0/1 for absent,
synthetic and then deleted private file credentials. Keychain and outbound
network access remain denied. The final rerun uses the production stable private
temporary alias. These checks exercise the production file
credential selector with synthetic data and inert programs; they do not
authenticate a real account or invoke CLI inference. The runner requires
`owned-production-entry.json` to report success before accepting the positive
fixture.

The positive fixture also restores two views of one synthetic archived task in real
WebKit, reloads, docks one through the command picker, closes the first view
without draining, and closes the final view with one completed native drain and
release. It then requests normal application quit through the production
`request_quit` command. Passing requires native prepare, session save, and drain
in that order, followed by the application exiting with code zero.

The retention fixture adds a synthetic same-boot unresolved ownership record
without a PID or process group. Final-view close and normal quit must fail while
the final task view and saved reference remain. The runner terminates only this
owned fixture process after recording the refusal; that cleanup is not counted
as successful application quit. These desktop fixtures do not qualify real
account authentication, provider compatibility, continuation or full G02.

The printed artifact directory contains `result.json`, completed native IPC
response records, fresh sessions/databases, activation logs, native logs and
own-window screenshots. `--no-build` reuses a previously built probe binary;
the renderer rejects an executable with a different application identifier.
`LOMI_AGENT_RUNTIME_SMOKE_BINARY` can select that previously built executable.
Qualification features are rejected in release builds by existing native guards.

The desktop supervisor runs with normal unsandboxed launchd access. An outer
sandbox that blocks `launchctl bootstrap` prevents Held admission and fails
closed. Native worker/runner children still receive the production restrictive
sandbox. The managed deny-list bound is 256 to accommodate inherited paths from
deep canonical macOS temporary roots; typed grant bounds remain unchanged.

The native proxy reserves IPv4 `127.0.0.1` and IPv6 `::1` on the same approved
port before admission, authenticates before DNS, and retains that reservation
through cancellation and failed Stop until sealed cohort retirement and durable
publication. Reservation after an application-supervisor crash remains
unqualified; recovery retains uncertain effects. Loopback fixtures establish
that `127.0.0.2` and unrelated IPv4/IPv6 ports remain denied.

## Production Claude file credential selector

Normal builds handle `--agent-runtime-host-file-keychain` in the early
`file_credentials` entry point. Managed Claude receives a trusted fixed PATH
whose `security` shim invokes that staged entry, rather than the system Keychain
client. The dispatcher returns 44 for `find-generic-password` and
`delete-generic-password`, and 1 for `add-generic-password` and interactive `-i`.
It bounds and drains `-i` input without parsing, storing or echoing it.

Those results select Claude 2.1.287's original OAuth private-file fallback and
CAS refresh behavior. The shim does not implement credential storage itself or
grant Keychain access. User-console, device and API-key-only storage flows remain
unsupported; real OAuth account qualification remains separate.

Managed Claude uses a stable private mode-0700 wrapper directory at
`/private/tmp/la<16-hex-digest>`, whose `t` symlink points to the existing
canonical account `.native-tmp`. The wrapper is admitted readonly through
literal and subtree runtime reads; writes remain confined to the same physical
account temporary root. Admission requires matching real/effective UIDs and a
per-UID Claude child path of at most 44 bytes. Held release rechecks wrapper
ownership/mode, directory and link identity, and the physical target inode;
wrong targets or extra wrapper entries fail closed. The alias persists for
account history. Version probes use their transient private directory without a
persistent alias. No shared `/tmp` writable grant is added.

## Source-derived Claude OAuth file storage

Run the storage fixture with Node 22.22.3 and an explicitly supplied owned public
Claude 2.1.287 macOS artifact:

```sh
node tests/native/claude-oauth-file-store.cjs /absolute/path/to/owned/public/claude
```

The script checks the full artifact SHA-256
`6eab8333fe2121553100d8f40bfada384a3e989b94f947e18ba6677a6fcb41ea`
and four extracted module hashes, then evaluates only the original storage,
cache, atomic file writer and OAuth save/refresh functions. A fresh private
temporary directory contains synthetic credentials; the security adapter runs
only an owned Node helper with synthetic exit codes. Its assertions cover save,
read, refresh CAS, stale refresh adoption, mode 0600, unknown-read refusal,
timeout and locked-primary preservation, stdin/argv helper contracts and logout
deletion. The fixture removes its temporary credential directory afterward.

All ten source-derived assertions passed with the pinned artifact. The JSON
result sets `sourceExtractionNotFullCLIQualification: true`. These
checks establish source-derived storage behavior, without authenticating an
account or qualifying full CLI OAuth, HTTP, Keychain, provider access or G02.

## Source-derived Claude temporary-directory selection

```sh
node tests/native/claude-temp-directory.cjs /absolute/path/to/owned/public/claude
```

With Node 22.22.3 on macOS, this fixture verifies the same full pinned artifact
hash plus the original temporary-directory utility and `nEe`/`UYo` function
hashes. It evaluates those utilities without CLI initialization, accounts or
network. Five assertions passed: the private alias retains correct ownership
and writes into the canonical physical root; original `UYo` preserves its
33-byte base; original `nEe` preserves its at-most-44-byte per-UID child path
without shared-temp delegation; a long base exposes `UYo`'s `/tmp` fallback;
and original `nEe` catches the fixture's denied shared-temp write and memoizes
its private fallback. All temporary fixture paths are removed. This source
acceptance does not qualify a full native Bash turn or account authentication.

The same script supports `--held <canonical-physical-temp>` after the artifact
argument for the owned native sandbox harness. It uses the harness's trusted
`CLAUDE_CODE_TMPDIR` alias and real filesystem calls, creates no wrapper, and
checks private writes plus kernel denial of alias removal/replacement and
unrelated/shared temporary writes. All eight checks passed under the actual
Held native policy, including denial of reads and writes through an alias
symlink to an owned marker outside the physical temporary root; that marker
remained unchanged. The stable-alias/replaced-target unit fixture also passed.
This mode requires the harness's restrictive policy; it is not an unsandboxed
standalone command. These results and the final desktop rerun establish the
private temporary-directory contract, while real-account native Bash-turn
qualification remains separate.
