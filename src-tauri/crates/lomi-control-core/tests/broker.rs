#![cfg(unix)]
use lomi_control_core::{broker::Broker, client::Client};
use lomi_control_protocol::{control::*, EmptyInput, ErrorCode};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn native_ui_thread_can_revoke_without_entering_the_tokio_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let broker = Broker::start(&temp.path().join("control")).unwrap();
    broker.publish(projection(&broker, temp.path())).unwrap();
    let client = approved(&broker, &["a"]).await;
    broker.wait_for_cleanup().await;
    let native = broker.clone();
    let revoked = std::thread::spawn(move || native.revoke()).join();
    assert!(
        revoked.is_ok(),
        "Native UI revocation panicked without an ambient Tokio runtime"
    );
    tokio::time::timeout(Duration::from_secs(2), broker.wait_for_cleanup())
        .await
        .unwrap();
    assert!(!matches!(
        client.call(Request::Workspaces(ListInput::default())).await,
        Ok(Reply::Ok { .. })
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn shortcut_source_requires_claim_revision_and_unconsumed_plan() {
    use lomi_control_core::broker::SettingsPlan;
    use serde_json::json;
    let temp = tempfile::tempdir().unwrap();
    let broker = Broker::start(&temp.path().join("control")).unwrap();
    let p = projection(&broker, temp.path());
    broker.publish(p.clone()).unwrap();
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| tx.send(c).map_err(std::io::Error::other)))
        .unwrap();
    broker
        .set_settings_prepare_dispatch(Arc::new(|patch, current, check| {
            check()?;
            let mut after = current.clone();
            patch.apply_values(&mut after)?;
            Ok(SettingsPlan {
                before: current.clone(),
                after,
                source_revision: None,
                apply: Arc::new(|check| {
                    check()?;
                    Ok("c".repeat(64))
                }),
            })
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "settings.read", "settings.write"],
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
    let input = SettingsUpdateInput {
        workspace_id: "a".into(),
        patch: SettingsPatch::KeybindingReset {
            action: "saveFile".into(),
        },
        expected_settings_revision: "b".repeat(64),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "shortcut-reset".into(),
    };
    client
        .call(Request::UpdateSettings(input.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    let source = |nonce: &str, revision: &str| {
        broker.settings_update_source_permit(&command.operation_id, nonce, revision)
    };
    assert!(source(&command.nonce, &input.expected_settings_revision).is_err());
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    assert!(source("forged", &input.expected_settings_revision).is_err());
    assert!(source(&command.nonce, &"0".repeat(64)).is_err());
    let permit = source(&command.nonce, &input.expected_settings_revision).unwrap();
    permit.check().unwrap();
    let current: SettingsUpdateValues = serde_json::from_value(json!({"focusFollowsPointer":false,"sourceRevision":null,"definitionsRevision":"a".repeat(64),"action":{"id":"saveFile","label":"Save file","shortcut":null,"defaultShortcut":"Meta+KeyS"}})).unwrap();
    broker
        .prepare_settings_update(
            &command.operation_id,
            &command.nonce,
            &input.expected_settings_revision,
            current,
        )
        .unwrap();
    assert!(source(&command.nonce, &input.expected_settings_revision).is_err());
    let pending =
        serde_json::to_value(broker.overview().unwrap().pending_settings_updates).unwrap();
    assert_eq!(pending[0]["patch"]["type"], "keybinding_reset");
    assert_eq!(
        pending[0]["before"]["action"]["shortcut"],
        serde_json::Value::Null
    );
    assert_eq!(pending[0]["after"]["action"]["shortcut"], "Meta+KeyS");
    broker
        .decide_settings_update(&command.operation_id, true)
        .unwrap();
    let reply = client.call(Request::UpdateSettings(input)).await.unwrap();
    let json = serde_json::to_value(reply).unwrap();
    assert_eq!(json["data"]["state"], "succeeded");
    assert_eq!(json["data"]["result"]["section"], "keybinds");
    assert!(source(&command.nonce, &"b".repeat(64)).is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn terminal_preference_changes_bind_native_plan_and_receipt_to_the_section() {
    use lomi_control_core::broker::SettingsPlan;
    use serde_json::json;
    let temp = tempfile::tempdir().unwrap();
    let broker = Broker::start(&temp.path().join("control")).unwrap();
    let p = projection(&broker, temp.path());
    broker.publish(p.clone()).unwrap();
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| tx.send(c).map_err(std::io::Error::other)))
        .unwrap();
    let current: SettingsUpdateValues = serde_json::from_value(json!({"appearance":{},"behavior":{
        "scrollback":10000,"scrollSensitivity":1,"fastScrollSensitivity":5,"smoothScrollDuration":0,"tabStopWidth":8,
        "scrollOnUserInput":true,"scrollOnEraseInDisplay":false,"altClickMovesCursor":true,"rightClickSelectsWord":false,
        "macOptionIsMeta":false,"macOptionClickForcesSelection":false,"screenReaderMode":false,"customGlyphs":true,
        "rescaleOverlappingGlyphs":false,"wordSeparator":" ()"
    },"windowsShell":"powershell","agentNotifications":true,"alwaysShowTitles":false})).unwrap();
    let native = current.clone();
    broker
        .set_settings_prepare_dispatch(Arc::new(move |patch, _current, check| {
            check()?;
            let mut after = native.clone();
            patch.apply_values(&mut after)?;
            Ok(SettingsPlan {
                before: native.clone(),
                after,
                source_revision: None,
                apply: Arc::new(|check| {
                    check()?;
                    Ok("a".repeat(64))
                }),
            })
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "settings.read", "settings.write"],
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
    let input = SettingsUpdateInput {
        workspace_id: "a".into(),
        patch: SettingsPatch::TerminalField {
            field: SettingsTerminalField::BehaviorTabStopWidth,
            value: Some(SettingsScalar::Number(4.0)),
        },
        expected_settings_revision: "b".repeat(64),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "terminal-preferences".into(),
    };
    client
        .call(Request::UpdateSettings(input.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    assert!(broker
        .prepare_settings_update(
            &command.operation_id,
            &command.nonce,
            &input.expected_settings_revision,
            SettingsEditorValues {
                tab_size: 4,
                insert_spaces: true
            }
            .into()
        )
        .is_err());
    broker
        .prepare_settings_update(
            &command.operation_id,
            &command.nonce,
            &input.expected_settings_revision,
            current,
        )
        .unwrap();
    let pending =
        serde_json::to_value(broker.overview().unwrap().pending_settings_updates).unwrap();
    assert_eq!(pending[0]["section"], "terminal");
    assert_eq!(pending[0]["after"]["behavior"]["tabStopWidth"], 4);
    assert_eq!(pending[0]["after"]["behavior"]["scrollback"], 10000);
    broker
        .decide_settings_update(&command.operation_id, true)
        .unwrap();
    let result = client
        .call(Request::Operation(
            OperationInput {
                operation_id: command.operation_id,
            }
            .into(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(&result, Reply::Ok { data: Data::Operation { state, result: Some(OperationResult::SettingsUpdated(s)), .. }, .. } if state == "succeeded" && s.section == SettingsSection::Terminal)
    );
    let retry = client.call(Request::UpdateSettings(input)).await.unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        serde_json::to_value(retry).unwrap()
    );
    broker.shutdown().await;
}

#[tokio::test]
async fn settings_updates_require_exact_approval_and_preserve_concurrent_changes() {
    use lomi_control_core::{atomic_file, broker::SettingsPlan};
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicUsize, Ordering};
    for case in [
        "create",
        "replace",
        "reject",
        "cancel",
        "stale",
        "symlink",
        "prepare-rejected",
        "mismatch",
        "wrong-plan",
        "revoke",
        "revoke-applying",
        "anchor-removed-applying",
        "uncertain",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap().join("preferences.json");
        let original = br#"{"tabSize":4,"insertSpaces":true}"#.to_vec();
        if case != "create" {
            std::fs::write(&path, &original).unwrap();
        }
        let broker = Broker::start(&temp.path().join("control")).unwrap();
        let p = projection(&broker, temp.path());
        broker.publish(p.clone()).unwrap();
        let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
        broker
            .set_ui_dispatch(Arc::new(move |c| tx.send(c).map_err(std::io::Error::other)))
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let apply_calls = calls.clone();
        let apply_path = path.clone();
        let weak = Arc::downgrade(&broker);
        let apply_projection = p.clone();
        broker
            .set_settings_prepare_dispatch(Arc::new(move |patch, _current, check| {
                check()?;
                if case == "prepare-rejected" {
                    return Err(ErrorCode::UnsupportedCapability);
                }
                let before = SettingsEditorValues {
                    tab_size: 4,
                    insert_spaces: true,
                };
                let mut after = before.clone();
                match patch {
                    SettingsPatch::EditorTabSize { value } => after.tab_size = *value,
                    SettingsPatch::EditorInsertSpaces { value } => after.insert_spaces = *value,
                    _ => return Err(ErrorCode::UnsupportedCapability),
                }
                if case == "wrong-plan" {
                    after.insert_spaces = false;
                }
                let source = std::fs::read(&apply_path).ok();
                let source_revision = source.as_ref().map(|s| format!("{:x}", Sha256::digest(s)));
                let expected = source_revision.clone();
                let bytes = serde_json::to_vec(&after).unwrap();
                let calls = apply_calls.clone();
                let path = apply_path.clone();
                let weak = weak.clone();
                let apply_projection = apply_projection.clone();
                Ok(SettingsPlan {
                    before: before.into(),
                    after: after.into(),
                    source_revision,
                    apply: Arc::new(move |check| {
                        if case == "revoke-applying" {
                            weak.upgrade().unwrap().revoke();
                        }
                        if case == "anchor-removed-applying" {
                            let mut retired = apply_projection.clone();
                            retired.revision = "2".into();
                            retired.workspaces.retain(|w| w.id != "a");
                            weak.upgrade().unwrap().publish(retired).unwrap();
                        }
                        check()?;
                        calls.fetch_add(1, Ordering::SeqCst);
                        let hash = match &expected {
                            Some(revision) => atomic_file::replace(&path, revision, &bytes, check)?,
                            None => atomic_file::create(&path, &bytes, check)?,
                        };
                        if case == "uncertain" {
                            return Err(atomic_file::ReplaceError::Uncertain);
                        }
                        Ok(hash)
                    }),
                })
            }))
            .unwrap();
        let client = approved_scopes(
            &broker,
            &["a"],
            &["workspace.read", "settings.read", "settings.write"],
        )
        .await;
        let readonly = approved_scopes(
            &broker,
            &["a"],
            &["workspace.read", "settings.read", "settings.open"],
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
        let input = SettingsUpdateInput {
            workspace_id: "a".into(),
            patch: SettingsPatch::EditorTabSize { value: 8 },
            expected_settings_revision: "a".repeat(64),
            expected_revision: "1".into(),
            retry_epoch,
            request_key: "settings-change".into(),
        };
        assert!(matches!(
            readonly
                .call(Request::UpdateSettings(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
        let mut foreign = input.clone();
        foreign.workspace_id = "foreign".into();
        assert!(matches!(
            client.call(Request::UpdateSettings(foreign)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        client
            .call(Request::UpdateSettings(input.clone()))
            .await
            .unwrap();
        let command = commands.recv().await.unwrap();
        assert!(broker
            .decide_settings_update(&command.operation_id, true)
            .is_err());
        let current = SettingsEditorValues {
            tab_size: 4,
            insert_spaces: true,
        };
        assert!(broker
            .prepare_settings_update(
                &command.operation_id,
                &command.nonce,
                &input.expected_settings_revision,
                current.clone().into()
            )
            .is_err());
        broker
            .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
            .unwrap();
        assert!(broker
            .prepare_settings_update(
                &command.operation_id,
                "wrong",
                &input.expected_settings_revision,
                current.clone().into()
            )
            .is_err());
        assert!(broker
            .prepare_settings_update(
                &command.operation_id,
                &command.nonce,
                &"b".repeat(64),
                current.clone().into()
            )
            .is_err());
        let forged = UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::SettingsUpdated(SettingsUpdated {
                workspace_id: "a".into(),
                section: SettingsSection::Editor,
                previous_stored_revision: None,
                stored_revision: "b".repeat(64),
                applied: true,
            }),
        };
        assert!(broker.acknowledge_ui(forged.clone()).is_err());
        let mut observed = current;
        if case == "mismatch" {
            observed.tab_size = 2;
        }
        let prepared = broker.prepare_settings_update(
            &command.operation_id,
            &command.nonce,
            &input.expected_settings_revision,
            observed.into(),
        );
        if matches!(case, "prepare-rejected" | "mismatch" | "wrong-plan") {
            let code = if case == "prepare-rejected" {
                ErrorCode::UnsupportedCapability
            } else {
                ErrorCode::RevisionConflict
            };
            assert!(matches!(prepared, Err(error) if error == code));
            assert!(broker
                .overview()
                .unwrap()
                .pending_settings_updates
                .is_empty());
            assert!(broker
                .decide_settings_update(&command.operation_id, true)
                .is_err());
            broker
                .acknowledge_ui(UiAck {
                    operation_id: command.operation_id.clone(),
                    nonce: command.nonce.clone(),
                    ui_epoch: p.ui_epoch.clone(),
                    result: OperationResult::Failure { code },
                })
                .unwrap();
        } else {
            let permit = prepared.unwrap();
            let pending = broker.overview().unwrap().pending_settings_updates;
            assert_eq!(pending.len(), 1);
            let pending = serde_json::to_value(pending).unwrap();
            assert_eq!(pending[0]["before"]["tabSize"], 4);
            assert_eq!(pending[0]["after"]["tabSize"], 8);
            assert_eq!(pending[0]["after"]["insertSpaces"], true);
            assert_eq!(pending[0]["operationId"], command.operation_id);
            assert!(broker.acknowledge_ui(forged).is_err());
            assert!(broker
                .prepare_settings_update(
                    &command.operation_id,
                    &command.nonce,
                    &input.expected_settings_revision,
                    SettingsEditorValues {
                        tab_size: 4,
                        insert_spaces: true
                    }
                    .into()
                )
                .is_err());
            match case {
                "reject" => broker
                    .decide_settings_update(&command.operation_id, false)
                    .unwrap(),
                "cancel" => {
                    client
                        .call(Request::CancelOperation(OperationInput {
                            operation_id: command.operation_id.clone(),
                        }))
                        .await
                        .unwrap();
                    assert!(permit.check().is_err());
                }
                "revoke" => {
                    broker.revoke();
                    assert!(permit.check().is_err());
                }
                _ => {
                    if case == "stale" {
                        std::fs::write(&path, b"concurrent-human-change").unwrap();
                    }
                    if case == "symlink" {
                        let outside = temp.path().join("outside.json");
                        std::fs::write(&outside, b"outside-private").unwrap();
                        std::fs::remove_file(&path).unwrap();
                        std::os::unix::fs::symlink(&outside, &path).unwrap();
                    }
                    let applied = broker.decide_settings_update(&command.operation_id, true);
                    assert_eq!(
                        applied.is_ok(),
                        matches!(case, "create" | "replace"),
                        "{case}: {applied:?}"
                    );
                }
            }
        }
        assert!(
            broker
                .decide_settings_update(&command.operation_id, true)
                .is_err(),
            "approval is single use: {case}"
        );
        let actual = std::fs::read(&path).unwrap();
        match case {
            "create" | "replace" | "uncertain" => {
                assert_eq!(
                    serde_json::from_slice::<SettingsEditorValues>(&actual).unwrap(),
                    SettingsEditorValues {
                        tab_size: 8,
                        insert_spaces: true
                    }
                );
                assert_eq!(calls.load(Ordering::SeqCst), 1);
            }
            "stale" => assert_eq!(actual, b"concurrent-human-change"),
            "symlink" => assert_eq!(actual, b"outside-private"),
            _ => {
                assert_eq!(actual, original);
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            }
        }
        if case == "anchor-removed-applying" {
            let hidden = client
                .call(Request::Operation(
                    OperationInput {
                        operation_id: command.operation_id.clone(),
                    }
                    .into(),
                ))
                .await
                .unwrap();
            assert!(matches!(
                hidden,
                Reply::Error {
                    code: ErrorCode::TargetNotFound,
                    ..
                }
            ));
            let mut restored = p.clone();
            restored.revision = "3".into();
            broker.publish(restored).unwrap();
        }
        if !matches!(case, "revoke" | "revoke-applying") {
            let result = client
                .call(Request::Operation(
                    OperationInput {
                        operation_id: command.operation_id.clone(),
                    }
                    .into(),
                ))
                .await
                .unwrap();
            let value = serde_json::to_value(&result).unwrap();
            let (state, effect) = match case {
                "create" | "replace" => ("succeeded", "complete"),
                "uncertain" => ("outcome_unknown", "unknown"),
                "prepare-rejected"
                | "mismatch"
                | "wrong-plan"
                | "stale"
                | "symlink"
                | "anchor-removed-applying" => ("failed", "none"),
                _ => ("cancelled", "none"),
            };
            assert!(
                matches!(&result, Reply::Ok { data: Data::Operation { state: s, effect_state: e, .. }, .. } if s==state && e==effect),
                "{case}: {value}"
            );
            let count = calls.load(Ordering::SeqCst);
            let retry = client
                .call(Request::UpdateSettings(input.clone()))
                .await
                .unwrap();
            assert_eq!(serde_json::to_value(retry).unwrap(), value);
            assert_eq!(calls.load(Ordering::SeqCst), count);
            let mut changed = input;
            changed.patch = SettingsPatch::EditorTabSize { value: 2 };
            assert!(matches!(
                client.call(Request::UpdateSettings(changed)).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::IdempotencyConflict,
                    ..
                }
            ));
        }
        broker.shutdown().await;
    }
}

#[tokio::test]
async fn settings_reads_isolate_global_grants_and_validate_replies_and_pages() {
    use std::sync::atomic::{AtomicU8, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let broker = Broker::start(&temp.path().join("control")).unwrap();
    broker.publish(projection(&broker, temp.path())).unwrap();
    let reader = approved_scopes(&broker, &["a"], &["workspace.read", "settings.read"]).await;
    let opener = approved_scopes(&broker, &["a"], &["workspace.read", "settings.open"]).await;
    let mode = Arc::new(AtomicU8::new(0));
    let fixture = mode.clone();
    let weak = Arc::downgrade(&broker);
    broker
        .set_settings_read_dispatch(Arc::new(move |r| {
            let broker = weak.upgrade().unwrap();
            let mode = fixture.load(Ordering::SeqCst);
            if mode == 7 {
                broker.revoke();
            }
            let values = if r.input.section == SettingsSection::Keybinds {
                SettingsValues::Keybinds {
                    focus_follows_pointer: false,
                    items: vec![SettingsBinding {
                        action: format!("action-{}", r.input.offset),
                        shortcut: None,
                        default_shortcut: Some("Ctrl+KeyK".into()),
                    }],
                    total: 2,
                    offset: r.input.offset,
                    next_offset: if mode == 6 {
                        None
                    } else {
                        (r.input.offset == 0).then_some(1)
                    },
                }
            } else {
                SettingsValues::Editor {
                    tab_size: if mode == 5 { 0 } else { 4 },
                    insert_spaces: true,
                }
            };
            let reply = SettingsReadReply {
                request_id: r.request_id,
                ui_epoch: r.ui_epoch,
                snapshot: Some(SettingsSnapshot {
                    workspace_id: if mode == 1 {
                        "foreign".into()
                    } else {
                        r.input.workspace_id
                    },
                    section: if mode == 2 {
                        SettingsSection::Themes
                    } else {
                        r.input.section
                    },
                    revision: if mode == 3 {
                        "malformed".into()
                    } else {
                        "a".repeat(64)
                    },
                    readiness: if mode == 4 {
                        SettingsReadiness::RecoveryRequired
                    } else {
                        SettingsReadiness::Ready
                    },
                    values,
                }),
                error: None,
            };
            broker.settings_read_reply(reply)
        }))
        .unwrap();
    let input = SettingsReadInput {
        workspace_id: "a".into(),
        section: SettingsSection::Editor,
        offset: 0,
        limit: 100,
        expected_revision: None,
    };
    assert!(matches!(
        opener
            .call(Request::ReadSettings(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let mut foreign = input.clone();
    foreign.workspace_id = "foreign".into();
    assert!(matches!(
        reader.call(Request::ReadSettings(foreign)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert!(matches!(
        reader
            .call(Request::ReadSettings(input.clone()))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::SettingsSnapshot(_),
            ..
        }
    ));
    for value in [1, 2, 3, 5] {
        mode.store(value, Ordering::SeqCst);
        assert!(
            matches!(
                reader
                    .call(Request::ReadSettings(input.clone()))
                    .await
                    .unwrap(),
                Reply::Error {
                    code: ErrorCode::OutcomeUnknown,
                    ..
                }
            ),
            "malformed mode {value}"
        );
    }
    mode.store(4, Ordering::SeqCst);
    assert!(
        matches!(reader.call(Request::ReadSettings(input.clone())).await.unwrap(),Reply::Ok{data:Data::SettingsSnapshot(ref s),..} if s.readiness==SettingsReadiness::RecoveryRequired)
    );
    mode.store(0, Ordering::SeqCst);
    let mut changed = input.clone();
    changed.expected_revision = Some("b".repeat(64));
    assert!(matches!(
        reader.call(Request::ReadSettings(changed)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::RevisionConflict,
            ..
        }
    ));
    let mut page = input.clone();
    page.section = SettingsSection::Keybinds;
    page.limit = 1;
    assert!(
        matches!(reader.call(Request::ReadSettings(page.clone())).await.unwrap(),Reply::Ok{data:Data::SettingsSnapshot(ref s),..} if matches!(s.values,SettingsValues::Keybinds{next_offset:Some(1),..}))
    );
    page.offset = 1;
    assert!(matches!(
        reader
            .call(Request::ReadSettings(page.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ResourceExhausted,
            ..
        }
    ));
    page.expected_revision = Some("a".repeat(64));
    assert!(
        matches!(reader.call(Request::ReadSettings(page.clone())).await.unwrap(),Reply::Ok{data:Data::SettingsSnapshot(ref s),..} if matches!(s.values,SettingsValues::Keybinds{next_offset:None,..}))
    );
    mode.store(6, Ordering::SeqCst);
    page.offset = 0;
    assert!(matches!(
        reader.call(Request::ReadSettings(page)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::OutcomeUnknown,
            ..
        }
    ));
    mode.store(7, Ordering::SeqCst);
    let revoked = reader.call(Request::ReadSettings(input)).await;
    assert!(
        revoked.is_err()
            || matches!(
                revoked.unwrap(),
                Reply::Error {
                    code: ErrorCode::ControlRevoked,
                    ..
                }
            )
    );
    broker.shutdown().await;
}

#[tokio::test]
async fn settings_open_requires_scope_native_commit_and_retains_exact_receipts() {
    for case in [
        "success",
        "cancel-queued",
        "cancel-committed",
        "revision",
        "revoke",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let broker = Broker::start(&temp.path().join("control")).unwrap();
        let mut p = projection(&broker, temp.path());
        broker.publish(p.clone()).unwrap();
        let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
        broker
            .set_ui_dispatch(Arc::new(move |c| tx.send(c).map_err(std::io::Error::other)))
            .unwrap();
        let client = approved_scopes(&broker, &["a"], &["workspace.read", "settings.open"]).await;
        let other = approved(&broker, &["a"]).await;
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
        let input = SettingsOpenInput {
            workspace_id: "a".into(),
            page: SettingsPage::Editor,
            expected_revision: "1".into(),
            retry_epoch,
            request_key: "open-settings".into(),
        };
        assert!(matches!(
            other
                .call(Request::OpenSettings(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
        let mut foreign = input.clone();
        foreign.workspace_id = "foreign".into();
        assert!(matches!(
            client.call(Request::OpenSettings(foreign)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        let queued = client
            .call(Request::OpenSettings(input.clone()))
            .await
            .unwrap();
        assert!(
            matches!(queued,Reply::Ok {data:Data::Operation{ref state,..},..} if state=="queued"),
            "{queued:?}"
        );
        let command = commands.recv().await.unwrap();
        assert!(broker
            .begin_settings_open(&command.operation_id, &command.nonce)
            .is_err());
        assert!(broker
            .complete_settings_open(&command.operation_id, &command.nonce)
            .is_err());
        let forged = UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::SettingsOpened(SettingsOpened {
                workspace_id: "a".into(),
                page: SettingsPage::Editor,
                requested: true,
            }),
        };
        assert!(broker.acknowledge_ui(forged.clone()).is_err());
        if case == "cancel-queued" {
            client
                .call(Request::CancelOperation(OperationInput {
                    operation_id: command.operation_id.clone(),
                }))
                .await
                .unwrap();
            assert!(broker
                .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
                .is_err());
            assert!(broker
                .begin_settings_open(&command.operation_id, &command.nonce)
                .is_err());
        } else {
            broker
                .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
                .unwrap();
            assert!(broker
                .begin_settings_open(&command.operation_id, "wrong-nonce")
                .is_err());
            if case == "revision" {
                p.revision = "2".into();
                broker.publish(p.clone()).unwrap();
                assert!(matches!(
                    broker.begin_settings_open(&command.operation_id, &command.nonce),
                    Err(ErrorCode::RevisionConflict)
                ));
                client
                    .call(Request::CancelOperation(OperationInput {
                        operation_id: command.operation_id.clone(),
                    }))
                    .await
                    .unwrap();
            } else {
                let (ticket, permit) = broker
                    .begin_settings_open(&command.operation_id, &command.nonce)
                    .unwrap();
                assert!(
                    matches!(ticket.action,UiAction::OpenSettings(ref c) if c.workspace_id=="a" && c.page==SettingsPage::Editor)
                );
                assert!(broker
                    .begin_settings_open(&command.operation_id, &command.nonce)
                    .is_err());
                assert!(
                    broker.acknowledge_ui(forged.clone()).is_err(),
                    "UI ACK cannot replace native completion"
                );
                if case == "cancel-committed" {
                    client
                        .call(Request::CancelOperation(OperationInput {
                            operation_id: command.operation_id.clone(),
                        }))
                        .await
                        .unwrap();
                    assert!(permit.check().is_err());
                    assert!(broker
                        .complete_settings_open(&command.operation_id, &command.nonce)
                        .is_err());
                } else if case == "revoke" {
                    broker.revoke();
                    assert!(permit.check().is_err());
                    assert!(broker
                        .complete_settings_open(&command.operation_id, &command.nonce)
                        .is_err());
                    assert!(broker.acknowledge_ui(forged).is_err());
                    broker.shutdown().await;
                    continue;
                } else {
                    permit.check().unwrap();
                    broker
                        .complete_settings_open(&command.operation_id, &command.nonce)
                        .unwrap();
                }
            }
        }
        let result = client
            .call(Request::Operation(
                OperationInput {
                    operation_id: command.operation_id.clone(),
                }
                .into(),
            ))
            .await
            .unwrap();
        if case == "success" {
            assert!(
                matches!(result,Reply::Ok{data:Data::Operation{ref state,ref effect_state,result:Some(OperationResult::SettingsOpened(ref s)),..},..}
                if state=="succeeded" && effect_state=="complete" && s.requested && s.page==SettingsPage::Editor),
                "{result:?}"
            );
        } else {
            assert!(
                !matches!(result,Reply::Ok{data:Data::Operation{ref state,..},..} if state=="succeeded"),
                "{result:?}"
            );
        }
        let retry = client
            .call(Request::OpenSettings(input.clone()))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(retry).unwrap(),
            serde_json::to_value(result).unwrap()
        );
        let mut changed = input.clone();
        changed.page = SettingsPage::Terminal;
        assert!(matches!(
            client.call(Request::OpenSettings(changed)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::IdempotencyConflict,
                ..
            }
        ));
        assert!(commands.try_recv().is_err(), "Retry dispatched again");
        assert!(matches!(
            other
                .call(Request::Operation(
                    OperationInput {
                        operation_id: command.operation_id
                    }
                    .into()
                ))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        drop(client);
        drop(other);
        broker.shutdown().await;
    }
}

fn projection(broker: &Broker, root: &std::path::Path) -> Projection {
    Projection {
        ui_epoch: broker.register_ui().unwrap(),
        revision: "1".into(),
        workspaces: [("a", "p"), ("b", "p"), ("foreign", "other")]
            .into_iter()
            .map(|(id, p)| Workspace {
                id: id.into(),
                project_id: p.into(),
                name: id.into(),
                project_name: p.into(),
                active_panel_id: None,
                project_path: root.canonicalize().unwrap().to_string_lossy().into(),
            })
            .collect(),
        ..Projection::default()
    }
}
async fn approved(broker: &Arc<Broker>, workspaces: &[&str]) -> Client {
    approved_scopes(broker, workspaces, &["workspace.read"]).await
}
async fn approved_scopes(broker: &Arc<Broker>, workspaces: &[&str], scopes: &[&str]) -> Client {
    approved_domains(broker, workspaces, scopes, &[]).await
}
async fn approved_domains(
    broker: &Arc<Broker>,
    workspaces: &[&str],
    scopes: &[&str],
    devices: &[&str],
) -> Client {
    approved_apps(broker, workspaces, scopes, devices, &[]).await
}
async fn approved_apps(
    broker: &Arc<Broker>,
    workspaces: &[&str],
    scopes: &[&str],
    devices: &[&str],
    packages: &[&str],
) -> Client {
    let endpoint = broker.endpoint.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        Client::connect(&endpoint, "Fixture", |id| {
            tx.send(id).unwrap();
        })
        .await
    });
    let id = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .unwrap()
        .unwrap();
    assert!(!broker
        .overview()
        .unwrap()
        .sessions
        .iter()
        .any(|s| s.id == id));
    assert!(
        !task.is_finished(),
        "No authenticated connection before approval"
    );
    broker
        .approve_android_apps(
            &id,
            &workspaces.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &scopes.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &[],
            &devices.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &packages.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
        .unwrap();
    task.await.unwrap().unwrap()
}

#[tokio::test]
async fn approval_filters_secondary_ids_and_binds_cursors_to_revision() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    broker.publish(p.clone()).unwrap();
    let client = approved(&broker, &["a", "b"]).await;
    let reply = client
        .call(Request::Workspaces(ListInput {
            limit: 1,
            cursor: None,
        }))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::Workspaces {
            items, next_cursor, ..
        },
        ..
    } = reply
    else {
        panic!("Expected workspaces")
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "a");
    let cursor = next_cursor.unwrap();
    assert!(matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: "foreign".into()
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert!(matches!(
        client
            .call(Request::Operation(
                OperationInput {
                    operation_id: "foreign".into()
                }
                .into()
            ))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert!(matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: "a".into()
            }))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::Connected { .. },
            ..
        }
    ));
    p.revision = "2".into();
    p.workspaces.retain(|w| w.id != "a");
    broker.publish(p).unwrap();
    assert!(matches!(
        client
            .call(Request::Workspaces(ListInput {
                limit: 1,
                cursor: Some(cursor)
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    assert!(matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: "a".into()
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    broker.revoke();
    assert!(client.call(Request::Status(EmptyInput {})).await.is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn rejected_pin_and_ui_reload_cannot_reuse_authority() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, root.path())).unwrap();
    let mut wrong = broker.endpoint.clone();
    wrong.broker_sha256 = "0".repeat(64);
    assert!(Client::connect(&wrong, "Wrong pin", |_| panic!(
        "Must authenticate broker before asking approval"
    ))
    .await
    .is_err());
    broker.revoke();
    broker.wait_for_cleanup().await;
    let client = approved(&broker, &["a"]).await;
    let old = broker.overview().unwrap().endpoint;
    broker.register_ui().unwrap();
    assert!(client.call(Request::Status(EmptyInput {})).await.is_err());
    broker.shutdown().await;
    drop(broker);
    let broker = Broker::start(&root.path().join("control")).unwrap();
    assert_ne!(old.instance_id, broker.endpoint.instance_id);
    assert_ne!(old.broker_sha256, broker.endpoint.broker_sha256);
    assert!(Client::connect(&old, "Old instance", |_| {}).await.is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn shutdown_releases_receipt_owner_before_immediate_restart() {
    let root = tempfile::tempdir().unwrap();
    for _ in 0..16 {
        let broker = Broker::start(&root.path().join("control")).unwrap();
        broker.register_ui().unwrap();
        broker.revoke();
        let weak = std::sync::Arc::downgrade(&broker);
        broker.shutdown().await;
        drop(broker);
        assert!(
            weak.upgrade().is_none(),
            "Shutdown left a cleanup worker alive"
        );
    }
}

#[tokio::test]
async fn mutations_are_durable_deduplicated_and_guarded_at_claim_and_ack() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(&broker, &["a"], &["workspace.read", "workspace.write"]).await;
    let connected = client
        .call(Request::Connect(ConnectInput {
            workspace_id: "a".into(),
        }))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::Connected { retry_epoch, .. },
        ..
    } = connected
    else {
        panic!("Missing connection")
    };
    let input = WorkspaceRenameInput {
        workspace_id: "a".into(),
        name: "Renamed 🙂".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "rename-1".into(),
    };
    let result = client
        .call(Request::RenameWorkspace(input.clone().into()))
        .await
        .unwrap();
    let Reply::Ok {
        data:
            Data::Operation {
                operation_id,
                state,
                ..
            },
        ..
    } = result
    else {
        panic!("Missing operation")
    };
    assert_eq!(state, "queued");
    let command = commands.recv().await.unwrap();
    assert_eq!(command.operation_id, operation_id);
    let repeat = client
        .call(Request::RenameWorkspace(input.clone().into()))
        .await
        .unwrap();
    assert!(
        matches!(repeat,Reply::Ok {data:Data::Operation {operation_id:ref id,..},..} if id==&operation_id)
    );
    assert!(commands.try_recv().is_err());
    let mut conflict = input.clone();
    conflict.name = "Different".into();
    assert!(matches!(
        client
            .call(Request::RenameWorkspace(conflict.into()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::IdempotencyConflict,
            ..
        }
    ));
    assert!(broker
        .claim_ui(&p.ui_epoch, &operation_id, "forged")
        .is_err());
    broker
        .claim_ui(&p.ui_epoch, &operation_id, &command.nonce)
        .unwrap();
    assert!(broker
        .claim_ui(&p.ui_epoch, &operation_id, &command.nonce)
        .is_err());
    let ack = UiAck {
        operation_id: operation_id.clone(),
        nonce: command.nonce,
        ui_epoch: p.ui_epoch.clone(),
        result: OperationResult::Workspace {
            workspace_id: "a".into(),
            name: input.name.clone(),
        },
    };
    assert!(
        broker.acknowledge_ui(ack.clone()).is_err(),
        "No success without matching domain publication"
    );
    p.revision = "2".into();
    p.workspaces[0].name = input.name.clone();
    broker.publish(p.clone()).unwrap();
    broker.acknowledge_ui(ack.clone()).unwrap();
    assert!(broker.acknowledge_ui(ack).is_err());
    let repeat = client
        .call(Request::RenameWorkspace(input.clone().into()))
        .await
        .unwrap();
    assert!(
        matches!(repeat,Reply::Ok {data:Data::Operation {ref state,result:Some(OperationResult::Workspace {..}),..},..} if state=="succeeded")
    );
    assert!(commands.try_recv().is_err());
    let mut stale = input.clone();
    stale.request_key = "stale".into();
    assert!(matches!(
        client
            .call(Request::RenameWorkspace(stale.into()))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::Operation {
                result: Some(OperationResult::Failure {
                    code: ErrorCode::RevisionConflict
                }),
                ..
            },
            ..
        }
    ));
    let mut queued = input.clone();
    queued.expected_revision = "2".into();
    queued.request_key = "cancel".into();
    client
        .call(Request::RenameWorkspace(queued.clone().into()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    let cancelled = client
        .call(Request::CancelOperation(OperationInput {
            operation_id: command.operation_id.clone(),
        }))
        .await
        .unwrap();
    assert!(
        matches!(cancelled,Reply::Ok {data:Data::Operation {ref state,..},..} if state=="cancelled")
    );
    assert!(broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .is_err());
    queued.request_key = "lost-ack".into();
    client
        .call(Request::RenameWorkspace(queued.into()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let pairing = broker.overview().unwrap().sessions[0].id.clone();
    broker.revoke();
    assert!(broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce,
            ui_epoch: p.ui_epoch,
            result: OperationResult::Workspace {
                workspace_id: "a".into(),
                name: input.name
            }
        })
        .is_err());
    broker.shutdown().await;
    drop(client);
    drop(broker);
    let store =
        lomi_control_core::receipts::Store::open(&root.path().join("control"), 200).unwrap();
    let receipt = store.get(&pairing, "p", &command.operation_id).unwrap();
    assert_eq!(
        receipt.state,
        lomi_control_core::receipts::State::OutcomeUnknown
    );
    assert_eq!(
        receipt.effect_state,
        lomi_control_core::receipts::Effect::Unknown
    );
}

#[tokio::test]
async fn workspace_read_grant_does_not_allow_a_rename() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, root.path())).unwrap();
    let client = approved(&broker, &["a"]).await;
    assert!(matches!(
        client
            .call(Request::RenameWorkspace(
                WorkspaceRenameInput {
                    workspace_id: "a".into(),
                    name: "Unauthorized".into(),
                    expected_revision: "1".into(),
                    retry_epoch: "missing".into(),
                    request_key: "rename".into()
                }
                .into()
            ))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn late_publication_failure_cannot_invalidate_a_new_ui_epoch() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let old = projection(&broker, root.path());
    broker.publish(old.clone()).unwrap();
    let current = projection(&broker, root.path());
    broker.publish(current.clone()).unwrap();
    assert!(broker.publish(old.clone()).is_err());
    broker.invalidate_ui_epoch(Some(&old.ui_epoch));
    assert!(broker.overview().unwrap().ui_ready);
    broker.invalidate_ui_epoch(Some(&current.ui_epoch));
    assert!(!broker.overview().unwrap().ui_ready);
    broker.shutdown().await;
}

#[tokio::test]
async fn terminal_spawn_requires_scope_native_ticket_profile_revision_and_live_grant() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    let profile = TerminalProfile {
        id: "zsh".into(),
        revision: "pinned".into(),
    };
    p.terminal_profile = Some(profile.clone());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let read_only = approved_scopes(&broker, &["a"], &["workspace.read", "panel.create"]).await;
    let mut input = TerminalCreateInput {
        workspace_id: "a".into(),
        cwd_relative: ".".into(),
        profile_id: None,
        title: "Agent".into(),
        expected_revision: "1".into(),
        retry_epoch: "missing".into(),
        request_key: "create".into(),
    };
    assert!(matches!(
        read_only
            .call(Request::CreateTerminal(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    assert!(commands.try_recv().is_err());
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "panel.create",
            "terminal.execute",
            "terminal.read",
        ],
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
    input.retry_epoch = retry_epoch;
    input.cwd_relative = "..".into();
    assert!(matches!(
        client
            .call(Request::CreateTerminal(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    input.cwd_relative = ".".into();
    client
        .call(Request::CreateTerminal(input.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    let UiAction::CreateTerminal {
        ref terminal_session_id,
        ref cwd,
        ref panel_id,
        ref tab_id,
        ..
    } = command.action
    else {
        panic!()
    };
    let start = || {
        broker.start_terminal(
            &command.operation_id,
            &command.nonce,
            terminal_session_id,
            &profile,
            cwd,
            |_| Ok(()),
        )
    };
    assert!(start().is_err(), "Cannot start before UI claim");
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let changed = TerminalProfile {
        revision: "changed".into(),
        ..profile.clone()
    };
    assert!(broker
        .start_terminal(
            &command.operation_id,
            &command.nonce,
            terminal_session_id,
            &changed,
            cwd,
            |_| Ok(())
        )
        .is_err());
    let monitor = broker
        .start_terminal(
            &command.operation_id,
            &command.nonce,
            terminal_session_id,
            &profile,
            cwd,
            Ok,
        )
        .unwrap();
    assert!(start().is_err(), "Ticket is single use");
    p.revision = "2".into();
    p.panels.push(Panel {
        android_device_id: None,
        browser_generation: None,
        id: panel_id.clone(),
        tab_id: tab_id.clone(),
        workspace_id: "a".into(),
        kind: "terminal".into(),
        title: "Agent".into(),
        terminal_session_id: Some(terminal_session_id.clone()),
    });
    broker.publish(p.clone()).unwrap();
    broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::Terminal {
                workspace_id: "a".into(),
                panel_id: panel_id.clone(),
                terminal_session_id: terminal_session_id.clone(),
                lease_id: Some("forged-ui-lease".into()),
                ready: true,
            },
        })
        .unwrap();
    let receipt = client.call(Request::CreateTerminal(input)).await.unwrap();
    let Reply::Ok {
        data:
            Data::Operation {
                result:
                    Some(OperationResult::Terminal {
                        lease_id: Some(lease),
                        ..
                    }),
                ..
            },
        ..
    } = receipt
    else {
        panic!("{receipt:?}")
    };
    assert_eq!(monitor.lock().unwrap().lease(), Some(lease.as_str()));
    assert_ne!(lease, "forged-ui-lease");
    assert!(commands.try_recv().is_err());
    let weak = Arc::downgrade(&broker);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed_calls = calls.clone();
    broker
        .set_screen_dispatch(Arc::new(move |request| {
            let broker = weak.upgrade().unwrap();
            assert!(broker
                .screen_reply(ScreenReply {
                    request_id: request.request_id.clone(),
                    ui_epoch: "wrong-epoch".into(),
                    screen: None
                })
                .is_err());
            let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            broker.screen_reply(ScreenReply {
                request_id: request.request_id,
                ui_epoch: request.ui_epoch,
                screen: Some(TerminalScreen {
                    workspace_id: request.workspace_id,
                    panel_id: request.panel_id,
                    terminal_session_id: if call == 0 {
                        "wrong-generation".into()
                    } else {
                        request.terminal_session_id
                    },
                    columns: 80,
                    rows: 24,
                    cursor_column: 0,
                    cursor_row: 0,
                    cursor_visible: true,
                    viewport_offset: 0,
                    buffer: ScreenBuffer::Normal,
                    text: "screen fixture".into(),
                    truncated: false,
                    parsed_sequence: if call == 1 { "900".into() } else { "0".into() },
                    stream_sequence: "900".into(),
                    parser_pending: false,
                }),
            })
        }))
        .unwrap();
    let screen_request = TerminalReadInput {
        mode: TerminalReadMode::Screen,
        minimum_parsed_sequence: None,
        workspace_id: "a".into(),
        panel_id: panel_id.clone(),
        terminal_session_id: terminal_session_id.clone(),
        cursor: None,
        max_bytes: Some(1024),
        operation_id: None,
    };
    assert!(matches!(
        read_only
            .call(Request::ReadTerminal(screen_request.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert_eq!(observed_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    for _ in 0..2 {
        assert!(matches!(
            client
                .call(Request::ReadTerminal(screen_request.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::StaleGeneration,
                ..
            }
        ));
    }
    let screen = client
        .call(Request::ReadTerminal(screen_request))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::TerminalScreen(screen),
        ..
    } = screen
    else {
        panic!("Expected screen");
    };
    assert_eq!(
        screen.stream_sequence, "0",
        "Native producer owns the watermark"
    );
    assert_eq!(screen.text, "screen fixture");
    let owner = monitor.lock().unwrap().owner.clone();
    broker.revoke();
    assert!(monitor.lock().unwrap().lease().is_none());
    assert!(start().is_err());
    broker.shutdown().await;
    drop(client);
    drop(read_only);
    drop(broker);
    let store =
        lomi_control_core::receipts::Store::open(&root.path().join("control"), 200).unwrap();
    let durable = store.get(&owner, "p", &command.operation_id).unwrap();
    assert!(
        matches!(
            durable.result,
            Some(OperationResult::Terminal { lease_id: None, .. })
        ),
        "Lease must never persist"
    );
}

#[tokio::test]
async fn workspace_creation_grants_only_its_creator_and_events_stay_scoped() {
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
    let creator = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "workspace.write", "panel.create"],
    )
    .await;
    let reader = approved(&broker, &["a"]).await;
    let Reply::Ok {
        data: Data::Connected { retry_epoch, .. },
        ..
    } = creator
        .call(Request::Connect(ConnectInput {
            workspace_id: "a".into(),
        }))
        .await
        .unwrap()
    else {
        panic!()
    };
    let input = WorkspaceRenameInput {
        workspace_id: "a".into(),
        name: "Scratch".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "new-workspace".into(),
    };
    creator
        .call(Request::CreateWorkspace(input.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    let UiAction::CreateWorkspace {
        ref workspace_id,
        ref tab_id,
        ..
    } = command.action
    else {
        panic!()
    };
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    p.revision = "2".into();
    p.workspaces.push(Workspace {
        id: workspace_id.clone(),
        name: "Scratch".into(),
        ..p.workspaces[0].clone()
    });
    p.panels.push(Panel {
        android_device_id: None,
        browser_generation: None,
        id: tab_id.clone(),
        tab_id: tab_id.clone(),
        workspace_id: workspace_id.clone(),
        kind: "file".into(),
        title: "Untitled-1".into(),
        terminal_session_id: None,
    });
    broker.publish(p.clone()).unwrap();
    broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce,
            ui_epoch: p.ui_epoch,
            result: OperationResult::Workspace {
                workspace_id: workspace_id.clone(),
                name: "Scratch".into(),
            },
        })
        .unwrap();
    let retry = creator.call(Request::CreateWorkspace(input)).await.unwrap();
    assert!(
        matches!(retry,Reply::Ok {data:Data::Operation {ref operation_id,ref state,..},..} if operation_id==&command.operation_id && state=="succeeded")
    );
    assert!(commands.try_recv().is_err());
    let list = WorkspaceListInput {
        workspace_id: workspace_id.clone(),
        limit: 100,
        cursor: None,
    };
    assert!(
        matches!(creator.call(Request::Panels(list.clone())).await.unwrap(),Reply::Ok {data:Data::Panels {ref items,..},..} if items.len()==1 && items[0].kind=="file")
    );
    assert!(matches!(
        reader.call(Request::Panels(list)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    let events = WorkspaceListInput {
        workspace_id: "a".into(),
        limit: 2,
        cursor: None,
    };
    let Reply::Ok {
        data:
            Data::Events {
                items,
                next_cursor,
                has_more,
                ..
            },
        ..
    } = creator.call(Request::Events(events.clone())).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert!(has_more);
    assert_eq!(items[0].state, "queued");
    assert_eq!(items[1].state, "running");
    let Reply::Ok {
        data: Data::Events {
            items, has_more, ..
        },
        ..
    } = creator
        .call(Request::Events(WorkspaceListInput {
            cursor: Some(next_cursor.clone()),
            ..events.clone()
        }))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].state, "succeeded");
    assert!(!has_more);
    assert!(
        matches!(reader.call(Request::Events(events.clone())).await.unwrap(),Reply::Ok {data:Data::Events {ref items,..},..} if items.is_empty())
    );
    assert!(matches!(
        reader
            .call(Request::Events(WorkspaceListInput {
                cursor: Some(next_cursor),
                ..events
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn terminal_claim_requires_a_current_explicit_decision_and_never_restores_a_released_lease() {
    use std::sync::Mutex;
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.terminal_profile = Some(TerminalProfile {
        id: "zsh".into(),
        revision: "qualified".into(),
    });
    p.panels.push(Panel {
        android_device_id: None,
        browser_generation: None,
        id: "human-panel".into(),
        tab_id: "human-tab".into(),
        workspace_id: "a".into(),
        kind: "terminal".into(),
        title: "Existing human shell".into(),
        terminal_session_id: Some("human-generation".into()),
    });
    broker.publish(p.clone()).unwrap();
    let monitors = Arc::new(Mutex::new(Vec::new()));
    let attached = monitors.clone();
    broker
        .set_terminal_attach_dispatch(Arc::new(move |generation, monitor, profile, _| {
            assert_eq!(generation, "human-generation");
            assert_eq!(profile.revision, "qualified");
            attached.lock().unwrap().push(monitor.clone());
            Ok(())
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "terminal.execute", "terminal.read"],
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
    let mut input = PanelControlInput {
        workspace_id: "a".into(),
        panel_id: "human-panel".into(),
        terminal_session_id: "human-generation".into(),
        action: PanelControlAction::Claim,
        retry_epoch,
        request_key: "pending-claim".into(),
    };
    let readonly = approved(&broker, &["a"]).await;
    assert!(matches!(
        readonly
            .call(Request::ControlPanel(input.clone().into()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let claim = client
        .call(Request::ControlPanel(input.clone().into()))
        .await
        .unwrap();
    let Reply::Ok {
        data:
            Data::Operation {
                operation_id,
                state,
                ..
            },
        ..
    } = claim
    else {
        panic!()
    };
    assert_eq!(state, "awaiting_user");
    assert!(monitors.lock().unwrap().is_empty());
    let duplicate = client
        .call(Request::ControlPanel(input.clone().into()))
        .await
        .unwrap();
    assert!(
        matches!(duplicate,Reply::Ok{data:Data::Operation{operation_id:ref id,..},..} if id==&operation_id)
    );
    let mut changed = input.clone();
    changed.action = PanelControlAction::Release;
    assert!(matches!(
        client
            .call(Request::ControlPanel(changed.into()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::IdempotencyConflict,
            ..
        }
    ));
    client
        .call(Request::CancelOperation(OperationInput {
            operation_id: operation_id.clone(),
        }))
        .await
        .unwrap();
    assert!(broker.decide_control(&operation_id, true).is_err());
    assert!(monitors.lock().unwrap().is_empty());
    input.request_key = "stale-generation-claim".into();
    let claim = client
        .call(Request::ControlPanel(input.clone().into()))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::Operation { operation_id, .. },
        ..
    } = claim
    else {
        panic!()
    };
    p.revision = "2".into();
    p.panels[0].terminal_session_id = Some("replacement-generation".into());
    broker.publish(p.clone()).unwrap();
    broker.decide_control(&operation_id, true).unwrap();
    assert!(monitors.lock().unwrap().is_empty());
    assert!(
        matches!(client.call(Request::Operation(OperationInput{operation_id}.into())).await.unwrap(),Reply::Ok{data:Data::Operation{state,..},..} if state=="cancelled")
    );
    p.revision = "3".into();
    p.panels[0].terminal_session_id = Some("human-generation".into());
    broker.publish(p.clone()).unwrap();
    input.request_key = "approved-claim".into();
    let claim = client
        .call(Request::ControlPanel(input.clone().into()))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::Operation { operation_id, .. },
        ..
    } = claim
    else {
        panic!()
    };
    broker.decide_control(&operation_id, true).unwrap();
    assert!(broker.decide_control(&operation_id, true).is_err());
    assert_eq!(monitors.lock().unwrap().len(), 1);
    let receipt = client
        .call(Request::ControlPanel(input.clone().into()))
        .await
        .unwrap();
    let Reply::Ok {
        data:
            Data::Operation {
                result:
                    Some(OperationResult::TerminalControl {
                        lease_id: Some(lease),
                        controlled: true,
                        ..
                    }),
                ..
            },
        ..
    } = receipt
    else {
        panic!("Expected live lease");
    };
    assert_eq!(
        monitors.lock().unwrap()[0].lock().unwrap().lease(),
        Some(lease.as_str())
    );
    input.action = PanelControlAction::Release;
    input.request_key = "release-claim".into();
    let released = client
        .call(Request::ControlPanel(input.clone().into()))
        .await
        .unwrap();
    assert!(
        matches!(released,Reply::Ok{data:Data::Operation{result:Some(OperationResult::TerminalControl{lease_id:None,controlled:false,..}),state,..},..} if state=="succeeded")
    );
    assert!(monitors.lock().unwrap()[0]
        .lock()
        .unwrap()
        .lease()
        .is_none());
    input.action = PanelControlAction::Claim;
    input.request_key = "revoked-claim".into();
    let claim = client
        .call(Request::ControlPanel(input.into()))
        .await
        .unwrap();
    let Reply::Ok {
        data:
            Data::Operation {
                operation_id,
                state,
                ..
            },
        ..
    } = claim
    else {
        panic!()
    };
    assert_eq!(state, "awaiting_user");
    broker.revoke();
    assert!(broker.decide_control(&operation_id, true).is_err());
    broker.shutdown().await;
    assert_eq!(monitors.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn browser_approval_requires_exact_origins_without_consuming_a_rejected_request() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, root.path())).unwrap();
    let endpoint = broker.endpoint.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let connecting = tokio::spawn(async move {
        Client::connect(&endpoint, "browser-grant-fixture", |id| {
            tx.send(id).unwrap();
        })
        .await
    });
    let id = rx.await.unwrap();
    let scopes = vec![
        "workspace.read".into(),
        "panel.create".into(),
        "browser.navigate".into(),
    ];
    for origins in [
        vec![],
        vec!["http://localhost:3000/path".into()],
        vec!["http://plugin.localhost".into()],
        vec!["https://*.example.com".into()],
    ] {
        assert!(broker
            .approve_policy(&id, &["a".into()], &scopes, &origins)
            .is_err());
        assert!(!connecting.is_finished());
        assert!(broker
            .overview()
            .unwrap()
            .pending
            .iter()
            .any(|p| p.id == id));
    }
    assert!(broker
        .approve_policy(
            &id,
            &["a".into()],
            &["workspace.read".into()],
            &["http://localhost:3000".into()]
        )
        .is_err());
    broker
        .approve_policy(
            &id,
            &["a".into()],
            &scopes,
            &[
                "https://EXAMPLE.com:443".into(),
                "http://localhost:3000/".into(),
            ],
        )
        .unwrap();
    let client = connecting.await.unwrap().unwrap();
    client
        .call(Request::Connect(ConnectInput {
            workspace_id: "a".into(),
        }))
        .await
        .unwrap();
    let overview = broker.overview().unwrap();
    assert_eq!(
        overview.sessions[0].browser_origins,
        vec!["https://example.com", "http://localhost:3000"]
    );
    drop(client);
    broker.shutdown().await;
}

#[tokio::test]
async fn browser_start_ticket_binds_the_approved_profile_and_never_persists_its_lease() {
    use lomi_control_core::broker::BrowserStart;
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let endpoint = broker.endpoint.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let connecting = tokio::spawn(async move {
        Client::connect(&endpoint, "browser-ticket", |id| {
            tx.send(id).unwrap();
        })
        .await
        .unwrap()
    });
    let id = rx.await.unwrap();
    broker
        .approve_policy(
            &id,
            &["a".into()],
            &[
                "workspace.read".into(),
                "panel.create".into(),
                "browser.navigate".into(),
            ],
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
        panic!("Expected connection");
    };
    let input = BrowserOpenInput {
        workspace_id: "a".into(),
        url: "http://localhost:3000/test".into(),
        visible: true,
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "open".into(),
    };
    let opened = client
        .call(Request::OpenBrowser(input.clone()))
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
        panic!("Expected browser command");
    };
    let start = || {
        broker.authorize_browser_start(BrowserStart {
            visible: true,
            operation: &command.operation_id,
            nonce: &command.nonce,
            panel: panel_id,
            generation: browser_generation,
            profile: profile_id,
            url,
        })
    };
    assert!(
        start().is_err(),
        "A queued operation cannot construct a view"
    );
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    assert!(broker
        .authorize_browser_start(BrowserStart {
            visible: true,
            operation: &command.operation_id,
            nonce: &command.nonce,
            panel: panel_id,
            generation: browser_generation,
            profile: "00000000000000000000000000000000",
            url
        })
        .is_err());
    assert!(
        broker
            .authorize_browser_start(BrowserStart {
                visible: false,
                operation: &command.operation_id,
                nonce: &command.nonce,
                panel: panel_id,
                generation: browser_generation,
                profile: profile_id,
                url,
            })
            .is_err(),
        "Native creation cannot change the approved visibility"
    );
    let control = start().unwrap();
    assert!(start().is_err(), "A ticket cannot create two native views");
    assert!(control.permits(url));
    assert!(!control.permits("http://localhost:3001/"));
    let lease = control.lease().unwrap().to_owned();
    control.mark_started();
    p.revision = "2".into();
    p.panels.push(Panel {
        android_device_id: None,
        id: panel_id.clone(),
        tab_id: panel_id.clone(),
        workspace_id: "a".into(),
        kind: "browser".into(),
        title: "Agent browser".into(),
        terminal_session_id: None,
        browser_generation: Some(browser_generation.clone()),
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
                navigation_id: "forged".into(),
                lease_id: Some("forged".into()),
                ready: false,
                engine: "forged".into(),
                network_isolation: "forged".into(),
            })),
        })
        .unwrap();
    let replay = client
        .call(Request::OpenBrowser(input.clone()))
        .await
        .unwrap();
    let Reply::Ok {
        data:
            Data::Operation {
                operation_id,
                result: Some(OperationResult::Browser(result)),
                ..
            },
        ..
    } = replay
    else {
        panic!("Expected native browser result: {opened:?}");
    };
    let BrowserResult {
        lease_id,
        engine,
        network_isolation,
        navigation_id,
        ready,
        ..
    } = *result;
    assert_eq!(operation_id, command.operation_id);
    assert_eq!(lease_id.as_deref(), Some(lease.as_str()));
    assert_eq!(engine, "WKWebView");
    assert_eq!(network_isolation, "none");
    assert!(ready);
    assert_eq!(navigation_id, control.navigation_id());
    let denied_capture = client
        .call(Request::ScreenshotBrowser(BrowserScreenshotInput {
            workspace_id: "a".into(),
            panel_id: panel_id.clone(),
            browser_generation: browser_generation.clone(),
            navigation_id: control.navigation_id(),
            max_width: 1280,
            max_bytes: 1024 * 1024,
        }))
        .await
        .unwrap();
    assert!(matches!(
        denied_capture,
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let denied_logs = client
        .call(Request::BrowserLogs(BrowserLogsInput {
            workspace_id: "a".into(),
            panel_id: panel_id.clone(),
            browser_generation: browser_generation.clone(),
            cursor: None,
            limit: 50,
        }))
        .await
        .unwrap();
    assert!(matches!(
        denied_logs,
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let denied_snapshot = client
        .call(Request::SnapshotBrowser(BrowserSnapshotInput {
            workspace_id: "a".into(),
            panel_id: panel_id.clone(),
            browser_generation: browser_generation.clone(),
            max_nodes: 100,
            max_bytes: 16384,
        }))
        .await
        .unwrap();
    assert!(matches!(
        denied_snapshot,
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let foreign_snapshot = client
        .call(Request::SnapshotBrowser(BrowserSnapshotInput {
            workspace_id: "b".into(),
            panel_id: panel_id.clone(),
            browser_generation: browser_generation.clone(),
            max_nodes: 100,
            max_bytes: 16384,
        }))
        .await
        .unwrap();
    assert!(matches!(
        foreign_snapshot,
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));

    control.document_loaded(url);
    let navigation_input = BrowserNavigateInput {
        workspace_id: "a".into(),
        panel_id: panel_id.clone(),
        browser_generation: browser_generation.clone(),
        lease_id: lease.clone(),
        url: "http://localhost:3000/pending".into(),
        wait_until: BrowserWaitUntil::Load,
        retry_epoch: input.retry_epoch.clone(),
        request_key: "native-cancel-navigation".into(),
    };
    client
        .call(Request::NavigateBrowser(navigation_input.clone()))
        .await
        .unwrap();
    let navigation = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &navigation.operation_id, &navigation.nonce)
        .unwrap();
    let dispatch = broker
        .authorize_browser_navigation(&navigation.operation_id, &navigation.nonce)
        .unwrap();
    assert!(broker
        .finish_browser_navigation(
            &navigation.operation_id,
            "wrong-nonce",
            Err(ErrorCode::DeadlineExceeded)
        )
        .is_err());
    dispatch.permit.check().unwrap();
    let cancelled = client
        .call(Request::CancelOperation(OperationInput {
            operation_id: navigation.operation_id.clone(),
        }))
        .await
        .unwrap();
    assert!(
        matches!(cancelled, Reply::Ok {data: Data::Operation {state, ..}, ..} if state == "cancelling")
    );
    assert_eq!(dispatch.permit.check(), Err(ErrorCode::ControlRevoked));
    control.fail_navigation(&navigation.operation_id, ErrorCode::ControlRevoked);
    broker
        .finish_browser_navigation(
            &navigation.operation_id,
            &navigation.nonce,
            Err(ErrorCode::ControlRevoked),
        )
        .unwrap();
    let replay = client
        .call(Request::NavigateBrowser(navigation_input))
        .await
        .unwrap();
    assert!(
        matches!(replay, Reply::Ok {data: Data::Operation {state, effect_state, operation_id, ..}, ..} if state == "outcome_unknown" && effect_state == "unknown" && operation_id == navigation.operation_id)
    );
    assert!(commands.try_recv().is_err());
    let hidden_input = BrowserOpenInput {
        visible: false,
        expected_revision: "2".into(),
        request_key: "cancel-hidden-creation".into(),
        ..input.clone()
    };
    client
        .call(Request::OpenBrowser(hidden_input.clone()))
        .await
        .unwrap();
    let hidden = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &hidden.operation_id, &hidden.nonce)
        .unwrap();
    let UiAction::CreateBrowser {
        panel_id: hidden_panel,
        browser_generation: hidden_generation,
        profile_id: hidden_profile,
        url: hidden_url,
        visible,
        ..
    } = &hidden.action
    else {
        panic!()
    };
    assert!(!visible);
    let hidden_control = broker
        .authorize_browser_start(BrowserStart {
            visible: false,
            operation: &hidden.operation_id,
            nonce: &hidden.nonce,
            panel: hidden_panel,
            generation: hidden_generation,
            profile: hidden_profile,
            url: hidden_url,
        })
        .unwrap();
    assert!(hidden_control.authorized());
    client
        .call(Request::CancelOperation(OperationInput {
            operation_id: hidden.operation_id.clone(),
        }))
        .await
        .unwrap();
    assert!(!hidden_control.authorized());
    hidden_control.mark_started();
    assert!(
        !hidden_control.native_navigation(hidden_url),
        "Cancelled creation cannot begin a late request"
    );
    broker
        .acknowledge_ui(UiAck {
            operation_id: hidden.operation_id.clone(),
            nonce: hidden.nonce,
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::Failure {
                code: ErrorCode::ControlRevoked,
            },
        })
        .unwrap();
    let repeated = client
        .call(Request::OpenBrowser(hidden_input))
        .await
        .unwrap();
    assert!(
        matches!(repeated, Reply::Ok { data: Data::Operation { operation_id, state, effect_state, .. }, .. } if operation_id == hidden.operation_id && state == "outcome_unknown" && effect_state == "unknown")
    );
    assert!(commands.try_recv().is_err());
    broker.revoke();
    assert!(control.lease().is_none());
    assert!(!control.native_navigation(url));
    broker.shutdown().await;
    let database = rusqlite::Connection::open_with_flags(
        root.path().join("control/control.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let stored: String = database
        .query_row(
            "SELECT result FROM receipts WHERE id=?1",
            [&command.operation_id],
            |row| row.get(0),
        )
        .unwrap();
    let stored: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert!(stored["leaseId"].is_null());
    assert_eq!(stored["browserGeneration"], *browser_generation);
}

#[tokio::test]
async fn workspace_selection_preserves_targets_and_rejects_lazy_stale_or_unapproved_work() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels = vec![Panel {
        android_device_id: None,
        id: "file-a".into(),
        tab_id: "file-a".into(),
        workspace_id: "a".into(),
        kind: "file".into(),
        title: "Scratch".into(),
        terminal_session_id: None,
        browser_generation: None,
    }];
    p.workspaces[0].active_panel_id = Some("file-a".into());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "workspace.write", "panel.focus"],
    )
    .await;
    let reader = approved_scopes(&broker, &["a"], &["workspace.read", "workspace.write"]).await;
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
    let mut input = WorkspaceSelectInput {
        action: WorkspaceSelectAction::Select,
        workspace_id: "a".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "select-file".into(),
    };
    let request =
        |input: WorkspaceSelectInput| Request::RenameWorkspace(WorkspaceUpdateInput::Select(input));
    assert!(matches!(
        reader.call(request(input.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let mut foreign = input.clone();
    foreign.workspace_id = "foreign".into();
    assert!(matches!(
        client.call(request(foreign)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    client.call(request(input.clone())).await.unwrap();
    let command = commands.recv().await.unwrap();
    assert!(
        matches!(&command.action, UiAction::SelectWorkspace { panel_id, .. } if panel_id == "file-a")
    );
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let ack = UiAck {
        operation_id: command.operation_id.clone(),
        nonce: command.nonce.clone(),
        ui_epoch: p.ui_epoch.clone(),
        result: OperationResult::Panel {
            workspace_id: "a".into(),
            panel_id: "file-a".into(),
            focused: true,
            closed: false,
        },
    };
    assert!(
        broker.acknowledge_ui(ack.clone()).is_err(),
        "Dispatch alone is not a successful selection"
    );
    p.revision = "2".into();
    p.focused_panel_id = Some("file-a".into());
    broker.publish(p.clone()).unwrap();
    broker.acknowledge_ui(ack).unwrap();
    let repeated = client.call(request(input.clone())).await.unwrap();
    assert!(
        matches!(repeated, Reply::Ok { data: Data::Operation { operation_id, state, .. }, .. } if operation_id == command.operation_id && state == "succeeded")
    );
    assert!(commands.try_recv().is_err());
    input.request_key = "select-stale".into();
    let stale = client.call(request(input.clone())).await.unwrap();
    assert!(matches!(
        stale,
        Reply::Ok {
            data: Data::Operation {
                result: Some(OperationResult::Failure {
                    code: ErrorCode::RevisionConflict
                }),
                ..
            },
            ..
        }
    ));
    input.expected_revision = "2".into();
    input.request_key = "select-before-change".into();
    client.call(request(input.clone())).await.unwrap();
    let pending = commands.recv().await.unwrap();
    p.revision = "3".into();
    p.panels[0].kind = "terminal".into();
    broker.publish(p.clone()).unwrap();
    assert!(broker
        .claim_ui(&p.ui_epoch, &pending.operation_id, &pending.nonce)
        .is_err());
    input.expected_revision = "3".into();
    input.request_key = "select-lazy".into();
    assert!(matches!(
        client.call(request(input)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    assert!(commands.try_recv().is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn android_metadata_requires_selected_devices_and_revalidates_disclosure() {
    const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
    const FOREIGN: &str = "00000000-0000-4000-8000-000000000002";
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, root.path())).unwrap();
    let reader = approved(&broker, &["a"]).await;
    let client = approved_domains(
        &broker,
        &["a"],
        &["workspace.read", "android.read"],
        &[DEVICE],
    )
    .await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    broker
        .set_android_list_dispatch(Arc::new(move |request| {
            request.check()?;
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(request.devices, vec![DEVICE.to_string()]);
            Ok(AndroidDevices {
                devices_revision: "4".into(),
                host_qualified: true,
                items: vec![AndroidDevice {
                    device_id: DEVICE.into(),
                    name: "Fixture".into(),
                    generation: None,
                    phase: AndroidPhase::Stopped,
                    process_alive: false,
                    display: None,
                }],
            })
        }))
        .unwrap();
    let input = AndroidListInput {
        workspace_id: "a".into(),
    };
    assert!(matches!(
        reader
            .call(Request::AndroidList(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    assert!(matches!(
        client
            .call(Request::AndroidList(AndroidListInput {
                workspace_id: "foreign".into()
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(
        matches!(client.call(Request::AndroidList(input.clone())).await.unwrap(), Reply::Ok { data: Data::AndroidDevices { devices, .. }, .. } if devices.items.len() == 1 && devices.items[0].device_id == DEVICE)
    );
    broker
        .set_android_list_dispatch(Arc::new(move |_| {
            Ok(AndroidDevices {
                devices_revision: "4".into(),
                host_qualified: true,
                items: vec![AndroidDevice {
                    device_id: FOREIGN.into(),
                    name: "Must not disclose".into(),
                    generation: None,
                    phase: AndroidPhase::Stopped,
                    process_alive: false,
                    display: None,
                }],
            })
        }))
        .unwrap();
    assert!(matches!(
        client
            .call(Request::AndroidList(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::OutcomeUnknown,
            ..
        }
    ));
    let weak = Arc::downgrade(&broker);
    broker
        .set_android_list_dispatch(Arc::new(move |_| {
            weak.upgrade().unwrap().revoke();
            Ok(AndroidDevices {
                devices_revision: "4".into(),
                host_qualified: true,
                items: vec![],
            })
        }))
        .unwrap();
    let revoked = client.call(Request::AndroidList(input)).await;
    assert!(
        revoked.is_err()
            || matches!(
                revoked,
                Ok(Reply::Error {
                    code: ErrorCode::ControlRevoked,
                    ..
                })
            )
    );
    broker.shutdown().await;
}

#[tokio::test]
async fn android_open_binds_selected_device_receipt_and_actual_projection() {
    const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    broker
        .set_android_list_dispatch(Arc::new(move |request| {
            request.check()?;
            Ok(AndroidDevices {
                devices_revision: "1".into(),
                host_qualified: true,
                items: vec![AndroidDevice {
                    device_id: DEVICE.into(),
                    name: "Phone".into(),
                    generation: None,
                    phase: AndroidPhase::Stopped,
                    process_alive: false,
                    display: None,
                }],
            })
        }))
        .unwrap();
    let client = approved_domains(
        &broker,
        &["a"],
        &["workspace.read", "panel.create", "android.read"],
        &[DEVICE],
    )
    .await;
    let reader = approved_domains(
        &broker,
        &["a"],
        &["workspace.read", "android.read"],
        &[DEVICE],
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
    let input = AndroidOpenInput {
        workspace_id: "a".into(),
        device_id: DEVICE.into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "open-android".into(),
    };
    assert!(matches!(
        reader
            .call(Request::AndroidOpen(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let foreign = AndroidOpenInput {
        device_id: "00000000-0000-4000-8000-000000000002".into(),
        ..input.clone()
    };
    assert!(matches!(
        client.call(Request::AndroidOpen(foreign)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    assert!(commands.try_recv().is_err());
    client
        .call(Request::AndroidOpen(input.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    let UiAction::CreateAndroid {
        panel_id,
        device_id,
        ..
    } = &command.action
    else {
        panic!()
    };
    assert_eq!(device_id, DEVICE);
    let ack = UiAck {
        operation_id: command.operation_id.clone(),
        nonce: command.nonce.clone(),
        ui_epoch: p.ui_epoch.clone(),
        result: OperationResult::AndroidPanel {
            workspace_id: "a".into(),
            panel_id: panel_id.clone(),
            device_id: DEVICE.into(),
        },
    };
    assert!(broker.acknowledge_ui(ack.clone()).is_err());
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    assert!(broker.acknowledge_ui(ack.clone()).is_err());
    p.revision = "2".into();
    p.panels.push(Panel {
        id: panel_id.clone(),
        tab_id: panel_id.clone(),
        workspace_id: "a".into(),
        kind: "android".into(),
        title: "Phone".into(),
        terminal_session_id: None,
        browser_generation: None,
        android_device_id: Some(DEVICE.into()),
    });
    broker.publish(p.clone()).unwrap();
    broker.acknowledge_ui(ack).unwrap();
    assert!(
        matches!(client.call(Request::AndroidOpen(input.clone())).await.unwrap(), Reply::Ok { data: Data::Operation { operation_id, state, .. }, .. } if operation_id == command.operation_id && state == "succeeded")
    );
    assert!(commands.try_recv().is_err());
    let stale = AndroidOpenInput {
        request_key: "stale-android-open".into(),
        ..input.clone()
    };
    assert!(matches!(
        client.call(Request::AndroidOpen(stale)).await.unwrap(),
        Reply::Ok {
            data: Data::Operation {
                result: Some(OperationResult::Failure {
                    code: ErrorCode::RevisionConflict
                }),
                ..
            },
            ..
        }
    ));
    let input = AndroidOpenInput {
        request_key: "revoke-android-open".into(),
        expected_revision: "2".into(),
        ..input
    };
    client.call(Request::AndroidOpen(input)).await.unwrap();
    let pending = commands.recv().await.unwrap();
    broker.revoke();
    assert!(broker
        .claim_ui(&p.ui_epoch, &pending.operation_id, &pending.nonce)
        .is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn android_runtime_requires_native_completion_and_preserves_generation_on_retry() {
    const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
    const GENERATION: &str = "00000000-0000-4000-8000-000000000002";
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels.push(Panel {
        id: "phone".into(),
        tab_id: "phone".into(),
        workspace_id: "a".into(),
        kind: "android".into(),
        title: "Phone".into(),
        android_device_id: Some(DEVICE.into()),
        terminal_session_id: None,
        browser_generation: None,
    });
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_domains(
        &broker,
        &["a"],
        &["workspace.read", "android.read", "android.control"],
        &[DEVICE],
    )
    .await;
    let reader = approved_domains(
        &broker,
        &["a"],
        &["workspace.read", "android.read"],
        &[DEVICE],
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
    let input = AndroidStartInput {
        workspace_id: "a".into(),
        panel_id: "phone".into(),
        device_id: DEVICE.into(),
        expected_revision: "1".into(),
        retry_epoch: retry_epoch.clone(),
        request_key: "start-phone".into(),
    };
    assert!(matches!(
        reader
            .call(Request::AndroidStart(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    client
        .call(Request::AndroidStart(input.clone()))
        .await
        .unwrap();
    let cmd = commands.recv().await.unwrap();
    assert!(broker
        .authorize_android_runtime(&cmd.operation_id, &cmd.nonce)
        .is_err());
    broker
        .claim_ui(&p.ui_epoch, &cmd.operation_id, &cmd.nonce)
        .unwrap();
    let runtime = broker
        .authorize_android_runtime(&cmd.operation_id, &cmd.nonce)
        .unwrap();
    assert!(broker
        .authorize_android_runtime(&cmd.operation_id, &cmd.nonce)
        .is_err());
    runtime.control.bind(GENERATION).unwrap();
    let result = AndroidRuntimeResult {
        workspace_id: "a".into(),
        device_id: DEVICE.into(),
        generation: GENERATION.into(),
        ready: true,
        stopped: false,
    };
    assert!(broker
        .acknowledge_ui(UiAck {
            operation_id: cmd.operation_id.clone(),
            nonce: cmd.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::AndroidRuntime(result.clone())
        })
        .is_err());
    broker
        .finish_android_runtime(&cmd.operation_id, &cmd.nonce, Ok(result.clone()))
        .unwrap();
    assert!(
        matches!(client.call(Request::AndroidStart(input)).await.unwrap(), Reply::Ok { data: Data::Operation { operation_id, state, .. }, .. } if operation_id == cmd.operation_id && state == "succeeded")
    );
    assert!(commands.try_recv().is_err());
    let mut stop = AndroidStopInput {
        workspace_id: "a".into(),
        panel_id: "phone".into(),
        device_id: DEVICE.into(),
        generation: DEVICE.into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "stop-phone".into(),
    };
    assert!(matches!(
        client
            .call(Request::AndroidStop(stop.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::StaleGeneration,
            ..
        }
    ));
    stop.generation = GENERATION.into();
    client
        .call(Request::AndroidStop(stop.clone()))
        .await
        .unwrap();
    let cmd = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &cmd.operation_id, &cmd.nonce)
        .unwrap();
    let dispatch = broker
        .authorize_android_runtime(&cmd.operation_id, &cmd.nonce)
        .unwrap();
    assert!(Arc::ptr_eq(&dispatch.control, &runtime.control));
    broker
        .finish_android_runtime(
            &cmd.operation_id,
            &cmd.nonce,
            Ok(AndroidRuntimeResult {
                ready: false,
                stopped: true,
                ..result
            }),
        )
        .unwrap();
    assert!(runtime.control.check().is_err());
    assert!(
        matches!(client.call(Request::AndroidStop(stop)).await.unwrap(), Reply::Ok { data: Data::Operation { operation_id, state, .. }, .. } if operation_id == cmd.operation_id && state == "succeeded")
    );
    assert!(commands.try_recv().is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn android_input_uses_one_native_lease_and_durable_sequence_receipts() {
    const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
    const GENERATION: &str = "00000000-0000-4000-8000-000000000002";
    const LEASE: &str = "00000000-0000-4000-8000-000000000003";
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels.push(Panel {
        id: "phone".into(),
        tab_id: "phone".into(),
        workspace_id: "a".into(),
        kind: "android".into(),
        title: "Phone".into(),
        android_device_id: Some(DEVICE.into()),
        terminal_session_id: None,
        browser_generation: None,
    });
    p.focused_panel_id = Some("phone".into());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_domains(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "android.read",
            "android.control",
            "android.interact",
        ],
        &[DEVICE],
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
    client
        .call(Request::AndroidStart(AndroidStartInput {
            workspace_id: "a".into(),
            panel_id: "phone".into(),
            device_id: DEVICE.into(),
            expected_revision: "1".into(),
            retry_epoch: retry_epoch.clone(),
            request_key: "boot".into(),
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
    assert!(matches!(
        client
            .call(Request::AndroidSnapshot(AndroidSnapshotInput {
                workspace_id: "a".into(),
                panel_id: "phone".into(),
                device_id: DEVICE.into(),
                generation: GENERATION.into(),
                max_nodes: 10,
                max_bytes: 4096
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let claim = AndroidControlInput {
        workspace_id: "a".into(),
        panel_id: "phone".into(),
        device_id: DEVICE.into(),
        generation: GENERATION.into(),
        action: PanelControlAction::Claim,
        expected_revision: "1".into(),
        retry_epoch: retry_epoch.clone(),
        request_key: "claim-input".into(),
    };
    client
        .call(Request::ControlPanel(PanelControlRequest::Android(claim)))
        .await
        .unwrap();
    let cmd = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &cmd.operation_id, &cmd.nonce)
        .unwrap();
    let claimed = broker
        .authorize_android_input(&cmd.operation_id, &cmd.nonce)
        .unwrap();
    let lease = claimed
        .control
        .acquire_input("phone", GENERATION, LEASE)
        .unwrap();
    broker
        .finish_android_input(
            &cmd.operation_id,
            &cmd.nonce,
            Ok(OperationResult::AndroidControl(AndroidControlResult {
                workspace_id: "a".into(),
                device_id: DEVICE.into(),
                generation: GENERATION.into(),
                controlled: true,
                lease_id: Some(LEASE.into()),
            })),
        )
        .unwrap();
    let reply = client
        .call(Request::Operation(
            OperationInput {
                operation_id: cmd.operation_id.clone(),
            }
            .into(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(reply, Reply::Ok { data: Data::Operation { result: Some(OperationResult::AndroidControl(result)), .. }, .. } if result.lease_id.as_deref() == Some(LEASE))
    );
    let input = AndroidInput {
        workspace_id: "a".into(),
        panel_id: "phone".into(),
        device_id: DEVICE.into(),
        generation: GENERATION.into(),
        lease_id: LEASE.into(),
        input_sequence: "1".into(),
        event: AndroidInputEvent::Text {
            text: "Zażółć 🙂".into(),
        },
        retry_epoch,
    };
    client
        .call(Request::AndroidInput(input.clone()))
        .await
        .unwrap();
    let cmd = commands.recv().await.unwrap();
    assert!(
        matches!(client.call(Request::AndroidInput(input.clone())).await.unwrap(), Reply::Ok { data: Data::Operation { operation_id, .. }, .. } if operation_id == cmd.operation_id)
    );
    let changed = AndroidInput {
        event: AndroidInputEvent::Text {
            text: "Different".into(),
        },
        ..input.clone()
    };
    assert!(matches!(
        client.call(Request::AndroidInput(changed)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::IdempotencyConflict,
            ..
        }
    ));
    let second = AndroidInput {
        input_sequence: "2".into(),
        ..input.clone()
    };
    assert!(matches!(
        client
            .call(Request::AndroidInput(second.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetBusy,
            ..
        }
    ));
    broker
        .claim_ui(&p.ui_epoch, &cmd.operation_id, &cmd.nonce)
        .unwrap();
    let dispatch = broker
        .authorize_android_input(&cmd.operation_id, &cmd.nonce)
        .unwrap();
    assert!(matches!(
        dispatch.action,
        lomi_control_core::broker::AndroidInputDispatchAction::Send { sequence: 1, .. }
    ));
    broker
        .finish_android_input(
            &cmd.operation_id,
            &cmd.nonce,
            Ok(OperationResult::AndroidInput(AndroidInputResult {
                workspace_id: "a".into(),
                device_id: DEVICE.into(),
                generation: GENERATION.into(),
                input_sequence: "1".into(),
            })),
        )
        .unwrap();
    assert!(lease.check().is_ok());
    client
        .call(Request::AndroidInput(second.clone()))
        .await
        .unwrap();
    let cancelled = commands.recv().await.unwrap();
    client
        .call(Request::CancelOperation(OperationInput {
            operation_id: cancelled.operation_id.clone(),
        }))
        .await
        .unwrap();
    assert!(lease.check().is_err());
    assert!(broker
        .authorize_android_input(&cancelled.operation_id, &cancelled.nonce)
        .is_err());
    assert!(matches!(
        client.call(Request::AndroidInput(second)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ControlRevoked,
            ..
        }
    ));
    assert!(commands.try_recv().is_err());
    let db = rusqlite::Connection::open_with_flags(
        root.path().join("control/control.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let persisted: String = db
        .query_row(
            "SELECT group_concat(request_key || coalesce(result, '')) FROM receipts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!persisted.contains(LEASE));
    assert!(!persisted.contains("Zażółć"));
    drop(db);
    broker.shutdown().await;
}

#[tokio::test]
async fn android_snapshot_checks_scope_identity_limits_and_revocation() {
    use std::sync::atomic::Ordering;
    const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
    const GENERATION: &str = "00000000-0000-4000-8000-000000000002";
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels.push(Panel {
        id: "phone".into(),
        tab_id: "phone".into(),
        workspace_id: "a".into(),
        kind: "android".into(),
        title: "Phone".into(),
        android_device_id: Some(DEVICE.into()),
        terminal_session_id: None,
        browser_generation: None,
    });
    p.focused_panel_id = Some("phone".into());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_domains(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "android.read",
            "android.control",
            "android.observe",
            "android.capture",
        ],
        &[DEVICE],
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
    client
        .call(Request::AndroidStart(AndroidStartInput {
            workspace_id: "a".into(),
            panel_id: "phone".into(),
            device_id: DEVICE.into(),
            expected_revision: "1".into(),
            retry_epoch: retry_epoch.clone(),
            request_key: "boot".into(),
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
    let input = AndroidSnapshotInput {
        workspace_id: "a".into(),
        panel_id: "phone".into(),
        device_id: DEVICE.into(),
        generation: GENERATION.into(),
        max_nodes: 10,
        max_bytes: 4096,
    };
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let mode = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let behavior = mode.clone();
    broker
        .set_android_snapshot_dispatch(Arc::new(move |control, i, snapshot_id, _deadline| {
            counted.fetch_add(1, Ordering::SeqCst);
            if behavior.load(Ordering::SeqCst) == 2 {
                control.revoke();
            }
            Ok(AndroidSnapshot {
                workspace_id: if behavior.load(Ordering::SeqCst) == 1 {
                    "foreign".into()
                } else {
                    i.workspace_id
                },
                panel_id: i.panel_id,
                device_id: i.device_id,
                generation: i.generation,
                snapshot_id,
                captured_at_millis: "1".into(),
                hardware_display: [720, 1280],
                rotation: 0,
                coordinate_space: "rotated_display".into(),
                nodes: vec![],
                truncated: false,
                limitations: vec![],
            })
        }))
        .unwrap();
    for (field, code) in [
        ("workspace", ErrorCode::TargetNotFound),
        ("panel", ErrorCode::TargetNotFound),
        ("generation", ErrorCode::StaleGeneration),
        ("limit", ErrorCode::ResourceExhausted),
    ] {
        let mut bad = input.clone();
        match field {
            "workspace" => bad.workspace_id = "foreign".into(),
            "panel" => bad.panel_id = "foreign".into(),
            "generation" => bad.generation = "00000000-0000-4000-8000-000000000099".into(),
            _ => bad.max_bytes = 999999,
        }
        assert!(
            matches!(client.call(Request::AndroidSnapshot(bad)).await.unwrap(), Reply::Error { code: actual, .. } if actual == code)
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let permit = runtime.control.begin_observation().unwrap();
    assert!(matches!(
        client
            .call(Request::AndroidSnapshot(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetBusy,
            ..
        }
    ));
    drop(permit);
    assert!(matches!(
        client
            .call(Request::AndroidSnapshot(input.clone()))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::AndroidSnapshot(_),
            ..
        }
    ));
    use base64::Engine;
    broker.set_android_capture_dispatch(Arc::new(|_control, _input, _deadline| {
        let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap();
        Ok(lomi_control_core::broker::AndroidCapture { bytes, geometry: AndroidImageGeometry { captured_at_millis: "1".into(), hardware_display: [720,1280], rotation: 1, pixel_width: 1, pixel_height: 1, capture_scale_x: 1./1280., capture_scale_y: 1./720., image_to_hardware: AndroidImageGeometry::transform([720,1280],1,1,1), coordinate_space: "image_pixels".into(), crop: "full_display".into() } })
    })).unwrap();
    let captured = client
        .call(Request::AndroidScreenshot(AndroidScreenshotInput {
            workspace_id: "a".into(),
            panel_id: "phone".into(),
            device_id: DEVICE.into(),
            generation: GENERATION.into(),
            max_edge: 64,
            max_bytes: 16384,
        }))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::Artifact { artifact, image },
        ..
    } = captured
    else {
        panic!("{captured:?}");
    };
    assert!(matches!(artifact.source, ArtifactSource::Android(_)));
    let reread = ArtifactReadInput {
        workspace_id: "a".into(),
        artifact_id: artifact.id.clone(),
    };
    assert!(
        matches!(client.call(Request::ReadArtifact(reread.clone())).await.unwrap(), Reply::Ok { data: Data::Artifact { image: other, .. }, .. } if other == image)
    );
    assert!(matches!(
        client
            .call(Request::ReadArtifact(ArtifactReadInput {
                workspace_id: "b".into(),
                artifact_id: artifact.id.clone()
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    mode.store(1, Ordering::SeqCst);
    assert!(matches!(
        client
            .call(Request::AndroidSnapshot(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::OutcomeUnknown,
            ..
        }
    ));
    mode.store(2, Ordering::SeqCst);
    assert!(matches!(
        client.call(Request::AndroidSnapshot(input)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ControlRevoked,
            ..
        }
    ));
    assert!(matches!(
        client.call(Request::ReadArtifact(reread)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ControlRevoked,
            ..
        }
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn apk_import_binds_approved_root_copy_hash_retry_and_native_completion() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("build")).unwrap();
    std::fs::write(root.path().join("build/app.apk"), b"private copied APK").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let p = projection(&broker, root.path());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(|_| std::io::Error::other("closed"))
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a", "b"],
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
    let input = ArtifactImportInput {
        workspace_id: "a".into(),
        relative_path: "build/app.apk".into(),
        kind: ArtifactImportKind::AndroidApk,
        expected_byte_length: 18,
        expected_sha256: format!("{:x}", Sha256::digest(b"private copied APK")),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "import-apk".into(),
    };
    let readonly = approved(&broker, &["a"]).await;
    assert!(matches!(
        readonly
            .call(Request::ImportArtifact(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let mut foreign = input.clone();
    foreign.workspace_id = "foreign".into();
    assert!(matches!(
        client.call(Request::ImportArtifact(foreign)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    for path in ["../app.apk", ".ssh/app.apk", "app.apk/../file.apk"] {
        let mut forbidden = input.clone();
        forbidden.relative_path = path.into();
        assert!(matches!(
            client
                .call(Request::ImportArtifact(forbidden))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
    }
    let first = client
        .call(Request::ImportArtifact(input.clone()))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::Operation { operation_id, .. },
        ..
    } = first
    else {
        panic!("{first:?}")
    };
    let command = commands.recv().await.unwrap();
    assert!(broker
        .execute_import(&operation_id, &command.nonce, |_, _| Ok(()))
        .is_err());
    broker
        .claim_ui(&p.ui_epoch, &operation_id, &command.nonce)
        .unwrap();
    assert!(broker
        .acknowledge_ui(UiAck {
            operation_id: operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::ArtifactImported {
                workspace_id: "a".into(),
                artifact_id: "forged".into(),
                sha256: input.expected_sha256.clone(),
                byte_length: 18
            }
        })
        .is_err());
    broker
        .execute_import(&operation_id, &command.nonce, |copy, check| {
            check()?;
            std::fs::write(root.path().join("build/app.apk"), b"new unapproved source").unwrap();
            let mut bytes = Vec::new();
            copy.try_clone().unwrap().read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"private copied APK");
            Ok(())
        })
        .unwrap();
    assert!(broker
        .execute_import(&operation_id, &command.nonce, |_, _| panic!("replayed"))
        .is_err());
    let result = client
        .call(Request::Operation(
            OperationInput {
                operation_id: operation_id.clone(),
            }
            .into(),
        ))
        .await
        .unwrap();
    let Reply::Ok {
        data:
            Data::Operation {
                state,
                result:
                    Some(OperationResult::ArtifactImported {
                        artifact_id,
                        sha256,
                        ..
                    }),
                ..
            },
        ..
    } = result
    else {
        panic!("{result:?}")
    };
    assert_eq!(state, "succeeded");
    assert_eq!(sha256, input.expected_sha256);
    let read = ArtifactReadInput {
        workspace_id: "a".into(),
        artifact_id: artifact_id.clone(),
    };
    assert!(matches!(
        client
            .call(Request::ReadArtifact(read.clone()))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::Artifact { image: None, .. },
            ..
        }
    ));
    assert!(matches!(
        readonly
            .call(Request::ReadArtifact(read.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert!(matches!(
        client
            .call(Request::ReadArtifact(ArtifactReadInput {
                workspace_id: "b".into(),
                ..read
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert!(
        matches!(client.call(Request::ImportArtifact(input.clone())).await.unwrap(),Reply::Ok { data:Data::Operation { operation_id:id,.. },.. } if id==operation_id)
    );
    assert!(commands.try_recv().is_err());
    let mut changed = input.clone();
    changed.expected_sha256 = "0".repeat(64);
    assert!(matches!(
        client.call(Request::ImportArtifact(changed)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::IdempotencyConflict,
            ..
        }
    ));
    let mut mismatch = input.clone();
    mismatch.request_key = "bad-hash".into();
    client
        .call(Request::ImportArtifact(mismatch))
        .await
        .unwrap();
    let bad = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &bad.operation_id, &bad.nonce)
        .unwrap();
    broker
        .execute_import(&bad.operation_id, &bad.nonce, |_, _| {
            panic!("changed source must fail first")
        })
        .unwrap();
    assert!(
        matches!(client.call(Request::Operation(OperationInput { operation_id:bad.operation_id }.into())).await.unwrap(),Reply::Ok { data:Data::Operation { state,result:Some(OperationResult::Failure { code:ErrorCode::RevisionConflict }),.. },.. } if state=="failed")
    );
    let mut cancelled = input.clone();
    cancelled.request_key = "cancel-copy".into();
    std::fs::write(root.path().join("build/app.apk"), b"private copied APK").unwrap();
    client
        .call(Request::ImportArtifact(cancelled))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let (started, ready) = std::sync::mpsc::channel();
    let b = broker.clone();
    let c = command.clone();
    let task = tokio::task::spawn_blocking(move || {
        b.execute_import(&c.operation_id, &c.nonce, |_, check| {
            started.send(()).unwrap();
            for _ in 0..200 {
                check()?;
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("cancellation did not interrupt import");
        })
    });
    tokio::task::spawn_blocking(move || ready.recv_timeout(Duration::from_secs(2)))
        .await
        .unwrap()
        .unwrap();
    client
        .call(Request::CancelOperation(OperationInput {
            operation_id: command.operation_id.clone(),
        }))
        .await
        .unwrap();
    task.await.unwrap().unwrap();
    assert!(
        matches!(client.call(Request::Operation(OperationInput { operation_id:command.operation_id }.into())).await.unwrap(),Reply::Ok { data:Data::Operation { state,.. },.. } if state=="cancelled")
    );
    assert_eq!(
        std::fs::read_dir(root.path().join("control/artifacts"))
            .unwrap()
            .count(),
        1
    );
    broker.shutdown().await;
}

#[tokio::test]
async fn apk_install_requires_exact_copy_native_approval_and_single_dispatch() {
    use sha2::{Digest, Sha256};
    use std::{
        io::Read,
        sync::atomic::{AtomicUsize, Ordering},
    };
    const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
    const GENERATION: &str = "00000000-0000-4000-8000-000000000002";
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("app.apk"), b"private APK").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels.push(Panel {
        id: "phone".into(),
        tab_id: "phone".into(),
        workspace_id: "a".into(),
        kind: "android".into(),
        title: "Fixture phone".into(),
        android_device_id: Some(DEVICE.into()),
        terminal_session_id: None,
        browser_generation: None,
    });
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_domains(
        &broker,
        &["a", "b"],
        &[
            "workspace.read",
            "files.read",
            "artifact.import",
            "android.read",
            "android.control",
            "android.install",
        ],
        &[DEVICE],
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
    client
        .call(Request::ImportArtifact(ArtifactImportInput {
            workspace_id: "a".into(),
            relative_path: "app.apk".into(),
            kind: ArtifactImportKind::AndroidApk,
            expected_byte_length: 11,
            expected_sha256: format!("{:x}", Sha256::digest(b"private APK")),
            expected_revision: "1".into(),
            retry_epoch: retry_epoch.clone(),
            request_key: "copy".into(),
        }))
        .await
        .unwrap();
    let c = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &c.operation_id, &c.nonce)
        .unwrap();
    broker
        .execute_import(&c.operation_id, &c.nonce, |_, _| Ok(()))
        .unwrap();
    let Reply::Ok {
        data:
            Data::Operation {
                result:
                    Some(OperationResult::ArtifactImported {
                        artifact_id,
                        sha256,
                        ..
                    }),
                ..
            },
        ..
    } = client
        .call(Request::Operation(
            OperationInput {
                operation_id: c.operation_id,
            }
            .into(),
        ))
        .await
        .unwrap()
    else {
        panic!()
    };
    client
        .call(Request::AndroidStart(AndroidStartInput {
            workspace_id: "a".into(),
            panel_id: "phone".into(),
            device_id: DEVICE.into(),
            expected_revision: "1".into(),
            retry_epoch: retry_epoch.clone(),
            request_key: "boot".into(),
        }))
        .await
        .unwrap();
    let c = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &c.operation_id, &c.nonce)
        .unwrap();
    let runtime = broker
        .authorize_android_runtime(&c.operation_id, &c.nonce)
        .unwrap();
    runtime.control.bind(GENERATION).unwrap();
    broker
        .finish_android_runtime(
            &c.operation_id,
            &c.nonce,
            Ok(AndroidRuntimeResult {
                workspace_id: "a".into(),
                device_id: DEVICE.into(),
                generation: GENERATION.into(),
                ready: true,
                stopped: false,
            }),
        )
        .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    broker
        .set_android_install_dispatch(Arc::new(move |mut r| {
            let p = r.permit.clone();
            let c = r.control.clone();
            let g = r.input.generation.clone();
            r.file.verify(|| {
                p.check()?;
                c.check_generation(&g)
            })?;
            let mut bytes = Vec::new();
            r.file.file.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"private APK");
            r.mark_dispatching()?;
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(AndroidInstallResult {
                workspace_id: r.input.workspace_id,
                device_id: r.input.device_id,
                generation: r.input.generation,
                artifact_id: r.input.artifact_id,
                sha256: r.input.sha256,
                installed: true,
                package_name: Some("org.lomi.fixture".into()),
                previous_version: None,
                new_version: Some("1".into()),
                installer_failure: None,
            })
        }))
        .unwrap();
    let mut input = AndroidInstallInput {
        workspace_id: "a".into(),
        panel_id: "phone".into(),
        device_id: DEVICE.into(),
        generation: GENERATION.into(),
        artifact_id: artifact_id.clone(),
        sha256,
        retry_epoch,
        request_key: "install-denied".into(),
    };
    let readonly = approved(&broker, &["a"]).await;
    assert!(matches!(
        readonly
            .call(Request::AndroidInstall(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let mut wrong = input.clone();
    wrong.sha256 = "0".repeat(64);
    assert!(matches!(
        client.call(Request::AndroidInstall(wrong)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::RevisionConflict,
            ..
        }
    ));
    let Reply::Ok {
        data:
            Data::Operation {
                operation_id,
                state,
                ..
            },
        ..
    } = client
        .call(Request::AndroidInstall(input.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(state, "awaiting_user");
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(broker.overview().unwrap().pending_installs.len(), 1);
    assert!(
        matches!(client.call(Request::AndroidInstall(input.clone())).await.unwrap(),Reply::Ok{data:Data::Operation{operation_id:id,..},..} if id==operation_id)
    );
    broker.decide_install(&operation_id, false).unwrap();
    assert!(broker.decide_install(&operation_id, true).is_err());
    assert!(
        matches!(client.call(Request::AndroidInstall(input.clone())).await.unwrap(),Reply::Ok{data:Data::Operation{state,..},..} if state=="cancelled")
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    input.request_key = "install-approve".into();
    let Reply::Ok {
        data: Data::Operation { operation_id, .. },
        ..
    } = client
        .call(Request::AndroidInstall(input.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    std::fs::write(root.path().join("app.apk"), b"source replacement").unwrap();
    broker.decide_install(&operation_id, true).unwrap();
    assert!(broker.decide_install(&operation_id, true).is_err());
    for _ in 0..100 {
        let done = client
            .call(Request::Operation(
                OperationInput {
                    operation_id: operation_id.clone(),
                }
                .into(),
            ))
            .await
            .unwrap();
        if matches!(done,Reply::Ok{data:Data::Operation{state,..},..} if state=="succeeded") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        matches!(client.call(Request::AndroidInstall(input.clone())).await.unwrap(),Reply::Ok{data:Data::Operation{state,result:Some(OperationResult::AndroidInstall(_)),..},..} if state=="succeeded")
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(broker.overview().unwrap().pending_installs.is_empty());
    input.request_key = "install-cancel".into();
    let Reply::Ok {
        data: Data::Operation { operation_id, .. },
        ..
    } = client
        .call(Request::AndroidInstall(input.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    client
        .call(Request::CancelOperation(OperationInput {
            operation_id: operation_id.clone(),
        }))
        .await
        .unwrap();
    assert!(broker.decide_install(&operation_id, true).is_err());
    assert!(broker.overview().unwrap().pending_installs.is_empty());
    input.request_key = "changed-private-copy".into();
    let Reply::Ok {
        data: Data::Operation { operation_id, .. },
        ..
    } = client
        .call(Request::AndroidInstall(input.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    std::fs::write(
        root.path()
            .join("control/artifacts")
            .join(format!("{artifact_id}.apk")),
        b"changed APK",
    )
    .unwrap();
    broker.decide_install(&operation_id, true).unwrap();
    for _ in 0..100 {
        let done = client
            .call(Request::Operation(
                OperationInput {
                    operation_id: operation_id.clone(),
                }
                .into(),
            ))
            .await
            .unwrap();
        if matches!(done,Reply::Ok{data:Data::Operation{state,..},..} if state=="failed") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        matches!(client.call(Request::AndroidInstall(input.clone())).await.unwrap(),Reply::Ok{data:Data::Operation{state,effect_state,..},..} if state=="failed" && effect_state=="none")
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    input.request_key = "stale-approved-generation".into();
    let Reply::Ok {
        data: Data::Operation { operation_id, .. },
        ..
    } = client.call(Request::AndroidInstall(input)).await.unwrap()
    else {
        panic!()
    };
    runtime.control.revoke();
    broker.decide_install(&operation_id, true).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    broker.shutdown().await;
}

#[tokio::test]
async fn android_apps_bind_package_generation_launch_receipt_and_log_pages() {
    use lomi_control_core::broker::AndroidLogBatch;
    use std::sync::atomic::{AtomicUsize, Ordering};
    const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
    const GEN: &str = "00000000-0000-4000-8000-000000000002";
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels.push(Panel {
        id: "phone".into(),
        tab_id: "phone".into(),
        workspace_id: "a".into(),
        kind: "android".into(),
        title: "Phone".into(),
        android_device_id: Some(DEVICE.into()),
        terminal_session_id: None,
        browser_generation: None,
    });
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| {
            send.send(c).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_apps(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "android.read",
            "android.control",
            "android.launch",
            "android.logs",
        ],
        &[DEVICE],
        &["org.example.app"],
    )
    .await;
    let reader = approved_domains(
        &broker,
        &["a"],
        &["workspace.read", "android.read", "android.control"],
        &[DEVICE],
    )
    .await;
    let connected = client
        .call(Request::Connect(ConnectInput {
            workspace_id: "a".into(),
        }))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::Connected { retry_epoch, .. },
        ..
    } = connected
    else {
        panic!()
    };
    client
        .call(Request::AndroidStart(AndroidStartInput {
            workspace_id: "a".into(),
            panel_id: "phone".into(),
            device_id: DEVICE.into(),
            expected_revision: "1".into(),
            retry_epoch: retry_epoch.clone(),
            request_key: "start".into(),
        }))
        .await
        .unwrap();
    let cmd = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &cmd.operation_id, &cmd.nonce)
        .unwrap();
    let runtime = broker
        .authorize_android_runtime(&cmd.operation_id, &cmd.nonce)
        .unwrap();
    runtime.control.bind(GEN).unwrap();
    broker
        .finish_android_runtime(
            &cmd.operation_id,
            &cmd.nonce,
            Ok(AndroidRuntimeResult {
                workspace_id: "a".into(),
                device_id: DEVICE.into(),
                generation: GEN.into(),
                ready: true,
                stopped: false,
            }),
        )
        .unwrap();
    let input = AndroidLaunchInput {
        workspace_id: "a".into(),
        panel_id: "phone".into(),
        device_id: DEVICE.into(),
        generation: GEN.into(),
        package_name: "org.example.app".into(),
        activity: Some(".Main$Nested".into()),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "launch".into(),
    };
    let mut bad = input.clone();
    bad.package_name = "org.other.app".into();
    assert!(matches!(
        client.call(Request::AndroidLaunch(bad)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let mut bad = input.clone();
    bad.activity = Some(".Main'; exit 0".into());
    assert!(matches!(
        client.call(Request::AndroidLaunch(bad)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ResourceExhausted,
            ..
        }
    ));
    assert!(matches!(
        reader
            .call(Request::AndroidLaunch(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    client
        .call(Request::AndroidLaunch(input.clone()))
        .await
        .unwrap();
    let cmd = commands.recv().await.unwrap();
    assert!(broker
        .authorize_android_launch(&cmd.operation_id, &cmd.nonce)
        .is_err());
    broker
        .claim_ui(&p.ui_epoch, &cmd.operation_id, &cmd.nonce)
        .unwrap();
    let request = broker
        .authorize_android_launch(&cmd.operation_id, &cmd.nonce)
        .unwrap();
    request.permit.check().unwrap();
    assert!(broker
        .authorize_android_launch(&cmd.operation_id, &cmd.nonce)
        .is_err());
    let result = AndroidLaunchResult {
        workspace_id: "a".into(),
        device_id: DEVICE.into(),
        generation: GEN.into(),
        package_name: input.package_name.clone(),
        activity: input.activity.clone().unwrap(),
        intent_delivered: true,
    };
    assert!(broker
        .acknowledge_ui(UiAck {
            operation_id: cmd.operation_id.clone(),
            nonce: cmd.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::AndroidLaunch(result.clone())
        })
        .is_err());
    broker
        .finish_android_launch(&cmd.operation_id, &cmd.nonce, Ok(result))
        .unwrap();
    assert!(
        matches!(client.call(Request::AndroidLaunch(input.clone())).await.unwrap(), Reply::Ok { data: Data::Operation { state, operation_id, .. }, .. } if state == "succeeded" && operation_id == cmd.operation_id)
    );
    assert!(commands.try_recv().is_err());
    let mut changed = input.clone();
    changed.activity = None;
    assert!(matches!(
        client.call(Request::AndroidLaunch(changed)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::IdempotencyConflict,
            ..
        }
    ));
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    broker
        .set_android_logcat_dispatch(Arc::new(move |control, input, _| {
            control.check_generation(&input.generation)?;
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(AndroidLogBatch {
                process_id: 123,
                lines: vec!["first".into(), "second".into(), "third".into()],
                truncated: false,
            })
        }))
        .unwrap();
    let mut logs = AndroidLogcatInput {
        workspace_id: "a".into(),
        panel_id: "phone".into(),
        device_id: DEVICE.into(),
        generation: GEN.into(),
        package_name: "org.example.app".into(),
        min_priority: AndroidLogPriority::I,
        limit: 1,
        cursor: None,
    };
    let Reply::Ok {
        data: Data::AndroidLogcat(first),
        ..
    } = client
        .call(Request::AndroidLogcat(logs.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(first.lines, ["first"]);
    assert!(!first.complete);
    assert_eq!(first.gap, "unknown");
    logs.cursor = first.next_cursor;
    let Reply::Ok {
        data: Data::AndroidLogcat(second),
        ..
    } = client
        .call(Request::AndroidLogcat(logs.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(second.lines, ["second"]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut changed = logs.clone();
    changed.min_priority = AndroidLogPriority::E;
    assert!(matches!(
        client.call(Request::AndroidLogcat(changed)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    let mut changed = logs.clone();
    changed.package_name = "org.other.app".into();
    assert!(matches!(
        client.call(Request::AndroidLogcat(changed)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let mut changed = logs.clone();
    changed.generation = DEVICE.into();
    assert!(matches!(
        client.call(Request::AndroidLogcat(changed)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::StaleGeneration,
            ..
        }
    ));
    assert!(matches!(
        reader
            .call(Request::AndroidLogcat(logs.clone()))
            .await
            .unwrap(),
        Reply::Error { .. }
    ));
    let mut cancel = input;
    cancel.request_key = "cancel-launch".into();
    client.call(Request::AndroidLaunch(cancel)).await.unwrap();
    let pending = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &pending.operation_id, &pending.nonce)
        .unwrap();
    let request = broker
        .authorize_android_launch(&pending.operation_id, &pending.nonce)
        .unwrap();
    client
        .call(Request::CancelOperation(OperationInput {
            operation_id: pending.operation_id.clone(),
        }))
        .await
        .unwrap();
    assert!(request.permit.check().is_err());
    broker
        .finish_android_launch(
            &pending.operation_id,
            &pending.nonce,
            Err(ErrorCode::OutcomeUnknown),
        )
        .unwrap();
    runtime.control.revoke();
    assert!(matches!(
        client.call(Request::AndroidLogcat(logs)).await.unwrap(),
        Reply::Error { .. }
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn file_move_replays_after_source_disappears_and_requires_separate_scope() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("source.txt"), b"original").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let p = projection(&broker, &project);
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "files.mutate",
            "files.rename",
        ],
    )
    .await;
    let create_only = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "files.mutate",
            "files.create",
        ],
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
    let directory =
        lomi_control_core::project_files::ProjectDirectory::open(&project.canonicalize().unwrap())
            .unwrap();
    let mut input = FilesMutateInput {
        workspace_id: "a".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "move-file".into(),
        operation: FileMutation::Rename {
            relative_path: "source.txt".into(),
            new_name: "renamed.txt".into(),
            expected_disk_revision: format!("{:x}", Sha256::digest(b"original")),
            expected_parent_revision: directory.list("", || Ok(())).unwrap().revision,
        },
    };
    assert!(matches!(
        create_only
            .call(Request::FilesMutate(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let reply = client
        .call(Request::FilesMutate(input.clone()))
        .await
        .unwrap();
    assert!(
        matches!(
            reply,
            Reply::Ok {
                data: Data::Operation { .. },
                ..
            }
        ),
        "{reply:?}"
    );
    let command = tokio::time::timeout(std::time::Duration::from_secs(3), commands.recv())
        .await
        .unwrap()
        .unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let result = broker
        .commit_files_mutate(&command.operation_id, &command.nonce)
        .unwrap();
    assert_eq!(result.old_path.as_deref(), Some("source.txt"));
    assert!(!project.join("source.txt").exists());
    assert_eq!(
        std::fs::read(project.join("renamed.txt")).unwrap(),
        b"original"
    );
    broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce,
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::FilesMutated(result),
        })
        .unwrap();
    assert!(
        matches!(client.call(Request::FilesMutate(input.clone())).await.unwrap(),Reply::Ok{data:Data::Operation{operation_id,state,..},..} if operation_id == command.operation_id && state == "succeeded")
    );
    assert!(commands.try_recv().is_err());
    input.request_key = "escape".into();
    let FileMutation::Rename { new_name, .. } = &mut input.operation else {
        unreachable!()
    };
    *new_name = "nested/escape.txt".into();
    assert!(matches!(
        client.call(Request::FilesMutate(input)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn trash_requires_exact_main_decision_and_never_replays_a_staged_effect() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("file.txt"), b"preserve me").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let p = projection(&broker, &project);
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let destination = root.path().join("system-trash-fixture");
    let target = destination.clone();
    broker
        .set_files_trash_dispatch(Arc::new(move |path| {
            std::fs::rename(path, &target).map_err(|_| ErrorCode::StorageUnavailable)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "files.mutate",
            "files.trash",
        ],
    )
    .await;
    let denied = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "files.mutate",
            "files.rename",
        ],
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
    let directory =
        lomi_control_core::project_files::ProjectDirectory::open(&project.canonicalize().unwrap())
            .unwrap();
    let revision = format!("{:x}", Sha256::digest(b"preserve me"));
    let mut input = FilesMutateInput {
        workspace_id: "a".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "trash-cancel".into(),
        operation: FileMutation::Trash {
            relative_path: "file.txt".into(),
            kind: FileEntryKind::File,
            expected_entry_revision: revision.clone(),
            expected_parent_revision: directory.list("", || Ok(())).unwrap().revision,
        },
    };
    assert!(matches!(
        denied
            .call(Request::FilesMutate(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let buffers = vec![FileTrashBuffer {
        document_id: "document".into(),
        relative_path: "file.txt".into(),
        buffer_revision: "document:1".into(),
        disk_revision: revision,
    }];
    client
        .call(Request::FilesMutate(input.clone()))
        .await
        .unwrap();
    let command = tokio::time::timeout(std::time::Duration::from_secs(3), commands.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(broker
        .prepare_file_trash(&command.operation_id, &command.nonce, buffers.clone())
        .is_err());
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    assert!(broker
        .commit_files_mutate(&command.operation_id, &command.nonce)
        .is_err());
    let plan = broker
        .prepare_file_trash(&command.operation_id, &command.nonce, buffers.clone())
        .unwrap();
    assert!(plan.awaiting_user);
    assert!(broker.file_trash_pending(&command.operation_id, &command.nonce, &plan.plan_hash));
    assert!(broker
        .prepare_file_trash(&command.operation_id, &command.nonce, vec![])
        .is_err());
    assert!(broker
        .commit_files_mutate(&command.operation_id, &command.nonce)
        .is_err());
    assert!(broker
        .decide_file_trash(&command.operation_id, &command.nonce, "forged", true)
        .is_err());
    broker
        .decide_file_trash(
            &command.operation_id,
            &command.nonce,
            &plan.plan_hash,
            false,
        )
        .unwrap();
    assert!(!broker.file_trash_pending(&command.operation_id, &command.nonce, &plan.plan_hash));
    assert_eq!(
        std::fs::read(project.join("file.txt")).unwrap(),
        b"preserve me"
    );
    assert!(broker
        .decide_file_trash(&command.operation_id, &command.nonce, &plan.plan_hash, true)
        .is_err());
    input.request_key = "trash-approve".into();
    client
        .call(Request::FilesMutate(input.clone()))
        .await
        .unwrap();
    let command = tokio::time::timeout(std::time::Duration::from_secs(3), commands.recv())
        .await
        .unwrap()
        .unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let plan = broker
        .prepare_file_trash(&command.operation_id, &command.nonce, buffers)
        .unwrap();
    broker
        .decide_file_trash(&command.operation_id, &command.nonce, &plan.plan_hash, true)
        .unwrap();
    assert!(broker
        .decide_file_trash(&command.operation_id, &command.nonce, &plan.plan_hash, true)
        .is_err());
    let result = broker
        .commit_files_mutate(&command.operation_id, &command.nonce)
        .unwrap();
    assert_eq!(result.new_path, None);
    assert_eq!(std::fs::read(&destination).unwrap(), b"preserve me");
    assert!(broker
        .commit_files_mutate(&command.operation_id, &command.nonce)
        .is_err());
    broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch,
            result: OperationResult::FilesMutated(result),
        })
        .unwrap();
    assert!(
        matches!(client.call(Request::FilesMutate(input)).await.unwrap(), Reply::Ok { data:Data::Operation { operation_id, state, .. }, .. } if operation_id == command.operation_id && state == "succeeded")
    );
    assert!(commands.try_recv().is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn file_create_requires_distinct_scope_native_commit_and_exact_ack() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let p = projection(&broker, &project);
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
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
        ],
    )
    .await;
    let save_only = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "files.mutate",
            "editor.read",
            "editor.write",
        ],
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
    let directory =
        lomi_control_core::project_files::ProjectDirectory::open(&project.canonicalize().unwrap())
            .unwrap();
    let mut input = FilesMutateInput {
        workspace_id: "a".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "create-file".into(),
        operation: FileMutation::Create {
            relative_path: "new.txt".into(),
            kind: FileEntryKind::File,
            expected_parent_revision: directory.list("", || Ok(())).unwrap().revision,
        },
    };
    assert!(matches!(
        save_only
            .call(Request::FilesMutate(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let _ = client
        .call(Request::FilesMutate(input.clone()))
        .await
        .unwrap();
    let command = tokio::time::timeout(std::time::Duration::from_secs(3), commands.recv())
        .await
        .expect("create command dispatch")
        .unwrap();
    assert!(broker
        .commit_files_mutate(&command.operation_id, &command.nonce)
        .is_err());
    assert!(!project.join("new.txt").exists());
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let created = broker
        .commit_files_mutate(&command.operation_id, &command.nonce)
        .unwrap();
    assert_eq!(std::fs::read(project.join("new.txt")).unwrap(), b"");
    assert!(broker
        .commit_files_mutate(&command.operation_id, &command.nonce)
        .is_err());
    let mut forged = created.clone();
    forged.new_path = Some("forged.txt".into());
    let mut ack = UiAck {
        operation_id: command.operation_id.clone(),
        nonce: command.nonce.clone(),
        ui_epoch: p.ui_epoch.clone(),
        result: OperationResult::FilesMutated(forged),
    };
    assert!(broker.acknowledge_ui(ack.clone()).is_err());
    ack.result = OperationResult::FilesMutated(created);
    broker.acknowledge_ui(ack).unwrap();
    assert!(
        matches!(client.call(Request::FilesMutate(input.clone())).await.unwrap(), Reply::Ok {data: Data::Operation {operation_id,state,..},..} if operation_id == command.operation_id && state == "succeeded")
    );
    assert!(commands.try_recv().is_err());
    // A fresh request with stale directory state cannot publish another entry.
    input.request_key = "stale-parent".into();
    let FileMutation::Create { relative_path, .. } = &mut input.operation else {
        unreachable!()
    };
    *relative_path = "second.txt".into();
    let _ = client.call(Request::FilesMutate(input)).await.unwrap();
    let stale = tokio::time::timeout(std::time::Duration::from_secs(3), commands.recv())
        .await
        .expect("stale create command dispatch")
        .unwrap();
    broker
        .claim_ui(&p.ui_epoch, &stale.operation_id, &stale.nonce)
        .unwrap();
    assert_eq!(
        broker
            .commit_files_mutate(&stale.operation_id, &stale.nonce)
            .unwrap_err(),
        ErrorCode::RevisionConflict
    );
    assert!(!project.join("second.txt").exists());
    assert!(
        matches!(client.call(Request::Operation(OperationInput {operation_id:stale.operation_id}.into())).await.unwrap(), Reply::Ok {data:Data::Operation {state,effect_state,..},..} if state == "failed" && effect_state == "none")
    );
    broker.shutdown().await;
}

#[tokio::test]
async fn editor_save_binds_native_bytes_revisions_scope_and_one_use_receipt() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let path = project.join("file.txt");
    std::fs::write(&path, "disk").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, &project);
    p.panels.push(Panel {
        id: "editor".into(),
        tab_id: "editor".into(),
        workspace_id: "a".into(),
        kind: "file".into(),
        title: "file.txt".into(),
        terminal_session_id: None,
        browser_generation: None,
        android_device_id: None,
    });
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "files.mutate",
            "editor.read",
            "editor.write",
        ],
    )
    .await;
    let buffer_only = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "editor.read",
            "editor.write",
        ],
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
    let mut input = EditorSaveInput {
        workspace_id: "a".into(),
        panel_id: "editor".into(),
        relative_path: "file.txt".into(),
        document_id: "doc".into(),
        expected_buffer_revision: "doc:1".into(),
        expected_disk_revision: format!("{:x}", Sha256::digest(b"disk")),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "save-one".into(),
    };
    assert!(matches!(
        buffer_only
            .call(Request::EditorSave(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let _ = client
        .call(Request::EditorSave(input.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    let body = EditorSaveBody {
        document_id: "doc".into(),
        buffer_revision: "doc:1".into(),
        disk_revision: input.expected_disk_revision.clone(),
        source_path: path.canonicalize().unwrap().to_string_lossy().into(),
        content: "Zażółć 🙂\r\n".into(),
    };
    let encode = |_: &[u8], text: &str| Ok(text.as_bytes().to_vec());
    assert!(broker
        .commit_editor_save(&command.operation_id, &command.nonce, body.clone(), encode)
        .is_err());
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let mut wrong = body.clone();
    wrong.source_path = project.join("other.txt").to_string_lossy().into();
    assert_eq!(
        broker
            .commit_editor_save(&command.operation_id, &command.nonce, wrong, encode)
            .unwrap_err(),
        ErrorCode::RevisionConflict
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"disk");
    let saved = broker
        .commit_editor_save(&command.operation_id, &command.nonce, body.clone(), encode)
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), body.content.as_bytes());
    assert!(broker
        .commit_editor_save(&command.operation_id, &command.nonce, body.clone(), encode)
        .is_err());
    let mut forged = saved.clone();
    forged.disk_revision = "0".repeat(64);
    let mut ack = UiAck {
        operation_id: command.operation_id.clone(),
        nonce: command.nonce.clone(),
        ui_epoch: p.ui_epoch.clone(),
        result: OperationResult::EditorSaved(Box::new(forged)),
    };
    assert!(broker.acknowledge_ui(ack.clone()).is_err());
    ack.result = OperationResult::EditorSaved(Box::new(saved));
    broker.acknowledge_ui(ack).unwrap();
    assert!(
        matches!(client.call(Request::EditorSave(input.clone())).await.unwrap(),Reply::Ok { data:Data::Operation {operation_id,state,..},..} if operation_id==command.operation_id && state=="succeeded")
    );
    assert!(commands.try_recv().is_err());
    input.request_key = "stale-disk".into();
    let _ = client.call(Request::EditorSave(input)).await.unwrap();
    let stale = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &stale.operation_id, &stale.nonce)
        .unwrap();
    assert_eq!(
        broker
            .commit_editor_save(&stale.operation_id, &stale.nonce, body.clone(), encode)
            .unwrap_err(),
        ErrorCode::RevisionConflict
    );
    let receipt = client
        .call(Request::Operation(
            OperationInput {
                operation_id: stale.operation_id,
            }
            .into(),
        ))
        .await
        .unwrap();
    assert!(
        matches!(receipt,Reply::Ok {data:Data::Operation {state,effect_state,..},..} if state=="failed" && effect_state=="none")
    );
    assert_eq!(std::fs::read(&path).unwrap(), body.content.as_bytes());
    broker.shutdown().await;
}

#[tokio::test]
async fn editor_open_prepares_once_from_pinned_source_and_requires_domain_ack() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(project.join("nested")).unwrap();
    std::fs::write(project.join("nested/Zażółć 🙂.txt"), "prepared").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, &project);
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "editor.read",
            "panel.create",
            "panel.focus",
        ],
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
    let input = EditorOpenInput {
        workspace_id: "a".into(),
        relative_path: "nested/Zażółć 🙂.txt".into(),
        presentation: EditorPresentation::Editor,
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "editor-open".into(),
    };
    let _ = client
        .call(Request::EditorOpen(input.clone()))
        .await
        .unwrap();
    let command = tokio::time::timeout(std::time::Duration::from_secs(3), commands.recv())
        .await
        .expect("nested Unicode open dispatch")
        .unwrap();
    let decode = |_: &str, bytes: &[u8]| {
        Ok(PreparedEditorBody::Text {
            content: String::from_utf8(bytes.to_vec()).unwrap(),
            encoding: TextEncoding::Utf8,
        })
    };
    assert!(broker
        .prepare_editor_open(&command.operation_id, &command.nonce, decode)
        .is_err());
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let prepared = broker
        .prepare_editor_open(&command.operation_id, &command.nonce, decode)
        .unwrap();
    assert!(
        matches!(&prepared.body, PreparedEditorBody::Text { content, .. } if content == "prepared")
    );
    assert_eq!(
        prepared.path,
        project
            .canonicalize()
            .unwrap()
            .join("nested/Zażółć 🙂.txt")
            .to_string_lossy()
    );
    assert!(broker
        .prepare_editor_open(&command.operation_id, &command.nonce, decode)
        .is_err());
    std::fs::write(project.join("nested/Zażółć 🙂.txt"), "changed externally").unwrap();
    assert!(
        matches!(&prepared.body, PreparedEditorBody::Text { content, .. } if content == "prepared")
    );
    let ack = UiAck {
        operation_id: command.operation_id.clone(),
        ui_epoch: p.ui_epoch.clone(),
        nonce: command.nonce,
        result: OperationResult::EditorOpened(Box::new(EditorOpened {
            workspace_id: "a".into(),
            panel_id: "new-editor".into(),
            relative_path: "nested/Zażółć 🙂.txt".into(),
            document_id: "doc".into(),
            buffer_revision: "doc:0".into(),
            disk_revision: prepared.revision,
            dirty: false,
            presentation: EditorPresentation::Editor,
        })),
    };
    assert!(broker.acknowledge_ui(ack.clone()).is_err());
    p.panels.push(Panel {
        id: "new-editor".into(),
        tab_id: "new-editor".into(),
        workspace_id: "a".into(),
        kind: "file".into(),
        title: "nested/Zażółć 🙂.txt".into(),
        terminal_session_id: None,
        browser_generation: None,
        android_device_id: None,
    });
    p.revision = "2".into();
    broker.publish(p.clone()).unwrap();
    broker.acknowledge_ui(ack).unwrap();
    assert!(
        matches!(client.call(Request::EditorOpen(input.clone())).await.unwrap(), Reply::Ok { data: Data::Operation { operation_id, state, .. }, .. } if operation_id==command.operation_id && state=="succeeded")
    );
    assert!(commands.try_recv().is_err());
    let mut invalid = input;
    invalid.request_key = "unsupported-file".into();
    invalid.expected_revision = "2".into();
    let _ = client.call(Request::EditorOpen(invalid)).await.unwrap();
    let failed = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &failed.operation_id, &failed.nonce)
        .unwrap();
    assert_eq!(
        broker
            .prepare_editor_open(&failed.operation_id, &failed.nonce, |_, _| Err(
                ErrorCode::UnsupportedCapability
            ))
            .unwrap_err(),
        ErrorCode::UnsupportedCapability
    );
    let receipt = client
        .call(Request::Operation(OperationLookup::Id(OperationInput {
            operation_id: failed.operation_id,
        })))
        .await
        .unwrap();
    assert!(
        matches!(receipt, Reply::Ok { data: Data::Operation { state, effect_state, .. }, .. } if state=="failed" && effect_state=="none")
    );
    broker.shutdown().await;
}

#[tokio::test]
async fn editor_edits_reserve_once_validate_receipts_and_revoke_queued_work() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("file.txt"), "disk").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, &project);
    p.panels.push(Panel {
        id: "editor".into(),
        tab_id: "editor".into(),
        workspace_id: "a".into(),
        kind: "file".into(),
        title: "file.txt".into(),
        terminal_session_id: None,
        browser_generation: None,
        android_device_id: None,
    });
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "editor.read",
            "editor.write",
        ],
    )
    .await;
    let read_only = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "files.read", "editor.read"],
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
    let input = EditorEditsInput {
        workspace_id: "a".into(),
        panel_id: "editor".into(),
        relative_path: "file.txt".into(),
        document_id: "doc".into(),
        expected_buffer_revision: "doc:0".into(),
        expected_disk_revision: "a".repeat(64),
        edits: vec![EditorEdit {
            from_utf16: 0,
            to_utf16: 0,
            insert: "🙂".into(),
        }],
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "edit-one".into(),
    };
    assert!(matches!(
        read_only
            .call(Request::EditorEdits(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let Reply::Ok {
        data: Data::Operation { operation_id, .. },
        ..
    } = client
        .call(Request::EditorEdits(input.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    let command = commands.recv().await.unwrap();
    assert!(matches!(command.action, UiAction::EditorEdits(_)));
    let ack = UiAck {
        operation_id: operation_id.clone(),
        ui_epoch: p.ui_epoch.clone(),
        nonce: command.nonce.clone(),
        result: OperationResult::EditorEdited(Box::new(EditorEdited {
            workspace_id: "a".into(),
            panel_id: "editor".into(),
            relative_path: "file.txt".into(),
            document_id: "doc".into(),
            previous_buffer_revision: "doc:0".into(),
            buffer_revision: "doc:1".into(),
            disk_revision: "a".repeat(64),
            dirty: true,
            edit_count: 1,
        })),
    };
    assert!(broker.acknowledge_ui(ack.clone()).is_err());
    broker
        .claim_ui(&p.ui_epoch, &operation_id, &command.nonce)
        .unwrap();
    assert!(broker
        .claim_ui(&p.ui_epoch, &operation_id, &command.nonce)
        .is_err());
    let mut wrong_ack = ack.clone();
    if let OperationResult::EditorEdited(ref mut e) = wrong_ack.result {
        e.buffer_revision = "doc:20".into();
    }
    assert!(broker.acknowledge_ui(wrong_ack).is_err());
    broker.acknowledge_ui(ack).unwrap();
    assert!(
        matches!(client.call(Request::EditorEdits(input.clone())).await.unwrap(), Reply::Ok { data: Data::Operation { operation_id: id, state, .. }, .. } if id==operation_id && state=="succeeded")
    );
    assert!(commands.try_recv().is_err());
    let mut wrong = input.clone();
    wrong.edits[0].insert = "different".into();
    assert!(matches!(
        client.call(Request::EditorEdits(wrong)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::IdempotencyConflict,
            ..
        }
    ));
    for value in ["\r", "\0"] {
        let mut wrong = input.clone();
        wrong.edits[0].insert = value.into();
        assert!(matches!(
            client.call(Request::EditorEdits(wrong)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::ResourceExhausted,
                ..
            }
        ));
    }
    let mut overlap = input.clone();
    overlap.edits.push(overlap.edits[0].clone());
    assert!(matches!(
        client.call(Request::EditorEdits(overlap)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ResourceExhausted,
            ..
        }
    ));
    let mut queued = input;
    queued.request_key = "revoked-edit".into();
    let _ = client.call(Request::EditorEdits(queued)).await.unwrap();
    let queued = commands.recv().await.unwrap();
    broker.revoke();
    assert!(broker
        .claim_ui(&p.ui_epoch, &queued.operation_id, &queued.nonce)
        .is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn editor_read_binds_buffer_identity_source_and_scope_before_disclosure() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("file.txt"), "disk").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut projection = projection(&broker, &project);
    projection.panels.push(Panel {
        id: "editor".into(),
        tab_id: "editor".into(),
        workspace_id: "a".into(),
        kind: "file".into(),
        title: "file.txt".into(),
        terminal_session_id: None,
        browser_generation: None,
        android_device_id: None,
    });
    broker.publish(projection).unwrap();
    let reader = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "files.read", "editor.read"],
    )
    .await;
    let disk_only = approved_scopes(&broker, &["a"], &["workspace.read", "files.read"]).await;
    let forged_source = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let forge = forged_source.clone();
    let weak = Arc::downgrade(&broker);
    broker
        .set_editor_read_dispatch(Arc::new(move |request| {
            let source_path = std::path::Path::new(&request.project_path).join(
                if forge.load(std::sync::atomic::Ordering::SeqCst) {
                    ".env"
                } else {
                    "file.txt"
                },
            );
            let input = request.input;
            weak.upgrade().unwrap().editor_read_reply(EditorReadReply {
                request_id: request.request_id,
                ui_epoch: request.ui_epoch,
                source_path: Some(source_path.to_string_lossy().into()),
                error: None,
                text: Some(EditorText {
                    workspace_id: input.workspace_id,
                    panel_id: input.panel_id,
                    relative_path: input.relative_path,
                    document_id: "doc-1".into(),
                    buffer_revision: "doc-1:2".into(),
                    disk_revision: "a".repeat(64),
                    source: BufferSource::Buffer,
                    dirty: true,
                    conflict: false,
                    encoding: TextEncoding::Utf8,
                    line_endings: TextLineEndings::Lf,
                    content: "unsaved".into(),
                    start_utf16: 0,
                    total_utf16: 7,
                    next_utf16: None,
                    truncated: false,
                }),
            })
        }))
        .unwrap();
    let input = EditorReadInput {
        workspace_id: "a".into(),
        panel_id: "editor".into(),
        relative_path: "file.txt".into(),
        document_id: None,
        expected_buffer_revision: None,
        start_utf16: 0,
        max_chars: 4096,
    };
    assert!(matches!(
        disk_only
            .call(Request::EditorRead(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let Reply::Ok {
        data: Data::EditorText(text),
        ..
    } = reader
        .call(Request::EditorRead(input.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(text.content, "unsaved");
    assert!(text.dirty);
    let mut wrong = input.clone();
    wrong.expected_buffer_revision = Some("doc-1:1".into());
    assert!(matches!(
        reader.call(Request::EditorRead(wrong)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::RevisionConflict,
            ..
        }
    ));
    let mut wrong = input.clone();
    wrong.document_id = Some("old-document".into());
    assert!(matches!(
        reader.call(Request::EditorRead(wrong)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::StaleGeneration,
            ..
        }
    ));
    let mut wrong = input.clone();
    wrong.workspace_id = "b".into();
    assert!(matches!(
        reader.call(Request::EditorRead(wrong)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    let mut wrong = input.clone();
    wrong.relative_path = ".env".into();
    assert!(matches!(
        reader.call(Request::EditorRead(wrong)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    forged_source.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        reader
            .call(Request::EditorRead(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    forged_source.store(false, std::sync::atomic::Ordering::SeqCst);
    broker.revoke();
    assert!(!matches!(
        reader.call(Request::EditorRead(input)).await,
        Ok(Reply::Ok { .. })
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn search_cursors_bind_owner_query_root_and_revoke_cached_content() {
    use lomi_control_core::broker::FileSearchBatch;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, &project)).unwrap();
    let one = approved_scopes(&broker, &["a"], &["workspace.read", "files.read"]).await;
    let two = approved_scopes(&broker, &["a"], &["workspace.read", "files.read"]).await;
    let metadata = approved(&broker, &["a"]).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    broker
        .set_files_search_dispatch(Arc::new(move |_, _, check| {
            check()?;
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(FileSearchBatch {
                matches: (1..=3)
                    .map(|n| FileSearchMatch {
                        relative_path: "a.txt".into(),
                        disk_revision: "a".repeat(64),
                        line: n,
                        column: 1,
                        length: 4,
                        preview: format!("test {n}"),
                        preview_start_utf16: 0,
                    })
                    .collect(),
                files_scanned: 1,
                ..Default::default()
            })
        }))
        .unwrap();
    let mut input = FilesSearchInput {
        workspace_id: "a".into(),
        relative_directory: String::new(),
        query: FileSearchQuery {
            text: "test".into(),
            ..Default::default()
        },
        limit: 1,
        cursor: None,
    };
    assert!(matches!(
        metadata
            .call(Request::FilesSearch(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let Reply::Ok {
        data: Data::FilesSearch(first),
        ..
    } = one.call(Request::FilesSearch(input.clone())).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(first.matches[0].line, 1);
    input.cursor = first.next_cursor;
    assert!(matches!(
        two.call(Request::FilesSearch(input.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    for change in ["query", "directory", "workspace"] {
        let mut wrong = input.clone();
        match change {
            "query" => wrong.query.case_sensitive = true,
            "directory" => wrong.relative_directory = "src".into(),
            _ => wrong.workspace_id = "b".into(),
        }
        assert!(matches!(
            one.call(Request::FilesSearch(wrong)).await.unwrap(),
            Reply::Error { .. }
        ));
    }
    let Reply::Ok {
        data: Data::FilesSearch(next),
        ..
    } = one.call(Request::FilesSearch(input.clone())).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(next.matches[0].line, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    std::fs::rename(&project, root.path().join("old-project")).unwrap();
    std::fs::create_dir(&project).unwrap();
    assert!(matches!(
        one.call(Request::FilesSearch(input.clone())).await.unwrap(),
        Reply::Error { .. }
    ));
    broker.revoke();
    assert!(!matches!(
        one.call(Request::FilesSearch(input)).await,
        Ok(Reply::Ok { .. })
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn disk_reads_require_file_scope_revision_and_live_pinned_root() {
    use lomi_control_core::broker::DecodedFile;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("example.txt"), b"abcdef").unwrap();
    std::fs::write(project.join(".env.fixture"), b"PRIVATE").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, &project)).unwrap();
    let reader = approved_scopes(&broker, &["a"], &["workspace.read", "files.read"]).await;
    let metadata_only = approved(&broker, &["a"]).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    broker
        .set_files_read_dispatch(Arc::new(move |bytes, input, check| {
            check()?;
            counter.fetch_add(1, Ordering::SeqCst);
            let start = input.start_utf16 as usize;
            let end = (start + usize::from(input.max_chars)).min(bytes.len());
            Ok(DecodedFile {
                content: std::str::from_utf8(&bytes[start..end]).unwrap().into(),
                encoding: TextEncoding::Utf8,
                line_endings: TextLineEndings::None,
                total_utf16: bytes.len() as u32,
                next_utf16: (end < bytes.len()).then_some(end as u32),
            })
        }))
        .unwrap();
    let input = FilesReadInput {
        workspace_id: "a".into(),
        relative_path: "example.txt".into(),
        start_utf16: 0,
        max_chars: 3,
        expected_disk_revision: None,
    };
    assert!(matches!(
        metadata_only
            .call(Request::FilesRead(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    for (path, code) in [
        (".env.fixture", ErrorCode::ScopeDenied),
        ("../example.txt", ErrorCode::ScopeDenied),
        ("missing.txt", ErrorCode::TargetNotFound),
    ] {
        let mut wrong = input.clone();
        wrong.relative_path = path.into();
        assert!(
            matches!(reader.call(Request::FilesRead(wrong)).await.unwrap(), Reply::Error { code: actual, .. } if actual == code)
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let Reply::Ok {
        data: Data::FileText(first),
        ..
    } = reader
        .call(Request::FilesRead(input.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(first.content, "abc");
    assert_eq!(first.next_utf16, Some(3));
    assert!(first.truncated);
    let mut next = input.clone();
    next.start_utf16 = 3;
    assert!(matches!(
        reader.call(Request::FilesRead(next.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ResourceExhausted,
            ..
        }
    ));
    next.expected_disk_revision = Some(first.disk_revision.clone());
    let Reply::Ok {
        data: Data::FileText(second),
        ..
    } = reader.call(Request::FilesRead(next.clone())).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(second.content, "def");
    assert_eq!(second.next_utf16, None);
    assert!(!second.truncated);
    std::fs::write(project.join("example.txt"), b"changed").unwrap();
    assert!(matches!(
        reader.call(Request::FilesRead(next)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::RevisionConflict,
            ..
        }
    ));
    let mut foreign = input.clone();
    foreign.workspace_id = "foreign".into();
    assert!(matches!(
        reader.call(Request::FilesRead(foreign)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let weak = Arc::downgrade(&broker);
    broker
        .set_files_read_dispatch(Arc::new(move |_, _, _| {
            weak.upgrade().unwrap().revoke();
            Ok(DecodedFile {
                content: "never disclosed".into(),
                encoding: TextEncoding::Utf8,
                line_endings: TextLineEndings::None,
                total_utf16: 15,
                next_utf16: None,
            })
        }))
        .unwrap();
    assert!(matches!(
        reader.call(Request::FilesRead(input)).await,
        Err(_)
            | Ok(Reply::Error {
                code: ErrorCode::ControlRevoked,
                ..
            })
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn directory_pages_bind_owner_workspace_path_and_revision() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    for name in ["a.txt", "b.txt", "c.txt", ".env.private"] {
        std::fs::write(project.join(name), b"fixture").unwrap();
    }
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, &project)).unwrap();
    let one = approved_scopes(&broker, &["a", "b"], &["workspace.read", "files.read"]).await;
    let two = approved_scopes(&broker, &["a"], &["workspace.read", "files.read"]).await;
    let metadata = approved(&broker, &["a"]).await;
    let mut input = FilesListInput {
        workspace_id: "a".into(),
        relative_directory: "".into(),
        limit: 1,
        cursor: None,
    };
    assert!(matches!(
        metadata
            .call(Request::FilesList(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let Reply::Ok {
        data: Data::FilesList(first),
        ..
    } = one.call(Request::FilesList(input.clone())).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].name, "a.txt");
    assert!(first.filtered);
    input.cursor = first.next_cursor;
    assert!(matches!(
        two.call(Request::FilesList(input.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    let mut wrong = input.clone();
    wrong.workspace_id = "b".into();
    assert!(matches!(
        one.call(Request::FilesList(wrong)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    let mut wrong = input.clone();
    wrong.relative_directory = "subdir".into();
    assert!(matches!(
        one.call(Request::FilesList(wrong)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    let Reply::Ok {
        data: Data::FilesList(second),
        ..
    } = one.call(Request::FilesList(input.clone())).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(second.entries[0].name, "b.txt");
    assert_eq!(second.directory_revision, first.directory_revision);
    std::fs::write(project.join("d.txt"), b"new").unwrap();
    assert!(matches!(
        one.call(Request::FilesList(input)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    let all = FilesListInput {
        workspace_id: "a".into(),
        relative_directory: "".into(),
        limit: 200,
        cursor: None,
    };
    let Reply::Ok {
        data: Data::FilesList(all),
        ..
    } = one.call(Request::FilesList(all)).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(all.entries.len(), 4);
    assert_eq!(all.next_cursor, None);
    assert!(!serde_json::to_string(&all).unwrap().contains(".env"));
    broker.shutdown().await;
}

#[tokio::test]
async fn preview_assets_remain_pinned_scoped_bounded_and_revocable() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("readme.md"), "![fixture](pixel.png)").unwrap();
    std::fs::write(project.join("pixel.png"), "authorized image bytes").unwrap();
    std::fs::write(project.join(".env.png"), "private").unwrap();
    let outside = root.path().join("outside.png");
    std::fs::write(&outside, "outside").unwrap();
    std::os::unix::fs::symlink(&outside, project.join("link.png")).unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let p = projection(&broker, &project);
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "editor.read",
            "panel.create",
            "panel.focus",
        ],
    )
    .await;
    let connected = client
        .call(Request::Connect(ConnectInput {
            workspace_id: "a".into(),
        }))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::Connected { retry_epoch, .. },
        ..
    } = connected
    else {
        panic!()
    };
    let input = EditorOpenInput {
        workspace_id: "a".into(),
        relative_path: "readme.md".into(),
        presentation: EditorPresentation::Preview,
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "preview".into(),
    };
    client.call(Request::EditorOpen(input)).await.unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let prepared = broker
        .prepare_editor_open(&command.operation_id, &command.nonce, |_, bytes| {
            Ok(PreparedEditorBody::Text {
                content: String::from_utf8(bytes.to_vec()).unwrap(),
                encoding: TextEncoding::Utf8,
            })
        })
        .unwrap();
    let permit = prepared.asset_permit.unwrap();
    let decode = |_: &str, bytes: &[u8]| {
        assert_eq!(bytes, b"authorized image bytes");
        Ok(PreparedPreviewImage {
            data_base64: "cGl4ZWw=".into(),
            mime_type: "image/png".into(),
            width: 1,
            height: 1,
            original_width: 1,
            original_height: 1,
        })
    };
    assert!(broker
        .read_preview_asset("forged", "pixel.png", decode)
        .is_err());
    for path in [".env.png", "link.png", "../outside.png", "/outside.png"] {
        assert!(
            broker
                .read_preview_asset(&permit, path, |_, _| panic!("Unauthorized asset decoded"))
                .is_err(),
            "{path}"
        );
    }
    assert_eq!(
        broker
            .read_preview_asset(&permit, "pixel.png", decode)
            .unwrap()
            .width,
        1
    );
    // Swapping the approved root never redirects this transient main-only permit.
    let old_project = root.path().join("old-project");
    std::fs::rename(&project, &old_project).unwrap();
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("pixel.png"), "replacement").unwrap();
    assert!(broker
        .read_preview_asset(&permit, "pixel.png", |_, _| panic!(
            "Replacement root decoded"
        ))
        .is_err());
    std::fs::remove_file(project.join("pixel.png")).unwrap();
    std::fs::remove_dir(&project).unwrap();
    std::fs::rename(&old_project, &project).unwrap();
    let mut successful = 0;
    while broker
        .read_preview_asset(&permit, "pixel.png", decode)
        .is_ok()
    {
        successful += 1;
        assert!(successful < 64);
    }
    assert!(successful > 0);
    broker.release_preview_permit(&permit);
    assert_eq!(
        broker
            .read_preview_asset(&permit, "pixel.png", decode)
            .unwrap_err(),
        ErrorCode::ControlRevoked
    );
    // A second permit is tied to the same live policy, never a freestanding credential.
    let prepared_input = EditorOpenInput {
        workspace_id: "a".into(),
        relative_path: "readme.md".into(),
        presentation: EditorPresentation::Editor,
        expected_revision: "1".into(),
        retry_epoch: client
            .call(Request::Connect(ConnectInput {
                workspace_id: "a".into(),
            }))
            .await
            .ok()
            .and_then(|r| {
                if let Reply::Ok {
                    data: Data::Connected { retry_epoch, .. },
                    ..
                } = r
                {
                    Some(retry_epoch)
                } else {
                    None
                }
            })
            .unwrap(),
        request_key: "preview-revocation".into(),
    };
    client
        .call(Request::EditorOpen(prepared_input))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let permit = broker
        .prepare_editor_open(&command.operation_id, &command.nonce, |_, _| {
            Ok(PreparedEditorBody::Text {
                content: "test".into(),
                encoding: TextEncoding::Utf8,
            })
        })
        .unwrap()
        .asset_permit
        .unwrap();
    broker.revoke();
    assert!(broker
        .read_preview_asset(&permit, "pixel.png", |_, _| panic!("Revoked asset decoded"))
        .is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn revoking_a_prepared_clean_trash_records_no_effect_after_restart() {
    use lomi_control_core::{
        project_files::ProjectDirectory,
        receipts::{Effect, State, Store},
    };
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("file.txt"), b"preserve me").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let p = projection(&broker, &project);
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    broker
        .set_files_trash_dispatch(Arc::new(|_| panic!("Revoked trash reached the OS")))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "files.mutate",
            "files.trash",
        ],
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
    let pairing = broker.overview().unwrap().sessions[0].id.clone();
    let directory = ProjectDirectory::open(&project.canonicalize().unwrap()).unwrap();
    let input = FilesMutateInput {
        workspace_id: "a".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "prepared-clean-trash".into(),
        operation: FileMutation::Trash {
            relative_path: "file.txt".into(),
            kind: FileEntryKind::File,
            expected_entry_revision: format!("{:x}", Sha256::digest(b"preserve me")),
            expected_parent_revision: directory.list("", || Ok(())).unwrap().revision,
        },
    };
    client.call(Request::FilesMutate(input)).await.unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    assert!(
        !broker
            .prepare_file_trash(&command.operation_id, &command.nonce, vec![])
            .unwrap()
            .awaiting_user
    );
    broker.revoke();
    assert!(broker
        .commit_files_mutate(&command.operation_id, &command.nonce)
        .is_err());
    broker.shutdown().await;
    drop(client);
    drop(broker);
    let store = Store::open(&root.path().join("control"), 200).unwrap();
    let receipt = store.get(&pairing, "p", &command.operation_id).unwrap();
    assert_eq!(receipt.state, State::Cancelled);
    assert_eq!(receipt.effect_state, Effect::None);
    assert_eq!(
        std::fs::read(project.join("file.txt")).unwrap(),
        b"preserve me"
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[tokio::test]
async fn git_status_filters_paths_and_binds_immutable_pages_to_grants_and_project() {
    let _serial = GIT_FIXTURE_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let repository = project.join("repo");
    std::fs::create_dir(&repository).unwrap();
    let output = std::process::Command::new("/Library/Developer/CommandLineTools/usr/bin/git")
        .args(["init", "-q"])
        .arg(&repository)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success());
    for name in ["a.txt", "b.txt", "Zażółć.txt", ".env"] {
        std::fs::write(repository.join(name), "fixture").unwrap();
    }
    std::os::unix::fs::symlink("a.txt", repository.join("link.txt")).unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, &project)).unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "files.read", "git.read"],
    )
    .await;
    let denied = approved_scopes(&broker, &["a"], &["workspace.read", "files.read"]).await;
    let other = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "files.read", "git.read"],
    )
    .await;
    let mut input = GitStatusInput {
        workspace_id: "a".into(),
        repository_relative: "repo".into(),
        limit: 1,
        cursor: None,
    };
    assert!(matches!(
        denied
            .call(Request::GitStatus(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let first = client
        .call(Request::GitStatus(input.clone()))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::GitStatus(first),
        ..
    } = first
    else {
        panic!("{first:?}")
    };
    assert_eq!(first.changes.len(), 1);
    assert_eq!(first.omitted_entries, 2);
    assert_eq!(first.changes[0].relative_path, "repo/Zażółć.txt");
    assert!(first.git_version.starts_with("git version "));
    assert_eq!(first.observation_revision.len(), 64);
    input.cursor = first.next_cursor.clone();
    assert!(matches!(
        other.call(Request::GitStatus(input.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    let mut changed = input.clone();
    changed.repository_relative = "".into();
    assert!(matches!(
        client.call(Request::GitStatus(changed)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::CursorExpired,
            ..
        }
    ));
    changed = input.clone();
    changed.workspace_id = "foreign".into();
    assert!(matches!(
        client.call(Request::GitStatus(changed)).await.unwrap(),
        Reply::Error { .. }
    ));
    std::fs::write(repository.join("new-after-snapshot.txt"), "later").unwrap();
    input.limit = 200;
    let Reply::Ok {
        data: Data::GitStatus(next),
        ..
    } = client
        .call(Request::GitStatus(input.clone()))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(next.observation_revision, first.observation_revision);
    assert_eq!(
        next.changes
            .iter()
            .map(|c| c.relative_path.as_str())
            .collect::<Vec<_>>(),
        vec!["repo/a.txt", "repo/b.txt"]
    );
    assert_eq!(next.next_cursor, None);
    // Root replacement cannot turn a cached metadata page into another project.
    std::fs::rename(&project, root.path().join("old-project")).unwrap();
    std::fs::create_dir(&project).unwrap();
    assert!(matches!(
        client.call(Request::GitStatus(input)).await.unwrap(),
        Reply::Error { .. }
    ));
    broker.shutdown().await;
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
static GIT_FIXTURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[tokio::test]
async fn git_observations_preserve_exact_history_and_bound_authorized_content() {
    let _serial = GIT_FIXTURE_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("/Library/Developer/CommandLineTools/usr/bin/git")
            .arg("-C")
            .arg(&project)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", root.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(project.join("Zażółć.txt"), "base\n").unwrap();
    std::fs::write(project.join("binary.dat"), [0, 1, 2]).unwrap();
    std::os::unix::fs::symlink("outside-private-target", project.join("old-link")).unwrap();
    git(&["add", "--", "Zażółć.txt", "binary.dat", "old-link"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-qm",
        "First 🙂\n\nExact body\n",
    ]);
    let first = git(&["rev-parse", "HEAD"]).trim().to_string();
    std::fs::write(project.join("Zażółć.txt"), "committed\n").unwrap();
    git(&["add", "--", "Zażółć.txt"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-qm",
        "Second",
    ]);
    let second = git(&["rev-parse", "HEAD"]).trim().to_string();
    std::fs::write(project.join("Zażółć.txt"), "staged 🙂\n").unwrap();
    git(&["add", "--", "Zażółć.txt"]);
    std::fs::write(project.join("Zażółć.txt"), "working 🙂\n").unwrap();
    std::fs::write(project.join("binary.dat"), [0, 3, 2]).unwrap();
    std::fs::write(project.join("new.txt"), "untracked\n").unwrap();
    std::fs::write(project.join(".env"), "MCP_SECRET_FIXTURE").unwrap();
    std::fs::remove_file(project.join("old-link")).unwrap();
    git(&["config", "remote.origin.url", "https://fixture-user:fixture-password@example.invalid/private-token-path?secret=value#hidden"]);
    git(&[
        "config",
        "remote.origin.pushurl",
        "fixture-user@example.invalid:private-token-path",
    ]);
    git(&["config", "remote.custom.url", "ext::private-helper secret"]);
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, &project)).unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "files.read", "git.read"],
    )
    .await;
    let denied = approved_scopes(&broker, &["a"], &["workspace.read", "files.read"]).await;
    let mut input = GitDiffInput {
        workspace_id: "a".into(),
        repository_relative: "".into(),
        relative_path: "Zażółć.txt".into(),
        comparison: GitComparison::Worktree,
        commit: None,
        start_utf16: 0,
        max_chars: 2,
        expected_observation_revision: None,
    };
    assert!(matches!(
        denied.call(Request::GitDiff(input.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let mut output = String::new();
    let mut pages = 0;
    loop {
        let reply = client.call(Request::GitDiff(input.clone())).await.unwrap();
        let Reply::Ok {
            data: Data::GitDiff(page),
            ..
        } = reply
        else {
            panic!("{reply:?}");
        };
        output.push_str(&page.patch);
        pages += 1;
        assert!(page.patch.encode_utf16().count() <= 2);
        input.expected_observation_revision = Some(page.observation_revision);
        if let Some(next) = page.next_utf16 {
            input.start_utf16 = next;
        } else {
            break;
        }
        assert!(pages < 200);
    }
    assert!(
        output.contains("+working 🙂\n") && output.contains("-staged 🙂\n"),
        "{output}"
    );
    std::fs::write(project.join("Zażółć.txt"), "changed after observation\n").unwrap();
    assert!(matches!(
        client.call(Request::GitDiff(input.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::RevisionConflict,
            ..
        }
    ));
    input.start_utf16 = 0;
    input.max_chars = 8192;
    input.expected_observation_revision = None;
    input.comparison = GitComparison::Staged;
    let reply = client.call(Request::GitDiff(input.clone())).await.unwrap();
    let Reply::Ok {
        data: Data::GitDiff(page),
        ..
    } = reply
    else {
        panic!("{reply:?}");
    };
    assert!(page.patch.contains("+staged 🙂\n") && page.patch.contains("-committed\n"));
    input.comparison = GitComparison::Worktree;
    for path in [".env", "old-link", "../outside"] {
        input.relative_path = path.into();
        assert!(
            matches!(
                client.call(Request::GitDiff(input.clone())).await.unwrap(),
                Reply::Error { .. }
            ),
            "{path}"
        );
    }
    input.relative_path = ":(glob)**".into();
    let reply = client.call(Request::GitDiff(input.clone())).await.unwrap();
    let Reply::Ok {
        data: Data::GitDiff(page),
        ..
    } = reply
    else {
        panic!("{reply:?}");
    };
    assert!(
        page.patch.is_empty(),
        "A literal path was interpreted as a glob"
    );
    for (path, binary) in [("new.txt", false), ("binary.dat", true)] {
        input.relative_path = path.into();
        let reply = client.call(Request::GitDiff(input.clone())).await.unwrap();
        let Reply::Ok {
            data: Data::GitDiff(page),
            ..
        } = reply
        else {
            panic!("{reply:?}");
        };
        assert!(matches!(
            (page.notice, binary),
            (Some(GitDiffNotice::Binary), true) | (Some(GitDiffNotice::Untracked), false)
        ));
    }
    let mut history = GitHistoryInput {
        workspace_id: "a".into(),
        repository_relative: "".into(),
        commit: None,
        skip: 0,
        limit: 1,
    };
    let reply = client
        .call(Request::GitHistory(history.clone()))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::GitHistory(page),
        ..
    } = reply
    else {
        panic!("{reply:?}");
    };
    assert_eq!(page.start_commit, second);
    assert_eq!(page.commits[0].subject, "Second");
    history.commit = Some(page.start_commit);
    history.skip = page.next_skip.unwrap();
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-qm",
        "Later HEAD",
    ]);
    let reply = client.call(Request::GitHistory(history)).await.unwrap();
    let Reply::Ok {
        data: Data::GitHistory(page),
        ..
    } = reply
    else {
        panic!("{reply:?}");
    };
    assert_eq!(page.commits[0].commit, first);
    assert_eq!(page.commits[0].subject, "First 🙂");
    let reply = client
        .call(Request::GitCommit(GitCommitInput {
            files_skip: 0,
            files_limit: 100,
            workspace_id: "a".into(),
            repository_relative: "".into(),
            commit: first.clone(),
        }))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::GitCommit(commit),
        ..
    } = reply
    else {
        panic!("{reply:?}");
    };
    assert_eq!(commit.message, "First 🙂\n\nExact body\n");
    assert_eq!(commit.author_email, "fixture@example.invalid");
    let blob = git(&["rev-parse", "HEAD:Zażółć.txt"]).trim().to_string();
    assert!(matches!(
        client
            .call(Request::GitCommit(GitCommitInput {
                files_skip: 0,
                files_limit: 100,
                workspace_id: "a".into(),
                repository_relative: "".into(),
                commit: blob
            }))
            .await
            .unwrap(),
        Reply::Error { .. }
    ));
    let remote_input = GitRemotesInput {
        workspace_id: "a".into(),
        repository_relative: "".into(),
    };
    let reply = client
        .call(Request::GitRemotes(remote_input.clone()))
        .await
        .unwrap();
    let encoded = serde_json::to_string(&reply).unwrap();
    for secret in [
        "fixture-user",
        "fixture-password",
        "private-token-path",
        "secret=value",
        "hidden",
        "private-helper",
    ] {
        assert!(!encoded.contains(secret), "{encoded}");
    }
    let Reply::Ok {
        data: Data::GitRemotes(remotes),
        ..
    } = reply
    else {
        panic!("{reply:?}");
    };
    assert_eq!(remotes.remotes.len(), 3);
    assert!(remotes.remotes.iter().all(|r| r.location_redacted));
    assert_eq!(remotes.remotes[0].host.as_deref(), Some("example.invalid"));
    assert_eq!(remotes.remotes[1].transport, "ssh");
    assert_eq!(remotes.remotes[2].transport, "unsupported");
    let mut commit_input = GitCommitInput {
        files_skip: 0,
        files_limit: 1,
        workspace_id: "a".into(),
        repository_relative: "".into(),
        commit: first.clone(),
    };
    let first_file = client
        .call(Request::GitCommit(commit_input.clone()))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::GitCommit(first_file),
        ..
    } = first_file
    else {
        panic!("{first_file:?}");
    };
    assert_eq!(first_file.files.len(), 1);
    assert_eq!(first_file.omitted_entries, 1);
    commit_input.files_skip = first_file.next_files_skip.unwrap();
    let next_file = client
        .call(Request::GitCommit(commit_input.clone()))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::GitCommit(next_file),
        ..
    } = next_file
    else {
        panic!("{next_file:?}");
    };
    assert_eq!(next_file.files.len(), 1);
    assert!(next_file.next_files_skip.is_none());
    assert_ne!(
        first_file.files[0].relative_path,
        next_file.files[0].relative_path
    );
    assert_eq!(next_file.committer_email, "fixture@example.invalid");
    input.comparison = GitComparison::Commit;
    input.commit = Some(first.clone());
    input.relative_path = "Zażółć.txt".into();
    let initial_diff = client.call(Request::GitDiff(input.clone())).await.unwrap();
    let Reply::Ok {
        data: Data::GitDiff(initial_diff),
        ..
    } = initial_diff
    else {
        panic!("{initial_diff:?}");
    };
    assert!(initial_diff.patch.contains("+base\n"));
    input.relative_path = "old-link".into();
    assert!(matches!(
        client.call(Request::GitDiff(input.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let prior = git(&["rev-parse", "HEAD"]).trim().to_string();
    git(&[
        "restore",
        "--source=HEAD",
        "--worktree",
        "--",
        "Zażółć.txt",
        "binary.dat",
        "old-link",
    ]);
    git(&["checkout", "-qb", "fixture-side", &first]);
    std::fs::write(project.join("side.txt"), "side change\n").unwrap();
    git(&["add", "--", "side.txt"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-qm",
        "Side",
    ]);
    git(&["checkout", "--detach", &prior]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "merge",
        "--no-ff",
        "-m",
        "Merge fixture",
        "fixture-side",
    ]);
    let merge = git(&["rev-parse", "HEAD"]).trim().to_string();
    commit_input.commit = merge.clone();
    commit_input.files_skip = 0;
    let merged = client.call(Request::GitCommit(commit_input)).await.unwrap();
    let Reply::Ok {
        data: Data::GitCommit(merged),
        ..
    } = merged
    else {
        panic!("{merged:?}");
    };
    assert_eq!(merged.parents.len(), 2);
    assert_eq!(merged.parents[0], prior);
    assert_eq!(merged.files.len(), 1);
    assert_eq!(merged.files[0].relative_path, "side.txt");
    input.commit = Some(merge);
    input.relative_path = "side.txt".into();
    let merged_diff = client.call(Request::GitDiff(input)).await.unwrap();
    let Reply::Ok {
        data: Data::GitDiff(merged_diff),
        ..
    } = merged_diff
    else {
        panic!("{merged_diff:?}");
    };
    assert!(merged_diff.patch.contains("+side change\n"));
    broker.revoke();
    assert!(!matches!(
        client.call(Request::GitRemotes(remote_input)).await,
        Ok(Reply::Ok { .. })
    ));
    broker.shutdown().await;
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[tokio::test]
async fn git_views_prepare_once_require_exact_ack_and_keep_followup_reads_scoped() {
    let _serial = GIT_FIXTURE_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("/Library/Developer/CommandLineTools/usr/bin/git")
            .arg("-C")
            .arg(&project)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", root.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(project.join("a.txt"), "base\n").unwrap();
    std::fs::write(project.join(".env"), "secret\n").unwrap();
    git(&["add", "--", "a.txt", ".env"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-qm",
        "View fixture",
    ]);
    let commit = git(&["rev-parse", "HEAD"]).trim().to_string();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut projection = projection(&broker, &project);
    broker.publish(projection.clone()).unwrap();
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| tx.send(c).map_err(std::io::Error::other)))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "git.read",
            "panel.create",
            "panel.focus",
            "panel.close",
        ],
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
    let input = GitOpenInput {
        workspace_id: "a".into(),
        repository_relative: "".into(),
        view: GitView::Commit { commit },
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "git-view".into(),
    };
    let _ = client.call(Request::GitOpen(input.clone())).await.unwrap();
    let command = commands.recv().await.unwrap();
    assert!(broker
        .prepare_git_open(&command.operation_id, &command.nonce)
        .is_err());
    broker
        .claim_ui(&projection.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let prepared = broker
        .prepare_git_open(&command.operation_id, &command.nonce)
        .unwrap();
    let GitViewBody::Commit(details) = &prepared.body else {
        panic!()
    };
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.omitted_entries, 1);
    assert_eq!(
        prepared.root,
        project.canonicalize().unwrap().to_str().unwrap()
    );
    assert!(broker
        .prepare_git_open(&command.operation_id, &command.nonce)
        .is_err());
    let patch = broker
        .read_git_view(&prepared.permit_id, Some("a.txt"))
        .unwrap();
    assert!(matches!(patch, GitViewBody::Diff(p) if p.patch.contains("+base\n")));
    assert!(broker
        .read_git_view(&prepared.permit_id, Some(".env"))
        .is_err());
    assert!(broker.read_git_view("foreign", Some("a.txt")).is_err());
    let mut ack = UiAck {
        operation_id: command.operation_id.clone(),
        ui_epoch: projection.ui_epoch.clone(),
        nonce: command.nonce,
        result: OperationResult::GitOpened(Box::new(GitOpened {
            workspace_id: "a".into(),
            panel_id: "git-panel".into(),
            repository_relative: "".into(),
            view: input.view.clone(),
            observation_revision: prepared.observation_revision.clone(),
        })),
    };
    assert!(broker.acknowledge_ui(ack.clone()).is_err());
    projection.panels.push(Panel {
        id: "git-panel".into(),
        tab_id: "git-panel".into(),
        workspace_id: "a".into(),
        kind: "commit".into(),
        title: "fixture".into(),
        terminal_session_id: None,
        browser_generation: None,
        android_device_id: None,
    });
    projection.revision = "2".into();
    broker.publish(projection.clone()).unwrap();
    if let OperationResult::GitOpened(value) = &mut ack.result {
        value.observation_revision = "0".repeat(64);
    }
    assert!(broker.acknowledge_ui(ack.clone()).is_err());
    if let OperationResult::GitOpened(value) = &mut ack.result {
        value.observation_revision = prepared.observation_revision;
    }
    broker.acknowledge_ui(ack).unwrap();
    assert!(
        matches!(client.call(Request::GitOpen(input)).await.unwrap(), Reply::Ok { data: Data::Operation { state, operation_id, .. }, .. } if state == "succeeded" && operation_id == command.operation_id)
    );
    assert!(commands.try_recv().is_err());
    std::fs::rename(&project, root.path().join("moved")).unwrap();
    std::fs::create_dir(&project).unwrap();
    assert!(broker
        .read_git_view(&prepared.permit_id, Some("a.txt"))
        .is_err());
    broker.release_git_view(&prepared.permit_id);
    assert!(broker.read_git_view(&prepared.permit_id, None).is_err());
    broker.shutdown().await;
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_mutations_require_exact_one_use_approval_and_preserve_retry_after_changes() {
    let _serial = GIT_FIXTURE_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("/Library/Developer/CommandLineTools/usr/bin/git")
            .arg("-C")
            .arg(&project)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", root.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q"]);
    git(&["config", "core.attributesFile", "/dev/null"]);
    git(&["config", "core.hooksPath", "/dev/null"]);
    git(&["config", "core.fsmonitor", "false"]);
    std::fs::write(project.join("a.txt"), "approved disk bytes 🙂\n").unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let projection = projection(&broker, &project);
    broker.publish(projection.clone()).unwrap();
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| tx.send(c).map_err(std::io::Error::other)))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "git.read",
            "git.write",
            "git.execute",
            "git.network",
            "git.push",
            "git.discard",
        ],
    )
    .await;
    let network_only = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "git.read",
            "git.write",
            "git.execute",
            "git.network",
        ],
    )
    .await;
    let remote_fixture = root.path().join("remote.git");
    let local_only = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "files.read",
            "git.read",
            "git.write",
            "git.execute",
        ],
    )
    .await;
    let read_only = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "files.read", "git.read"],
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
    for case in [
        "preview-budget",
        "deny",
        "stale-file",
        "stale-config",
        "cancel",
        "stage",
        "unstage",
        "commit",
        "fetch",
        "push",
        "discard",
        "revoke",
    ] {
        if case == "discard" {
            std::fs::write(project.join("a.txt"), "working changes to discard\n").unwrap();
        }
        if case == "commit" {
            git(&["config", "user.email", "fixture@example.invalid"]);
            git(&["config", "commit.gpgSign", "false"]);
            git(&["add", "--", "a.txt"]);
        }
        if case == "fetch" {
            git(&["init", "-q", "--bare", remote_fixture.to_str().unwrap()]);
            git(&["remote", "add", "fixture", remote_fixture.to_str().unwrap()]);
            git(&["push", "-q", "fixture", "HEAD:refs/heads/fetched"]);
            git(&["update-ref", "-d", "refs/remotes/fixture/fetched"]);
        }
        let paths = if matches!(case, "fetch" | "push") {
            vec![]
        } else if case == "preview-budget" {
            let parent = format!(
                "{}/{}/{}",
                "a".repeat(200),
                "b".repeat(200),
                "c".repeat(200)
            );
            std::fs::create_dir_all(project.join(&parent)).unwrap();
            (0..64)
                .map(|index| {
                    let path = format!("{parent}/{index}.txt");
                    std::fs::write(project.join(&path), "bounded preview fixture\n").unwrap();
                    path
                })
                .collect()
        } else {
            vec!["a.txt".into()]
        };
        let input = GitMutateInput {
            source_commit: (case == "push").then(|| git(&["rev-parse", "HEAD"]).trim().into()),
            expected_remote_commit: None,
            pull_mode: None,
            remote: matches!(case, "fetch" | "push").then(|| "fixture".into()),
            reference: if case == "fetch" {
                Some("refs/heads/fetched".into())
            } else if case == "push" {
                Some("refs/heads/pushed".into())
            } else {
                None
            },
            message: (case == "commit")
                .then(|| "Approved fixture commit 🙂\n\nExact body  \n".into()),
            workspace_id: "a".into(),
            repository_relative: "".into(),
            operation: if case == "discard" {
                GitMutation::Discard
            } else if case == "push" {
                GitMutation::Push
            } else if case == "fetch" {
                GitMutation::Fetch
            } else if case == "commit" {
                GitMutation::Commit
            } else if case == "unstage" {
                GitMutation::Unstage
            } else {
                GitMutation::Stage
            },
            paths,
            expected_revision: "1".into(),
            retry_epoch: retry_epoch.clone(),
            request_key: case.into(),
        };
        assert!(matches!(
            read_only
                .call(Request::GitMutate(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
        if matches!(case, "fetch" | "discard") {
            assert!(matches!(
                local_only
                    .call(Request::GitMutate(input.clone()))
                    .await
                    .unwrap(),
                Reply::Error {
                    code: ErrorCode::ScopeDenied,
                    ..
                }
            ));
        }
        if case == "push" {
            assert!(matches!(
                network_only
                    .call(Request::GitMutate(input.clone()))
                    .await
                    .unwrap(),
                Reply::Error {
                    code: ErrorCode::ScopeDenied,
                    ..
                }
            ));
        }
        let started = client
            .call(Request::GitMutate(input.clone()))
            .await
            .unwrap();
        assert!(
            matches!(
                &started,
                Reply::Ok {
                    data: Data::Operation { .. },
                    ..
                }
            ),
            "{case}: {started:?}"
        );
        let command = commands.recv().await.unwrap();
        let op = &command.operation_id;
        let nonce = &command.nonce;
        assert!(broker.prepare_git_mutation(op, nonce, |_| Ok(())).is_err());
        broker.claim_ui(&projection.ui_epoch, op, nonce).unwrap();
        assert!(broker
            .prepare_git_mutation(op, "wrong", |_| Ok(()))
            .is_err());
        if case == "preview-budget" {
            assert!(matches!(
                broker.prepare_git_mutation(op, nonce, |_| Ok(())),
                Err(ErrorCode::ResourceExhausted)
            ));
            let retry = client.call(Request::GitMutate(input)).await.unwrap();
            assert!(
                matches!(retry, Reply::Ok { data: Data::Operation { state, effect_state, .. }, .. } if state == "failed" && effect_state == "none")
            );
            assert!(commands.try_recv().is_err());
            assert!(git(&["ls-files"]).is_empty());
            continue;
        }
        let plan = broker.prepare_git_mutation(op, nonce, |_| Ok(())).unwrap();
        if case == "fetch" {
            assert!(plan.files.is_empty());
            assert_eq!(plan.network.as_ref().unwrap().remote, "fixture");
            assert_eq!(git(&["for-each-ref", "refs/remotes/fixture/"]), "");
        } else if case == "push" {
            assert!(plan.files.is_empty());
            assert_eq!(
                plan.push.as_ref().unwrap().target.source_commit,
                input.source_commit.as_deref().unwrap()
            );
        } else {
            assert_eq!(plan.files[0].relative_path, "a.txt");
        }
        if case == "commit" {
            assert_eq!(
                plan.commit.as_ref().unwrap().message,
                input.message.as_deref().unwrap()
            );
            assert_eq!(
                plan.commit.as_ref().unwrap().author.email,
                "fixture@example.invalid"
            );
        } else {
            assert!(plan.commit.is_none());
        }
        assert!(broker.git_mutation_pending(op, nonce, &plan.plan_hash));
        assert!(broker
            .commit_git_mutation(op, nonce, &plan.plan_hash, |_| Ok(()))
            .is_err());
        assert!(broker.prepare_git_mutation(op, nonce, |_| Ok(())).is_err());
        assert!(broker
            .decide_git_mutation(op, nonce, &"0".repeat(64), true)
            .is_err());
        if case == "revoke" {
            broker.revoke();
            broker.wait_for_cleanup().await;
            assert!(!broker.git_mutation_pending(op, nonce, &plan.plan_hash));
            assert!(broker
                .decide_git_mutation(op, nonce, &plan.plan_hash, true)
                .is_err());
            assert!(broker
                .commit_git_mutation(op, nonce, &plan.plan_hash, |_| Ok(()))
                .is_err());
            break;
        }
        if case == "deny" {
            broker
                .decide_git_mutation(op, nonce, &plan.plan_hash, false)
                .unwrap();
        } else if case == "cancel" {
            let _ = client
                .call(Request::CancelOperation(OperationInput {
                    operation_id: op.clone(),
                }))
                .await
                .unwrap();
        } else {
            broker
                .decide_git_mutation(op, nonce, &plan.plan_hash, true)
                .unwrap();
            assert!(!broker.git_mutation_pending(op, nonce, &plan.plan_hash));
            assert!(broker
                .decide_git_mutation(op, nonce, &plan.plan_hash, true)
                .is_err());
            if case == "stale-file" {
                std::fs::write(project.join("a.txt"), "a newer disk version\n").unwrap();
            }
            if case == "stale-config" {
                git(&["config", "user.name", "Changed fixture identity"]);
            }
            let result = broker.commit_git_mutation(op, nonce, &plan.plan_hash, |_| Ok(()));
            if case.starts_with("stale-") {
                assert!(
                    matches!(result, Err(ErrorCode::RevisionConflict)),
                    "{result:?}"
                );
            } else {
                let result = result.unwrap_or_else(|code| panic!("{case}: {code:?}"));
                assert_eq!(result.selected_files_changed, case == "discard");
                if case == "discard" {
                    assert_eq!(
                        std::fs::read_to_string(project.join("a.txt")).unwrap(),
                        git(&["show", ":a.txt"])
                    );
                }
                assert_ne!(result.before_revision, result.after_revision);
                assert_eq!(
                    git(&["ls-files"]).contains("a.txt"),
                    matches!(case, "stage" | "commit" | "fetch" | "push" | "discard")
                );
                if case == "stage" {
                    assert_eq!(git(&["show", ":a.txt"]), "a newer disk version\n");
                }
                if case == "commit" {
                    use sha2::{Digest, Sha256};
                    assert_eq!(
                        git(&["cat-file", "commit", "HEAD"])
                            .split_once("\n\n")
                            .unwrap()
                            .1,
                        input.message.as_deref().unwrap()
                    );
                    assert_eq!(
                        result.head.as_deref().unwrap(),
                        git(&["rev-parse", "HEAD"]).trim()
                    );
                    assert_eq!(
                        result.commit.as_ref().unwrap().message_sha256,
                        format!(
                            "{:x}",
                            Sha256::digest(input.message.as_deref().unwrap().as_bytes())
                        )
                    );
                }
                if case == "fetch" {
                    let fetch = result.fetch.as_ref().unwrap();
                    assert_eq!(fetch.commit, git(&["rev-parse", "HEAD"]).trim());
                    assert_eq!(git(&["rev-parse", &fetch.destination]).trim(), fetch.commit);
                }
                if case == "push" {
                    let pushed = result.push.as_ref().unwrap();
                    assert!(pushed.changed);
                    assert_eq!(
                        pushed.verification,
                        GitPushVerification::ServerAcknowledgement
                    );
                    assert_eq!(
                        git(&[
                            &format!("--git-dir={}", remote_fixture.display()),
                            "rev-parse",
                            "refs/heads/pushed"
                        ])
                        .trim(),
                        pushed.target.source_commit
                    );
                }
                let mut forged = result.clone();
                forged.index_revision = "0".repeat(64);
                let ack = |result| UiAck {
                    operation_id: op.clone(),
                    ui_epoch: projection.ui_epoch.clone(),
                    nonce: nonce.clone(),
                    result: OperationResult::GitMutated(Box::new(result)),
                };
                assert!(broker.acknowledge_ui(ack(forged)).is_err());
                broker.acknowledge_ui(ack(result)).unwrap();
            }
        }
        assert!(broker
            .commit_git_mutation(op, nonce, &plan.plan_hash, |_| Ok(()))
            .is_err());
        let retry = client
            .call(Request::GitMutate(input.clone()))
            .await
            .unwrap();
        let expected = match case {
            "deny" | "cancel" => "cancelled",
            "stage" | "unstage" | "commit" | "fetch" | "push" | "discard" => "succeeded",
            _ => "failed",
        };
        assert!(
            matches!(&retry, Reply::Ok { data: Data::Operation { state, operation_id, .. }, .. } if state == expected && operation_id == op),
            "{retry:?}"
        );
        assert!(commands.try_recv().is_err());
        if !matches!(
            case,
            "stage" | "unstage" | "commit" | "fetch" | "push" | "discard"
        ) {
            assert!(git(&["ls-files"]).is_empty());
        }
        let mut altered = input;
        if matches!(case, "fetch" | "push") {
            altered.reference = Some("refs/heads/different".into());
        } else {
            altered.paths = vec!["different.txt".into()];
        }
        assert!(matches!(
            client.call(Request::GitMutate(altered)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::IdempotencyConflict,
                ..
            }
        ));
    }
    assert!(git(&["diff", "--cached", "--name-only"]).is_empty());
    broker.shutdown().await;
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_pull_records_exact_native_success_or_partial_conflict_without_replay() {
    let _serial = GIT_FIXTURE_LOCK.lock().await;
    for case in ["applied", "remote-changed", "conflict"] {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let peer = root.path().join("peer");
        let remote = root.path().join("remote.git");
        std::fs::create_dir(&project).unwrap();
        let git = |dir: &std::path::Path, args: &[&str]| {
            let out = std::process::Command::new("/Library/Developer/CommandLineTools/usr/bin/git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{case} {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap()
        };
        let config = |dir: &std::path::Path| {
            for (key, value) in [
                ("user.name", "Fixture"),
                ("user.email", "fixture@example.invalid"),
                ("core.hooksPath", "/dev/null"),
                ("core.fsmonitor", "false"),
                ("core.attributesFile", "/dev/null"),
                ("commit.gpgSign", "false"),
            ] {
                git(dir, &["config", key, value]);
            }
        };
        git(&project, &["init", "-q", "--initial-branch=main"]);
        config(&project);
        std::fs::write(project.join("a.txt"), "base\n").unwrap();
        git(&project, &["add", "."]);
        git(&project, &["commit", "-qm", "base"]);
        git(
            root.path(),
            &["init", "-q", "--bare", remote.to_str().unwrap()],
        );
        git(
            &project,
            &["remote", "add", "fixture", remote.to_str().unwrap()],
        );
        git(&project, &["push", "-q", "fixture", "HEAD:refs/heads/main"]);
        git(
            root.path(),
            &[
                "clone",
                "-q",
                "--branch=main",
                remote.to_str().unwrap(),
                peer.to_str().unwrap(),
            ],
        );
        config(&peer);
        if case == "conflict" {
            std::fs::write(project.join("a.txt"), "local commit\n").unwrap();
            git(&project, &["commit", "-qam", "local"]);
        }
        std::fs::write(peer.join("a.txt"), "remote commit\n").unwrap();
        git(&peer, &["commit", "-qam", "remote"]);
        git(&peer, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
        git(&project, &["fetch", "-q", "fixture"]);
        let broker = Broker::start(&root.path().join("control")).unwrap();
        let projection = projection(&broker, &project);
        broker.publish(projection.clone()).unwrap();
        let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
        broker
            .set_ui_dispatch(Arc::new(move |c| tx.send(c).map_err(std::io::Error::other)))
            .unwrap();
        let scopes = [
            "workspace.read",
            "files.read",
            "git.read",
            "git.write",
            "git.execute",
            "git.network",
            "git.pull",
        ];
        let client = approved_scopes(&broker, &["a"], &scopes).await;
        let denied = approved_scopes(&broker, &["a"], &scopes[..6]).await;
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
        let input = GitMutateInput {
            workspace_id: "a".into(),
            repository_relative: "".into(),
            operation: GitMutation::Pull,
            paths: vec![],
            message: None,
            remote: Some("fixture".into()),
            reference: Some("refs/heads/main".into()),
            source_commit: Some(git(&project, &["rev-parse", "HEAD"]).trim().into()),
            expected_remote_commit: Some(git(&peer, &["rev-parse", "HEAD"]).trim().into()),
            pull_mode: Some(if case == "conflict" {
                GitPullMode::Rebase
            } else {
                GitPullMode::FfOnly
            }),
            expected_revision: "1".into(),
            retry_epoch,
            request_key: case.into(),
        };
        assert!(matches!(
            denied
                .call(Request::GitMutate(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
        let started = client
            .call(Request::GitMutate(input.clone()))
            .await
            .unwrap();
        assert!(
            matches!(
                started,
                Reply::Ok {
                    data: Data::Operation { .. },
                    ..
                }
            ),
            "{started:?}"
        );
        let command = commands.recv().await.unwrap();
        let (op, nonce) = (&command.operation_id, &command.nonce);
        broker.claim_ui(&projection.ui_epoch, op, nonce).unwrap();
        let plan = broker.prepare_git_mutation(op, nonce, |_| Ok(())).unwrap();
        assert_eq!(
            plan.pull.as_ref().unwrap().target.expected_remote_commit,
            input.expected_remote_commit.clone().unwrap()
        );
        broker
            .decide_git_mutation(op, nonce, &plan.plan_hash, true)
            .unwrap();
        if case == "remote-changed" {
            std::fs::write(peer.join("a.txt"), "newer remote commit\n").unwrap();
            git(&peer, &["commit", "-qam", "newer remote"]);
            git(&peer, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
        }
        let result = broker
            .commit_git_mutation(op, nonce, &plan.plan_hash, |_| Ok(()))
            .unwrap();
        let status = match case {
            "applied" => GitPullOutcome::Applied,
            "remote-changed" => GitPullOutcome::RemoteChanged,
            _ => GitPullOutcome::Conflicted,
        };
        assert_eq!(result.pull.as_ref().unwrap().outcome, status);
        let ack = |result| UiAck {
            operation_id: op.clone(),
            ui_epoch: projection.ui_epoch.clone(),
            nonce: nonce.clone(),
            result: OperationResult::GitMutated(Box::new(result)),
        };
        let mut forged = result.clone();
        forged.pull.as_mut().unwrap().fetched_commit = "0".repeat(40);
        assert!(broker.acknowledge_ui(ack(forged)).is_err());
        broker.acknowledge_ui(ack(result)).unwrap();
        let expected_state = if case == "applied" {
            "succeeded"
        } else {
            "failed"
        };
        let expected_effect = if case == "applied" {
            "complete"
        } else {
            "partial"
        };
        let retry = client
            .call(Request::GitMutate(input.clone()))
            .await
            .unwrap();
        assert!(
            matches!(&retry, Reply::Ok { data: Data::Operation { operation_id, state, effect_state, .. }, .. } if operation_id == op && state == expected_state && effect_state == expected_effect),
            "{case}: {retry:?}"
        );
        assert!(commands.try_recv().is_err());
        assert!(broker
            .commit_git_mutation(op, nonce, &plan.plan_hash, |_| Ok(()))
            .is_err());
        if case == "remote-changed" {
            assert_eq!(
                git(&project, &["rev-parse", "HEAD"]).trim(),
                input.source_commit.unwrap()
            );
            assert_eq!(
                std::fs::read_to_string(project.join("a.txt")).unwrap(),
                "base\n"
            );
        } else if case == "conflict" {
            assert!(project.join(".git/rebase-merge").is_dir());
            assert!(!git(&project, &["ls-files", "--unmerged"]).is_empty());
        } else {
            assert_eq!(
                std::fs::read_to_string(project.join("a.txt")).unwrap(),
                "remote commit\n"
            );
        }
        broker.shutdown().await;
    }
}

#[tokio::test]
async fn panel_moves_bind_both_targets_runtime_identities_scope_and_retry() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.panels = [
        ("file", "file", "a", "file", None),
        ("pty", "terminal", "a", "terminal", Some("live")),
        ("other", "other", "b", "file", None),
    ]
    .into_iter()
    .map(|(id, tab, workspace, kind, session)| Panel {
        id: id.into(),
        tab_id: tab.into(),
        workspace_id: workspace.into(),
        kind: kind.into(),
        title: id.into(),
        terminal_session_id: session.map(str::to_owned),
        browser_generation: None,
        android_device_id: None,
    })
    .collect();
    p.focused_panel_id = Some("pty".into());
    p.workspaces[0].active_panel_id = Some("pty".into());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "workspace.write",
            "panel.move",
            "panel.focus",
        ],
    )
    .await;
    let reader = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "workspace.write", "panel.focus"],
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
    let mut input = PanelMoveInput {
        workspace_id: "a".into(),
        movement: PanelMove::ReorderTab {
            tab_id: "file".into(),
            before_tab_id: None,
        },
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "reorder".into(),
    };
    assert!(matches!(
        reader
            .call(Request::MovePanel(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let mut foreign = input.clone();
    foreign.movement = PanelMove::DockTab {
        tab_id: "other".into(),
        target_tab_id: "terminal".into(),
        side: DockSide::Right,
    };
    assert!(matches!(
        client.call(Request::MovePanel(foreign)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    client
        .call(Request::MovePanel(input.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let UiAction::MovePanel(movement) = &command.action else {
        panic!()
    };
    let ack = UiAck {
        operation_id: command.operation_id.clone(),
        nonce: command.nonce.clone(),
        ui_epoch: p.ui_epoch.clone(),
        result: OperationResult::PanelMoved(
            PanelMoved {
                destination: None,
                workspace_id: "a".into(),
                movement: movement.movement.clone(),
                panels: movement.panels.clone(),
            }
            .into(),
        ),
    };
    assert!(
        broker.acknowledge_ui(ack.clone()).is_err(),
        "Tab order must actually change before success"
    );
    p.panels.swap(0, 1);
    p.revision = "2".into();
    broker.publish(p.clone()).unwrap();
    broker.acknowledge_ui(ack).unwrap();
    assert!(
        matches!(client.call(Request::MovePanel(input.clone())).await.unwrap(), Reply::Ok { data:Data::Operation {operation_id,state,..},..} if operation_id==command.operation_id && state=="succeeded")
    );
    assert!(commands.try_recv().is_err());
    input.request_key = "dock".into();
    input.expected_revision = "2".into();
    input.movement = PanelMove::DockTab {
        tab_id: "file".into(),
        target_tab_id: "terminal".into(),
        side: DockSide::Right,
    };
    client
        .call(Request::MovePanel(input.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let UiAction::MovePanel(movement) = &command.action else {
        panic!()
    };
    let mut result = PanelMoved {
        destination: None,
        workspace_id: "a".into(),
        movement: movement.movement.clone(),
        panels: movement.panels.clone(),
    };
    for panel in &mut result.panels {
        panel.tab_id = "terminal".into();
    }
    let mut ack = UiAck {
        operation_id: command.operation_id.clone(),
        nonce: command.nonce.clone(),
        ui_epoch: p.ui_epoch.clone(),
        result: OperationResult::PanelMoved(result.clone().into()),
    };
    assert!(broker.acknowledge_ui(ack.clone()).is_err());
    p.panels[1].tab_id = "terminal".into();
    p.focused_panel_id = Some("file".into());
    p.workspaces[0].active_panel_id = Some("file".into());
    p.revision = "3".into();
    broker.publish(p.clone()).unwrap();
    result
        .panels
        .iter_mut()
        .find(|p| p.panel_id == "pty")
        .unwrap()
        .terminal_session_id = Some("restarted".into());
    ack.result = OperationResult::PanelMoved(result.into());
    assert!(
        broker.acknowledge_ui(ack.clone()).is_err(),
        "An ACK cannot replace the retained terminal identity"
    );
    if let OperationResult::PanelMoved(result) = &mut ack.result {
        result
            .panels
            .iter_mut()
            .find(|p| p.panel_id == "pty")
            .unwrap()
            .terminal_session_id = Some("live".into());
    }
    broker.acknowledge_ui(ack).unwrap();
    assert!(
        matches!(client.call(Request::MovePanel(input.clone())).await.unwrap(),Reply::Ok {data:Data::Operation {operation_id,state,..},..} if operation_id==command.operation_id && state=="succeeded")
    );
    input.expected_revision = "3".into();
    input.request_key = "stale-runtime".into();
    input.movement = PanelMove::MovePane {
        panel_id: "file".into(),
        target_panel_id: "pty".into(),
        side: DockSide::Top,
    };
    client
        .call(Request::MovePanel(input.clone()))
        .await
        .unwrap();
    let pending = commands.recv().await.unwrap();
    p.panels[0].terminal_session_id = None;
    p.revision = "4".into();
    broker.publish(p.clone()).unwrap();
    assert!(broker
        .claim_ui(&p.ui_epoch, &pending.operation_id, &pending.nonce)
        .is_err());
    input.expected_revision = "4".into();
    input.request_key = "lazy".into();
    assert!(matches!(
        client.call(Request::MovePanel(input)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    assert!(commands.try_recv().is_err());
    let unknown = PanelMoveInput {
        workspace_id: "a".into(),
        movement: PanelMove::ReorderTab {
            tab_id: "terminal".into(),
            before_tab_id: None,
        },
        expected_revision: "4".into(),
        retry_epoch: match client
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
            _ => panic!(),
        },
        request_key: "uncertain-publication".into(),
    };
    client
        .call(Request::MovePanel(unknown.clone()))
        .await
        .unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
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
    assert!(
        matches!(client.call(Request::MovePanel(unknown)).await.unwrap(),Reply::Ok {data:Data::Operation {operation_id,state,effect_state,..},..} if operation_id==command.operation_id && state=="outcome_unknown" && effect_state=="unknown")
    );
    assert!(commands.try_recv().is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn workspace_close_retries_and_receipts_survive_removal_without_extending_resource_scope() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.workspaces.retain(|w| w.id != "b");
    p.panels = vec![Panel {
        id: "file".into(),
        tab_id: "file".into(),
        workspace_id: "a".into(),
        kind: "file".into(),
        title: "Draft".into(),
        terminal_session_id: None,
        browser_generation: None,
        android_device_id: None,
    }];
    p.workspaces[0].active_panel_id = Some("file".into());
    p.focused_panel_id = Some("file".into());
    broker.publish(p.clone()).unwrap();
    let (send, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            send.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let client = approved_scopes(
        &broker,
        &["a"],
        &[
            "workspace.read",
            "workspace.write",
            "workspace.close",
            "panel.close",
        ],
    )
    .await;
    let reader = approved_scopes(
        &broker,
        &["a"],
        &["workspace.read", "workspace.write", "panel.close"],
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
    let mut input = WorkspaceCloseInput {
        action: WorkspaceCloseAction::Close,
        workspace_id: "a".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "close-saved".into(),
    };
    let request = |input| Request::RenameWorkspace(WorkspaceUpdateInput::Close(input));
    assert!(matches!(
        reader.call(request(input.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::ScopeDenied,
            ..
        }
    ));
    let mut foreign = input.clone();
    foreign.workspace_id = "foreign".into();
    assert!(matches!(
        client.call(request(foreign)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    client.call(request(input.clone())).await.unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    assert!(broker.workspace_close_pending(&command.operation_id, &command.nonce, &p.ui_epoch));
    let saved = WorkspaceClosure {
        workspace_id: "a".into(),
        project_id: "p".into(),
        panel_ids: vec!["file".into()],
        terminal_session_ids: vec![],
        closed: Some(false),
        project_closed: Some(false),
    };
    broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce,
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::WorkspaceClosure(saved),
        })
        .unwrap();
    assert!(
        matches!(client.call(request(input.clone())).await.unwrap(),Reply::Ok {data:Data::Operation {state,effect_state,..},..} if state=="failed" && effect_state=="partial")
    );
    assert!(commands.try_recv().is_err());
    input.request_key = "close-after-save".into();
    client.call(request(input.clone())).await.unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    let result = WorkspaceClosure {
        workspace_id: "a".into(),
        project_id: "p".into(),
        panel_ids: vec!["file".into()],
        terminal_session_ids: vec![],
        closed: Some(true),
        project_closed: Some(true),
    };
    let ack = UiAck {
        operation_id: command.operation_id.clone(),
        nonce: command.nonce.clone(),
        ui_epoch: p.ui_epoch.clone(),
        result: OperationResult::WorkspaceClosure(result),
    };
    assert!(
        broker.acknowledge_ui(ack.clone()).is_err(),
        "A closed ACK requires the native close commit"
    );
    broker
        .commit_panel_close(&command.operation_id, &command.nonce, &p.ui_epoch)
        .unwrap();
    assert!(!broker.workspace_close_pending(&command.operation_id, &command.nonce, &p.ui_epoch));
    assert!(
        broker
            .commit_panel_close(&command.operation_id, &command.nonce, &p.ui_epoch)
            .is_err(),
        "Close commit is one-use"
    );
    assert!(
        broker.acknowledge_ui(ack.clone()).is_err(),
        "A closed ACK requires actual domain removal"
    );
    p.revision = "2".into();
    p.workspaces.retain(|w| w.id != "a");
    p.panels.clear();
    p.focused_panel_id = None;
    broker.publish(p).unwrap();
    broker.acknowledge_ui(ack).unwrap();
    let lookup = Request::Operation(
        OperationInput {
            operation_id: command.operation_id.clone(),
        }
        .into(),
    );
    assert!(
        matches!(client.call(lookup.clone()).await.unwrap(),Reply::Ok {data:Data::Operation {state,effect_state,result:Some(OperationResult::WorkspaceClosure(WorkspaceClosure {closed:Some(true),project_closed:Some(true),..})),..},..} if state=="succeeded" && effect_state=="complete")
    );
    assert!(matches!(
        reader.call(lookup).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert!(
        matches!(client.call(request(input.clone())).await.unwrap(),Reply::Ok {data:Data::Operation {operation_id,state,..},..} if operation_id==command.operation_id && state=="succeeded")
    );
    assert!(commands.try_recv().is_err());
    input.expected_revision = "2".into();
    assert!(matches!(
        client.call(request(input.clone())).await.unwrap(),
        Reply::Error {
            code: ErrorCode::IdempotencyConflict,
            ..
        }
    ));
    input.request_key = "new-close-after-removal".into();
    assert!(matches!(
        client.call(request(input)).await.unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert!(matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: "a".into()
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert!(matches!(
        client
            .call(Request::Panels(WorkspaceListInput {
                workspace_id: "a".into(),
                limit: 50,
                cursor: None
            }))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn uncertain_workspace_close_preserves_preparation_after_target_removal_and_cancel() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    p.workspaces.retain(|w| w.id == "a");
    p.panels = vec![Panel {
        id: "file".into(),
        tab_id: "file".into(),
        workspace_id: "a".into(),
        kind: "file".into(),
        title: "File".into(),
        terminal_session_id: None,
        browser_generation: None,
        android_device_id: None,
    }];
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
            "workspace.write",
            "workspace.close",
            "panel.close",
        ],
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
    let request = Request::RenameWorkspace(WorkspaceUpdateInput::Close(WorkspaceCloseInput {
        action: WorkspaceCloseAction::Close,
        workspace_id: "a".into(),
        expected_revision: "1".into(),
        retry_epoch,
        request_key: "uncertain-close".into(),
    }));
    client.call(request.clone()).await.unwrap();
    let command = commands.recv().await.unwrap();
    broker
        .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
        .unwrap();
    broker
        .commit_panel_close(&command.operation_id, &command.nonce, &p.ui_epoch)
        .unwrap();
    p.workspaces.clear();
    p.panels.clear();
    p.revision = "2".into();
    broker.publish(p.clone()).unwrap();
    assert!(
        matches!(client.call(Request::CancelOperation(OperationInput {operation_id:command.operation_id.clone()})).await.unwrap(),Reply::Ok {data:Data::Operation {state,..},..} if state=="cancelling")
    );
    broker
        .acknowledge_ui(UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce,
            ui_epoch: p.ui_epoch,
            result: OperationResult::Failure {
                code: ErrorCode::OutcomeUnknown,
            },
        })
        .unwrap();
    assert!(
        matches!(client.call(request).await.unwrap(),Reply::Ok {data:Data::Operation {state,effect_state,result:Some(OperationResult::WorkspaceClosure(WorkspaceClosure {closed:None,project_closed:None,..})),..},..} if state=="outcome_unknown" && effect_state=="unknown")
    );
    assert!(commands.try_recv().is_err());
    broker.shutdown().await;
}

#[tokio::test]
async fn project_close_requires_every_workspace_and_preserves_retired_receipts() {
    for uncertain in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let broker = Broker::start(&root.path().join("control")).unwrap();
        let mut p = projection(&broker, root.path());
        p.panels = ["a", "b", "foreign"]
            .into_iter()
            .map(|workspace| Panel {
                id: format!("file-{workspace}"),
                tab_id: format!("tab-{workspace}"),
                workspace_id: workspace.into(),
                kind: "file".into(),
                title: "File".into(),
                terminal_session_id: None,
                browser_generation: None,
                android_device_id: None,
            })
            .collect();
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
            "workspace.close",
            "project.close",
            "panel.close",
            "terminal.execute",
        ];
        let client = approved_scopes(&broker, &["a", "b"], &scopes).await;
        let partial = approved_scopes(&broker, &["a"], &scopes).await;
        let epoch = |reply| match reply {
            Reply::Ok {
                data: Data::Connected { retry_epoch, .. },
                ..
            } => retry_epoch,
            other => panic!("{other:?}"),
        };
        let mut input = ProjectCloseInput {
            project_id: "p".into(),
            workspace_id: "a".into(),
            expected_revision: "1".into(),
            retry_epoch: epoch(
                partial
                    .call(Request::Connect(ConnectInput {
                        workspace_id: "a".into(),
                    }))
                    .await
                    .unwrap(),
            ),
            request_key: "close-project".into(),
        };
        assert!(matches!(
            partial
                .call(Request::CloseProject(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        input.retry_epoch = epoch(
            client
                .call(Request::Connect(ConnectInput {
                    workspace_id: "a".into(),
                }))
                .await
                .unwrap(),
        );
        let mut foreign = input.clone();
        foreign.project_id = "other".into();
        assert!(matches!(
            client.call(Request::CloseProject(foreign)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        // A protected descendant in another approved workspace blocks the whole ancestor.
        let mut protected = p.clone();
        protected.revision = "2".into();
        protected.panels.push(Panel {
            id: "protected".into(),
            tab_id: "protected".into(),
            workspace_id: "b".into(),
            kind: "terminal".into(),
            title: "Human".into(),
            terminal_session_id: Some("unowned".into()),
            browser_generation: None,
            android_device_id: None,
        });
        broker.publish(protected).unwrap();
        input.expected_revision = "2".into();
        assert!(matches!(
            client
                .call(Request::CloseProject(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ProtectedOriginTerminal,
                ..
            }
        ));
        assert!(commands.try_recv().is_err());
        p.revision = "3".into();
        broker.publish(p.clone()).unwrap();
        input.expected_revision = "3".into();
        client
            .call(Request::CloseProject(input.clone()))
            .await
            .unwrap();
        let command = commands.recv().await.unwrap();
        broker
            .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
            .unwrap();
        let result = ProjectClosure {
            workspace_id: "a".into(),
            project_id: "p".into(),
            workspace_ids: vec!["a".into(), "b".into()],
            panel_ids: vec!["file-a".into(), "file-b".into()],
            terminal_session_ids: vec![],
            closed: Some(true),
        };
        let mut ack = UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::ProjectClosure(result.into()),
        };
        assert!(broker.acknowledge_ui(ack.clone()).is_err());
        broker
            .commit_panel_close(&command.operation_id, &command.nonce, &p.ui_epoch)
            .unwrap();
        assert!(broker
            .commit_panel_close(&command.operation_id, &command.nonce, &p.ui_epoch)
            .is_err());
        assert!(
            broker.acknowledge_ui(ack.clone()).is_err(),
            "Whole project must disappear before successful ACK"
        );
        p.workspaces.retain(|w| w.id == "foreign");
        p.panels.retain(|p| p.workspace_id == "foreign");
        p.revision = "4".into();
        broker.publish(p.clone()).unwrap();
        if uncertain {
            assert!(
                matches!(client.call(Request::CancelOperation(OperationInput { operation_id: command.operation_id.clone() })).await.unwrap(), Reply::Ok { data: Data::Operation { state, .. }, .. } if state == "cancelling")
            );
            ack.result = OperationResult::Failure {
                code: ErrorCode::OutcomeUnknown,
            };
        } else {
            let mut forged = ack.clone();
            if let OperationResult::ProjectClosure(r) = &mut forged.result {
                r.workspace_ids.pop();
            }
            assert!(broker.acknowledge_ui(forged).is_err());
        }
        broker.acknowledge_ui(ack).unwrap();
        let replayed = client
            .call(Request::CloseProject(input.clone()))
            .await
            .unwrap();
        assert!(
            matches!(&replayed, Reply::Ok { data: Data::Operation { operation_id, state, effect_state, result: Some(OperationResult::ProjectClosure(r)), .. }, .. } if operation_id == &command.operation_id && r.closed == if uncertain { None } else { Some(true) } && state == if uncertain { "outcome_unknown" } else { "succeeded" } && effect_state == if uncertain { "unknown" } else { "complete" }),
            "{replayed:?}"
        );
        let lookup = Request::Operation(
            OperationInput {
                operation_id: command.operation_id.clone(),
            }
            .into(),
        );
        assert!(matches!(
            partial.call(lookup).await.unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        input.expected_revision = "4".into();
        assert!(matches!(
            client
                .call(Request::CloseProject(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::IdempotencyConflict,
                ..
            }
        ));
        input.request_key = "fresh-after-close".into();
        assert!(matches!(
            client.call(Request::CloseProject(input)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        assert!(matches!(
            client
                .call(Request::Panels(WorkspaceListInput {
                    workspace_id: "b".into(),
                    cursor: None,
                    limit: 50
                }))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        assert!(commands.try_recv().is_err());
        broker.shutdown().await;
    }
}

#[tokio::test]
async fn panel_transfer_migrates_only_live_authorized_publication_and_preserves_retry() {
    for cancelled in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let broker = Broker::start(&root.path().join("control")).unwrap();
        let mut p = projection(&broker, root.path());
        let profile = TerminalProfile {
            id: "zsh".into(),
            revision: "pinned".into(),
        };
        p.terminal_profile = Some(profile.clone());
        p.panels = [("file", "a"), ("target", "b")]
            .into_iter()
            .map(|(id, workspace)| Panel {
                id: id.into(),
                tab_id: id.into(),
                workspace_id: workspace.into(),
                kind: "file".into(),
                title: id.into(),
                terminal_session_id: None,
                browser_generation: None,
                android_device_id: None,
            })
            .collect();
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
            "panel.create",
            "terminal.execute",
            "terminal.read",
        ];
        let client = approved_scopes(&broker, &["a", "b"], &scopes).await;
        let foreign = approved_scopes(&broker, &["a"], &scopes).await;
        let epoch = |reply: Reply| match reply {
            Reply::Ok {
                data: Data::Connected { retry_epoch, .. },
                ..
            } => retry_epoch,
            other => panic!("{other:?}"),
        };
        let retry = epoch(
            client
                .call(Request::Connect(ConnectInput {
                    workspace_id: "a".into(),
                }))
                .await
                .unwrap(),
        );
        let foreign_retry = epoch(
            foreign
                .call(Request::Connect(ConnectInput {
                    workspace_id: "a".into(),
                }))
                .await
                .unwrap(),
        );
        let mut denied = PanelMoveInput {
            workspace_id: "a".into(),
            movement: PanelMove::TransferTab {
                tab_id: "file".into(),
                target_workspace_id: "b".into(),
                before_tab_id: None,
            },
            expected_revision: "1".into(),
            retry_epoch: foreign_retry,
            request_key: "foreign-destination".into(),
        };
        assert!(matches!(
            foreign
                .call(Request::MovePanel(denied.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        denied.retry_epoch = retry.clone();
        if let PanelMove::TransferTab {
            target_workspace_id,
            ..
        } = &mut denied.movement
        {
            *target_workspace_id = "foreign".into();
        }
        assert!(matches!(
            client.call(Request::MovePanel(denied)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        client
            .call(Request::CreateTerminal(TerminalCreateInput {
                workspace_id: "a".into(),
                cwd_relative: ".".into(),
                profile_id: None,
                title: "Retained".into(),
                expected_revision: "1".into(),
                retry_epoch: retry.clone(),
                request_key: "terminal".into(),
            }))
            .await
            .unwrap();
        let create = commands.recv().await.unwrap();
        let UiAction::CreateTerminal {
            terminal_session_id,
            panel_id,
            tab_id,
            cwd,
            ..
        } = &create.action
        else {
            panic!()
        };
        broker
            .claim_ui(&p.ui_epoch, &create.operation_id, &create.nonce)
            .unwrap();
        let monitor = broker
            .start_terminal(
                &create.operation_id,
                &create.nonce,
                terminal_session_id,
                &profile,
                cwd,
                Ok,
            )
            .unwrap();
        let lease = monitor.lock().unwrap().lease().map(str::to_owned);
        p.panels[0].tab_id = tab_id.clone();
        p.panels.push(Panel {
            id: panel_id.clone(),
            tab_id: tab_id.clone(),
            workspace_id: "a".into(),
            kind: "terminal".into(),
            title: "Retained".into(),
            terminal_session_id: Some(terminal_session_id.clone()),
            browser_generation: None,
            android_device_id: None,
        });
        p.focused_panel_id = Some(panel_id.clone());
        p.workspaces[0].active_panel_id = Some(panel_id.clone());
        p.revision = "2".into();
        broker.publish(p.clone()).unwrap();
        broker
            .acknowledge_ui(UiAck {
                operation_id: create.operation_id.clone(),
                nonce: create.nonce.clone(),
                ui_epoch: p.ui_epoch.clone(),
                result: OperationResult::Terminal {
                    workspace_id: "a".into(),
                    panel_id: panel_id.clone(),
                    terminal_session_id: terminal_session_id.clone(),
                    lease_id: None,
                    ready: true,
                },
            })
            .unwrap();
        let dispatched = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = dispatched.clone();
        broker
            .set_terminal_dispatch(Arc::new(move |request| {
                assert!(request.permit.as_ref()().is_some());
                let mut terminal = request.control.lock().unwrap();
                if let Some(operation) = request.interrupt {
                    terminal
                        .prepare_interrupt(&request.lease, &operation)
                        .unwrap();
                    terminal.observe(b"\x1b]133;D;130\x07\x1b]133;A\x07\x1b]133;B\x07");
                } else {
                    terminal
                        .prepare_run(&request.lease, &request.operation, &request.command, false)
                        .unwrap();
                    terminal.observe(b"\x1b]133;C\x07");
                }
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }))
            .unwrap();
        monitor
            .lock()
            .unwrap()
            .observe(b"\x1b]133;A\x07\x1b]133;B\x07");
        let run = TerminalRunInput {
            workspace_id: "a".into(),
            panel_id: panel_id.clone(),
            terminal_session_id: terminal_session_id.clone(),
            lease_id: lease.clone().unwrap(),
            command: "fixture-long-command".into(),
            retry_epoch: retry.clone(),
            request_key: "run-before-transfer".into(),
        };
        let running = client
            .call(Request::RunTerminal(run.clone()))
            .await
            .unwrap();
        let run_id = match running {
            Reply::Ok {
                data: Data::Operation { operation_id, .. },
                ..
            } => operation_id,
            other => panic!("{other:?}"),
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while dispatched.load(std::sync::atomic::Ordering::SeqCst) != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let input = PanelMoveInput {
            workspace_id: "a".into(),
            movement: PanelMove::TransferTab {
                tab_id: tab_id.clone(),
                target_workspace_id: "b".into(),
                before_tab_id: None,
            },
            expected_revision: "2".into(),
            retry_epoch: retry,
            request_key: "transfer".into(),
        };
        let accepted = client
            .call(Request::MovePanel(input.clone()))
            .await
            .unwrap();
        assert!(
            matches!(
                accepted,
                Reply::Ok {
                    data: Data::Operation { .. },
                    ..
                }
            ),
            "{accepted:?}"
        );
        let command = commands.recv().await.unwrap();
        broker
            .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
            .unwrap();
        let UiAction::MovePanel(movement) = &command.action else {
            panic!()
        };
        let mut destination = movement.destination.clone().unwrap();
        destination.panels.extend(movement.panels.clone());
        destination
            .panels
            .sort_by(|a, b| a.panel_id.cmp(&b.panel_id));
        destination.tab_order.push(tab_id.clone());
        let mut ack = UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::PanelMoved(
                PanelMoved {
                    workspace_id: "a".into(),
                    movement: movement.movement.clone(),
                    panels: vec![],
                    destination: Some(destination),
                }
                .into(),
            ),
        };
        assert!(
            broker.acknowledge_ui(ack.clone()).is_err(),
            "ACK alone cannot migrate ownership"
        );
        if cancelled {
            client
                .call(Request::CancelOperation(OperationInput {
                    operation_id: command.operation_id.clone(),
                }))
                .await
                .unwrap();
        }
        let mut moved: Vec<_> = p
            .panels
            .iter()
            .filter(|panel| panel.workspace_id == "a")
            .cloned()
            .collect();
        for panel in &mut moved {
            panel.workspace_id = "b".into();
        }
        p.panels.retain(|panel| panel.workspace_id != "a");
        p.panels.extend(moved);
        p.focused_panel_id = None;
        p.workspaces[0].active_panel_id = None;
        p.revision = "3".into();
        broker.publish(p.clone()).unwrap();
        let read = |workspace: &str| {
            Request::ReadTerminal(serde_json::from_value(serde_json::json!({"workspaceId":workspace,"panelId":panel_id,"terminalSessionId":terminal_session_id})).unwrap())
        };
        assert!(matches!(
            client.call(read("a")).await.unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        if cancelled {
            assert!(broker.acknowledge_ui(ack.clone()).is_err());
            assert!(monitor.lock().unwrap().lease().is_none());
            assert!(matches!(
                client.call(read("b")).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::TargetNotFound,
                    ..
                }
            ));
            ack.result = OperationResult::Failure {
                code: ErrorCode::OutcomeUnknown,
            };
        } else {
            assert_eq!(monitor.lock().unwrap().lease(), lease.as_deref());
            assert!(matches!(
                client.call(read("b")).await.unwrap(),
                Reply::Ok { .. }
            ));
            let mut forged = ack.clone();
            if let OperationResult::PanelMoved(result) = &mut forged.result {
                result.destination.as_mut().unwrap().panels[0].terminal_session_id =
                    Some("forged".into());
            }
            assert!(broker.acknowledge_ui(forged).is_err());
        }
        broker.acknowledge_ui(ack).unwrap();
        let replayed = client
            .call(Request::RunTerminal(run.clone()))
            .await
            .unwrap();
        assert!(
            matches!(&replayed, Reply::Ok { data: Data::Operation { operation_id, .. }, .. } if operation_id == &run_id),
            "{replayed:?}"
        );
        let mut changed_run = run.clone();
        changed_run.command = "different-command".into();
        assert!(matches!(
            client
                .call(Request::RunTerminal(changed_run))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::IdempotencyConflict,
                ..
            }
        ));
        let mut fresh_run = run;
        fresh_run.request_key = "new-request-in-old-workspace".into();
        assert!(matches!(
            client.call(Request::RunTerminal(fresh_run)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        assert_eq!(dispatched.load(std::sync::atomic::Ordering::SeqCst), 1);
        if !cancelled {
            client
                .call(Request::CancelOperation(OperationInput {
                    operation_id: run_id,
                }))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while dispatched.load(std::sync::atomic::Ordering::SeqCst) != 2 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        let repeated = client
            .call(Request::MovePanel(input.clone()))
            .await
            .unwrap();
        let expected = if cancelled {
            "outcome_unknown"
        } else {
            "succeeded"
        };
        assert!(
            matches!(&repeated,Reply::Ok {data:Data::Operation {operation_id,state,..},..} if operation_id==&command.operation_id && state==expected),
            "{repeated:?}"
        );
        assert!(commands.try_recv().is_err());
        if !cancelled {
            let mut changed = input.clone();
            if let PanelMove::TransferTab { before_tab_id, .. } = &mut changed.movement {
                *before_tab_id = Some("target".into());
            }
            assert!(matches!(
                client.call(Request::MovePanel(changed)).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::IdempotencyConflict,
                    ..
                }
            ));
            // An ordinary domain move does not inherit the consumed migration permit.
            for panel in &mut p.panels {
                if panel.tab_id == *tab_id {
                    panel.workspace_id = "a".into();
                }
            }
            p.revision = "4".into();
            broker.publish(p.clone()).unwrap();
            assert!(monitor.lock().unwrap().lease().is_none());
            assert!(matches!(
                client.call(read("a")).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::TargetNotFound,
                    ..
                }
            ));
        }
        broker.shutdown().await;
    }
}

#[tokio::test]
async fn multiple_project_bindings_keep_roots_receipts_and_transfers_separate() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(first.join("first.txt"), b"first").unwrap();
    std::fs::write(second.join("second.txt"), b"second").unwrap();
    let broker = Broker::start(&temp.path().join("control")).unwrap();
    let mut p = projection(&broker, &first);
    p.workspaces
        .iter_mut()
        .find(|w| w.id == "foreign")
        .unwrap()
        .project_path = second.canonicalize().unwrap().to_string_lossy().into();
    p.panels = ["a", "foreign"]
        .into_iter()
        .map(|workspace| Panel {
            id: format!("file-{workspace}"),
            tab_id: format!("file-{workspace}"),
            workspace_id: workspace.into(),
            kind: "file".into(),
            title: "File".into(),
            terminal_session_id: None,
            browser_generation: None,
            android_device_id: None,
        })
        .collect();
    broker.publish(p.clone()).unwrap();
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |c| tx.send(c).map_err(std::io::Error::other)))
        .unwrap();
    let scopes = [
        "workspace.read",
        "workspace.write",
        "files.read",
        "panel.move",
        "panel.focus",
    ];
    let client = approved_scopes(&broker, &["a", "foreign"], &scopes).await;
    let other = approved_scopes(&broker, &["a"], &scopes).await;
    let connected = client
        .call(Request::Connect(ConnectInput {
            workspace_id: "a".into(),
        }))
        .await
        .unwrap();
    let Reply::Ok {
        data: Data::Connected { retry_epoch, .. },
        ..
    } = connected
    else {
        panic!("{connected:?}")
    };
    for (workspace, name) in [("a", "first.txt"), ("foreign", "second.txt")] {
        let reply = client
            .call(Request::FilesList(FilesListInput {
                workspace_id: workspace.into(),
                relative_directory: "".into(),
                limit: 100,
                cursor: None,
            }))
            .await
            .unwrap();
        let Reply::Ok {
            data: Data::FilesList(list),
            ..
        } = reply
        else {
            panic!("{reply:?}")
        };
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.entries[0].relative_path, name);
    }
    let denied = other
        .call(Request::FilesList(FilesListInput {
            workspace_id: "foreign".into(),
            relative_directory: "".into(),
            limit: 100,
            cursor: None,
        }))
        .await
        .unwrap();
    assert!(matches!(
        denied,
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    let transfer = client
        .call(Request::MovePanel(PanelMoveInput {
            workspace_id: "a".into(),
            movement: PanelMove::TransferTab {
                tab_id: "file-a".into(),
                target_workspace_id: "foreign".into(),
                before_tab_id: None,
            },
            expected_revision: p.revision.clone(),
            retry_epoch: retry_epoch.clone(),
            request_key: "cross-root-transfer".into(),
        }))
        .await
        .unwrap();
    assert!(matches!(
        transfer,
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    assert!(commands.try_recv().is_err());
    let mut operations = Vec::new();
    for (workspace, project) in [("a", "p"), ("foreign", "other")] {
        let input = WorkspaceRenameInput {
            workspace_id: workspace.into(),
            name: format!("renamed-{workspace}"),
            expected_revision: p.revision.clone(),
            retry_epoch: retry_epoch.clone(),
            request_key: "same-key".into(),
        };
        client
            .call(Request::RenameWorkspace(input.clone().into()))
            .await
            .unwrap();
        let command = commands.recv().await.unwrap();
        assert_eq!(command.project_id, project);
        broker
            .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
            .unwrap();
        p.revision = (p.revision.parse::<u64>().unwrap() + 1).to_string();
        p.workspaces
            .iter_mut()
            .find(|w| w.id == workspace)
            .unwrap()
            .name = input.name.clone();
        broker.publish(p.clone()).unwrap();
        broker
            .acknowledge_ui(UiAck {
                operation_id: command.operation_id.clone(),
                nonce: command.nonce.clone(),
                ui_epoch: p.ui_epoch.clone(),
                result: OperationResult::Workspace {
                    workspace_id: workspace.into(),
                    name: input.name,
                },
            })
            .unwrap();
        let reply = client
            .call(Request::Operation(
                OperationInput {
                    operation_id: command.operation_id.clone(),
                }
                .into(),
            ))
            .await
            .unwrap();
        assert!(
            matches!(reply, Reply::Ok {data:Data::Operation {ref state,..},..} if state=="succeeded")
        );
        operations.push(command.operation_id);
    }
    assert_ne!(operations[0], operations[1]);
    for (index, project) in ["p", "other"].into_iter().enumerate() {
        let reply = client
            .call(Request::Operation(OperationLookup::Key(
                OperationKeyInput {
                    project_id: Some(project.into()),
                    retry_epoch: retry_epoch.clone(),
                    request_key: "same-key".into(),
                    tool: "lomi_workspace_update".into(),
                },
            )))
            .await
            .unwrap();
        assert!(
            matches!(reply,Reply::Ok {data:Data::Operation {ref operation_id,..},..} if operation_id==&operations[index])
        );
    }
    for project_id in [None, Some("unapproved".into())] {
        let reply = client
            .call(Request::Operation(OperationLookup::Key(
                OperationKeyInput {
                    project_id,
                    retry_epoch: retry_epoch.clone(),
                    request_key: "same-key".into(),
                    tool: "lomi_workspace_update".into(),
                },
            )))
            .await
            .unwrap();
        assert!(matches!(
            reply,
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
    }
    let denied = other
        .call(Request::Operation(
            OperationInput {
                operation_id: operations[1].clone(),
            }
            .into(),
        ))
        .await
        .unwrap();
    assert!(matches!(
        denied,
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    // A changed projection cannot attach a previously granted ID to another root.
    p.revision = "4".into();
    let rebound = p.workspaces.iter_mut().find(|w| w.id == "a").unwrap();
    rebound.project_id = "other".into();
    rebound.project_path = second.canonicalize().unwrap().to_string_lossy().into();
    broker.publish(p.clone()).unwrap();
    let denied = client
        .call(Request::FilesList(FilesListInput {
            workspace_id: "a".into(),
            relative_directory: "".into(),
            limit: 100,
            cursor: None,
        }))
        .await
        .unwrap();
    assert!(matches!(
        denied,
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    // Replacing one granted directory cannot redirect its pinned file access.
    std::fs::rename(&second, temp.path().join("old-second")).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(second.join("replacement.txt"), b"not approved").unwrap();
    let denied = client
        .call(Request::FilesList(FilesListInput {
            workspace_id: "foreign".into(),
            relative_directory: "".into(),
            limit: 100,
            cursor: None,
        }))
        .await
        .unwrap();
    assert!(matches!(
        denied,
        Reply::Error {
            code: ErrorCode::RevisionConflict,
            ..
        }
    ));
    broker.shutdown().await;
}

#[tokio::test]
async fn project_open_requires_exact_settings_approval_and_verified_publication() {
    for case in [
        "success",
        "reject",
        "cancel",
        "replaced",
        "cancel-after-commit",
        "revoke",
        "revision",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("new-project");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("new.txt"), b"new approved root").unwrap();
        let broker = Broker::start(&temp.path().join("control")).unwrap();
        let mut p = projection(&broker, temp.path());
        broker.publish(p.clone()).unwrap();
        let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
        broker
            .set_ui_dispatch(Arc::new(move |c| tx.send(c).map_err(std::io::Error::other)))
            .unwrap();
        let scopes = [
            "workspace.read",
            "workspace.write",
            "panel.create",
            "project.open",
            "files.read",
        ];
        let client = approved_scopes(&broker, &["a"], &scopes).await;
        let unapproved = approved_scopes(&broker, &["a"], &["workspace.read", "files.read"]).await;
        let connected = client
            .call(Request::Connect(ConnectInput {
                workspace_id: "a".into(),
            }))
            .await
            .unwrap();
        let Reply::Ok {
            data: Data::Connected { retry_epoch, .. },
            ..
        } = connected
        else {
            panic!("{connected:?}")
        };
        let input = ProjectOpenInput {
            workspace_id: "a".into(),
            project_path: target.to_string_lossy().into(),
            name: "Approved new workspace".into(),
            expected_revision: "1".into(),
            retry_epoch,
            request_key: "new-root".into(),
        };
        assert!(matches!(
            unapproved
                .call(Request::OpenProject(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
        let queued = client
            .call(Request::OpenProject(input.clone()))
            .await
            .unwrap();
        assert!(
            matches!(queued,Reply::Ok{data:Data::Operation{ref state,..},..} if state=="queued"),
            "{case}: {queued:?}"
        );
        let command = commands.recv().await.unwrap();
        let UiAction::OpenProject(open) = command.action.clone() else {
            panic!("{command:?}")
        };
        assert_eq!(
            open.project_path,
            target.canonicalize().unwrap().to_string_lossy()
        );
        let file_input = FilesListInput {
            workspace_id: open.new_workspace_id.clone(),
            relative_directory: "".into(),
            limit: 100,
            cursor: None,
        };
        assert!(matches!(
            client
                .call(Request::FilesList(file_input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::TargetNotFound,
                ..
            }
        ));
        broker
            .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
            .unwrap();
        assert!(!broker
            .project_open_ready(&command.operation_id, &command.nonce, &p.ui_epoch)
            .unwrap());
        assert_eq!(
            broker.commit_project_open(&command.operation_id, &command.nonce, &p.ui_epoch),
            Err(ErrorCode::ScopeDenied)
        );
        let pending = serde_json::to_value(broker.overview().unwrap()).unwrap();
        assert_eq!(
            pending["pendingProjectOpens"][0]["projectPath"],
            open.project_path
        );
        assert_eq!(
            pending["pendingProjectOpens"][0]["workspaceName"],
            input.name
        );
        assert_eq!(
            pending["pendingProjectOpens"][0]["requestKey"],
            input.request_key
        );
        let result = ProjectOpened {
            anchor_workspace_id: "a".into(),
            project_id: open.project_id.clone(),
            project_path: open.project_path.clone(),
            workspace_id: open.new_workspace_id.clone(),
            panel_id: open.tab_id.clone(),
            name: open.name.clone(),
            opened: Some(true),
        };
        let ack = UiAck {
            operation_id: command.operation_id.clone(),
            nonce: command.nonce.clone(),
            ui_epoch: p.ui_epoch.clone(),
            result: OperationResult::ProjectOpened(Box::new(result)),
        };
        assert!(
            broker.acknowledge_ui(ack.clone()).is_err(),
            "{case}: early ACK accepted"
        );
        if case == "reject" {
            broker
                .decide_project_open(&command.operation_id, false)
                .unwrap();
        } else if case == "cancel" {
            client
                .call(Request::CancelOperation(OperationInput {
                    operation_id: command.operation_id.clone(),
                }))
                .await
                .unwrap();
        } else if case == "replaced" {
            std::fs::rename(&target, temp.path().join("old-project")).unwrap();
            std::fs::create_dir(&target).unwrap();
            assert_eq!(
                broker.decide_project_open(&command.operation_id, true),
                Err(ErrorCode::RevisionConflict)
            );
            client
                .call(Request::CancelOperation(OperationInput {
                    operation_id: command.operation_id.clone(),
                }))
                .await
                .unwrap();
        } else if case == "revision" {
            p.revision = "2".into();
            broker.publish(p.clone()).unwrap();
            assert_eq!(
                broker.decide_project_open(&command.operation_id, true),
                Err(ErrorCode::RevisionConflict)
            );
            client
                .call(Request::CancelOperation(OperationInput {
                    operation_id: command.operation_id.clone(),
                }))
                .await
                .unwrap();
        } else {
            broker
                .decide_project_open(&command.operation_id, true)
                .unwrap();
            assert!(broker
                .decide_project_open(&command.operation_id, true)
                .is_err());
            assert!(broker
                .project_open_ready(&command.operation_id, &command.nonce, &p.ui_epoch)
                .unwrap());
            broker
                .commit_project_open(&command.operation_id, &command.nonce, &p.ui_epoch)
                .unwrap();
            assert!(broker
                .commit_project_open(&command.operation_id, &command.nonce, &p.ui_epoch)
                .is_err());
            assert!(matches!(
                client
                    .call(Request::FilesList(file_input.clone()))
                    .await
                    .unwrap(),
                Reply::Error {
                    code: ErrorCode::TargetNotFound,
                    ..
                }
            ));
            if case == "cancel-after-commit" {
                client
                    .call(Request::CancelOperation(OperationInput {
                        operation_id: command.operation_id.clone(),
                    }))
                    .await
                    .unwrap();
            } else if case == "revoke" {
                broker.revoke();
                assert!(broker.acknowledge_ui(ack).is_err());
                broker.shutdown().await;
                continue;
            }
            p.revision = "2".into();
            // Opening remains addressable through its original grant after its anchor disappears.
            p.workspaces.retain(|w| w.project_id != "p");
            p.workspaces.push(Workspace {
                id: open.new_workspace_id.clone(),
                project_id: open.project_id.clone(),
                name: open.name.clone(),
                project_name: "new-project".into(),
                project_path: open.project_path.clone(),
                active_panel_id: Some(open.tab_id.clone()),
            });
            p.panels.push(Panel {
                id: open.tab_id.clone(),
                tab_id: open.tab_id.clone(),
                workspace_id: open.new_workspace_id.clone(),
                kind: "file".into(),
                title: "Untitled-1".into(),
                terminal_session_id: None,
                browser_generation: None,
                android_device_id: None,
            });
            broker.publish(p.clone()).unwrap();
            assert!(matches!(
                client
                    .call(Request::FilesList(file_input.clone()))
                    .await
                    .unwrap(),
                Reply::Error {
                    code: ErrorCode::TargetNotFound,
                    ..
                }
            ));
            let mut forged = ack.clone();
            if let OperationResult::ProjectOpened(result) = &mut forged.result {
                result.project_path = "/".into();
            }
            assert!(broker.acknowledge_ui(forged).is_err());
            if case == "cancel-after-commit" {
                assert!(
                    broker.acknowledge_ui(ack.clone()).is_err(),
                    "Cancelled publication extended the grant"
                );
                broker
                    .acknowledge_ui(UiAck {
                        result: OperationResult::Failure {
                            code: ErrorCode::OutcomeUnknown,
                        },
                        ..ack
                    })
                    .unwrap();
            } else {
                broker.acknowledge_ui(ack).unwrap();
                assert!(matches!(
                    client
                        .call(Request::FilesList(file_input.clone()))
                        .await
                        .unwrap(),
                    Reply::Ok {
                        data: Data::FilesList(_),
                        ..
                    }
                ));
                assert!(matches!(
                    unapproved
                        .call(Request::FilesList(file_input.clone()))
                        .await
                        .unwrap(),
                    Reply::Error {
                        code: ErrorCode::TargetNotFound,
                        ..
                    }
                ));
                let connected = client
                    .call(Request::Connect(ConnectInput {
                        workspace_id: open.new_workspace_id.clone(),
                    }))
                    .await
                    .unwrap();
                assert!(
                    matches!(connected,Reply::Ok{data:Data::Connected{ref project_id,..},..} if project_id==&open.project_id)
                );
            }
        }
        let receipt = client
            .call(Request::Operation(
                OperationInput {
                    operation_id: command.operation_id.clone(),
                }
                .into(),
            ))
            .await
            .unwrap();
        let Reply::Ok {
            data:
                Data::Operation {
                    ref state,
                    ref effect_state,
                    ref result,
                    ..
                },
            ..
        } = receipt
        else {
            panic!("{case}: {receipt:?}")
        };
        let (expected_state, expected_effect) = match case {
            "success" => ("succeeded", "complete"),
            "cancel-after-commit" => ("outcome_unknown", "unknown"),
            _ => ("cancelled", "none"),
        };
        assert_eq!(
            (state.as_str(), effect_state.as_str()),
            (expected_state, expected_effect),
            "{case}"
        );
        assert!(
            matches!(result,Some(OperationResult::ProjectOpened(value)) if value.opened==if case=="success"{Some(true)}else{None})
        );
        let replay = client
            .call(Request::OpenProject(input.clone()))
            .await
            .unwrap();
        assert!(
            matches!(replay,Reply::Ok{data:Data::Operation{ref operation_id,..},..} if operation_id==&command.operation_id)
        );
        let mut changed = input;
        changed.name = "different request".into();
        assert!(matches!(
            client.call(Request::OpenProject(changed)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::IdempotencyConflict,
                ..
            }
        ));
        assert!(commands.try_recv().is_err());
        if case != "success" {
            assert!(matches!(
                client.call(Request::FilesList(file_input)).await.unwrap(),
                Reply::Error {
                    code: ErrorCode::TargetNotFound,
                    ..
                }
            ));
            assert!(broker
                .decide_project_open(&command.operation_id, true)
                .is_err());
        }
        broker.shutdown().await;
    }
}
