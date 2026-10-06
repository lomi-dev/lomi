use super::{
    service::{
        append, continuation, handoff, prepare_send, settle_switches, Inner, CONTEXT_BUDGET,
    },
    store::{workspace, Store},
    types::*,
};
use crate::cli_catalog::TitleCli;
use serde_json::json;
use std::sync::atomic::Ordering;
fn fixture() -> (tempfile::TempDir, Inner, Task) {
    let root = tempfile::tempdir().unwrap();
    let mut s = Store::open(root.path().join("owner")).unwrap();
    for id in ["a", "b"] {
        let a = account(id);
        let binding = CredentialBinding {
            account_id: id.into(),
            auth_revision: 1,
            physical_root: s
                .root
                .join("accounts")
                .join(id)
                .to_string_lossy()
                .into_owned(),
            namespace: "codex".into(),
            credential_reference: "native-managed".into(),
        };
        s.save_account(
            &json!({"id":id}),
            &format!("create-{id}"),
            &a,
            &binding,
            &json!({}),
        )
        .unwrap();
    }
    let (cwd, identity) = workspace(root.path()).unwrap();
    let t = Task {
        task_id: "task".into(),
        cwd,
        title: "task".into(),
        cli: Some(TitleCli::Codex),
        availability_reason: None,
        model: "gpt-6.1-sol".into(),
        reasoning_effort: Some("high".into()),
        revision: 1,
        history_revision: 0,
        generation: 0,
        state: TaskState::Idle,
        next_account_id: "a".into(),
        active_account_id: None,
        active_attempt_id: None,
        status_message: String::new(),
        attempts: vec![],
        history: vec![],
        grants: vec![
            HistoryGrant {
                account_id: "a".into(),
                auth_revision: 1,
                revision: 1,
            },
            HistoryGrant {
                account_id: "b".into(),
                auth_revision: 1,
                revision: 1,
            },
        ],
        switches: vec![],
    };
    s.create_task(&json!({}), "create-task", &t, "local", identity)
        .unwrap();
    let inner = Inner {
        store: Some(s),
        ..Default::default()
    };
    (root, inner, t)
}
fn account(id: &str) -> AccountInstance {
    AccountInstance {
        account_id: id.into(),
        cli: TitleCli::Codex,
        label: id.into(),
        enabled: true,
        revision: 1,
        auth_revision: 1,
        auth_state: "unverified".into(),
        availability_reason: None,
        accepted_version: Some("0.160.0".into()),
        recovery: None,
    }
}
#[test]
fn mcp_existing_session_keeps_durable_native_ceiling_across_owner_restart() {
    use lomi_control_protocol::{control::Request, EmptyInput, ErrorCode};
    let (_root, inner, task) = fixture();
    let runtime = super::service::AgentRuntime {
        inner: std::sync::Arc::new(std::sync::Mutex::new(inner)),
        ..Default::default()
    };
    let policy = super::mcp_policy::capture(&runtime, Some(std::process::id())).unwrap();
    let request = Request::Workspaces(lomi_control_protocol::control::ListInput {
        cursor: None,
        limit: 1,
    });
    assert!(policy.check(&request).is_ok());
    let path = {
        let mut inner = runtime.inner.lock().unwrap();
        let store = inner.store.as_mut().unwrap();
        let account = store.account("a").unwrap();
        store
            .helper_intent("verify", &account, &task.cwd, None)
            .unwrap();
        store.root.clone()
    };
    assert_eq!(policy.check(&request), Err(ErrorCode::ScopeDenied));
    assert!(policy.check(&Request::Status(EmptyInput {})).is_ok());
    // Replace the in-memory store: ordinary client admission cannot erase an
    // unresolved helper merely because the app has reconstructed its owner.
    {
        let mut inner = runtime.inner.lock().unwrap();
        drop(inner.store.take());
        inner.store = Some(Store::open(path).unwrap());
    }
    assert_eq!(policy.check(&request), Err(ErrorCode::ScopeDenied));
    let reconnected = super::mcp_policy::capture(&runtime, Some(std::process::id())).unwrap();
    assert_eq!(reconnected.check(&request), Err(ErrorCode::ScopeDenied));
    assert_eq!(
        reconnected.admit_effect(&request).err(),
        Some(ErrorCode::ScopeDenied)
    );
}

#[test]
fn mcp_unknown_peer_missing_owner_and_close_never_get_ordinary_fallback() {
    use lomi_control_protocol::{control::Request, EmptyInput, ErrorCode};
    let empty = super::service::AgentRuntime::default();
    assert_eq!(
        super::mcp_policy::capture(&empty, None).err(),
        Some(ErrorCode::ScopeDenied)
    );
    assert_eq!(
        super::mcp_policy::capture(&empty, Some(0)).err(),
        Some(ErrorCode::ScopeDenied)
    );
    let policy = super::mcp_policy::capture(&empty, Some(std::process::id())).unwrap();
    assert_eq!(
        policy.check(&Request::Status(EmptyInput {})),
        Err(ErrorCode::AppUnavailable)
    );
    let (_root, inner, _task) = fixture();
    let runtime = super::service::AgentRuntime {
        inner: std::sync::Arc::new(std::sync::Mutex::new(inner)),
        ..Default::default()
    };
    let policy = super::mcp_policy::capture(&runtime, Some(std::process::id())).unwrap();
    runtime.closing.store(true, Ordering::SeqCst);
    assert_eq!(
        policy.check(&Request::Status(EmptyInput {})),
        Err(ErrorCode::ControlRevoked)
    );
}

#[test]
fn mcp_unknown_turn_intent_is_fenced_without_a_native_pid() {
    use lomi_control_protocol::{control::Request, EmptyInput, ErrorCode};
    let (_root, mut inner, task) = fixture();
    let (task, _) =
        prepare_send(&mut inner, &send(&task, "a", "mcp-native-intent"), false).unwrap();
    inner
        .store
        .as_mut()
        .unwrap()
        .process_intent(&task, task.attempts.last().unwrap())
        .unwrap();
    let runtime = super::service::AgentRuntime {
        inner: std::sync::Arc::new(std::sync::Mutex::new(inner)),
        ..Default::default()
    };
    let policy = super::mcp_policy::capture(&runtime, Some(std::process::id())).unwrap();
    assert_eq!(
        policy.check(&Request::Workspaces(
            lomi_control_protocol::control::ListInput {
                cursor: None,
                limit: 1,
            }
        )),
        Err(ErrorCode::ScopeDenied)
    );
    assert!(policy.check(&Request::Status(EmptyInput {})).is_ok());
}

#[test]
#[ignore = "Process-global effect admission fixture; run this test serially"]
fn pending_unscoped_mcp_effect_refuses_send_before_durable_attempt() {
    let (_root, mut inner, task) = fixture();
    let _effect = crate::project_write_guard::admit_unscoped().unwrap();
    let result = prepare_send(&mut inner, &send(&task, "a", "mcp-effect-send"), false);
    assert!(result.is_err());
    assert!(inner.active.is_empty());
    let saved = inner.store.as_ref().unwrap().task(&task.task_id).unwrap();
    assert_eq!(saved.revision, task.revision);
    assert!(saved.attempts.is_empty());
    assert!(saved.history.is_empty());
}

#[test]
#[ignore = "Process-global effect admission fixture; run this test serially"]
fn pending_mcp_effect_refuses_prepared_switch_before_generation_or_receipt() {
    let (_root, mut inner, mut task) = fixture();
    task.state = TaskState::Prepared;
    task.switches.push(SwitchOperation {
        operation_id: "prepared-switch".into(),
        source_attempt_id: None,
        account_id: "b".into(),
        auth_revision: 1,
        history_revision: 0,
        mode: "stop_and_continue".into(),
        phase: "prepared".into(),
        continuation_method: "fresh".into(),
        reason: None,
        coverage: 0,
        budget_bytes: CONTEXT_BUDGET as u64,
        context_digest: None,
        stop_supervision_qualified: true,
    });
    inner
        .store
        .as_mut()
        .unwrap()
        .update_task(&task, None, None)
        .unwrap();
    let request = SwitchCommit {
        operation_id: "mcp-effect-switch".into(),
        task_id: task.task_id.clone(),
        switch_operation_id: "prepared-switch".into(),
        expected_revision: task.revision,
        account_id: "b".into(),
        auth_revision: 1,
    };
    let effect = crate::project_write_guard::admit_unscoped().unwrap();
    assert!(super::service::prepare_switch_commit(&mut inner, &request, false).is_err());
    assert!(inner.active.is_empty());
    let saved = inner.store.as_ref().unwrap().task(&task.task_id).unwrap();
    assert_eq!(saved.revision, task.revision);
    assert_eq!(saved.generation, task.generation);
    assert!(saved.history.is_empty());
    assert!(saved.attempts.is_empty());
    assert_eq!(saved.switches[0].phase, "prepared");
    assert!(inner
        .store
        .as_ref()
        .unwrap()
        .replay::<_, Task>(&request.operation_id, &request)
        .unwrap()
        .is_none());
    drop(effect);
    // The identical operation can still be committed once the MCP effect ends.
    let (started, dispatch) =
        super::service::prepare_switch_commit(&mut inner, &request, false).unwrap();
    assert!(dispatch);
    assert_eq!(started.generation, task.generation + 1);
    assert_eq!(started.attempts.len(), 1);
}

