//! Explicit Darwin fixture: real kernel cohort membership, TLS enrollment and
//! the production workspace resolver. Uses only a private synthetic account.
use super::*;
use crate::agent_runtime::{
    host_boundary::{self, NativeIo, NativeSpec, Policy, Purpose},
    host_child::{Context, HostChild},
    service::{Active, Inner},
    store::{workspace, Store},
    types::{AccountInstance, CredentialBinding, Task},
};
use lomi_control_core::{broker::Broker, client::Client, discovery};
use lomi_control_protocol::control::{Projection, Reply, Workspace};
use serde_json::json;
use std::{
    io::{BufRead, Read, Write},
    os::unix::fs::PermissionsExt,
    sync::{atomic::AtomicBool, Mutex, Weak},
    time::{Duration, Instant},
};
const ENTRY: &str = "agent_runtime::mcp_policy::cohort_tests::compiled_cohort_peer_entry";
const FLAG: &str = "LOMI_OWNED_MCP_FIXTURE";
fn emit(value: serde_json::Value) {
    println!("LOMI_PEER {}", value);
    std::io::stdout().flush().unwrap();
}

#[test]
fn compiled_cohort_peer_entry() {
    let Ok(discovery_path) = std::env::var(FLAG) else {
        return;
    };
    let endpoint = discovery::read(
        std::path::Path::new(&discovery_path),
        &std::env::var("LOMI_OWNED_MCP_KEY").unwrap(),
    )
    .unwrap();
    emit(json!({"pid":std::process::id()}));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut client = Client::connect(&endpoint, "owned task fixture", |id| {
            emit(json!({"pending":id}))
        })
        .await
        .unwrap();
        emit(json!({"connected":true}));
        for line in std::io::stdin().lock().lines() {
            let line = line.unwrap();
            if line == "reconnect" {
                drop(client);
                client = Client::connect(&endpoint, "owned task fixture", |_| {})
                    .await
                    .unwrap();
                emit(json!({"connected":true}));
                continue;
            }
            if line == "exit" {
                break;
            }
            let request: Request = serde_json::from_str(&line).unwrap();
            emit(serde_json::to_value(client.call(request).await.unwrap()).unwrap());
        }
    });
}

