use super::{
    attachments,
    delivery::Delivery,
    preferences::{Settings, SystemSecrets},
    process::{Event, Process},
    storage::Owner,
    store::{Start, Store},
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, Manager};

const MAX_MCP_CALLS: usize = 128;
const MAX_MCP_INPUT_BYTES: usize = 64 * 1024;
const MAX_MCP_RESULT_BYTES: usize = 8 * 1024 * 1024;
const MAX_MCP_RESULT_TOTAL_BYTES: usize = 16 * 1024 * 1024;

pub struct Services {
    pub store: Result<Store, String>,
    pub settings: Result<Settings, String>,
}
pub struct Request {
    pub conversation: Option<String>,
    pub connection: String,
    pub delivery: Mutex<Delivery>,
    pub channel: Mutex<Option<Channel<Value>>>,
    pub cancelled: AtomicBool,
    pub finalizing: AtomicBool,
    pub checkpoint_sequence: AtomicI64,
    pub saved_sequence: AtomicI64,
    saved_bytes: AtomicI64,
    pub result: Mutex<Value>,
    pub done: (Mutex<Option<Result<(), String>>>, Condvar),
    pub mcp_session: Mutex<Option<Arc<lomi_mcp::chat::ChatSession>>>,
    mcp_calls: Mutex<McpCalls>,
    mcp_result_bytes: AtomicUsize,
    mcp_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}
