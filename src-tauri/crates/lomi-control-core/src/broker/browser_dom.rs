use super::*;
use crate::browser::BrowserControl;

/// Native-only producer. The application registers this adapter; neither the
/// renderer nor a browser child can provide its implementation or answer.
pub type BrowserSnapshotDispatch = Arc<
    dyn Fn(
            Arc<BrowserControl>,
            BrowserSnapshotInput,
            String,
            String,
            Instant,
        ) -> Result<BrowserSnapshot, ErrorCode>
        + Send
        + Sync,
>;

impl Broker {
    pub fn set_browser_snapshot_dispatch(
        &self,
        dispatch: BrowserSnapshotDispatch,
    ) -> io::Result<()> {
        *self
            .browser_snapshot_dispatch
            .lock()
            .map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }
    pub(super) fn snapshot_browser(&self, owner: &str, input: BrowserSnapshotInput) -> Reply {
        self.snapshot_browser_until(owner, input, Instant::now() + Duration::from_secs(3))
    }
    fn snapshot_browser_until(
        &self,
        owner: &str,
        input: BrowserSnapshotInput,
        deadline: Instant,
    ) -> Reply {
        if !(1..=500).contains(&input.max_nodes) || !(1024..=49152).contains(&input.max_bytes) {
            return error(ErrorCode::ResourceExhausted);
        }
        let control = {
            let Ok(state) = self.lock_state() else {
                return error(ErrorCode::ControlRevoked);
            };
            match Self::browser_target(
                &state,
                owner,
                &input.workspace_id,
                &input.panel_id,
                &input.browser_generation,
                "browser.read",
            ) {
                Ok(t) => t.control.clone(),
                Err(e) => return error(e),
            }
        };
        let _guard = match control.begin_dom() {
            Ok(g) => g,
            Err(e) => return error(e),
        };
        let Some(dispatch) = self
            .browser_snapshot_dispatch
            .lock()
            .ok()
            .and_then(|d| d.clone())
        else {
            return error(ErrorCode::UnsupportedCapability);
        };
        let Ok(snapshot) = new_id() else {
            return error(ErrorCode::ResourceExhausted);
        };
        let navigation = control.navigation_id();
        let result = match dispatch(
            control.clone(),
            input.clone(),
            snapshot.clone(),
            navigation.clone(),
            deadline,
        ) {
            Ok(r) => r,
            Err(e) => return error(e),
        };
        let Ok(state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let target = match Self::browser_target(
            &state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.browser_generation,
            "browser.read",
        ) {
            Ok(t) => t,
            Err(e) => return error(e),
        };
        if !Arc::ptr_eq(&control, &target.control) {
            return error(ErrorCode::StaleGeneration);
        }
        if result.workspace_id != input.workspace_id
            || result.panel_id != input.panel_id
            || result.browser_generation != input.browser_generation
            || result.snapshot_id != snapshot
            || result.navigation_id != navigation
            || result.frame_id != "main"
            || result.snapshot_kind != "dom"
            || !valid_frames(&result)
            || !control.permits(&result.url)
            || result.elements.len() > input.max_nodes as usize
            || serde_json::to_vec(&result).map_or(true, |v| v.len() > input.max_bytes as usize)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        if let Err(e) = control.retain_snapshot(&snapshot, &navigation) {
            return error(e);
        }
        Reply::ok(Data::BrowserSnapshot(Box::new(result)))
    }
}

impl Broker {
    pub fn interact_browser(
        self: &Arc<Self>,
        owner: &str,
        input: BrowserClickInput,
        interaction: BrowserInteraction,
    ) -> Reply {
        if !valid_id(&input.request_key)
            || !valid_id(&input.snapshot_id)
            || !valid_id(&input.element_ref)
            || matches!(&interaction, BrowserInteraction::Fill { text } if text.len() > 16384 || text.contains('\0'))
            || matches!(&interaction, BrowserInteraction::Scroll {delta_x,delta_y} if !delta_x.is_finite() || !delta_y.is_finite() || delta_x.abs()>10000. || delta_y.abs()>10000.)
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let Some(session) = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
        else {
            return error(ErrorCode::ControlRevoked);
        };
        let Some(root) = session.grant.workspace(&input.workspace_id) else {
            return error(ErrorCode::TargetNotFound);
        };
        let project = root.project_id.clone();
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let tool = interaction.tool();
        let hash = match receipts::fingerprint(
            &(&input, &interaction),
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.panel_id,
                generation: &input.browser_generation,
                revision: &input.navigation_id,
            },
        ) {
            Ok(h) => h,
            Err(e) => return operations::storage_error(e),
        };
        match self.replay(
            &state,
            owner,
            &receipts::Key {
                pairing_id: owner,
                project_id: &project,
                retry_epoch: &input.retry_epoch,
                request_key: &input.request_key,
                tool,
            },
            hash,
        ) {
            Ok(Some(r)) => return r,
            Err(r) => return *r,
            Ok(None) => {}
        }
        let target = match Self::browser_target(
            &state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.browser_generation,
            "browser.interact",
        ) {
            Ok(t) => t,
            Err(e) => return error(e),
        };
        if target.control.lease() != Some(input.lease_id.as_str()) {
            return error(ErrorCode::ControlRevoked);
        }
        if let Err(e) = target.control.require_renderable() {
            return error(e);
        }
        if let Err(e) = target
            .control
            .check_snapshot(&input.snapshot_id, &input.navigation_id)
        {
            return error(e);
        }
        let action = UiAction::InteractBrowser(BrowserDomCommand {
            workspace_id: input.workspace_id.clone(),
            panel_id: input.panel_id,
            browser_generation: input.browser_generation,
            navigation_id: input.navigation_id,
            snapshot_id: input.snapshot_id,
            element_ref: input.element_ref,
            interaction,
        });
        let revision = state.projection.revision.clone();
        self.enqueue_ui(
            &mut state,
            owner,
            operations::UiMutation {
                workspace: input.workspace_id,
                project,
                revision,
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool,
                hash,
                action,
            },
        )
    }
    pub fn authorize_browser_interaction(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<(Arc<BrowserControl>, BrowserDomCommand, NativePermit), ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
        let UiAction::InteractBrowser(command) = &work.command.action else {
            return Err(ErrorCode::ControlRevoked);
        };
        if !work.claimed
            || work.native_committed
            || work.command.nonce != nonce
            || work.command.ui_epoch != state.projection.ui_epoch
            || work.deadline <= Instant::now()
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let control = Self::browser_target(
            &state,
            &work.pairing,
            &command.workspace_id,
            &command.panel_id,
            &command.browser_generation,
            "browser.interact",
        )?
        .control
        .clone();
        control.check_snapshot(&command.snapshot_id, &command.navigation_id)?;
        control.require_renderable()?;
        let permit = work.native_permit.clone();
        permit.check()?;
        let command = command.clone();
        state.work.get_mut(operation).unwrap().native_committed = true;
        Ok((control, command, permit))
    }
}

