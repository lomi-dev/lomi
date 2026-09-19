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
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, Manager};

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
}
pub struct Backend {
    pub services: Mutex<Services>,
    pub owner: Owner,
    pub requests: Mutex<HashMap<String, Arc<Request>>>,
    pub changing: Mutex<HashSet<String>>,
    pub cancelled: Mutex<HashSet<String>>,
    process: Mutex<Option<Arc<Process>>>,
    node: PathBuf,
    bundle: PathBuf,
}
impl Backend {
    #[cfg(feature = "chat-probe")]
    pub fn probe_process_id(&self) -> Option<u32> {
        self.process.lock().unwrap().as_ref().map(|p| p.id())
    }
    pub fn open(app: &tauri::AppHandle) -> Result<Arc<Self>, String> {
        let root = app
            .path()
            .app_data_dir()
            .map_err(|_| "Cannot locate chat data.")?
            .join("chat-ai");
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
        let (node, bundle) = if cfg!(debug_assertions) {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            (
                root.join(format!(
                    "binaries/simplebench-node-{}{}",
                    env!("SIMPLEBENCH_AI_TARGET"),
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
                        "simplebench-node.exe"
                    } else {
                        "simplebench-node"
                    }),
                app.path()
                    .resource_dir()
                    .map_err(|_| "Cannot locate AI resources.")?
                    .join("ai-runtime/index.cjs"),
            )
        };
        #[cfg(feature = "chat-probe")]
        let bundle = if std::env::var_os("SIMPLEBENCH_CHAT_PROBE_DIRECTORY").is_some()
            && std::env::var_os("SIMPLEBENCH_CHAT_LIVE_KEYS_FILE").is_none()
        {
            bundle.with_file_name("fixture.cjs")
        } else {
            bundle
        };
        Ok(Arc::new(Self {
            services: Mutex::new(Services { store, settings }),
            owner,
            requests: Mutex::new(HashMap::new()),
            changing: Mutex::new(HashSet::new()),
            cancelled: Mutex::new(HashSet::new()),
            process: Mutex::new(None),
            node,
            bundle,
        }))
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
                    delivery.snapshot["blocks"][format!("stored-{index}")] =
                        json!({"index":index,"type":part["type"],"open":false});
                }
                delivery.terminal = Some(json!({"type":"terminal","status":message.status}));
                channel
                    .send(delivery.subscribe())
                    .map_err(|_| "Chat channel unavailable.")?;
            }
            return Ok(serde_json::to_value(accepted).unwrap());
        }
        runtime_supported()?;
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
        let key = settings.key(&connection_id)?;
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
        let context = store.preview(&input);
        let payload=context.and_then(|messages|{
            let mut result=Vec::new();
            let mut context_bytes = loaded.config.system.len();
            for (message, attachment_ids) in messages {
                if message.parts_version!=1{return Err("This message uses an unsupported parts version. Start a new conversation.".into());}
                let mut parts=Vec::new();
                for part in message.parts.as_array().ok_or("Invalid stored message.")? {
                    if part["type"]=="text"{parts.push(json!({"type":"text","text":part["text"]}));}
                    else if part["type"]!="reasoning"{return Err("This message contains unsupported content. Start a new conversation.".into());}
                }
                for id in attachment_ids {
                    let (attachment,bytes)=attachments::read_object(store,&self.owner.root,&id)?;
                    if attachment.mime!="text/plain" {
                        if !capability(&connection.provider, &loaded.config.model, "images") { return Err("Image input is not verified for this model. Choose a supported model or remove the images.".into()); }
                        parts.push(json!({"type":"file","mediaType":attachment.mime,"filename":attachment.name,"url":format!("data:{};base64,{}",attachment.mime,super::process::encode(&bytes))}));
                        continue;
                    }
                    let text=std::str::from_utf8(&bytes).map_err(|_|"Attachment encoding changed.")?.trim_start_matches('\u{feff}');
                    parts.push(json!({"type":"text","text":format!("\nAttached file: {}\n{}",attachment.name,text)}));
                }
                if parts.len() > 100 || result.len() >= 2000 {return Err("The conversation exceeds supported message limits. Start a new conversation.".into());}
                let value = json!({"id":message.id,"role":message.role,"parts":parts});
                context_bytes += serde_json::to_vec(&value).map_err(|_| "Invalid message.")?.len();
                if context_bytes > 40*1024*1024 { return Err("The conversation exceeds the 40 MiB context limit.".into()); }
                result.push(value);
            }
            Ok(json!({"provider":connection.provider,"apiKey":key,"model":loaded.config.model,"assistantId":input.assistant_id,"messages":result,"system":loaded.config.system,"maxOutputTokens":loaded.config.max_output_tokens}))
        })?;
        let mut payload = payload;
        if let Some(temperature) = loaded.config.temperature {
            if !capability(&connection.provider, &loaded.config.model, "temperature") {
                return Err(
                    "Temperature is not verified for this model. Reset it to the default.".into(),
                );
            }
            payload["temperature"] = json!(temperature);
        }
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
            .begin(&input, &metadata)?;
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
        if request.cancelled.load(Ordering::Acquire) {
            self.finish(&input.request_id, &request, "cancelled", &json!({}));
        } else {
            let outcome = self.process().and_then(|process| {
                if request.cancelled.load(Ordering::Acquire) {
                    return Err("cancelled".into());
                }
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
    fn event(&self, event: Event) {
        let Some(request) = self
            .requests
            .lock()
            .unwrap()
            .get(&event.request_id)
            .cloned()
        else {
            return;
        };
        if request.done.0.lock().unwrap().is_some() {
            return;
        }
        if request.conversation.is_none() {
            match event.r#type.as_str() {
                "models" => *request.result.lock().unwrap() = event.payload,
                "completed" | "failed" | "cancelled" => {
                    self.finish(&event.request_id, &request, &event.r#type, &event.payload)
                }
                _ => (),
            }
            return;
        }
        match event.r#type.as_str() {
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
                self.finish(&event.request_id, &request, &event.r#type, &event.payload)
            }
            _ => (),
        }
    }
    fn finish(&self, id: &str, request: &Request, status: &str, result: &Value) {
        if request.finalizing.swap(true, Ordering::AcqRel) {
            return;
        }
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
            if request.finalizing.load(Ordering::Acquire)
                || request.done.0.lock().unwrap().is_some()
            {
                return Ok(());
            }
            request.cancelled.store(true, Ordering::Release);
            if let Ok(slot) = self.process.try_lock() {
                if let Some(process) = slot.as_ref() {
                    process.cancel(id)?;
                }
            }
        } else {
            let mut cancelled = self.cancelled.lock().unwrap();
            if cancelled.len() >= 256 {
                return Err("Too many pending cancellations.".into());
            }
            cancelled.insert(id.into());
        }
        Ok(())
    }
    pub fn close_all(&self) -> Result<(), String> {
        let requests = self
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect();
        self.close_requests(requests)
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
        self.close_requests(requests)
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
        if let Some(process) = self.process.lock().unwrap().take() {
            process.stop();
        }
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

fn capability(provider: &str, model: &str, name: &str) -> bool {
    let value: Value =
        serde_json::from_str(include_str!("../../../src/chat/model-capabilities.json")).unwrap();
    value["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["provider"] == provider && v["id"] == model && v[name] == true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{
        preferences::Connection,
        store::{Config, Origin},
    };
    fn setup() -> (tempfile::TempDir, Arc<Backend>) {
        let temp = tempfile::tempdir().unwrap();
        let owner = Owner::acquire(temp.path().join("chat-ai")).unwrap();
        let store = Store::open(&owner.root);
        let mut settings = Settings::open(
            temp.path().join("preferences.json"),
            &owner.root,
            SystemSecrets,
        )
        .unwrap();
        let mut data = settings.data.clone();
        data.connections.push(Connection {
            id: "fixture".into(),
            name: "Fixture".into(),
            provider: "openai".into(),
            enabled: true,
            credential_revision: 0,
            secret_mode: "session".into(),
            secret_id: None,
            models: vec![],
            tested_model: None,
            test_status: None,
        });
        settings
            .save(data, 0, Some(("fixture", "fixture-only-key")))
            .unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let backend = Arc::new(Backend {
            services: Mutex::new(Services {
                store,
                settings: Ok(settings),
            }),
            owner,
            requests: Mutex::new(HashMap::new()),
            changing: Mutex::new(HashSet::new()),
            cancelled: Mutex::new(HashSet::new()),
            process: Mutex::new(None),
            node: root.join(format!(
                "binaries/simplebench-node-{}{}",
                env!("SIMPLEBENCH_AI_TARGET"),
                if cfg!(windows) { ".exe" } else { "" }
            )),
            bundle: root.join("../packages/ai-runtime/src/fixture.ts"),
        });
        (temp, backend)
    }
    fn input(backend: &Backend, id: &str) -> Start {
        let mut services = backend.services.lock().unwrap();
        let store = services.store.as_mut().unwrap();
        store
            .create(
                id,
                &Origin {
                    project_id: "p".into(),
                    project_name: "Project".into(),
                    workspace_id: "w".into(),
                    workspace_name: "Workspace".into(),
                },
                &Config {
                    connection_id: Some("fixture".into()),
                    model: "fixture-fast".into(),
                    configured: true,
                    ..Default::default()
                },
            )
            .unwrap();
        store.save_draft(id, "Unicode 日本語", 0).unwrap();
        Start {
            request_id: format!("request-{id}"),
            conversation_id: id.into(),
            assistant_id: format!("assistant-{id}"),
            user_id: format!("user-{id}"),
            expected_revision: 0,
            draft_revision: 1,
            action: "send".into(),
            target_id: None,
            text: "Unicode 日本語".into(),
        }
    }
    fn channel() -> Channel<Value> {
        Channel::new(|_| Ok(()))
    }
    #[test]
    fn model_preview_uses_private_runtime_without_saving_settings_or_keys() {
        let (root, backend) = setup();
        let path = root.path().join("preferences.json");
        let before = std::fs::read(&path).unwrap();
        let result = backend
            .preview_models("google", "fixture-unsaved-key")
            .unwrap();
        assert_eq!(result["status"], "completed");
        assert_eq!(result["models"], json!(["fixture-catalog-model"]));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(backend.requests.lock().unwrap().is_empty());
        let denied = backend
            .preview_models("openai", "fixture-catalog-denied")
            .unwrap();
        assert_eq!(denied["status"], "failed");
        assert_eq!(denied["result"]["code"], "auth");
        assert!(!denied.to_string().contains("PRIVATE"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(backend.requests.lock().unwrap().is_empty());
        backend.stop();
    }
    #[test]
    fn model_preview_rejects_unsupported_providers_and_invalid_keys_before_startup() {
        let (_root, backend) = setup();
        assert!(backend
            .preview_models("https://untrusted.example", "fixture-key")
            .is_err());
        for key in [
            "",
            "  ",
            "key\nheader",
            "key\rheader",
            "key\0header",
            &"x".repeat(8193),
        ] {
            assert!(backend.preview_models("openai", key).is_err());
        }
        assert!(backend.process.lock().unwrap().is_none());
        assert!(backend.requests.lock().unwrap().is_empty());
    }
    #[test]
    fn saved_model_refresh_still_records_catalog_results() {
        let (_root, backend) = setup();
        let result = backend.auxiliary("fixture", "", "list-models").unwrap();
        assert_eq!(result["status"], "completed");
        let services = backend.services.lock().unwrap();
        let settings = services.settings.as_ref().unwrap();
        assert_eq!(settings.data.revision, 2);
        assert_eq!(
            settings.data.connections[0].models,
            vec!["fixture-catalog-model"]
        );
        drop(services);
        backend.stop();
    }
    fn finished(request: &Request) -> Result<(), String> {
        let done = request.done.0.lock().unwrap();
        let result = request
            .done
            .1
            .wait_timeout_while(done, Duration::from_secs(10), |done| done.is_none())
            .unwrap()
            .0;
        result
            .as_ref()
            .expect("native fixture did not finish")
            .clone()
    }
    #[test]
    fn cancel_before_dispatch_persists_without_starting_node() {
        let (_root, backend) = setup();
        let input = input(&backend, "early");
        backend.cancel(&input.request_id).unwrap();
        backend.start(input.clone(), channel()).unwrap();
        assert!(backend.process.lock().unwrap().is_none());
        assert_eq!(
            backend
                .services
                .lock()
                .unwrap()
                .store
                .as_ref()
                .unwrap()
                .load("early", 0)
                .unwrap()
                .request
                .unwrap()["status"],
            "cancelled"
        );
        // A retransmission is acknowledged even when connection metadata is unavailable.
        backend.services.lock().unwrap().settings = Err("unavailable".into());
        assert_eq!(
            backend.start(input.clone(), channel()).unwrap()["repeated"],
            true
        );
        let mut wrong = input;
        wrong.text = "different".into();
        assert!(backend.start(wrong, channel()).is_err());
    }
    #[test]
    fn real_sdk_pipe_persists_without_a_ui_and_retries_failed_storage() {
        let (_root, backend) = setup();
        let input = input(&backend, "durable");
        backend.start(input.clone(), channel()).unwrap();
        let request = backend.requests.lock().unwrap()[&input.request_id].clone();
        *request.channel.lock().unwrap() = None;
        backend
            .services
            .lock()
            .unwrap()
            .store
            .as_mut()
            .unwrap()
            .connection
            .execute_batch("PRAGMA query_only=ON")
            .unwrap();
        assert!(finished(&request).is_err());
        assert!(
            !request.delivery.lock().unwrap().snapshot["message"]["parts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(backend.close(&["durable".into()]).is_err());
        backend
            .services
            .lock()
            .unwrap()
            .store
            .as_mut()
            .unwrap()
            .connection
            .execute_batch("PRAGMA query_only=OFF")
            .unwrap();
        backend.flush("durable").unwrap();
        backend.close(&["durable".into()]).unwrap();
        let stored = backend
            .services
            .lock()
            .unwrap()
            .store
            .as_ref()
            .unwrap()
            .load("durable", 0)
            .unwrap();
        assert_eq!(stored.request.unwrap()["status"], "cancelled");
        assert!(stored.messages.last().unwrap().parts[0]["text"]
            .as_str()
            .unwrap()
            .contains("日本語"));
        backend.stop();
    }
    #[test]
    fn unsupported_images_and_oversized_context_preserve_unsent_drafts() {
        let (_root, backend) = setup();
        let mut image_input = input(&backend, "image");
        {
            let mut services = backend.services.lock().unwrap();
            let store = services.store.as_mut().unwrap();
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 2)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            attachments::import(
                store,
                &backend.owner.root,
                attachments::Import {
                    conversation: "image",
                    id: "image-file",
                    name: "image.png",
                    bytes: bytes.get_ref(),
                    expected: 1,
                    approved: false,
                },
            )
            .unwrap();
            image_input.draft_revision = 2;
        }
        assert!(backend
            .start(image_input, channel())
            .unwrap_err()
            .contains("Image input is not verified"));
        let large = input(&backend, "large");
        {
            let mut services = backend.services.lock().unwrap();
            let store = services.store.as_mut().unwrap();
            let tx = store.connection.transaction().unwrap();
            let parts =
                json!([{"type":"text","text":"x".repeat(2 * 1024 * 1024 - 100)}]).to_string();
            let mut parent: Option<String> = None;
            for n in 0..21 {
                let user = format!("context-user-{n}");
                let assistant = format!("context-assistant-{n}");
                tx.execute(r#"INSERT INTO messages(id,conversation_id,parent_id,role,parts,status) VALUES (?1,'large',?2,'user','[{"type":"text","text":"continue"}]','completed')"#,rusqlite::params![user,parent]).unwrap();
                tx.execute("INSERT INTO messages(id,conversation_id,parent_id,role,parts,status) VALUES (?1,'large',?2,'assistant',?3,'completed')",rusqlite::params![assistant,user,parts]).unwrap();
                parent = Some(assistant);
            }
            tx.execute(
                "UPDATE conversations SET active_leaf=?1 WHERE id='large'",
                [parent],
            )
            .unwrap();
            tx.commit().unwrap();
        }
        assert!(backend
            .start(large, channel())
            .unwrap_err()
            .contains("40 MiB"));
        let services = backend.services.lock().unwrap();
        let store = services.store.as_ref().unwrap();
        assert_eq!(store.draft("large").unwrap().text, "Unicode 日本語");
        assert_eq!(
            store.draft("image").unwrap().attachments,
            vec!["image-file"]
        );
        assert!(backend.requests.lock().unwrap().is_empty());
        assert!(backend.process.lock().unwrap().is_none());
    }
    #[test]
    fn native_global_limit_preserves_fifth_draft_and_process_death_keeps_history() {
        let (_root, backend) = setup();
        let mut requests = Vec::new();
        for i in 0..4 {
            let input = input(&backend, &format!("c{i}"));
            backend.start(input.clone(), channel()).unwrap();
            requests.push(input);
        }
        let fifth = input(&backend, "fifth");
        assert!(backend
            .start(fifth, channel())
            .unwrap_err()
            .contains("Four"));
        assert_eq!(
            backend
                .services
                .lock()
                .unwrap()
                .store
                .as_ref()
                .unwrap()
                .draft("fifth")
                .unwrap()
                .text,
            "Unicode 日本語"
        );
        backend.stop();
        for input in requests {
            let request = backend.requests.lock().unwrap()[&input.request_id].clone();
            finished(&request).unwrap();
            assert_eq!(request.result.lock().unwrap()["status"], "interrupted");
        }
    }
}
