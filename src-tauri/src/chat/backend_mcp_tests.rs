#![cfg(unix)]

use super::{tests, Backend};
use crate::chat::store::{Config, Loaded, Start};
use lomi_control_core::broker::Broker;
use lomi_control_protocol::control::{Projection, Workspace};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use tauri::ipc::Channel;

fn publish_fixture_workspaces(broker: &Broker, root: &Path) {
    let allowed = root.join("allowed-workspace");
    let denied = root.join("denied-workspace");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&denied).unwrap();

    let epoch = broker.register_ui().unwrap();
    broker
        .publish(Projection {
            ui_epoch: epoch,
            revision: "1".into(),
            workspaces: vec![
                Workspace {
                    id: "allowed-workspace".into(),
                    active_panel_id: None,
                    project_id: "allowed-project".into(),
                    name: "Authorized fixture".into(),
                    project_name: "Authorized project".into(),
                    project_path: allowed.canonicalize().unwrap().to_string_lossy().into(),
                },
                Workspace {
                    id: "denied-workspace".into(),
                    active_panel_id: None,
                    project_id: "denied-project".into(),
                    name: "Unapproved fixture".into(),
                    project_name: "Unapproved project".into(),
                    project_path: denied.canonicalize().unwrap().to_string_lossy().into(),
                },
            ],
            ..Projection::default()
        })
        .unwrap();
}

fn install_endpoint(backend: &mut Arc<Backend>, broker: &Broker) {
    let endpoint = broker.endpoint.clone();
    Arc::get_mut(backend)
        .expect("the fixture backend must not be shared before setup")
        .set_test_endpoint_provider(move || Some(endpoint.clone()));
}

fn use_model(backend: &Backend, start: &mut Start, model: &str) {
    let mut services = backend.services.lock().unwrap();
    let store = services.store.as_mut().unwrap();
    let conversation = store.conversation(&start.conversation_id).unwrap();
    let config = Config {
        model: model.into(),
        ..conversation.config
    };
    start.expected_revision = store
        .configure(&start.conversation_id, &config, conversation.revision)
        .unwrap()
        .revision;
}

fn next_start(backend: &Backend, conversation_id: &str, request_id: &str, model: &str) -> Start {
    let mut services = backend.services.lock().unwrap();
    let store = services.store.as_mut().unwrap();
    let conversation = store.conversation(conversation_id).unwrap();
    let draft = store.draft(conversation_id).unwrap();
    let text = "Check the saved MCP tool metadata.";
    let draft = store
        .save_draft(conversation_id, text, draft.revision)
        .unwrap();
    let config = Config {
        model: model.into(),
        ..conversation.config
    };
    let conversation = store
        .configure(conversation_id, &config, conversation.revision)
        .unwrap();
    Start {
        request_id: request_id.into(),
        conversation_id: conversation_id.into(),
        assistant_id: format!("assistant-{request_id}"),
        user_id: format!("user-{request_id}"),
        expected_revision: conversation.revision,
        draft_revision: draft.revision,
        action: "send".into(),
        target_id: None,
        text: text.into(),
    }
}

