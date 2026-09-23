use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DockSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PanelMove {
    ReorderTab {
        tab_id: String,
        before_tab_id: Option<String>,
    },
    DockTab {
        tab_id: String,
        target_tab_id: String,
        side: DockSide,
    },
    MovePane {
        panel_id: String,
        target_panel_id: String,
        side: DockSide,
    },
    TransferTab {
        tab_id: String,
        target_workspace_id: String,
        before_tab_id: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelMoveInput {
    pub workspace_id: String,
    pub movement: PanelMove,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelMoveIdentity {
    pub panel_id: String,
    pub tab_id: String,
    pub kind: String,
    pub terminal_session_id: Option<String>,
    pub browser_generation: Option<String>,
    pub android_device_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelMoveCommand {
    pub tab_order: Vec<String>,
    pub focused_panel_id: Option<String>,
    pub workspace_id: String,
    pub movement: PanelMove,
    pub panels: Vec<PanelMoveIdentity>,
    pub destination: Option<PanelMoveDestination>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelMoveDestination {
    pub workspace_id: String,
    pub tab_order: Vec<String>,
    pub panels: Vec<PanelMoveIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelMoved {
    pub workspace_id: String,
    pub movement: PanelMove,
    pub panels: Vec<PanelMoveIdentity>,
    pub destination: Option<PanelMoveDestination>,
}
