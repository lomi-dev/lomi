use crate::cli_catalog::TitleCli;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AccountInstance {
    pub account_id: String,
    pub cli: TitleCli,
    pub label: String,
    pub enabled: bool,
    pub revision: u64,
    pub auth_revision: u64,
    pub auth_state: String,
    pub availability_reason: Option<String>,
    pub accepted_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<AccountRecovery>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CredentialBinding {
    pub account_id: String,
    pub auth_revision: u64,
    pub physical_root: String,
    pub namespace: String,
    pub credential_reference: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Capability {
    pub cli: TitleCli,
    pub account_terminal: bool,
    pub managed_execution: bool,
    pub versions: Vec<String>,
    pub cross_account_native_resume: bool,
    pub reviewed_transfer: bool,
    pub stop_and_continue_qualified: bool,
    pub stop_and_continue_reason: Option<String>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AccountsSnapshot {
    pub schema: u32,
    pub revision: u64,
    pub accounts: Vec<AccountInstance>,
    pub capabilities: Vec<Capability>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TasksSnapshot {
    pub schema: u32,
    pub revision: u64,
    pub tasks: Vec<Task>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskState {
    Idle,
    Starting,
    Running,
    Stopping,
    Stopped,
    Completed,
    Prepared,
    RecoveryRequired,
    DeliveryUncertain,
    Archived,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Attempt {
    pub attempt_id: String,
    pub operation_id: String,
    pub account_id: String,
    pub auth_revision: u64,
    pub generation: u64,
    pub input: String,
    pub continuation_method: String,
    pub state: String,
    pub output: String,
    pub native_ref: Option<String>,
    pub version: Option<String>,
    pub effects_state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HistoryRecord {
    pub sequence: u64,
    pub attempt_id: String,
    pub account_id: String,
    pub auth_revision: u64,
    pub kind: String,
    pub state: String,
    pub content: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HistoryGrant {
    pub account_id: String,
    pub auth_revision: u64,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SwitchOperation {
    pub operation_id: String,
    pub source_attempt_id: Option<String>,
    pub account_id: String,
    pub auth_revision: u64,
    pub history_revision: u64,
    pub mode: String,
    pub phase: String,
    pub continuation_method: String,
    pub reason: Option<String>,
    pub coverage: u64,
    pub budget_bytes: u64,
    pub context_digest: Option<String>,
    pub stop_supervision_qualified: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Task {
    pub task_id: String,
    pub cwd: String,
    pub title: String,
    pub cli: Option<TitleCli>,
    pub availability_reason: Option<String>,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub revision: u64,
    pub history_revision: u64,
    pub generation: u64,
    pub state: TaskState,
    pub next_account_id: String,
    pub active_account_id: Option<String>,
    pub active_attempt_id: Option<String>,
    pub status_message: String,
    pub attempts: Vec<Attempt>,
    pub history: Vec<HistoryRecord>,
    pub grants: Vec<HistoryGrant>,
    pub switches: Vec<SwitchOperation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HistoryCheckpoint {
    pub task_id: String,
    pub attempt_id: String,
    pub account_id: String,
    pub auth_revision: u64,
    pub generation: u64,
    pub history_revision: u64,
    pub digest: String,
    pub version: String,
    pub model: String,
    pub cwd_identity: (u64, u64),
    pub native_ref: String,
    pub session_file: Option<String>,
    pub settled: bool,
}
macro_rules! request { ($name:ident { $($(#[$attr:meta])* $field:ident : $type:ty),* $(,)? }) => {
 #[derive(Clone, Debug, Serialize, Deserialize)]
 #[serde(rename_all="camelCase", deny_unknown_fields)]
 pub(crate) struct $name { $($(#[$attr])* pub $field:$type),* }
}; }
request!(AccountCreate {
    operation_id: String,
    cli: TitleCli,
    label: String
});
request!(AccountUpdate { operation_id:String, account_id:String, expected_revision:u64, label:Option<String>, enabled:Option<bool> });
request!(AccountRemove {
    operation_id: String,
    account_id: String,
    expected_revision: u64
});
request!(AccountVerify {
    operation_id: String,
    account_id: String,
    expected_revision: u64,
    shell_profile_id: String,
    cwd: String
});
request!(TaskCreate { operation_id:String, account_id:String, auth_revision:u64, cwd:String, title:String, model:String, reasoning_effort:Option<String>, shell_profile_id:Option<String> });
request!(TaskSend { operation_id:String, task_id:String, expected_revision:u64, account_id:String, auth_revision:u64, text:String, #[serde(rename="continue")] continue_requested:Option<bool> });
request!(GrantUpdate {
    operation_id: String,
    task_id: String,
    expected_revision: u64,
    account_id: String,
    auth_revision: u64,
    expected_history_revision: u64,
    allow: bool
});
request!(SwitchPrepare { operation_id:String, task_id:String, expected_revision:u64, account_id:String, auth_revision:u64, expected_history_revision:u64, source_attempt_id:Option<String>, mode:String });
request!(SwitchCommit {
    operation_id: String,
    task_id: String,
    switch_operation_id: String,
    expected_revision: u64,
    account_id: String,
    auth_revision: u64
});
request!(TaskStop {
    operation_id: String,
    task_id: String,
    expected_revision: u64
});
request!(TaskRecover {
    operation_id: String,
    task_id: String,
    expected_revision: u64,
    expected_history_revision: u64,
    acknowledge_effects: bool
});
request!(PermissionReply { operation_id:String, task_id:String, attempt_id:String, generation:u64, approval_token:String, allow:bool, choice:Option<Value>, editor_freeze_token:Option<String> });
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PendingPermission {
    pub task_id: String,
    pub attempt_id: String,
    pub generation: u64,
    pub approval_token: String,
    pub permission: super::native_wire::NativePermission,
    pub project_root: String,
}
request!(TransferPreview {
    operation_id: String,
    task_id: String,
    expected_revision: u64,
    account_id: String,
    auth_revision: u64,
    expected_history_revision: u64
});
request!(TransferApply {
    operation_id: String,
    task_id: String,
    expected_revision: u64,
    account_id: String,
    auth_revision: u64,
    expected_history_revision: u64,
    expected_digest: String,
    expected_bytes: u64
});
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TransferReview {
    pub digest: String,
    pub bytes: u64,
    pub task_id: String,
    pub account_id: String,
    pub auth_revision: u64,
    pub coverage: u64,
    pub source_attempt_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AccountRecovery {
    pub state: String,
    pub reason: String,
    pub operation_ids: Vec<String>,
    pub recoverable: bool,
    pub requires_verified_boot_change: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AccountRecover {
    pub operation_id: String,
    pub account_id: String,
    pub expected_revision: u64,
    pub acknowledge_effects: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NativeOperation {
    #[serde(default)]
    pub host_boundary_owned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_account_root: Option<String>,
    pub operation_id: String,
    pub cli: TitleCli,
    pub purpose: String,
    pub account_id: String,
    pub auth_revision: u64,
    pub task_id: Option<String>,
    pub attempt_id: Option<String>,
    pub generation: Option<u64>,
    pub cwd: String,
    pub boot: String,
    pub state: String,
}
