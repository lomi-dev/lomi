mod commands;
mod environment;
#[cfg(unix)]
mod execution;
#[cfg(not(unix))]
#[path = "execution_unsupported.rs"]
mod execution;
#[cfg(target_os = "macos")]
mod file_credentials;
mod grok_artifact;
#[cfg(target_os = "macos")]
mod host_boundary;
#[cfg(target_os = "macos")]
mod host_child;
#[cfg(all(feature = "native-smoke", target_os = "macos"))]
pub(crate) mod host_smoke;
#[cfg(unix)]
mod managed_config;
#[cfg(target_os = "macos")]
mod managed_mcp;
#[cfg(unix)]
mod mcp_policy;
pub(crate) mod migration;
mod native_accounts;
mod native_artifact;
#[cfg(unix)]
mod native_history;
#[cfg(unix)]
pub(crate) mod native_launch;
#[cfg(target_os = "macos")]
mod native_network;
#[cfg(unix)]
mod native_policy;
#[cfg(unix)]
mod native_process;
#[cfg(not(unix))]
#[path = "native_unsupported.rs"]
mod native_process;
#[cfg(unix)]
mod native_transfer;
mod native_wire;
#[cfg(target_os = "macos")]
mod owned_operation;
#[cfg(unix)]
mod pi_artifact;
#[cfg(unix)]
mod process_identity;
#[cfg(unix)]
mod process_owner;
#[cfg(all(test, unix))]
mod process_owner_tests;
#[cfg(unix)]
mod process_supervision;
mod service;
mod store;
#[cfg(unix)]
mod transfer;
mod types;
pub(crate) use commands::*;
pub(crate) use service::account_terminal;
pub(crate) use service::AgentRuntime;
#[cfg(target_os = "macos")]
pub(crate) fn host_worker_entry() -> Option<i32> {
    file_credentials::entry().or_else(host_boundary::entry)
}
use std::io::Read;
pub(crate) fn new_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    #[cfg(unix)]
    {
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|_| "Cannot create runtime identity.")?;
    }
    #[cfg(not(unix))]
    {
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
            .map_err(|_| "Cannot create runtime identity.")?;
    }
    Ok(bytes.iter().map(|v| format!("{v:02x}")).collect())
}
pub(crate) fn valid_operation(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'-' | b'_' | b':'))
    {
        Err("Invalid operation identity.".into())
    } else {
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests;
