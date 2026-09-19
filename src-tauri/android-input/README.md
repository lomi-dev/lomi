# Managed Android keyboard

SimpleBench selects this small, local IME inside its managed Android devices to
deliver Unicode and composition through Android's `InputConnection`. Physical
keys and touch use the emulator's authenticated gRPC service. Explicit Paste
sets Android’s actual clipboard and invokes its Paste action through the selected
IME. Other text uses InputConnection directly and never changes the clipboard.

The IME has no network permission, host clipboard listener, telemetry or file
storage. Its local socket accepts only the guest shell/root forwarding process,
checks the device and private instance nonce, bounds each UTF-8 message and
waits for the main-thread edit before acknowledging it. Closing a host view
does not uninstall the keyboard or erase phone data.

`simplebench-input.apk` is built from the Java, manifest and XML in this folder.
`artifact.json` records their SHA-256 digests, the APK digest, the signing
certificate fingerprint and the actual build-tool versions. A Rust test rejects
source/artifact mismatches; runtime checks the bundled APK digest before sending
it to the owned guest. A matching installed APK is reused across boots. After installation, native
startup waits up to 15 seconds for Android to list the keyboard before enabling
and selecting it. Package installation can finish before the input-method
registry processes the package change. This wait is cancellable and preserves
the same-transport device and generation checks; selection still requires an
authenticated readiness response from the keyboard.

Git attributes preserve the verified source bytes even when a Windows checkout
enables automatic line-ending conversion. Keep those attributes with this folder.

The desktop executable embeds the APK. End users do not need a JDK, build tools,
Node, Python or Android Studio to build this component. Android's private JRE is
used separately for device creation, as documented in [the architecture guide](../../docs/android-architecture.md).

Maintainers can rebuild with `build.mjs`, passing a JDK home, `android.jar`,
build-tools directory, isolated output directory, private keystore and key alias
in that order. Supply `SIMPLEBENCH_INPUT_KEY_PASSWORD` through the environment.
The script calls Java directly for D8 and signing; it does not download tools,
create signing keys or change user configuration. The keystore and password must
stay outside the repository. Preserve the same signing key when updating the
package in existing devices; never use the native fixture's public test key.

The version 3 APK uses a persistent private development key outside Git, under
`~/Library/Application Support/SimpleBench Development/android-signing/` on the
recorded Mac. Its keystore and password are separate owner-only files inside an
owner-only directory. Do not copy either into build logs or the repository.
The earlier unpublished version 2 key was lost when the host restarted and
cleared the isolated `/tmp` trial. Version 3 therefore cannot update that old
signature in place; preserve existing phone data and explicitly resolve such
a development device before installing it. The native trials use new AVDs.
Before a public release, back up the current key in the project's approved
signing storage and test upgrades with the release packaging process.
