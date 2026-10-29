mod adb;
#[cfg(unix)]
pub(crate) mod agent;
#[cfg(unix)]
pub(crate) mod agent_apk;
#[cfg(unix)]
pub(crate) mod agent_capture;
#[cfg(unix)]
pub(crate) mod agent_install;
#[cfg(unix)]
pub(crate) mod agent_setup;
#[cfg(unix)]
mod apk_manifest;
mod artifact;
mod auth;
mod avd;
mod bootstrap;
mod catalog;
pub mod commands;
mod devices;
mod diagnostics;
mod disk;
mod download;
#[cfg(windows)]
mod emulator_process;
mod environment;
mod events;

mod frames;
#[cfg(unix)]
mod hierarchy;
mod host;
mod input;
mod installation;
mod installer;
mod installer_process;
mod installer_tree;
mod jre_archive;
mod maintenance;
mod managed_adb;
pub mod manager;
pub mod open;
mod process_identity;
mod recovery;

mod repository;
mod rpc;
mod runtime;
#[cfg(windows)]
mod shared_frames;
mod storage;
#[cfg(windows)]
mod window_focus;
