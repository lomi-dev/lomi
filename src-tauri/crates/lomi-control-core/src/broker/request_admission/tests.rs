use super::*;
use crate::client::Client;
use lomi_control_protocol::EmptyInput;
use std::sync::atomic::AtomicUsize;

struct ReadOnlyProject {
    available: Arc<AtomicBool>,
    workspaces: HashSet<String>,
}
impl SessionRequestPolicy for ReadOnlyProject {
    fn check(&self, request: &Request) -> Result<(), ErrorCode> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(ErrorCode::ControlRevoked);
        }
        let workspace = match request {
            Request::Status(_) => return Ok(()),
            Request::Connect(input) => &input.workspace_id,
            Request::FilesRead(input) => &input.workspace_id,
            Request::FilesList(input) => &input.workspace_id,
            Request::FilesSearch(input) => &input.workspace_id,
            _ => return Err(ErrorCode::ScopeDenied),
        };
        if self.workspaces.contains(workspace) {
            Ok(())
        } else {
            Err(ErrorCode::ScopeDenied)
        }
    }
}

fn publish(broker: &Broker, root: &Path) {
    broker
        .publish(Projection {
            ui_epoch: broker.register_ui().unwrap(),
            revision: "1".into(),
            workspaces: [
                ("project-a-1", "project-a"),
                ("project-a-2", "project-a"),
                ("project-b", "other-project"),
            ]
            .into_iter()
            .map(|(id, project)| Workspace {
                id: id.into(),
                project_id: project.into(),
                name: id.into(),
                project_name: project.into(),
                active_panel_id: None,
                project_path: root.canonicalize().unwrap().to_string_lossy().into(),
            })
            .collect(),
            ..Projection::default()
        })
        .unwrap();
}
async fn pair(broker: &Arc<Broker>, label: &str) -> (String, Client) {
    let endpoint = broker.endpoint.clone();
    let label = label.to_owned();
    let (send, receive) = oneshot::channel();
    let task = tokio::spawn(async move {
        Client::connect(&endpoint, &label, |id| {
            let _ = send.send(id);
        })
        .await
    });
    let id = tokio::time::timeout(Duration::from_secs(3), receive)
        .await
        .unwrap()
        .unwrap();
    let client = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    (id, client)
}
fn request(tool: &str, arguments: serde_json::Value) -> Request {
    serde_json::from_value(serde_json::json!({"tool": tool, "arguments": arguments})).unwrap()
}
fn denied(reply: Reply, code: ErrorCode) {
    assert!(matches!(reply, Reply::Error { code: actual, .. } if actual == code));
}

