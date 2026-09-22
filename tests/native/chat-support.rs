use crate::chat::{
    credentials,
    delivery::Delivery,
    process::Process,
    storage::{self, Owner},
};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{ipc::Channel, Manager, State, Window};

#[derive(Default)]
pub struct Probe {
    inner: Arc<Mutex<Option<Running>>>,
    metrics: Arc<Mutex<Value>>,
}
struct Running {
    process: Process,
    delivery: Delivery,
    channel: Channel<Value>,
    root: PathBuf,
    _owner: Owner,
}

fn cpu_seconds(pid: u32) -> Option<f64> {
    if !cfg!(target_os = "macos") || std::env::var_os("LOMI_CHAT_PROBE_OFFLINE").is_some() {
        return None;
    }
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let fields = text
        .trim()
        .split(':')
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(fields.iter().fold(0.0, |total, n| total * 60.0 + n))
}

fn directory() -> Result<PathBuf, String> {
    std::env::var_os("LOMI_CHAT_PROBE_DIRECTORY")
        .map(PathBuf::from)
        .ok_or("Chat probe is not enabled.".into())
}

pub fn page(view: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if directory().is_ok()
        && view.label() == "main"
        && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        let _ = view.eval(include_str!("chat-smoke.js").replace(
            "PROBE_MODULE",
            if cfg!(debug_assertions) {
                "/tests/native/chat-ui.ts"
            } else {
                "/chat-native-probe.js"
            },
        ));
    }
}

#[tauri::command]
pub async fn chat_probe_start(
    window: Window,
    state: State<'_, Probe>,
    channel: Channel<Value>,
) -> Result<Value, String> {
    crate::files::main_window(&window)?;
    let root = directory()?;
    window.show().map_err(|_| "Cannot show the native probe.")?;
    window
        .set_focus()
        .map_err(|_| "Cannot focus the native probe.")?;
    let resources = window
        .app_handle()
        .path()
        .resource_dir()
        .map_err(|_| "Missing resources.")?;
    let inner = state.inner.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let owner = Owner::acquire(root.join("store"))?;
        storage::atomic(&owner.root.join("probe.json"), b"{}")?;
        let key_id = format!("probe-{}", std::process::id());
        credentials::put(&key_id, "fixture-private-key")?;
        let secret = credentials::get(&key_id);
        let cleanup = credentials::remove(&key_id);
        if secret? != "fixture-private-key" { return Err("System key store round trip failed.".into()); }
        cleanup?;
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target = if cfg!(target_os="macos") { if cfg!(target_arch="aarch64") {"aarch64-apple-darwin"} else {"x86_64-apple-darwin"} } else if cfg!(windows) {"x86_64-pc-windows-msvc.exe"} else {"x86_64-unknown-linux-gnu"};
        let (node, bundle) = if cfg!(debug_assertions) {
            (base.join(format!("binaries/lomi-node-{target}")),base.join("resources/ai-runtime/fixture.cjs"))
        } else {
            (std::env::current_exe().map_err(|_| "Missing app executable.")?.parent().ok_or("Missing app directory.")?.join(if cfg!(windows) {"lomi-node.exe"} else {"lomi-node"}), resources.join("ai-runtime/fixture.cjs"))
        };
        let started = Instant::now();
        let (process, events) = Process::start(&node, &bundle)?;
        let ready = events.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_| "AI handshake timed out.")?;
        if ready.r#type != "ready" { return Err("Invalid AI handshake.".into()); }
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let pid = process.id();
        let mut delivery = Delivery::new("assistant-1", 512);
        channel.send(delivery.subscribe()).map_err(|_| "Channel unavailable.")?;
        let db = rusqlite::Connection::open(owner.root.join("probe.sqlite3")).map_err(|_| "DB unavailable.")?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE checkpoints (sequence INTEGER PRIMARY KEY, snapshot TEXT NOT NULL);").map_err(|_| "DB initialization failed.")?;
        for entry in std::fs::read_dir(&owner.root).map_err(|_| "Storage unavailable.")? {
            let entry = entry.map_err(|_| "Storage unavailable.")?;
            if entry.file_type().map_err(|_| "Storage unavailable.")?.is_file() { storage::private(&entry.path(), false)?; }
        }
        process.generate("request-1", &json!({"provider":"openai","apiKey":"fixture-private-key","model":"fixture-fast","assistantId":"assistant-1","messages":[{"id":"user-1","role":"user","parts":[{"type":"text","text":"fixture-only"}]}]}))?;
        *inner.lock().unwrap() = Some(Running { process, delivery, channel, root: root.clone(), _owner: owner });
        let worker_inner = inner.clone();
        std::thread::spawn(move || {
            for event in events {
                if event.r#type == "message-snapshot" {
                    if db.execute("INSERT INTO checkpoints VALUES (?1, ?2)", rusqlite::params![event.sequence as i64, event.payload.to_string()]).is_err() {
                        if let Some(running) = worker_inner.lock().unwrap().as_ref() { running.process.stop(); }
                        break;
                    }
                    continue;
                }
                let mut state = worker_inner.lock().unwrap();
                let Some(running) = state.as_mut() else { break; };
                match event.r#type.as_str() {
                    "chunk" => {
                        match running.delivery.chunk(event.sequence, &event.payload) {
                            Ok(Some(mut value)) => { value["sentAt"]=json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64); let _ = running.channel.send(value); }
                            Ok(None) => (),
                            Err(_) => { running.process.stop(); break; }
                        }
                    }
                    "completed" | "cancelled" | "failed" => {
                        let terminal = json!({"type":"terminal","epoch":running.delivery.epoch,"sequence":event.sequence,"status":event.r#type});
                        running.delivery.terminal = Some(terminal.clone());
                        let _ = running.channel.send(terminal);
                        break;
                    }
                    _ => (),
                }
            }
        });
        Ok(json!({"startupMs":elapsed,"pid":pid,"keychain":true,"node":ready.payload}))
    }).await.map_err(|_| "Probe worker failed.")?
}

