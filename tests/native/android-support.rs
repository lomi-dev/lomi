//! Opt-in native feasibility probe; its test commands are excluded from normal builds.
use std::{
    fs::{self, File},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, Manager, State, Window};
use tokio::sync::{watch, Notify};
use tonic::{transport::Channel as GrpcChannel, Request};

use crate::android_protocol as protocol;
use protocol::{emulator_controller_client::EmulatorControllerClient, Image, ImageFormat};

const MAX_PIXELS: u32 = 1920 * 1080;
const MAX_MESSAGE: usize = 9 * 1024 * 1024;
const FRAME_HEADER: usize = 60;
// Tauri 2.11 serializes Raw channel payloads smaller than 1024 bytes as JSON arrays.
const MIN_BINARY_PACKET: usize = 1024;
const GRPC_PORT: u16 = 18557;
const ADB_PORT: u16 = 15037;
const CONSOLE_PORT: u16 = 5580;

#[derive(Default)]
pub struct Probe {
    processes: Mutex<Processes>,
    stream: Mutex<Option<Stream>>,
    sequence: Mutex<u64>,
    finished: AtomicBool,
}
impl Probe {
    pub fn cleanup(&self) {
        self.finished.store(true, Ordering::Release);
        self.stream.lock().unwrap().take();
        let mut processes = self.processes.lock().unwrap();
        // Emergency cleanup is limited to disposable children owned by this probe.
        // Normal tests send SHUTDOWN and wait before destroying the test window.
        for child in [&mut processes.emulator.take(), &mut processes.adb.take()]
            .into_iter()
            .flatten()
        {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

/// The fixture must remain stoppable when the OS suspends its hidden webview.
pub fn watch_native_control(app: tauri::AppHandle) {
    let Ok(path) = root() else { return };
    tauri::async_runtime::spawn(async move {
        let mut last = String::new();
        while !app.state::<Probe>().finished.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let Ok(file) = File::open(path.join("native-control.json")) else {
                continue;
            };
            let mut bytes = Vec::new();
            if file.take(4097).read_to_end(&mut bytes).is_err() || bytes.len() > 4096 {
                continue;
            }
            let Ok(task) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let Some(id) = task["id"].as_str() else {
                continue;
            };
            if id == last || id.len() > 64 {
                continue;
            }
            let Some(action) = task["action"].as_str() else {
                continue;
            };
            if !matches!(
                action,
                "stop"
                    | "stop-and-quit"
                    | "reload"
                    | "present"
                    | "large-window"
                    | "small-window"
                    | "process-info"
            ) {
                continue;
            }
            last = id.to_string();
            let Some(window) = app.get_window("main") else {
                break;
            };
            let wrong_application = task
                .get("application")
                .is_some_and(|value| value.as_u64() != Some(u64::from(std::process::id())));
            let wrong_generation = task.get("generation").is_some_and(|value| {
                value.as_u64() != Some(app.state::<Probe>().processes.lock().unwrap().generation)
            });
            let result = if wrong_application || wrong_generation {
                Err("Native fixture identity changed; command rejected".to_string())
            } else if action == "reload" {
                app.state::<Probe>().stream.lock().unwrap().take();
                app.get_webview("main")
                    .ok_or_else(|| "Missing probe webview".to_string())
                    .and_then(|view| view.reload().map_err(|e| e.to_string()))
                    .map(|()| serde_json::Value::Null)
            } else {
                android_probe_control(
                    window.clone(),
                    app.state::<Probe>(),
                    if action == "stop-and-quit" {
                        "stop"
                    } else {
                        action
                    }
                    .into(),
                )
                .await
            };
            let report = match &result {
                Ok(value) => serde_json::json!({"id":id,"ok":true,"result":value}),
                Err(error) => serde_json::json!({"id":id,"ok":false,"error":error}),
            };
            let _ = fs::write(
                path.join("evidence/native-control.json"),
                report.to_string(),
            );
            if result.is_ok() && action == "stop-and-quit" {
                let _ = android_probe_control(window, app.state::<Probe>(), "quit".into()).await;
            }
        }
    });
}
#[derive(Default)]
struct Processes {
    emulator: Option<Child>,
    adb: Option<Child>,
    generation: u64,
}
struct Stream {
    epoch: u64,
    pending: Arc<Mutex<Option<u64>>>,
    ack: Arc<Notify>,
    reader: tauri::async_runtime::JoinHandle<()>,
    sender: tauri::async_runtime::JoinHandle<()>,
    stats: Arc<Mutex<Stats>>,
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.reader.abort();
        self.sender.abort();
    }
}
#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    grpc_frames: u64,
    grpc_bytes: u64,
    ipc_frames: u64,
    ipc_bytes: u64,
    encode_micros: u64,
    error: Option<String>,
}

fn root() -> Result<PathBuf, String> {
    let path = PathBuf::from(
        std::env::var("LOMI_ANDROID_PROBE_DIRECTORY").map_err(|_| "Probe is not enabled")?,
    );
    let consent: serde_json::Value = fs::read(path.join("evidence/consent.json"))
        .map_err(|e| e.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))?;
    if consent.get("accepted") != Some(&serde_json::Value::Bool(true)) {
        return Err("An isolated probe with explicit SDK consent is required".into());
    }
    path.canonicalize().map_err(|e| e.to_string())
}
fn check(window: &Window) -> Result<(), String> {
    crate::files::main_window(window)?;
    root().map(|_| ())
}

