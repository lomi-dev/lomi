pub use crate::android::*;
pub use crate::android_setup::*;
pub use crate::artifact::*;
pub use crate::browser_dom::*;
pub use crate::browser_logs::*;
pub use crate::chat::*;
pub use crate::editor::*;
pub use crate::files::*;
pub use crate::git::*;
pub use crate::layout::*;
pub use crate::settings::*;
pub use crate::workspace::*;
use crate::{ErrorCode, CONTROL_API_VERSION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Workspace {
    pub id: String,
    #[serde(default)]
    pub active_panel_id: Option<String>,
    pub project_id: String,
    pub name: String,
    pub project_name: String,
    pub project_path: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capability {
    pub name: String,
    pub available: bool,
    pub authorized: bool,
    pub qualified: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Projection {
    pub ui_epoch: String,
    pub revision: String,
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub panels: Vec<Panel>,
    #[serde(default)]
    pub terminal_profile: Option<TerminalProfile>,
    #[serde(default)]
    pub focused_panel_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalProfile {
    pub id: String,
    pub revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Panel {
    pub id: String,
    pub tab_id: String,
    pub workspace_id: String,
    pub kind: String,
    pub title: String,
    pub terminal_session_id: Option<String>,
    #[serde(default)]
    pub browser_generation: Option<String>,
    #[serde(default)]
    pub android_device_id: Option<String>,
    #[serde(default)]
    pub chat_conversation_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Enrollment {
    pub ipc_version: u16,
    pub instance_id: String,
    pub client_label: String,
    pub certificate: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Welcome {
    pub ipc_version: u16,
    pub instance_id: String,
    pub certificate: Vec<u8>,
    pub pairing_request_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectInput {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListInput {
    #[serde(default = "default_limit")]
    pub limit: u16,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceListInput {
    pub workspace_id: String,
    #[serde(default = "default_limit")]
    pub limit: u16,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelView {
    pub id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub kind: String,
    pub title: String,
    pub terminal_session_id: Option<String>,
    pub browser_generation: Option<String>,
    pub android_device_id: Option<String>,
    pub ownership: String,
    pub runtime_state: String,
    pub input_controlled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlEvent {
    pub sequence: String,
    pub kind: String,
    pub operation_id: String,
    pub workspace_id: String,
    pub tool: String,
    pub state: String,
    pub effect_state: String,
    pub timestamp_seconds: String,
}
fn default_limit() -> u16 {
    100
}
impl Default for ListInput {
    fn default() -> Self {
        Self {
            limit: default_limit(),
            cursor: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationInput {
    pub operation_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationKeyInput {
    /// Required when this connection has more than one approved project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub retry_epoch: String,
    pub request_key: String,
    pub tool: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(extend("type"="object"))]
pub enum OperationLookup {
    Id(OperationInput),
    Key(OperationKeyInput),
}
impl From<OperationInput> for OperationLookup {
    fn from(value: OperationInput) -> Self {
        Self::Id(value)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceRenameInput {
    pub workspace_id: String,
    pub name: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(extend("type"="object"))]
pub enum WorkspaceUpdateInput {
    Rename(WorkspaceRenameInput),
    Select(WorkspaceSelectInput),
    Close(WorkspaceCloseInput),
}
impl From<WorkspaceRenameInput> for WorkspaceUpdateInput {
    fn from(value: WorkspaceRenameInput) -> Self {
        Self::Rename(value)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSelectInput {
    pub action: WorkspaceSelectAction,
    pub workspace_id: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSelectAction {
    Select,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum OperationResult {
    ChatOpened(ChatOpened),
    ChatDraftUpdated(Box<ChatDraftUpdated>),
    ChatSent(Box<ChatSent>),
    ChatStopped(Box<ChatStopped>),
    SettingsOpened(SettingsOpened),
    SettingsUpdated(SettingsUpdated),
    WorkspaceClosure(WorkspaceClosure),
    ProjectClosure(Box<ProjectClosure>),
    ProjectOpened(Box<ProjectOpened>),
    PanelMoved(Box<PanelMoved>),
    GitMutated(Box<GitMutated>),
    FilesMutated(FilesMutated),
    ArtifactExported(Box<ArtifactExported>),
    BrowserDownloaded(Box<Artifact>),
    EditorSaved(Box<EditorSaved>),
    EditorOpened(Box<EditorOpened>),
    GitOpened(Box<GitOpened>),
    EditorPreviewed(Box<EditorPreviewed>),
    EditorEdited(Box<EditorEdited>),
    AndroidLaunch(AndroidLaunchResult),
    AndroidInstall(Box<AndroidInstallResult>),
    AndroidManagement(Box<AndroidManagementResult>),
    ArtifactImported {
        workspace_id: String,
        artifact_id: String,
        sha256: String,
        byte_length: u32,
    },
    AndroidControl(AndroidControlResult),
    AndroidInput(AndroidInputResult),
    AndroidRuntime(AndroidRuntimeResult),
    AndroidPanel {
        workspace_id: String,
        panel_id: String,
        device_id: String,
    },
    BrowserInteraction(Box<BrowserInteractionResult>),
    Browser(Box<BrowserResult>),
    BrowserNavigation(Box<BrowserNavigationResult>),
    TerminalControl {
        workspace_id: String,
        panel_id: String,
        terminal_session_id: String,
        lease_id: Option<String>,
        controlled: bool,
    },
    Panel {
        workspace_id: String,
        panel_id: String,
        focused: bool,
        closed: bool,
    },
    Workspace {
        workspace_id: String,
        name: String,
    },
    Failure {
        code: ErrorCode,
    },
    TerminalInterrupt {
        workspace_id: String,
        panel_id: String,
        terminal_session_id: String,
        target_operation_id: String,
        dispatched: bool,
    },
    TerminalCommand {
        workspace_id: String,
        panel_id: String,
        terminal_session_id: String,
        observation: Box<CommandObservation>,
    },
    Terminal {
        workspace_id: String,
        panel_id: String,
        terminal_session_id: String,
        lease_id: Option<String>,
        ready: bool,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserResult {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub profile_id: String,
    pub navigation_id: String,
    pub lease_id: Option<String>,
    pub ready: bool,
    pub engine: String,
    pub network_isolation: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserWaitUntil {
    Commit,
    #[default]
    Load,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserNavigateInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub lease_id: String,
    pub url: String,
    #[serde(default)]
    pub wait_until: BrowserWaitUntil,
    pub retry_epoch: String,
    pub request_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserNavigationResult {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub url: String,
    pub committed: bool,
    pub loaded: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelMutationInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub terminal_session_id: Option<String>,
    pub browser_generation: Option<String>,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalCreateInput {
    pub workspace_id: String,
    pub cwd_relative: String,
    pub profile_id: Option<String>,
    pub title: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserOpenInput {
    pub workspace_id: String,
    pub url: String,
    #[serde(default = "default_visible")]
    pub visible: bool,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
fn default_visible() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TerminalReadMode {
    #[default]
    Output,
    Command,
    Screen,
    Raw,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalScreen {
    pub workspace_id: String,
    pub panel_id: String,
    pub terminal_session_id: String,
    pub columns: u16,
    pub rows: u16,
    pub cursor_column: u16,
    pub cursor_row: u16,
    pub cursor_visible: bool,
    pub viewport_offset: u32,
    pub buffer: ScreenBuffer,
    pub text: String,
    pub truncated: bool,
    pub parsed_sequence: String,
    pub stream_sequence: String,
    pub parser_pending: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScreenBuffer {
    Normal,
    Alternate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScreenRequest {
    pub request_id: String,
    pub ui_epoch: String,
    pub workspace_id: String,
    pub panel_id: String,
    pub terminal_session_id: String,
    pub minimum_parsed_sequence: String,
    pub max_bytes: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScreenReply {
    pub request_id: String,
    pub ui_epoch: String,
    pub screen: Option<TerminalScreen>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalReadInput {
    #[serde(default)]
    pub mode: TerminalReadMode,
    pub minimum_parsed_sequence: Option<String>,
    pub workspace_id: String,
    pub panel_id: String,
    pub terminal_session_id: String,
    pub cursor: Option<String>,
    pub max_bytes: Option<u32>,
    pub operation_id: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanelControlAction {
    Claim,
    Release,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(extend("type"="object"))]
pub enum PanelControlRequest {
    Terminal(PanelControlInput),
    Android(AndroidControlInput),
}
impl From<PanelControlInput> for PanelControlRequest {
    fn from(value: PanelControlInput) -> Self {
        Self::Terminal(value)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelControlInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub terminal_session_id: String,
    pub action: PanelControlAction,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalRunInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub terminal_session_id: String,
    pub lease_id: String,
    pub command: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalInterruptInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub terminal_session_id: String,
    pub lease_id: String,
    pub operation_id: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub terminal_session_id: String,
    pub lease_id: String,
    pub input_sequence: String,
    pub input: TerminalPayload,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalPayload {
    Text { text: String },
    Key { key: TerminalKey },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TerminalKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    CtrlC,
    CtrlD,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandObservation {
    pub operation_id: String,
    pub started_observed: bool,
    pub completed: bool,
    pub exit_code: Option<i32>,
    pub source: String,
    pub start_cursor: String,
    pub end_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiCommand {
    pub operation_id: String,
    pub nonce: String,
    pub ui_epoch: String,
    pub domain_revision: String,
    pub project_id: String,
    pub action: UiAction,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum UiAction {
    OpenChat(ChatOpenCommand),
    DraftChat(ChatDraftCommand),
    SendChat(ChatSendCommand),
    CloseProject(ProjectCloseCommand),
    OpenProject(ProjectOpenCommand),
    OpenSettings(SettingsOpenCommand),
    UpdateSettings(Box<SettingsUpdateCommand>),
    CloseWorkspace(WorkspaceCloseCommand),
    MovePanel(PanelMoveCommand),
    FilesMutate(FilesMutateCommand),
    EditorSave(EditorSaveCommand),
    EditorOpen(EditorOpenCommand),
    GitOpen(GitOpenCommand),
    GitMutate(GitMutateCommand),
    EditorEdits(EditorEditsCommand),
    ImportArtifact(ArtifactImportInput),
    ExportArtifact(ArtifactExportCommand),
    DownloadBrowser(BrowserDownloadInput),
    AndroidLaunch(AndroidLaunchInput),
    AndroidInputControl(AndroidControlInput),
    AndroidInput(AndroidInput),
    AndroidRuntime {
        workspace_id: String,
        panel_id: String,
        device_id: String,
        generation: Option<String>,
    },
    CreateAndroid {
        workspace_id: String,
        panel_id: String,
        device_id: String,
        title: String,
    },
    InteractBrowser(BrowserDomCommand),
    NavigateBrowser {
        workspace_id: String,
        panel_id: String,
        browser_generation: String,
        url: String,
        wait_until: BrowserWaitUntil,
    },
    CreateBrowser {
        visible: bool,
        workspace_id: String,
        panel_id: String,
        browser_generation: String,
        profile_id: String,
        url: String,
    },
    ClosePanel {
        workspace_id: String,
        panel_id: String,
        tab_id: String,
        terminal_session_id: Option<String>,
        browser_generation: Option<String>,
        replacement_tab_id: String,
    },
    SelectWorkspace {
        workspace_id: String,
        panel_id: String,
        tab_id: String,
        terminal_session_id: Option<String>,
        browser_generation: Option<String>,
    },
    FocusPanel {
        workspace_id: String,
        panel_id: String,
        tab_id: String,
        terminal_session_id: Option<String>,
        browser_generation: Option<String>,
    },
    CreateWorkspace {
        anchor_workspace_id: String,
        workspace_id: String,
        tab_id: String,
        name: String,
    },
    RenameWorkspace {
        workspace_id: String,
        name: String,
    },
    CreateTerminal {
        workspace_id: String,
        panel_id: String,
        tab_id: String,
        terminal_session_id: String,
        profile_id: String,
        cwd: String,
        title: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiAck {
    pub operation_id: String,
    pub nonce: String,
    pub ui_epoch: String,
    pub result: OperationResult,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "tool", content = "arguments", deny_unknown_fields)]
pub enum Request {
    #[serde(rename = "lomi_chat_export")]
    ChatExport(ChatExportInput),
    #[serde(rename = "lomi_chat_stop")]
    ChatStop(ChatStopInput),
    #[serde(rename = "lomi_chat_send")]
    ChatSend(ChatSendInput),
    #[serde(rename = "lomi_chat_draft")]
    ChatDraft(ChatDraftInput),
    #[serde(rename = "lomi_chat_open")]
    ChatOpen(ChatOpenInput),
    #[serde(rename = "lomi_chat_list")]
    ChatList(ChatListInput),
    #[serde(rename = "lomi_chat_read")]
    ChatRead(ChatReadInput),
    #[serde(rename = "lomi_files_mutate")]
    FilesMutate(FilesMutateInput),
    #[serde(rename = "lomi_editor_save")]
    EditorSave(EditorSaveInput),
    #[serde(rename = "lomi_editor_open")]
    EditorOpen(EditorOpenInput),
    #[serde(rename = "lomi_editor_apply_edits")]
    EditorEdits(EditorEditsInput),
    #[serde(rename = "lomi_editor_read")]
    EditorRead(EditorReadInput),
    #[serde(rename = "lomi_files_search")]
    FilesSearch(FilesSearchInput),
    #[serde(rename = "lomi_files_list")]
    FilesList(FilesListInput),
    #[serde(rename = "lomi_files_read")]
    FilesRead(FilesReadInput),
    #[serde(rename = "lomi_git_status")]
    GitStatus(GitStatusInput),
    #[serde(rename = "lomi_git_open")]
    GitOpen(GitOpenInput),
    #[serde(rename = "lomi_git_mutate")]
    GitMutate(GitMutateInput),
    #[serde(rename = "lomi_git_diff")]
    GitDiff(GitDiffInput),
    #[serde(rename = "lomi_git_history")]
    GitHistory(GitHistoryInput),
    #[serde(rename = "lomi_git_commit")]
    GitCommit(GitCommitInput),
    #[serde(rename = "lomi_git_remotes")]
    GitRemotes(GitRemotesInput),

    #[serde(rename = "lomi_android_logcat")]
    AndroidLogcat(AndroidLogcatInput),
    #[serde(rename = "lomi_android_launch")]
    AndroidLaunch(AndroidLaunchInput),
    #[serde(rename = "lomi_android_install_apk")]
    AndroidInstall(AndroidInstallInput),
    #[serde(rename = "lomi_browser_download")]
    DownloadBrowser(BrowserDownloadInput),
    #[serde(rename = "lomi_artifact_export")]
    ExportArtifact(ArtifactExportInput),
    #[serde(rename = "lomi_artifact_import")]
    ImportArtifact(ArtifactImportInput),
    #[serde(rename = "lomi_android_screenshot")]
    AndroidScreenshot(AndroidScreenshotInput),
    #[serde(rename = "lomi_android_snapshot")]
    AndroidSnapshot(AndroidSnapshotInput),
    #[serde(rename = "lomi_android_input")]
    AndroidInput(AndroidInput),
    #[serde(rename = "lomi_android_start")]
    AndroidStart(AndroidStartInput),
    #[serde(rename = "lomi_android_stop")]
    AndroidStop(AndroidStopInput),
    #[serde(rename = "lomi_android_open")]
    AndroidOpen(AndroidOpenInput),
    #[serde(rename = "lomi_android_list")]
    AndroidList(AndroidListInput),
    #[serde(rename = "lomi_android_setup_plan")]
    AndroidSetupPlan(AndroidSetupPlanInput),
    #[serde(rename = "lomi_android_setup_apply")]
    AndroidSetupApply(AndroidSetupApplyInput),
    #[serde(rename = "lomi_android_device_manage")]
    AndroidDeviceManage(AndroidDeviceManageInput),
    #[serde(rename = "lomi_browser_screenshot")]
    ScreenshotBrowser(BrowserScreenshotInput),
    #[serde(rename = "lomi_browser_logs")]
    BrowserLogs(BrowserLogsInput),
    #[serde(rename = "lomi_artifact_read")]
    ReadArtifact(ArtifactReadInput),
    #[serde(rename = "lomi_browser_wait")]
    WaitBrowser(BrowserWaitInput),
    #[serde(rename = "lomi_browser_key")]
    KeyBrowser(BrowserKeyInput),
    #[serde(rename = "lomi_browser_scroll")]
    ScrollBrowser(BrowserScrollInput),
    #[serde(rename = "lomi_browser_click")]
    ClickBrowser(BrowserClickInput),
    #[serde(rename = "lomi_browser_fill")]
    FillBrowser(BrowserFillInput),
    #[serde(rename = "lomi_browser_snapshot")]
    SnapshotBrowser(BrowserSnapshotInput),
    #[serde(rename = "lomi_browser_navigate")]
    NavigateBrowser(BrowserNavigateInput),
    #[serde(rename = "lomi_browser_open")]
    OpenBrowser(BrowserOpenInput),
    #[serde(rename = "lomi_status")]
    Status(crate::EmptyInput),
    #[serde(rename = "lomi_connect")]
    Connect(ConnectInput),
    #[serde(rename = "lomi_workspace_list")]
    Workspaces(ListInput),
    #[serde(rename = "lomi_panel_list")]
    Panels(WorkspaceListInput),
    #[serde(rename = "lomi_panel_move")]
    MovePanel(PanelMoveInput),
    #[serde(rename = "lomi_panel_focus")]
    FocusPanel(PanelMutationInput),
    #[serde(rename = "lomi_panel_control")]
    ControlPanel(PanelControlRequest),
    #[serde(rename = "lomi_panel_close")]
    ClosePanel(PanelMutationInput),
    #[serde(rename = "lomi_events_read")]
    Events(WorkspaceListInput),
    #[serde(rename = "lomi_diagnostics")]
    Diagnostics(crate::EmptyInput),
    #[serde(rename = "lomi_operation_get")]
    Operation(OperationLookup),
    #[serde(rename = "lomi_workspace_update")]
    RenameWorkspace(WorkspaceUpdateInput),
    #[serde(rename = "lomi_project_open")]
    OpenProject(ProjectOpenInput),
    #[serde(rename = "lomi_settings_open")]
    OpenSettings(SettingsOpenInput),
    #[serde(rename = "lomi_settings_read")]
    ReadSettings(SettingsReadInput),
    #[serde(rename = "lomi_settings_update")]
    UpdateSettings(SettingsUpdateInput),
    #[serde(rename = "lomi_project_close")]
    CloseProject(ProjectCloseInput),
    #[serde(rename = "lomi_workspace_create")]
    CreateWorkspace(WorkspaceRenameInput),
    #[serde(rename = "lomi_operation_cancel")]
    CancelOperation(OperationInput),
    #[serde(rename = "lomi_terminal_create")]
    CreateTerminal(TerminalCreateInput),
    #[serde(rename = "lomi_terminal_read")]
    ReadTerminal(TerminalReadInput),
    #[serde(rename = "lomi_terminal_run")]
    RunTerminal(TerminalRunInput),
    #[serde(rename = "lomi_terminal_input")]
    InputTerminal(TerminalInput),
    #[serde(rename = "lomi_terminal_interrupt")]
    InterruptTerminal(TerminalInterruptInput),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Message {
    pub ipc_version: u16,
    pub request_id: String,
    pub request: Request,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Response {
    pub request_id: String,
    pub result: Reply,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalOutput {
    pub mode: TerminalReadMode,
    pub workspace_id: String,
    pub panel_id: String,
    pub terminal_session_id: String,
    pub text: String,
    pub next_cursor: String,
    pub available_from: String,
    pub gap: bool,
    pub prompt: String,
    pub lease_id: Option<String>,
    pub command: Option<CommandObservation>,
    pub exited: bool,
    pub shell_exit_code: Option<u32>,
    pub next_input_sequence: String,
    pub stream_sequence: String,
    pub parsed_sequence: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Data {
    ChatList(Box<ChatList>),
    ChatRead(Box<ChatRead>),
    ChatExport(Box<ChatExport>),
    EditorText(Box<EditorText>),
    GitStatus(Box<GitStatusPage>),
    GitDiff(Box<GitDiffPage>),
    GitHistory(Box<GitHistoryPage>),
    GitCommit(Box<GitCommitDetails>),
    GitRemotes(Box<GitRemotes>),
    SettingsSnapshot(Box<SettingsSnapshot>),
    FileText(Box<FileText>),
    FilesList(Box<FilesList>),
    FilesSearch(Box<FilesSearch>),
    AndroidDevices {
        workspace_id: String,
        devices: AndroidDevices,
    },
    AndroidLogcat(Box<AndroidLogcat>),
    AndroidSetup(Box<AndroidSetupView>),
    AndroidSnapshot(Box<AndroidSnapshot>),
    Artifact {
        artifact: Box<Artifact>,
        /// Private IPC attachment; the MCP helper moves it to image content.
        #[schemars(skip)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    BrowserLogs(Box<BrowserLogs>),
    BrowserWait(Box<BrowserWaitResult>),
    BrowserSnapshot(Box<BrowserSnapshot>),
    Status {
        connection: String,
        pairing_request_id: Option<String>,
        instance_id: Option<String>,
        ui_ready: bool,
        platform: String,
        capabilities: Vec<Capability>,
        limitations: Vec<String>,
    },
    Connected {
        instance_id: String,
        workspace_id: String,
        project_id: String,
        retry_epoch: String,
        scopes: Vec<String>,
    },
    Events {
        items: Vec<ControlEvent>,
        next_cursor: String,
        has_more: bool,
        gap: bool,
    },
    Panels {
        items: Vec<PanelView>,
        next_cursor: Option<String>,
        domain_revision: String,
    },
    Workspaces {
        items: Vec<Workspace>,
        next_cursor: Option<String>,
        domain_revision: String,
    },
    TerminalInputAck {
        workspace_id: String,
        panel_id: String,
        terminal_session_id: String,
        input_sequence: String,
        dispatch: String,
    },
    TerminalOutput(Box<TerminalOutput>),
    TerminalScreen(Box<TerminalScreen>),
    TerminalRaw {
        workspace_id: String,
        panel_id: String,
        terminal_session_id: String,
        base64: String,
        next_cursor: String,
        available_from: String,
        gap: bool,
        stream_sequence: String,
    },
    Diagnostics {
        connection: String,
        ui_ready: bool,
        next_step: String,
    },
    Operation {
        operation_id: String,
        state: String,
        effect_state: String,
        workspace_id: Option<String>,
        result: Option<OperationResult>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Reply {
    Ok {
        control_api_version: String,
        data: Data,
    },
    Error {
        control_api_version: String,
        code: ErrorCode,
        message: String,
    },
}
impl Reply {
    pub fn ok(data: Data) -> Self {
        Self::Ok {
            control_api_version: CONTROL_API_VERSION.into(),
            data,
        }
    }
    pub fn error(code: ErrorCode, message: &str) -> Self {
        Self::Error {
            control_api_version: CONTROL_API_VERSION.into(),
            code,
            message: message.into(),
        }
    }
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}