#[tokio::test]
async fn session_request_admission_precedes_yolo_guards_handlers_receipts_and_async_work() {
    let root = tempfile::tempdir().unwrap();
    let available = Arc::new(AtomicBool::new(true));
    let policy: Arc<dyn SessionRequestPolicy> = Arc::new(ReadOnlyProject {
        available: available.clone(),
        workspaces: ["project-a-1".into(), "project-a-2".into()].into(),
    });
    let enrollments = Arc::new(AtomicUsize::new(0));
    let observed = enrollments.clone();
    let admission_available = available.clone();
    let guard_calls = Arc::new(AtomicUsize::new(0));
    let guarded = guard_calls.clone();
    let broker = Broker::start_with_admission(
        &root.path().join("control"),
        Some(Arc::new(move || {
            guarded.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(()))
        })),
        Some(Arc::new(move |pid| {
            assert_eq!(pid, Some(std::process::id()));
            observed.fetch_add(1, Ordering::SeqCst);
            if !admission_available.load(Ordering::SeqCst) {
                return Err(ErrorCode::ControlRevoked);
            }
            Ok(Some(policy.clone()))
        })),
    )
    .unwrap();
    publish(&broker, root.path());
    broker.set_yolo_mode(true).unwrap();
    let dispatches = Arc::new(AtomicUsize::new(0));
    let dispatched = dispatches.clone();
    broker
        .set_ui_dispatch(Arc::new(move |_| {
            dispatched.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }))
        .unwrap();
    let (id, client) = pair(&broker, "ordinary trusted application label").await;
    assert!(matches!(
        client.call(Request::Status(EmptyInput {})).await.unwrap(),
        Reply::Ok { .. }
    ));
    assert!(matches!(
        client
            .call(request(
                "lomi_files_list",
                serde_json::json!({"workspaceId":"project-a-2"})
            ))
            .await
            .unwrap(),
        Reply::Ok { .. }
    ));
    denied(
        client
            .call(request(
                "lomi_files_list",
                serde_json::json!({"workspaceId":"project-b"}),
            ))
            .await
            .unwrap(),
        ErrorCode::ScopeDenied,
    );
    assert!(matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: "project-a-1".into()
            }))
            .await
            .unwrap(),
        Reply::Ok { .. }
    ));
    let denied_requests = vec![
        request(
            "lomi_terminal_run",
            serde_json::json!({"workspaceId":"project-a-1","panelId":"panel","terminalSessionId":"terminal","leaseId":"lease","command":"echo forbidden","retryEpoch":"epoch","requestKey":"request"}),
        ),
        request(
            "lomi_chat_send",
            serde_json::json!({"workspaceId":"project-a-1","panelId":"panel","conversationId":"conversation","connectionId":"connection","model":"model","expectedDraftRevision":"1","expectedConversationRevision":"1","expectedRevision":"1","retryEpoch":"epoch","requestKey":"request"}),
        ),
        request(
            "lomi_git_mutate",
            serde_json::json!({"workspaceId":"project-a-1","operation":"commit","paths":[],"message":"forbidden","expectedRevision":"1","retryEpoch":"epoch","requestKey":"request"}),
        ),
        request(
            "lomi_files_mutate",
            serde_json::json!({"workspaceId":"project-a-1","operation":{"type":"create","relativePath":"forbidden.txt","kind":"file","expectedParentRevision":"1"},"expectedRevision":"1","retryEpoch":"epoch","requestKey":"request"}),
        ),
        Request::Diagnostics(EmptyInput {}),
        Request::InputTerminal(TerminalInput {
            workspace_id: "project-a-1".into(),
            panel_id: "panel".into(),
            terminal_session_id: "terminal".into(),
            lease_id: "lease".into(),
            input_sequence: "1".into(),
            input: TerminalPayload::Text {
                text: "forbidden\n".into(),
            },
        }),
    ];
    for request in denied_requests {
        denied(
            broker.call_async(&id, request.clone()).await,
            ErrorCode::ScopeDenied,
        );
        denied(broker.call(&id, request), ErrorCode::ScopeDenied);
    }
    assert_eq!(guard_calls.load(Ordering::SeqCst), 0);
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    {
        let state = broker.lock_state().unwrap();
        assert!(state.work.is_empty() && state.runs.is_empty() && state.claims.is_empty());
    }
    let db = rusqlite::Connection::open(root.path().join("control/control.sqlite3")).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM receipts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(enrollments.load(Ordering::SeqCst), 1);
    available.store(false, Ordering::SeqCst);
    denied(
        client.call(Request::Status(EmptyInput {})).await.unwrap(),
        ErrorCode::ControlRevoked,
    );
    assert_eq!(
        enrollments.load(Ordering::SeqCst),
        1,
        "policy was rediscovered instead of retained"
    );
    assert!(tokio::time::timeout(
        Duration::from_secs(3),
        Client::connect(
            &broker.endpoint,
            "new label cannot remove failed provenance",
            |_| panic!("failed provenance reached pairing")
        )
    )
    .await
    .unwrap()
    .is_err());
    drop(client);
    broker.shutdown().await;
}