fn check_native_input(window: &Window) -> Result<(), String> {
    crate::files::main_window(window)?;
    if crate::android::fixture::directory()?.is_some() {
        Ok(())
    } else {
        root().map(|_| ())
    }
}

#[cfg(target_os = "macos")]
pub(super) async fn webkit_processes(app: &tauri::AppHandle) -> Result<serde_json::Value, String> {
    let mut result = serde_json::Map::new();
    for label in ["main", "settings"] {
        let Some(webview) = app.get_webview(label) else {
            continue;
        };
        let (send, receive) = tokio::sync::oneshot::channel();
        webview
            .with_webview(move |platform| {
                use objc2::{msg_send, runtime::AnyObject, sel};
                // Private WebKit diagnostics are confined to this opt-in test binary.
                // Query the WKWebView instead of guessing ownership from launchd PIDs.
                let value = unsafe {
                    let view = &*(platform.inner() as *const AnyObject);
                    let configuration: *const AnyObject = msg_send![view, configuration];
                    let store: *const AnyObject = msg_send![configuration, websiteDataStore];
                    let mut ids = serde_json::Map::new();
                    for (name, object, selector) in [
                        ("webContent", view, sel!(_webProcessIdentifier)),
                        ("gpu", view, sel!(_gpuProcessIdentifier)),
                        ("networking", &*store, sel!(_networkProcessIdentifier)),
                    ] {
                        let supported: bool = msg_send![object, respondsToSelector: selector];
                        if supported {
                            let pid: i32 = match name {
                                "webContent" => msg_send![object, _webProcessIdentifier],
                                "gpu" => msg_send![object, _gpuProcessIdentifier],
                                _ => msg_send![object, _networkProcessIdentifier],
                            };
                            ids.insert(name.into(), pid.into());
                        }
                    }
                    let window: *const AnyObject = msg_send![view, window];
                    let view_hidden: bool = msg_send![view, isHiddenOrHasHiddenAncestor];
                    ids.insert("viewHidden".into(), view_hidden.into());
                    if !window.is_null() {
                        let frame: objc2_foundation::NSRect = msg_send![window, frame];
                        let behavior: usize = msg_send![window, collectionBehavior];
                        let level: isize = msg_send![window, level];
                        let visible: bool = msg_send![window, isVisible];
                        let minimized: bool = msg_send![window, isMiniaturized];
                        let active_space: bool = msg_send![window, isOnActiveSpace];
                        let occlusion: usize = msg_send![window, occlusionState];
                        ids.insert(
                            "windowState".into(),
                            serde_json::json!({
                                "visible":visible,"minimized":minimized,
                                "activeSpace":active_space,"occlusion":occlusion,
                                "collectionBehavior":behavior,"level":level,
                                "frame":[frame.origin.x,frame.origin.y,frame.size.width,frame.size.height]
                            }),
                        );
                    }
                    serde_json::Value::Object(ids)
                };
                let _ = send.send(value);
            })
            .map_err(|error| error.to_string())?;
        let value = tokio::time::timeout(Duration::from_secs(5), receive)
            .await
            .map_err(|_| "WebKit process diagnostics timed out")?
            .map_err(|_| "WebKit process diagnostics were cancelled")?;
        result.insert(label.into(), value);
    }
    Ok(result.into())
}

#[cfg(target_os = "macos")]
pub(super) async fn present_on_spaces(window: &Window) -> Result<(), String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior as Behavior};
            let result = target
                .ns_window()
                .map_err(|error| error.to_string())
                .map(|pointer| {
                    // Only the measurement window joins full-screen/Stage Manager spaces.
                    // Ordinary application windows retain their existing desktop behavior.
                    let native = unsafe { &*pointer.cast::<NSWindow>() };
                    let mut behavior = native.collectionBehavior();
                    behavior.remove(
                        Behavior::MoveToActiveSpace
                            | Behavior::FullScreenPrimary
                            | Behavior::FullScreenNone
                            | Behavior::Primary
                            | Behavior::Auxiliary,
                    );
                    behavior.insert(
                        Behavior::CanJoinAllSpaces
                            | Behavior::FullScreenAuxiliary
                            | Behavior::CanJoinAllApplications,
                    );
                    native.setCollectionBehavior(behavior);
                });
            let _ = send.send(result);
        })
        .map_err(|error| error.to_string())?;
    receive.await.map_err(|error| error.to_string())?
}

