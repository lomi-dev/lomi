use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitMutation {
    Stage,
    Unstage,
    Commit,
    Fetch,
    Push,
    Discard,
    Pull,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitMutateInput {
    pub workspace_id: String,
    #[serde(default)]
    pub repository_relative: String,
    pub operation: GitMutation,
    pub paths: Vec<String>,
    pub message: Option<String>,
    pub remote: Option<String>,
    pub reference: Option<String>,
    pub source_commit: Option<String>,
    pub expected_remote_commit: Option<String>,
    pub pull_mode: Option<GitPullMode>,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitMutateCommand {
    pub workspace_id: String,
    pub input: GitMutateInput,
    pub not_after_millis: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitMutationFile {
    pub relative_path: String,
    pub sha256: Option<String>,
    pub byte_length: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitMutationPlan {
    pub plan_hash: String,
    pub repository_path: String,
    pub operation: GitMutation,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub files: Vec<GitMutationFile>,
    pub commit: Option<GitCommitApproval>,
    pub network: Option<GitFetchApproval>,
    pub push: Option<GitPushApproval>,
    pub discard: Option<Vec<GitDiscardFile>>,
    pub pull: Option<GitPullApproval>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitDiscardFile {
    pub relative_path: String,
    pub index_object: String,
    pub index_mode: String,
    pub patch: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitPushTarget {
    pub remote: String,
    pub reference: String,
    pub source_commit: String,
    pub expected_remote_commit: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitPushApproval {
    pub target: GitPushTarget,
    pub location: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitPushVerification {
    ServerAcknowledgement,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitPushed {
    pub target: GitPushTarget,
    pub changed: bool,
    pub verification: GitPushVerification,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitFetchApproval {
    pub remote: String,
    pub reference: String,
    pub destination: String,
    pub location: String,
    pub previous_commit: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitFetched {
    pub remote: String,
    pub reference: String,
    pub destination: String,
    pub commit: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitStagedChange {
    pub relative_path: String,
    pub old_mode: String,
    pub new_mode: String,
    pub old_object: String,
    pub new_object: String,
    pub status: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitCommitIdentity {
    pub name: String,
    pub email: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitCommitApproval {
    pub message: String,
    pub final_newline_added: bool,
    pub author: GitCommitIdentity,
    pub committer: GitCommitIdentity,
    pub changes: Vec<GitStagedChange>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitCommitCreated {
    pub message_sha256: String,
    pub author: GitCommitIdentity,
    pub committer: GitCommitIdentity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitMutated {
    pub workspace_id: String,
    pub repository_relative: String,
    pub operation: GitMutation,
    pub plan_hash: String,
    pub before_revision: String,
    pub after_revision: String,
    pub head: Option<String>,
    pub index_revision: String,
    pub files: Vec<GitMutationFile>,
    pub selected_files_changed: bool,
    pub commit: Option<GitCommitCreated>,
    pub fetch: Option<GitFetched>,
    pub push: Option<GitPushed>,
    pub pull: Option<GitPulled>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitPullMode {
    FfOnly,
    Rebase,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitPullTarget {
    pub remote: String,
    pub reference: String,
    pub source_commit: String,
    pub expected_remote_commit: String,
    pub mode: GitPullMode,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitPullApproval {
    pub target: GitPullTarget,
    pub network: GitFetchApproval,
    pub replay_commits: Vec<String>,
    pub affected_paths: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitPullOutcome {
    Applied,
    RemoteChanged,
    Conflicted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitPulled {
    pub target: GitPullTarget,
    pub fetched_commit: String,
    pub outcome: GitPullOutcome,
}
fn default_limit() -> u16 {
    100
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitStatusInput {
    pub workspace_id: String,
    #[serde(default)]
    pub repository_relative: String,
    #[serde(default = "default_limit")]
    pub limit: u16,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitChange {
    pub relative_path: String,
    pub original_path: Option<String>,
    pub index: String,
    pub worktree: String,
    pub directory: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitSource {
    Git,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitConsistency {
    PerCommandSnapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitStatusPage {
    pub workspace_id: String,
    pub repository_relative: String,
    pub source: GitSource,
    pub consistency: GitConsistency,
    pub git_version: String,
    pub observation_revision: String,
    pub changes: Vec<GitChange>,
    pub omitted_entries: u32,
    pub next_cursor: Option<String>,
}

fn default_chars() -> u16 {
    4096
}
fn default_history_limit() -> u16 {
    20
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitComparison {
    Worktree,
    Staged,
    Commit,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitDiffInput {
    pub workspace_id: String,
    #[serde(default)]
    pub repository_relative: String,
    pub relative_path: String,
    pub comparison: GitComparison,
    pub commit: Option<String>,
    #[serde(default)]
    pub start_utf16: u32,
    #[serde(default = "default_chars")]
    pub max_chars: u16,
    pub expected_observation_revision: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitDiffNotice {
    Binary,
    Untracked,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitDiffPage {
    pub workspace_id: String,
    pub repository_relative: String,
    pub relative_path: String,
    pub comparison: GitComparison,
    pub commit: Option<String>,
    pub source: GitSource,
    pub consistency: GitConsistency,
    pub observation_revision: String,
    pub patch: String,
    pub start_utf16: u32,
    pub total_utf16: u32,
    pub next_utf16: Option<u32>,
    pub notice: Option<GitDiffNotice>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitHistoryInput {
    pub workspace_id: String,
    #[serde(default)]
    pub repository_relative: String,
    pub commit: Option<String>,
    #[serde(default)]
    pub skip: u32,
    #[serde(default = "default_history_limit")]
    pub limit: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitCommitSummary {
    pub commit: String,
    pub parents: Vec<String>,
    pub author: String,
    pub authored_at: String,
    pub subject: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitHistoryPage {
    pub workspace_id: String,
    pub repository_relative: String,
    pub source: GitSource,
    pub consistency: GitConsistency,
    pub start_commit: String,
    pub commits: Vec<GitCommitSummary>,
    pub skip: u32,
    pub next_skip: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitCommitInput {
    #[serde(default)]
    pub files_skip: u32,
    #[serde(default = "default_limit")]
    pub files_limit: u16,
    pub workspace_id: String,
    #[serde(default)]
    pub repository_relative: String,
    pub commit: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitCommitDetails {
    pub committer: String,
    pub committer_email: String,
    pub committed_at: String,
    pub files: Vec<GitCommitFile>,
    pub files_skip: u32,
    pub next_files_skip: Option<u32>,
    pub omitted_entries: u32,
    pub workspace_id: String,
    pub repository_relative: String,
    pub source: GitSource,
    pub commit: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_email: String,
    pub authored_at: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitRemotesInput {
    pub workspace_id: String,
    #[serde(default)]
    pub repository_relative: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitRemoteRole {
    Fetch,
    Push,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitRemote {
    pub name: String,
    pub role: GitRemoteRole,
    pub transport: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub location_redacted: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitRemotes {
    pub workspace_id: String,
    pub repository_relative: String,
    pub source: GitSource,
    pub remotes: Vec<GitRemote>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitCommitFile {
    pub relative_path: String,
    pub status: String,
    pub old_mode: String,
    pub new_mode: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum GitView {
    Diff { relative_path: String, staged: bool },
    Commit { commit: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitOpenInput {
    pub workspace_id: String,
    #[serde(default)]
    pub repository_relative: String,
    pub view: GitView,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitOpenCommand {
    pub workspace_id: String,
    pub repository_relative: String,
    pub view: GitView,
    pub not_after_millis: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GitOpened {
    pub workspace_id: String,
    pub panel_id: String,
    pub repository_relative: String,
    pub view: GitView,
    pub observation_revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GitViewBody {
    Diff(Box<GitDiffPage>),
    Commit(Box<GitCommitDetails>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedGitView {
    pub root: String,
    pub permit_id: String,
    pub observation_revision: String,
    pub body: GitViewBody,
}
