use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
const GENERATION: &str = "00000000-0000-4000-8000-000000000002";

#[tokio::test]
async fn slow_android_close_keeps_broker_responsive_and_rejects_early_success() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels = vec![phone("phone", "a")];
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let entered = Mutex::new(Some(entered));
    let (release, wait) = std::sync::mpsc::channel();
    let wait = Mutex::new(wait);
    let commits = Arc::new(AtomicUsize::new(0));
    let count = commits.clone();
    broker
        .set_android_close_dispatch(Arc::new(move |_, permit| {
            if let Some(permit) = permit {
                count.fetch_add(1, Ordering::SeqCst);
                entered.lock().unwrap().take().unwrap().send(()).unwrap();
                wait.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                permit.check().map_err(|_| ErrorCode::OutcomeUnknown)?;
            }
            Ok(())
        }))
        .unwrap();
    let client = approved_domains(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "panel.close",
            "panel.create",
            "android.read",
            "android.control",
        ],
        &[DEVICE],
    )
    .await;
    let input = close(&p, &p.panels[0], &epoch(&client).await, "slow-close");
    client
        .call(Request::ClosePanel(input.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let (b, c) = (broker.clone(), command.clone());
    let running = tokio::task::spawn_blocking(move || {
        b.commit_panel_close(&c.operation_id, &c.nonce, &c.ui_epoch)
    });
    ready.await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(1),
        client.call(Request::Operation(
            OperationInput {
                operation_id: command.operation_id.clone(),
            }
            .into(),
        )),
    )
    .await
    .expect("native Stop blocked operation reads")
    .unwrap();
    p.panels.clear();
    p.revision = "2".into();
    broker.publish(p.clone()).unwrap();
    assert!(broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::Panel {
                workspace_id: "a".into(),
                panel_id: "phone".into(),
                focused: false,
                closed: true
            }
        })
        .is_err());
    client
        .call(Request::CancelOperation(OperationInput {
            operation_id: command.operation_id.clone(),
        }))
        .await
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(running.await.unwrap(), Err(ErrorCode::OutcomeUnknown));
    broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce,
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::Failure {
                code: ErrorCode::OutcomeUnknown,
            },
        })
        .unwrap();
    let replay = client.call(Request::ClosePanel(input)).await.unwrap();
    assert!(
        matches!(replay, Reply::Ok { data: Data::Operation { state, effect_state, .. }, .. } if state == "outcome_unknown" && effect_state == "unknown")
    );
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert!(commands.try_recv().is_err());
    broker.shutdown().await;
}
fn phone(id: &str, workspace: &str) -> Panel {
    Panel {
        id: id.into(),
        tab_id: id.into(),
        workspace_id: workspace.into(),
        kind: "android".into(),
        title: "Phone".into(),
        android_device_id: Some(DEVICE.into()),
        chat_conversation_id: None,
        terminal_session_id: None,
        browser_generation: None,
    }
}
async fn epoch(client: &Client) -> String {
    match client
        .call(Request::Connect(ConnectInput {
            workspace_id: "a".into(),
        }))
        .await
        .unwrap()
    {
        Reply::Ok {
            data: Data::Connected { retry_epoch, .. },
            ..
        } => retry_epoch,
        reply => panic!("{reply:?}"),
    }
}
fn close(p: &Projection, panel: &Panel, retry: &str, key: &str) -> PanelMutationInput {
    PanelMutationInput {
        workspace_id: panel.workspace_id.clone(),
        panel_id: panel.id.clone(),
        terminal_session_id: None,
        browser_generation: None,
        expected_revision: p.revision.clone(),
        retry_epoch: retry.into(),
        request_key: key.into(),
    }
}