impl Broker {
    fn finish_wait(
        &self,
        owner: &str,
        input: &BrowserWaitInput,
        snapshot: BrowserSnapshot,
        start: Instant,
        matched: bool,
    ) -> Reply {
        let Ok(state) = self.lock_state() else {
            return error(ErrorCode::ControlRevoked);
        };
        let target = match Self::browser_target(
            &state,
            owner,
            &input.workspace_id,
            &input.panel_id,
            &input.browser_generation,
            "browser.read",
        ) {
            Ok(t) => t,
            Err(e) => return error(e),
        };
        if let Err(e) = target
            .control
            .check_snapshot(&snapshot.snapshot_id, &snapshot.navigation_id)
        {
            return error(e);
        }
        Reply::ok(Data::BrowserWait(Box::new(BrowserWaitResult {
            matched,
            elapsed_ms: start.elapsed().as_millis().min(u32::MAX as u128) as u32,
            snapshot,
        })))
    }
    pub(super) fn wait_browser(&self, owner: &str, input: BrowserWaitInput) -> Reply {
        if !(1..=10000).contains(&input.timeout_ms)
            || match &input.condition {
                BrowserWaitCondition::Text { text } => text.is_empty() || text.len() > 512,
                BrowserWaitCondition::Element { role, name } => {
                    role.is_empty() || role.len() > 64 || name.len() > 512
                }
                BrowserWaitCondition::Url { url } => {
                    lomi_control_protocol::browser::address(url).is_err()
                }
                BrowserWaitCondition::Load => false,
            }
        {
            return error(ErrorCode::ResourceExhausted);
        }
        let start = Instant::now();
        let deadline = start + Duration::from_millis(input.timeout_ms.into());
        let mut latest: Option<Box<BrowserSnapshot>> = None;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining <= Duration::from_millis(150) {
                if let Some(snapshot) = latest.take() {
                    std::thread::sleep(remaining);
                    return self.finish_wait(owner, &input, *snapshot, start, false);
                }
            }
            let reply = self.snapshot_browser_until(
                owner,
                BrowserSnapshotInput {
                    workspace_id: input.workspace_id.clone(),
                    panel_id: input.panel_id.clone(),
                    browser_generation: input.browser_generation.clone(),
                    max_nodes: 500,
                    max_bytes: 49152,
                },
                deadline.min(Instant::now() + Duration::from_secs(3)),
            );
            match reply {
                Reply::Ok {
                    data: Data::BrowserSnapshot(snapshot),
                    ..
                } => {
                    let matched = match &input.condition {
                        BrowserWaitCondition::Text { text } => {
                            snapshot.elements.iter().any(|e| e.name.contains(text))
                        }
                        BrowserWaitCondition::Element { role, name } => snapshot
                            .elements
                            .iter()
                            .any(|e| e.role == *role && e.name == *name),
                        BrowserWaitCondition::Url { url } => snapshot.url == *url,
                        BrowserWaitCondition::Load => true,
                    };
                    if matched || Instant::now() >= deadline {
                        return self.finish_wait(owner, &input, *snapshot, start, matched);
                    }
                    latest = Some(snapshot);
                }
                Reply::Error {
                    code: ErrorCode::TargetBusy | ErrorCode::StaleSnapshot,
                    ..
                } if Instant::now() < deadline => {}
                other => return other,
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return error(ErrorCode::DeadlineExceeded);
            }
            std::thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }
}