#[tokio::test]
async fn session_request_admission_rejects_enrollment_before_manual_or_yolo_pairing() {
    for yolo in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let broker = Broker::start_with_admission(
            &root.path().join("control"),
            None,
            Some(Arc::new(|pid| {
                assert_eq!(pid, Some(std::process::id()));
                Err(ErrorCode::ScopeDenied)
            })),
        )
        .unwrap();
        publish(&broker, root.path());
        broker.set_yolo_mode(yolo).unwrap();
        let connection = tokio::time::timeout(
            Duration::from_secs(3),
            Client::connect(&broker.endpoint, "trusted strict-host label", |_| {
                panic!("denied enrollment reached pairing")
            }),
        )
        .await
        .unwrap();
        assert!(connection.is_err());
        let overview = broker.overview().unwrap();
        assert!(overview.pending.is_empty() && overview.sessions.is_empty());
        broker.shutdown().await;
    }
}

#[tokio::test]
async fn session_request_admission_none_preserves_ordinary_grants_and_labels_do_not_select_policy()
{
    for configured in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let admission: Option<SessionRequestAdmission> =
            configured.then(|| Arc::new(|_: Option<u32>| Ok(None)) as SessionRequestAdmission);
        let broker = if configured {
            Broker::start_with_admission(&root.path().join("control"), None, admission).unwrap()
        } else {
            Broker::start(&root.path().join("control")).unwrap()
        };
        publish(&broker, root.path());
        broker.set_yolo_mode(true).unwrap();
        let (_, client) = pair(&broker, "strict-host label without server restriction").await;
        assert!(matches!(
            client
                .call(Request::Diagnostics(EmptyInput {}))
                .await
                .unwrap(),
            Reply::Ok { .. }
        ));
        assert!(matches!(
            client
                .call(Request::Connect(ConnectInput {
                    workspace_id: "project-b".into()
                }))
                .await
                .unwrap(),
            Reply::Ok { .. }
        ));
        drop(client);
        broker.shutdown().await;
    }
}

#[test]
fn restricted_stub_allows_only_exact_project_filesystem_observations() {
    let policy = ReadOnlyProject {
        available: Arc::new(AtomicBool::new(true)),
        workspaces: ["project-a-1".into(), "project-a-2".into()].into(),
    };
    for workspace in ["project-a-1", "project-a-2", "project-b"] {
        for request in [
            request(
                "lomi_files_list",
                serde_json::json!({"workspaceId":workspace}),
            ),
            request(
                "lomi_files_read",
                serde_json::json!({"workspaceId":workspace,"relativePath":"file.txt"}),
            ),
            request(
                "lomi_files_search",
                serde_json::json!({"workspaceId":workspace,"query":{"text":"needle","caseSensitive":false,"wholeWord":false,"include":"","exclude":""}}),
            ),
        ] {
            assert_eq!(
                policy.check(&request),
                if workspace == "project-b" {
                    Err(ErrorCode::ScopeDenied)
                } else {
                    Ok(())
                }
            );
        }
    }
}

struct RevocableEffects {
    available: Arc<AtomicBool>,
    retained: Arc<AtomicUsize>,
    broker: Mutex<Option<std::sync::Weak<Broker>>>,
}
struct EffectGuard(Arc<AtomicUsize>);
impl Drop for EffectGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl SessionRequestPolicy for RevocableEffects {
    fn check(&self, _: &Request) -> Result<(), ErrorCode> {
        if let Some(broker) = self
            .broker
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|b| b.upgrade())
        {
            assert!(
                broker.state.try_lock().is_ok(),
                "host callback held broker state"
            );
            assert!(
                broker.store.try_lock().is_ok(),
                "host callback held receipt store"
            );
        }
        if self.available.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(ErrorCode::ControlRevoked)
        }
    }
    fn admit_effect(&self, _: &Request) -> Result<Option<Arc<dyn SessionEffectPermit>>, ErrorCode> {
        self.retained.fetch_add(1, Ordering::SeqCst);
        Ok(Some(Arc::new(EffectGuard(self.retained.clone()))))
    }
}