#[tauri::command]
pub fn chat_probe_cancel(
    window: Window,
    state: State<'_, Probe>,
    action: String,
    epoch: Option<u64>,
    sequence: Option<u64>,
    channel: Channel<Value>,
) -> Result<Value, String> {
    crate::files::main_window(&window)?;
    let metrics = state.metrics.clone();
    let mut state = state.inner.lock().unwrap();
    let running = state.as_mut().ok_or("Probe not running.")?;
    match action.as_str() {
        "ack" => {
            running
                .delivery
                .ack(epoch.unwrap_or_default(), sequence.unwrap_or_default());
            Ok(Value::Null)
        }
        "resync" => {
            // The first 512-byte window deliberately forces a stalled-consumer
            // resync. Resume with the product's normal window for latency/Stop.
            running.delivery.restore_production_window();
            running.channel = channel;
            let snapshot = running.delivery.subscribe();
            running
                .channel
                .send(snapshot.clone())
                .map_err(|_| "Channel failed.")?;
            Ok(snapshot)
        }
        "stop" => {
            running.process.cancel("request-1")?;
            Ok(Value::Null)
        }
        "minimize" => {
            let pid = running.process.id();
            let node_before = cpu_seconds(pid);
            let host_before = cpu_seconds(std::process::id());
            let start = Instant::now();
            window.minimize().map_err(|_| "Cannot minimize.")?;
            let window = window.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                let node_after = cpu_seconds(pid);
                let host_after = cpu_seconds(std::process::id());
                let seconds = start.elapsed().as_secs_f64();
                *metrics.lock().unwrap() = json!({"sampleMs":seconds*1000.0,"nodeOneCorePercent":node_before.zip(node_after).map(|(a,b)| (b-a)*100.0/seconds),"hostOneCorePercent":host_before.zip(host_after).map(|(a,b)| (b-a)*100.0/seconds)});
                let _ = window.unminimize();
                let _ = window.set_focus();
            });
            Ok(Value::Null)
        }
        _ => Err("Unknown probe action.".into()),
    }
}

#[tauri::command]
pub fn chat_probe_result(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Probe>,
    mut result: Value,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let root = directory()?;
    result["minimizedCpu"] = state.metrics.lock().unwrap().clone();
    if let Some(running) = state.inner.lock().unwrap().take() {
        let _ = &running.root;
        #[cfg(target_os = "macos")]
        for (name, pid) in [
            ("hostRssKiB", std::process::id()),
            ("nodeRssKiB", running.process.id()),
        ] {
            if let Ok(output) = std::process::Command::new("/bin/ps")
                .args(["-o", "rss=", "-p", &pid.to_string()])
                .output()
            {
                if let Ok(value) = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<u64>()
                {
                    result[name] = json!(value);
                }
            }
        }
        running.process.stop();
    }
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .map_err(|_| "Cannot save probe result.")?;
    app.exit(if result["passed"] == true { 0 } else { 1 });
    Ok(())
}

