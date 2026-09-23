use crate::layout::PanelMoveIdentity;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceCloseInput {
    pub action: WorkspaceCloseAction,
    pub workspace_id: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceCloseAction {
    Close,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceCloseCommand {
    pub workspace_id: String,
    pub panels: Vec<PanelMoveIdentity>,
    pub not_after_millis: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceClosure {
    pub workspace_id: String,
    pub project_id: String,
    pub panel_ids: Vec<String>,
    pub terminal_session_ids: Vec<String>,
    pub closed: Option<bool>,
    pub project_closed: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectCloseInput {
    pub project_id: String,
    pub workspace_id: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectCloseCommand {
    pub workspace_id: String,
    pub workspaces: Vec<WorkspaceCloseCommand>,
    pub not_after_millis: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectClosure {
    pub workspace_id: String,
    pub project_id: String,
    pub workspace_ids: Vec<String>,
    pub panel_ids: Vec<String>,
    pub terminal_session_ids: Vec<String>,
    pub closed: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOpenInput {
    /// An approved workspace binds the receipt, even after that workspace closes.
    pub workspace_id: String,
    pub project_path: String,
    pub name: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOpenCommand {
    pub workspace_id: String,
    pub project_id: String,
    pub project_path: String,
    pub new_workspace_id: String,
    pub tab_id: String,
    pub name: String,
    pub request_key: String,
    pub not_after_millis: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOpened {
    pub anchor_workspace_id: String,
    pub project_id: String,
    pub project_path: String,
    pub workspace_id: String,
    pub panel_id: String,
    pub name: String,
    pub opened: Option<bool>,
}
