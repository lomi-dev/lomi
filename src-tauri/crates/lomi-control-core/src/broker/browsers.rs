use super::{
    operations::{storage_error, UiMutation},
    *,
};
use crate::browser::BrowserControl;

pub(super) struct OwnedBrowser {
    pub owner: String,
    pub workspace: String,
    pub control: Arc<BrowserControl>,
}

pub struct BrowserStart<'a> {
    pub operation: &'a str,
    pub visible: bool,
    pub nonce: &'a str,
    pub panel: &'a str,
    pub generation: &'a str,
    pub profile: &'a str,
    pub url: &'a str,
}
pub struct BrowserNavigationDispatch {
    pub control: Arc<BrowserControl>,
    pub url: String,
    pub workspace: String,
    pub wait: BrowserWaitUntil,
    pub permit: NativePermit,
}

impl Broker {
    /// Native completion must not wait for the main renderer to resume JavaScript.
    pub fn finish_browser_navigation(
        &self,
        operation: &str,
        nonce: &str,
        result: Result<BrowserNavigationResult, ErrorCode>,
    ) -> io::Result<()> {
        let ack = {
            let state = self.lock_state().map_err(|_| failure())?;
            let work = state.work.get(operation).ok_or_else(failure)?;
            if !work.claimed
                || work.command.nonce != nonce
                || !matches!(work.command.action, UiAction::NavigateBrowser { .. })
            {
                return Err(failure());
            }
            UiAck {
                operation_id: operation.into(),
                nonce: nonce.into(),
                ui_epoch: work.command.ui_epoch.clone(),
                result: match result {
                    Ok(result) => OperationResult::BrowserNavigation(Box::new(result)),
                    Err(code) => OperationResult::Failure { code },
                },
            }
        };
        self.acknowledge_ui(ack)
    }
    pub fn open_browser(self: &Arc<Self>, id: &str, input: BrowserOpenInput) -> Reply {
        if !valid_id(&input.request_key) || input.expected_revision.parse::<u64>().is_err() {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(url) = lomi_control_protocol::browser::address(&input.url) else {
            return error(ErrorCode::ScopeDenied);
        };
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::AppUnavailable);
        };
        let Some(session) = state
            .sessions
            .get(id)
            .filter(|s| s.alive.load(Ordering::SeqCst))
        else {
            return error(ErrorCode::ControlRevoked);
        };
        if !["panel.create", "browser.navigate"]
            .iter()
            .all(|s| session.grant.scopes.contains(*s))
            || !session
                .grant
                .browser_origins
                .iter()
                .any(|o| o.permits(&url))
        {
            return error(ErrorCode::ScopeDenied);
        }
        let Some(profile) = session.grant.browser_profile.clone() else {
            return error(ErrorCode::ScopeDenied);
        };
        let Some(workspace) = state
            .projection
            .workspaces
            .iter()
            .find(|w| w.id == input.workspace_id && session.grant.permits(w))
        else {
            return error(ErrorCode::TargetNotFound);
        };
        if input.retry_epoch != session.retry_epoch {
            return error(ErrorCode::RetryWindowExpired);
        }
        let project = workspace.project_id.clone();
        let target = receipts::Target {
            workspace_id: &input.workspace_id,
            resource_id: &input.workspace_id,
            generation: &state.projection.ui_epoch,
            revision: &input.expected_revision,
        };
        let hash =
            match receipts::fingerprint(&(&input, &workspace.project_path, &profile), &target) {
                Ok(v) => v,
                Err(e) => return storage_error(e),
            };
        let (Ok(panel), Ok(generation)) = (new_id(), new_id()) else {
            return error(ErrorCode::ResourceExhausted);
        };
        let action = UiAction::CreateBrowser {
            visible: input.visible,
            workspace_id: input.workspace_id.clone(),
            panel_id: panel,
            browser_generation: generation,
            profile_id: profile,
            url: url.to_string(),
        };
        self.enqueue_ui(
            &mut state,
            id,
            UiMutation {
                workspace: input.workspace_id,
                project,
                revision: input.expected_revision,
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool: "lomi_browser_open",
                hash,
                action,
            },
        )
    }

    /// Consume the durable operation's one-use ticket before constructing a native view.
    pub fn authorize_browser_start(
        &self,
        request: BrowserStart<'_>,
    ) -> Result<Arc<BrowserControl>, String> {
        let denied = || "Browser authorization expired or changed.".to_owned();
        let mut state = self.lock_state().map_err(|_| denied())?;
        let work = state.work.get(request.operation).ok_or_else(denied)?;
        let UiAction::CreateBrowser {
            visible,
            workspace_id,
            panel_id,
            browser_generation,
            profile_id,
            url,
        } = &work.command.action
        else {
            return Err(denied());
        };
        let session = state
            .sessions
            .get(&work.pairing)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or_else(denied)?;
        if !work.claimed
            || work.native_committed
            || work.deadline <= Instant::now()
            || work.command.nonce != request.nonce
            || work.command.ui_epoch != state.projection.ui_epoch
            || *visible != request.visible
            || panel_id != request.panel
            || browser_generation != request.generation
            || profile_id != request.profile
            || url != request.url
            || session.grant.browser_profile.as_deref() != Some(request.profile)
            || session.grant.workspace(workspace_id).is_none()
            || !Self::action_scopes(&work.command.action)
                .iter()
                .all(|s| session.grant.scopes.contains(*s))
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == *workspace_id && session.grant.permits(w))
            || state.browsers.contains_key(request.generation)
        {
            return Err(denied());
        }
        if state.browsers.len() >= 16 {
            return Err("Agent browser capacity reached.".into());
        }
        let control = Arc::new(
            BrowserControl::new(
                request.generation.into(),
                request.panel.into(),
                request.profile.into(),
                session.grant.browser_origins.clone(),
                self.authorization.clone(),
                state.policy_revision,
                session.alive.clone(),
            )
            .map_err(|_| denied())?,
        );
        if !control.permits(request.url) {
            return Err(denied());
        }
        let browser = OwnedBrowser {
            owner: work.pairing.clone(),
            workspace: workspace_id.clone(),
            control: control.clone(),
        };
        state
            .work
            .get_mut(request.operation)
            .unwrap()
            .native_committed = true;
        state.browsers.insert(request.generation.into(), browser);
        Ok(control)
    }
}

