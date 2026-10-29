# Releases

Lomi follows Simple Voice's distribution targets: GitHub Releases for
macOS Apple Silicon and Intel, Windows x64, and Linux x86_64; then AUR packages
`lomi` and `lomi-bin`. Flathub is a separate, opt-in integration
after an initial submission has been accepted. Simple Voice's Flathub repository
did not exist when this workflow was prepared on 2026-09-11.

## Release workflow

### Canonical source and host configuration

Desktop release builds require the canonical host and Remote UI repositories as
siblings of each desktop checkout. Configure these repository variables before
using the release workflow:

- `LOMI_REMOTE_CORE_REPOSITORY`: the canonical host repository as
  `owner/repository`. Set this to the established repository location; the
  workflow does not assume an unpublished repository name.
- `LOMI_REMOTE_CORE_SHA`: a full 40- or 64-hex commit SHA from that repository.
- `REMOTE_WEB_SHA`: a full 40- or 64-hex commit SHA from `lomi-dev/remote-web`.
- `LOMI_HOST_IDENTITY_ORIGIN`, `LOMI_HOST_REMOTE_API_ORIGIN`,
  `LOMI_HOST_RELAY_ENDPOINT`, and `LOMI_HOST_CLIENT_ID`: the public host
  authentication bootstrap pins required by release builds.

Before enabling desktop release builds, publish the canonical host source to
its own Git repository and set its actual owner/repository and full commit SHA.
This workflow only checks out configured immutable revisions; it does not
create or publish the canonical source repository.

The checkout action rejects missing or unpinned refs, verifies each checked-out
commit, and places the sources beside the desktop checkout without replacing an
existing sibling. The desktop repository's `GITHUB_TOKEN` is used by default.
If the source repositories are private and that token cannot read them, set the
existing `CANONICAL_SOURCE_TOKEN` repository secret to a token with read access
to those repositories. The four host bootstrap values are public application
configuration, not signing credentials. `LOMI_HOST_CLIENT_ID` identifies host
enrollment; the desktop OAuth consumer client remains separate.

### Publishing

Pushing a version tag runs `.github/workflows/release.yml`, which builds and
signs platform assets into a GitHub draft. It does not publish the draft, AUR
packages, or a Flathub update. After release qualification, upload the reviewed
canonical whole-release manifest as the fixed `release-set.json` draft asset,
alongside every artifact declared by all seven components. This includes Host
bundles and declared installer metadata, Web bundles/WASM, signed Mobile
packages/native libraries, exported OCI container archives and Desktop
bundles/update metadata. Collect these reviewed files from their native build
pipelines; this publisher does not build, extract or deploy them. Each asset
must have a unique declared size/SHA-256 binding; ambiguous duplicate payload
declarations and extra files are rejected.

Configure the protected source pins before dispatch:

- `LOMI_REMOTE_CORE_REPOSITORY`, `LOMI_REMOTE_CORE_SHA` and `REMOTE_WEB_SHA`
  identify the canonical Host and Web commits as before.
- `LOMI_AUTH_REPOSITORY` and `LOMI_AUTH_SHA` identify the account service
  repository and full immutable commit. API, Identity and Relay must declare
  that same commit; their actual source tree is checked for each component.
- `LOMI_MOBILE_REPOSITORY` and `LOMI_MOBILE_SHA` identify the Mobile repository
  and full immutable commit.

Repository values use `owner/repository`; commit pins are full lowercase
40- or 64-hex Git object IDs. The existing `CANONICAL_SOURCE_TOKEN` must have
read access to the selected source repositories. Checkout credentials are not
persisted in those worktrees.

```sh
gh release upload vX.Y.Z ./release-set.json
sha256sum ./release-set.json
```

Then dispatch `.github/workflows/publish-coordinated-release.yml` from the
protected default branch with the exact draft tag and the lowercase SHA-256 of
those manifest bytes. It validates the
manifest against all seven pinned source trees with
`--require-complete-mappings`, uniquely binds every declared product artifact by
size and SHA-256, and requires the manifest
to cover the complete draft asset set other than itself. It binds `latest.json`
to its declared metadata digest, checks each updater URL against a bundle for
the right platform, and matches each updater signature to its declared `.sig`
asset. Container, Host, Web and Mobile signature files retain their own
component bindings and are not interpreted as Desktop updater metadata.
These checks run again after production approval using the already validated
source commits and the exact approved manifest digest. A missing, extra,
ambiguous, changed, or mismatched asset leaves the release unpublished. The
manifest validator checks recorded qualification and signature declarations;
it does not independently verify their evidence. Configure the GitHub
`production` environment with required reviewers and protected-branch
restrictions before enabling publication. AUR publication uses the manifest
verified DEB or a source archive pinned to the validated Desktop commit. AUR
and the optional Flathub notification run only after this explicit publisher
succeeds.