fn command(path: PathBuf) -> Result<Command, String> {
    let root = root()?;
    let mut command = Command::new(path);
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if [
            "ANDROID_", "ADB_", "JAVA_", "JDK_", "_JAVA_", "REPO_", "QT_",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        {
            command.env_remove(key);
        }
    }
    command
        .current_dir(&root)
        .env("TMPDIR", root.join("logs"))
        .env("ANDROID_HOME", root.join("sdk"))
        .env("ANDROID_USER_HOME", root.join("user"))
        .env("ANDROID_EMULATOR_HOME", root.join("emulator-home"))
        .env("ANDROID_AVD_HOME", root.join("avd"))
        .env("ANDROID_ADB_SERVER_PORT", ADB_PORT.to_string())
        .env("ADB_SERVER_SOCKET", format!("tcp:127.0.0.1:{ADB_PORT}"))
        .stdin(Stdio::null());
    Ok(command)
}
async fn client() -> Result<EmulatorControllerClient<GrpcChannel>, String> {
    let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:18557")
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .connect()
        .await
        .map_err(|e| e.to_string())?;
    Ok(EmulatorControllerClient::new(channel)
        .max_decoding_message_size(MAX_MESSAGE)
        .max_encoding_message_size(4096))
}
fn request<T>(value: T) -> Result<Request<T>, String> {
    let token = fs::read_to_string(root()?.join("runtime/token")).map_err(|e| e.to_string())?;
    let mut request = Request::new(value);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", token.trim())
            .parse()
            .map_err(|_| "Invalid native token")?,
    );
    Ok(request)
}
fn format(width: u32, height: u32, png: bool) -> ImageFormat {
    ImageFormat {
        format: i32::from(!png),
        width,
        height,
        ..Default::default()
    }
}
fn frame_packet(
    frame: &Image,
    generation: u64,
    epoch: u64,
    sequence: u64,
    payload: &[u8],
    encoding: u32,
) -> Result<Vec<u8>, String> {
    let (width, height) = validate(frame)?;
    if payload.is_empty()
        || payload.len() > MAX_MESSAGE - FRAME_HEADER
        || encoding > 1
        || (encoding == 0 && payload.len() != width as usize * height as usize * 4)
    {
        return Err("Encoded Android frame exceeds the binary channel limit".into());
    }
    let length = (FRAME_HEADER + payload.len()).max(MIN_BINARY_PACKET);
    let mut packet = Vec::with_capacity(length);
    packet.extend_from_slice(b"SBAP");
    packet.extend_from_slice(&3u32.to_le_bytes());
    for n in [generation, epoch, sequence, frame.timestamp_us] {
        packet.extend_from_slice(&n.to_le_bytes());
    }
    packet.extend_from_slice(&width.to_le_bytes());
    packet.extend_from_slice(&height.to_le_bytes());
    let rotation = frame
        .format
        .as_ref()
        .and_then(|format| format.rotation.as_ref())
        .map_or(0, |rotation| rotation.rotation);
    if !(0..=3).contains(&rotation) {
        return Err("Invalid Android frame orientation".into());
    }
    packet.extend_from_slice(&rotation.to_le_bytes());
    packet.extend_from_slice(&encoding.to_le_bytes());
    packet.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    packet.extend_from_slice(payload);
    packet.resize(length, 0);
    Ok(packet)
}

fn validate(frame: &Image) -> Result<(u32, u32), String> {
    let format = frame.format.as_ref().ok_or("Frame has no format")?;
    let size = format
        .width
        .checked_mul(format.height)
        .filter(|pixels| *pixels > 0 && *pixels <= MAX_PIXELS)
        .ok_or("Frame pixel limit exceeded")?;
    if format.format != 1 || frame.image.len() != size as usize * 4 {
        return Err("Invalid RGBA frame length or format".into());
    }
    Ok((format.width, format.height))
}

pub fn page(view: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if view.label() != "main" || !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
    {
        return;
    }
    if let Ok(path) = root() {
        if let Ok(script) = fs::read_to_string(path.join("probe.js")) {
            let _ = view.eval(script);
        }
    }
}

