use super::{installer::Progress, runtime::Status};

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Event {
    Operation(Progress),
    Status(Status),
    Stream(super::frames::Status),
    InputError(String),
    InputControl(super::input::AgentInputState),
    Metadata,
}