#[tokio::test]
async fn android_last_view_close_binds_scope_native_barrier_and_retry() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels = vec![phone("phone-a", "a"), phone("phone-b", "b")];
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let commits = Arc::new(AtomicUsize::new(0));
    let retained = Arc::new(Mutex::new(None));
    let (count, barrier) = (commits.clone(), retained.clone());
    broker
        .set_android_close_dispatch(Arc::new(move |targets, permit| {
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].device, DEVICE);
            if let Some(permit) = permit {
                permit.check()?;
                if targets[0].last_view {
                    count.fetch_add(1, Ordering::SeqCst);
                }
                *barrier.lock().unwrap() = Some(permit);
            }
            Ok(())
        }))
        .unwrap();
    let scopes = [
        "workspace.read",
        "workspace.write",
        "workspace.close",
        "project.close",
        "panel.close",
        "panel.create",
        "android.read",
    ];
    let viewer = approved_domains(&broker, &["a", "b"], &scopes, &[DEVICE]).await;
    let mut full = scopes.to_vec();
    full.push("android.control");
    let owner = approved_domains(&broker, &["a", "b"], &full, &[DEVICE]).await;
    let viewer_epoch = epoch(&viewer).await;
    let owner_epoch = epoch(&owner).await;
    let denied = viewer
        .call(Request::CloseProject(ProjectCloseInput {
            project_id: "p".into(),
            workspace_id: "a".into(),
            expected_revision: "1".into(),
            retry_epoch: viewer_epoch.clone(),
            request_key: "close-project-no-stop".into(),
        }))
        .await
        .unwrap();
    assert!(
        matches!(
            denied,
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ),
        "{denied:?}"
    );
    for index in 0..2 {
        let panel = p.panels[0].clone();
        let input = close(
            &p,
            &panel,
            if index == 0 {
                &viewer_epoch
            } else {
                &owner_epoch
            },
            &format!("close-{index}"),
        );
        if index == 1 {
            let mut denied = input.clone();
            denied.retry_epoch.clone_from(&viewer_epoch);
            assert!(matches!(
                viewer.call(Request::ClosePanel(denied)).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::ScopeDenied,
                    ..
                }
            ));
        }
        let client = if index == 0 { &viewer } else { &owner };
        let reply = client
            .call(Request::ClosePanel(input.clone()))
            .await
            .unwrap();
        assert!(matches!(reply, Reply::Ok { .. }), "{reply:?}");
        let command = commands.recv().await.unwrap();
        broker
            .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
            .unwrap();
        broker
            .commit_panel_close(&command.operation_id, &command.nonce, &p.ui_epoch)
            .unwrap();
        assert_eq!(commits.load(Ordering::SeqCst), index);
        retained.lock().unwrap().as_ref().unwrap().check().unwrap();
        p.panels.remove(0);
        p.revision = (index + 2).to_string();
        broker.publish(p.clone()).unwrap();
        broker
            .acknowledge_ui(UiAck {
                operation_id: command.operation_id.clone(),
                nonce: command.nonce,
                ui_epoch: p.ui_epoch.clone(),
                result: OperationResult::Panel {
                    workspace_id: panel.workspace_id,
                    panel_id: panel.id,
                    focused: false,
                    closed: true,
                },
            })
            .unwrap();
        assert!(retained.lock().unwrap().as_ref().unwrap().check().is_err());
        assert!(
            matches!(client.call(Request::ClosePanel(input)).await.unwrap(), Reply::Ok { data: Data::Operation { operation_id, state, .. }, .. } if operation_id == command.operation_id && state == "succeeded")
        );
        assert!(commands.try_recv().is_err());
    }
    broker.shutdown().await;
}

