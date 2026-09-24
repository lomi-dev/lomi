use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserSnapshotInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    #[serde(default = "default_nodes")]
    pub max_nodes: u16,
    #[serde(default = "default_bytes")]
    pub max_bytes: u32,
}
fn default_nodes() -> u16 {
    100
}
fn default_bytes() -> u32 {
    16384
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserFrame {
    pub frame_id: String,
    pub parent_frame_id: Option<String>,
    pub origin: String,
    pub url: String,
    pub viewport_ref: String,
    pub viewport: BrowserViewport,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserSnapshot {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub snapshot_id: String,
    pub snapshot_kind: String,
    pub frame_id: String,
    pub origin: String,
    pub url: String,
    pub captured_at: String,
    pub viewport: BrowserViewport,
    pub frames: Vec<BrowserFrame>,
    pub elements: Vec<BrowserElement>,
    pub truncated: bool,
    pub omitted_frames: u16,
    pub limitations: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserViewport {
    pub width: f64,
    pub height: f64,
    pub device_scale_factor: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserElement {
    pub frame_id: String,
    pub element_ref: String,
    pub role: String,
    pub name: String,
    pub enabled: bool,
    pub editable: bool,
    pub checked: Option<bool>,
    pub value_length: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserClickInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub snapshot_id: String,
    pub element_ref: String,
    pub lease_id: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserFillInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub snapshot_id: String,
    pub element_ref: String,
    pub lease_id: String,
    pub retry_epoch: String,
    pub request_key: String,
    pub text: String,
}
impl BrowserFillInput {
    pub fn split(self) -> (BrowserClickInput, BrowserInteraction) {
        (
            BrowserClickInput {
                workspace_id: self.workspace_id,
                panel_id: self.panel_id,
                browser_generation: self.browser_generation,
                navigation_id: self.navigation_id,
                snapshot_id: self.snapshot_id,
                element_ref: self.element_ref,
                lease_id: self.lease_id,
                retry_epoch: self.retry_epoch,
                request_key: self.request_key,
            },
            BrowserInteraction::Fill { text: self.text },
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BrowserInteraction {
    Click,
    Key { key: BrowserKey },
    Scroll { delta_x: f64, delta_y: f64 },
    Fill { text: String },
}
impl BrowserInteraction {
    pub fn tool(&self) -> &'static str {
        match self {
            Self::Click => "lomi_browser_click",
            Self::Key { .. } => "lomi_browser_key",
            Self::Scroll { .. } => "lomi_browser_scroll",
            Self::Fill { .. } => "lomi_browser_fill",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserDomCommand {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub snapshot_id: String,
    pub element_ref: String,
    pub interaction: BrowserInteraction,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserInteractionResult {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub snapshot_id: String,
    pub element_ref: String,
    pub dispatched: bool,
    pub input_mode: String,
    pub default_action: Option<bool>,
    pub scroll_position: Option<BrowserScrollPosition>,
    pub value_length: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserKeyInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub snapshot_id: String,
    pub element_ref: String,
    pub lease_id: String,
    pub retry_epoch: String,
    pub request_key: String,
    pub key: BrowserKey,
}
impl BrowserKeyInput {
    pub fn split(self) -> (BrowserClickInput, BrowserInteraction) {
        (
            BrowserClickInput {
                workspace_id: self.workspace_id,
                panel_id: self.panel_id,
                browser_generation: self.browser_generation,
                navigation_id: self.navigation_id,
                snapshot_id: self.snapshot_id,
                element_ref: self.element_ref,
                lease_id: self.lease_id,
                retry_epoch: self.retry_epoch,
                request_key: self.request_key,
            },
            BrowserInteraction::Key { key: self.key },
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserScrollInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub snapshot_id: String,
    #[serde(default = "default_viewport_ref")]
    pub viewport_ref: String,
    pub lease_id: String,
    pub retry_epoch: String,
    pub request_key: String,
    pub delta_x: f64,
    pub delta_y: f64,
}
fn default_viewport_ref() -> String {
    "viewport".into()
}
impl BrowserScrollInput {
    pub fn split(self) -> (BrowserClickInput, BrowserInteraction) {
        (
            BrowserClickInput {
                workspace_id: self.workspace_id,
                panel_id: self.panel_id,
                browser_generation: self.browser_generation,
                navigation_id: self.navigation_id,
                snapshot_id: self.snapshot_id,
                element_ref: self.viewport_ref,
                lease_id: self.lease_id,
                retry_epoch: self.retry_epoch,
                request_key: self.request_key,
            },
            BrowserInteraction::Scroll {
                delta_x: self.delta_x,
                delta_y: self.delta_y,
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
pub enum BrowserKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserScrollPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserWaitInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub condition: BrowserWaitCondition,
    #[serde(default = "wait_timeout")]
    pub timeout_ms: u32,
}
fn wait_timeout() -> u32 {
    5000
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserWaitCondition {
    Text { text: String },
    Element { role: String, name: String },
    Url { url: String },
    Load,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserWaitResult {
    pub matched: bool,
    pub elapsed_ms: u32,
    pub snapshot: BrowserSnapshot,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_scroll_preserves_main_viewport_compatibility() {
        let mut input = serde_json::json!({"workspaceId":"w","panelId":"p","browserGeneration":"g","navigationId":"n","snapshotId":"s","leaseId":"l","retryEpoch":"r","requestKey":"k","deltaX":0,"deltaY":10});
        let old: BrowserScrollInput = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(old.split().0.element_ref, "viewport");
        input["viewportRef"] = "f1-viewport".into();
        let frame: BrowserScrollInput = serde_json::from_value(input).unwrap();
        assert_eq!(frame.split().0.element_ref, "f1-viewport");
    }
}
