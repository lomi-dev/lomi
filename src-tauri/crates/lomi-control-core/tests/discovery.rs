#![cfg(unix)]

use lomi_control_core::{
    broker::{Broker, Endpoint},
    client::Client,
    discovery,
};
use lomi_control_protocol::{
    control::{Data, Projection, Reply, Request, Workspace},
    EmptyInput, ErrorCode,
};
use std::{path::Path, sync::Arc, time::Duration};

fn publish_ui(broker: &Broker, root: &Path) {
    let allowed_project = root.join("allowed-project");
    let other_project = root.join("other-project");
    std::fs::create_dir_all(&allowed_project).unwrap();
    std::fs::create_dir_all(&other_project).unwrap();
    broker
        .publish(Projection {
            ui_epoch: broker.register_ui().unwrap(),
            revision: "1".into(),
            workspaces: [
                ("allowed", "allowed-project", allowed_project),
                ("other", "other-project", other_project),
            ]
            .into_iter()
            .map(|(id, project_id, project_path)| Workspace {
                id: id.into(),
                project_id: project_id.into(),
                name: id.into(),
                project_name: project_id.into(),
                active_panel_id: None,
                project_path: project_path
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into(),
            })
            .collect(),
            ..Projection::default()
        })
        .unwrap();
}

async fn pair(broker: &Arc<Broker>, endpoint: &Endpoint, workspace_ids: &[&str]) -> Client {
    let endpoint = endpoint.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    let connection = tokio::spawn(async move {
        Client::connect(&endpoint, "Discovery regression", |id| {
            send.send(id).unwrap();
        })
        .await
    });
    let request_id = tokio::time::timeout(Duration::from_secs(2), receive)
        .await
        .unwrap()
        .unwrap();
    assert!(broker
        .overview()
        .unwrap()
        .pending
        .iter()
        .any(|pending| pending.id == request_id));
    broker
        .approve_scopes(
            &request_id,
            &workspace_ids
                .iter()
                .map(|id| (*id).into())
                .collect::<Vec<_>>(),
            &["workspace.read".into()],
        )
        .unwrap();
    connection.await.unwrap().unwrap()
}

fn assert_same_endpoint(actual: &Endpoint, expected: &Endpoint) {
    assert_eq!(actual.instance_id, expected.instance_id);
    assert_eq!(actual.endpoint, expected.endpoint);
    assert_eq!(actual.broker_sha256, expected.broker_sha256);
    assert_eq!(actual.ipc_version, expected.ipc_version);
}

#[tokio::test]
async fn signed_discovery_tracks_broker_restart_without_reusing_old_connections_or_grants() {
    let temp = tempfile::tempdir().unwrap();
    let state_root = temp.path().join("agent-control-state");
    let registration_root = temp.path().join("agent-control-registration");
    let registration_file = registration_root.join(discovery::FILE);

    let broker = Broker::start(&state_root).unwrap();
    publish_ui(&broker, temp.path());
    let first_endpoint = broker.overview().unwrap().endpoint;
    let pinned_key = discovery::ensure_public_key(&registration_root).unwrap();
    discovery::publish(&registration_root, &first_endpoint).unwrap();
    let first_registration = discovery::read(&registration_file, &pinned_key).unwrap();
    assert_same_endpoint(&first_registration, &first_endpoint);

    let old_client = pair(&broker, &first_registration, &["allowed"]).await;
    assert!(matches!(
        old_client.call(Request::Status(EmptyInput {})).await.unwrap(),
        Reply::Ok {
            data: Data::Status {
                instance_id: Some(instance_id),
                ..
            },
            ..
        } if instance_id == first_endpoint.instance_id
    ));

    broker.shutdown().await;
    drop(broker);

    let restarted = Broker::start(&state_root).unwrap();
    publish_ui(&restarted, temp.path());
    let second_endpoint = restarted.overview().unwrap().endpoint;
    assert_ne!(second_endpoint.instance_id, first_endpoint.instance_id);
    assert_ne!(second_endpoint.broker_sha256, first_endpoint.broker_sha256);
    assert_ne!(second_endpoint.endpoint, first_endpoint.endpoint);

    discovery::publish(&registration_root, &second_endpoint).unwrap();
    assert_eq!(
        discovery::ensure_public_key(&registration_root).unwrap(),
        pinned_key
    );
    let second_registration = discovery::read(&registration_file, &pinned_key).unwrap();
    assert_same_endpoint(&second_registration, &second_endpoint);
    assert!(restarted.overview().unwrap().sessions.is_empty());
    assert!(old_client
        .call(Request::Status(EmptyInput {}))
        .await
        .is_err());

    let client = pair(&restarted, &second_registration, &["allowed"]).await;
    assert!(matches!(
        client.call(Request::Status(EmptyInput {})).await.unwrap(),
        Reply::Ok {
            data: Data::Status {
                instance_id: Some(instance_id),
                capabilities,
                ..
            },
            ..
        } if instance_id == second_endpoint.instance_id
            && capabilities.iter().any(|capability| capability.name == "workspace.read" && capability.authorized)
            && capabilities.iter().any(|capability| capability.name == "files.read" && !capability.authorized)
    ));
    assert!(matches!(
        client
            .call(Request::Workspaces(lomi_control_protocol::control::ListInput {
                limit: 20,
                cursor: None,
            }))
            .await
            .unwrap(),
        Reply::Ok {
            data: Data::Workspaces { items, .. },
            ..
        } if items.iter().map(|workspace| workspace.id.as_str()).collect::<Vec<_>>() == ["allowed"]
    ));
    assert!(matches!(
        client
            .call(Request::Connect(
                lomi_control_protocol::control::ConnectInput {
                    workspace_id: "other".into(),
                }
            ))
            .await
            .unwrap(),
        Reply::Error {
            code: ErrorCode::TargetNotFound,
            ..
        }
    ));

    drop(old_client);
    drop(client);
    restarted.shutdown().await;
}