#[tokio::test]
async fn android_transfer_preserves_generation_and_shared_views_but_releases_input() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels = vec![phone("phone-a", "a"), phone("phone-shared", "a")];
    p.focused_panel_id = Some("phone-a".into());
    p.workspaces[0].active_panel_id = p.focused_panel_id.clone();
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let scopes = [
        "workspace.read",
        "workspace.write",
        "panel.move",
        "panel.focus",
        "panel.close",
        "panel.create",
        "android.read",
        "android.control",
    ];
    let client = approved_domains(&broker, &["a", "b"], &scopes, &[DEVICE]).await;
    let retry = epoch(&client).await;
    client
        .call(Request::AndroidStart(AndroidStartInput {
            workspace_id: "a".into(),
            panel_id: "phone-a".into(),
            device_id: DEVICE.into(),
            expected_revision: "1".into(),
            retry_epoch: retry.clone(),
            request_key: "start".into(),
        }))
        .await
        .unwrap();
    let start = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &start.operation_id, &start.nonce)
        .unwrap();
    let runtime = broker
        .authorize_android_runtime(&start.operation_id, &start.nonce)
        .unwrap();
    runtime.control.bind(GENERATION).unwrap();
    broker
        .finish_android_runtime(
            &start.operation_id,
            &start.nonce,
            Ok(AndroidRuntimeResult {
                workspace_id: "a".into(),
                device_id: DEVICE.into(),
                generation: GENERATION.into(),
                ready: true,
                stopped: false,
            }),
        )
        .unwrap();
    let lease = runtime
        .control
        .acquire_input(
            "phone-a",
            GENERATION,
            "00000000-0000-4000-8000-000000000003",
        )
        .unwrap();
    let input = PanelMoveInput {
        workspace_id: "a".into(),
        movement: PanelMove::TransferTab {
            tab_id: "phone-a".into(),
            target_workspace_id: "b".into(),
            before_tab_id: None,
        },
        expected_revision: "1".into(),
        retry_epoch: retry.clone(),
        request_key: "transfer".into(),
    };
    let reply = client
        .call(Request::MovePanel(input.clone()))
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Ok { .. }), "{reply:?}");
    let command = commands.recv().await.unwrap();
    let UiAction::MovePanel(move_command) = &command.action else {
        panic!()
    };
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let mut destination = move_command.destination.clone().unwrap();
    destination.tab_order.push("phone-a".into());
    destination.panels.push(
        move_command
            .panels
            .iter()
            .find(|p| p.panel_id == "phone-a")
            .unwrap()
            .clone(),
    );
    p.panels[0].workspace_id = "b".into();
    p.revision = "2".into();
    p.focused_panel_id = None;
    p.workspaces[0].active_panel_id = None;
    broker.publish(p.clone()).unwrap();
    assert!(lease.check().is_err());
    runtime.control.check_generation(GENERATION).unwrap();
    broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::PanelMoved(Box::new(PanelMoved {
                workspace_id: "a".into(),
                movement: input.movement.clone(),
                panels: move_command
                    .panels
                    .iter()
                    .filter(|p| p.panel_id != "phone-a")
                    .cloned()
                    .collect(),
                destination: Some(destination),
            })),
        })
        .unwrap();
    assert!(
        matches!(client.call(Request::MovePanel(input)).await.unwrap(), Reply::Ok { data: Data::Operation { state, .. }, .. } if state == "succeeded")
    );
    // Both retained views keep the same device authority after the admitted transfer.
    for panel in &p.panels {
        let reply = client
            .call(Request::FocusPanel(close(
                &p,
                panel,
                &retry,
                &format!("focus-{}", panel.id),
            )))
            .await
            .unwrap();
        assert!(matches!(reply, Reply::Ok { .. }), "{reply:?}");
        let c = commands.recv().await.unwrap();
        broker
            .claim_ui(&p.ui_epoch, &c.operation_id, &c.nonce)
            .unwrap();
        broker
            .acknowledge_ui(UiAck {
                operation_id: c.operation_id,
                nonce: c.nonce,
                ui_epoch: p.ui_epoch.clone(),
                result: OperationResult::Failure {
                    code: ErrorCode::ControlRevoked,
                },
            })
            .unwrap();
    }
    // Human takeover invalidates previously queued layout authority without stopping Android.
    client
        .call(Request::FocusPanel(close(
            &p,
            &p.panels[0],
            &retry,
            "late-focus",
        )))
        .await
        .unwrap();
    let c = commands.recv().await.unwrap();
    runtime.control.revoke();
    assert!(broker
        .claim_ui(&p.ui_epoch, &c.operation_id, &c.nonce)
        .is_err());
    broker.shutdown().await;
}
