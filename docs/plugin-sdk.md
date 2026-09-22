# Standalone plugin SDK

The public SDK source is in [lomi-dev/plugin-sdk](https://github.com/lomi-dev/plugin-sdk).
This application consumes `@lomi-dev/plugin-sdk` as an external dependency.
Public types are imported from the package root; runtime validators use the pure
`/manifest` and `/shortcuts` exports. The application's React context and host
implementation stay in `src/plugins`. There is no local SDK source package.

The application pins the rebrand candidate SDK 1.1.0-alpha.1. Publish the tested
archive before installing that version from npm in a clean checkout. During
local qualification, use the candidate tarball override in a disposable checkout
as described in the SDK's consumer guide. A candidate archive test does not
establish npm publication or verify registry installation.

The workspace exempts this exact SDK version from pnpm's minimum release age.
Do not substitute a floating range or Git branch.

Vite bundles the validators needed by the host. The installed application does
not fetch SDK code from npm or GitHub. Build helpers and Node-only package tools
are authoring dependencies and are not imported into the browser entry points.

## Upgrade procedure

1. Review the SDK's release notes, host API requirements and artifact integrity.
   Update the root dependency and `tests/fixtures/context-plugin/package.json`
   together. Run `pnpm install` and review the lockfile diff.
2. Run `pnpm sdk:verify`. If the contract fixture changed intentionally, review
   it and run `pnpm sdk:sync-contract`. The SDK owns the canonical cases; Cargo
   uses the committed snapshot so Rust-only builds do not read node_modules.
3. Run `pnpm check`, `pnpm test`, `pnpm sdk:test:consumer`, `pnpm build`,
   `cargo test --manifest-path src-tauri/Cargo.toml --locked plugins::tests`, and
   plugin UI tests. The `lomi.plugin-api.v1` runtime symbol must match the installed SDK.
4. Qualify the installed host before changing the supported-version matrix.
   Mocked browser tests do not qualify native webview engines.

SDK CI installs candidate tarballs in disposable consumer checkouts using an
explicit `LOMI_SDK_TARBALL` override. Normal application CI requires a qualified
external pin and verifies the snapshot byte-for-byte, including on Windows.

For rollback, revert the dependency, lockfile and fixture snapshot together.
Keep the previous SDK release available and rerun the same checks. The rebrand intentionally changes runtime identity; previously built plugins
require rebuilding with the Lomi SDK.