// Frames are output metadata, never a caller-selected origin or script target.
fn valid_frames(snapshot: &BrowserSnapshot) -> bool {
    use lomi_control_protocol::browser::address;
    let Ok(root) = address(&snapshot.url) else {
        return false;
    };
    if root.origin().ascii_serialization() != snapshot.origin
        || snapshot.frames.is_empty()
        || snapshot.frames.len() > 16
    {
        return false;
    }
    let mut frames = HashSet::new();
    let mut refs = HashSet::new();
    for (index, frame) in snapshot.frames.iter().enumerate() {
        if !valid_id(&frame.frame_id)
            || !valid_id(&frame.viewport_ref)
            || !refs.insert(&frame.viewport_ref)
            || frame.origin != snapshot.origin
            || address(&frame.url).map_or(true, |url| url.origin() != root.origin())
            || !frame.viewport.width.is_finite()
            || frame.viewport.width < 0.
            || !frame.viewport.height.is_finite()
            || frame.viewport.height < 0.
            || !frame.viewport.device_scale_factor.is_finite()
            || frame.viewport.device_scale_factor <= 0.
            || if index == 0 {
                frame.frame_id != "main"
                    || frame.parent_frame_id.is_some()
                    || frame.url != snapshot.url
                    || frame.viewport_ref != "viewport"
            } else {
                frame
                    .parent_frame_id
                    .as_ref()
                    .is_none_or(|parent| !frames.contains(parent))
            }
            || !frames.insert(&frame.frame_id)
        {
            return false;
        }
    }
    snapshot.elements.iter().all(|element| {
        frames.contains(&element.frame_id)
            && valid_id(&element.element_ref)
            && refs.insert(&element.element_ref)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> BrowserSnapshot {
        serde_json::from_value(json!({
            "workspaceId":"workspace","panelId":"panel","browserGeneration":"generation",
            "navigationId":"nav","snapshotId":"snapshot","snapshotKind":"dom",
            "frameId":"main","origin":"https://example.test","url":"https://example.test/",
            "capturedAt":"fixture","viewport":{"width":800.,"height":600.,"deviceScaleFactor":2.},
            "frames":[
                {"frameId":"main","parentFrameId":null,"origin":"https://example.test","url":"https://example.test/","viewportRef":"viewport","viewport":{"width":800.,"height":600.,"deviceScaleFactor":2.}},
                {"frameId":"f1","parentFrameId":"main","origin":"https://example.test","url":"https://example.test/child","viewportRef":"f1-viewport","viewport":{"width":600.,"height":400.,"deviceScaleFactor":2.}}
            ],
            "elements":[{"frameId":"f1","elementRef":"f1-e1","role":"button","name":"Save","enabled":true,"editable":false,"checked":null,"valueLength":null}],
            "truncated":false,"omittedFrames":0,"limitations":[]
        })).unwrap()
    }
    #[test]
    fn frame_projection_rejects_foreign_origins_and_ambiguous_refs() {
        let value = snapshot();
        assert!(valid_frames(&value));
        let mut forged = value.clone();
        forged.frames[1].url = "https://foreign.test/child".into();
        assert!(!valid_frames(&forged));
        forged.frames[1].origin = "https://foreign.test".into();
        assert!(!valid_frames(&forged));
        let mut forged = value.clone();
        forged.elements[0].frame_id = "foreign".into();
        assert!(!valid_frames(&forged));
        forged.elements[0].frame_id = "f1".into();
        forged.elements[0].element_ref = "viewport".into();
        assert!(!valid_frames(&forged));
        let mut forged = value.clone();
        forged.frames[1].parent_frame_id = Some("missing".into());
        assert!(!valid_frames(&forged));
        let mut forged = value;
        forged.frames[1].frame_id = "main".into();
        assert!(!valid_frames(&forged));
    }
}