#[tauri::command]
pub async fn android_probe_control(
    window: Window,
    state: State<'_, Probe>,
    action: String,
) -> Result<serde_json::Value, String> {
    check(&window)?;
    let path = root()?;
    match action.as_str() {
        "large-window" | "small-window" => {
            let (width, height) = if action == "large-window" {
                (1440.0, 900.0)
            } else {
                (520.0, 760.0)
            };
            window
                .set_min_size(Some(tauri::LogicalSize::new(480.0, 600.0)))
                .map_err(|e| e.to_string())?;
            window
                .set_size(tauri::LogicalSize::new(width, height))
                .map_err(|e| e.to_string())?;
            window
                .set_position(tauri::LogicalPosition::new(15.0, 40.0))
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({"width":width,"height":height}))
        }
        "present" => {
            if let Some(settings) = window.app_handle().get_window("settings") {
                settings.hide().map_err(|e| e.to_string())?;
            }
            window.show().map_err(|e| e.to_string())?;
            window
                .set_title("Android native transport probe")
                .map_err(|e| e.to_string())?;
            window.unminimize().map_err(|e| e.to_string())?;
            window.set_always_on_top(true).map_err(|e| e.to_string())?;
            window
                .set_visible_on_all_workspaces(true)
                .map_err(|e| e.to_string())?;
            #[cfg(target_os = "macos")]
            {
                present_on_spaces(&window).await?;
                window.app_handle().show().map_err(|e| e.to_string())?;
            }
            window.set_focus().map_err(|e| e.to_string())?;
            Ok(serde_json::json!({}))
        }
        "minimize" | "hide" => {
            state.stream.lock().unwrap().take();
            if action == "minimize" {
                window.minimize().map_err(|e| e.to_string())?;
            } else {
                window.hide().map_err(|e| e.to_string())?;
            }
            Ok(serde_json::json!({}))
        }
        "process-info" => {
            #[cfg(target_os = "macos")]
            let webkit = webkit_processes(window.app_handle()).await?;
            #[cfg(not(target_os = "macos"))]
            let webkit = serde_json::Value::Null;
            let processes = state.processes.lock().unwrap();
            Ok(serde_json::json!({
                "application":std::process::id(),
                "emulator":processes.emulator.as_ref().map(Child::id),
                "adb":processes.adb.as_ref().map(Child::id),
                "generation":processes.generation,
                "webkit":webkit,
                "window":{
                    "physical":window.inner_size().map_err(|e|e.to_string())?,
                    "scaleFactor":window.scale_factor().map_err(|e|e.to_string())?
                }
            }))
        }
        "instruction" => fs::read(path.join("instruction.json"))
            .map_err(|e| e.to_string())
            .and_then(|data| serde_json::from_slice(&data).map_err(|e| e.to_string())),
        "start" => {
            let avd =
                std::env::var("LOMI_ANDROID_PROBE_AVD").unwrap_or_else(|_| "sb_stage0".into());
            if avd != "sb_stage0"
                && !avd.strip_prefix("sb_stage0_").is_some_and(|suffix| {
                    !suffix.is_empty()
                        && suffix.len() <= 24
                        && suffix
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                })
            {
                return Err("Invalid isolated fixture AVD name".into());
            }
            let mut processes = state.processes.lock().unwrap();
            if let Some(child) = processes.emulator.as_mut() {
                if child.try_wait().map_err(|e| e.to_string())?.is_none() {
                    return Ok(
                        serde_json::json!({"pid":child.id(),"generation":processes.generation}),
                    );
                }
            }
            for port in [GRPC_PORT, CONSOLE_PORT, CONSOLE_PORT + 1] {
                TcpListener::bind(("127.0.0.1", port))
                    .map_err(|e| format!("Probe port {port}: {e}"))?;
            }
            if processes.adb.is_none() {
                TcpListener::bind(("127.0.0.1", ADB_PORT))
                    .map_err(|e| format!("Probe ADB port is occupied: {e}"))?;
                let log = File::create(path.join("logs/adb.log")).map_err(|e| e.to_string())?;
                let child = command(path.join("sdk/platform-tools/adb"))?
                    .args(["-L", "tcp:15037", "server", "nodaemon"])
                    .stdout(log.try_clone().map_err(|e| e.to_string())?)
                    .stderr(log)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                processes.adb = Some(child);
            }
            let log = File::create(path.join("logs/emulator.log")).map_err(|e| e.to_string())?;
            let child = command(path.join("sdk/emulator/emulator"))?
                .args([
                    "-avd",
                    &avd,
                    "-no-window",
                    "-no-audio",
                    "-no-boot-anim",
                    "-camera-back",
                    "none",
                    "-camera-front",
                    "none",
                    "-gpu",
                    "host",
                    "-cores",
                    "2",
                    "-memory",
                    "2048",
                    "-vsync-rate",
                    "30",
                    "-port",
                    "5580",
                    "-grpc",
                    "18557",
                    "-grpc-use-jwt",
                    "-no-snapshot",
                    "-no-metrics",
                    "-crash-report-mode",
                    "disabled",
                    "-append-userspace-opt",
                    "androidboot.lomi.device=00000000-0000-0000-0000-000000000001",
                    "-adb-path",
                ])
                .arg(path.join("runtime/no-external-adb"))
                .args(["-grpc-allowlist"])
                .arg(path.join("runtime/allowlist.json"))
                .stdout(log.try_clone().map_err(|e| e.to_string())?)
                .stderr(log)
                .spawn()
                .map_err(|e| e.to_string())?;
            let pid = child.id();
            processes.emulator = Some(child);
            processes.generation += 1;
            Ok(serde_json::json!({"pid":pid,"generation":processes.generation}))
        }
        "status" => {
            let reply = client()
                .await?
                .get_status(request(())?)
                .await
                .map_err(|e| e.to_string())?
                .into_inner();
            // Emulator 37.1.11 still leaves platformConfig empty on the tested AOSP image.
            #[allow(deprecated)]
            let legacy_hardware = reply.hardware_config.as_ref();
            let dimension = |name: &str| {
                reply
                    .platform_config
                    .get(name)
                    .or_else(|| {
                        legacy_hardware
                            .and_then(|config| config.entry.iter().find(|entry| entry.key == name))
                            .map(|entry| &entry.value)
                    })
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| (1..=1920).contains(value))
            };
            let width = dimension("hw.lcd.width").ok_or("Missing native display width")?;
            let height = dimension("hw.lcd.height").ok_or("Missing native display height")?;
            if width * height > MAX_PIXELS {
                return Err("Native display exceeds fixture limit".into());
            }
            Ok(
                serde_json::json!({"booted":reply.booted,"display":{"width":width,"height":height},"status":format!("{reply:?}")}),
            )
        }
        "auth" => {
            let mut results = serde_json::Map::new();
            let mut cases = vec!["missing", "invalid", "expired", "wrong-audience"];
            if path.join("runtime/previous-token").is_file() {
                cases.push("previous-instance");
            }
            for name in cases {
                let mut req = Request::new(());
                if name != "missing" {
                    let token = if name == "invalid" {
                        "invalid".to_string()
                    } else {
                        let name = if name == "previous-instance" {
                            "previous-token"
                        } else {
                            name
                        };
                        fs::read_to_string(path.join(format!("runtime/{name}")))
                            .map_err(|e| e.to_string())?
                    };
                    req.metadata_mut().insert(
                        "authorization",
                        format!("Bearer {}", token.trim())
                            .parse()
                            .map_err(|_| "Invalid probe token")?,
                    );
                }
                let code = match client().await?.get_status(req).await {
                    Ok(_) => "UNEXPECTED_SUCCESS".into(),
                    Err(e) => format!("{:?}", e.code()),
                };
                results.insert(name.into(), serde_json::json!(code));
            }
            Ok(results.into())
        }
        "rtc" => {
            let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:18557")
                .timeout(Duration::from_secs(5))
                .connect()
                .await
                .map_err(|e| e.to_string())?;
            let mut client = tonic::client::Grpc::new(channel);
            client.ready().await.map_err(|e| e.to_string())?;
            let response: Result<tonic::Response<()>, _> = client
                .unary(
                    request(())?,
                    tonic::codegen::http::uri::PathAndQuery::from_static(
                        "/android.emulation.control.Rtc/requestRtcStream",
                    ),
                    tonic_prost::ProstCodec::default(),
                )
                .await;
            Ok(match response {
                Ok(_) => serde_json::json!({"available":true}),
                Err(error) => {
                    serde_json::json!({"available":false,"code":format!("{:?}",error.code()),"message":error.message()})
                }
            })
        }
        "quit" => {
            let mut processes = state.processes.lock().unwrap();
            if let Some(emulator) = processes.emulator.as_mut() {
                if emulator.try_wait().map_err(|e| e.to_string())?.is_none() {
                    return Err("Stop the test emulator before quitting".into());
                }
            }
            if let Some(mut adb) = processes.adb.take() {
                if adb.try_wait().map_err(|e| e.to_string())?.is_none() {
                    adb.kill().map_err(|e| e.to_string())?;
                }
                adb.wait().map_err(|e| e.to_string())?;
            }
            window.destroy().map_err(|error| error.to_string())?;
            Ok(serde_json::Value::Null)
        }
        "screenshot" => {
            let image = client()
                .await?
                .get_screenshot(request(format(1080, 1920, true))?)
                .await
                .map_err(|e| e.to_string())?
                .into_inner();
            fs::write(path.join("evidence/screen.png"), &image.image).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({"bytes":image.image.len()}))
        }
        "stats" => {
            let stream = state.stream.lock().unwrap();
            Ok(stream
                .as_ref()
                .map(|s| serde_json::to_value(&*s.stats.lock().unwrap()).unwrap())
                .unwrap_or(serde_json::Value::Null))
        }
        "unsubscribe" => {
            let stream = state.stream.lock().unwrap().take();
            Ok(stream
                .as_ref()
                .map(|s| serde_json::to_value(&*s.stats.lock().unwrap()).unwrap())
                .unwrap_or(serde_json::Value::Null))
        }
        "stop" => {
            state.stream.lock().unwrap().take();
            {
                let mut processes = state.processes.lock().unwrap();
                if let Some(status) = processes
                    .emulator
                    .as_mut()
                    .map(|child| child.try_wait())
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .flatten()
                {
                    processes.emulator = None;
                    return Ok(serde_json::json!({"exit":status.code()}));
                }
                if processes.emulator.is_none() {
                    return Ok(serde_json::Value::Null);
                }
            }
            client()
                .await?
                .set_vm_state(request(protocol::VmRunState {
                    state: protocol::vm_run_state::RunState::Shutdown as i32,
                })?)
                .await
                .map_err(|e| e.to_string())?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                {
                    let mut processes = state.processes.lock().unwrap();
                    if let Some(child) = processes.emulator.as_mut() {
                        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                            processes.emulator = None;
                            return Ok(serde_json::json!({"exit":status.code()}));
                        }
                    } else {
                        return Ok(serde_json::Value::Null);
                    }
                }
                if Instant::now() > deadline {
                    return Err("Stop timed out; process handle retained".into());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        _ => Err("Unknown probe action".into()),
    }
}

