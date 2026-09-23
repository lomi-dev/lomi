use crate::{
    files::{TextEncoding, TextLineEndings},
    ErrorCode,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_chars() -> u16 {
    4096
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorReadInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub relative_path: String,
    pub document_id: Option<String>,
    pub expected_buffer_revision: Option<String>,
    #[serde(default)]
    pub start_utf16: u32,
    #[serde(default = "default_chars")]
    pub max_chars: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BufferSource {
    Buffer,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorText {
    pub workspace_id: String,
    pub panel_id: String,
    pub relative_path: String,
    pub document_id: String,
    pub buffer_revision: String,
    pub disk_revision: String,
    pub source: BufferSource,
    pub dirty: bool,
    pub conflict: bool,
    pub encoding: TextEncoding,
    pub line_endings: TextLineEndings,
    /// CodeMirror's normalized LF text. Offsets count UTF-16 code units.
    pub content: String,
    pub start_utf16: u32,
    pub total_utf16: u32,
    pub next_utf16: Option<u32>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorReadRequest {
    pub request_id: String,
    pub ui_epoch: String,
    pub project_path: String,
    pub project_id: String,
    pub input: EditorReadInput,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorReadReply {
    pub request_id: String,
    pub ui_epoch: String,
    /// Provenance of the bytes most recently loaded into the shared buffer.
    /// Kept on the trusted main/broker bridge, never exposed as an MCP path.
    pub source_path: Option<String>,
    pub text: Option<EditorText>,
    pub error: Option<ErrorCode>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorEdit {
    pub from_utf16: u32,
    pub to_utf16: u32,
    pub insert: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorEditsInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub relative_path: String,
    pub document_id: String,
    pub expected_buffer_revision: String,
    pub expected_disk_revision: String,
    pub edits: Vec<EditorEdit>,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
impl EditorEditsInput {
    pub fn read_target(&self) -> EditorReadInput {
        EditorReadInput {
            workspace_id: self.workspace_id.clone(),
            panel_id: self.panel_id.clone(),
            relative_path: self.relative_path.clone(),
            document_id: Some(self.document_id.clone()),
            expected_buffer_revision: Some(self.expected_buffer_revision.clone()),
            start_utf16: 0,
            max_chars: 2,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorEditsCommand {
    pub workspace_id: String,
    pub project_path: String,
    pub not_after_millis: String,
    pub input: EditorEditsInput,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorEdited {
    pub workspace_id: String,
    pub panel_id: String,
    pub relative_path: String,
    pub document_id: String,
    pub previous_buffer_revision: String,
    pub buffer_revision: String,
    pub disk_revision: String,
    pub dirty: bool,
    pub edit_count: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorOpenInput {
    pub workspace_id: String,
    pub relative_path: String,
    #[serde(default)]
    pub presentation: EditorPresentation,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorOpenCommand {
    pub workspace_id: String,
    pub relative_path: String,
    pub presentation: EditorPresentation,
    pub project_path: String,
    pub not_after_millis: String,
}
/// Native-to-main data only. The MCP receipt contains metadata, never this body.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedEditorFile {
    pub path: String,
    pub relative: String,
    pub revision: String,
    pub read_only: bool,
    pub body: PreparedEditorBody,
    pub asset_permit: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EditorPresentation {
    #[default]
    Editor,
    Preview,
    Split,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditorFileKind {
    Text,
    Markdown,
    Svg,
    Image,
}
impl EditorFileKind {
    pub fn from_relative(relative: &str) -> Self {
        let extension = relative
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase());
        match extension.as_deref() {
            Some("md" | "markdown") => Self::Markdown,
            Some("svg") => Self::Svg,
            Some(
                "png" | "apng" | "jpg" | "jpeg" | "jpe" | "jfif" | "webp" | "gif" | "avif" | "ico"
                | "bmp" | "dib" | "tif" | "tiff" | "tga" | "dds" | "pbm" | "pgm" | "ppm" | "pam"
                | "pnm" | "qoi" | "hdr" | "exr" | "ff",
            ) => Self::Image,
            _ => Self::Text,
        }
    }
    pub fn supports(self, presentation: EditorPresentation) -> bool {
        match self {
            Self::Image => presentation == EditorPresentation::Preview,
            Self::Text => presentation == EditorPresentation::Editor,
            _ => true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparedEditorBody {
    Text {
        content: String,
        encoding: TextEncoding,
    },
    Image(PreparedPreviewImage),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedPreviewImage {
    pub data_base64: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorOpened {
    pub workspace_id: String,
    pub panel_id: String,
    pub relative_path: String,
    pub document_id: String,
    pub buffer_revision: String,
    pub disk_revision: String,
    pub dirty: bool,
    pub presentation: EditorPresentation,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorPreviewed {
    pub workspace_id: String,
    pub panel_id: String,
    pub relative_path: String,
    pub disk_revision: String,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorSaveInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub relative_path: String,
    pub document_id: String,
    pub expected_buffer_revision: String,
    pub expected_disk_revision: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
impl EditorSaveInput {
    pub fn read_target(&self) -> EditorReadInput {
        EditorReadInput {
            workspace_id: self.workspace_id.clone(),
            panel_id: self.panel_id.clone(),
            relative_path: self.relative_path.clone(),
            document_id: Some(self.document_id.clone()),
            expected_buffer_revision: Some(self.expected_buffer_revision.clone()),
            start_utf16: 0,
            max_chars: 2,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorSaveCommand {
    pub workspace_id: String,
    pub project_path: String,
    pub not_after_millis: String,
    pub input: EditorSaveInput,
}
/// Trusted main-to-native captured buffer. Never accepted in MCP arguments.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorSaveBody {
    pub document_id: String,
    pub buffer_revision: String,
    pub disk_revision: String,
    pub source_path: String,
    pub content: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorSaved {
    pub workspace_id: String,
    pub panel_id: String,
    pub relative_path: String,
    pub document_id: String,
    pub saved_buffer_revision: String,
    pub previous_disk_revision: String,
    pub disk_revision: String,
    pub byte_length: u32,
}
