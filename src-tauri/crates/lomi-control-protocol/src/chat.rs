use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatListInput {
    pub workspace_id: String,
    #[serde(default)]
    pub offset: u16,
    #[serde(default = "default_limit")]
    pub limit: u16,
    pub expected_revision: Option<String>,
}
fn default_limit() -> u16 {
    20
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSummary {
    pub conversation_id: String,
    pub title: String,
    pub conversation_revision: String,
    pub latest_message_id: Option<String>,
    pub updated_at_millis: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatList {
    pub workspace_id: String,
    pub revision: String,
    pub items: Vec<ChatSummary>,
    pub total: u16,
    pub offset: u16,
    pub next_offset: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChatReadPart {
    Draft,
    Message { message_id: Option<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatReadInput {
    pub workspace_id: String,
    pub conversation_id: String,
    pub part: ChatReadPart,
    #[serde(default)]
    pub start_utf16: u32,
    #[serde(default = "default_chars")]
    pub max_chars: u16,
    pub expected_revision: Option<String>,
    #[serde(default)]
    pub include_send_target: bool,
}
fn default_chars() -> u16 {
    8192
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatReadSource {
    PersistedCheckpoint,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatRead {
    pub workspace_id: String,
    pub conversation: ChatSummary,
    pub source: ChatReadSource,
    pub part: ChatReadPart,
    pub revision: String,
    pub draft_revision: Option<String>,
    pub parent_message_id: Option<String>,
    pub role: String,
    pub status: String,
    pub content: String,
    pub start_utf16: u32,
    pub total_utf16: u32,
    pub next_utf16: Option<u32>,
    pub attachments_omitted: bool,
    pub non_text_parts_omitted: bool,
    pub send_target: Option<ChatSendTarget>,
    pub request: Option<ChatRequest>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatRequest {
    pub request_id: String,
    pub assistant_id: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatStopInput {
    pub workspace_id: String,
    pub conversation_id: String,
    pub request_id: String,
    pub retry_epoch: String,
    pub request_key: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatStopped {
    pub workspace_id: String,
    pub conversation_id: String,
    pub request: ChatRequest,
    pub already_finished: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatExportFormat {
    Markdown,
    Json,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatExportInput {
    pub workspace_id: String,
    pub conversation_id: String,
    pub format: ChatExportFormat,
    #[serde(default)]
    pub start_utf16: u32,
    #[serde(default = "default_chars")]
    pub max_chars: u16,
    pub expected_revision: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatExport {
    pub workspace_id: String,
    pub conversation_id: String,
    pub format: ChatExportFormat,
    pub revision: String,
    pub total_bytes: u32,
    pub message_count: u16,
    pub content: String,
    pub start_utf16: u32,
    pub total_utf16: u32,
    pub next_utf16: Option<u32>,
    pub attachments_omitted: bool,
    pub non_text_parts_omitted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSendTarget {
    pub connection_id: String,
    pub model: String,
    pub max_output_tokens: u32,
}

pub fn valid_chat_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChatOpenTarget {
    Existing { conversation_id: String },
    New,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatOpenInput {
    pub workspace_id: String,
    pub target: ChatOpenTarget,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatOpenCommand {
    pub workspace_id: String,
    pub project_name: String,
    pub workspace_name: String,
    pub conversation_id: String,
    pub panel_id: String,
    pub create: bool,
    pub not_after_millis: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatOpened {
    pub workspace_id: String,
    pub panel_id: String,
    pub conversation_id: String,
    pub created: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatDraftInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub conversation_id: String,
    pub text: String,
    pub expected_draft_revision: String,
    pub expected_conversation_revision: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatDraftCommand {
    pub workspace_id: String,
    pub input: ChatDraftInput,
    pub not_after_millis: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatDraftUpdated {
    pub workspace_id: String,
    pub panel_id: String,
    pub conversation_id: String,
    pub draft_revision: String,
    pub conversation_revision: String,
    pub text_sha256: String,
    pub total_utf16: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSendInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub conversation_id: String,
    pub connection_id: String,
    pub model: String,
    pub expected_draft_revision: String,
    pub expected_conversation_revision: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSendCommand {
    pub workspace_id: String,
    pub input: ChatSendInput,
    pub request_id: String,
    pub user_id: String,
    pub assistant_id: String,
    pub not_after_millis: String,
}

/// Native Main-only approval data. Never include this in tool results or logs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSendPlan {
    pub plan_hash: String,
    pub conversation_title: String,
    pub connection_id: String,
    pub connection_name: String,
    pub provider: String,
    pub model: String,
    pub max_output_tokens: u32,
    pub temperature: Option<f64>,
    pub draft_text: String,
    pub system: String,
    pub message_count: u32,
    pub context_bytes: u32,
    pub attachments: Vec<ChatSendAttachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSendAttachment {
    pub name: String,
    pub mime: String,
    pub byte_length: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSent {
    pub workspace_id: String,
    pub panel_id: String,
    pub conversation_id: String,
    pub request_id: String,
    pub user_id: String,
    pub assistant_id: String,
    pub connection_id: String,
    pub model: String,
    /// None identifies a durable reservation, not provider acceptance.
    pub draft_revision: Option<String>,
    pub rejection: Option<crate::ErrorCode>,
}
