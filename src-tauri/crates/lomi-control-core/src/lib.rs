#[cfg(unix)]
pub mod android;
#[cfg(unix)]
pub mod android_input;
#[cfg(unix)]
pub mod artifacts;
#[cfg(unix)]
pub mod atomic_file;
pub mod authentication;
#[cfg(unix)]
pub mod broker;
#[cfg(unix)]
pub mod browser;
#[cfg(unix)]
pub mod client;
#[cfg(unix)]
pub mod file_mutation;
#[cfg(unix)]
pub mod file_trash;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub mod git_execution;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub mod git_read;
#[cfg(unix)]
pub mod project_files;
#[cfg(unix)]
pub mod receipts;
#[cfg(unix)]
pub mod staging;
#[cfg(unix)]
pub mod terminal;
#[cfg(unix)]
pub mod terminal_io;

/// Process ownership shared by the native installer and explicitly approved Git execution.
#[cfg(any(unix, windows))]
pub mod process_tree;
