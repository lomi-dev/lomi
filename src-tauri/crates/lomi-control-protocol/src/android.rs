use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Restricted identifiers are also safe when passed as quoted guest-shell arguments.
pub fn valid_package(value: &str) -> bool {
    value.len() <= 255
        && value.contains('.')
        && value.split('.').all(|part| {
            part.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}
pub fn valid_activity(value: &str) -> bool {
    let value = value.strip_prefix('.').unwrap_or(value);
    !value.is_empty()
        && value.len() <= 512
        && value.split('.').all(|part| {
            part.as_bytes()
                .first()
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
        })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidLaunchInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    pub package_name: String,
    pub activity: Option<String>,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidLaunchResult {
    pub workspace_id: String,
    pub device_id: String,
    pub generation: String,
    pub package_name: String,
    pub activity: String,
    pub intent_delivered: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum AndroidLogPriority {
    V,
    D,
    I,
    W,
    E,
    F,
}
impl AndroidLogPriority {
    pub fn filter(self) -> &'static str {
        match self {
            Self::V => "V",
            Self::D => "D",
            Self::I => "I",
            Self::W => "W",
            Self::E => "E",
            Self::F => "F",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidLogcatInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    pub package_name: String,
    pub min_priority: AndroidLogPriority,
    pub limit: u16,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidLogcat {
    pub workspace_id: String,
    pub device_id: String,
    pub generation: String,
    pub package_name: String,
    pub process_id: u32,
    pub lines: Vec<String>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
    pub complete: bool,
    pub scope: String,
    pub gap: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidInstallInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    pub artifact_id: String,
    pub sha256: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidInstallResult {
    pub workspace_id: String,
    pub device_id: String,
    pub generation: String,
    pub artifact_id: String,
    pub sha256: String,
    pub installed: bool,
    pub package_name: Option<String>,
    pub previous_version: Option<String>,
    pub new_version: Option<String>,
    pub installer_failure: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidListInput {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidDevice {
    pub device_id: String,
    pub name: String,
    pub generation: Option<String>,
    pub phase: AndroidPhase,
    pub process_alive: bool,
    pub display: Option<[u32; 2]>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AndroidPhase {
    Stopped,
    Starting,
    Booting,
    Running,
    Stopping,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidDevices {
    pub devices_revision: String,
    pub host_qualified: bool,
    pub items: Vec<AndroidDevice>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidOpenInput {
    pub workspace_id: String,
    pub device_id: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}

pub fn valid_device_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidStartInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidStopInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidRuntimeResult {
    pub workspace_id: String,
    pub device_id: String,
    pub generation: String,
    pub ready: bool,
    pub stopped: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidControlInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    pub action: crate::control::PanelControlAction,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    pub lease_id: String,
    pub input_sequence: String,
    pub event: AndroidInputEvent,
    pub retry_epoch: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AndroidInputEvent {
    Rotate {
        quarter_turns: u8,
    },
    Text {
        text: String,
    },
    Key {
        key: String,
        down: bool,
    },
    Navigation {
        key: AndroidNavigation,
    },
    Touch {
        space: AndroidCoordinateSpace,
        identifier: u8,
        x: u32,
        y: u32,
        phase: AndroidTouchPhase,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AndroidNavigation {
    Back,
    Home,
    Overview,
    Power,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AndroidCoordinateSpace {
    HardwareDisplay,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AndroidTouchPhase {
    Down,
    Move,
    Up,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidControlResult {
    pub workspace_id: String,
    pub device_id: String,
    pub generation: String,
    pub controlled: bool,
    pub lease_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidInputResult {
    pub workspace_id: String,
    pub device_id: String,
    pub generation: String,
    pub input_sequence: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidSnapshotInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    #[serde(default = "snapshot_nodes")]
    pub max_nodes: u16,
    #[serde(default = "snapshot_bytes")]
    pub max_bytes: u32,
}
fn snapshot_nodes() -> u16 {
    250
}
fn snapshot_bytes() -> u32 {
    32768
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidNode {
    pub id: u16,
    pub parent: Option<u16>,
    pub class: String,
    pub resource_id: String,
    pub package: String,
    pub description: String,
    pub text: String,
    pub bounds: Option<[i32; 4]>,
    pub enabled: bool,
    pub clickable: bool,
    pub focused: bool,
    pub scrollable: bool,
    pub checked: bool,
    pub value_omitted: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidSnapshot {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    pub snapshot_id: String,
    pub captured_at_millis: String,
    pub hardware_display: [u32; 2],
    pub rotation: u8,
    pub coordinate_space: String,
    pub nodes: Vec<AndroidNode>,
    pub truncated: bool,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidScreenshotInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    #[serde(default = "screenshot_edge")]
    pub max_edge: u16,
    #[serde(default = "screenshot_bytes")]
    pub max_bytes: u32,
}
fn screenshot_edge() -> u16 {
    1280
}
fn screenshot_bytes() -> u32 {
    1024 * 1024
}

#[cfg(test)]
mod app_identity_tests {
    use super::*;
    #[test]
    fn app_identifiers_cannot_add_shell_syntax_options_or_components() {
        assert!(valid_package("org.example_app.Main2"));
        for package in [
            "",
            "android",
            "org..app",
            "1org.app",
            "org.app ",
            "org.app;id",
            "org.app$(id)",
            "org.app/Other",
            "-n",
            "org.app'",
            "org.app\n",
        ] {
            assert!(!valid_package(package), "{package}");
        }
        for activity in [".MainActivity", "MainActivity", "org.example.Main$Nested"] {
            assert!(valid_activity(activity));
        }
        for activity in [
            "",
            ".",
            "..Main",
            "-n",
            "Main/name",
            "Main;id",
            "Main'",
            "Main$(id)",
            "Main\n",
            "Main\\",
        ] {
            assert!(!valid_activity(activity), "{activity}");
        }
        assert!(!valid_activity(&"x".repeat(513)));
        assert!(!valid_package(&("org.".to_owned() + &"x".repeat(252))));
    }
}
