#![cfg(unix)]

use lomi_control_core::{broker::Broker, discovery};
use lomi_control_protocol::control::{Projection, Workspace};
use serde_json::{json, Value};
use std::{
    env,
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

struct McpChild {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: mpsc::Receiver<Result<Value, String>>,
    next_id: u64,
}

impl McpChild {
    fn spawn(executable: &Path, discovery_file: &Path, pinned_key: &str) -> Self {
        let mut command = Command::new(executable);
        if executable.file_stem().and_then(|name| name.to_str()) == Some("lomi") {
            command.arg("--mcp");
        }
        let mut child = command
            .args([
                "--discovery-file",
                discovery_file.to_str().expect("temporary path is UTF-8"),
                "--discovery-key",
                pinned_key,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|error| panic!("could not start {}: {error}", executable.display()));
        let input = BufWriter::new(child.stdin.take().unwrap());
        let stdout = child.stdout.take().unwrap();
        let (send, output) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output,
            next_id: 0,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        writeln!(self.input, "{message}").unwrap();
        self.input.flush().unwrap();
        loop {
            let response = self
                .output
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|error| {
                    let status = self.child.try_wait().ok().flatten();
                    panic!("MCP {method} response timed out ({error}); child status: {status:?}")
                })
                .unwrap_or_else(|error| panic!("invalid MCP response: {error}"));
            if response.get("id") == Some(&json!(id)) {
                assert_eq!(response["jsonrpc"], "2.0");
                assert!(response.get("error").is_none(), "{response}");
                return response;
            }
        }
    }

    fn notify(&mut self, method: &str) {
        writeln!(self.input, "{}", json!({"jsonrpc":"2.0", "method":method})).unwrap();
        self.input.flush().unwrap();
    }

    fn initialize(&mut self) {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "discovery-helper-test", "version": "1"},
            }),
        );
        assert_eq!(response["result"]["serverInfo"]["name"], "lomi-mcp");
        self.notify("notifications/initialized");
    }

    fn tool_call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))["result"].clone()
    }

    fn status(&mut self) -> Value {
        let response = self.tool_call("lomi_status", json!({}));
        assert_ne!(response["isError"], true, "{response}");
        response["structuredContent"]["data"].clone()
    }

    fn workspace_list(&mut self) -> Value {
        self.tool_call("lomi_workspace_list", json!({}))
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn launch(executable: &Path, discovery_file: &Path, pinned_key: &str) -> McpChild {
    let mut child = McpChild::spawn(executable, discovery_file, pinned_key);
    child.initialize();
    child
}

async fn wait_for_connection(child: &mut McpChild, phase: &str) -> Value {
    for _ in 0..100 {
        let status = tokio::task::block_in_place(|| child.status());
        if status["connection"] == phase {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("helper did not reach {phase:?} phase");
}

async fn approve_pairing(broker: &Broker, status: &Value) {
    let request_id = status["pairingRequestId"]
        .as_str()
        .expect("pairing phase includes a request ID");
    let pending = broker.overview().unwrap().pending;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, request_id);
    broker
        .approve_scopes(request_id, &["allowed".into()], &["workspace.read".into()])
        .unwrap();
}

fn publish_ui(broker: &Broker, root: &Path) {
    let project_path = root.join("approved-project");
    std::fs::create_dir_all(&project_path).unwrap();
    broker
        .publish(Projection {
            ui_epoch: broker.register_ui().unwrap(),
            revision: "1".into(),
            workspaces: vec![Workspace {
                id: "allowed".into(),
                project_id: "approved-project".into(),
                name: "Allowed".into(),
                project_name: "Approved project".into(),
                active_panel_id: None,
                project_path: project_path
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into(),
            }],
            ..Projection::default()
        })
        .unwrap();
}

fn assert_workspaces_only_allowed(response: &Value) {
    assert_ne!(response["isError"], true, "{response}");
    assert_eq!(response["structuredContent"]["status"], "ok");
    assert_eq!(response["structuredContent"]["data"]["kind"], "workspaces");
    assert_eq!(
        response["structuredContent"]["data"]["items"][0]["id"],
        "allowed"
    );
    assert_eq!(
        response["structuredContent"]["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

fn executable_from_environment() -> Option<PathBuf> {
    env::var_os("LOMI_MCP_TEST_EXECUTABLE").map(PathBuf::from)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_helper_uses_signed_discovery_requires_pairing_after_restart_and_rejects_tampering(
) {
    let Some(executable) = executable_from_environment() else {
        eprintln!("SKIP: set LOMI_MCP_TEST_EXECUTABLE to the lomi or lomi-mcp executable");
        return;
    };
    assert!(
        executable.is_file(),
        "{} does not exist",
        executable.display()
    );

    let temp = tempfile::tempdir().unwrap();
    let state_root = temp.path().join("agent-control-state");
    let registration_root = temp.path().join("agent-control-registration");
    let discovery_file = registration_root.join(discovery::FILE);
    let pinned_key = discovery::ensure_public_key(&registration_root).unwrap();

    let broker = Broker::start(&state_root).unwrap();
    publish_ui(&broker, temp.path());
    let first_endpoint = broker.overview().unwrap().endpoint;
    discovery::publish(&registration_root, &first_endpoint).unwrap();

    let mut helper = launch(&executable, &discovery_file, &pinned_key);
    let pairing = wait_for_connection(&mut helper, "pairing_required").await;
    let pending_workspace_call = tokio::task::block_in_place(|| helper.workspace_list());
    assert_eq!(pending_workspace_call["isError"], true);
    assert_eq!(
        pending_workspace_call["structuredContent"]["code"],
        "PAIRING_REQUIRED"
    );
    approve_pairing(&broker, &pairing).await;
    let connected = wait_for_connection(&mut helper, "connected").await;
    assert_eq!(connected["instanceId"], first_endpoint.instance_id);
    let authorized_workspace_call = tokio::task::block_in_place(|| helper.workspace_list());
    assert_workspaces_only_allowed(&authorized_workspace_call);
    drop(helper);

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
    assert!(restarted.overview().unwrap().pending.is_empty());

    let mut restarted_helper = launch(&executable, &discovery_file, &pinned_key);
    let restarted_pairing = wait_for_connection(&mut restarted_helper, "pairing_required").await;
    let pending_workspace_call = tokio::task::block_in_place(|| restarted_helper.workspace_list());
    assert_eq!(pending_workspace_call["isError"], true);
    assert_eq!(
        pending_workspace_call["structuredContent"]["code"],
        "PAIRING_REQUIRED"
    );
    approve_pairing(&restarted, &restarted_pairing).await;
    let connected = wait_for_connection(&mut restarted_helper, "connected").await;
    assert_eq!(connected["instanceId"], second_endpoint.instance_id);
    let authorized_workspace_call =
        tokio::task::block_in_place(|| restarted_helper.workspace_list());
    assert_workspaces_only_allowed(&authorized_workspace_call);
    drop(restarted_helper);

    let mut signed: Value =
        serde_json::from_slice(&std::fs::read(&discovery_file).unwrap()).unwrap();
    let mut payload: Value = serde_json::from_str(signed["payload"].as_str().unwrap()).unwrap();
    payload["instanceId"] = json!("tampered-instance");
    signed["payload"] = json!(payload.to_string());
    std::fs::write(&discovery_file, serde_json::to_vec(&signed).unwrap()).unwrap();

    let mut tampered_helper = launch(&executable, &discovery_file, &pinned_key);
    let status = wait_for_connection(&mut tampered_helper, "app_unavailable").await;
    assert_eq!(status["instanceId"], Value::Null);
    assert!(restarted.overview().unwrap().pending.is_empty());
    let unavailable_workspace_call =
        tokio::task::block_in_place(|| tampered_helper.workspace_list());
    assert_eq!(unavailable_workspace_call["isError"], true);
    assert_eq!(
        unavailable_workspace_call["structuredContent"]["code"],
        "APP_UNAVAILABLE"
    );
    assert!(restarted.overview().unwrap().pending.is_empty());

    drop(tampered_helper);
    restarted.shutdown().await;
}
