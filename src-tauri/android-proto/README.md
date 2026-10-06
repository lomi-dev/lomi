# Android protocol provenance

`emulator_controller.proto` is copied without modifications from
`emulator/lib/emulator_controller.proto` in Google's macOS ARM64 Android
Emulator **37.1.11**, build **15917651**:

<https://dl.google.com/android/repository/emulator-darwin_aarch64-15917651.zip>

The official `repository2-3.xml` advertised archive size 394555844 bytes and
SHA-1 `f22f44948a2b7f0a0103645b9a639290eef92426`; both were checked before
extraction. The extracted protocol's SHA-256 is
`1d62c6bcad5f06621f90ec2bf26c661ba769ccd0f1416b5314d25a68e04eee5f`.
The upstream Apache-2.0 copyright and license notice remains in the file.
The repository's root `LICENSE` contains the license text.

The application and opt-in native fixture use this client. Generation uses
vendored protoc at build time without downloading protocol definitions. Native
qualification currently covers only the recorded Apple M3/macOS ARM64 host.
Other platforms remain unverified; see [native verification](../../tests/native/ANDROID.md)
for isolated fixture setup and qualification limits.

The RTC availability probe sends an empty request to
`/android.emulation.control.Rtc/requestRtcStream`, the first RPC in the
[upstream experimental RTC contract](https://android.googlesource.com/platform/prebuilts/android-emulator/+/master/linux-x86_64/lib/rtc_service.proto).
This service is absent from the tested binary (`UNIMPLEMENTED`); no RTC protocol
is vendored, no frontend bridge is treated as an encoder, and no RTC dependency
is added.
