# Isolated Android qualification fixture

State: VERIFIED for isolated SDK/device preparation on macOS ARM64. Explicit user consent was received on 2026-09-23 and recorded outside Git. This is not qualification of the public MCP Android adapter.

The existing SDK is `/Users/woro/Library/Android/sdk`; the existing `Pixel_10_Pro` AVD belongs to the user. Neither is the prepared managed Lomi fixture. The installed image is a preview track, so it does not replace the stable qualification image. Existing license marker files are not copied into a new acceptance record.

Disposable root: `/tmp/lomi-android-stage0-mcp-20260923` (canonical `/private/tmp/...`). Managed directory: `/tmp/lomi-android-stage0-mcp-20260923/native-managed-ca5c1615-3555-4833-a00f-e563569bf434`. Its `sdk/` and `avd/` contain the isolated resources. Device UUID: `c38f8ddd-0b24-4cae-81a1-fd9fcdd1e2e2`, name `MCP qualification`. It is stopped and retained for reuse.

The native managed installer completed with the previously qualified selections:

| Component                       | Version            | Download bytes |
| ------------------------------- | ------------------ | -------------: |
| Android direct CLI, macOS ARM64 | 1.0.16261425       |     83,918,832 |
| Private Temurin JRE             | 21.0.12.1+1        |     48,144,965 |
| Command-line tools              | 23.0               |    155,384,151 |
| Platform tools                  | 37.0.1             |     16,110,554 |
| Emulator                        | 37.1.11            |    394,555,844 |
| AOSP Android 16 ARM64 image     | API 36, revision 2 |    810,736,863 |

Total compressed download is approximately 1.51 GB. Preflight allowed approximately 12 GiB for unpacking, the isolated device and its test data. The completed managed directory occupies approximately 4.2 GiB. Device: Small Phone, 720×1280, 2 CPU, 2.5 GiB RAM, 6 GiB data, Host GPU, cold boot. No Google account is needed.

Provider terms: [Android SDK license](https://developer.android.com/studio/terms). The exact metadata license text is outside Git at `evidence/android-sdk-license.txt`, SHA-256 `1f8729233617b193fd619213792ae16a41b95d2bbbf525dfe66998252ba68b16`. Private Java uses the license bundled with the pinned toolchain. Consent must cover the actual displayed terms; changed hashes stop the trial.

The installer creates its own `native-managed-*` directory, private Android homes and Java. The managed actor creates the new device. Tests retain device/generation identities and use the private ADB transport; they never kill the user's shared ADB server. Successful test resources are stopped and their cleanup is recorded. Downloaded packages and user acceptance stay outside Git.

Executed tests:

- `android::installer::tests::actual_installer_prepares_clean_sdk_and_cancels_safely_during_exit`: PASS, 296.84 s, including cancellation/restart and verified publication. The first attempt correctly rejected changed bytes at Google's mutable `latest` URL. Switching only macOS ARM64 to the exact versioned URL retained the original hash and version.
- `android::mcp_qualification::prepare_and_verify_isolated_mcp_device`: PASS, 36.41 s. Production manager/actor booted a new device, installed the disposable APK, entered `Zażółć gęślą jaźń 🙂`, returned UI hierarchy and an authenticated native 720×1280 PNG, rejected input after lease release, and stopped the device/private ADB server.

Reports and consent are in `evidence/`, including `native-managed-installation.json`, `mcp-device.json`, `mcp-android.png` and `mcp-android-hierarchy.xml`. The PNG was visually inspected. Runtime records are empty after Stop. The pre-existing ADB process under `dev.simplebench.desktop` was left untouched.

To rerun the retained device fixture from the repository:

```sh
LOMI_ANDROID_PROBE_DIRECTORY=/tmp/lomi-android-stage0-mcp-20260923 cargo test --manifest-path src-tauri/Cargo.toml --locked --lib android::mcp_qualification::prepare_and_verify_isolated_mcp_device -- --ignored --nocapture
```

The fixture may disappear when the OS cleans temporary files. Do not recreate consent from an old report if terms change. This provision step does not qualify MCP artifact staging, public tools or the Tauri canvas.
