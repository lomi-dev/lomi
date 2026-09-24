#![cfg(unix)]

use lomi_control_core::{broker::Broker, client::Client};
use lomi_control_protocol::{control::*, EmptyInput, ErrorCode};
use std::{
    io,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{sleep, timeout},
};

fn projection(broker: &Broker, root: &Path) -> Projection {
    Projection {
        ui_epoch: broker.register_ui().unwrap(),
        revision: "1".into(),
        workspaces: [("a", "p"), ("b", "p"), ("foreign", "other")]
            .into_iter()
            .map(|(id, project_id)| Workspace {
                id: id.into(),
                project_id: project_id.into(),
                name: id.into(),
                project_name: project_id.into(),
                active_panel_id: None,
                project_path: root.canonicalize().unwrap().to_string_lossy().into(),
            })
            .collect(),
        ..Projection::default()
    }
}

fn start_client(
    endpoint: lomi_control_core::broker::Endpoint,
) -> (JoinHandle<io::Result<Client>>, oneshot::Receiver<String>) {
    let (send, received) = oneshot::channel();
    let task = tokio::spawn(async move {
        Client::connect(&endpoint, "YOLO integration test", move |id| {
            let _ = send.send(id);
        })
        .await
    });
    (task, received)
}

async fn pairing_id(received: oneshot::Receiver<String>) -> String {
    timeout(Duration::from_secs(2), received)
        .await
        .expect("client should receive its pairing request id")
        .expect("pairing id callback should run")
}

async fn wait_for_pending(broker: &Broker, id: &str) {
    timeout(Duration::from_secs(2), async {
        loop {
            if broker
                .overview()
                .unwrap()
                .pending
                .iter()
                .any(|pending| pending.id == id)
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("pairing request should appear in the broker overview");
}

async fn auto_pair(broker: &Broker) -> (String, Client) {
    let (task, received) = start_client(broker.endpoint.clone());
    let id = pairing_id(received).await;
    let client = timeout(Duration::from_secs(3), task)
        .await
        .expect("YOLO should finish pairing a ready client")
        .expect("client task should not panic")
        .expect("YOLO client should authenticate");
    (id, client)
}

#[tokio::test]
async fn yolo_defaults_off_and_leaves_pairing_for_manual_approval() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let p = projection(&broker, root.path());
    broker.publish(p).unwrap();

    assert!(!broker.yolo_mode());
    let (task, received) = start_client(broker.endpoint.clone());
    let id = pairing_id(received).await;
    wait_for_pending(&broker, &id).await;
    assert!(
        !task.is_finished(),
        "default-off pairing must remain pending"
    );
    assert!(broker
        .overview()
        .unwrap()
        .sessions
        .iter()
        .all(|session| session.id != id));

    broker
        .approve_terminal_profile(
            &id,
            &["a".into()],
            &["workspace.read".into()],
            &[],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();
    let client = task.await.unwrap().unwrap();
    assert!(matches!(
        client.call(Request::Status(EmptyInput {})).await.unwrap(),
        Reply::Ok {
            data: Data::Status { .. },
            ..
        }
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn yolo_pairs_waiting_clients_and_grants_current_and_future_projection_workspaces() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());

    // Enabling before the first published projection must leave enrollment
    // waiting until the UI becomes ready.
    broker.set_yolo_mode(true).unwrap();
    assert!(broker.yolo_mode());
    let (task, received) = start_client(broker.endpoint.clone());
    let id = pairing_id(received).await;
    wait_for_pending(&broker, &id).await;
    assert!(
        !task.is_finished(),
        "an unpublished projection is not ready"
    );

    broker.publish(p.clone()).unwrap();
    let client = timeout(Duration::from_secs(3), task)
        .await
        .expect("publishing the ready projection should auto-pair the client")
        .unwrap()
        .unwrap();

    let Reply::Ok {
        data: Data::Status { capabilities, .. },
        ..
    } = client.call(Request::Status(EmptyInput {})).await.unwrap()
    else {
        panic!("YOLO session should report its capabilities")
    };
    assert!(
        capabilities.len() >= 40,
        "expected the full known capability set"
    );
    assert!(capabilities.iter().all(|capability| capability.authorized));
    for expected in [
        "workspace.read",
        "workspace.write",
        "project.open",
        "terminal.execute",
        "files.mutate",
        "editor.write",
        "git.write",
        "settings.write",
        "browser.interact",
        "android.control",
        "chat.send",
    ] {
        assert!(
            capabilities
                .iter()
                .any(|capability| capability.name == expected && capability.authorized),
            "YOLO should grant {expected}"
        );
    }

    for workspace_id in ["a", "b", "foreign"] {
        assert!(matches!(
            client
                .call(Request::Connect(ConnectInput {
                    workspace_id: workspace_id.into(),
                }))
                .await
                .unwrap(),
            Reply::Ok {
                data: Data::Connected { .. },
                ..
            }
        ));
    }

    p.revision = "2".into();
    p.workspaces.push(Workspace {
        id: "future".into(),
        project_id: "future-project".into(),
        name: "future".into(),
        project_name: "Future project".into(),
        active_panel_id: None,
        project_path: root.path().canonicalize().unwrap().to_string_lossy().into(),
    });
    broker.publish(p).unwrap();
    assert!(matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: "future".into(),
            }))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::Connected { .. },
            ..
        }
    ));
    let session = broker
        .overview()
        .unwrap()
        .sessions
        .into_iter()
        .find(|session| session.id == id)
        .expect("auto-paired client should stay in the broker session list");
    for workspace_id in ["a", "b", "foreign", "future"] {
        assert!(
            session.workspace_ids.iter().any(|id| id == workspace_id),
            "YOLO session should include workspace {workspace_id}"
        );
    }
    broker.shutdown().await;
}

