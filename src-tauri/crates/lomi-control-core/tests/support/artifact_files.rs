use super::*;
use sha2::{Digest, Sha256};
fn operation(reply: Reply) -> String {
    match reply {
        Reply::Ok {
            data: Data::Operation { operation_id, .. },
            ..
        } => operation_id,
        other => panic!("{other:?}"),
    }
}
async fn result(client: &Client, id: &str) -> OperationResult {
    match client
        .call(Request::Operation(
            OperationInput {
                operation_id: id.into(),
            }
            .into(),
        ))
        .await
        .unwrap()
    {
        Reply::Ok {
            data:
                Data::Operation {
                    result: Some(result),
                    ..
                },
            ..
        } => result,
        other => panic!("{other:?}"),
    }
}
#[tokio::test]
async fn generic_file_import_export_preserves_bytes_permissions_receipts_and_destinations() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    let broker = Broker::start(&path.join("control")).unwrap();
    let p = projection(&broker, &path);
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "files.mutate",
            "files.create",
            "artifact.import_file",
            "artifact.export",
        ],
    )
    .await;
    let legacy = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "files.read", "artifact.import"],
    )
    .await;
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
    let maximum = vec![0xA5; 4 * 1024 * 1024];
    for (index, bytes) in [
        &b"binary\0 Za\xc5\xbc\xc3\xb3\xc5\x82\xc4\x87 \xf0\x9f\x99\x82"[..],
        &b""[..],
        maximum.as_slice(),
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::write(path.join("source.bin"), bytes).unwrap();
        let input = ArtifactImportInput {
            workspace_id: "a".into(),
            relative_path: "source.bin".into(),
            kind: ArtifactImportKind::File,
            expected_byte_length: bytes.len() as u64,
            expected_sha256: format!("{:x}", Sha256::digest(bytes)),
            expected_revision: "1".into(),
            retry_epoch: retry_epoch.clone(),
            request_key: format!("import-{index}"),
        };
        assert!(matches!(
            legacy
                .call(Request::ImportArtifact(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
        let imported = operation(
            client
                .call(Request::ImportArtifact(input.clone()))
                .await
                .unwrap(),
        );
        let command = commands.recv().await.unwrap();
        broker
            .claim_ui(&p.ui_epoch, &imported, &command.nonce)
            .unwrap();
        broker
            .execute_import(&imported, &command.nonce, |_, _| {
                panic!("Generic file reached APK inspection")
            })
            .unwrap();
        let OperationResult::ArtifactImported { artifact_id, .. } =
            result(&client, &imported).await
        else {
            panic!()
        };
        std::fs::write(path.join("source.bin"), b"rewritten after private copy").unwrap();
        assert_eq!(
            operation(client.call(Request::ImportArtifact(input)).await.unwrap()),
            imported
        );
        let directory = lomi_control_core::project_files::ProjectDirectory::open(&path).unwrap();
        let export = ArtifactExportInput {
            workspace_id: "a".into(),
            artifact_id: artifact_id.clone(),
            expected_sha256: format!("{:x}", Sha256::digest(bytes)),
            relative_path: format!("export-{index}.bin"),
            expected_parent_revision: directory.list("", || Ok(())).unwrap().revision,
            expected_revision: "1".into(),
            retry_epoch: retry_epoch.clone(),
            request_key: format!("export-{index}"),
        };
        assert!(matches!(
            legacy
                .call(Request::ExportArtifact(export.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
        let mut foreign = export.clone();
        foreign.workspace_id = "b".into();
        assert!(matches!(
            client.call(Request::ExportArtifact(foreign)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        for forbidden in ["../escape.bin", ".env", ".ssh/key"] {
            let mut denied = export.clone();
            denied.relative_path = forbidden.into();
            assert!(matches!(
                client.call(Request::ExportArtifact(denied)).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::ScopeDenied,
                    ..
                }
            ));
        }
        let exported = operation(
            client
                .call(Request::ExportArtifact(export.clone()))
                .await
                .unwrap(),
        );
        let command = commands.recv().await.unwrap();
        assert!(broker
            .commit_artifact_export(&exported, &command.nonce)
            .is_err());
        broker
            .claim_ui(&p.ui_epoch, &exported, &command.nonce)
            .unwrap();
        let saved = broker
            .commit_artifact_export(&exported, &command.nonce)
            .unwrap();
        let mut forged = saved.clone();
        forged.relative_path = "forged".into();
        assert!(broker
            .acknowledge_ui(UiAck {
                operation_id: exported.clone(),
                nonce: command.nonce.clone(),
                ui_epoch: p.ui_epoch.clone(),
                result: OperationResult::ArtifactExported(Box::new(forged))
            })
            .is_err());
        broker
            .acknowledge_ui(UiAck {
                operation_id: exported.clone(),
                nonce: command.nonce.clone(),
                ui_epoch: p.ui_epoch.clone(),
                result: OperationResult::ArtifactExported(Box::new(saved)),
            })
            .unwrap();
        assert_eq!(
            std::fs::read(path.join(&export.relative_path)).unwrap(),
            bytes
        );
        assert_eq!(
            operation(
                client
                    .call(Request::ExportArtifact(export.clone()))
                    .await
                    .unwrap()
            ),
            exported
        );
        assert!(commands.try_recv().is_err());
        let mut conflict = export.clone();
        conflict.relative_path = "different.bin".into();
        assert!(matches!(
            client
                .call(Request::ExportArtifact(conflict))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::IdempotencyConflict,
                ..
            }
        ));
        for case in ["collision", "symlink", "stale", "revoke"] {
            let mut denied = export.clone();
            denied.request_key = format!("{case}-{index}");
            denied.relative_path = format!("{case}-{index}.bin");
            let destination = path.join(&denied.relative_path);
            if case == "collision" {
                std::fs::write(&destination, b"preserve").unwrap();
            }
            if case == "symlink" {
                std::os::unix::fs::symlink(path.join(&export.relative_path), &destination).unwrap();
            }
            denied.expected_parent_revision = directory.list("", || Ok(())).unwrap().revision;
            if case == "stale" {
                std::fs::write(path.join(format!("new-entry-{index}")), b"human").unwrap();
            }
            let id = operation(client.call(Request::ExportArtifact(denied)).await.unwrap());
            let command = commands.recv().await.unwrap();
            broker.claim_ui(&p.ui_epoch, &id, &command.nonce).unwrap();
            if case == "revoke" {
                client
                    .call(Request::CancelOperation(OperationInput {
                        operation_id: id.clone(),
                    }))
                    .await
                    .unwrap();
            }
            assert!(
                broker.commit_artifact_export(&id, &command.nonce).is_err(),
                "{case}"
            );
            if case == "collision" {
                assert_eq!(std::fs::read(&destination).unwrap(), b"preserve");
            } else if case == "symlink" {
                assert!(destination
                    .symlink_metadata()
                    .unwrap()
                    .file_type()
                    .is_symlink());
            } else {
                assert!(!destination.exists());
            }
            assert_eq!(
                std::fs::read(path.join(&export.relative_path)).unwrap(),
                bytes
            );
        }
    }
    broker.shutdown().await;
}