#[derive(Default)]
struct McpCalls {
    ids: HashSet<String>,
    active: Option<String>,
}
struct McpSessionEntry {
    session: Arc<lomi_mcp::chat::ChatSession>,
    last_used: Instant,
}
pub struct Backend {
    notifications_app: Option<tauri::AppHandle>,
    pub services: Mutex<Services>,
    pub owner: Owner,
    pub requests: Mutex<HashMap<String, Arc<Request>>>,
    pub changing: Mutex<HashSet<String>>,
    pub(super) closing: Mutex<HashMap<String, Box<dyn Fn() -> bool + Send + Sync>>>,
    pub cancelled: Mutex<HashSet<String>>,
    mcp_sessions: Mutex<HashMap<String, McpSessionEntry>>,
    process: Mutex<Option<Arc<Process>>>,
    node: PathBuf,
    bundle: PathBuf,
    #[cfg(unix)]
    endpoint_provider: Arc<dyn Fn() -> Option<lomi_control_core::broker::Endpoint> + Send + Sync>,
}
impl Backend {
    pub fn open(app: &tauri::AppHandle) -> Result<Arc<Self>, String> {
        let app_data = app
            .path()
            .app_data_dir()
            .map_err(|_| "Cannot locate chat data.")?;
        let root = app_data.join("chat-ai");
        let owner = Owner::acquire(root)?;
        let mut store = Store::open(&owner.root);
        if let Ok(opened) = &mut store {
            if let Err(error) = attachments::reconcile(opened, &owner.root) {
                store = Err(error);
            }
        }
        let config = app
            .path()
            .app_config_dir()
            .map_err(|_| "Cannot locate chat preferences.")?;
        std::fs::create_dir_all(&config).map_err(|_| "Cannot create chat preferences.")?;
        let settings = Settings::open(
            config.join("chat-ai-preferences.json"),
            &owner.root,
            SystemSecrets,
        );
        #[cfg(unix)]
        let endpoint_provider = {
            let app = app.clone();
            Arc::new(move || {
                let control = app.state::<crate::agent_control::Control>();
                control
                    .required()
                    .ok()
                    .map(|broker| broker.endpoint.clone())
            })
        };
        let (node, bundle) = if cfg!(debug_assertions) {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            (
                root.join(format!(
                    "binaries/lomi-node-{}{}",
                    env!("LOMI_AI_TARGET"),
                    if cfg!(windows) { ".exe" } else { "" }
                )),
                root.join("resources/ai-runtime/index.cjs"),
            )
        } else {
            (
                tauri::utils::platform::current_exe()
                    .map_err(|_| "Cannot locate the installed application.")?
                    .parent()
                    .ok_or("Cannot locate AI runtime.")?
                    .join(if cfg!(windows) {
                        "lomi-node.exe"
                    } else {
                        "lomi-node"
                    }),
                app.path()
                    .resource_dir()
                    .map_err(|_| "Cannot locate AI resources.")?
                    .join("ai-runtime/index.cjs"),
            )
        };

        Ok(Arc::new(Self {
            notifications_app: Some(app.clone()),
            services: Mutex::new(Services { store, settings }),
            owner,
            requests: Mutex::new(HashMap::new()),
            changing: Mutex::new(HashSet::new()),
            closing: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashSet::new()),
            mcp_sessions: Mutex::new(HashMap::new()),
            process: Mutex::new(None),
            node,
            bundle,
            #[cfg(unix)]
            endpoint_provider,
        }))
    }

    fn mcp_session(
        &self,
        conversation: &super::store::Conversation,
    ) -> Arc<lomi_mcp::chat::ChatSession> {
        const MAX_RETAINED_SESSIONS: usize = 64;
        let mut label = format!(
            "Lomi Chat AI {}",
            conversation.id.chars().take(32).collect::<String>()
        );
        if label.len() > 80 {
            label.truncate(80);
        }
        #[cfg(unix)]
        let endpoint = (self.endpoint_provider)();
        let mut sessions = self.mcp_sessions.lock().unwrap();
        if let Some(entry) = sessions.get_mut(&conversation.id) {
            #[cfg(unix)]
            let same_endpoint = entry.session.matches_endpoint(endpoint.as_ref());
            #[cfg(not(unix))]
            let same_endpoint = true;
            if same_endpoint && !entry.session.is_revoked() {
                entry.last_used = Instant::now();
                return entry.session.clone();
            }
            if let Some(entry) = sessions.remove(&conversation.id) {
                entry.session.close();
            }
        }
        if sessions.len() >= MAX_RETAINED_SESSIONS {
            let active = self
                .requests
                .lock()
                .unwrap()
                .values()
                .filter(|request| request.done.0.lock().unwrap().is_none())
                .filter_map(|request| request.conversation.clone())
                .collect::<HashSet<_>>();
            if let Some(oldest) = sessions
                .iter()
                .filter(|(id, _)| !active.contains(*id))
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(id, _)| id.clone())
            {
                if let Some(entry) = sessions.remove(&oldest) {
                    entry.session.close();
                }
            }
        }
        #[cfg(unix)]
        let session = Arc::new(lomi_mcp::chat::ChatSession::new(endpoint, label));
        #[cfg(not(unix))]
        let session = Arc::new(lomi_mcp::chat::ChatSession::new(label));
        sessions.insert(
            conversation.id.clone(),
            McpSessionEntry {
                session: session.clone(),
                last_used: Instant::now(),
            },
        );
        session
    }

    pub(super) fn attach_mcp_context(
        &self,
        context: &mut super::generation::Context,
        conversation: &super::store::Conversation,
    ) -> Arc<lomi_mcp::chat::ChatSession> {
        let session = self.mcp_session(conversation);
        context.payload["mcp"] = json!({
            "tools": session.tools(),
            "instructions": session.instructions_for_origin(
                &conversation.origin.project_id,
                &conversation.origin.workspace_id,
            ),
        });
        session
    }

    fn process(self: &Arc<Self>) -> Result<Arc<Process>, String> {
        runtime_supported()?;
        let mut slot = self
            .process
            .lock()
            .map_err(|_| "AI process state unavailable.")?;
        if let Some(process) = slot.as_ref() {
            return Ok(process.clone());
        }
        let (process, events) = Process::start(&self.node, &self.bundle)?;
        let ready = events
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "AI process startup timed out.")?;
        if ready.r#type != "ready" {
            return Err("Invalid AI runtime handshake.".into());
        }
        let process = Arc::new(process);
        *slot = Some(process.clone());
        let owner = self.clone();
        let pid = process.id();
        std::thread::spawn(move || {
            let mut last_checkpoint = Instant::now();
            let mut idle_since = Instant::now();
            loop {
                match events.recv_timeout(Duration::from_millis(100)) {
                    Ok(event) => owner.event(event),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
                }
                let active = owner
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(_, r)| r.done.0.lock().unwrap().is_none())
                    .map(|(id, r)| (id.clone(), r.clone()))
                    .collect::<Vec<_>>();
                if !active.is_empty() {
                    idle_since = Instant::now();
                }
                if last_checkpoint.elapsed() >= Duration::from_millis(500) {
                    for (id, request) in &active {
                        if request.conversation.is_some()
                            && owner.checkpoint(id, request, "active", &json!({})).is_err()
                        {
                            let _ = owner.cancel(id);
                        }
                    }
                    last_checkpoint = Instant::now();
                }
                if idle_since.elapsed() >= Duration::from_secs(60) {
                    if let Some(process) = owner.process.lock().unwrap().as_ref() {
                        process.stop();
                    }
                    break;
                }
            }
            let requests = owner
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, r)| r.done.0.lock().unwrap().is_none())
                .map(|(id, r)| (id.clone(), r.clone()))
                .collect::<Vec<_>>();
            for (id, request) in requests {
                owner.finish(&id, &request, "interrupted", &json!({"code":"process"}));
            }
            let mut slot = owner.process.lock().unwrap();
            if slot.as_ref().is_some_and(|p| p.id() == pid) {
                *slot = None;
            }
        });
        Ok(process)
    }
    pub fn start(self: &Arc<Self>, input: Start, channel: Channel<Value>) -> Result<Value, String> {
        self.start_checked(input, channel, |_, _, _, _| Ok(()), || Ok(()))
    }
    pub(super) fn start_checked(
        self: &Arc<Self>,
        input: Start,
        channel: Channel<Value>,
        preflight: impl FnOnce(
            &Start,
            &super::store::Conversation,
            &super::preferences::Connection,
            &super::generation::Context,
        ) -> Result<(), String>,
        check: impl Fn() -> Result<(), String>,
    ) -> Result<Value, String> {
        check()?;
        let admission = self
            .notifications_app
            .as_ref()
            .map(crate::push::capture)
            .unwrap_or_default();
        let mut services = self
            .services
            .lock()
            .map_err(|_| "Chat storage unavailable.")?;
        let store = services.store.as_mut().map_err(|e| e.clone())?;
        if let Some(accepted) = store.replay(&input)? {
            if let Some(request) = self
                .requests
                .lock()
                .unwrap()
                .get(&input.request_id)
                .cloned()
            {
                let mut delivery = request.delivery.lock().unwrap();
                *request.channel.lock().unwrap() = Some(channel.clone());
                channel
                    .send(delivery.subscribe())
                    .map_err(|_| "Chat channel unavailable.")?;
            } else {
                let message = store.message(&input.conversation_id, &accepted.assistant_id)?;
                let mut delivery = Delivery::new(&accepted.assistant_id, 4 * 1024 * 1024);
                delivery.snapshot["message"]["parts"] = message.parts.clone();
                for (index, part) in message
                    .parts
                    .as_array()
                    .ok_or("Invalid stored message.")?
                    .iter()
                    .enumerate()
                {
                    if matches!(part["type"].as_str(), Some("text" | "reasoning")) {
                        let mut block = json!({"index":index,"type":part["type"],"open":false});
                        if let Some(id) = part.get("id").filter(|id| id.is_string()) {
                            block["id"] = id.clone();
                        }
                        delivery.snapshot["blocks"][format!("stored-{index}")] = block;
                    }
                }
                delivery.terminal = Some(json!({"type":"terminal","status":message.status}));
                channel
                    .send(delivery.subscribe())
                    .map_err(|_| "Chat channel unavailable.")?;
            }
            return Ok(serde_json::to_value(accepted).unwrap());
        }
        runtime_supported()?;
        {
            let mut closing = self
                .closing
                .lock()
                .map_err(|_| "Chat close state unavailable.")?;
            closing.retain(|_, active| active());
            if closing.contains_key(&input.conversation_id) {
                return Err("This conversation is closing. Retry when the close finishes.".into());
            }
        }
        let loaded = store.conversation(&input.conversation_id)?;
        let connection_id = loaded
            .config
            .connection_id
            .as_deref()
            .ok_or("Configure a connection in Settings → Chat AI.")?
            .to_owned();
        if self.changing.lock().unwrap().contains(&connection_id) {
            return Err("This connection is being changed. Retry after Settings finishes.".into());
        }
        let settings = services.settings.as_ref().map_err(|e| e.clone())?;
        let connection = settings
            .data
            .connections
            .iter()
            .find(|c| c.id == connection_id && c.enabled)
            .ok_or("Choose an enabled AI connection.")?
            .clone();
        if loaded.config.model.is_empty() {
            return Err("Choose a model before sending.".into());
        }
        let requests = self.requests.lock().unwrap();
        if requests
            .values()
            .filter(|r| r.done.0.lock().unwrap().is_none())
            .count()
            >= 4
        {
            return Err("Four AI requests are active. Retry when one finishes.".into());
        }
        drop(requests);
        let store = services.store.as_ref().map_err(|e| e.clone())?;
        let mut context = super::generation::context(
            store,
            &self.owner.root,
            &input,
            &loaded,
            &connection,
            &check,
        )?;
        let mcp_session = self.attach_mcp_context(&mut context, &loaded);
        preflight(&input, &loaded, &connection, &context)?;
        check()?;
        let key = settings.key(&connection_id)?;
        let mut payload = context.payload;
        payload["apiKey"] = json!(key);
        if serde_json::to_vec(&payload)
            .map_err(|_| "Invalid context.")?
            .len()
            > 40 * 1024 * 1024
        {
            return Err("The conversation exceeds the 40 MiB context limit.".into());
        }
        let metadata = json!({"provider":connection.provider,"model":loaded.config.model,"connectionId":connection_id,"credentialRevision":connection.credential_revision});
        let accepted = services
            .store
            .as_mut()
            .map_err(|e| e.clone())?
            .begin_with_notifications(&input, &metadata, &admission)?;
        if accepted.repeated {
            return Ok(serde_json::to_value(accepted).unwrap());
        }
        let request = Arc::new(Request {
            conversation: Some(input.conversation_id.clone()),
            connection: connection_id,
            delivery: Mutex::new(Delivery::new(&input.assistant_id, 4 * 1024 * 1024)),
            channel: Mutex::new(Some(channel)),
            checkpoint_sequence: AtomicI64::new(0),
            saved_sequence: AtomicI64::new(-1),
            saved_bytes: AtomicI64::new(0),
            finalizing: AtomicBool::new(false),
            cancelled: AtomicBool::new(self.cancelled.lock().unwrap().remove(&input.request_id)),
            result: Mutex::new(Value::Null),
            done: (Mutex::new(None), Condvar::new()),
            mcp_session: Mutex::new(Some(mcp_session)),
            mcp_calls: Mutex::new(McpCalls::default()),
            mcp_result_bytes: AtomicUsize::new(0),
            mcp_task: Mutex::new(None),
        });
        {
            let mut requests = self.requests.lock().unwrap();
            // Persisted requests can be replayed from SQLite. Retain failed saves
            // and active streams, but release earlier responses in a long chat.
            requests.retain(|_, old| {
                old.conversation != request.conversation
                    || !matches!(*old.done.0.lock().unwrap(), Some(Ok(())))
            });
            requests.insert(input.request_id.clone(), request.clone());
        }
        if self.cancelled.lock().unwrap().remove(&input.request_id) {
            request.cancelled.store(true, Ordering::Release);
        }
        drop(services);
        if check().is_err() {
            request.cancelled.store(true, Ordering::Release);
        }
        if request.cancelled.load(Ordering::Acquire) {
            self.finish(&input.request_id, &request, "cancelled", &json!({}));
        } else {
            let outcome = self.process().and_then(|process| {
                if request.cancelled.load(Ordering::Acquire) {
                    return Err("cancelled".into());
                }
                check()?;
                process.generate(&input.request_id, &payload)?;
                if request.cancelled.load(Ordering::Acquire) {
                    process.cancel(&input.request_id)?;
                }
                Ok(())
            });
            if let Err(error) = outcome {
                self.finish(
                    &input.request_id,
                    &request,
                    if request.cancelled.load(Ordering::Acquire) {
                        "cancelled"
                    } else {
                        "failed"
                    },
                    &json!({"code":if error=="cancelled"{"cancelled"}else{"process"}}),
                );
            }
        }
        Ok(serde_json::to_value(accepted).unwrap())
    }
    fn event(self: &Arc<Self>, event: Event) {
        let Some(request) = self
            .requests
            .lock()
            .unwrap()
            .get(&event.request_id)
            .cloned()
        else {
            return;
        };
        if request.finalizing.load(Ordering::Acquire) || request.done.0.lock().unwrap().is_some() {
            return;
        }
        let terminal = matches!(event.r#type.as_str(), "completed" | "cancelled" | "failed");
        if request.cancelled.load(Ordering::Acquire) {
            if terminal {
                self.finish(&event.request_id, &request, "cancelled", &event.payload);
            }
            return;
        }
        if request.conversation.is_none() {
            match event.r#type.as_str() {
                "models" => *request.result.lock().unwrap() = event.payload,
                "completed" | "failed" | "cancelled" => {
                    let status = if request.cancelled.load(Ordering::Acquire) {
                        "cancelled"
                    } else {
                        event.r#type.as_str()
                    };
                    self.finish(&event.request_id, &request, status, &event.payload)
                }
                _ => (),
            }
            return;
        }
        match event.r#type.as_str() {
            "tool-call" => self.tool_call(&event.request_id, &request, &event.payload),
            "chunk" => {
                let packet = request
                    .delivery
                    .lock()
                    .unwrap()
                    .chunk(event.sequence, &event.payload);
                match packet {
                    Ok(Some(value)) => {
                        if let Some(channel) = request.channel.lock().unwrap().as_ref() {
                            let _ = channel.send(value);
                        }
                    }
                    Ok(None) => (),
                    Err(_) => {
                        let _ = self.cancel(&event.request_id);
                    }
                }
                let bytes = request.delivery.lock().unwrap().response_bytes as i64;
                if bytes - request.saved_bytes.load(Ordering::Acquire) >= 32 * 1024
                    && self
                        .checkpoint(&event.request_id, &request, "active", &json!({}))
                        .is_err()
                {
                    let _ = self.cancel(&event.request_id);
                }
            }
            "message-snapshot" => (),
            "completed" | "cancelled" | "failed" => {
                let status = if request.cancelled.load(Ordering::Acquire) {
                    "cancelled"
                } else {
                    event.r#type.as_str()
                };
                self.finish(&event.request_id, &request, status, &event.payload)
            }
            _ => (),
        }
    }

    fn tool_call(self: &Arc<Self>, id: &str, request: &Arc<Request>, payload: &Value) {
        let Some(session) = request.mcp_session.lock().unwrap().as_ref().cloned() else {
            let _ = self.cancel_known(id, request);
            return;
        };
        let Some(tool_call_id) = payload["toolCallId"].as_str().map(str::to_owned) else {
            let _ = self.cancel_known(id, request);
            return;
        };
        if !super::process::valid_id(&tool_call_id) {
            let _ = self.cancel_known(id, request);
            return;
        }
        let (tool_name, input) = (
            payload["toolName"].as_str().map(str::to_owned),
            payload.get("input").cloned().unwrap_or(Value::Null),
        );
        {
            let mut calls = request.mcp_calls.lock().unwrap();
            if calls.active.is_some()
                || calls.ids.contains(&tool_call_id)
                || calls.ids.len() >= MAX_MCP_CALLS
            {
                drop(calls);
                let _ = self.cancel_known(id, request);
                return;
            }
            calls.ids.insert(tool_call_id.clone());
            calls.active = Some(tool_call_id.clone());
        }
        if request.finalizing.load(Ordering::Acquire)
            || request.cancelled.load(Ordering::Acquire)
            || request.done.0.lock().unwrap().is_some()
        {
            return;
        }
        let input_size = serde_json::to_vec(&input)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX);
        let input_error = if input_size > MAX_MCP_INPUT_BYTES {
            Some("Lomi tool arguments exceed 64 KiB.")
        } else if !input.is_object() {
            Some("Lomi tool arguments must be a JSON object.")
        } else if tool_name
            .as_deref()
            .is_none_or(|name| !session.has_tool(name))
        {
            Some("Unknown Lomi MCP tool.")
        } else {
            None
        };
        let name = tool_name.unwrap_or_default();
        let backend = self.clone();
        let request = request.clone();
        let request_task = request.clone();
        let request_id = id.to_owned();
        let tool_call_for_task = tool_call_id.clone();
        register_mcp_task(&request, || {
            tauri::async_runtime::spawn(async move {
                if request_task.finalizing.load(Ordering::Acquire)
                    || request_task.cancelled.load(Ordering::Acquire)
                    || request_task.done.0.lock().unwrap().is_some()
                {
                    return;
                }
                let mut result = if let Some(message) = input_error {
                    lomi_mcp::chat::error_result(message)
                } else {
                    session.call(&name, input).await
                };
                let mut bytes = serde_json::to_vec(&result)
                .unwrap_or_else(|_| b"{\"isError\":true,\"content\":[{\"type\":\"text\",\"text\":\"Cannot encode Lomi result.\"}]}".to_vec());
                if bytes.len() > MAX_MCP_RESULT_BYTES {
                    result = lomi_mcp::chat::error_result("Lomi tool result exceeds 8 MiB.");
                    bytes = serde_json::to_vec(&result).unwrap_or_default();
                }
                let current = request_task.mcp_result_bytes.load(Ordering::Acquire);
                if current.saturating_add(bytes.len()) > MAX_MCP_RESULT_TOTAL_BYTES {
                    result = lomi_mcp::chat::error_result(
                        "Lomi tool results exceeded the 16 MiB request limit.",
                    );
                    bytes = serde_json::to_vec(&result).unwrap_or_default();
                }
                if current.saturating_add(bytes.len()) > MAX_MCP_RESULT_TOTAL_BYTES {
                    let _ = backend.cancel_known(&request_id, &request_task);
                    return;
                }
                request_task
                    .mcp_result_bytes
                    .fetch_add(bytes.len(), Ordering::AcqRel);
                if request_task.cancelled.load(Ordering::Acquire)
                    || request_task.finalizing.load(Ordering::Acquire)
                    || request_task.done.0.lock().unwrap().is_some()
                {
                    return;
                }
                let process = backend.process.lock().ok().and_then(|slot| slot.clone());
                let Some(process) = process else {
                    let _ = backend.cancel_known(&request_id, &request_task);
                    return;
                };
                let transfer_id = request_id.clone();
                let transfer_call_id = tool_call_for_task.clone();
                let completion_request = request_task.clone();
                let completion_call_id = tool_call_for_task.clone();
                let transfer = tauri::async_runtime::spawn_blocking(move || {
                    process.tool_result(&transfer_id, &transfer_call_id, &bytes, move || {
                        let mut calls = completion_request.mcp_calls.lock().unwrap();
                        if calls.active.as_deref() == Some(completion_call_id.as_str()) {
                            calls.active = None;
                        }
                        completion_request.mcp_task.lock().unwrap().take();
                    })
                })
                .await;
                if (transfer.is_err()
                    || transfer
                        .is_ok_and(|result| result.is_err() && result != Err("cancelled".into())))
                    && !request_task.cancelled.load(Ordering::Acquire)
                {
                    let _ = backend.cancel_known(&request_id, &request_task);
                }
            })
        });
    }
    fn finish(&self, id: &str, request: &Request, status: &str, result: &Value) {
        if request.finalizing.swap(true, Ordering::AcqRel) {
            return;
        }
        // A dead process has no acknowledged terminal response, even when
        // shutdown also stopped pending tool work.
        let status = if status != "interrupted" && request.cancelled.load(Ordering::Acquire) {
            "cancelled"
        } else {
            status
        };
        if request.mcp_calls.lock().unwrap().active.is_some() {
            if let Some(session) = request.mcp_session.lock().unwrap().as_ref() {
                session.close();
            }
        }
        if let Some(task) = request
            .mcp_task
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
        {
            task.abort();
        }
        request.mcp_session.lock().unwrap().take();
        if request.conversation.is_none() {
            let mut value = request.result.lock().unwrap();
            if !value.is_object() {
                *value = json!({});
            }
            value["status"] = json!(status);
            value["result"] = result.clone();
            *request.done.0.lock().unwrap() = Some(Ok(()));
            request.done.1.notify_all();
            return;
        }
        *request.result.lock().unwrap() = json!({"status":status,"result":result});
        let saved = self.checkpoint(id, request, status, result);
        let value = if let Err(error) = &saved {
            json!({"type":"storage-error","error":error})
        } else {
            json!({"type":"terminal","status":status,"result":result})
        };
        request.delivery.lock().unwrap().terminal = Some(value.clone());
        *request.done.0.lock().unwrap() = Some(saved);
        request.done.1.notify_all();
        if let Some(channel) = request.channel.lock().unwrap().as_ref() {
            let _ = channel.send(value);
        }
    }
    fn checkpoint(
        &self,
        id: &str,
        request: &Request,
        status: &str,
        result: &Value,
    ) -> Result<(), String> {
        let (parts, sequence, bytes) = {
            let delivery = request.delivery.lock().unwrap();
            (
                delivery.snapshot["message"]["parts"].clone(),
                delivery.sequence as i64,
                delivery.response_bytes as i64,
            )
        };
        if status == "active" && request.saved_sequence.load(Ordering::Acquire) == sequence {
            return Ok(());
        }
        let counter = request.checkpoint_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let saved = self
            .services
            .lock()
            .map_err(|_| "Chat storage unavailable.")?
            .store
            .as_mut()
            .map_err(|e| e.clone())?
            .checkpoint(id, counter, &parts, status, result);
        if saved.is_ok() {
            request.saved_sequence.store(sequence, Ordering::Release);
            request.saved_bytes.store(bytes, Ordering::Release);
        }
        saved
    }
    pub fn flush(&self, conversation: &str) -> Result<(), String> {
        let requests = self
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| r.conversation.as_deref() == Some(conversation))
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect::<Vec<_>>();
        for (id, request) in requests {
            let retry = request
                .done
                .0
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|r| r.is_err());
            if retry {
                request.finalizing.store(false, Ordering::Release);
                let result = request.result.lock().unwrap().clone();
                self.finish(
                    &id,
                    &request,
                    result["status"].as_str().unwrap_or("interrupted"),
                    &result["result"],
                );
                request.done.0.lock().unwrap().as_ref().unwrap().clone()?;
            }
        }
        Ok(())
    }
    pub fn close_connections(&self, connections: &[String]) -> Result<(), String> {
        let requests = self
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| connections.contains(&r.connection))
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect::<Vec<_>>();
        self.close_requests(requests)
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        if !super::process::valid_id(id) {
            return Err("Invalid request ID.".into());
        }
        if let Some(request) = self.requests.lock().unwrap().get(id).cloned() {
            self.cancel_known(id, &request)?;
        } else {
            let mut cancelled = self.cancelled.lock().unwrap();
            if cancelled.len() >= 256 {
                return Err("Too many pending cancellations.".into());
            }
            cancelled.insert(id.into());
        }
        Ok(())
    }
    pub(super) fn cancel_known(&self, id: &str, request: &Request) -> Result<(), String> {
        if request.finalizing.load(Ordering::Acquire) || request.done.0.lock().unwrap().is_some() {
            return Ok(());
        }
        request.cancelled.store(true, Ordering::Release);
        if request.mcp_calls.lock().unwrap().active.is_some() {
            if let Some(session) = request.mcp_session.lock().unwrap().as_ref() {
                session.close();
            }
        }
        if let Some(task) = request
            .mcp_task
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
        {
            task.abort();
        }
        let mut process_result = Ok(());
        if let Ok(slot) = self.process.try_lock() {
            if let Some(process) = slot.as_ref() {
                process_result = process.cancel(id);
            }
        }
        self.finish(id, request, "cancelled", &json!({}));
        process_result
    }
    pub fn close_all(&self) -> Result<(), String> {
        let requests = self
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect();
        self.close_requests(requests)?;
        let mut sessions = self.mcp_sessions.lock().unwrap();
        for entry in sessions.values() {
            entry.session.close();
        }
        sessions.clear();
        Ok(())
    }
    pub fn close(&self, conversations: &[String]) -> Result<(), String> {
        let requests = self
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| {
                r.conversation
                    .as_ref()
                    .is_some_and(|id| conversations.contains(id))
            })
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect::<Vec<_>>();
        self.close_requests(requests)?;
        self.release_mcp_sessions(conversations);
        Ok(())
    }
    pub(super) fn close_for_delete(&self, conversation: &str) -> Result<(), String> {
        let requests = self
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, request)| request.conversation.as_deref() == Some(conversation))
            .map(|(id, request)| (id.clone(), request.clone()))
            .collect();
        self.close_requests(requests)
    }
    pub(super) fn release_mcp_session(&self, conversation: &str) {
        if let Some(entry) = self.mcp_sessions.lock().unwrap().remove(conversation) {
            entry.session.close();
        }
    }
    fn release_mcp_sessions(&self, conversations: &[String]) {
        let mut sessions = self.mcp_sessions.lock().unwrap();
        for conversation in conversations {
            if let Some(entry) = sessions.remove(conversation) {
                entry.session.close();
            }
        }
    }
    fn close_requests(&self, requests: Vec<(String, Arc<Request>)>) -> Result<(), String> {
        for (id, request) in requests {
            self.cancel(&id)?;
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut done = request.done.0.lock().unwrap();
            while done.is_none() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err("Chat shutdown timed out. The view remains open.".into());
                }
                done = request.done.1.wait_timeout(done, remaining).unwrap().0;
            }
            done.as_ref().unwrap().clone()?;
        }
        Ok(())
    }
    pub fn stop(&self) {
        for request in self.requests.lock().unwrap().values() {
            request.cancelled.store(true, Ordering::Release);
            if let Some(task) = request
                .mcp_task
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
            {
                task.abort();
            }
        }
        let mut sessions = self.mcp_sessions.lock().unwrap();
        for entry in sessions.values() {
            entry.session.close();
        }
        sessions.clear();
        drop(sessions);
        if let Some(process) = self.process.lock().unwrap().take() {
            process.stop();
        }
    }
}

