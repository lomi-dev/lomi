use super::operations::Work;
use super::*;

#[derive(Clone)]
pub struct AndroidCloseTarget {
    pub device: String,
    pub control: Option<Arc<crate::android::AndroidControl>>,
    pub generation: Option<String>,
    pub last_view: bool,
}
pub type AndroidCloseDispatch =
    Arc<dyn Fn(&[AndroidCloseTarget], Option<NativePermit>) -> Result<(), ErrorCode> + Send + Sync>;

impl Broker {
    pub fn set_android_close_dispatch(&self, dispatch: AndroidCloseDispatch) -> io::Result<()> {
        *self.android_close_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }

    pub(super) fn android_panel_access(
        state: &State,
        owner: &str,
        panel: &str,
        reveal: bool,
    ) -> Result<(), ErrorCode> {
        let panel = state
            .projection
            .panels
            .iter()
            .find(|p| p.id == panel && p.kind == "android")
            .ok_or(ErrorCode::TargetNotFound)?;
        let devices = Self::android_access(state, owner, &panel.workspace_id)?;
        let device = panel
            .android_device_id
            .as_ref()
            .ok_or(ErrorCode::TargetNotFound)?;
        if !devices.contains(device) {
            return Err(ErrorCode::ScopeDenied);
        }
        if let Some(target) = state.android.get(device) {
            if target.owner != owner || !target.workspaces.contains(&panel.workspace_id) {
                return Err(ErrorCode::TargetBusy);
            }
            target.control.check()?;
        }
        if reveal {
            let target = state.android.get(device).ok_or(ErrorCode::UiNotReady)?;
            let generation = target.control.generation()?.ok_or(ErrorCode::UiNotReady)?;
            Self::android_runtime_access(
                state,
                owner,
                &panel.workspace_id,
                &panel.id,
                device,
                Some(&generation),
            )?;
        }
        Ok(())
    }

    // Native generations are private admission bindings, independent of UI revisions.
    pub(super) fn android_layout_bindings(
        state: &State,
        action: &UiAction,
    ) -> Vec<(String, Option<String>)> {
        let selected = |p: &&Panel| match action {
            UiAction::FocusPanel {
                workspace_id,
                tab_id,
                ..
            }
            | UiAction::SelectWorkspace {
                workspace_id,
                tab_id,
                ..
            } => &p.workspace_id == workspace_id && &p.tab_id == tab_id,
            UiAction::ClosePanel { panel_id, .. } => &p.id == panel_id,
            UiAction::MovePanel(c) => {
                p.workspace_id == c.workspace_id
                    && match &c.movement {
                        PanelMove::ReorderTab { tab_id, .. }
                        | PanelMove::TransferTab { tab_id, .. } => &p.tab_id == tab_id,
                        PanelMove::DockTab {
                            tab_id,
                            target_tab_id,
                            ..
                        } => &p.tab_id == tab_id || &p.tab_id == target_tab_id,
                        PanelMove::MovePane { panel_id, .. } => c.panels.iter().any(|source| {
                            &source.panel_id == panel_id && source.tab_id == p.tab_id
                        }),
                    }
            }
            UiAction::CloseWorkspace(c) => p.workspace_id == c.workspace_id,
            UiAction::CloseProject(c) => c
                .workspaces
                .iter()
                .any(|w| w.workspace_id == p.workspace_id),
            _ => false,
        };
        let mut bindings: Vec<_> = state
            .projection
            .panels
            .iter()
            .filter(|p| p.kind == "android")
            .filter(selected)
            .filter_map(|p| p.android_device_id.as_ref())
            .map(|id| {
                (
                    id.clone(),
                    state
                        .android
                        .get(id)
                        .and_then(|t| t.control.generation().ok().flatten()),
                )
            })
            .collect();
        bindings.sort();
        bindings.dedup();
        bindings
    }

    pub(super) fn validate_android_layout(state: &State, work: &Work) -> Result<(), ErrorCode> {
        for (device, generation) in &work.android_layout {
            let current = state
                .android
                .get(device)
                .map(|t| t.control.generation())
                .transpose()?
                .flatten();
            if &current != generation {
                return Err(ErrorCode::StaleGeneration);
            }
        }
        Ok(())
    }

    pub(super) fn android_close_plan(
        &self,
        state: &State,
        owner: &str,
        panels: &[String],
    ) -> Result<Option<(Vec<AndroidCloseTarget>, AndroidCloseDispatch)>, ErrorCode> {
        let mut targets = Vec::new();
        for panel in state
            .projection
            .panels
            .iter()
            .filter(|p| p.kind == "android" && panels.contains(&p.id))
        {
            Self::android_panel_access(state, owner, &panel.id, false)?;
            let device = panel
                .android_device_id
                .as_ref()
                .ok_or(ErrorCode::TargetNotFound)?;
            if targets
                .iter()
                .any(|t: &AndroidCloseTarget| &t.device == device)
            {
                continue;
            }
            let last_view = !state.projection.panels.iter().any(|p| {
                p.kind == "android"
                    && p.android_device_id.as_ref() == Some(device)
                    && !panels.contains(&p.id)
            });
            if last_view
                && !state.sessions[owner]
                    .grant
                    .scopes
                    .contains("android.control")
            {
                return Err(ErrorCode::ScopeDenied);
            }
            let control = state.android.get(device).map(|t| t.control.clone());
            let generation = control
                .as_ref()
                .map(|c| c.generation())
                .transpose()?
                .flatten();
            targets.push(AndroidCloseTarget {
                device: device.clone(),
                control,
                generation,
                last_view,
            });
        }
        if targets.is_empty() {
            return Ok(None);
        }
        let dispatch = self
            .android_close_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
            .ok_or(ErrorCode::HostUnqualified)?;
        dispatch(&targets, None)?;
        Ok(Some((targets, dispatch)))
    }
    pub(super) fn preflight_android_close(
        &self,
        state: &State,
        owner: &str,
        panels: &[String],
    ) -> Result<(), ErrorCode> {
        self.android_close_plan(state, owner, panels).map(|_| ())
    }
}
