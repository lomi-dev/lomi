# macOS event exception propagation

This directory contains the published `tao` 0.35.3 crate, including its license
and cross-platform sources. The upstream crate SHA-256 is
`d1c93047acf68669466a34690ac58cca7010bd1b201e1ec86f1fd0a75d3dd4a9`.
Registry cache markers and the crate's independent `Cargo.lock` are omitted.

`macos-event-unwind.patch` is the only source change. Both macOS `sendEvent:`
overrides and their registration casts use `extern "C-unwind"` so that native
Objective-C exceptions can reach the caller's exception handler. The application
override otherwise aborts with `panic_cannot_unwind` before AppKit's outer
event handler can receive an exception. The window override has the same ABI
boundary. This patch adds no exception suppression, input filtering, or replay.

Both Lomi 0.6.0 reports from macOS 27.0.1 end at the application override's
unwind landing pad (`send_event +1260`). They do not include the original
exception reason. The native regression deliberately raises a known exception
while dispatching a keyboard event to verify the ABI correction. It also posts
a throwing event followed by a normal key through the real AppKit event loop
and checks that the following key arrives, without an extra exception catcher.
It does not identify the original AppKit/WebKit failure.

The ABI follows the [Rust FFI unwinding rules](https://doc.rust-lang.org/nomicon/ffi.html#ffi-and-unwinding)
and [objc2's supported method implementations](https://docs.rs/objc2/0.6.4/objc2/runtime/struct.ClassBuilder.html).
AppKit retains its [normal exception policy](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Exceptions/Tasks/HandlingExceptions.html).
This also permits Rust unwinding through these two frames; it does not make
foreign exception handlers safe for catching Rust panics.

To verify on macOS:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --locked --example macos-keyboard-exception
```

To refresh, unpack the published crate, reapply `macos-event-unwind.patch`, and
run the native regression and application checks. Remove this override when
upstream provides the equivalent fix. Retain the other platforms' sources
unchanged so all desktop builds continue to resolve the same Tao version.
