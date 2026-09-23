pub mod android;
pub mod artifact;
pub mod browser;
pub mod browser_dom;
pub mod browser_logs;
pub mod control;
pub mod editor;
pub mod files;
pub mod framing;
pub mod git;
pub mod layout;
pub mod settings;
pub mod workspace;

pub const CONTROL_API_VERSION: &str = "1.0";
pub const IPC_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;
pub const MAX_METADATA_BYTES: usize = 64 * 1024;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyInput {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    AppUnavailable,
    PairingRequired,
    ScopeDenied,
    UiNotReady,
    AppClosing,
    TargetNotFound,
    StaleGeneration,
    RevisionConflict,
    StaleSnapshot,
    TargetBusy,
    ControlRevoked,
    PromptStateUnknown,
    PanelNotRenderable,
    ProtectedOriginTerminal,
    UnsupportedCapability,
    HostUnqualified,
    DeadlineExceeded,
    CursorExpired,
    ArtifactTooLarge,
    ArtifactInvalid,
    OutcomeUnknown,
    IdempotencyConflict,
    RetryWindowExpired,
    ResourceExhausted,
    StorageUnavailable,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolError {
    pub control_api_version: String,
    pub status: ErrorStatus,
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStatus {
    Error,
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            control_api_version: CONTROL_API_VERSION.into(),
            status: ErrorStatus::Error,
            code,
            message: message.into(),
        }
    }
}
