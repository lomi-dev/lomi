use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserLogsInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}
fn default_limit() -> u32 {
    50
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLogKind {
    JavascriptError,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserLogEntry {
    pub sequence: u64,
    pub captured_at_millis: u64,
    pub kind: BrowserLogKind,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserLogs {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub frame_id: String,
    pub origin: String,
    pub capture_started_at_millis: u64,
    pub coverage: Vec<String>,
    pub entries: Vec<BrowserLogEntry>,
    pub next_cursor: String,
    pub dropped: u64,
    pub has_more: bool,
}