The README downloads badge counts release packages only. Every six hours, or
when started manually, `.github/workflows/downloads-badge.yml` sums release
assets except `.json` and `.sig` files, because the updater fetches
`latest.json` on every launch. It force-pushes the resulting Shields endpoint
file as the only commit on the `badges` branch; keep that branch.

Lomi selects Xcode 26.3 to compile its Icon Composer source and uses its
own pnpm version. It does not need Simple Voice's audio, Vulkan, or ONNX build
dependencies. Windows installers are not Authenticode-signed. Tauri updater
signatures verify downloaded updates independently of Apple signing and Windows
Authenticode. macOS app archives are still published as downloads.

## In-app updates

Like Simple Voice, Lomi checks for updates three seconds after the
workspace initializes. **Settings → About → Check for updates** requests a
manual check in the main window. Automatic network errors stay quiet; manual
checks report errors or confirm that the installed version is current. Checks
use a 15-second timeout and Tauri's semantic version comparison, without downgrades.
Versions shipped before this updater was added need one manual installation of
an updater-enabled release before they can receive automatic update checks.

Windows and macOS show the version and release notes, then download and verify
the signed package when the user chooses **Update now**. Downloads show progress
(indeterminate when the server omits a length). Native connection and read
timeouts are 30 seconds; each received chunk resets the read timeout. Downloads
have no total time limit, so a slow but active transfer can finish. Metadata
checks retain their separate 15-second total limit. Failed downloads can be
retried, and the error dialog offers GitHub Releases for manual installation.
Before installation, the existing
close guard checks running processes and offers save/discard/cancel for dirty
editors, then stops chat generations and flushes their drafts and responses.
Failed editor or chat saves, or cancellation, prevent installation. The session is saved
before PTYs are stopped and installation starts. Windows' installer relaunches
the application; macOS requests a restart after replacing the app bundle.
Terminals restart as fresh shells, with no command or output replay.

Versions 0.3.0 and 0.4.0 impose a two-minute total download timeout. A timeout
while reading the installer can appear as `error decoding response body`;
the same message can also describe other interrupted response bodies. If an
affected installation cannot finish its update, download the matching Windows
installer (`x64-setup.exe` for NSIS, or `.msi` for MSI) from GitHub Releases,
close Lomi normally, and run the installer. The timeout fix applies once
a release containing it is installed.

Linux always shows external update instructions, including for AppImages:

- Flatpak: `flatpak update` (detected at runtime, before checking the host distro).
- Arch and derivatives: `yay -Syu lomi-bin`, or `yay -Syu lomi`
  when the source package is installed. Users of another AUR helper can use its
  equivalent command; pacman alone does not build AUR packages.
- Other distributions: download the latest package from GitHub Releases and
  reinstall it with the distribution's package manager, or replace the AppImage.

The dialog can copy the instructions or open GitHub Releases. It never runs
package-manager commands. Flatpak detection does not imply a published Flathub
package; the Flathub prerequisites below still apply. Installation permissions
exist only for the main application webview on Windows and macOS. Settings can
request a check but cannot download/install updates, and embedded browser pages
have no updater access.

## Updater signing key

The public key is stored in `src-tauri/tauri.conf.json`. Keep the corresponding
private key and password outside the repository and back them up securely.
Losing or replacing this key prevents existing installations from accepting
future updates. The implementation's initial key files are in
`~/.config/lomi/updater/` (`lomi.key`, `lomi.key.pub`, and
`lomi.password`), with access restricted to the local user.

Configure these repository Actions secrets, passing file contents via stdin:

```sh
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo lomi-dev/lomi < ~/.config/lomi/updater/lomi.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo lomi-dev/lomi < ~/.config/lomi/updater/lomi.password
```

