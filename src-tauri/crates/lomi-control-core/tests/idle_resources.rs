//! Opt-in measurement of the production broker without desktop renderer costs.
#![cfg(target_os = "macos")]
use lomi_control_core::{broker::Broker, client::Client};
use lomi_control_protocol::control::*;
use std::time::{Duration, Instant};

fn cpu_seconds() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
    let usage = unsafe { usage.assume_init() };
    usage.ru_utime.tv_sec as f64
        + usage.ru_stime.tv_sec as f64
        + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1_000_000.
}

#[tokio::test]
#[ignore = "Ten-minute isolated broker CPU measurement; run as the only test"]
async fn connected_idle_broker_stays_below_one_percent_of_one_core() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker
        .publish(Projection {
            ui_epoch: broker.register_ui().unwrap(),
            revision: "1".into(),
            workspaces: vec![Workspace {
                id: "workspace".into(),
                project_id: "project".into(),
                name: "Idle fixture".into(),
                project_name: "Fixture".into(),
                active_panel_id: None,
                project_path: root.path().canonicalize().unwrap().to_string_lossy().into(),
            }],
            ..Projection::default()
        })
        .unwrap();
    let endpoint = broker.endpoint.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    let connecting = tokio::spawn(async move {
        Client::connect(&endpoint, "Idle resource fixture", |id| {
            send.send(id).unwrap();
        })
        .await
    });
    let id = tokio::time::timeout(Duration::from_secs(5), receive)
        .await
        .unwrap()
        .unwrap();
    broker
        .approve_terminal_profile(
            &id,
            &["workspace".into()],
            &["workspace.read".into()],
            &[],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();
    let client = connecting.await.unwrap().unwrap();
    assert!(matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: "workspace".into()
            }))
            .await
            .unwrap(),
        Reply::Ok { .. }
    ));
    tokio::time::sleep(Duration::from_secs(10)).await;
    let start = Instant::now();
    let before = cpu_seconds();
    tokio::time::sleep(Duration::from_secs(600)).await;
    let cpu = cpu_seconds() - before;
    let elapsed = start.elapsed().as_secs_f64();
    let percent = cpu / elapsed * 100.;
    // This response must use the original authenticated connection after idle.
    assert!(matches!(
        client
            .call(Request::Connect(ConnectInput {
                workspace_id: "workspace".into()
            }))
            .await
            .unwrap(),
        Reply::Ok { .. }
    ));
    drop(client);
    broker.shutdown().await;
    let report = serde_json::json!({"elapsedSeconds":elapsed,"cpuSeconds":cpu,"cpuPercentOneCore":percent,"budgetPercent":1,"passed":percent<1.,"scope":"Production broker + authenticated IPC client in one isolated test process; excludes desktop/renderer/helper/model","connectionAliveAfterIdle":true});
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if let Some(path) = std::env::var_os("LOMI_MCP_IDLE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(percent < 1., "Idle broker exceeded budget: {report}");
}
