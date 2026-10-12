# Android G1/R04 source prototype

This feature-only Rust prototype uses one existing managed DeviceRuntime
process generation, one authenticated `streamScreenshot`, one immutable latest
RGBA frame, and a shared quality-70 JPEG encoder capped at 10 FPS.
JPEG output uses a capped writer with a hard 512 KiB payload budget; oversized
frames fail the prototype instead of growing the output buffer. Adaptive
quality/size reduction remains unqualified. Its local
memory sink and two remote JPEG memory sinks have independent epochs, pending
ACKs, two-second deadlines, and latest-only waiting slots. It has no remote
permission, network, encryption, or production Tauri Channel implementation.
Frame retrieval, ACK and removal require the source fence and the exact epoch
returned by join. An old poller cannot consume a same-ID replacement's frame.
Production `Streams` is unchanged. Do not instantiate both owners for the same
running device during a native trial.

Run the deterministic source-state tests:

```sh
cargo test --lib --manifest-path src-tauri/Cargo.toml --locked \
  --features remote-visual-probe android::remote_probe
```

These tests publish fake frames. They prove state-machine properties, not a
working native emulator capture.

The ignored native test requires the previously accepted canonical isolated
stage-0 fixture and its managed-installation evidence. It starts a configured
isolated device, opens exactly one screenshot stream, checks actual stream
receipts and all three memory sinks, hides local demand, and checks a new remote
receipt without changing the process generation. It stops only that isolated
device during cleanup. It never creates, deletes, wipes, installs, or accepts an
SDK license. Do not point this test at ordinary SDKs or the user's devices.

```sh
LOMI_ANDROID_PROBE_DIRECTORY=/private/tmp/lomi-android-stage0-... \
  cargo test --manifest-path src-tauri/Cargo.toml --locked \
  --features remote-visual-probe \
  android::remote_probe::tests::native_one_stream_three_memory_sinks_hide_keeps_remote_generation \
  -- --ignored --exact --nocapture
```

Evidence is written to `evidence/native-remote-source-prototype.json` only after
a successful real test. Running these focused tests does not qualify a 30-minute
native run, production rendering, native minimize wiring, signed remote grants,
relay transport, or a browser receiver. The 10-second idle probe period,
three-second unary deadline, and two-second ACK deadline are prototype values,
not measured production freshness budgets. At most one unary request and one
JPEG encode are in flight; late unary results use a native receipt watermark,
never guest sequence comparison (`getScreenshot` sequence is always zero).

For an explicitly authorized nondestructive observation of the currently
running macOS arm64 Lomi emulator, first focus a non-Android panel so the
production local-only screenshot stream has ended. Then use the exact native
device UUID and current process generation returned by Lomi:

```sh
LOMI_REMOTE_ANDROID_ATTACH=observe-current-generation \
LOMI_REMOTE_ANDROID_DEVICE=<current-device-uuid> \
LOMI_REMOTE_ANDROID_GENERATION=<current-running-generation-uuid> \
  cargo test --manifest-path src-tauri/Cargo.toml --locked \
  --features remote-visual-probe \
  android::remote_probe::tests::native_attach_current_lomi_generation_three_sinks \
  -- --ignored --exact --nocapture
```

This separate test reads only the bounded managed runtime record and discovery
metadata, verifies process creation/boot identity, managed emulator executable,
AVD ownership and generation, registers an ephemeral authenticated JWK, and
removes that registration on connection drop. It never starts, stops, signals,
wipes, installs, uses ADB, accepts licenses, or writes device data. It prints only
source/sink counters and booleans. No screenshot, runtime record, credentials or
key material are saved or printed. It requires at least one actual stream frame;
a unary-only static bootstrap is inconclusive. Its three sinks are memory sinks,
so this trial does not prove coexistence with the production Tauri renderer.