#[tokio::test]
async fn deferred_ui_commit_revalidates_original_policy_and_retains_effect_admission() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    let available = Arc::new(AtomicBool::new(true));
    let retained = Arc::new(AtomicUsize::new(0));
    let policy = Arc::new(RevocableEffects {
        available: available.clone(),
        retained: retained.clone(),
        broker: Mutex::new(None),
    });
    let enroll = policy.clone();
    let broker = Broker::start_with_admission(
        &root.path().join("control"),
        None,
        Some(Arc::new(move |_| Ok(Some(enroll.clone())))),
    )
    .unwrap();
    *policy.broker.lock().unwrap() = Some(Arc::downgrade(&broker));
    publish(&broker, &project);
    broker.set_yolo_mode(true).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(io::Error::other)
        }))
        .unwrap();
    let (id, client) = pair(&broker, "changing claims do not replace captured policy").await;
    let Reply::Ok {
        data: Data::Connected { retry_epoch, .. },
        ..
    } = client
        .call(Request::Connect(ConnectInput {
            workspace_id: "project-a-1".into(),
        }))
        .await
        .unwrap()
    else {
        panic!()
    };
    let directory =
        crate::project_files::ProjectDirectory::open(&project.canonicalize().unwrap()).unwrap();
    let input = FilesMutateInput {
        workspace_id: "project-a-1".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "deferred-create".into(),
        operation: FileMutation::Create {
            relative_path: "forbidden.txt".into(),
            kind: FileEntryKind::File,
            expected_parent_revision: directory.list("", || Ok(())).unwrap().revision,
        },
    };
    assert!(matches!(
        client
            .call(Request::FilesMutate(input.clone()))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::Operation { .. },
            ..
        }
    ));
    let command = commands.recv().await.unwrap();
    assert_eq!(
        retained.load(Ordering::SeqCst),
        1,
        "effect guard dropped with initial reply"
    );
    broker
        .claim_ui(&command.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    available.store(false, Ordering::SeqCst);
    assert_eq!(
        broker
            .commit_files_mutate(&command.operation_id, &command.nonce)
            .unwrap_err(),
        ErrorCode::ControlRevoked
    );
    assert!(!project.join("forbidden.txt").exists());
    assert!(broker
        .claim_ui(&command.ui_epoch, &command.operation_id, &command.nonce)
        .is_err());
    let db = rusqlite::Connection::open(root.path().join("control/control.sqlite3")).unwrap();
    let succeeded: i64 = db
        .query_row(
            "SELECT count(*) FROM receipts WHERE state = 'succeeded'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(succeeded, 0);
    // The ordinary grant remains required even when a host policy admits work.
    available.store(true, Ordering::SeqCst);
    broker
        .lock_state()
        .unwrap()
        .sessions
        .get_mut(&id)
        .unwrap()
        .grant
        .scopes
        .remove("files.create");
    assert!(matches!(
        broker.commit_files_mutate(&command.operation_id, &command.nonce),
        Err(ErrorCode::ScopeDenied)
    ));
    assert!(!project.join("forbidden.txt").exists());
    let permit = broker.lock_state().unwrap().work[&command.operation_id]
        .native_permit
        .clone();
    let renewed = permit.renew(Instant::now() + Duration::from_secs(10));
    available.store(false, Ordering::SeqCst);
    assert_eq!(renewed.check(), Err(ErrorCode::ControlRevoked));
    drop(permit);
    drop(renewed);
    broker
        .lock_state()
        .unwrap()
        .work
        .remove(&command.operation_id);
    assert_eq!(retained.load(Ordering::SeqCst), 0);
    available.store(true, Ordering::SeqCst);
    broker
        .lock_state()
        .unwrap()
        .sessions
        .get_mut(&id)
        .unwrap()
        .grant
        .scopes
        .insert("files.create".into());
    let mut committed_input = input;
    committed_input.request_key = "commit-before-cancel".into();
    assert!(matches!(
        client
            .call(Request::FilesMutate(committed_input))
            .await
            .unwrap(),
        Reply::Ok { .. }
    ));
    let committed = commands.recv().await.unwrap();
    broker
        .claim_ui(
            &committed.ui_epoch,
            &committed.operation_id,
            &committed.nonce,
        )
        .unwrap();
    let result = broker
        .commit_files_mutate(&committed.operation_id, &committed.nonce)
        .unwrap();
    assert!(project.join("forbidden.txt").exists());
    assert!(matches!(
        broker.cancel_operation(
            &id,
            OperationInput {
                operation_id: committed.operation_id.clone()
            }
        ),
        Reply::Ok { .. }
    ));
    assert!(broker.lock_state().unwrap().work[&committed.operation_id]
        .native_permit
        .check_local()
        .is_err());
    available.store(false, Ordering::SeqCst);
    // Cancellation and host revocation cannot erase an already-observed effect.
    broker
        .acknowledge_ui(UiAck {
            operation_id: committed.operation_id.clone(),
            nonce: committed.nonce,
            ui_epoch: committed.ui_epoch,
            result: OperationResult::FilesMutated(result),
        })
        .unwrap();
    let succeeded: i64 = db
        .query_row(
            "SELECT count(*) FROM receipts WHERE state = 'succeeded'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(succeeded, 1);
    assert_eq!(retained.load(Ordering::SeqCst), 0);
    available.store(true, Ordering::SeqCst);
    let close_input = request(
        "lomi_workspace_update",
        serde_json::json!({
            "workspaceId":"project-a-1", "name":"Renamed", "expectedRevision":"1",
            "retryEpoch":broker.lock_state().unwrap().sessions[&id].retry_epoch, "requestKey":"blocked-close-step"
        }),
    );
    assert!(matches!(
        client.call(close_input).await.unwrap(),
        Reply::Ok { .. }
    ));
    let close = commands.recv().await.unwrap();
    broker
        .claim_ui(&close.ui_epoch, &close.operation_id, &close.nonce)
        .unwrap();
    broker
        .lock_state()
        .unwrap()
        .work
        .get_mut(&close.operation_id)
        .unwrap()
        .native_committed = true;
    let (started, started_rx) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let closing = broker.clone();
    let operation = close.operation_id.clone();
    let in_flight = std::thread::spawn(move || {
        closing.finish_close_resources(
            &operation,
            vec![Box::new(move || {
                started.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })],
        )
    });
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    broker.revoke();
    broker
        .state
        .lock()
        .unwrap()
        .work
        .remove(&close.operation_id);
    assert_eq!(
        retained.load(Ordering::SeqCst),
        1,
        "revocation released admission during native close"
    );
    release.send(()).unwrap();
    assert!(in_flight.join().unwrap().is_err());
    assert_eq!(retained.load(Ordering::SeqCst), 0);
    drop(client);
    broker.shutdown().await;
}

#[test]
fn synchronous_request_scopes_restore_nested_and_reused_thread_contexts() {
    assert!(current_request().is_none());
    let outer_available = Arc::new(AtomicBool::new(true));
    let outer = CapturedRequest {
        policy: Some(Arc::new(ReadOnlyProject {
            available: outer_available.clone(),
            workspaces: HashSet::new(),
        })),
        request: Request::Status(EmptyInput {}),
        connected: Arc::new(AtomicBool::new(true)),
        _effect: None,
    };
    let denied = CapturedRequest {
        connected: Arc::new(AtomicBool::new(false)),
        ..outer.clone()
    };
    {
        let _outer = RequestScope::enter(Some(outer));
        assert!(current_request().unwrap().check().is_ok());
        {
            let _nested = RequestScope::enter(Some(denied));
            assert_eq!(
                current_request().unwrap().check(),
                Err(ErrorCode::ControlRevoked)
            );
        }
        assert!(current_request().unwrap().check().is_ok());
        outer_available.store(false, Ordering::SeqCst);
        assert_eq!(
            current_request().unwrap().check(),
            Err(ErrorCode::ControlRevoked)
        );
        assert!(std::thread::spawn(|| current_request().is_none())
            .join()
            .unwrap());
    }
    assert!(current_request().is_none());
    let _fresh = RequestScope::enter(None);
    assert!(current_request().is_none());
}