#[tauri::command]
pub async fn android_probe_subscribe(
    window: Window,
    state: State<'_, Probe>,
    width: u32,
    height: u32,
    encoding: String,
    frames: Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<u64, String> {
    check(&window)?;
    if !matches!(encoding.as_str(), "rgba" | "jpeg") {
        return Err("Unknown probe frame encoding".into());
    }
    if width == 0 || height == 0 || width.checked_mul(height).is_none_or(|n| n > MAX_PIXELS) {
        return Err("Invalid stream dimensions".into());
    }
    state.stream.lock().unwrap().take();
    let epoch = {
        let mut n = state.sequence.lock().unwrap();
        *n += 1;
        *n
    };
    let generation = state.processes.lock().unwrap().generation;
    let mut input = client()
        .await?
        .stream_screenshot(request(format(width, height, false))?)
        .await
        .map_err(|e| e.to_string())?
        .into_inner();
    let (latest, mut receiver) = watch::channel::<Option<Image>>(None);
    let stats = Arc::new(Mutex::new(Stats::default()));
    let reader_stats = stats.clone();
    let reader = tauri::async_runtime::spawn(async move {
        loop {
            let message = tokio::select! { message = input.message() => message, _ = latest.closed() => break };
            match message {
                Ok(Some(frame)) => {
                    if let Err(error) = validate(&frame) {
                        reader_stats.lock().unwrap().error = Some(error);
                        break;
                    }
                    let mut stats = reader_stats.lock().unwrap();
                    stats.grpc_frames += 1;
                    stats.grpc_bytes += frame.image.len() as u64;
                    drop(stats);
                    latest.send_replace(Some(frame));
                }
                Ok(None) => break,
                Err(error) => {
                    reader_stats.lock().unwrap().error = Some(error.to_string());
                    break;
                }
            }
        }
    });
    let pending = Arc::new(Mutex::new(None));
    let ack = Arc::new(Notify::new());
    let sender_pending = pending.clone();
    let sender_ack = ack.clone();
    let sender_stats = stats.clone();
    let sender = tauri::async_runtime::spawn(async move {
        let mut compressor = match turbojpeg::Compressor::new() {
            Ok(value) => value,
            Err(error) => {
                sender_stats.lock().unwrap().error = Some(error.to_string());
                return;
            }
        };
        let mut sequence = 0u64;
        let mut last_sent = Instant::now() - Duration::from_secs(1);
        while receiver.changed().await.is_ok() {
            tokio::time::sleep_until((last_sent + Duration::from_micros(33_334)).into()).await;
            let Some(frame) = receiver.borrow_and_update().clone() else {
                continue;
            };
            let Ok((width, height)) = validate(&frame) else {
                break;
            };
            last_sent = Instant::now();
            sequence += 1;
            let packet = if encoding == "rgba" {
                match frame_packet(&frame, generation, epoch, sequence, &frame.image, 0) {
                    Ok(packet) => packet,
                    Err(error) => {
                        sender_stats.lock().unwrap().error = Some(error);
                        break;
                    }
                }
            } else {
                let started = Instant::now();
                let encoded = match compressor.compress_to_owned(turbojpeg::Image {
                    pixels: frame.image.as_ref(),
                    width: width as usize,
                    pitch: width as usize * 4,
                    height: height as usize,
                    format: turbojpeg::PixelFormat::RGBA,
                }) {
                    Ok(value) => value,
                    Err(error) => {
                        sender_stats.lock().unwrap().error = Some(error.to_string());
                        break;
                    }
                };
                sender_stats.lock().unwrap().encode_micros += started.elapsed().as_micros() as u64;
                match frame_packet(&frame, generation, epoch, sequence, &encoded, 1) {
                    Ok(packet) => packet,
                    Err(error) => {
                        sender_stats.lock().unwrap().error = Some(error);
                        break;
                    }
                }
            };
            drop(frame);
            let bytes = packet.len() as u64;
            *sender_pending.lock().unwrap() = Some(sequence);
            if let Err(error) = frames.send(tauri::ipc::InvokeResponseBody::Raw(packet)) {
                sender_stats.lock().unwrap().error = Some(error.to_string());
                break;
            }
            {
                let mut stats = sender_stats.lock().unwrap();
                stats.ipc_frames += 1;
                stats.ipc_bytes += bytes;
            }
            let wait = async {
                loop {
                    let notified = sender_ack.notified();
                    if sender_pending.lock().unwrap().is_none() {
                        break;
                    }
                    notified.await;
                }
            };
            if tokio::time::timeout(Duration::from_secs(2), wait)
                .await
                .is_err()
            {
                sender_stats.lock().unwrap().error = Some("ACK timeout".into());
                break;
            }
        }
        // Dropping the receiver makes the producer's next iteration terminate.
        drop(receiver);
    });
    *state.stream.lock().unwrap() = Some(Stream {
        epoch,
        pending,
        ack,
        reader,
        sender,
        stats,
    });
    Ok(epoch)
}

#[tauri::command]
pub fn android_probe_ack(
    window: Window,
    state: State<'_, Probe>,
    epoch: u64,
    sequence: u64,
) -> Result<(), String> {
    check(&window)?;
    if let Some(stream) = state.stream.lock().unwrap().as_ref() {
        if stream.epoch == epoch {
            let mut pending = stream.pending.lock().unwrap();
            if *pending == Some(sequence) {
                *pending = None;
                stream.ack.notify_one();
            }
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Input {
    Key {
        key: String,
    },
    Text {
        text: String,
    },
    Paste {
        text: String,
    },
    Touch {
        x: i32,
        y: i32,
        pressure: i32,
    },
    Ime {
        action: String,
        text: String,
    },
    Rotate {
        #[serde(rename = "quarterTurns")]
        quarter_turns: u32,
    },
}

#[tauri::command]
pub async fn android_probe_native_text(
    window: Window,
    action: String,
    text: String,
) -> Result<(), String> {
    check_native_input(&window)?;
    if text.len() > 1024 || !["compose", "commit"].contains(&action.as_str()) {
        return Err("Invalid native text fixture".into());
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::{msg_send, runtime::AnyObject, sel};
        use objc2_foundation::{NSRange, NSString};
        let (tx, rx) = tokio::sync::oneshot::channel();
        let target = window.clone();
        window.run_on_main_thread(move || {
            let result = (|| {
                let native = target.ns_window().map_err(|e| e.to_string())?;
                let native = unsafe { &*native.cast::<AnyObject>() };
                let responder: *mut AnyObject = unsafe { msg_send![native, firstResponder] };
                if responder.is_null() { return Err("No native text responder".into()); }
                let selector = if action == "compose" { sel!(setMarkedText:selectedRange:replacementRange:) } else { sel!(insertText:replacementRange:) };
                let supported: bool = unsafe { msg_send![responder, respondsToSelector: selector] };
                if !supported { return Err("Responder has no native text input API".into()); }
                let value = NSString::from_str(&text);
                let replacement = NSRange::new(isize::MAX as usize, 0);
                if action == "compose" {
                    let selected = NSRange::new(text.encode_utf16().count(), 0);
                    unsafe { let _: () = msg_send![responder, setMarkedText: &*value, selectedRange: selected, replacementRange: replacement]; }
                } else {
                    unsafe { let _: () = msg_send![responder, insertText: &*value, replacementRange: replacement]; }
                }
                Ok(())
            })();
            let _ = tx.send(result);
        }).map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    Err("Native host text fixture has only been implemented for macOS".into())
}

#[tauri::command]
pub async fn android_probe_native_pointer(
    window: Window,
    phase: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    check_native_input(&window)?;
    let kind = match phase.as_str() {
        "down" => 1usize,
        "up" => 2usize,
        "drag" => 6usize,
        _ => return Err("Invalid native pointer phase".into()),
    };
    let size = window
        .inner_size()
        .map_err(|error| error.to_string())?
        .to_logical::<f64>(window.scale_factor().map_err(|error| error.to_string())?);
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x >= size.width || y >= size.height
    {
        return Err("Native fixture pointer is outside its own window".into());
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::{class, msg_send, runtime::AnyObject};
        use objc2_foundation::{NSPoint, NSRect};
        let (send, receive) = tokio::sync::oneshot::channel();
        let target = window.clone();
        window
            .run_on_main_thread(move || {
                let result = (|| {
                    let pointer = target.ns_window().map_err(|error| error.to_string())?;
                    let native = unsafe { &*pointer.cast::<AnyObject>() };
                    // Dispatch inside the owned window, never through the global OS event queue.
                    unsafe {
                        let content: *const AnyObject = msg_send![native, contentView];
                        if content.is_null() {
                            return Err("Missing fixture content view".to_string());
                        }
                        let bounds: NSRect = msg_send![content, bounds];
                        let number: isize = msg_send![native, windowNumber];
                        let info: *const AnyObject = msg_send![class!(NSProcessInfo), processInfo];
                        let time: f64 = msg_send![info, systemUptime];
                        let event: *const AnyObject = msg_send![class!(NSEvent),
                            mouseEventWithType: kind,
                            location: NSPoint::new(x, bounds.size.height - y),
                            modifierFlags: 0usize,
                            timestamp: time,
                            windowNumber: number,
                            context: std::ptr::null::<AnyObject>(),
                            eventNumber: 0isize,
                            clickCount: 1isize,
                            pressure: if kind == 2 { 0.0f32 } else { 1.0f32 }
                        ];
                        if event.is_null() {
                            return Err("Native pointer creation failed".to_string());
                        }
                        let _: () = msg_send![native, sendEvent: event];
                    }
                    Ok(())
                })();
                let _ = send.send(result);
            })
            .map_err(|error| error.to_string())?;
        receive.await.map_err(|error| error.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Err("Native pointer fixture is qualified only for macOS".into())
    }
}

fn ime(action: &str, text: &str) -> Result<(), String> {
    if !["commit", "compose", "finish", "delete"].contains(&action) || text.len() > 16384 {
        return Err("Invalid IME operation".into());
    }
    fn service(socket: &mut TcpStream, name: &str) -> Result<(), String> {
        socket
            .write_all(format!("{:04x}{name}", name.len()).as_bytes())
            .map_err(|e| e.to_string())?;
        let mut status = [0; 4];
        socket.read_exact(&mut status).map_err(|e| e.to_string())?;
        if &status != b"OKAY" {
            return Err("Isolated ADB service is unavailable".into());
        }
        Ok(())
    }
    // This probe uses its own server and never invokes an ADB client that can restart it.
    let mut socket =
        TcpStream::connect_timeout(&([127, 0, 0, 1], ADB_PORT).into(), Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    socket
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    service(&mut socket, "host:transport:emulator-5580")?;
    service(&mut socket, "localabstract:lomi.input.v1")?;
    let bytes =
        serde_json::to_vec(&serde_json::json!({"version":1,"id":1,"deviceId":"00000000-0000-0000-0000-000000000001","generationKey":"00000000-0000-0000-0000-000000000002","action":action,"text":text}))
            .map_err(|e| e.to_string())?;
    socket
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .map_err(|e| e.to_string())?;
    socket.write_all(&bytes).map_err(|e| e.to_string())?;
    let mut length = [0; 4];
    socket.read_exact(&mut length).map_err(|e| e.to_string())?;
    let length = u32::from_be_bytes(length) as usize;
    if length > 4096 {
        return Err("IME response limit exceeded".into());
    }
    let mut bytes = vec![0; length];
    socket.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let response: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if response["version"] != 1 || response["id"] != 1 || response["ok"] != true {
        return Err(format!("Guest IME rejected input: {}", response["error"]));
    }
    Ok(())
}
#[tauri::command]
pub async fn android_probe_input(window: Window, input: Input) -> Result<(), String> {
    check(&window)?;
    if let Input::Ime { action, text } = input {
        return tauri::async_runtime::spawn_blocking(move || ime(&action, &text))
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut client = client().await?;
    match input {
        Input::Ime { .. } => unreachable!(),
        Input::Rotate { quarter_turns } => {
            let z = match quarter_turns {
                0 => 0.0,
                1 => 90.0,
                2 => 180.0,
                3 => -90.0,
                _ => return Err("Invalid Android orientation".into()),
            };
            client
                .set_physical_model(request(protocol::PhysicalModelValue {
                    target: protocol::physical_model_value::PhysicalType::Rotation as i32,
                    value: Some(protocol::ParameterValue {
                        data: vec![0.0, 0.0, z],
                    }),
                    interpolation: protocol::physical_model_value::Interpolation::Step as i32,
                    ..Default::default()
                })?)
                .await
                .map_err(|error| error.to_string())?;
        }
        Input::Key { key } => {
            if key.len() > 64 {
                return Err("Key too long".into());
            }
            client
                .send_key(request(protocol::KeyboardEvent {
                    key,
                    event_type: 2,
                    ..Default::default()
                })?)
                .await
                .map_err(|e| e.to_string())?;
        }
        Input::Text { text } => {
            if text.len() > 1024 {
                return Err("Text too long".into());
            }
            client
                .send_key(request(protocol::KeyboardEvent {
                    text,
                    ..Default::default()
                })?)
                .await
                .map_err(|e| e.to_string())?;
        }
        Input::Paste { text } => {
            if text.len() > 1024 {
                return Err("Paste too long".into());
            }
            client
                .set_clipboard(request(protocol::ClipData { text })?)
                .await
                .map_err(|e| e.to_string())?;
            client
                .send_key(request(protocol::KeyboardEvent {
                    key: "Paste".into(),
                    event_type: 2,
                    ..Default::default()
                })?)
                .await
                .map_err(|e| e.to_string())?;
        }
        Input::Touch { x, y, pressure } => {
            if !(0..1920).contains(&x) || !(0..1920).contains(&y) || !(0..=1024).contains(&pressure)
            {
                return Err("Invalid touch".into());
            }
            client
                .send_touch(request(protocol::TouchEvent {
                    touches: vec![protocol::Touch {
                        x,
                        y,
                        pressure,
                        identifier: 0,
                        ..Default::default()
                    }],
                    ..Default::default()
                })?)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
#[tauri::command]
pub fn android_probe_report(
    window: Window,
    name: String,
    data: serde_json::Value,
) -> Result<(), String> {
    check(&window)?;
    if name.is_empty()
        || name.len() > 64
        || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err("Invalid probe report name".into());
    }
    let bytes = serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Report too large".into());
    }
    fs::write(root()?.join("evidence").join(format!("{name}.json")), bytes)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_orientation_is_explicit_and_unknown_values_are_rejected() {
        let mut frame = Image {
            format: Some(format(2, 2, false)),
            image: vec![255; 16].into(),
            ..Default::default()
        };
        for rotation in 0..=3 {
            frame.format.as_mut().unwrap().rotation = Some(protocol::Rotation {
                rotation,
                ..Default::default()
            });
            let packet = frame_packet(&frame, 1, 2, 3, &frame.image, 0).unwrap();
            assert_eq!(u32::from_le_bytes(packet[4..8].try_into().unwrap()), 3);
            assert_eq!(
                i32::from_le_bytes(packet[48..52].try_into().unwrap()),
                rotation
            );
        }
        frame
            .format
            .as_mut()
            .unwrap()
            .rotation
            .as_mut()
            .unwrap()
            .rotation = 4;
        assert!(frame_packet(&frame, 1, 2, 3, &frame.image, 0).is_err());
    }

    #[test]
    fn rejects_untrusted_frame_dimensions_and_payloads() {
        for (width, height, length, encoding) in [
            (0, 10, 0, 1),
            (u32::MAX, u32::MAX, 0, 1),
            (1921, 1080, 0, 1),
            (2, 2, 15, 1),
            (2, 2, 17, 1),
            (2, 2, 16, 0),
        ] {
            let frame = Image {
                format: Some(ImageFormat {
                    width,
                    height,
                    format: encoding,
                    ..Default::default()
                }),
                image: vec![0; length].into(),
                ..Default::default()
            };
            assert!(
                validate(&frame).is_err(),
                "accepted {width}x{height}/{length}/{encoding}"
            );
        }
        assert!(validate(&Image::default()).is_err());
    }

    #[test]
    fn permits_bounded_rgba_and_full_resolution_with_protocol_headroom() {
        for (width, height) in [(720, 1280), (1080, 1920)] {
            let length = (width * height * 4) as usize;
            assert!(length + FRAME_HEADER < MAX_MESSAGE);
            let frame = Image {
                format: Some(format(width, height, false)),
                image: vec![0; length].into(),
                ..Default::default()
            };
            assert_eq!(validate(&frame).unwrap(), (width, height));
        }
    }

    #[test]
    fn tiny_jpeg_stays_binary_and_padding_is_not_part_of_the_image() {
        let frame = Image {
            format: Some(format(1, 1, false)),
            image: vec![255; 4].into(),
            ..Default::default()
        };
        let mut compressor = turbojpeg::Compressor::new().unwrap();
        let jpeg = compressor
            .compress_to_vec(turbojpeg::Image {
                pixels: frame.image.as_ref(),
                width: 1,
                pitch: 4,
                height: 1,
                format: turbojpeg::PixelFormat::RGBA,
            })
            .unwrap();
        assert!(jpeg.len() + FRAME_HEADER < MIN_BINARY_PACKET);
        let packet = frame_packet(&frame, 1, 2, 3, &jpeg, 1).unwrap();
        assert_eq!(packet.len(), MIN_BINARY_PACKET);
        let length = u32::from_le_bytes(packet[56..60].try_into().unwrap()) as usize;
        assert_eq!(&packet[FRAME_HEADER..FRAME_HEADER + length], &jpeg);
        assert!(packet[FRAME_HEADER + length..]
            .iter()
            .all(|byte| *byte == 0));
        assert!(frame_packet(&frame, 1, 2, 3, &[], 1).is_err());
        assert!(frame_packet(&frame, 1, 2, 3, &vec![0; MAX_MESSAGE], 1).is_err());
        let raw = frame_packet(&frame, 1, 2, 3, &frame.image, 0).unwrap();
        assert_eq!(&raw[FRAME_HEADER..FRAME_HEADER + 4], &[255; 4]);
        assert_eq!(&raw[52..56], &[0; 4]);
        assert!(frame_packet(&frame, 1, 2, 3, &[0; 3], 0).is_err());
        assert!(frame_packet(&frame, 1, 2, 3, &frame.image, 2).is_err());
    }
}