fn register_mcp_task(
    request: &Arc<Request>,
    spawn: impl FnOnce() -> tauri::async_runtime::JoinHandle<()>,
) -> bool {
    let mut slot = request.mcp_task.lock().unwrap();
    let task = spawn();
    if request.cancelled.load(Ordering::Acquire)
        || request.finalizing.load(Ordering::Acquire)
        || request.done.0.lock().unwrap().is_some()
    {
        task.abort();
        false
    } else {
        *slot = Some(task);
        true
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Backend {
    pub fn auxiliary(
        self: &Arc<Self>,
        connection_id: &str,
        model: &str,
        operation: &str,
    ) -> Result<Value, String> {
        if operation == "test-connection" {
            if model.is_empty() {
                return Err("Choose a model to test.".into());
            }
            super::store::Config {
                model: model.into(),
                ..Default::default()
            }
            .validate()?;
        }
        if !matches!(operation, "test-connection" | "list-models") {
            return Err("Unknown connection action.".into());
        }
        let services = self.services.lock().map_err(|_| "Settings unavailable.")?;
        if self.changing.lock().unwrap().contains(connection_id) {
            return Err("The connection is being changed.".into());
        }
        let settings = services.settings.as_ref().map_err(|e| e.clone())?;
        let connection = settings
            .data
            .connections
            .iter()
            .find(|c| c.id == connection_id && c.enabled)
            .ok_or("Choose an enabled connection.")?
            .clone();
        let key = settings.key(connection_id)?;
        let (id, request) = self.begin_auxiliary(connection_id)?;
        drop(services);
        let payload = json!({"operation":operation,"provider":connection.provider,"apiKey":key,"model":if operation=="list-models"{"catalog"}else{model},"assistantId":"connection-test","messages":[{"id":"test-user","role":"user","parts":[{"type":"text","text":"Reply with OK."}]}],"maxOutputTokens":32});
        let mut payload = payload;
        connection.configure_payload(&mut payload);
        let result = self.run_auxiliary(&id, request, payload)?;
        self.services
            .lock()
            .unwrap()
            .settings
            .as_mut()
            .map_err(|e| e.clone())?
            .record_result(
                connection_id,
                connection.credential_revision,
                model,
                operation,
                &result,
            )?;
        Ok(result)
    }

    pub fn preview_models(self: &Arc<Self>, provider: &str, key: &str) -> Result<Value, String> {
        if !matches!(
            provider,
            "openai" | "anthropic" | "google" | "xai" | "openrouter" | "deepseek" | "nvidia"
        ) {
            return Err("Choose a supported AI provider.".into());
        }
        if key.trim().is_empty() || key.len() > 8192 || key.contains(['\r', '\n', '\0']) {
            return Err("Enter a valid API key.".into());
        }
        // Reserve under the same lock as generation without saving a connection or key.
        let services = self.services.lock().map_err(|_| "Settings unavailable.")?;
        let (id, request) = self.begin_auxiliary("")?;
        drop(services);
        let payload = json!({"operation":"list-models","provider":provider,"apiKey":key,"model":"catalog","assistantId":"connection-test","messages":[{"id":"catalog-user","role":"user","parts":[{"type":"text","text":"List models."}]}],"maxOutputTokens":32});
        self.run_auxiliary(&id, request, payload)
    }

    fn begin_auxiliary(&self, connection_id: &str) -> Result<(String, Arc<Request>), String> {
        let mut requests = self.requests.lock().unwrap();
        if requests
            .values()
            .filter(|r| r.done.0.lock().unwrap().is_none())
            .count()
            >= 4
            || requests.values().any(|r| {
                !connection_id.is_empty()
                    && r.connection == connection_id
                    && r.conversation.is_none()
                    && r.done.0.lock().unwrap().is_none()
            })
        {
            return Err(
                "An AI connection operation is already active. Retry when it finishes.".into(),
            );
        }
        let id = format!(
            "aux-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let request = Arc::new(Request {
            conversation: None,
            connection: connection_id.into(),
            delivery: Mutex::new(Delivery::new("connection-test", 4 * 1024 * 1024)),
            channel: Mutex::new(None),
            checkpoint_sequence: AtomicI64::new(0),
            saved_sequence: AtomicI64::new(-1),
            saved_bytes: AtomicI64::new(0),
            finalizing: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            result: Mutex::new(Value::Null),
            done: (Mutex::new(None), Condvar::new()),
            mcp_session: Mutex::new(None),
            mcp_calls: Mutex::new(McpCalls::default()),
            mcp_result_bytes: AtomicUsize::new(0),
            mcp_task: Mutex::new(None),
        });
        requests.insert(id.clone(), request.clone());
        Ok((id, request))
    }

    fn run_auxiliary(
        self: &Arc<Self>,
        id: &str,
        request: Arc<Request>,
        payload: Value,
    ) -> Result<Value, String> {
        let launched = self.process().and_then(|process| {
            if request.cancelled.load(Ordering::Acquire) {
                self.finish(id, &request, "cancelled", &json!({}));
                return Ok(());
            }
            process.generate(id, &payload)?;
            if request.cancelled.load(Ordering::Acquire) {
                process.cancel(id)?;
            }
            Ok(())
        });
        if let Err(error) = launched {
            self.requests.lock().unwrap().remove(id);
            return Err(error);
        }
        let mut done = request.done.0.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(31);
        while done.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                drop(done);
                self.cancel(id)?;
                self.close_requests(vec![(id.into(), request.clone())])?;
                self.requests.lock().unwrap().remove(id);
                return Err("The connection operation timed out. It was cancelled.".into());
            }
            done = request.done.1.wait_timeout(done, remaining).unwrap().0;
        }
        let result = request.result.lock().unwrap().clone();
        drop(done);
        self.requests.lock().unwrap().remove(id);
        Ok(result)
    }
}

fn runtime_supported() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let supported = plist::Value::from_file("/System/Library/CoreServices/SystemVersion.plist")
            .ok()
            .and_then(|v| {
                v.as_dictionary()
                    .and_then(|d| d.get("ProductVersion"))
                    .and_then(|v| v.as_string())
                    .map(str::to_owned)
            })
            .is_some_and(|v| {
                let mut parts = v.split('.').filter_map(|p| p.parse::<u32>().ok());
                let major = parts.next().unwrap_or(0);
                major > 13 || (major == 13 && parts.next().unwrap_or(0) >= 5)
            });
        if !supported {
            return Err(
                "Chat AI generation requires macOS 13.5 or later. Local history remains available."
                    .into(),
            );
        }
    }
    Ok(())
}

pub(super) fn capability(provider: &str, model: &str, name: &str) -> bool {
    let value: Value =
        serde_json::from_str(include_str!("../../../src/chat/model-capabilities.json")).unwrap();
    value["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["provider"] == provider && v["id"] == model && v[name] == true)
}
