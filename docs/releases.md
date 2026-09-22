# Releases

Lomi follows Simple Voice's distribution targets: GitHub Releases for
macOS Apple Silicon and Intel, Windows x64, and Linux x86_64; then AUR packages
`lomi` and `lomi-bin`. Flathub is a separate, opt-in integration
after an initial submission has been accepted. Simple Voice's Flathub repository
did not exist when this workflow was prepared on 2026-09-11.

## Release workflow

`.github/workflows/release.yml` is named **publish** and follows Simple Voice's
tag-triggered `publish-tauri` matrix:

1. Start four parallel jobs on `macos-latest` (Apple Silicon), `macos-15-intel`,
   `ubuntu-22.04`, and `windows-latest`.
2. Install pnpm from `package.json`, Node LTS, Rust stable, and the platform's
   build dependencies. Validate the tag, app versions, and Apache-2.0 metadata.
3. Install frontend dependencies with the frozen lockfile and run
   `tauri-apps/tauri-action@v0` through `pnpm tauri`, using the locked Rust
   dependencies. Tauri's frontend build includes TypeScript checking.
4. Sign and notarize the macOS apps using the Apple secrets. Build the native
   installers, sign updater artifacts with the dedicated Tauri key, and upload
   them with `latest.json` to a draft `vX.Y.Z` GitHub release. Use
   `releases/vX.Y.Z.md` for the release description and updater notes when present.
5. After all four builds succeed, validate `latest.json` with
   `scripts/check-updater.mjs` and publish the complete release. Then publish `lomi` and `lomi-bin`
   to AUR. Retry temporary AUR failures up to three times; the **publish AUR**
   workflow can also be started independently for an existing release.
6. Notify Flathub only when `FLATHUB_TOKEN` is configured.

The release stays a draft if a platform fails or updater metadata is incomplete.
The previous stable release remains the updater endpoint until publication.
GitHub Actions only builds and distributes the application; test suites,
formatting checks and Clippy run locally. TypeScript checking remains part of
the frontend build, and release metadata and signatures are verified before
publication. Run the relevant local checks before tagging, as described in the
[development guide](../README.md#validation-and-builds).

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

1. Update the four app version entries together and prepare optional user-facing
   notes in `releases/vX.Y.Z.md`.
2. Run `pnpm release:check vX.Y.Z` and the checks appropriate to the changes.
3. Commit and push the release source to `main`.
4. Create and push the tag:

   ```sh
   git tag -a vX.Y.Z -m "Release vX.Y.Z"
   git push origin vX.Y.Z
   ```

5. Follow the **publish** workflow. Wait for all four platforms and AUR jobs to
   finish. Download and smoke-test the installers on the supported systems;
   successful CI builds do not establish native behavior on every operating
   system.
6. Release notes prepared before tagging are included in both the GitHub
   description and `latest.json`. Editing the GitHub description after publication
   does not automatically change the notes already stored in `latest.json`.

Retry failed jobs when the failure was an external service or credential issue.
For a source change after publication, use a new version. Never move a tag already
used by a published release. The first Apple notarization can take considerably
longer than later submissions; Tauri waits for Apple's response.

If GitHub publication succeeded but AUR failed, run **publish AUR** with the
same tag. It only accepts the latest stable release, avoiding an accidental
package downgrade. The `SKIP` checksums in the upstream PKGBUILD templates are
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
`NODE-LICENSE` and `THIRD-PARTY-NOTICES`. Update the artifact manifest and run
native probes when changing Node or SDK versions. Supported packaging targets
are macOS ARM64/x64, Windows x64 and Linux x64; generation requires macOS 13.5
or later. Older macOS can still open locally stored history.

macOS signs the nested Node executable with the hardened runtime and the
`com.apple.security.cs.allow-jit` entitlement. Do not add unsigned executable
memory or disable library validation. Developer ID signing, notarization,
Gatekeeper and updater replacement must be tested on real release artifacts;
an ad-hoc signature is only a local packaging test. Never ship `chat-probe` or
`fixture.cjs` in a release. See the [Chat AI guide](chat-ai.md) for the current
native validation scope.

Linux system-key storage requires an unlocked Secret Service implementation,
such as GNOME Keyring or KWallet's Secret Service support. Session-only storage
is an explicit user choice, not an automatic fallback when the service fails.