struct Peer {
    child: HostChild,
    bytes: Vec<u8>,
}
impl Peer {
    fn next(&mut self) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            if let Some(end) = self.bytes.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.bytes.drain(..=end).collect();
                if let Some(at) = line.windows(10).position(|s| s == b"LOMI_PEER ") {
                    return serde_json::from_slice(&line[at + 10..]).unwrap();
                }
                continue;
            }
            let mut data = [0; 8192];
            match self.child.stdout.as_mut().unwrap().read(&mut data) {
                Ok(n) if n > 0 => self.bytes.extend_from_slice(&data[..n]),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("fixture stdout: {e}"),
            }
            if Instant::now() > deadline {
                let mut errors = String::new();
                let _ = self
                    .child
                    .stderr
                    .as_mut()
                    .unwrap()
                    .read_to_string(&mut errors);
                panic!(
                    "fixture protocol timeout; stderr={errors}; stdout={:?}",
                    String::from_utf8_lossy(&self.bytes)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn send(&mut self, value: serde_json::Value) -> Reply {
        writeln!(self.child.stdin.as_mut().unwrap(), "{value}").unwrap();
        serde_json::from_value(self.next()).unwrap()
    }
    fn command(&mut self, command: &str) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{command}").unwrap();
    }
}
fn request(tool: &str, arguments: serde_json::Value) -> serde_json::Value {
    json!({"tool":tool,"arguments":arguments})
}
fn read(alias: &str) -> serde_json::Value {
    request(
        "lomi_files_read",
        json!({"workspaceId":alias,"relativePath":"owned.txt"}),
    )
}
fn denied(reply: Reply, code: ErrorCode) {
    assert!(matches!(reply,Reply::Error{code:c,..} if c==code));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Darwin kernel/cohort and process-global Broker fixture; run explicitly serially"]
async fn owned_cohort_broker_admission_and_revocation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.keep().canonicalize().unwrap();
    eprintln!("owned fixture root: {}", root.display());
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let project = root.join("project");
    let other = root.join("other");
    let scratch = root.join("scratch");
    for p in [&project, &other, &scratch] {
        std::fs::create_dir(p).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(project.join("owned.txt"), "owned fixture observation\n").unwrap();
    let git = std::path::Path::new("/Library/Developer/CommandLineTools/usr/bin/git");
    assert!(std::process::Command::new(git)
        .args(["-c", "core.hooksPath=/dev/null", "init", "--quiet"])
        .current_dir(&project)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .unwrap()
        .success());
    let mut store = Store::open(root.join("owner")).unwrap();
    let account:AccountInstance=serde_json::from_value(json!({"accountId":"fixture","cli":"codex","label":"Fixture","enabled":true,"revision":1,"authRevision":1,"authState":"unverified","availabilityReason":null,"acceptedVersion":null,"recovery":null})).unwrap();
    let account_root = store.root.join("accounts/fixture");
    std::fs::create_dir_all(&account_root).unwrap();
    std::fs::set_permissions(&account_root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let binding = CredentialBinding {
        account_id: "fixture".into(),
        auth_revision: 1,
        physical_root: account_root.to_str().unwrap().into(),
        namespace: "fixture".into(),
        credential_reference: "fixture".into(),
    };
    store
        .save_account(&json!({}), "create-fixture", &account, &binding, &json!({}))
        .unwrap();
    let (cwd, identity) = workspace(&project).unwrap();
    let task:Task=serde_json::from_value(json!({"taskId":"task","cwd":cwd,"title":"Fixture","cli":"codex","availabilityReason":null,"model":"fixture","reasoningEffort":null,"revision":1,"historyRevision":0,"generation":1,"state":"running","nextAccountId":"fixture","activeAccountId":"fixture","activeAttemptId":"attempt","statusMessage":"","attempts":[{"attemptId":"attempt","operationId":"attempt-op","accountId":"fixture","authRevision":1,"generation":1,"input":"fixture","continuationMethod":"new","state":"dispatched","output":"","nativeRef":null,"version":null,"effectsState":"unsettled"}],"history":[],"grants":[],"switches":[]})).unwrap();
    store
        .create_task(&json!({}), "create-task", &task, "fixture", identity)
        .unwrap();
    store
        .owned_process_intent(&task, &task.attempts[0])
        .unwrap();
    let context: Context = store.host_context("attempt-op").unwrap();
    let mut inner = Inner {
        store: Some(store),
        ..Default::default()
    };
    let stop = Arc::new(AtomicBool::new(false));
    inner.active.insert(
        "task".into(),
        Active {
            attempt: "attempt".into(),
            generation: 1,
            account: "fixture".into(),
            stop: stop.clone(),
            abort: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicBool::new(false)),
            control: Arc::new(Mutex::new(())),
            worker_finished: Arc::new(AtomicBool::new(false)),
        },
    );
    let runtime = AgentRuntime {
        inner: Arc::new(Mutex::new(inner)),
        ..Default::default()
    };
    let slot = Arc::new(Mutex::new(Weak::<Broker>::new()));
    let resolver_slot = slot.clone();
    let resolver: WorkspaceResolver = Arc::new(move |alias| {
        resolver_slot
            .lock()
            .unwrap()
            .upgrade()
            .ok_or(ErrorCode::AppUnavailable)?
            .request_workspace_root(alias)
    });
    let captured = runtime.clone();
    let broker = Broker::start_with_admission(
        &root.join("broker"),
        None,
        Some(Arc::new(move |pid| {
            capture_with_workspace_resolver(&captured, pid, Some(resolver.clone())).map(Some)
        })),
    )
    .unwrap();
    *slot.lock().unwrap() = Arc::downgrade(&broker);
    assert_eq!(
        broker.endpoint.endpoint,
        broker.endpoint.endpoint.canonicalize().unwrap()
    );
    broker
        .set_files_read_dispatch(Arc::new(crate::files::agent::decode))
        .unwrap();
    broker
        .publish(Projection {
            ui_epoch: broker.register_ui().unwrap(),
            revision: "1".into(),
            workspaces: vec![("own", &project), ("other", &other)]
                .into_iter()
                .map(|(id, path)| Workspace {
                    id: id.into(),
                    project_id: format!("project-{id}"),
                    name: id.into(),
                    project_name: id.into(),
                    project_path: path.to_str().unwrap().into(),
                    active_panel_id: None,
                })
                .collect(),
            ..Default::default()
        })
        .unwrap();
    let public_key = discovery::ensure_public_key(&root.join("broker")).unwrap();
    discovery::publish(&root.join("broker"), &broker.endpoint).unwrap();
    let document = root.join("broker/discovery.json");
    let binary = host_boundary::staged_worker(&context.storage_root).unwrap();
    for fence in ["auth", "generation", "stop"] {
        let spec = NativeSpec {
            program: binary.clone(),
            arguments: vec![
                "--exact".into(),
                ENTRY.into(),
                "--nocapture".into(),
                "--test-threads=1".into(),
            ],
            environment: vec![
                (FLAG.into(), document.to_str().unwrap().into()),
                ("LOMI_OWNED_MCP_KEY".into(), public_key.clone()),
                ("HOME".into(), account_root.to_str().unwrap().into()),
            ],
            cwd: project.clone(),
            io: NativeIo::Pipes,
            policy: Policy {
                project_root: project.clone(),
                account_root: account_root.clone(),
                temp_root: scratch.clone(),
                runtime_reads: vec![binary.clone()],
                protected_reads: vec![document.clone()],
                blocked_reads: vec![],
                runtime_read_roots: vec![],
                codex_preferences: false,
                loopback_tcp_ports: vec![],
                loopback_listener: false,
                unix_sockets: vec![broker.endpoint.endpoint.clone()],
                allow_native_tools: false,
            },
        };
        let mut peer = Peer {
            child: context.spawn(Purpose::Attempt, spec).unwrap(),
            bytes: vec![],
        };
        let pid = peer.next()["pid"].as_u64().unwrap() as u32;
        let (scope, _) = host_boundary::registered_scope(pid)
            .unwrap()
            .expect("actual native peer belongs to owned cohort");
        assert_eq!(scope.parent_operation_id.as_deref(), Some("attempt-op"));
        if fence == "auth" {
            let pending = peer.next()["pending"].as_str().unwrap().to_owned();
            let overview = broker.overview().unwrap();
            let cert = overview.pending.iter().find(|p| p.id == pending).unwrap();
            assert_eq!(cert.client_label, "owned task fixture");
            assert_eq!(cert.certificate_sha256.len(), 64);
            broker
                .approve_scopes(
                    &pending,
                    &["own".into(), "other".into()],
                    &[
                        "workspace.read".into(),
                        "files.read".into(),
                        "git.read".into(),
                        "files.create".into(),
                        "files.mutate".into(),
                    ],
                )
                .unwrap();
            assert_eq!(peer.next()["connected"], true);
            broker.set_yolo_mode(true).unwrap();
            peer.command("reconnect");
            assert_eq!(peer.next()["connected"], true);
        } else {
            assert!(peer.next()["pending"].is_string());
            assert_eq!(peer.next()["connected"], true);
        }
        assert!(matches!(peer.send(read("own")), Reply::Ok { .. }));
        assert!(matches!(
            peer.send(request(
                "lomi_git_status",
                json!({"workspaceId":"own","repositoryRelative":"","limit":10})
            )),
            Reply::Ok { .. }
        ));
        denied(peer.send(read("other")), ErrorCode::ScopeDenied);
        denied(
            peer.send(request("lomi_workspace_list", json!({"limit":1}))),
            ErrorCode::ScopeDenied,
        );
        denied(peer.send(request("lomi_files_mutate",json!({"workspaceId":"own","operation":{"type":"create","relativePath":"blocked.txt","kind":"file","expectedParentRevision":"fixture"},"expectedRevision":"fixture","retryEpoch":"fixture","requestKey":"fixture"}))),ErrorCode::ScopeDenied);
        // A host peer claiming this label is still outside the registered cohort.
        let unmanaged = Client::connect(&broker.endpoint, "owned task fixture", |_| {})
            .await
            .unwrap();
        denied(
            unmanaged
                .call(serde_json::from_value(read("own")).unwrap())
                .await
                .unwrap(),
            ErrorCode::ScopeDenied,
        );
        drop(unmanaged);
        {
            let mut owner = runtime.inner.lock().unwrap();
            match fence {
                "auth" => {
                    let store = owner.store.as_mut().unwrap();
                    let mut a = store.account("fixture").unwrap();
                    a.auth_revision = 2;
                    let mut b = store.binding("fixture").unwrap();
                    b.auth_revision = 2;
                    store
                        .save_account(&json!({}), "revoke-auth", &a, &b, &json!({}))
                        .unwrap();
                }
                "generation" => {
                    let store = owner.store.as_mut().unwrap();
                    let mut t = store.task("task").unwrap();
                    t.generation = 2;
                    store.update_task(&t, None, None).unwrap();
                }
                "stop" => stop.store(true, Ordering::SeqCst),
                _ => unreachable!(),
            }
        }
        denied(peer.send(read("own")), ErrorCode::ControlRevoked);
        peer.child.stop_and_wait().unwrap();
        assert!(host_boundary::parent_completed(&context.storage_root, "attempt-op").unwrap());
        assert!(matches!(
            host_boundary::registered_scope(pid),
            Ok(None) | Err(_)
        ));
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        // Restore only synthetic fences before a distinct cohort proves the next.
        let mut owner = runtime.inner.lock().unwrap();
        let store = owner.store.as_mut().unwrap();
        if fence == "auth" {
            store
                .save_account(&json!({}), "restore-auth", &account, &binding, &json!({}))
                .unwrap();
        }
        if fence == "generation" {
            store.update_task(&task, None, None).unwrap();
        }
        stop.store(false, Ordering::SeqCst);
    }
    // A retired cohort is not a complete native-turn journal. Publication of
    // local retirement must preserve the generic unknown-effects marker.
    {
        let mut owner = runtime.inner.lock().unwrap();
        let store = owner.store.as_mut().unwrap();
        store.complete_owned_helper("attempt-op").unwrap();
        assert!(store.reconcile_processes().unwrap().is_empty());
        assert!(!store.processes_settled("task").unwrap());
        assert!(store
            .unresolved_accounts()
            .unwrap()
            .iter()
            .any(|id| id == "fixture"));
        assert!(store
            .recover_helpers(&crate::agent_runtime::types::AccountRecover {
                operation_id: "same-boot-ack".into(),
                account_id: "fixture".into(),
                expected_revision: 1,
                acknowledge_effects: true
            })
            .is_err());
    }
    assert!(!project.join("blocked.txt").exists());
    broker.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}