impl Broker {
    pub(super) fn browser_target<'a>(
        state: &'a State,
        owner: &str,
        workspace: &str,
        panel: &str,
        generation: &str,
        scope: &str,
    ) -> Result<&'a OwnedBrowser, ErrorCode> {
        let session = state
            .sessions
            .get(owner)
            .filter(|s| s.alive.load(Ordering::SeqCst))
            .ok_or(ErrorCode::ControlRevoked)?;
        if session.grant.workspace(workspace).is_none()
            || !state
                .projection
                .workspaces
                .iter()
                .any(|w| w.id == workspace && session.grant.permits(w))
            || !state.projection.panels.iter().any(|p| {
                p.id == panel
                    && p.workspace_id == workspace
                    && p.browser_generation.as_deref() == Some(generation)
            })
        {
            return Err(ErrorCode::TargetNotFound);
        }
        let target = state
            .browsers
            .get(generation)
            .filter(|b| {
                b.owner == owner
                    && b.workspace == workspace
                    && b.control.panel_id == panel
                    && b.control.started()
            })
            .ok_or(ErrorCode::TargetNotFound)?;
        if !session.grant.scopes.contains(scope) {
            return Err(ErrorCode::ScopeDenied);
        }
        Ok(target)
    }
    pub fn navigate_browser(self: &Arc<Self>, id: &str, input: BrowserNavigateInput) -> Reply {
        if !valid_id(&input.request_key) {
            return error(ErrorCode::ResourceExhausted);
        }
        let Ok(url) = lomi_control_protocol::browser::address(&input.url) else {
            return error(ErrorCode::ScopeDenied);
        };
        let Ok(mut state) = self.lock_state() else {
            return error(ErrorCode::AppUnavailable);
        };
        let Some(session) = state
            .sessions
            .get(id)
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
        let hash = match receipts::fingerprint(
            &input,
            &receipts::Target {
                workspace_id: &input.workspace_id,
                resource_id: &input.panel_id,
                generation: &input.browser_generation,
                revision: "",
            },
        ) {
            Ok(h) => h,
            Err(e) => return storage_error(e),
        };
        match self.replay(
            &state,
            id,
            &receipts::Key {
                pairing_id: id,
                project_id: &project,
                retry_epoch: &input.retry_epoch,
                request_key: &input.request_key,
                tool: "lomi_browser_navigate",
            },
            hash,
        ) {
            Ok(Some(r)) => return r,
            Err(r) => return *r,
            Ok(None) => {}
        }
        let target = match Self::browser_target(
            &state,
            id,
            &input.workspace_id,
            &input.panel_id,
            &input.browser_generation,
            "browser.navigate",
        ) {
            Ok(t) => t,
            Err(e) => return error(e),
        };
        if target.control.lease() != Some(input.lease_id.as_str()) {
            return error(ErrorCode::ControlRevoked);
        }
        if !target.control.permits(url.as_str()) {
            return error(ErrorCode::ScopeDenied);
        }
        let revision = state.projection.revision.clone();
        let action = UiAction::NavigateBrowser {
            workspace_id: input.workspace_id.clone(),
            panel_id: input.panel_id,
            browser_generation: input.browser_generation,
            url: url.to_string(),
            wait_until: input.wait_until,
        };
        self.enqueue_ui(
            &mut state,
            id,
            UiMutation {
                workspace: input.workspace_id,
                project,
                revision,
                retry_epoch: input.retry_epoch,
                request_key: input.request_key,
                tool: "lomi_browser_navigate",
                hash,
                action,
            },
        )
    }
    pub fn authorize_browser_navigation(
        &self,
        operation: &str,
        nonce: &str,
    ) -> Result<BrowserNavigationDispatch, ErrorCode> {
        let mut state = self.lock_state().map_err(|_| ErrorCode::ControlRevoked)?;
        let work = state.work.get(operation).ok_or(ErrorCode::ControlRevoked)?;
        let UiAction::NavigateBrowser {
            workspace_id,
            panel_id,
            browser_generation,
            url,
            wait_until,
        } = &work.command.action
        else {
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
            workspace_id,
            panel_id,
            browser_generation,
            "browser.navigate",
        )?
        .control
        .clone();
        if !control.permits(url) {
            return Err(ErrorCode::ControlRevoked);
        }
        work.native_permit.check()?;
        control.prepare_navigation(operation, url)?;
        let result = BrowserNavigationDispatch {
            control,
            url: url.clone(),
            workspace: workspace_id.clone(),
            wait: *wait_until,
            permit: work.native_permit.clone(),
        };
        state.work.get_mut(operation).unwrap().native_committed = true;
        Ok(result)
    }
}