#[tokio::test]
#[ignore = "Process-global native/MCP integration fixture; run serially"]
async fn real_broker_enrollment_keeps_runtime_ceiling_after_yolo_reconnect() {
    use lomi_control_core::{broker::Broker, client::Client};
    use lomi_control_protocol::{control::*, EmptyInput, ErrorCode};
    let (root, inner, task) = fixture();
    let runtime = super::service::AgentRuntime {
        inner: std::sync::Arc::new(std::sync::Mutex::new(inner)),
        ..Default::default()
    };
    let captured_runtime = runtime.clone();
    let broker = Broker::start_with_admission(
        &root.path().join("mcp-control"),
        None,
        Some(std::sync::Arc::new(move |pid| {
            super::mcp_policy::capture(&captured_runtime, pid).map(Some)
        })),
    )
    .unwrap();
    broker
        .publish(Projection {
            ui_epoch: broker.register_ui().unwrap(),
            revision: "1".into(),
            workspaces: vec![Workspace {
                id: "workspace".into(),
                project_id: "project".into(),
                name: "workspace".into(),
                project_name: "project".into(),
                active_panel_id: None,
                project_path: task.cwd.clone(),
            }],
            ..Default::default()
        })
        .unwrap();
    broker.set_yolo_mode(true).unwrap();
    let client = Client::connect(&broker.endpoint, "ordinary trusted client", |_| {})
        .await
        .unwrap();
    let workspaces = Request::Workspaces(ListInput {
        limit: 1,
        cursor: None,
    });
    assert!(matches!(
        client.call(workspaces.clone()).await.unwrap(),
        Reply::Ok { .. }
    ));
    {
        let mut inner = runtime.inner.lock().unwrap();
        let store = inner.store.as_mut().unwrap();
        let account = store.account("a").unwrap();
        store
            .helper_intent("verify", &account, &task.cwd, None)
            .unwrap();
    }
    assert!(matches!(
        client.call(workspaces.clone()).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    assert!(matches!(
        client.call(Request::Status(EmptyInput {})).await.unwrap(),
        Reply::Ok { .. }
    ));
    drop(client);
    let client = Client::connect(
        &broker.endpoint,
        "owned task a with administrator grant",
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(
        client.call(workspaces).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    drop(client);
    broker.shutdown().await;
}
fn send(t: &Task, id: &str, operation: &str) -> TaskSend {
    TaskSend {
        operation_id: operation.into(),
        task_id: t.task_id.clone(),
        expected_revision: t.revision,
        account_id: id.into(),
        auth_revision: 1,
        text: "work".into(),
        continue_requested: None,
    }
}
fn settle(inner: &mut Inner, mut t: Task) -> Task {
    t.state = TaskState::Completed;
    t.active_attempt_id = None;
    t.active_account_id = None;
    let a = t.attempts.last_mut().unwrap();
    a.state = "completed".into();
    a.effects_state = "settled".into();
    t.revision += 1;
    inner
        .store
        .as_mut()
        .unwrap()
        .update_task(&t, None, None)
        .unwrap();
    inner.active.clear();
    inner.account_leases.clear();
    t
}
#[test]
fn same_receipt_never_creates_second_intention_and_changed_content_fails() {
    let (_root, mut i, t) = fixture();
    let r = send(&t, "a", "send-one");
    let (started, dispatch) = prepare_send(&mut i, &r, false).unwrap();
    assert!(dispatch);
    let (replayed, dispatch) = prepare_send(&mut i, &r, false).unwrap();
    assert!(!dispatch);
    assert_eq!(replayed.attempts.len(), 1);
    assert_eq!(
        started.attempts[0].attempt_id,
        replayed.attempts[0].attempt_id
    );
    let mut changed = r;
    changed.account_id = "b".into();
    assert!(prepare_send(&mut i, &changed, false)
        .unwrap_err()
        .contains("different command"));
}
#[test]
fn explicit_continue_uses_b_without_previous_owner_fallback() {
    let (_root, mut i, t) = fixture();
    let (first, _) = prepare_send(&mut i, &send(&t, "a", "send-a"), false).unwrap();
    let done = settle(&mut i, first);
    let mut r = send(&done, "b", "continue-b");
    r.text = String::new();
    r.continue_requested = Some(true);
    let (next, _) = prepare_send(&mut i, &r, false).unwrap();
    assert_eq!(next.active_account_id.as_deref(), Some("b"));
    assert_eq!(next.attempts.last().unwrap().account_id, "b");
    assert_eq!(next.attempts.last().unwrap().continuation_method, "handoff");
}
#[test]
fn failed_receipt_transaction_does_not_publish_projection_or_intention() {
    let (_root, mut i, t) = fixture();
    let s = i.store.as_mut().unwrap();
    let before = s.revision().unwrap();
    let mut modified = t.clone();
    modified.title = "changed".into();
    assert!(s
        .mutate_task(&json!({"different":true}), "create-task", &modified, None)
        .is_err());
    assert_eq!(s.task("task").unwrap().title, "task");
    assert_eq!(s.revision().unwrap(), before);
}
#[test]
fn restart_pending_is_not_dispatched_and_same_command_still_replays() {
    let (root, mut i, t) = fixture();
    let r = send(&t, "a", "pending");
    let (started, _) = prepare_send(&mut i, &r, false).unwrap();
    drop(i);
    let s = Store::open(root.path().join("owner")).unwrap();
    let current = s.task("task").unwrap();
    assert_eq!(current.state, TaskState::Stopped);
    assert_eq!(current.attempts[0].state, "rejected");
    let replay: Task = s.replay("pending", &r).unwrap().unwrap();
    assert_eq!(
        replay.attempts[0].attempt_id,
        started.attempts[0].attempt_id
    );
    assert!(s.processes_settled("task").unwrap());
}
#[test]
fn restart_spawn_gap_keeps_account_reserved_and_cannot_claim_death() {
    let (root, mut i, t) = fixture();
    let (mut started, _) = prepare_send(&mut i, &send(&t, "a", "spawn-gap"), false).unwrap();
    started.attempts[0].state = "spawn_intent".into();
    let a = started.attempts[0].clone();
    i.store
        .as_mut()
        .unwrap()
        .process_intent(&started, &a)
        .unwrap();
    drop(i);
    let mut s = Store::open(root.path().join("owner")).unwrap();
    assert_eq!(s.task("task").unwrap().state, TaskState::DeliveryUncertain);
    assert_eq!(s.unresolved_accounts().unwrap(), vec!["a"]);
    assert!(s.reconcile_processes().unwrap().is_empty());
    assert!(!s.processes_settled("task").unwrap());
}
#[test]
fn proven_no_child_releases_spawn_intention() {
    let (_root, mut i, t) = fixture();
    let (mut started, _) = prepare_send(&mut i, &send(&t, "a", "spawn-error"), false).unwrap();
    started.attempts[0].state = "spawn_intent".into();
    let a = started.attempts[0].clone();
    let s = i.store.as_mut().unwrap();
    s.process_intent(&started, &a).unwrap();
    s.process_not_spawned(&a.attempt_id).unwrap();
    assert!(s.processes_settled("task").unwrap());
    assert!(s.unresolved_accounts().unwrap().is_empty());
}
#[test]
fn full_journal_handoff_returns_a_with_b_work_and_refuses_oversize() {
    let (_root, mut i, t) = fixture();
    let (first, _) = prepare_send(&mut i, &send(&t, "a", "a1"), false).unwrap();
    let mut t = settle(&mut i, first);
    let a = t.attempts[0].clone();
    append(&mut t, &a, "assistant", "completed", json!("A work"));
    let mut b = a.clone();
    b.attempt_id = "attempt-b".into();
    b.account_id = "b".into();
    b.generation = 2;
    b.operation_id = "b1".into();
    t.generation = 2;
    t.attempts.push(b.clone());
    append(
        &mut t,
        &b,
        "tool",
        "completed",
        json!({"toolId":"b-tool","result":"B final project changes"}),
    );
    i.store
        .as_mut()
        .unwrap()
        .update_task(&t, None, None)
        .unwrap();
    assert_eq!(
        continuation(i.store.as_ref().unwrap(), &t, &account("a")).unwrap(),
        "handoff"
    );
    assert!(handoff(&t).unwrap().contains("B final project changes"));
    append(
        &mut t,
        &b,
        "assistant",
        "completed",
        json!("x".repeat(CONTEXT_BUDGET)),
    );
    assert!(handoff(&t)
        .unwrap_err()
        .contains("No history was truncated"));
}
#[test]
fn stop_switch_revalidates_target_auth_and_grants_after_source_settlement() {
    let (_root, mut i, t) = fixture();
    let (first, _) = prepare_send(&mut i, &send(&t, "a", "source"), false).unwrap();
    let mut t = settle(&mut i, first);
    t.switches.push(SwitchOperation {
        operation_id: "switch".into(),
        source_attempt_id: Some(t.attempts[0].attempt_id.clone()),
        account_id: "b".into(),
        auth_revision: 1,
        history_revision: 0,
        mode: "stop_and_continue".into(),
        phase: "stopping_source".into(),
        continuation_method: "blocked".into(),
        reason: None,
        coverage: 0,
        budget_bytes: CONTEXT_BUDGET as u64,
        context_digest: None,
        stop_supervision_qualified: true,
    });
    let s = i.store.as_mut().unwrap();
    settle_switches(s, &mut t).unwrap();
    assert_eq!(t.switches[0].phase, "prepared");
    assert_eq!(t.switches[0].continuation_method, "handoff");
    assert!(t.switches[0].reason.is_none());
    t.state = TaskState::Stopped;
    t.switches[0].phase = "stopping_source".into();
    t.grants.retain(|g| g.account_id != "b");
    settle_switches(s, &mut t).unwrap();
    assert!(t.switches[0].reason.as_ref().unwrap().contains("grant"));
    t.grants.push(HistoryGrant {
        account_id: "b".into(),
        auth_revision: 1,
        revision: 2,
    });
    let mut b = account("b");
    b.auth_revision = 2;
    b.revision = 2;
    let mut binding = s.binding("b").unwrap();
    binding.auth_revision = 2;
    s.save_account(&json!({"auth":2}), "change-auth", &b, &binding, &json!({}))
        .unwrap();
    t.state = TaskState::Stopped;
    t.switches[0].phase = "stopping_source".into();
    settle_switches(s, &mut t).unwrap();
    assert!(t.switches[0].reason.as_ref().unwrap().contains("changed"));
}
#[test]
fn stop_control_serialization_fences_queued_permission() {
    let (_root, mut i, t) = fixture();
    let (_started, _) = prepare_send(&mut i, &send(&t, "a", "approval-source"), false).unwrap();
    let active = i.active.get("task").unwrap().clone();
    let guard = active.control.lock().unwrap();
    active.stop.store(true, Ordering::SeqCst);
    drop(guard);
    let guard = active.control.lock().unwrap();
    assert!(active.stop.load(Ordering::SeqCst));
    drop(guard);
}
#[test]
fn unknown_schema_is_preserved_before_table_creation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("owner");
    std::fs::create_dir(&path).unwrap();
    crate::chat::storage::private(&path, true).unwrap();
    let db = rusqlite::Connection::open(path.join("runtime.sqlite")).unwrap();
    db.execute_batch("PRAGMA user_version=99;CREATE TABLE foreign_data(value TEXT);INSERT INTO foreign_data VALUES('preserve');").unwrap();
    drop(db);
    crate::chat::storage::private(&path.join("runtime.sqlite"), false).unwrap();
    let before = std::fs::read(path.join("runtime.sqlite")).unwrap();
    assert!(Store::open(path.clone()).is_err());
    assert_eq!(std::fs::read(path.join("runtime.sqlite")).unwrap(), before);
}
#[test]
fn exact_codex_version_does_not_admit_installed_patch() {
    assert!(
        super::native_wire::NativeKind::from_version(TitleCli::Codex, "codex-cli 0.160.1").is_err()
    );
    assert!(
        super::native_wire::NativeKind::from_version(TitleCli::Codex, "codex-cli 0.160.0").is_ok()
    );
}

#[test]
fn pi_json_null_cancel_survives_deserialization_and_command_validation() {
    for method in ["input", "editor", "select"] {
        let request: PermissionReply = serde_json::from_value(json!({
            "operationId":"cancel", "taskId":"task", "attemptId":"attempt",
            "generation":1, "approvalToken":"approval", "allow":false,
            "choice":null, "editorFreezeToken":null
        }))
        .unwrap();
        let permission = super::native_wire::NativePermission {
            request_id: json!("native-dialog"),
            session_id: "session".into(),
            turn_id: "turn".into(),
            tool_id: None,
            choices: vec![json!("first")],
            raw: json!({"method":method}),
        };
        assert!(super::commands::permission_choice(&request, &permission)
            .unwrap()
            .is_null());
        let mut forged = request;
        forged.allow = true;
        assert!(super::commands::permission_choice(&forged, &permission).is_err());
    }
}

#[test]
fn durable_history_digest_is_stable_across_canonical_projection_reopen() {
    let (root, mut i, t) = fixture();
    let (started, _) = prepare_send(&mut i, &send(&t, "a", "canonical-history"), false).unwrap();
    let mut t = settle(&mut i, started);
    let a = t.attempts[0].clone();
    let mut content = serde_json::Map::new();
    content.insert("z".into(), json!({"b":2,"a":1}));
    content.insert("a".into(), json!("first"));
    append(
        &mut t,
        &a,
        "tool",
        "completed",
        serde_json::Value::Object(content),
    );
    i.store
        .as_mut()
        .unwrap()
        .update_task(&t, None, None)
        .unwrap();
    let digest = super::service::history_digest(&t).unwrap();
    drop(i);
    let s = Store::open(root.path().join("owner")).unwrap();
    assert_eq!(
        super::service::history_digest(&s.task("task").unwrap()).unwrap(),
        digest
    );
}

#[test]
fn oversized_projection_refuses_before_commit_and_previous_record_reopens() {
    let (root, mut i, t) = fixture();
    let original = t.clone();
    let mut oversize = t;
    oversize.status_message = "x".repeat(64 * 1024 * 1024);
    assert!(i
        .store
        .as_mut()
        .unwrap()
        .update_task(&oversize, None, None)
        .is_err());
    drop(i);
    let s = Store::open(root.path().join("owner")).unwrap();
    assert_eq!(s.task("task").unwrap().revision, original.revision);
    assert_eq!(
        s.task("task").unwrap().status_message,
        original.status_message
    );
}

#[cfg(unix)]
#[test]
fn restart_owned_child_reserves_binding_until_private_marker_cleanup() {
    use std::os::unix::process::CommandExt;
    let (root, mut i, t) = fixture();
    let (mut started, _) = prepare_send(&mut i, &send(&t, "a", "owned-spawn"), false).unwrap();
    started.attempts[0].state = "spawn_intent".into();
    let a = started.attempts[0].clone();
    let s = i.store.as_mut().unwrap();
    let marker = s.process_intent(&started, &a).unwrap();
    let mut cmd = std::process::Command::new("/usr/bin/python3");
    cmd.args([
        "-c",
        "import time; print(\"ready\",flush=True); time.sleep(60)",
    ])
    .env_clear()
    .env(super::process_supervision::OWNER_ENV, &marker)
    .process_group(0)
    .stdout(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let pid = child.id();
    struct Cleanup(u32);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            unsafe {
                libc::kill(self.0 as i32, libc::SIGKILL);
            }
        }
    }
    let _cleanup = Cleanup(pid);
    use std::io::BufRead;
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    s.process_spawned(&a.attempt_id, pid).unwrap();
    drop(i);
    super::process_supervision::with_test_pids(&[pid], || {
        let mut s = Store::open(root.path().join("owner")).unwrap();
        assert_eq!(s.unresolved_accounts().unwrap(), vec!["a"]);
        assert!(s.reconcile_processes().unwrap().is_empty());
        assert!(!s.processes_settled("task").unwrap());
        let owned = super::process_supervision::remaining(&marker).unwrap();
        assert_eq!(
            owned.len(),
            1,
            "private fixture child must carry exact owned environment witness"
        );
        // This Store fixture checks reservation/reconciliation; the leaf's
        // detached-child fixture separately qualifies its supervised stop.
        child.kill().unwrap();
        child.wait().unwrap();
        super::process_supervision::stop(&marker).unwrap();
        s.reconcile_processes().unwrap();
        assert!(!s.processes_settled("task").unwrap());
        // Death of this observed child cannot attest to a lost native journal.
        // The durable pre-dispatch boundary still requires qualified completion
        // or verified boot change; effects remain explicitly uncertain.
        s.test_previous_boot(&a.attempt_id).unwrap();
        s.reconcile_processes().unwrap();
        assert!(s.processes_settled("task").unwrap());
        assert!(s.unresolved_accounts().unwrap().is_empty());
        assert_eq!(s.task("task").unwrap().state, TaskState::DeliveryUncertain);
    });
}

#[cfg(unix)]
#[test]
fn failed_final_projection_retains_worker_fence_until_durable_retry() {
    let (_root, mut i, t) = fixture();
    prepare_send(&mut i, &send(&t, "a", "final-receipt"), false).unwrap();
    let active = i.active.get("task").unwrap().clone();
    i.pending_finishes.insert(
        "task".into(),
        (active.clone(), Err("known pre-dispatch rejection".into())),
    );
    i.store.as_mut().unwrap().fail_next_update = true;
    assert!(super::execution::retry_finishes(&mut i).is_err());
    assert!(!active.worker_finished.load(Ordering::SeqCst));
    assert!(!active.done.load(Ordering::SeqCst));
    assert!(i.active.contains_key("task"));
    assert!(i.pending_finishes.contains_key("task"));
    assert_eq!(
        i.store.as_ref().unwrap().task("task").unwrap().state,
        TaskState::Starting
    );
    super::execution::retry_finishes(&mut i).unwrap();
    assert!(active.worker_finished.load(Ordering::SeqCst));
    assert!(active.done.load(Ordering::SeqCst));
    assert!(i.pending_finishes.is_empty());
    assert_eq!(
        i.store.as_ref().unwrap().task("task").unwrap().state,
        TaskState::Stopped
    );
    let recovered = i.store.as_ref().unwrap().task("task").unwrap();
    assert!(
        prepare_send(&mut i, &send(&recovered, "b", "after-durable-retry"), false)
            .unwrap()
            .1
    );
}

#[cfg(unix)]
#[test]
fn verified_boot_change_proves_process_death_but_never_replays_delivery() {
    let (root, mut i, t) = fixture();
    let (mut started, _) =
        prepare_send(&mut i, &send(&t, "a", "old-boot-spawn-gap"), false).unwrap();
    started.attempts[0].state = "spawn_intent".into();
    let attempt = started.attempts[0].clone();
    let s = i.store.as_mut().unwrap();
    s.process_intent(&started, &attempt).unwrap();
    assert_eq!(s.process_markers("task").unwrap().len(), 1);
    assert!(s.reconcile_processes().unwrap().is_empty());
    s.test_previous_boot(&attempt.attempt_id).unwrap();
    drop(i);
    let mut s = Store::open(root.path().join("owner")).unwrap();
    assert_eq!(s.task("task").unwrap().state, TaskState::DeliveryUncertain);
    s.reconcile_processes().unwrap();
    assert!(s.processes_settled("task").unwrap());
    assert!(s.unresolved_accounts().unwrap().is_empty());
    assert_eq!(
        s.task("task").unwrap().attempts[0].effects_state,
        "uncertain"
    );
}

#[cfg(unix)]
#[test]
fn explicit_untracked_native_effects_cannot_be_cleared_by_empty_owner_scan() {
    assert!(super::execution::untracked_effect(
        &json!({"input":{"run_in_background":true}})
    ));
    assert!(!super::execution::untracked_effect(
        &json!({"input":{"run_in_background":false}})
    ));
    let (_root, mut i, t) = fixture();
    let (started, _) = prepare_send(&mut i, &send(&t, "a", "untracked-native"), false).unwrap();
    let a = started.attempts[0].clone();
    let s = i.store.as_mut().unwrap();
    s.process_intent(&started, &a).unwrap();
    s.process_untracked(&a.attempt_id).unwrap();
    super::process_supervision::with_test_pids(&[], || {
        assert!(s.reconcile_processes().unwrap().is_empty());
        assert!(!s.processes_settled("task").unwrap());
    });
}

#[cfg(unix)]
#[test]
fn queued_allow_then_accepted_stop_emits_no_native_transport_frame() {
    use std::io::{Read, Write};
    let (_root, mut i, t) = fixture();
    let (_, _) = prepare_send(&mut i, &send(&t, "a", "queued-native-choice"), false).unwrap();
    let active = i.active.get("task").unwrap().clone();
    let (mut transport, mut observer) = std::os::unix::net::UnixStream::pair().unwrap();
    observer.set_nonblocking(true).unwrap();
    let pending_allow = json!({"method":"permission","choice":"allow"});
    {
        let _stop_control = active.control.lock().unwrap();
        active.stop.store(true, Ordering::SeqCst);
    }
    let mut consumed = false;
    super::execution::permission_dispatch(&active, || {
        consumed = true;
        serde_json::to_writer(&mut transport, &pending_allow).unwrap();
        transport.write_all(b"\n").unwrap();
        Ok(())
    })
    .unwrap();
    assert!(!consumed);
    let mut byte = [0u8; 1];
    assert_eq!(
        observer.read(&mut byte).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(!active.done.load(Ordering::SeqCst));
}

#[cfg(unix)]
#[test]
fn claude_external_tools_retain_unknown_ownership_while_file_tools_are_admitted() {
    use super::native_wire::{NativeKind, NativeTool, ToolState};
    let mut tool = NativeTool {
        session_id: "s".into(),
        turn_id: "t".into(),
        tool_id: "u".into(),
        state: ToolState::Started,
        raw: json!({"name":"Bash","input":{"command":"echo fixture"}}),
    };
    assert!(super::execution::tool_ownership_unqualified(
        NativeKind::Claude,
        &tool
    ));
    tool.raw = json!({"content_block":{"name":"Write","input":{"file_path":"fixture.txt","content":"safe"}}});
    assert!(!super::execution::tool_ownership_unqualified(
        NativeKind::Claude,
        &tool
    ));
    tool.raw = json!({"name":"Task"});
    assert!(super::execution::tool_ownership_unqualified(
        NativeKind::Claude,
        &tool
    ));
}

#[test]
fn pi_supplied_cancel_named_option_is_a_value_and_synthetic_cancel_is_null() {
    use super::native_wire::{NativeKind, NativeWire, WireRequest};
    for value in [json!("cancel"), json!("reject"), serde_json::Value::Null] {
        let mut wire = NativeWire::new(NativeKind::Pi, "session", "turn").unwrap();
        let events=wire.observe(json!({"type":"extension_ui_request","id":"choice","method":"select","options":["cancel","reject"]})).unwrap();
        let permission = events
            .into_iter()
            .find_map(|event| {
                if let super::native_wire::NativeEvent::Permission(p) = event {
                    Some(p)
                } else {
                    None
                }
            })
            .unwrap();
        let request:PermissionReply=serde_json::from_value(json!({"operationId":"choice-op","taskId":"task","attemptId":"attempt","generation":1,"approvalToken":"token","choice":value,"allow":!value.is_null(),"editorFreezeToken":null})).unwrap();
        let choice = super::commands::permission_choice(&request, &permission).unwrap();
        let WireRequest::Stdio(frame) = wire.permission_reply(&json!("choice"), choice).unwrap()
        else {
            panic!("Pi must use its native stdio");
        };
        if value.is_null() {
            assert_eq!(frame["cancelled"], true);
            assert!(frame.get("value").is_none());
        } else {
            assert_eq!(frame["value"], value);
            assert!(frame.get("cancelled").is_none());
        }
    }
}

#[cfg(unix)]
#[test]
fn failed_final_task_load_retains_whole_finish_work_for_next_owner_entry() {
    let (_root, mut i, t) = fixture();
    prepare_send(&mut i, &send(&t, "a", "finish-read-error"), false).unwrap();
    let active = i.active.get("task").unwrap().clone();
    i.pending_finishes.insert(
        "task".into(),
        (active.clone(), Err("known pre-dispatch rejection".into())),
    );
    i.store.as_ref().unwrap().fail_next_read.set(true);
    assert!(super::execution::retry_finishes(&mut i).is_err());
    assert!(i.pending_finishes.contains_key("task"));
    assert!(!active.worker_finished.load(Ordering::SeqCst));
    assert!(!active.done.load(Ordering::SeqCst));
    super::execution::retry_finishes(&mut i).unwrap();
    assert!(i.pending_finishes.is_empty());
    assert!(active.done.load(Ordering::SeqCst));
    assert!(!i.active.contains_key("task"));
    assert_eq!(
        i.store.as_ref().unwrap().task("task").unwrap().state,
        TaskState::Stopped
    );
}

#[cfg(unix)]
#[test]
fn explicitly_denied_claude_tool_proposal_does_not_claim_external_dispatch() {
    use super::{
        native_wire::{NativePermission, NativeTool, ToolState},
        service::Permission,
    };
    let (_root, mut i, mut t) = fixture();
    t.cli = Some(TitleCli::Claude);
    let (started, _) = prepare_send(&mut i, &send(&t, "a", "deny-tool"), false).unwrap();
    let mut t = started;
    t.cli = Some(TitleCli::Claude);
    let attempt = t.attempts[0].clone();
    let tool = NativeTool {
        session_id: "s".into(),
        turn_id: "t".into(),
        tool_id: "bash".into(),
        state: ToolState::Started,
        raw: json!({"name":"Bash"}),
    };
    append(
        &mut t,
        &attempt,
        "tool",
        "started",
        serde_json::to_value(tool).unwrap(),
    );
    assert!(super::execution::unqualified_observed_tools(
        &t,
        &attempt.attempt_id,
        &i.permissions
    ));
    i.permissions.insert(
        "deny".into(),
        Permission {
            preview: PendingPermission {
                task_id: t.task_id.clone(),
                attempt_id: attempt.attempt_id.clone(),
                generation: attempt.generation,
                approval_token: "deny".into(),
                project_root: t.cwd.clone(),
                permission: NativePermission {
                    request_id: json!("ask"),
                    session_id: "s".into(),
                    turn_id: "t".into(),
                    tool_id: Some("bash".into()),
                    choices: vec![json!("deny")],
                    raw: json!({"request":{"tool_name":"Bash"}}),
                },
            },
            decision: Some((json!("deny"), false, String::new())),
            consumed: true,
        },
    );
    assert!(!super::execution::unqualified_observed_tools(
        &t,
        &attempt.attempt_id,
        &i.permissions
    ));
}

#[test]
fn restart_preserves_committed_checkpoint_and_clears_unfinalized_active_projection() {
    let (root, mut i, t) = fixture();
    let (mut done, _) = prepare_send(&mut i, &send(&t, "a", "checkpoint-crash"), false).unwrap();
    done.state = TaskState::Completed;
    let a = &mut done.attempts[0];
    a.state = "completed".into();
    a.effects_state = "settled".into();
    let c = HistoryCheckpoint {
        task_id: done.task_id.clone(),
        attempt_id: a.attempt_id.clone(),
        account_id: a.account_id.clone(),
        auth_revision: a.auth_revision,
        generation: a.generation,
        history_revision: done.history_revision,
        digest: super::service::history_digest(&done).unwrap(),
        version: "0.160.0".into(),
        model: done.model.clone(),
        cwd_identity: workspace(root.path()).unwrap().1,
        native_ref: "thread".into(),
        session_file: None,
        settled: true,
    };
    i.store
        .as_mut()
        .unwrap()
        .update_task(&done, Some(&c), Some(("checkpoint-crash", "settled")))
        .unwrap();
    drop(i);
    let s = Store::open(root.path().join("owner")).unwrap();
    let restored = s.task("task").unwrap();
    assert_eq!(restored.state, TaskState::Completed);
    assert!(restored.active_attempt_id.is_none());
    assert!(restored.active_account_id.is_none());
    assert_eq!(s.checkpoint("task").unwrap().unwrap().digest, c.digest);
    assert_eq!(restored.attempts[0].state, "completed");
}

#[cfg(unix)]
#[test]
fn durable_unknown_tool_proposal_survives_restart_and_correlated_deny_clears_only_it() {
    let (root, mut i, t) = fixture();
    let (started, _) =
        prepare_send(&mut i, &send(&t, "a", "unknown-tool-proposal"), false).unwrap();
    let attempt = started.attempts[0].clone();
    let s = i.store.as_mut().unwrap();
    s.process_intent(&started, &attempt).unwrap();
    s.process_tool_untracked(&attempt.attempt_id, "first-bash")
        .unwrap();
    s.process_tool_untracked(&attempt.attempt_id, "second-bash")
        .unwrap();
    drop(i);
    let mut s = Store::open(root.path().join("owner")).unwrap();
    s.process_tool_denied(&attempt.attempt_id, "first-bash")
        .unwrap();
    assert!(!s.processes_settled("task").unwrap());
    assert!(s.reconcile_processes().unwrap().is_empty());
    s.process_tool_denied(&attempt.attempt_id, "second-bash")
        .unwrap();
    // No stored child identity still means unknown creation gap, never guessed death.
    assert!(!s.processes_settled("task").unwrap());
    s.process_not_spawned(&attempt.attempt_id).unwrap();
    assert!(s.processes_settled("task").unwrap());
}

#[test]
fn unqualified_active_stop_switch_cannot_cross_backend_dispatch_boundary() {
    let (_root, mut i, t) = fixture();
    let (first, _) = prepare_send(&mut i, &send(&t, "a", "qualification-source"), false).unwrap();
    let mut t = settle(&mut i, first);
    t.switches.push(SwitchOperation {
        operation_id: "unqualified-switch".into(),
        source_attempt_id: Some(t.attempts[0].attempt_id.clone()),
        account_id: "b".into(),
        auth_revision: 1,
        history_revision: t.history_revision,
        mode: "stop_and_continue".into(),
        phase: "stopping_source".into(),
        continuation_method: "blocked".into(),
        reason: None,
        coverage: 0,
        budget_bytes: CONTEXT_BUDGET as u64,
        context_digest: None,
        stop_supervision_qualified: false,
    });
    settle_switches(i.store.as_ref().unwrap(), &mut t).unwrap();
    assert_eq!(t.switches[0].phase, "prepared");
    assert!(super::service::switch_dispatch_allowed(&t.switches[0]).is_err());
    assert_eq!(t.attempts.len(), 1);
}

#[test]
fn imported_native_a_b_generations_keep_exact_auth_and_observation_provenance() {
    use std::collections::{BTreeMap, BTreeSet};
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("archive");
    let native = archive.join("originals/runs/run/native");
    std::fs::create_dir_all(&native).unwrap();
    for (generation, profile, input, auth) in [(1, "a", "ia", 3), (2, "b", "ib", 7)] {
        std::fs::write(native.join(format!("attempt-{generation}.json")),serde_json::to_vec(&json!({"schema":1,"runId":"run","inputId":input,"profileId":profile,"profileRevision":auth,"generation":generation,"sessionId":format!("s-{profile}"),"version":"0.160.0","output":format!("{profile} native work"),"tools":[{"toolId":format!("tool-{profile}"),"state":"completed"}],"ledger":{"observed":format!("{profile} native journal")}})).unwrap()).unwrap();
    }
    let snapshot = json!({"profiles":[{"id":"a","cli":"codex","label":"A","revision":10,"enabled":false},{"id":"b","cli":"codex","label":"B","revision":11,"enabled":false}],"routers":[{"id":"router","cli":"codex"}],"runs":[{"id":"run","routerId":"router","cwd":root.path(),"model":"gpt-6.1-sol","generation":2,"inputs":[{"id":"ia","text":"A user"},{"id":"ib","text":"B user"}],"attempts":[{"id":"attempt-a","inputId":"ia","profileId":"a","generation":1,"state":"completed"},{"id":"attempt-b","inputId":"ib","profileId":"b","generation":2,"state":"completed"}],"turns":[],"allowedProfileIds":["a","b"]}]});
    let bundle = super::migration::ImportBundle {
        migration_id: "native-provenance".into(),
        source_schema: 5,
        snapshot,
        requests: vec![],
        credential_journal: vec![],
        run_id_map: BTreeMap::from([("run".into(), "legacy:run".into())]),
        missing_run_ids: BTreeSet::new(),
        physical_auth_roots: BTreeMap::new(),
        keyring_service: "dev.lomi.desktop.cli-router".into(),
        archive,
    };
    let mut s = Store::open(root.path().join("owner")).unwrap();
    s.import_bundle(&bundle).unwrap();
    let t = s.task("legacy:run").unwrap();
    let history = t
        .history
        .iter()
        .filter(|h| h.kind == "legacy_native_observation")
        .collect::<Vec<_>>();
    assert_eq!(history.len(), 2);
    assert_eq!(
        (
            &*history[0].attempt_id,
            &*history[0].account_id,
            history[0].auth_revision
        ),
        ("attempt-a", "a", 3)
    );
    assert_eq!(
        (
            &*history[1].attempt_id,
            &*history[1].account_id,
            history[1].auth_revision
        ),
        ("attempt-b", "b", 7)
    );
    assert_eq!(t.attempts[0].auth_revision, 3);
    assert_eq!(t.attempts[1].auth_revision, 7);
    assert!(t.availability_reason.is_none());
    s.validate_records().unwrap();
    drop(s);
    let s = Store::open(root.path().join("owner")).unwrap();
    assert_eq!(s.task("legacy:run").unwrap().history.len(), t.history.len());
}

#[cfg(unix)]
#[test]
fn exact_reviewed_pi_transfer_preserves_oversize_context_and_rejects_stale_fences() {
    let (_root, mut i, t) = fixture();
    let (started, _) = prepare_send(&mut i, &send(&t, "a", "pi-source"), false).unwrap();
    let mut t = settle(&mut i, started);
    t.cli = Some(TitleCli::Pi);
    t.model = "openai/gpt-4.1".into();
    t.reasoning_effort = None;
    let source = t.attempts[0].clone();
    let text = "x".repeat(CONTEXT_BUDGET + 1);
    append(&mut t, &source, "assistant", "completed", json!(text));
    assert!(handoff(&t).is_err());
    let s = i.store.as_mut().unwrap();
    for id in ["a", "b"] {
        let mut account = account(id);
        account.cli = TitleCli::Pi;
        account.accepted_version = Some("1.0.1".into());
        let mut binding = s.binding(id).unwrap();
        binding.namespace = "pi".into();
        s.save_account(
            &json!({"pi":id}),
            &format!("pi-account-{id}"),
            &account,
            &binding,
            &json!({}),
        )
        .unwrap();
    }
    let root = s.task_root("task").unwrap();
    let mut entries = super::native_transfer::tests::entries();
    entries[0]["cwd"] = json!(t.cwd);
    entries[4]["message"]["content"][0]["text"] = json!(text);
    let mut bytes = vec![];
    for entry in entries {
        bytes.extend(serde_json::to_vec(&entry).unwrap());
        bytes.push(b'\n');
    }
    let file = root.join("pi-session-1.jsonl");
    std::fs::write(&file, &bytes).unwrap();
    crate::chat::storage::private(&file, false).unwrap();
    let c = HistoryCheckpoint {
        task_id: t.task_id.clone(),
        attempt_id: source.attempt_id.clone(),
        account_id: "a".into(),
        auth_revision: 1,
        generation: t.generation,
        history_revision: t.history_revision,
        digest: super::service::history_digest(&t).unwrap(),
        version: "1.0.1".into(),
        model: t.model.clone(),
        cwd_identity: workspace(std::path::Path::new(&t.cwd)).unwrap().1,
        native_ref: "source".into(),
        session_file: Some("pi-session-1.jsonl".into()),
        settled: true,
    };
    s.update_task(&t, Some(&c), None).unwrap();
    let request = super::transfer::request(s, &t, "b", 1).unwrap();
    let (digest, size) = super::native_transfer::inspect(&root, &request).unwrap();
    assert!(
        super::native_transfer::prepare_reviewed(&root, &request, &"0".repeat(64), size).is_err()
    );
    super::native_transfer::prepare_reviewed(&root, &request, &digest, size).unwrap();
    t.switches.push(SwitchOperation {
        operation_id: "pi-transfer-review".into(),
        source_attempt_id: Some(source.attempt_id),
        account_id: "b".into(),
        auth_revision: 1,
        history_revision: t.history_revision,
        mode: "reviewed_transfer".into(),
        phase: "prepared".into(),
        continuation_method: "reviewed_transfer".into(),
        reason: None,
        coverage: t.history_revision,
        budget_bytes: CONTEXT_BUDGET as u64,
        context_digest: Some(digest),
        stop_supervision_qualified: true,
    });
    s.update_task(&t, None, None).unwrap();
    assert_eq!(
        continuation(s, &t, &s.account("b").unwrap()).unwrap(),
        "reviewed_transfer"
    );
    let mut stale = t.clone();
    stale.history_revision += 1;
    assert!(super::transfer::request(s, &stale, "b", 1).is_err());
    assert!(super::transfer::request(s, &t, "b", 2).is_err());
}

#[cfg(unix)]
#[test]
fn failed_observation_write_retries_exact_text_and_remaining_native_tool_batch_before_finish() {
    use super::native_wire::{NativeEvent, NativeTool, ToolState};
    let (_root, mut i, t) = fixture();
    let (mut observed, _) =
        prepare_send(&mut i, &send(&t, "a", "observed-write-failure"), false).unwrap();
    let active = i.active.get("task").unwrap().clone();
    observed.attempts[0].state = "dispatched".into();
    let attempt = observed.attempts[0].clone();
    i.store
        .as_mut()
        .unwrap()
        .process_intent(&observed, &attempt)
        .unwrap();
    append(
        &mut observed,
        &attempt,
        "assistant",
        "partial",
        json!({"text":"first observed text"}),
    );
    observed.attempts[0].output.push_str("first observed text");
    let s = i.store.as_mut().unwrap();
    s.fail_next_update = true;
    assert!(s.update_task(&observed, None, None).is_err());
    i.pending_observations.insert("task".into(), observed);
    let tool = NativeTool {
        session_id: "s".into(),
        turn_id: "t".into(),
        tool_id: "file-change".into(),
        state: ToolState::Completed,
        raw: json!({"type":"fileChange","content":"exact tool result"}),
    };
    i.pending_event_batches.insert(
        "task".into(),
        (
            attempt.clone(),
            vec![
                NativeEvent::Text {
                    session_id: "s".into(),
                    turn_id: "t".into(),
                    text: "remaining batch text".into(),
                },
                NativeEvent::Tool(tool),
            ],
        ),
    );
    super::execution::retry_observations(&mut i).unwrap();
    assert!(i.pending_observations.is_empty());
    assert!(i.pending_event_batches.is_empty());
    let stored = i.store.as_ref().unwrap().task("task").unwrap();
    assert_eq!(
        stored.attempts[0].output,
        "first observed textremaining batch text"
    );
    assert!(handoff(&stored).unwrap().contains("exact tool result"));
    assert!(handoff(&stored).unwrap().contains("first observed text"));
    i.store
        .as_mut()
        .unwrap()
        .test_previous_boot(&attempt.attempt_id)
        .unwrap();
    i.pending_finishes.insert(
        "task".into(),
        (
            active.clone(),
            Err("Observed write interrupted native execution; no replay".into()),
        ),
    );
    super::execution::retry_finishes(&mut i).unwrap();
    assert!(active.done.load(Ordering::SeqCst));
    assert_eq!(
        i.store.as_ref().unwrap().task("task").unwrap().state,
        TaskState::RecoveryRequired
    );
    assert_eq!(
        i.store.as_ref().unwrap().task("task").unwrap().attempts[0].output,
        "first observed textremaining batch text"
    );
}

#[cfg(unix)]
#[test]
fn native_artifact_and_tool_names_cannot_discharge_unqualified_lifecycle_helpers() {
    use super::native_wire::{NativeKind, NativeTool, ToolState};
    for kind in [
        NativeKind::Codex,
        NativeKind::Claude,
        NativeKind::Kimi,
        NativeKind::Kilo,
        NativeKind::OpenCode,
        NativeKind::Grok,
    ] {
        assert!(!super::execution::native_boundary_qualified(kind));
    }
    assert!(!super::execution::native_boundary_qualified(NativeKind::Pi));
    let mut tool = NativeTool {
        session_id: "s".into(),
        turn_id: "t".into(),
        tool_id: "builtin-read".into(),
        state: ToolState::Started,
        raw: json!({"toolName":"read"}),
    };
    assert!(!super::execution::tool_ownership_unqualified(
        NativeKind::Pi,
        &tool
    ));
    tool.raw = json!({"toolName":"bash"});
    assert!(super::execution::tool_ownership_unqualified(
        NativeKind::Pi,
        &tool
    ));
}

#[cfg(unix)]
#[test]
fn completed_only_external_native_tool_never_proves_descendant_settlement() {
    use super::native_wire::{NativeKind, NativeTool, ToolState};
    for state in [ToolState::Completed, ToolState::Failed] {
        let mut tool = NativeTool {
            session_id: "s".into(),
            turn_id: "t".into(),
            tool_id: "external-completed-only".into(),
            state,
            raw: json!({"toolName":"bash"}),
        };
        assert!(super::execution::tool_ownership_unqualified(
            NativeKind::Pi,
            &tool
        ));
        tool.raw = json!({"toolName":"read"});
        assert!(!super::execution::tool_ownership_unqualified(
            NativeKind::Pi,
            &tool
        ));
    }
}

fn change_helper_boot(store: &Store, id: &str, boot: &str) {
    let db = rusqlite::Connection::open(store.root.join("runtime.sqlite")).unwrap();
    let raw: String = db
        .query_row(
            "SELECT record FROM native_operations WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    let mut op: NativeOperation = serde_json::from_str(&raw).unwrap();
    op.boot = boot.into();
    db.execute(
        "UPDATE native_operations SET record=?2 WHERE id=?1",
        rusqlite::params![id, super::store::encode(&op).unwrap()],
    )
    .unwrap();
}
#[test]
fn helper_intent_survives_exit_restart_and_same_boot_ack_cannot_release() {
    let (_root, mut i, t) = fixture();
    let s = i.store.as_mut().unwrap();
    let a = s.account("a").unwrap();
    let op = s.helper_intent("verify", &a, &t.cwd, None).unwrap();
    // The ledger is committed before any helper executes. An ordinary clean
    // exit cannot prove that native lifecycle helpers left no detached work.
    let status = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("exit 0")
        .env_clear()
        .status()
        .unwrap();
    assert!(status.success());
    let path = s.root.clone();
    drop(i.store.take());
    let mut s = Store::open(path).unwrap();
    assert_eq!(s.native_operations().unwrap()[0].operation_id, op);
    assert!(s.unresolved_accounts().unwrap().contains(&"a".into()));
    let recovery = s.account_recovery("a").unwrap().unwrap();
    assert!(!recovery.recoverable);
    assert!(recovery.requires_verified_boot_change);
    assert!(s
        .recover_helpers(&AccountRecover {
            operation_id: "same-boot-ack".into(),
            account_id: "a".into(),
            expected_revision: 1,
            acknowledge_effects: true
        })
        .is_err());
    assert_eq!(s.account("a").unwrap().auth_revision, 1);
    assert!(!s.helpers_settled(None).unwrap());
}
#[test]
fn helper_recovery_requires_all_old_boot_operations_and_invalidates_auth_grants_atomically() {
    let (_root, mut i, t) = fixture();
    let s = i.store.as_mut().unwrap();
    let a = s.account("a").unwrap();
    let first = s.helper_intent("verify", &a, &t.cwd, None).unwrap();
    let second = s
        .helper_intent("account_terminal", &a, &t.cwd, None)
        .unwrap();
    change_helper_boot(s, &first, "fixture-previous-kernel-boot");
    assert!(!s.account_recovery("a").unwrap().unwrap().recoverable);
    change_helper_boot(s, &second, "fixture-previous-kernel-boot");
    assert!(s.account_recovery("a").unwrap().unwrap().recoverable);
    let mut request = AccountRecover {
        operation_id: "review-helper-effects".into(),
        account_id: "a".into(),
        expected_revision: 2,
        acknowledge_effects: true,
    };
    assert!(s.recover_helpers(&request).is_err());
    request.expected_revision = 1;
    // Inject a real SQLite failure after account UPDATE but before commit.
    let db = rusqlite::Connection::open(s.root.join("runtime.sqlite")).unwrap();
    db.execute_batch("CREATE TEMP TABLE unused(id INTEGER);")
        .unwrap();
    db.execute_batch("CREATE TRIGGER helper_recovery_fail BEFORE UPDATE ON native_operations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(s.recover_helpers(&request).is_err());
    assert_eq!(s.account("a").unwrap().auth_revision, 1);
    assert_eq!(s.binding("a").unwrap().auth_revision, 1);
    assert!(s
        .task(&t.task_id)
        .unwrap()
        .grants
        .iter()
        .any(|g| g.account_id == "a"));
    assert!(!s.helpers_settled(None).unwrap());
    db.execute_batch("DROP TRIGGER helper_recovery_fail;")
        .unwrap();
    let result = s.recover_helpers(&request).unwrap();
    assert_eq!(s.account("a").unwrap().auth_revision, 2);
    assert_eq!(s.binding("a").unwrap().auth_revision, 2);
    assert_eq!(s.account("a").unwrap().auth_state, "unverified");
    assert!(!s
        .task(&t.task_id)
        .unwrap()
        .grants
        .iter()
        .any(|g| g.account_id == "a"));
    assert!(s
        .task(&t.task_id)
        .unwrap()
        .grants
        .iter()
        .any(|g| g.account_id == "b"));
    assert!(s.helpers_settled(None).unwrap());
    assert!(result
        .accounts
        .iter()
        .find(|a| a.account_id == "a")
        .unwrap()
        .recovery
        .is_none());
    assert_eq!(
        s.recover_helpers(&request).unwrap().revision,
        result.revision
    );
    assert_eq!(s.account("a").unwrap().auth_revision, 2);
}
#[test]
fn known_schema_one_upgrade_preserves_exact_private_sqlite_backup() {
    let (_root, mut i, _t) = fixture();
    let s = i.store.take().unwrap();
    let path = s.root.clone();
    drop(s);
    let db = rusqlite::Connection::open(path.join("runtime.sqlite")).unwrap();
    db.execute_batch("DROP TABLE native_operations; PRAGMA user_version=1;")
        .unwrap();
    drop(db);
    let s = Store::open(path.clone()).unwrap();
    assert_eq!(s.accounts().unwrap().len(), 2);
    let backup = rusqlite::Connection::open_with_flags(
        path.join("schema-1-backup.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(s.native_operations().unwrap().len(), 0);
    drop(s);
    assert!(Store::open(path).is_ok());
}
#[test]
fn reviewed_helper_history_remains_valid_after_account_removal_and_restart() {
    let (_root, mut i, t) = fixture();
    let s = i.store.as_mut().unwrap();
    let a = s.account("a").unwrap();
    let op = s.helper_intent("verify", &a, &t.cwd, None).unwrap();
    change_helper_boot(s, &op, "fixture-previous-boot");
    s.recover_helpers(&AccountRecover {
        operation_id: "review-before-remove".into(),
        account_id: "a".into(),
        expected_revision: 1,
        acknowledge_effects: true,
    })
    .unwrap();
    s.delete_account(
        &json!({"remove":"a"}),
        "remove-recovered-account",
        "a",
        &json!({}),
    )
    .unwrap();
    let path = s.root.clone();
    drop(i.store.take());
    let s = Store::open(path).unwrap();
    let archived = s.native_operations().unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].state, "reviewed");
    assert_eq!(archived[0].cli, TitleCli::Codex);
    assert_eq!(archived[0].auth_revision, 1);
    assert!(s.account("a").is_err());
    assert!(s.helpers_settled(None).unwrap());
}
#[test]
fn schema_upgrade_reopens_after_durable_backup_before_schema_commit() {
    let (_root, mut i, _) = fixture();
    let s = i.store.take().unwrap();
    let path = s.root.clone();
    drop(s);
    let db = rusqlite::Connection::open(path.join("runtime.sqlite")).unwrap();
    db.execute_batch("DROP TABLE native_operations; PRAGMA user_version=1;")
        .unwrap();
    super::store::prepare_schema_one_upgrade(&db, &path).unwrap();
    let original = std::fs::read(path.join("schema-1-backup.sqlite")).unwrap();
    drop(db);
    let s = Store::open(path.clone()).unwrap();
    assert_eq!(s.accounts().unwrap().len(), 2);
    assert_eq!(
        std::fs::read(path.join("schema-1-backup.sqlite")).unwrap(),
        original
    );
    assert!(s.native_operations().unwrap().is_empty());
}
#[test]
fn unavailable_native_family_refuses_helper_intent_without_publication() {
    let (_root, mut i, t) = fixture();
    let s = i.store.as_mut().unwrap();
    let mut a = s.account("a").unwrap();
    a.cli = TitleCli::Gemini;
    let revision = s.revision().unwrap();
    assert!(s.helper_intent("verify", &a, &t.cwd, None).is_err());
    assert!(s.native_operations().unwrap().is_empty());
    assert_eq!(s.revision().unwrap(), revision);
}
#[test]
fn durable_helper_controls_block_auth_login_verify_remove_and_send_without_ram_lease() {
    let (_root, mut i, t) = fixture();
    let s = i.store.as_mut().unwrap();
    let a = s.account("a").unwrap();
    s.helper_intent("verify", &a, &t.cwd, None).unwrap();
    // Every control mutation and new Send uses this same durable admission
    // predicate; no RAM lease is needed for a post-crash helper fence.
    assert!(super::service::account_control_reserved(s, &Default::default(), "a").unwrap());
    assert!(!super::service::account_control_reserved(s, &Default::default(), "b").unwrap());
    let path = s.root.clone();
    drop(i.store.take());
    let s = Store::open(path).unwrap();
    assert!(super::service::account_control_reserved(&s, &Default::default(), "a").unwrap());
}
#[test]
fn close_accepted_before_helper_reservation_prevents_intent_and_execution() {
    let (_root, mut i, t) = fixture();
    let a = i.store.as_ref().unwrap().account("a").unwrap();
    assert!(!super::service::account_control_reserved(
        i.store.as_ref().unwrap(),
        &i.account_leases,
        "a"
    )
    .unwrap());
    // The previously read available account is deliberately paused while
    // shutdown wins. Reservation and durable intent share the final owner gate.
    let closing = true;
    assert!(super::service::begin_helper(&mut i, closing, &a, &t.cwd, "verify").is_err());
    assert!(super::service::begin_helper(&mut i, closing, &a, &t.cwd, "account_terminal").is_err());
    assert!(i.store.as_ref().unwrap().helpers_settled(None).unwrap());
    assert!(i
        .store
        .as_ref()
        .unwrap()
        .native_operations()
        .unwrap()
        .is_empty());
    assert!(i.account_leases.is_empty());
    assert!(i.recovery_projects.is_empty());
}
#[test]
fn initial_empty_database_recovers_only_with_exact_owned_initialization_intent() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("owner");
    std::fs::create_dir(&path).unwrap();
    crate::chat::storage::private(&path, true).unwrap();
    super::store::publish_initialization_intent(&path).unwrap();
    let db = rusqlite::Connection::open(path.join("runtime.sqlite")).unwrap();
    drop(db);
    crate::chat::storage::private(&path.join("runtime.sqlite"), false).unwrap();
    let s = Store::open(path).unwrap();
    assert!(s.accounts().unwrap().is_empty());
    assert!(s.native_operations().unwrap().is_empty());
    let unknown = root.path().join("unknown");
    std::fs::create_dir(&unknown).unwrap();
    crate::chat::storage::private(&unknown, true).unwrap();
    let db = rusqlite::Connection::open(unknown.join("runtime.sqlite")).unwrap();
    drop(db);
    crate::chat::storage::private(&unknown.join("runtime.sqlite"), false).unwrap();
    let before = std::fs::read(unknown.join("runtime.sqlite")).unwrap();
    assert!(Store::open(unknown.clone()).is_err());
    assert_eq!(
        std::fs::read(unknown.join("runtime.sqlite")).unwrap(),
        before
    );
}
#[test]
fn schema_upgrade_preserves_partial_vacuum_stage_and_exclusive_publish_crash_gap() {
    let (_root, mut i, _) = fixture();
    let s = i.store.take().unwrap();
    let path = s.root.clone();
    drop(s);
    let db = rusqlite::Connection::open(path.join("runtime.sqlite")).unwrap();
    db.execute_batch("DROP TABLE native_operations; PRAGMA user_version=1;")
        .unwrap();
    super::store::prepare_schema_one_upgrade(&db, &path).unwrap();
    let backup = path.join("schema-1-backup.sqlite");
    let stage = path.join("schema-1-backup.staged.sqlite");
    std::fs::remove_file(&backup).unwrap();
    std::fs::write(&stage, b"partial VACUUM snapshot").unwrap();
    crate::chat::storage::private(&stage, false).unwrap();
    super::store::prepare_schema_one_upgrade(&db, &path).unwrap();
    let partial = std::fs::read_dir(&path)
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("schema-1-backup.partial-")
        })
        .unwrap();
    assert_eq!(
        std::fs::read(partial.path()).unwrap(),
        b"partial VACUUM snapshot"
    );
    std::fs::hard_link(&backup, &stage).unwrap();
    drop(db);
    let s = Store::open(path.clone()).unwrap();
    assert_eq!(s.accounts().unwrap().len(), 2);
    assert!(!stage.exists());
    assert!(backup.exists());
}
#[test]
fn reviewed_attempt_linked_helper_survives_missing_account_and_task_without_replay() {
    let (_root, mut i, t) = fixture();
    let (active, _) = prepare_send(&mut i, &send(&t, "a", "source-helper-attempt"), false).unwrap();
    let s = i.store.as_mut().unwrap();
    let a = s.account("a").unwrap();
    let attempt = active.attempts.last().unwrap();
    let operation = s
        .helper_intent("attempt_prepare", &a, &t.cwd, Some((&t.task_id, attempt)))
        .unwrap();
    change_helper_boot(s, &operation, "fixture-previous-boot");
    let op = s
        .native_operations()
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == operation)
        .unwrap();
    assert!(!s.helper_requires_live_fence(&op).unwrap());
    s.recover_helpers(&AccountRecover {
        operation_id: "review-orphan-attempt".into(),
        account_id: "a".into(),
        expected_revision: 1,
        acknowledge_effects: true,
    })
    .unwrap();
    s.delete_account(
        &json!({"remove":"a"}),
        "remove-orphan-account",
        "a",
        &json!({}),
    )
    .unwrap();
    let path = s.root.clone();
    drop(i.store.take());
    let db = rusqlite::Connection::open(path.join("runtime.sqlite")).unwrap();
    db.execute("DELETE FROM tasks WHERE id=?1", [&t.task_id])
        .unwrap();
    drop(db);
    let s = Store::open(path).unwrap();
    let history = s.native_operations().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].task_id.as_deref(), Some("task"));
    assert_eq!(
        history[0].attempt_id.as_deref(),
        Some(attempt.attempt_id.as_str())
    );
    assert_eq!(history[0].state, "reviewed");
    assert!(s.helpers_settled(None).unwrap());
}
#[test]
fn dangling_schema_backup_stage_is_refused_without_writing_its_target() {
    use std::os::unix::fs::symlink;
    let (_root, mut i, _) = fixture();
    let s = i.store.take().unwrap();
    let path = s.root.clone();
    drop(s);
    let db = rusqlite::Connection::open(path.join("runtime.sqlite")).unwrap();
    db.execute_batch("DROP TABLE native_operations; PRAGMA user_version=1;")
        .unwrap();
    let outside = path.parent().unwrap().join("must-not-be-created.sqlite");
    symlink(&outside, path.join("schema-1-backup.staged.sqlite")).unwrap();
    assert!(super::store::prepare_schema_one_upgrade(&db, &path).is_err());
    assert!(!outside.exists());
    assert!(
        std::fs::symlink_metadata(path.join("schema-1-backup.staged.sqlite"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn repeated_query_observation_failures_append_all_interleaved_frames_once() {
    use super::native_wire::NativeEvent;
    use std::cell::RefCell;
    let (_root, mut inner, task) = fixture();
    let (mut projected, _) = prepare_send(
        &mut inner,
        &send(&task, "a", "query-retention-failure"),
        false,
    )
    .unwrap();
    let attempt = projected.attempts[0].clone();
    let owner = RefCell::new(inner);
    let event = |label: &str| NativeEvent::Observed(json!({"frame": label}));
    let first = super::execution::observe_batch(
        vec![event("A"), event("B")],
        |event, represented| {
            let mut inner = owner.borrow_mut();
            append(
                &mut projected,
                &attempt,
                "observation",
                "observed",
                serde_json::to_value(event).unwrap(),
            );
            inner.store.as_mut().unwrap().fail_next_update = true;
            let error = inner
                .store
                .as_mut()
                .unwrap()
                .update_task(&projected, None, None)
                .unwrap_err();
            inner
                .pending_observations
                .insert(projected.task_id.clone(), projected.clone());
            *represented = true;
            Err(error)
        },
        |events| {
            super::execution::retain_event_batch(
                &mut owner.borrow_mut(),
                &task.task_id,
                &attempt,
                events,
            )
        },
    );
    assert!(first.is_err());
    // A correlated query reply can be followed by another callback while the
    // first projected write still fails. That projection only contains A.
    let trailing = super::execution::observe_batch(
        vec![event("C"), event("D")],
        |_, represented| {
            assert!(!*represented);
            let mut inner = owner.borrow_mut();
            inner.store.as_mut().unwrap().fail_next_update = true;
            super::execution::retry_observations(&mut inner)
        },
        |events| {
            super::execution::retain_event_batch(
                &mut owner.borrow_mut(),
                &task.task_id,
                &attempt,
                events,
            )
        },
    );
    assert!(trailing.is_err());
    let mut inner = owner.into_inner();
    assert_eq!(inner.pending_event_batches[&task.task_id].1.len(), 3);
    super::execution::retry_observations(&mut inner).unwrap();
    super::execution::retry_observations(&mut inner).unwrap();
    let saved = inner.store.as_ref().unwrap().task(&task.task_id).unwrap();
    for label in ["A", "B", "C", "D"] {
        let exact = format!("\"frame\":\"{label}\"");
        assert_eq!(
            saved
                .history
                .iter()
                .filter(|h| serde_json::to_string(&h.content).unwrap().contains(&exact))
                .count(),
            1
        );
    }
    assert!(inner.pending_observations.is_empty());
    assert!(inner.pending_event_batches.is_empty());
}
