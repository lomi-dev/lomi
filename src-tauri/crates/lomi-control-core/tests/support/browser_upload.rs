use super::*;
use lomi_control_core::broker::{BrowserDownloadFailure, BrowserStart};
fn operation(reply: Reply) -> String {
    match reply {
        Reply::Ok {
            data: Data::Operation { operation_id, .. },
            ..
        } => operation_id,
        other => panic!("{other:?}"),
    }
}
#[tokio::test]
async fn uploads_require_one_use_approval_and_recheck_sources_targets_and_receipts() {
    for allowed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let broker = Broker::start(&path.join("control")).unwrap();
        let mut p = projection(&broker, &path);
        broker.publish(p.clone()).unwrap();
        let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
        broker
            .set_ui_dispatch(Arc::new(move |c| {
                send.send(c).map_err(std::io::Error::other)
            }))
            .unwrap();
        let endpoint = broker.endpoint.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let connecting = tokio::spawn(async move {
            Client::connect(&endpoint, "download", |id| {
                tx.send(id).unwrap();
            })
            .await
            .unwrap()
        });
        let id = rx.await.unwrap();
        let mut scopes = vec![
            "workspace.read".into(),
            "panel.create".into(),
            "browser.navigate".into(),
            "browser.read".into(),
            "browser.interact".into(),
            "files.read".into(),
            "artifact.import_file".into(),
        ];
        if allowed {
            scopes.push("browser.upload".into());
        }
        broker
            .approve_policy(
                &id,
                &["a".into()],
                &scopes,
                &["http://localhost:3000".into()],
            )
            .unwrap();
        let client = connecting.await.unwrap();
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
            panic!()
        };
        client
            .call(Request::OpenBrowser(BrowserOpenInput {
                workspace_id: "a".into(),
                url: "http://localhost:3000/".into(),
                visible: true,
                expected_revision: "1".into(),
                retry_epoch: retry_epoch.clone(),
                request_key: "open".into(),
            }))
            .await
            .unwrap();
        let command = commands.recv().await.unwrap();
        let UiAction::CreateBrowser {
            panel_id,
            browser_generation,
            profile_id,
            url,
            ..
        } = &command.action
        else {
            panic!()
        };
        broker
            .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
            .unwrap();
        let control = broker
            .authorize_browser_start(BrowserStart {
                visible: true,
                operation: &command.operation_id,
                nonce: &command.nonce,
                panel: panel_id,
                generation: browser_generation,
                profile: profile_id,
                url,
            })
            .unwrap();
        control.mark_started();
        control.document_loaded(url);
        p.revision = "2".into();
        p.panels.push(Panel {
            id: panel_id.clone(),
            tab_id: panel_id.clone(),
            workspace_id: "a".into(),
            kind: "browser".into(),
            title: "Browser".into(),
            browser_generation: Some(browser_generation.clone()),
            terminal_session_id: None,
            android_device_id: None,
            chat_conversation_id: None,
        });
        p.focused_panel_id = Some(panel_id.clone());
        broker.publish(p.clone()).unwrap();
        broker
            .acknowledge_ui(UiAck {
                operation_id: command.operation_id.clone(),
                nonce: command.nonce.clone(),
                ui_epoch: p.ui_epoch.clone(),
                result: OperationResult::Browser(Box::new(BrowserResult {
                    workspace_id: "a".into(),
                    panel_id: panel_id.clone(),
                    browser_generation: browser_generation.clone(),
                    profile_id: profile_id.clone(),
                    navigation_id: control.navigation_id(),
                    lease_id: control.lease().map(str::to_owned),
                    ready: true,
                    engine: "WKWebView".into(),
                    network_isolation: "none".into(),
                })),
            })
            .unwrap();
        use sha2::{Digest, Sha256};
        std::fs::write(path.join("source.bin"), b"immutable").unwrap();
        let hash = format!("{:x}", Sha256::digest(b"immutable"));
        let imported = operation(
            client
                .call(Request::ImportArtifact(ArtifactImportInput {
                    workspace_id: "a".into(),
                    relative_path: "source.bin".into(),
                    kind: ArtifactImportKind::File,
                    expected_byte_length: 9,
                    expected_sha256: hash.clone(),
                    expected_revision: "2".into(),
                    retry_epoch: retry_epoch.clone(),
                    request_key: "import".into(),
                }))
                .await
                .unwrap(),
        );
        let command = commands.recv().await.unwrap();
        broker
            .claim_ui(&p.ui_epoch, &imported, &command.nonce)
            .unwrap();
        broker
            .execute_import(&imported, &command.nonce, |_, _| panic!("APK probe"))
            .unwrap();
        let Reply::Ok {
            data:
                Data::Operation {
                    result: Some(OperationResult::ArtifactImported { artifact_id, .. }),
                    ..
                },
            ..
        } = client
            .call(Request::Operation(
                OperationInput {
                    operation_id: imported,
                }
                .into(),
            ))
            .await
            .unwrap()
        else {
            panic!()
        };
        std::fs::write(path.join("source.bin"), b"changed").unwrap();
        control
            .retain_snapshot("snapshot", &control.navigation_id())
            .unwrap();
        let input = BrowserUploadInput {
            workspace_id: "a".into(),
            panel_id: panel_id.clone(),
            browser_generation: browser_generation.clone(),
            lease_id: control.lease().unwrap().into(),
            navigation_id: control.navigation_id(),
            snapshot_id: "snapshot".into(),
            element_ref: "main-e1".into(),
            frame_id: "main".into(),
            artifact_id,
            expected_sha256: hash,
            file_name: "file.bin".into(),
            expected_revision: "2".into(),
            retry_epoch,
            request_key: "upload".into(),
        };
        if !allowed {
            assert!(matches!(
                client.call(Request::UploadBrowser(input)).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::ScopeDenied,
                    ..
                }
            ));
            broker.shutdown().await;
            continue;
        }
        for case in ["deny", "cancel", "success", "before", "after", "stale"] {
            control
                .retain_snapshot("snapshot", &control.navigation_id())
                .unwrap();
            let mut request = input.clone();
            request.request_key = case.into();
            let id = operation(
                client
                    .call(Request::UploadBrowser(request.clone()))
                    .await
                    .unwrap(),
            );
            let command = commands.recv().await.unwrap();
            assert!(broker
                .prepare_browser_upload(&id, &command.nonce, |_, _, _| panic!(
                    "unclaimed preparation"
                ))
                .is_err());
            broker.claim_ui(&p.ui_epoch, &id, &command.nonce).unwrap();
            assert!(broker
                .decide_browser_upload(&id, true, |_| panic!("unprepared approval"))
                .is_err());
            broker
                .prepare_browser_upload(&id, &command.nonce, |_, _, _| {
                    Ok(BrowserUploadTarget {
                        frame_id: "main".into(),
                        origin: "http://localhost:3000".into(),
                        document_url: "http://localhost:3000/".into(),
                        label: "File".into(),
                    })
                })
                .unwrap();
            assert_eq!(broker.overview().unwrap().pending_browser_uploads.len(), 1);
            assert!(broker
                .prepare_browser_upload(&id, &command.nonce, |_, _, _| panic!("reused preparation"))
                .is_err());
            if case == "cancel" {
                client
                    .call(Request::CancelOperation(OperationInput {
                        operation_id: id.clone(),
                    }))
                    .await
                    .unwrap();
                assert!(broker
                    .decide_browser_upload(&id, true, |_| panic!("cancelled dispatch"))
                    .is_err());
            } else {
                if case == "stale" {
                    control
                        .retain_snapshot("replacement", &control.navigation_id())
                        .unwrap();
                }
                broker
                    .decide_browser_upload(&id, case != "deny", |mut approval| {
                        assert_ne!(case, "deny");
                        assert_ne!(case, "stale");
                        (approval.check)().unwrap();
                        approval.file.verify(|| Ok(())).unwrap();
                        use std::io::Read;
                        let mut bytes = Vec::new();
                        approval.file.file.read_to_end(&mut bytes).unwrap();
                        assert_eq!(bytes, b"immutable");
                        if matches!(case, "before" | "after") {
                            Err(BrowserDownloadFailure {
                                code: ErrorCode::DeadlineExceeded,
                                no_effect: case == "before",
                            })
                        } else {
                            Ok(())
                        }
                    })
                    .unwrap();
            }
            let reply = client
                .call(Request::Operation(
                    OperationInput {
                        operation_id: id.clone(),
                    }
                    .into(),
                ))
                .await
                .unwrap();
            let value = serde_json::to_value(&reply).unwrap();
            assert_eq!(
                value["data"]["state"],
                match case {
                    "deny" | "cancel" => "cancelled",
                    "success" => "succeeded",
                    "after" => "outcome_unknown",
                    _ => "failed",
                },
                "{value}"
            );
            assert_eq!(
                operation(client.call(Request::UploadBrowser(request)).await.unwrap()),
                id
            );
            assert!(broker
                .decide_browser_upload(&id, true, |_| panic!("reused approval"))
                .is_err());
        }
        control
            .retain_snapshot("snapshot", &control.navigation_id())
            .unwrap();
        let mut invalid = input.clone();
        invalid.file_name = "../escape".into();
        assert!(matches!(
            client.call(Request::UploadBrowser(invalid)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::ResourceExhausted,
                ..
            }
        ));
        let mut invalid = input.clone();
        invalid.expected_sha256 = "0".repeat(64);
        assert!(matches!(
            client.call(Request::UploadBrowser(invalid)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::RevisionConflict,
                ..
            }
        ));
        control.take_over();
        let mut replay = input;
        replay.request_key = "success".into();
        assert!(matches!(
            client.call(Request::UploadBrowser(replay)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::ControlRevoked,
                ..
            }
        ));
        broker.shutdown().await;
    }
}