async fn pending_pairing(broker: &Broker, label: &str) -> String {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(pairing) = broker
                .overview()
                .unwrap()
                .pending
                .into_iter()
                .find(|pending| pending.client_label == label)
            {
                return pairing.id;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Chat MCP did not request pairing")
}

async fn wait_connected(session: &lomi_mcp::chat::ChatSession) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while session.connection_status() != "connected" {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Chat MCP did not connect after approval");
}

fn load(backend: &Backend, conversation_id: &str) -> Loaded {
    backend
        .services
        .lock()
        .unwrap()
        .store
        .as_ref()
        .unwrap()
        .load(conversation_id, 0)
        .unwrap()
}

fn message_text(message: &Value) -> String {
    message["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "text")
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

fn tool_parts(message: &Value) -> Vec<Value> {
    message["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "dynamic-tool")
        .cloned()
        .collect()
}

fn assistant(backend: &Backend, conversation_id: &str, assistant_id: &str) -> Value {
    let services = backend.services.lock().unwrap();
    serde_json::to_value(
        services
            .store
            .as_ref()
            .unwrap()
            .message(conversation_id, assistant_id)
            .unwrap(),
    )
    .unwrap()
}

fn channel_with_events() -> (Channel<Value>, mpsc::Receiver<Value>) {
    let (sender, receiver) = mpsc::channel();
    let channel = Channel::<Value>::new(move |body: tauri::ipc::InvokeResponseBody| {
        let value = match body {
            tauri::ipc::InvokeResponseBody::Json(value) => serde_json::from_str(&value).unwrap(),
            tauri::ipc::InvokeResponseBody::Raw(value) => serde_json::from_slice(&value).unwrap(),
        };
        let _ = sender.send(value);
        Ok(())
    });
    (channel, receiver)
}

fn wait_for_packet(
    receiver: &mpsc::Receiver<Value>,
    mut predicate: impl FnMut(&Value) -> bool,
    timeout: Duration,
) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "Timed out waiting for Chat event");
        let packet = receiver
            .recv_timeout(remaining)
            .expect("Chat event stream ended before the expected packet");
        if predicate(&packet) {
            return packet;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fixture_model_uses_authorized_broker_and_sqlite_roundtrips_tool_metadata() {
    let (profile, mut backend) = tests::setup();
    let broker = Broker::start(&profile.path().join("control")).unwrap();
    publish_fixture_workspaces(&broker, profile.path());
    install_endpoint(&mut backend, &broker);

    let mut start = tests::input(&backend, "mcp-roundtrip");
    use_model(&backend, &mut start, "fixture-mcp");
    let conversation = {
        let services = backend.services.lock().unwrap();
        services
            .store
            .as_ref()
            .unwrap()
            .conversation(&start.conversation_id)
            .unwrap()
    };
    let session = backend.mcp_session(&conversation);
    session.call("lomi_status", json!({})).await;
    let pairing = pending_pairing(&broker, "Lomi Chat AI mcp-roundtrip").await;
    broker
        .approve_scopes(
            &pairing,
            &["allowed-workspace".into()],
            &["workspace.read".into()],
        )
        .unwrap();
    wait_connected(&session).await;

    let accepted = backend.start(start.clone(), tests::channel()).unwrap();
    assert_eq!(accepted["repeated"], false);
    let request = backend.requests.lock().unwrap()[&start.request_id].clone();
    tests::finished(&request).unwrap();

    let first = assistant(&backend, &start.conversation_id, &start.assistant_id);
    assert_eq!(first["status"], "completed");
    let text = message_text(&first);
    assert!(
        text.contains("Workspace result: Authorized fixture"),
        "{text}"
    );
    let saved_tools = tool_parts(&first);
    assert_eq!(saved_tools.len(), 1);
    let tool = &saved_tools[0];
    assert_eq!(tool["toolName"], "lomi_workspace_list");
    assert_eq!(tool["input"], json!({}));
    assert_eq!(tool["state"], "output-available");
    assert_ne!(tool["output"]["isError"], true);
    let workspaces = tool["output"]["structuredContent"]["data"]["items"]
        .as_array()
        .expect("Broker returned a structured workspace list");
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0]["id"], "allowed-workspace");
    assert_eq!(workspaces[0]["name"], "Authorized fixture");
    assert!(!tool["output"].to_string().contains("Unapproved fixture"));
    let sentinel = json!({"google":{"thoughtSignature":"fixture-tool-signature"}});
    assert_eq!(tool["callProviderMetadata"], sentinel);

    let replay = backend.start(start.clone(), tests::channel()).unwrap();
    assert_eq!(replay["repeated"], true);
    assert_eq!(
        tool_parts(&assistant(
            &backend,
            &start.conversation_id,
            &start.assistant_id
        ))
        .len(),
        1
    );

    let second = next_start(
        &backend,
        &start.conversation_id,
        "mcp-history",
        "fixture-history",
    );
    let accepted = backend.start(second.clone(), tests::channel()).unwrap();
    assert_eq!(accepted["repeated"], false);
    let request = backend.requests.lock().unwrap()[&second.request_id].clone();
    tests::finished(&request).unwrap();
    let history = assistant(&backend, &second.conversation_id, &second.assistant_id);
    assert_eq!(history["status"], "completed");
    assert!(message_text(&history).contains("metadata-roundtrip-ok"));
    assert!(tool_parts(&history).is_empty());
    let saved = load(&backend, &start.conversation_id);
    assert_eq!(
        saved.messages.len(),
        4,
        "user/assistant history was retained"
    );
    assert_eq!(
        saved.request.as_ref().unwrap()["status"],
        "completed",
        "SQLite reports the latest generation status"
    );

    backend.stop();
    broker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sequential_tool_queue_accepts_the_next_call_before_result_end_is_observed() {
    let (profile, mut backend) = tests::setup();
    let broker = Broker::start(&profile.path().join("control")).unwrap();
    publish_fixture_workspaces(&broker, profile.path());
    install_endpoint(&mut backend, &broker);

    let mut start = tests::input(&backend, "mcp-sequential");
    use_model(&backend, &mut start, "fixture-mcp-parallel");
    let conversation = {
        let services = backend.services.lock().unwrap();
        services
            .store
            .as_ref()
            .unwrap()
            .conversation(&start.conversation_id)
            .unwrap()
    };
    let session = backend.mcp_session(&conversation);
    session.call("lomi_status", json!({})).await;
    let pairing = pending_pairing(&broker, "Lomi Chat AI mcp-sequential").await;
    broker
        .approve_scopes(
            &pairing,
            &["allowed-workspace".into()],
            &["workspace.read".into()],
        )
        .unwrap();
    wait_connected(&session).await;

    backend.start(start.clone(), tests::channel()).unwrap();
    let request = backend.requests.lock().unwrap()[&start.request_id].clone();
    tests::finished(&request).unwrap();
    let response = assistant(&backend, &start.conversation_id, &start.assistant_id);
    assert_eq!(response["status"], "completed");
    let tools = tool_parts(&response);
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["toolCallId"], "fixture-tool-1");
    assert_eq!(tools[1]["toolCallId"], "fixture-tool-2");
    assert!(tools.iter().all(|tool| tool["state"] == "output-available"));

    backend.stop();
    broker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tool_denied_before_pairing_is_not_replayed_after_later_approval() {
    let (profile, mut backend) = tests::setup();
    let broker = Broker::start(&profile.path().join("control")).unwrap();
    publish_fixture_workspaces(&broker, profile.path());
    install_endpoint(&mut backend, &broker);

    let mut start = tests::input(&backend, "mcp-unpaired");
    use_model(&backend, &mut start, "fixture-mcp");
    let accepted = backend.start(start.clone(), tests::channel()).unwrap();
    assert_eq!(accepted["repeated"], false);
    let request = backend.requests.lock().unwrap()[&start.request_id].clone();
    tests::finished(&request).unwrap();

    let pairing = pending_pairing(&broker, "Lomi Chat AI mcp-unpaired").await;
    let first = assistant(&backend, &start.conversation_id, &start.assistant_id);
    assert_eq!(first["status"], "completed");
    let tools = tool_parts(&first);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["toolName"], "lomi_workspace_list");
    assert_eq!(tools[0]["state"], "output-available");
    assert_eq!(tools[0]["output"]["isError"], true);
    assert!(message_text(&first).contains("Workspace result missing"));
    assert_eq!(broker.overview().unwrap().sessions.len(), 0);

    broker
        .approve_scopes(
            &pairing,
            &["allowed-workspace".into()],
            &["workspace.read".into()],
        )
        .unwrap();
    let conversation = {
        let services = backend.services.lock().unwrap();
        services
            .store
            .as_ref()
            .unwrap()
            .conversation(&start.conversation_id)
            .unwrap()
    };
    wait_connected(&backend.mcp_session(&conversation)).await;
    let replay = backend.start(start.clone(), tests::channel()).unwrap();
    assert_eq!(replay["repeated"], true);
    let after_approval = assistant(&backend, &start.conversation_id, &start.assistant_id);
    assert_eq!(tool_parts(&after_approval).len(), 1);
    assert_eq!(after_approval, first);

    backend.stop();
    broker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoked_broker_session_fails_closed_without_reconnecting_or_replaying() {
    let (profile, mut backend) = tests::setup();
    let broker = Broker::start(&profile.path().join("control")).unwrap();
    publish_fixture_workspaces(&broker, profile.path());
    install_endpoint(&mut backend, &broker);

    let mut start = tests::input(&backend, "mcp-revoked");
    use_model(&backend, &mut start, "fixture-mcp");
    let conversation = {
        let services = backend.services.lock().unwrap();
        services
            .store
            .as_ref()
            .unwrap()
            .conversation(&start.conversation_id)
            .unwrap()
    };
    let session = backend.mcp_session(&conversation);
    session.call("lomi_status", json!({})).await;
    let pairing = pending_pairing(&broker, "Lomi Chat AI mcp-revoked").await;
    broker
        .approve_scopes(
            &pairing,
            &["allowed-workspace".into()],
            &["workspace.read".into()],
        )
        .unwrap();
    wait_connected(&session).await;
    backend.start(start.clone(), tests::channel()).unwrap();
    let request = backend.requests.lock().unwrap()[&start.request_id].clone();
    tests::finished(&request).unwrap();
    assert_eq!(
        tool_parts(&assistant(
            &backend,
            &start.conversation_id,
            &start.assistant_id
        ))
        .len(),
        1
    );

    broker.revoke();
    let denied = session.call("lomi_workspace_list", json!({})).await;
    assert_eq!(denied["isError"], true);
    assert!(session.is_revoked());
    let no_reconnect = session.call("lomi_workspace_list", json!({})).await;
    assert_eq!(no_reconnect["isError"], true);
    assert!(no_reconnect.to_string().contains("not dispatched"));
    assert!(broker.overview().unwrap().sessions.is_empty());

    backend.stop();
    broker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_after_broker_result_persists_once_and_same_request_does_not_replay() {
    let (profile, mut backend) = tests::setup();
    let broker = Broker::start(&profile.path().join("control")).unwrap();
    publish_fixture_workspaces(&broker, profile.path());
    install_endpoint(&mut backend, &broker);

    let mut start = tests::input(&backend, "mcp-cancel");
    use_model(&backend, &mut start, "fixture-mcp-cancel");
    let conversation = {
        let services = backend.services.lock().unwrap();
        services
            .store
            .as_ref()
            .unwrap()
            .conversation(&start.conversation_id)
            .unwrap()
    };
    let session = backend.mcp_session(&conversation);
    session.call("lomi_status", json!({})).await;
    let pairing = pending_pairing(&broker, "Lomi Chat AI mcp-cancel").await;
    broker
        .approve_scopes(
            &pairing,
            &["allowed-workspace".into()],
            &["workspace.read".into()],
        )
        .unwrap();
    wait_connected(&session).await;

    let (channel, events) = channel_with_events();
    backend.start(start.clone(), channel).unwrap();
    let request = backend.requests.lock().unwrap()[&start.request_id].clone();
    wait_for_packet(
        &events,
        |packet| {
            packet["chunk"]["type"] == "text-delta" && packet["chunk"]["delta"] == "post-tool:"
        },
        Duration::from_secs(8),
    );
    backend.cancel(&start.request_id).unwrap();
    tests::finished(&request).unwrap();

    let cancelled = assistant(&backend, &start.conversation_id, &start.assistant_id);
    assert_eq!(cancelled["status"], "cancelled");
    assert!(message_text(&cancelled).contains("post-tool:"));
    let tools = tool_parts(&cancelled);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["state"], "output-available");
    assert_eq!(
        tools[0]["output"]["structuredContent"]["data"]["items"][0]["id"],
        "allowed-workspace"
    );

    let replay = backend.start(start.clone(), tests::channel()).unwrap();
    assert_eq!(replay["repeated"], true);
    let after_replay = assistant(&backend, &start.conversation_id, &start.assistant_id);
    assert_eq!(after_replay, cancelled);
    assert_eq!(tool_parts(&after_replay).len(), 1);

    backend.stop();
    broker.shutdown().await;
}