#[tokio::test]
async fn yolo_project_open_autoapproves_through_prepare_validate_and_commit() {
    let root = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(io::Error::other)
        }))
        .unwrap();
    broker.set_yolo_mode(true).unwrap();
    let (_id, client) = auto_pair(&broker).await;

    let Reply::Ok {
        data: Data::Connected { retry_epoch, .. },
        ..
    } = client
        .call(Request::Connect(ConnectInput {
            workspace_id: "a".into(),
        }))
        .await
        .unwrap()
    else {
        panic!("YOLO client should connect to a current workspace")
    };

    let malformed = ProjectOpenInput {
        workspace_id: "a".into(),
        project_path: target.path().to_string_lossy().into(),
        name: "bad\0name".into(),
        expected_revision: p.revision.clone(),
        retry_epoch: retry_epoch.clone(),
        request_key: "malformed-project".into(),
    };
    assert!(matches!(
        client.call(Request::OpenProject(malformed)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ResourceExhausted,
            ..
        }
    ));

    let stale = ProjectOpenInput {
        workspace_id: "a".into(),
        project_path: target.path().to_string_lossy().into(),
        name: "Stale project request".into(),
        expected_revision: "0".into(),
        retry_epoch: retry_epoch.clone(),
        request_key: "stale-project".into(),
    };
    assert!(matches!(
        client.call(Request::OpenProject(stale)).await.unwrap(),
        Reply::Ok {
            data: Data::Operation {
                state,
                ..
            },
            ..
        } if state == "failed"
    ));
    assert!(
        commands.try_recv().is_err(),
        "invalid requests must not reach the UI"
    );

    let input = ProjectOpenInput {
        workspace_id: "a".into(),
        project_path: target.path().to_string_lossy().into(),
        name: "YOLO opened project".into(),
        expected_revision: p.revision.clone(),
        retry_epoch,
        request_key: "valid-project-open".into(),
    };
    assert!(matches!(
        client.call(Request::OpenProject(input)).await.unwrap(),
        Reply::Ok {
            data: Data::Operation { state, .. },
            ..
        } if state == "queued"
    ));
    let command = timeout(Duration::from_secs(2), commands.recv())
        .await
        .unwrap()
        .expect("valid project-open should be dispatched to the UI");
    let UiAction::OpenProject(open) = &command.action else {
        panic!("expected an open-project UI command")
    };

    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    assert!(broker
        .project_open_ready(&command.operation_id, &command.nonce, &p.ui_epoch)
        .unwrap());
    broker
        .commit_project_open(&command.operation_id, &command.nonce, &p.ui_epoch)
        .unwrap();

    p.revision = "2".into();
    p.workspaces.push(Workspace {
        id: open.new_workspace_id.clone(),
        project_id: open.project_id.clone(),
        name: open.name.clone(),
        project_name: open.name.clone(),
        active_panel_id: Some(open.tab_id.clone()),
        project_path: open.project_path.clone(),
    });
    p.panels.push(Panel {
        id: open.tab_id.clone(),
        tab_id: open.tab_id.clone(),
        workspace_id: open.new_workspace_id.clone(),
        kind: "file".into(),
        title: "Project".into(),
        terminal_session_id: None,
        browser_generation: None,
        android_device_id: None,
        chat_conversation_id: None,
    });
    broker.publish(p.clone()).unwrap();
    broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::ProjectOpened(Box::new(ProjectOpened {
                anchor_workspace_id: open.workspace_id.clone(),
                project_id: open.project_id.clone(),
                project_path: open.project_path.clone(),
                workspace_id: open.new_workspace_id.clone(),
                panel_id: open.tab_id.clone(),
                name: open.name.clone(),
                opened: Some(true),
            })),
        })
        .unwrap();

    assert!(matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: open.new_workspace_id.clone(),
            }))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::Connected { .. },
            ..
        }
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn yolo_enumerates_unopened_android_and_chat_resources_but_rejects_invalid_ids() {
    const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let p = projection(&broker, root.path());
    broker.publish(p).unwrap();
    broker.set_yolo_mode(true).unwrap();
    let (_id, client) = auto_pair(&broker).await;
    let Reply::Ok {
        data: Data::Connected { retry_epoch, .. },
        ..
    } = client
        .call(Request::Connect(ConnectInput {
            workspace_id: "a".into(),
        }))
        .await
        .unwrap()
    else {
        panic!("YOLO client should connect to a current workspace")
    };

    let android_calls = Arc::new(AtomicUsize::new(0));
    let android_call = android_calls.clone();
    broker
        .set_android_list_dispatch(Arc::new(move |request| {
            request.check()?;
            assert!(
                request.all_devices,
                "YOLO should enumerate all host devices"
            );
            let call = android_call.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                assert!(request.devices.is_empty());
            } else {
                assert_eq!(request.devices, vec![DEVICE.to_string()]);
            }
            let device_id = if call == 0 {
                DEVICE
            } else {
                "invalid-device-id"
            };
            Ok(AndroidDevices {
                devices_revision: (call + 1).to_string(),
                host_qualified: true,
                items: vec![AndroidDevice {
                    device_id: device_id.into(),
                    name: "Fixture device".into(),
                    generation: None,
                    phase: AndroidPhase::Stopped,
                    process_alive: false,
                    display: None,
                }],
            })
        }))
        .unwrap();

    let android_list = AndroidListInput {
        workspace_id: "a".into(),
    };
    assert!(matches!(
        client
            .call(Request::AndroidList(android_list.clone()))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::AndroidDevices { devices, .. },
            ..
        } if devices.items.len() == 1 && devices.items[0].device_id == DEVICE
    ));
    let session = broker
        .overview()
        .unwrap()
        .sessions
        .into_iter()
        .find(|session| session.client_label == "YOLO integration test")
        .expect("YOLO session should remain connected");
    assert_eq!(session.android_device_ids, vec![DEVICE.to_string()]);
    assert!(matches!(
        client
            .call(Request::AndroidList(android_list))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::OutcomeUnknown,
            ..
        }
    ));
    assert!(matches!(
        client
            .call(Request::AndroidOpen(AndroidOpenInput {
                workspace_id: "a".into(),
                device_id: "invalid-device-id".into(),
                expected_revision: "1".into(),
                retry_epoch: retry_epoch.clone(),
                request_key: "invalid-android-device".into(),
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ResourceExhausted,
            ..
        }
    ));

    let chat_calls = Arc::new(AtomicUsize::new(0));
    let chat_call = chat_calls.clone();
    broker
        .set_chat_list_dispatch(Arc::new(move |project, selection, check| {
            check()?;
            assert_eq!(project, "p");
            assert!(matches!(
                selection,
                lomi_control_core::broker::ChatConversationSelection::All
            ));
            let call = chat_call.fetch_add(1, Ordering::SeqCst);
            let conversation_id = if call == 0 {
                "unopened-chat"
            } else {
                "invalid chat id"
            };
            Ok(vec![ChatSummary {
                conversation_id: conversation_id.into(),
                title: "Fixture conversation".into(),
                conversation_revision: "1".into(),
                latest_message_id: None,
                updated_at_millis: "1234".into(),
            }])
        }))
        .unwrap();

    let chat_list = ChatListInput {
        workspace_id: "a".into(),
        offset: 0,
        limit: 20,
        expected_revision: None,
    };
    assert!(matches!(
        client
            .call(Request::ChatList(chat_list.clone()))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::ChatList(list),
            ..
        } if list.total == 1 && list.items[0].conversation_id == "unopened-chat"
    ));
    assert!(matches!(
        client.call(Request::ChatList(chat_list)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::OutcomeUnknown,
            ..
        }
    ));
    assert_eq!(chat_calls.load(Ordering::SeqCst), 2);
    broker.shutdown().await;
}