For a new installation of this release infrastructure only, generate a key using
`pnpm tauri signer generate -w /safe/path/lomi.key` and put its public key
contents in `plugins.updater.pubkey`. Do not regenerate a deployed updater key.
For local bundle builds, set `TAURI_SIGNING_PRIVATE_KEY` to the key path and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` to its password in the build environment.
Development runs and `--no-bundle` builds do not need signing secrets.

The workflow enables `bundle.createUpdaterArtifacts` and
`includeUpdaterJson`, producing signed macOS app archives, Windows installers
(NSIS preferred in updater metadata), and the Linux AppImage metadata used for
version notifications. The endpoint is the public GitHub release asset:
`https://github.com/lomi-dev/lomi/releases/latest/download/latest.json`.
No GitHub token is embedded in the app. See the
[Tauri updater documentation](https://v2.tauri.app/plugin/updater/) and
[tauri-action v0 inputs](https://github.com/tauri-apps/tauri-action/blob/v0/action.yml).

## Apple Developer configuration

Use a **Developer ID Application** certificate for direct distribution. The
workflow uses the [Tauri macOS signing and notarization flow](https://v2.tauri.app/distribute/sign/macos/):
the action imports the certificate into a temporary runner keychain, and Tauri
signs with the Hardened Runtime, submits to Apple, waits, and staples the ticket.
While the release remains a draft, the workflow separately submits each signed
DMG to Apple, requires an accepted receipt, and staples and validates its ticket.
It checks the DMG signature and Gatekeeper assessment before replacing the draft
download. Both macOS jobs must pass before the release becomes public, so the app
and its DMG are notarized and stapled before publication.
No App Store sandbox or microphone entitlements are needed for Lomi.

Configure these repository Actions secrets:

| Secret                       | Value                                                                             |
| ---------------------------- | --------------------------------------------------------------------------------- |
| `APPLE_CERTIFICATE`          | Base64-encoded `.p12` containing the Developer ID certificate and its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting that `.p12` export                                            |
| `APPLE_SIGNING_IDENTITY`     | Full `Developer ID Application: ...` identity                                     |
| `APPLE_ID`                   | Apple account used for notarization                                               |
| `APPLE_PASSWORD`             | An Apple app-specific password, not the normal account password                   |
| `APPLE_TEAM_ID`              | Apple Developer team identifier                                                   |
| `AUR_SSH_PRIVATE_KEY`        | Unencrypted dedicated SSH private key registered with the AUR maintainer account  |

Repository secrets are configured at
[Lomi Actions secrets](https://github.com/lomi-dev/lomi/settings/secrets/actions).
The corresponding secrets in Simple Voice are not automatically available to
Lomi. Keep their values out of source files, commits, and logs.

For a local signed and notarized build, make the same Apple account variables
available in the shell and install the Developer ID identity in Keychain Access,
then run `pnpm tauri build --bundles app,dmg -- --locked`. A locally installed
identity does not require `APPLE_CERTIFICATE` or its export password. Omitting
the notarization credentials can produce a signed app without notarization;
configure all listed Apple secrets before starting a release.

## Publish a version

Lomi uses Apache-2.0, with the license text in [`LICENSE`](../LICENSE)
and its SPDX identifier in `package.json` and `src-tauri/Cargo.toml`. The workflow
requires matching license identifiers and the license file before publication.
Both AUR recipes declare the same license.

Remote services and the browser client have separate deployments. A desktop tag
does not update them. Before tagging a release that changes Remote behavior:

- Deploy compatible identity exchange and Remote API versions together. Apply
  `auth-app/ops/remote-database-permissions.sql` after migrations, including the
  five column UPDATE permissions needed for authenticated session renewal.
  Run `auth-app/ops/remote-db-inspect.ts` using the owner inspection connection;
  its read-only receipt checks both schema and required renewal permissions.
  Deploy an API that supports `GET /v1/remote/native/hosts` and conditional
  heartbeats with `expectedLastSeen` before updating the desktop. A delayed
  heartbeat must return `409 PUBLICATION_CHANGED` after a newer publication,
  preserving the currently shared workspace and its grants.
- Build the canonical sibling `remote-web` repository and deploy its reviewed
  static output separately. Keep browser assets and backend/API releases
  coordinated so each client matches the deployed Remote contract.
- Check an authenticated production connection after deployment: workspace
  discovery, refresh, control, renewal and Stop. Public health endpoints alone
  do not prove these flows.

Remote hosting is currently qualified only on macOS Apple Silicon. The helper's
32-terminal limit counts all active desktop terminals, including unshared ones.
Window Close hides Lomi and preserves Remote; explicit Quit ends its sessions.

1. Update the four app version entries together and prepare optional user-facing
   notes in `releases/vX.Y.Z.md`.
2. Run `pnpm release:check vX.Y.Z` and the checks appropriate to the changes.
3. Commit and push the release source to `main`.
4. Create and push the tag:

   ```sh
   git tag -a vX.Y.Z -m "Release vX.Y.Z"
   git push origin vX.Y.Z
   ```

5. Follow **Prepare signed draft**. Wait for all four platform builds and
   notarization checks. The tag workflow leaves the release as a draft.
6. Complete release qualification, upload the canonical `release-set.json`
   described above, and record its exact lowercase SHA-256. Dispatch
   **Publish coordinated release** with that digest and tag. After the protected
   `production` environment approval, the workflow rechecks the manifest and
   updater metadata before publication; AUR and optional Flathub work follows.
   Download and smoke-test the installers on supported systems: successful CI
   builds do not establish native behavior on every operating system.
7. Release notes prepared before tagging are included in both the GitHub
   description and `latest.json`. Editing the GitHub description after
   publication does not automatically update the notes stored in `latest.json`.

Retry failed jobs when the failure was an external service or credential issue.
For a source change after publication, use a new version. Never move a tag already
used by a published release. The first Apple notarization can take considerably
longer than later submissions; Tauri waits for Apple's response.

If GitHub publication succeeded but AUR failed, rerun the failed `publish-aur`
jobs in the **Publish coordinated release** run. They recheck the approved
manifest and DEB digest, and use the validated Desktop commit for source
archives. They only accept the latest stable release, avoiding an accidental
package downgrade. The `SKIP` checksums in the protected PKGBUILD templates are
replaced before anything is sent to AUR; published package recipes contain the
real SHA-256 values. Arch's standard package hooks maintain desktop and icon
caches without custom install scripts.

## Flathub

Flathub is not an automatic first-release destination: its
[initial submission process](https://docs.flathub.org/docs/for-app-authors/submission/)
requires a Flatpak manifest, upstream application metadata, a tested sandbox,
and review. A terminal application must also design and verify host-shell and
project-folder access before claiming Flatpak support.

After a dedicated `flathub/dev.lomi.desktop` repository and
its maintenance workflow exist, configure `FLATHUB_TOKEN` with access to that
repository. Successful releases then dispatch `lomi-release` with
`client_payload.tag` and `client_payload.commit`, matching Simple Voice. Leave
the token unset until the receiving workflow is ready; this hook alone does not
publish an application on Flathub.

## Bundled Chat AI runtime

`pnpm tauri dev`, normal builds and Cargo's build script prepare the pinned
Node archive and AI SDK bundle through `scripts/prepare-ai-runtime.mjs`.
The build downloads only the manifest-pinned official Node archive, verifies
SHA-256 and caches it in `src-tauri/target/ai-runtime`. End users never download
or discover a runtime. Cross-target builds select `TAURI_ENV_TARGET_TRIPLE`.
AUR source builds need network access during preparation; binary packages
include the runtime from the application bundle.

Ship the Node executable, `ai-runtime/index.cjs`, `node.json`, bundle checksum,
`NODE-LICENSE` and `THIRD-PARTY-NOTICES`. Update the artifact manifest when
changing Node or SDK versions. Supported packaging targets
are macOS ARM64/x64, Windows x64 and Linux x64; generation requires macOS 13.5
or later. Older macOS can still open locally stored history.

macOS signs the nested Node executable with the hardened runtime and the
`com.apple.security.cs.allow-jit` entitlement. Do not add unsigned executable
memory or disable library validation. Developer ID signing, notarization,
Gatekeeper and updater replacement require the configured release identity.

Linux system-key storage requires an unlocked Secret Service implementation,
such as GNOME Keyring or KWallet's Secret Service support. Session-only storage
is an explicit user choice, not an automatic fallback when the service fails.
