use super::*;
use lomi_control_core::broker::{
    AndroidManagementPlan, AndroidPrepareInput, AndroidSetupRead, AndroidTerms,
};
use std::sync::atomic::{AtomicUsize, Ordering};

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
fn operation(reply: Reply) -> String {
    match reply {
        Reply::Ok {
            data:
                Data::Operation {
                    operation_id,
                    state,
                    ..
                },
            ..
        } if state == "awaiting_user" => operation_id,
        reply => panic!("{reply:?}"),
    }
}
async fn settled(client: &Client, id: &str) -> Reply {
    for _ in 0..100 {
        let reply = client
            .call(Request::Operation(
                OperationInput {
                    operation_id: id.into(),
                }
                .into(),
            ))
            .await
            .unwrap();
        if matches!(&reply,Reply::Ok{data:Data::Operation{state,..},..} if !matches!(state.as_str(),"queued"|"running"|"cancelling"))
        {
            return reply;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Android management did not settle")
}
#[tokio::test]
async fn android_setup_requires_exact_human_terms_and_deduplicates_before_native_prepare() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.publish(projection(&broker, root.path())).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let prepared = Arc::new(AtomicUsize::new(0));
    let (count, plans) = (calls.clone(), prepared.clone());
    let device = "00000000-0000-4000-8000-000000000001";
    broker
        .set_android_setup_dispatch(Arc::new(move |input, _, check| {
            check()?;
            let number = plans.fetch_add(1, Ordering::SeqCst);
            let create = matches!(
                &input,
                AndroidPrepareInput::Device(AndroidDeviceAction::Create { .. })
            );
            let view = AndroidPreparedPlan {
                plan_id: format!("plan-{number}"),
                revision: "a".repeat(64),
                downloads: vec![],
                licenses: if create {
                    vec![]
                } else {
                    vec![AndroidLicenseSummary {
                        id: "terms".into(),
                        digest: "b".repeat(64),
                    }]
                },
                download_bytes: 0,
                target: "Fixture Android".into(),
                expires_in_seconds: 600,
            };
            let count = count.clone();
            let licenses = if create {
                vec![]
            } else {
                vec![AndroidTerms {
                    id: "terms".into(),
                    digest: "b".repeat(64),
                    text: "Fixture provider terms".into(),
                }]
            };
            let plan = AndroidManagementPlan {
                view: view.clone(),
                licenses,
                present: Arc::new(|_| {}),
                apply: Arc::new(move |request| {
                    request.mark_dispatching()?;
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(AndroidManagementResult {
                        workspace_id: request.workspace_id,
                        native_operation_id: "00000000-0000-4000-8000-000000000009".into(),
                        device_id: create.then(|| device.into()),
                        devices_revision: Some("2".into()),
                        manifest_revision: Some("1".into()),
                    })
                }),
            };
            Ok(AndroidSetupRead {
                view: AndroidSetupView::Prepared(view),
                plan: Some(plan),
            })
        }))
        .unwrap();
    let scopes = [
        "workspace.read",
        "android.read",
        "android.setup",
        "android.manage",
    ];
    let client = approved_domains(&broker, &["a"], &scopes, &[]).await;
    let foreign = approved_domains(&broker, &["a"], &scopes, &[]).await;
    let retry_epoch = epoch(&client).await;
    let prepare = Request::AndroidSetupPlan(AndroidSetupPlanInput {
        workspace_id: "a".into(),
        action: AndroidSetupQuery::Prepare {
            catalog_revision: "c".repeat(64),
            packages: vec![],
            prepare_tools: true,
        },
    });
    let Reply::Ok {
        data: Data::AndroidSetup(view),
        ..
    } = client.call(prepare).await.unwrap()
    else {
        panic!()
    };
    let AndroidSetupView::Prepared(plan) = *view else {
        panic!()
    };
    let mut input = AndroidSetupApplyInput {
        workspace_id: "a".into(),
        plan_id: plan.plan_id,
        plan_revision: plan.revision.clone(),
        retry_epoch: retry_epoch.clone(),
        request_key: "apply".into(),
    };
    let mut other = input.clone();
    other.retry_epoch = epoch(&foreign).await;
    assert!(matches!(
        foreign
            .call(Request::AndroidSetupApply(other))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));
    input.plan_revision = "d".repeat(64);
    assert!(matches!(
        client
            .call(Request::AndroidSetupApply(input.clone()))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::RevisionConflict,
            ..
        }
    ));
    input.plan_revision = plan.revision.clone();
    let op = operation(
        client
            .call(Request::AndroidSetupApply(input.clone()))
            .await
            .unwrap(),
    );
    assert!(broker
        .decide_android_management(&op, &"d".repeat(64), true, vec!["b".repeat(64)], None)
        .is_err());
    assert!(broker
        .decide_android_management(&op, &plan.revision, true, vec![], None)
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    broker
        .decide_android_management(&op, &plan.revision, true, vec!["b".repeat(64)], None)
        .unwrap();
    assert!(
        matches!(settled(&client,&op).await,Reply::Ok{data:Data::Operation{state,effect_state,..},..} if state=="succeeded" && effect_state=="complete")
    );
    assert!(
        matches!(client.call(Request::AndroidSetupApply(input)).await.unwrap(),Reply::Ok{data:Data::Operation{operation_id,state,..},..} if operation_id==op && state=="succeeded")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let create = AndroidDeviceManageInput {
        workspace_id: "a".into(),
        retry_epoch,
        request_key: "create".into(),
        action: AndroidDeviceAction::Create {
            expected_devices_revision: "1".into(),
            name: "Fixture".into(),
            image: "fixture".into(),
            profile: "fixture".into(),
            hardware: AndroidHardware {
                ram_mib: 2560,
                cpu_count: 2,
                data_gib: 4,
                gpu: AndroidGpu::Host,
                quick_boot: false,
            },
        },
    };
    let op = operation(
        client
            .call(Request::AndroidDeviceManage(create.clone()))
            .await
            .unwrap(),
    );
    broker
        .decide_android_management(&op, &plan.revision, true, vec![], None)
        .unwrap();
    assert!(
        matches!(settled(&client,&op).await,Reply::Ok{data:Data::Operation{state,..},..} if state=="succeeded")
    );
    let before = prepared.load(Ordering::SeqCst);
    client
        .call(Request::AndroidDeviceManage(create))
        .await
        .unwrap();
    assert_eq!(
        prepared.load(Ordering::SeqCst),
        before,
        "Exact retry must precede native revision validation"
    );
    assert_eq!(
        broker
            .overview()
            .unwrap()
            .sessions
            .iter()
            .filter(|s| s.android_device_ids.contains(&device.to_string()))
            .count(),
        1,
        "Only the creator receives the new device grant"
    );
    let denied = AndroidDeviceManageInput {
        workspace_id: "a".into(),
        retry_epoch: epoch(&foreign).await,
        request_key: "delete-foreign".into(),
        action: AndroidDeviceAction::Delete {
            expected_devices_revision: "2".into(),
            device_id: device.into(),
            generation: None,
            confirmation: "Fixture".into(),
        },
    };
    assert!(matches!(
        foreign
            .call(Request::AndroidDeviceManage(denied))
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
async fn android_management_revocation_preserves_uncertainty_and_never_replays_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let projection = projection(&broker, root.path());
    broker.publish(projection.clone()).unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicUsize::new(0));
    let (calls, ends) = (started.clone(), stopped.clone());
    broker
        .set_android_setup_dispatch(Arc::new(move |_, _, check| {
            check()?;
            let view = AndroidPreparedPlan {
                plan_id: "job".into(),
                revision: "a".repeat(64),
                downloads: vec![],
                licenses: vec![],
                download_bytes: 0,
                target: "Fixture cleanup".into(),
                expires_in_seconds: 600,
            };
            let (calls, ends) = (calls.clone(), ends.clone());
            Ok(AndroidSetupRead {
                view: AndroidSetupView::Prepared(view.clone()),
                plan: Some(AndroidManagementPlan {
                    view,
                    licenses: vec![],
                    present: Arc::new(|_| {}),
                    apply: Arc::new(move |request| {
                        request.mark_dispatching()?;
                        calls.fetch_add(1, Ordering::SeqCst);
                        for _ in 0..500 {
                            if request.permit.check().is_err() {
                                ends.fetch_add(1, Ordering::SeqCst);
                                return Err(ErrorCode::ControlRevoked);
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(ErrorCode::DeadlineExceeded)
                    }),
                }),
            })
        }))
        .unwrap();
    let client = approved_domains(
        &broker,
        &["a"],
        &["workspace.read", "android.read", "android.setup"],
        &[],
    )
    .await;
    let retry_epoch = epoch(&client).await;
    for index in 0..3 {
        let input = AndroidDeviceManageInput {
            workspace_id: "a".into(),
            action: AndroidDeviceAction::Cleanup,
            retry_epoch: retry_epoch.clone(),
            request_key: format!("cleanup-{index}"),
        };
        let op = operation(
            client
                .call(Request::AndroidDeviceManage(input.clone()))
                .await
                .unwrap(),
        );
        if index == 0 {
            broker
                .decide_android_management(&op, &"a".repeat(64), false, vec![], None)
                .unwrap();
            assert!(
                matches!(settled(&client,&op).await,Reply::Ok{data:Data::Operation{state,effect_state,..},..} if state=="cancelled" && effect_state=="none")
            );
            assert_eq!(started.load(Ordering::SeqCst), 0);
            continue;
        }
        broker
            .decide_android_management(&op, &"a".repeat(64), true, vec![], None)
            .unwrap();
        for _ in 0..100 {
            if started.load(Ordering::SeqCst) == index {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(started.load(Ordering::SeqCst), index);
        if index == 1 {
            client
                .call(Request::CancelOperation(OperationInput {
                    operation_id: op.clone(),
                }))
                .await
                .unwrap();
            assert!(
                matches!(settled(&client,&op).await,Reply::Ok{data:Data::Operation{state,effect_state,..},..} if state=="outcome_unknown" && effect_state=="unknown")
            );
            assert!(
                matches!(client.call(Request::AndroidDeviceManage(input)).await.unwrap(),Reply::Ok{data:Data::Operation{operation_id,state,..},..} if operation_id==op && state=="outcome_unknown")
            );
        } else {
            let mut closed = projection.clone();
            closed.revision = "2".into();
            closed.workspaces.retain(|w| w.id != "a");
            closed.panels.retain(|p| p.workspace_id != "a");
            closed.focused_panel_id = None;
            broker.publish(closed).unwrap();
        }
        for _ in 0..100 {
            if stopped.load(Ordering::SeqCst) == index {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            stopped.load(Ordering::SeqCst),
            index,
            "The exact native permit must end after cancellation or workspace removal"
        );
    }
    broker.shutdown().await;
}
