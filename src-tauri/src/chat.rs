#[cfg(unix)]
pub(crate) mod agent;
#[cfg(unix)]
pub(crate) mod agent_close;
#[cfg(unix)]
pub(crate) mod agent_export;
#[cfg(unix)]
pub(crate) mod agent_send;
#[cfg(unix)]
pub(crate) mod agent_stop;
pub mod attachments;
pub mod backend;
pub mod commands;
pub mod credentials;
pub mod delivery;
mod generation;
pub mod preferences;
pub mod process;
pub mod storage;
pub mod store;

#[cfg(windows)]
mod windows_permissions;