#[tokio::test]
async fn disabling_yolo_revokes_live_authority_and_restores_manual_pairing() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let p = projection(&broker, root.path());
    broker.publish(p).unwrap();

    let (manual_task, manual_received) = start_client(broker.endpoint.clone());
    let manual_id = pairing_id(manual_received).await;
    wait_for_pending(&broker, &manual_id).await;
    broker
        .approve_terminal_profile(
            &manual_id,
            &["a".into()],
            &["workspace.read".into()],
            &[],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();
    let manual_client = manual_task.await.unwrap().unwrap();

    broker.set_yolo_mode(true).unwrap();
    broker.wait_for_cleanup().await;
    assert!(!matches!(
        manual_client
            .call(Request::Connect(ConnectInput {
                workspace_id: "a".into(),
            }))
            .await,
        Ok(Reply::Ok { .. })
    ));
    assert!(broker
        .overview()
        .unwrap()
        .sessions
        .iter()
        .all(|session| session.id != manual_id));

    let (id, client) = auto_pair(&broker).await;
    assert!(matches!(
        client.call(Request::Status(EmptyInput {})).await.unwrap(),
        Reply::Ok {
            data: Data::Status { .. },
            ..
        }
    ));

    broker.set_yolo_mode(false).unwrap();
    assert!(!broker.yolo_mode());
    broker.wait_for_cleanup().await;
    assert!(!matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: "a".into(),
            }))
            .await,
        Ok(Reply::Ok { .. })
    ));
    assert!(broker
        .overview()
        .unwrap()
        .sessions
        .iter()
        .all(|session| session.id != id));

    let (task, received) = start_client(broker.endpoint.clone());
    let new_id = pairing_id(received).await;
    wait_for_pending(&broker, &new_id).await;
    assert!(
        !task.is_finished(),
        "disabled YOLO must require manual pairing"
    );
    broker
        .approve_terminal_profile(
            &new_id,
            &["a".into()],
            &["workspace.read".into()],
            &[],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();
    let restricted = task.await.unwrap().unwrap();
    let Reply::Ok {
        data: Data::Status { capabilities, .. },
        ..
    } = restricted
        .call(Request::Status(EmptyInput {}))
        .await
        .unwrap()
    else {
        panic!("manually paired client should be connected")
    };
    assert!(capabilities
        .iter()
        .any(|capability| capability.name == "workspace.read" && capability.authorized));
    assert!(capabilities
        .iter()
        .any(|capability| capability.name == "terminal.execute" && !capability.authorized));
    broker.shutdown().await;
}