#[tauri::command]
pub async fn chat_probe_backend(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, crate::chat::commands::Chats>,
    action: Option<String>,
) -> Result<Value, String> {
    crate::files::main_window(&window)?;
    directory()?;
    if action.as_deref() == Some("mode") {
        return Ok(json!({"live":std::env::var_os("LOMI_CHAT_LIVE_KEYS_FILE").is_some()}));
    }
    if action.as_deref() == Some("browser-url") {
        return Ok(
            json!({"url":std::env::var("LOMI_CHAT_BROWSER_URL").ok(),"offline":std::env::var_os("LOMI_CHAT_PROBE_OFFLINE").is_some()}),
        );
    }
    if action.as_deref() == Some("browser-result") {
        return Ok(std::fs::read(directory()?.join("browser-result.json"))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or(Value::Null));
    }
    let backend = state.backend(&app)?;
    if action.as_deref() == Some("live-setup") {
        window.show().map_err(|_| "Cannot show native live test.")?;
        window
            .set_focus()
            .map_err(|_| "Cannot focus native live test.")?;
        return tauri::async_runtime::spawn_blocking(move || {
            let path = std::env::var_os("LOMI_CHAT_LIVE_KEYS_FILE")
                .ok_or("Provide an explicit live-test key file.")?;
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|_| "Cannot open the live-test key file.")?
                .take(16 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cannot read the live-test key file.")?;
            if bytes.len() > 16 * 1024 {
                return Err("The live-test key file exceeds 16 KiB.".into());
            }
            let keys: Value = serde_json::from_slice(&bytes)
                .map_err(|_| "The live-test key file must contain JSON.")?;
            let mut services = backend.services.lock().unwrap();
            let settings = services.settings.as_mut().map_err(|e| e.clone())?;
            let mut providers = Vec::new();
            for (provider, key_fields, model_fields, fallback) in [
                (
                    "openai",
                    ["OPENAI_API_KEY", "openaiApiKey"],
                    ["OPENAI_MODEL", "openaiModel"],
                    "gpt-4.1-mini",
                ),
                (
                    "anthropic",
                    ["ANTHROPIC_API_KEY", "anthropicApiKey"],
                    ["ANTHROPIC_MODEL", "anthropicModel"],
                    "claude-haiku-4-5",
                ),
                (
                    "google",
                    ["GOOGLE_GENERATIVE_AI_API_KEY", "googleApiKey"],
                    ["GOOGLE_MODEL", "googleModel"],
                    "gemini-2.5-flash",
                ),
            ] {
                let Some(key) = key_fields
                    .iter()
                    .find_map(|field| keys[*field].as_str())
                    .filter(|key| !key.trim().is_empty())
                else {
                    continue;
                };
                let model = model_fields
                    .iter()
                    .find_map(|field| keys[*field].as_str())
                    .unwrap_or(fallback);
                let id = format!("live-{provider}");
                let mut data = settings.data.clone();
                data.connections.retain(|connection| connection.id != id);
                data.connections.push(crate::chat::preferences::Connection {
                    id: id.clone(),
                    name: format!("Live test {provider}"),
                    provider: provider.into(),
                    enabled: true,
                    credential_revision: 0,
                    secret_mode: "session".into(),
                    secret_id: None,
                    models: vec![model.into()],
                    tested_model: None,
                    test_status: None,
                });
                settings.save(data.clone(), data.revision, Some((&id, key)))?;
                providers.push(json!({"id":id,"provider":provider,"model":model}));
            }
            if providers.is_empty() {
                return Err("No recognized live-test keys were provided.".into());
            }
            drop(services);
            for provider in &mut providers {
                let id = provider["id"].as_str().unwrap().to_owned();
                let model = provider["model"].as_str().unwrap().to_owned();
                let catalog = backend.auxiliary(&id, &model, "list-models")?;
                provider["catalog"] = json!({"status":catalog["status"],"code":catalog["result"]["code"],"selectedModelPresent":catalog["models"].as_array().is_some_and(|models| models.iter().any(|value| value.as_str()==Some(&model)))});
                let tested = backend.auxiliary(&id, &model, "test-connection")?;
                provider["connectionTest"] = json!({"status":tested["status"],"code":tested["result"]["code"]});
            }
            Ok(json!({"providers":providers}))
        })
        .await
        .map_err(|_| "Live-test setup failed.")?;
    }
    if action.as_deref() == Some("metrics") {
        if std::env::var_os("LOMI_CHAT_PROBE_OFFLINE").is_some() {
            return Ok(
                json!({"unavailable":"The offline sandbox does not allow process metrics."}),
            );
        }
        let mut value = json!({});
        #[cfg(target_os = "macos")]
        for (name, pid) in [
            ("hostRssKiB", Some(std::process::id())),
            ("nodeRssKiB", backend.probe_process_id()),
        ] {
            if let Some(pid) = pid {
                let output = std::process::Command::new("/bin/ps")
                    .args(["-o", "rss=", "-p", &pid.to_string()])
                    .output()
                    .map_err(|_| "Cannot measure native memory.")?;
                value[name] = json!(String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "Invalid memory sample.")?);
            }
        }
        return Ok(value);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut services = backend.services.lock().unwrap();
        let settings = services.settings.as_mut().map_err(|e| e.clone())?;
        let mut data = settings.data.clone();
        data.connections = vec![crate::chat::preferences::Connection {
            id: "native-fixture".into(),
            name: "Native fixture".into(),
            provider: "openai".into(),
            enabled: true,
            credential_revision: 0,
            secret_mode: "session".into(),
            secret_id: None,
            models: vec![],
            tested_model: None,
            test_status: None,
        }];
        data.defaults = crate::chat::store::Config {
            connection_id: Some("native-fixture".into()),
            model: "fixture-fast".into(),
            configured: true,
            ..Default::default()
        };
        settings.save(
            data.clone(),
            data.revision,
            Some(("native-fixture", "fixture-private-key")),
        )?;
        Ok(json!({"ready":true}))
    })
    .await
    .map_err(|_| "Native backend setup failed.")?
}
