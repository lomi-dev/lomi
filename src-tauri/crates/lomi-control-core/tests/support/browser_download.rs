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
async fn downloads_require_explicit_scope_and_preserve_uncertain_receipts_and_source_authority() {
    for allowed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let broker = Broker::start(&root.path().join("control")).unwrap();
        let mut p = projection(&broker, root.path());
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
        ];
        if allowed {
            scopes.push("browser.download".into());
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
        let input = BrowserDownloadInput {
            workspace_id: "a".into(),
            panel_id: panel_id.clone(),
            browser_generation: browser_generation.clone(),
            navigation_id: control.navigation_id(),
            url: "http://localhost:3000/file".into(),
            max_bytes: 4096,
            expected_revision: "2".into(),
            retry_epoch,
            request_key: "download".into(),
        };
        if !allowed {
            assert!(matches!(
                client.call(Request::DownloadBrowser(input)).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::ScopeDenied,
                    ..
                }
            ));
            assert!(commands.try_recv().is_err());
            broker.shutdown().await;
            continue;
        }
        for url in [
            "http://localhost:3001/file",
            "http://localhost:3000/file#fragment",
            "http://user@localhost:3000/file",
            "file:///private",
        ] {
            let mut denied = input.clone();
            denied.url = url.into();
            assert!(matches!(
                client.call(Request::DownloadBrowser(denied)).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::ScopeDenied,
                    ..
                }
            ));
        }
        let mut saved = None;
        for case in ["success", "before", "after", "oversize"] {
            let mut request = input.clone();
            request.request_key = case.into();
            let id = operation(
                client
                    .call(Request::DownloadBrowser(request.clone()))
                    .await
                    .unwrap(),
            );
            let command = commands.recv().await.unwrap();
            assert!(broker
                .execute_browser_download(&id, &command.nonce, |_, _, _| panic!(
                    "unclaimed dispatch"
                ))
                .is_err());
            broker.claim_ui(&p.ui_epoch, &id, &command.nonce).unwrap();
            broker
                .execute_browser_download(&id, &command.nonce, |_, _, _| match case {
                    "success" => Ok(vec![0, 255, 4]),
                    "oversize" => Ok(vec![0; 4097]),
                    _ => Err(BrowserDownloadFailure {
                        code: ErrorCode::DeadlineExceeded,
                        no_effect: case == "before",
                    }),
                })
                .unwrap();
            let reply = client
                .call(Request::Operation(
                    OperationInput {
                        operation_id: id.clone(),
                    }
                    .into(),
                ))
                .await
                .unwrap();
            let Reply::Ok {
                data:
                    Data::Operation {
                        state,
                        effect_state,
                        result,
                        ..
                    },
                ..
            } = reply
            else {
                panic!("{reply:?}")
            };
            assert_eq!(
                state,
                if case == "success" {
                    "succeeded"
                } else if case == "before" {
                    "failed"
                } else {
                    "outcome_unknown"
                }
            );
            assert_eq!(
                effect_state,
                if case == "success" {
                    "complete"
                } else if case == "before" {
                    "none"
                } else {
                    "unknown"
                }
            );
            if let Some(OperationResult::BrowserDownloaded(artifact)) = result {
                saved = Some((id.clone(), artifact));
            }
            assert_eq!(
                operation(
                    client
                        .call(Request::DownloadBrowser(request))
                        .await
                        .unwrap()
                ),
                id
            );
            assert!(broker
                .execute_browser_download(&id, &command.nonce, |_, _, _| panic!("duplicate GET"))
                .is_err());
            assert!(commands.try_recv().is_err());
        }
        let (id, artifact) = saved.unwrap();
        assert!(matches!(
            client
                .call(Request::ReadArtifact(ArtifactReadInput {
                    workspace_id: "a".into(),
                    artifact_id: artifact.id.clone()
                }))
                .await
                .unwrap(),
            Reply::Ok {
                data: Data::Artifact { image: None, .. },
                ..
            }
        ));
        control.take_over();
        assert!(matches!(
            client
                .call(Request::Operation(
                    OperationInput { operation_id: id }.into()
                ))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ControlRevoked,
                ..
            }
        ));
        assert!(matches!(
            client
                .call(Request::ReadArtifact(ArtifactReadInput {
                    workspace_id: "a".into(),
                    artifact_id: artifact.id.clone()
                }))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ControlRevoked,
                ..
            }
        ));
        broker.shutdown().await;
    }
}
