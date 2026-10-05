use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceProjection {
    pub id: String,
    pub name: String,
    pub terminals: Vec<TerminalProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<Vec<TabProjection>>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TabProjection {
    pub id: String,
    pub title: String,
    pub layout: RemoteLayout,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum RemoteLayout {
    Terminal {
        #[serde(rename = "paneId")]
        pane_id: String,
    },
    Split {
        axis: SplitAxis,
        ratio: f64,
        first: Box<RemoteLayout>,
        second: Box<RemoteLayout>,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}
impl RemoteLayout {
    fn validate<'a>(
        &'a self,
        depth: usize,
        members: &HashSet<&str>,
        seen: &mut HashSet<&'a str>,
    ) -> Result<(), String> {
        if depth > 32 {
            return Err("Workspace layout exceeds its depth budget.".into());
        }
        match self {
            Self::Terminal { pane_id } => {
                if !members.contains(pane_id.as_str()) || !seen.insert(pane_id.as_str()) {
                    return Err("Invalid workspace tab membership.".into());
                }
            }
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if !ratio.is_finite() || *ratio <= 0.0 || *ratio >= 1.0 {
                    return Err("Invalid workspace split ratio.".into());
                }
                first.validate(depth + 1, members, seen)?;
                second.validate(depth + 1, members, seen)?;
            }
        }
        Ok(())
    }
    fn prune(&self, members: &HashSet<&str>) -> Option<Self> {
        match self {
            Self::Terminal { pane_id } => members.contains(pane_id.as_str()).then(|| self.clone()),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.prune(members), second.prune(members)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis: axis.clone(),
                    ratio: *ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (first, second) => first.or(second),
            },
        }
    }
}
fn filtered_tabs(tabs: &[TabProjection], members: &HashSet<&str>) -> Vec<TabProjection> {
    tabs.iter()
        .filter_map(|tab| {
            tab.layout.prune(members).map(|layout| TabProjection {
                id: tab.id.clone(),
                title: tab.title.clone(),
                layout,
            })
        })
        .collect()
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalProjection {
    pub pane_id: String,
    pub session_id: Option<String>,
    pub title: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub id: String,
    pub shared: bool,
    pub online: bool,
    pub message: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Consent {
    pub id: String,
    pub epoch: String,
    pub revision: u64,
    pub shared: bool,
    #[serde(default)]
    pub sessions: Vec<(String, String)>,
    #[serde(default)]
    pub projection: Option<WorkspaceProjection>,
}
#[derive(Default)]
pub(super) struct Domain {
    pub epoch: Option<String>,
    pub revision: u64,
    pub workspaces: Vec<WorkspaceProjection>,
}
impl Domain {
    pub fn begin(&mut self) -> Result<String, String> {
        let epoch = uuid()?;
        self.epoch = Some(epoch.clone());
        self.revision = 0;
        self.workspaces.clear();
        Ok(epoch)
    }
    pub fn sync(
        &mut self,
        epoch: &str,
        revision: u64,
        workspaces: Vec<WorkspaceProjection>,
    ) -> Result<(), String> {
        if self.epoch.as_deref() != Some(epoch)
            || revision <= self.revision
            || revision > 9_007_199_254_740_991
        {
            return Err("Stale workspace publication.".into());
        }
        if workspaces.len() > 1024 {
            return Err("Workspace inventory exceeds its budget.".into());
        }
        let mut ids = HashSet::new();
        let mut panes = HashSet::new();
        let mut tab_ids = HashSet::new();
        let mut sessions = HashSet::new();
        for w in &workspaces {
            uuid_bytes(&w.id)?;
            if !ids.insert(&w.id) || w.name.chars().count() > 256 || w.terminals.len() > 1024 {
                return Err("Invalid workspace inventory.".into());
            }
            for t in &w.terminals {
                if t.pane_id.is_empty()
                    || t.pane_id.len() > 256
                    || !panes.insert(&t.pane_id)
                    || t.title.chars().count() > 256
                {
                    return Err("Invalid workspace terminal.".into());
                }
                if let Some(id) = &t.session_id {
                    uuid_bytes(id)?;
                    if !sessions.insert(id) {
                        return Err("Duplicate workspace terminal.".into());
                    }
                }
            }
            if let Some(tabs) = &w.tabs {
                if tabs.len() > 1024 {
                    return Err("Workspace tab inventory exceeds its budget.".into());
                }
                let members = w.terminals.iter().map(|t| t.pane_id.as_str()).collect();
                let mut seen = HashSet::new();
                for tab in tabs {
                    if tab.id.is_empty()
                        || tab.id.len() > 256
                        || !tab_ids.insert(&tab.id)
                        || tab.title.chars().count() > 256
                    {
                        return Err("Invalid workspace tab.".into());
                    }
                    tab.layout.validate(0, &members, &mut seen)?;
                }
                if seen != members {
                    return Err("Incomplete workspace tab membership.".into());
                }
            }
        }
        self.workspaces = workspaces;
        self.revision = revision;
        Ok(())
    }
}

impl Remote {
    pub(super) fn reconcile_workspaces(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "Terminal model unavailable.")?;
        if core.domain.revision == 0 {
            return Ok(());
        }
        let domain = core.domain.workspaces.clone();
        let count: usize = domain
            .iter()
            .filter(|w| {
                core.policy
                    .as_ref()
                    .is_some_and(|p| p.workspaces.iter().any(|c| c.id == w.id && c.shared))
            })
            .map(|w| w.terminals.len())
            .sum();
        if count > 32 {
            core.deadline = None;
            core.message = Some("Remote supports at most 32 shared terminals.".into());
            for stop in core.channels.values() {
                stop.store(true, Ordering::SeqCst);
            }
            for id in &core.shares {
                app.state::<Terminals>().remote_revoke(id);
            }
            return Err("Remote supports at most 32 shared terminals.".into());
        }
        let mut changed = false;
        let mut affected = HashSet::new();
        if let Some(policy) = core.policy.as_mut() {
            for consent in &mut policy.workspaces {
                let workspace = domain.iter().find(|w| w.id == consent.id);
                let sessions = if consent.shared && workspace_ready_projection(workspace, &runtime)
                {
                    workspace
                        .map(|w| {
                            w.terminals
                                .iter()
                                .filter_map(|t| t.session_id.as_ref())
                                .filter_map(|id| runtime.sessions.get(id))
                                .filter(|s| s.available)
                                .map(|s| (s.id.clone(), s.epoch.clone()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                } else {
                    vec![]
                };
                let old = consent.sessions.clone();
                if consent.reconcile(workspace, sessions)? {
                    affected.extend(old.into_iter().map(|s| s.0));
                    changed = true;
                }
            }
            if changed {
                for grant in &mut policy.grants {
                    if let Some(w) = policy
                        .workspaces
                        .iter()
                        .find(|w| uuid_bytes(&w.id).ok() == grant.approval.approval.workspace_id)
                    {
                        if !w.shared {
                            grant.revoked = true;
                        }
                        if w.revision != grant.approval.approval.revision {
                            grant.confirmed = false;
                        }
                    }
                }
                if let Err(error) = policy.save() {
                    core.storage_failed();
                    for id in affected {
                        app.state::<Terminals>().remote_revoke(&id);
                    }
                    return Err(error);
                }
            }
        }
        for id in affected {
            app.state::<Terminals>().remote_revoke(&id);
        }
        let workspace_sessions: HashSet<String> = core
            .policy
            .as_ref()
            .map(|p| {
                p.workspaces
                    .iter()
                    .filter(|w| w.shared && workspace_ready(&core.domain, &runtime, &w.id))
                    .flat_map(|w| w.sessions.iter().map(|s| s.0.clone()))
                    .collect()
            })
            .unwrap_or_default();
        core.shares = workspace_sessions;
        let legacy = core.legacy_shares.clone();
        core.shares.extend(legacy);
        if changed {
            core.policy_revision = core.policy_revision.wrapping_add(1);
        }
        drop(runtime);
        drop(core);
        if changed {
            let _ = tauri::Emitter::emit(app, "lomi-remote-state", self.state());
        }
        Ok(())
    }

    pub(super) fn workspace_metadata(&self, id: &str) -> Result<Value, String> {
        let core = self.core.lock().map_err(|_| "Remote unavailable.")?;
        let runtime = self.runtime.lock().map_err(|_| "Terminal unavailable.")?;
        let consent = core
            .policy
            .as_ref()
            .and_then(|p| p.workspaces.iter().find(|w| w.id == id && w.shared))
            .ok_or("Workspace is not shared.")?;
        let w = core
            .domain
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .ok_or("Workspace unavailable.")?;
        let terminals = w.terminals.iter().filter_map(|t| {
            let s = runtime.sessions.get(t.session_id.as_ref()?)?;
            (s.available && consent.sessions.contains(&(s.id.clone(), s.epoch.clone())))
                .then(|| json!({"id":s.id,"paneId":t.pane_id,"title":t.title,"cols":s.cols,"rows":s.rows}))
        }).collect::<Vec<_>>();
        let mut value = json!({"v":1,"type":"workspace","workspace":{"id":w.id,"name":w.name,"epoch":consent.epoch,"revision":consent.revision,"terminals":terminals}});
        if let Some(tabs) = &w.tabs {
            let members = terminals
                .iter()
                .filter_map(|t| t["paneId"].as_str())
                .collect();
            value["workspace"]["tabs"] = json!(filtered_tabs(tabs, &members));
        }
        if serde_json::to_vec(&value)
            .map_err(|_| "Invalid workspace metadata.")?
            .len()
            > 65536
        {
            return Err("Workspace metadata exceeds its budget.".into());
        }
        Ok(value)
    }
}

#[tauri::command]
pub fn remote_begin_workspace_sync(
    window: Window,
    remote: State<'_, Remote>,
) -> Result<Value, String> {
    crate::files::main_window(&window)?;
    let epoch = {
        let mut core = remote.core.lock().map_err(|_| "Remote unavailable.")?;
        for stop in core.channels.values() {
            stop.store(true, Ordering::SeqCst);
        }
        for id in &core.shares {
            window.state::<Terminals>().remote_revoke(id);
        }
        core.shares.clear();
        core.domain.begin()?
    };
    Ok(json!({"epoch":epoch}))
}
#[tauri::command]
pub fn remote_sync_workspaces(
    window: Window,
    remote: State<'_, Remote>,
    epoch: String,
    revision: u64,
    workspaces: Vec<WorkspaceProjection>,
) -> Result<RemoteState, String> {
    crate::files::main_window(&window)?;
    {
        let mut core = remote.core.lock().map_err(|_| "Remote unavailable.")?;
        let old = core.domain.workspaces.clone();
        core.domain.sync(&epoch, revision, workspaces)?;
        let affected: Vec<String> = old
            .iter()
            .filter(|w| core.domain.workspaces.iter().find(|next| next.id == w.id) != Some(*w))
            .map(|w| w.id.clone())
            .collect();
        for id in affected {
            fence_workspace(&mut core, window.app_handle(), &id);
        }
    }
    remote.reconcile_workspaces(window.app_handle())?;
    Ok(remote.state())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn remote_share_workspace(
    window: Window,
    remote: State<'_, Remote>,
    workspace_id: String,
    shared: bool,
) -> Result<RemoteState, String> {
    crate::files::main_window(&window)?;
    uuid_bytes(&workspace_id)?;
    remote.reset_changed_binding(window.app_handle());
    if shared
        && !remote
            .core
            .lock()
            .map_err(|_| "Remote unavailable.")?
            .enabled
    {
        #[cfg(feature = "remote-probe")]
        if window.state::<AuthController>().remote_environment()? == "development" {
            remote.enable_inner(window.app_handle(), true, true).await?;
        } else {
            remote.enable(window.app_handle(), true).await?;
        }
        #[cfg(not(feature = "remote-probe"))]
        remote.enable(window.app_handle(), true).await?;
    }
    {
        let mut core = remote.core.lock().map_err(|_| "Remote unavailable.")?;
        if !core.domain.workspaces.iter().any(|w| w.id == workspace_id) {
            return Err("Workspace unavailable.".into());
        }
        if shared {
            let count: usize =
                core.domain
                    .workspaces
                    .iter()
                    .filter(|w| {
                        w.id == workspace_id
                            || core.policy.as_ref().is_some_and(|p| {
                                p.workspaces.iter().any(|c| c.id == w.id && c.shared)
                            })
                    })
                    .map(|w| w.terminals.len())
                    .sum();
            if count > 32 {
                return Err("Remote supports at most 32 shared terminals.".into());
            }
        }
        if !shared {
            fence_workspace(&mut core, window.app_handle(), &workspace_id);
        }
        let policy = core.policy.as_mut().ok_or("Remote identity unavailable.")?;
        if shared
            && policy.workspaces.iter().filter(|w| w.shared).count() >= 32
            && !policy
                .workspaces
                .iter()
                .any(|w| w.id == workspace_id && w.shared)
        {
            return Err("Remote supports at most 32 workspaces.".into());
        }
        if !policy.workspaces.iter().any(|w| w.id == workspace_id) {
            policy.workspaces.push(Consent {
                id: workspace_id.clone(),
                epoch: uuid()?,
                revision: 1,
                shared: false,
                sessions: vec![],
                projection: None,
            });
        }
        let w = policy
            .workspaces
            .iter_mut()
            .find(|w| w.id == workspace_id)
            .ok_or("Workspace unavailable.")?;
        if w.shared != shared {
            w.shared = shared;
            w.epoch = uuid()?;
            w.revision = w
                .revision
                .checked_add(1)
                .filter(|r| *r <= 9_007_199_254_740_991)
                .ok_or("Workspace revision exhausted.")?;
        }
        let removed = if !shared {
            w.sessions.iter().map(|s| s.0.clone()).collect::<Vec<_>>()
        } else {
            vec![]
        };
        for g in &mut policy.grants {
            if g.approval.approval.workspace_id == Some(uuid_bytes(&workspace_id)?) && !shared {
                g.revoked = true;
            }
        }
        let saved = policy.save();
        for id in removed {
            core.shares.remove(&id);
            window.state::<Terminals>().remote_revoke(&id);
        }
        if let Err(error) = saved {
            core.storage_failed();
            return Err(error);
        }
        core.policy_revision = core.policy_revision.wrapping_add(1);
    }
    // Unsharing never depends on terminal readiness. Cloud scope retirement is
    // best effort after durable local fencing and has one total network budget.
    if !shared {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        publish_unshare(deadline, async {
            // Capture publication state only after earlier heartbeat requests
            // settle, so a paused Stop cannot overwrite a concurrent Resume.
            let publication = remote.publication.lock().await;
            let (enabled, host_id) = {
                let core = remote.core.lock().map_err(|_| "Remote unavailable.")?;
                (
                    core.enabled,
                    core.policy
                        .as_ref()
                        .ok_or("Remote identity unavailable.")?
                        .host_id
                        .clone(),
                )
            };
            if enabled {
                remote.poll_locked(window.app_handle(), &publication).await
            } else {
                // Paused channels are already fenced. Empty cloud scope preserves
                // local consent and permits enrollment after Resume or re-sharing.
                let auth = window.state::<AuthController>();
                let last_seen = Remote::publication_last_seen(&auth, &host_id).await?;
                // A request can finish server-side after its local timeout. An
                // older paused publication must not erase a newer active scope.
                auth.remote_request(
                    reqwest::Method::POST,
                    &format!("/v1/remote/native/hosts/{host_id}/heartbeat"),
                    Some(&json!({
                        "shares":[], "workspaces":[], "expectedLastSeen":last_seen
                    })),
                )
                .await
                .map(|_| ())
            }
        })
        .await;
        return Ok(remote.state());
    }
    for _ in 0..50 {
        remote.reconcile_workspaces(window.app_handle())?;
        let ready = {
            let core = remote.core.lock().map_err(|_| "Remote unavailable.")?;
            let w = core
                .domain
                .workspaces
                .iter()
                .find(|w| w.id == workspace_id)
                .ok_or("Workspace unavailable.")?;
            let runtime = remote.runtime.lock().map_err(|_| "Terminal unavailable.")?;
            if let Some(message) = workspace_unavailable(&core.domain, &runtime, &workspace_id) {
                return Err(message);
            }
            w.terminals.iter().all(|t| {
                t.session_id
                    .as_ref()
                    .is_some_and(|id| runtime.sessions.get(id).is_some_and(|s| s.available))
            })
        };
        if ready {
            remote.poll(window.app_handle()).await?;
            return Ok(remote.state());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("Waiting for every workspace terminal to become available.".into())
}

// Durable local revocation must not wait for each HTTP operation's timeout.
async fn publish_unshare<Fut>(deadline: tokio::time::Instant, publication: Fut)
where
    Fut: std::future::Future,
{
    let _ = tokio::time::timeout_at(deadline, publication).await;
}

#[cfg(test)]
#[allow(
    clippy::items_after_test_module,
    reason = "Keep lifecycle fixtures beside the state transitions they exercise."
)]
mod tests {
    use super::*;
    fn leaf(id: &str) -> RemoteLayout {
        RemoteLayout::Terminal { pane_id: id.into() }
    }
    fn split(first: RemoteLayout, second: RemoteLayout, ratio: f64) -> RemoteLayout {
        RemoteLayout::Split {
            axis: SplitAxis::Horizontal,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }
    #[tokio::test]
    async fn failed_unshare_publication_has_one_deadline_for_all_requests() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let grants: Vec<String> = (0..32).map(|n| n.to_string()).collect();
        let started = tokio::time::Instant::now();
        let deadline = started + Duration::from_millis(50);
        publish_unshare(deadline, async {
            for _ in grants {
                attempts.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                let _ = Err::<(), _>("Cloud unavailable");
            }
            std::future::pending::<()>().await;
        })
        .await;
        assert!(attempts.load(Ordering::SeqCst) < 32);
        assert!(started.elapsed() < Duration::from_millis(500));
        // A stalled heartbeat cannot extend the exhausted retirement budget.
        assert!(
            tokio::time::timeout_at(deadline, std::future::pending::<()>())
                .await
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_millis(500));
    }
    #[test]
    fn renewal_is_proactive_and_capped_by_both_current_owners() {
        assert_eq!(renewal_expiry(1301, 100000, 100000, 1000), None);
        assert_eq!(renewal_expiry(1300, 100000, 100000, 1000), Some(44200));
        assert_eq!(renewal_expiry(1300, 2000, 3000, 1000), Some(2000));
        assert_eq!(renewal_expiry(1300, 3000, 2000, 1000), Some(2000));
        assert_eq!(renewal_expiry(1300, 1300, 3000, 1000), None);
        assert_eq!(renewal_expiry(1300, 1200, 3000, 1000), None);
    }
    #[test]
    fn tab_layouts_validate_membership_bounds_and_legacy_metadata() {
        let mut domain = Domain::default();
        let epoch = domain.begin().unwrap();
        let mut w = WorkspaceProjection {
            id: uuid().unwrap(),
            name: "Workspace".into(),
            terminals: ["one", "two"]
                .into_iter()
                .map(|id| TerminalProjection {
                    pane_id: id.into(),
                    session_id: None,
                    title: "Shell".into(),
                })
                .collect(),
            tabs: Some(vec![TabProjection {
                id: "tab".into(),
                title: "Shell".into(),
                layout: split(leaf("one"), leaf("two"), 0.3),
            }]),
        };
        domain.sync(&epoch, 1, vec![w.clone()]).unwrap();
        let good = w.clone();
        for layout in [
            leaf("one"),
            leaf("unknown"),
            split(leaf("one"), leaf("one"), 0.5),
            split(leaf("one"), leaf("two"), 0.0),
            split(leaf("one"), leaf("two"), 1.0),
            split(leaf("one"), leaf("two"), f64::NAN),
        ] {
            w.tabs.as_mut().unwrap()[0].layout = layout;
            assert!(domain.sync(&epoch, 2, vec![w.clone()]).is_err());
            assert_eq!(domain.workspaces, vec![good.clone()]);
        }
        let mut deep = leaf("one");
        for _ in 0..33 {
            deep = split(deep, leaf("two"), 0.5);
        }
        w.tabs.as_mut().unwrap()[0].layout = deep;
        assert!(domain
            .sync(&epoch, 2, vec![w.clone()])
            .unwrap_err()
            .contains("depth"));
        w = good.clone();
        let duplicate = w.tabs.as_ref().unwrap()[0].clone();
        w.tabs.as_mut().unwrap().push(duplicate);
        assert!(domain.sync(&epoch, 2, vec![w]).is_err());
        let mut legacy = serde_json::to_value(&good).unwrap();
        legacy.as_object_mut().unwrap().remove("tabs");
        let legacy: WorkspaceProjection = serde_json::from_value(legacy).unwrap();
        assert!(legacy.tabs.is_none());
        domain.sync(&epoch, 2, vec![legacy]).unwrap();
        assert!(serde_json::from_value::<RemoteLayout>(json!({
            "type":"split", "axis":"diagonal", "ratio":0.5,
            "first":{"type":"terminal","paneId":"one"},
            "second":{"type":"terminal","paneId":"two"}
        }))
        .is_err());
    }
    #[test]
    fn encrypted_tabs_prune_unapproved_panes_and_omit_empty_tabs() {
        let tabs = vec![
            TabProjection {
                id: "one".into(),
                title: "Same".into(),
                layout: split(leaf("a"), split(leaf("b"), leaf("private"), 0.6), 0.3),
            },
            TabProjection {
                id: "two".into(),
                title: "Same".into(),
                layout: leaf("c"),
            },
            TabProjection {
                id: "private-tab".into(),
                title: "Private".into(),
                layout: leaf("hidden"),
            },
        ];
        let allowed = HashSet::from(["a", "b", "c"]);
        let filtered = filtered_tabs(&tabs, &allowed);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].layout, split(leaf("a"), leaf("b"), 0.3));
        assert_eq!(filtered[1].id, "two");
        assert!(!serde_json::to_string(&filtered)
            .unwrap()
            .contains("private"));
        let mut consent = Consent {
            id: uuid().unwrap(),
            epoch: uuid().unwrap(),
            revision: 1,
            shared: true,
            sessions: vec![],
            projection: None,
        };
        let mut workspace = WorkspaceProjection {
            id: consent.id.clone(),
            name: "Shared".into(),
            terminals: vec![],
            tabs: Some(filtered),
        };
        consent.reconcile(Some(&workspace), vec![]).unwrap();
        workspace.tabs.as_mut().unwrap()[0].layout = split(leaf("b"), leaf("a"), 0.7);
        assert!(consent.reconcile(Some(&workspace), vec![]).unwrap());
        assert_eq!(consent.revision, 3);
        assert!(!consent.reconcile(Some(&workspace), vec![]).unwrap());
    }
    #[test]
    fn membership_epochs_missing_models_moves_and_metadata_changes_advance_once() {
        let id = uuid().unwrap();
        let terminal = uuid().unwrap();
        let w = WorkspaceProjection {
            id: id.clone(),
            name: "Private".into(),
            tabs: None,
            terminals: vec![TerminalProjection {
                pane_id: "pane".into(),
                session_id: Some(terminal.clone()),
                title: "Shell".into(),
            }],
        };
        let mut c = Consent {
            id,
            epoch: uuid().unwrap(),
            revision: 1,
            shared: true,
            sessions: vec![],
            projection: None,
        };
        let first = vec![(terminal.clone(), uuid().unwrap())];
        assert!(c.reconcile(Some(&w), first.clone()).unwrap());
        assert_eq!(c.revision, 2);
        assert!(!c.reconcile(Some(&w), first).unwrap());
        let restarted = vec![(terminal, uuid().unwrap())];
        assert!(c.reconcile(Some(&w), restarted).unwrap());
        assert_eq!(c.revision, 3);
        assert!(c.reconcile(Some(&w), vec![]).unwrap());
        assert!(c.shared);
        let mut renamed = w.clone();
        renamed.name = "Renamed".into();
        assert!(c.reconcile(Some(&renamed), vec![]).unwrap());
        assert!(c.reconcile(None, vec![]).unwrap());
        assert!(!c.shared);
        let revision = c.revision;
        assert!(!c.reconcile(None, vec![]).unwrap());
        assert_eq!(c.revision, revision);
    }
    #[test]
    fn historical_refresh_grants_do_not_exhaust_the_active_device_budget() {
        let (mut policy, host) = Policy::probe("account", "parent", [1; 16]).unwrap();
        let device =
            Identity::generate([1; 16], [2; 16], lomi_remote_crypto::Role::Device, 1).unwrap();
        for n in 0..40 {
            let id = uuid().unwrap();
            let approval = host
                .sign_peer_approval(PeerApproval {
                    version: 2,
                    account_id: [1; 16],
                    host_id: uuid_bytes(&policy.host_id).unwrap(),
                    device_id: [2; 16],
                    host_fingerprint: policy.bundle.bundle.fingerprint().unwrap(),
                    device_fingerprint: device.public_bundle().bundle.fingerprint().unwrap(),
                    pairing_nonce: [n; 32],
                    grant_id: uuid_bytes(&id).unwrap(),
                    workspace_id: Some([3; 16]),
                    workspace_epoch: Some([4; 16]),
                    session_ids: vec![],
                    session_epochs: vec![],
                    permissions: Permissions::Control,
                    access_epoch: 1,
                    revision: 1,
                    expires_at: if n % 2 == 0 { now() - 1 } else { now() + 60 },
                })
                .unwrap();
            policy.grants.push(LocalGrant {
                id,
                device_bundle: device.public_bundle(),
                approval,
                revoked: n % 2 == 1,
                confirmed: true,
                pairing_id: None,
            });
        }
        let expired = policy.grants[0].id.clone();
        let mut valid = policy.grants.remove(1);
        valid.revoked = false;
        let survivor = valid.id.clone();
        policy.grants.push(valid);
        let old_stop = Arc::new(AtomicBool::new(false));
        let live_stop = Arc::new(AtomicBool::new(false));
        let mut core = Core {
            policy: Some(policy),
            ..Core::default()
        };
        core.channels
            .insert("obsolete-channel".into(), old_stop.clone());
        core.channel_grants
            .insert("obsolete-channel".into(), expired);
        core.channels
            .insert("current-channel".into(), live_stop.clone());
        core.channel_grants
            .insert("current-channel".into(), survivor.clone());
        core.prune_inactive_grants().unwrap();
        assert!(old_stop.load(Ordering::SeqCst));
        assert!(!live_stop.load(Ordering::SeqCst));
        assert_eq!(core.policy.as_ref().unwrap().grants.len(), 1);
        assert_eq!(core.policy.as_ref().unwrap().grants[0].id, survivor);
        assert!(core.policy.as_ref().unwrap().grants.len() < 32);
    }
    #[test]
    fn secure_storage_failure_fences_channels_before_confirmation() {
        let (mut policy, _) = Policy::probe("test-account", "desktop-parent", [1; 16]).unwrap();
        policy.fail_save = true;
        let stop = Arc::new(AtomicBool::new(false));
        let mut core = Core {
            policy: Some(policy),
            enabled: true,
            deadline: Some(Instant::now() + Duration::from_secs(5)),
            ..Core::default()
        };
        core.channels.insert(uuid().unwrap(), stop.clone());
        assert!(core.commit_policy().is_err());
        assert!(core.deadline.is_none());
        assert!(!core.enabled);
        assert!(core.policy.is_none());
        assert!(stop.load(Ordering::SeqCst));
        assert_eq!(core.policy_revision, 0);
    }
    #[test]
    fn empty_workspace_is_ready_but_a_missing_terminal_never_partially_shares() {
        let mut domain = Domain::default();
        let epoch = domain.begin().unwrap();
        let id = uuid().unwrap();
        let mut w = WorkspaceProjection {
            id: id.clone(),
            name: "Empty".into(),
            tabs: None,
            terminals: vec![],
        };
        domain.sync(&epoch, 1, vec![w.clone()]).unwrap();
        assert!(workspace_ready(&domain, &runtime::Runtime::default(), &id));
        w.terminals.push(TerminalProjection {
            pane_id: "pending".into(),
            session_id: None,
            title: "Shell".into(),
        });
        domain.sync(&epoch, 2, vec![w]).unwrap();
        assert!(!workspace_ready(&domain, &runtime::Runtime::default(), &id));
    }
    #[test]
    fn projection_rejects_old_epoch_revision_and_duplicate_membership_atomically() {
        let mut d = Domain::default();
        let first = d.begin().unwrap();
        let w = WorkspaceProjection {
            id: uuid().unwrap(),
            name: "Private name".into(),
            tabs: None,
            terminals: vec![TerminalProjection {
                pane_id: "pane".into(),
                session_id: Some(uuid().unwrap()),
                title: "Private title".into(),
            }],
        };
        d.sync(&first, 1, vec![w.clone()]).unwrap();
        assert!(d.sync(&first, 1, vec![]).is_err());
        assert_eq!(d.workspaces, vec![w.clone()]);
        let mut other = w.clone();
        other.id = uuid().unwrap();
        assert!(d.sync(&first, 2, vec![w.clone(), other]).is_err());
        d.begin().unwrap();
        assert!(d.sync(&first, 3, vec![w]).is_err());
    }
}

impl Remote {
    pub(super) async fn update_workspace_grants(
        &self,
        app: &tauri::AppHandle,
        cloud_grants: &[Value],
    ) -> Result<(), String> {
        let auth = app.state::<AuthController>();
        let binding = auth.remote_binding()?;
        let updates = {
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if !core
                .binding
                .as_ref()
                .is_some_and(|b| b.same_authority(&binding))
            {
                return Err("Account session changed.".into());
            }
            core.prune_inactive_grants()?;
            let policy = core.policy.as_ref().ok_or("Remote identity unavailable.")?;
            let mut updates = Vec::new();
            // Existing enrollment retains its captured browser key and grant identity.
            for g in policy
                .grants
                .iter()
                .filter(|g| !g.revoked && g.approval.approval.version == 2)
            {
                let a = &g.approval.approval;
                let Some(w) = policy.workspaces.iter().find(|w| {
                    w.shared
                        && uuid_bytes(&w.id).ok() == a.workspace_id
                        && uuid_bytes(&w.epoch).ok() == a.workspace_epoch
                }) else {
                    continue;
                };
                let cap = cloud_grants
                    .iter()
                    .find(|cloud| cloud.get("id").and_then(Value::as_str) == Some(g.id.as_str()))
                    .and_then(|cloud| cloud.get("maxApprovalExpiresAt"))
                    .and_then(Value::as_str)
                    .and_then(|value| parse_expiry(value).ok());
                let renewal = a.revision == w.revision && g.confirmed;
                let expiry = cap
                    .and_then(|cap| renewal_expiry(a.expires_at, cap, binding.expires_at, now()));
                if renewal && expiry.is_none() {
                    continue;
                }
                let mut next = a.clone();
                if renewal {
                    next.expires_at = expiry.ok_or("Missing grant owner deadline.")?;
                }
                next.revision = w.revision;
                next.session_ids = w
                    .sessions
                    .iter()
                    .map(|s| uuid_bytes(&s.0))
                    .collect::<Result<Vec<_>, _>>()?;
                next.session_epochs = w
                    .sessions
                    .iter()
                    .map(|s| uuid_bytes(&s.1))
                    .collect::<Result<Vec<_>, _>>()?;
                let signed = core
                    .identity
                    .as_ref()
                    .ok_or("Remote identity unavailable.")?
                    .sign_peer_approval(next)?;
                updates.push((
                    g.id.clone(),
                    g.pairing_id.clone(),
                    g.device_bundle.clone(),
                    signed,
                ));
            }
            for pairing in &core.pairings {
                let (Some(id), Some(epoch)) = (&pairing.workspace_id, &pairing.workspace_epoch)
                else {
                    continue;
                };
                let Some(w) = policy
                    .workspaces
                    .iter()
                    .find(|w| w.shared && &w.id == id && &w.epoch == epoch)
                else {
                    continue;
                };
                if pairing.workspace_revision != Some(w.revision)
                    || policy.grants.iter().any(|g| {
                        g.approval.approval.device_id == pairing.device_bundle.bundle.subject_id
                            && g.approval.approval.pairing_nonce
                                == hex_bytes::<32>(&pairing.nonce).unwrap_or_default()
                    })
                {
                    continue;
                }
                let hostfp = policy.bundle.bundle.fingerprint()?;
                let devicefp = pairing.device_bundle.bundle.fingerprint()?;
                pairing.device_bundle.verify(&devicefp)?;
                let account_id = policy.bundle.bundle.account_id;
                let host_id = uuid_bytes(&policy.host_id)?;
                let device_id = uuid_bytes(&pairing.device_id)?;
                let nonce = hex_bytes::<32>(&pairing.nonce)?;
                if pairing.host_bundle != policy.bundle
                    || pairing.host_id != policy.host_id
                    || pairing.device_bundle.bundle.account_id != account_id
                    || pairing.device_bundle.bundle.subject_id != device_id
                    || pairing.device_bundle.bundle.role != lomi_remote_crypto::Role::Device
                    || pairing.host_fingerprint != hex(&hostfp)
                    || pairing.device_fingerprint != hex(&devicefp)
                    || pairing.pairing_fingerprint
                        != hex(&lomi_remote_crypto::pairing_fingerprint(
                            &account_id,
                            &host_id,
                            &device_id,
                            &hostfp,
                            &devicefp,
                            &nonce,
                        ))
                {
                    return Err("Workspace pairing identity mismatch.".into());
                }
                let grant_id = uuid()?;
                let signed = core
                    .identity
                    .as_ref()
                    .ok_or("Remote identity unavailable.")?
                    .sign_peer_approval(PeerApproval {
                        version: 2,
                        account_id,
                        host_id,
                        device_id,
                        host_fingerprint: hostfp,
                        device_fingerprint: devicefp,
                        pairing_nonce: nonce,
                        grant_id: uuid_bytes(&grant_id)?,
                        workspace_id: Some(uuid_bytes(&w.id)?),
                        workspace_epoch: Some(uuid_bytes(&w.epoch)?),
                        session_ids: w
                            .sessions
                            .iter()
                            .map(|s| uuid_bytes(&s.0))
                            .collect::<Result<Vec<_>, _>>()?,
                        session_epochs: w
                            .sessions
                            .iter()
                            .map(|s| uuid_bytes(&s.1))
                            .collect::<Result<Vec<_>, _>>()?,
                        permissions: Permissions::Control,
                        access_epoch: 1,
                        revision: w.revision,
                        expires_at: binding
                            .expires_at
                            .min(pairing.max_approval_expires_at)
                            .min(now() + 12 * 3600),
                    })?;
                updates.push((
                    grant_id,
                    Some(pairing.id.clone()),
                    pairing.device_bundle.clone(),
                    signed,
                ));
            }
            let policy = core.policy.as_mut().ok_or("Remote identity unavailable.")?;
            for (id, pairing_id, device_bundle, signed) in &updates {
                if let Some(g) = policy.grants.iter_mut().find(|g| &g.id == id) {
                    // Keep the previously confirmed authority in secure storage
                    // until a pure expiry renewal has actually been published.
                    stage_grant_update(g, signed);
                } else {
                    if policy.grants.len() >= 32 {
                        return Err("Remote device budget exceeded.".into());
                    }
                    policy.grants.push(LocalGrant {
                        id: id.clone(),
                        device_bundle: device_bundle.clone(),
                        approval: signed.clone(),
                        revoked: false,
                        confirmed: false,
                        pairing_id: pairing_id.clone(),
                    });
                }
            }
            if !updates.is_empty() {
                core.commit_policy()?;
            }
            updates
        };
        for (id, pairing_id, _, signed) in updates {
            let path = if let Some(pairing_id) = &pairing_id {
                format!("/v1/remote/native/pairings/{pairing_id}/approve")
            } else {
                format!("/v1/remote/native/grants/{id}/update")
            };
            let result = auth
                .remote_request(
                    reqwest::Method::POST,
                    &path,
                    Some(&json!({"signedApproval":signed})),
                )
                .await;
            if let Err(error) = result {
                if pairing_id.is_none() {
                    let _ = error;
                    continue;
                }
                if auth
                    .remote_request(
                        reqwest::Method::POST,
                        &format!("/v1/remote/native/grants/{id}/update"),
                        Some(&json!({"signedApproval":signed})),
                    )
                    .await
                    .is_err()
                {
                    continue;
                }
            }
            let mut core = self.core.lock().map_err(|_| "Remote unavailable.")?;
            if !core
                .binding
                .as_ref()
                .is_some_and(|b| b.same_authority(&binding))
            {
                return Err("Account session changed.".into());
            }
            let policy = core.policy.as_mut().ok_or("Remote identity unavailable.")?;
            let current = policy.workspaces.iter().any(|w| {
                w.shared
                    && uuid_bytes(&w.id).ok() == signed.approval.workspace_id
                    && uuid_bytes(&w.epoch).ok() == signed.approval.workspace_epoch
                    && w.revision == signed.approval.revision
            });
            let g = policy
                .grants
                .iter_mut()
                .find(|g| {
                    g.id == id
                        && !g.revoked
                        && (g.approval == signed
                            || (g.confirmed
                                && same_approval_scope(&g.approval.approval, &signed.approval)
                                && signed.approval.expires_at > g.approval.approval.expires_at))
                })
                .ok_or("Local workspace permission changed.")?;
            if !current {
                return Err("Workspace membership changed during publication.".into());
            }
            g.approval = signed;
            g.confirmed = true;
            g.pairing_id = None;
            if let Err(error) = policy.save() {
                if let Some(g) = policy.grants.iter_mut().find(|g| g.id == id) {
                    g.confirmed = false;
                }
                core.storage_failed();
                return Err(error);
            }
            core.policy_revision = core.policy_revision.wrapping_add(1);
        }
        Ok(())
    }
}

pub(super) fn workspace_ready(domain: &Domain, runtime: &runtime::Runtime, id: &str) -> bool {
    domain.revision > 0
        && workspace_ready_projection(domain.workspaces.iter().find(|w| w.id == id), runtime)
}
fn workspace_ready_projection(
    workspace: Option<&WorkspaceProjection>,
    runtime: &runtime::Runtime,
) -> bool {
    workspace.is_some_and(|w| {
        w.terminals.iter().all(|t| {
            t.session_id
                .as_ref()
                .is_some_and(|id| runtime.sessions.get(id).is_some_and(|s| s.available))
        })
    })
}
fn fence_workspace(core: &mut Core, app: &tauri::AppHandle, id: &str) {
    for (channel, workspace) in &core.channel_workspaces {
        if workspace == id {
            if let Some(stop) = core.channels.get(channel) {
                stop.store(true, Ordering::SeqCst);
            }
        }
    }
    if let Some(policy) = core.policy.as_mut() {
        if let Some(w) = policy.workspaces.iter().find(|w| w.id == id) {
            for s in &w.sessions {
                app.state::<Terminals>().remote_revoke(&s.0);
            }
        }
        for g in &mut policy.grants {
            if g.approval.approval.workspace_id == uuid_bytes(id).ok() {
                g.confirmed = false;
            }
        }
    }
}

impl Consent {
    fn reconcile(
        &mut self,
        workspace: Option<&WorkspaceProjection>,
        sessions: Vec<(String, String)>,
    ) -> Result<bool, String> {
        let projection = if self.shared { workspace } else { None };
        let changed = projection != self.projection.as_ref()
            || sessions != self.sessions
            || (workspace.is_none() && self.shared);
        if !changed {
            return Ok(false);
        }
        let revision = self
            .revision
            .checked_add(1)
            .filter(|r| *r <= 9_007_199_254_740_991)
            .ok_or("Workspace revision exhausted.")?;
        if workspace.is_none() {
            self.shared = false;
        }
        self.projection = projection.cloned();
        self.sessions = sessions;
        self.revision = revision;
        Ok(true)
    }
}

pub(super) fn workspace_unavailable(
    domain: &Domain,
    runtime: &runtime::Runtime,
    id: &str,
) -> Option<String> {
    domain
        .workspaces
        .iter()
        .find(|w| w.id == id)?
        .terminals
        .iter()
        .filter_map(|t| t.session_id.as_ref())
        .filter_map(|id| runtime.sessions.get(id))
        .find_map(|s| s.unavailable_reason.map(str::to_owned))
}

/// Exact signed authority, excluding only its renewable deadline.
pub(super) fn same_approval_scope(old: &PeerApproval, next: &PeerApproval) -> bool {
    if old.version != 2 || next.version != 2 {
        return false;
    }
    let mut normalized = next.clone();
    normalized.expires_at = old.expires_at;
    old == &normalized
}

pub(super) fn stage_grant_update(grant: &mut LocalGrant, signed: &SignedPeerApproval) {
    if !grant.confirmed || !same_approval_scope(&grant.approval.approval, &signed.approval) {
        grant.approval = signed.clone();
        grant.confirmed = false;
    }
}

fn renewal_expiry(current: u64, owner: u64, native: u64, timestamp: u64) -> Option<u64> {
    let next = owner.min(native).min(timestamp.saturating_add(12 * 3600));
    (current <= timestamp.saturating_add(300) && next > current).then_some(next)
}
