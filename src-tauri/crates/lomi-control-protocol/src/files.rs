use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesMutateInput {
    pub workspace_id: String,
    pub operation: FileMutation,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum FileMutation {
    Trash {
        relative_path: String,
        kind: FileEntryKind,
        expected_entry_revision: String,
        expected_parent_revision: String,
    },
    Create {
        relative_path: String,
        kind: FileEntryKind,
        expected_parent_revision: String,
    },
    Rename {
        relative_path: String,
        new_name: String,
        expected_disk_revision: String,
        expected_parent_revision: String,
    },
    Move {
        relative_path: String,
        target_relative_path: String,
        expected_disk_revision: String,
        expected_parent_revision: String,
    },
    RenameDirectory {
        relative_path: String,
        new_name: String,
        expected_directory_revision: String,
        expected_parent_revision: String,
    },
    MoveDirectory {
        relative_path: String,
        target_relative_path: String,
        expected_directory_revision: String,
        expected_parent_revision: String,
    },
}
impl FileMutation {
    pub fn is_trash(&self) -> bool {
        matches!(self, Self::Trash { .. })
    }
    pub fn relative_path(&self) -> &str {
        match self {
            Self::Trash { relative_path, .. }
            | Self::Create { relative_path, .. }
            | Self::Rename { relative_path, .. }
            | Self::Move { relative_path, .. }
            | Self::RenameDirectory { relative_path, .. }
            | Self::MoveDirectory { relative_path, .. } => relative_path,
        }
    }
    pub fn destination(&self) -> String {
        match self {
            Self::Trash { relative_path, .. } | Self::Create { relative_path, .. } => {
                relative_path.clone()
            }
            Self::Move {
                target_relative_path,
                ..
            }
            | Self::MoveDirectory {
                target_relative_path,
                ..
            } => target_relative_path.clone(),
            Self::Rename {
                relative_path,
                new_name,
                ..
            }
            | Self::RenameDirectory {
                relative_path,
                new_name,
                ..
            } => relative_path.rsplit_once('/').map_or_else(
                || new_name.clone(),
                |(parent, _)| format!("{parent}/{new_name}"),
            ),
        }
    }
    pub fn expected_parent_revision(&self) -> &str {
        match self {
            Self::Trash {
                expected_parent_revision,
                ..
            }
            | Self::Create {
                expected_parent_revision,
                ..
            }
            | Self::Rename {
                expected_parent_revision,
                ..
            }
            | Self::Move {
                expected_parent_revision,
                ..
            }
            | Self::RenameDirectory {
                expected_parent_revision,
                ..
            }
            | Self::MoveDirectory {
                expected_parent_revision,
                ..
            } => expected_parent_revision,
        }
    }
    pub fn scope(&self) -> &'static str {
        match self {
            Self::Trash { .. } => "files.trash",
            Self::Create { .. } => "files.create",
            Self::Rename { .. }
            | Self::Move { .. }
            | Self::RenameDirectory { .. }
            | Self::MoveDirectory { .. } => "files.rename",
        }
    }
}
/// Native main-view observation, never supplied as MCP authorization.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileTrashBuffer {
    pub document_id: String,
    pub relative_path: String,
    pub buffer_revision: String,
    pub disk_revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileTrashPlan {
    pub plan_hash: String,
    pub awaiting_user: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesMutateCommand {
    pub workspace_id: String,
    pub project_path: String,
    pub not_after_millis: String,
    pub input: FilesMutateInput,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesMutated {
    pub workspace_id: String,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub entry_kind: FileEntryKind,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FileSearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub include: String,
    pub exclude: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesSearchInput {
    pub workspace_id: String,
    #[serde(default)]
    pub relative_directory: String,
    pub query: FileSearchQuery,
    #[serde(default = "default_limit")]
    pub limit: u16,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileSearchMatch {
    pub relative_path: String,
    pub disk_revision: String,
    /// One-based line/column; column and length count UTF-16 code units.
    pub line: u32,
    pub column: u32,
    pub length: u32,
    pub preview: String,
    pub preview_start_utf16: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchConsistency {
    PerFileSnapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchFilterPolicy {
    ExplicitPatternsAndSecretPaths,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesSearch {
    pub workspace_id: String,
    pub relative_directory: String,
    pub matches: Vec<FileSearchMatch>,
    pub next_cursor: Option<String>,
    pub limited: bool,
    pub skipped: u32,
    pub files_scanned: u32,
    pub filtered: bool,
    pub consistency: SearchConsistency,
    pub filter_policy: SearchFilterPolicy,
}

fn default_limit() -> u16 {
    100
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesListInput {
    pub workspace_id: String,
    #[serde(default)]
    pub relative_directory: String,
    #[serde(default = "default_limit")]
    pub limit: u16,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileEntryKind {
    Directory,
    File,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileEntry {
    pub name: String,
    pub relative_path: String,
    pub kind: FileEntryKind,
    /// Decimal bytes avoid loss of precision for large host files.
    pub byte_length: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesList {
    pub workspace_id: String,
    pub relative_directory: String,
    pub directory_revision: String,
    pub entries: Vec<FileEntry>,
    pub next_cursor: Option<String>,
    pub filtered: bool,
}

fn default_chars() -> u16 {
    4096
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesReadInput {
    pub workspace_id: String,
    pub relative_path: String,
    #[serde(default)]
    pub start_utf16: u32,
    #[serde(default = "default_chars")]
    pub max_chars: u16,
    pub expected_disk_revision: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiskSource {
    Disk,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TextLineEndings {
    None,
    Lf,
    CrLf,
    Cr,
    Mixed,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileText {
    pub workspace_id: String,
    pub relative_path: String,
    pub source: DiskSource,
    pub disk_revision: String,
    pub encoding: TextEncoding,
    pub line_endings: TextLineEndings,
    pub content: String,
    pub start_utf16: u32,
    pub total_utf16: u32,
    pub next_utf16: Option<u32>,
    pub truncated: bool,
}
