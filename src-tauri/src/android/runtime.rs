use super::{
    adb,
    auth::{self, Authority},
    environment,
    process_identity::Identity,
    rpc::Connection,
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::Child,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::Instant,
};

const LOG_LIMIT: usize = 64 * 1024;
const BOOT_TIMEOUT: Duration = Duration::from_secs(120);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Stopped,
    Starting,
    Booting,
    Running,
    Stopping,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub device_id: String,
    pub generation: Option<String>,
    pub phase: Phase,
    pub process_alive: bool,
    pub serial: Option<String>,
    pub error: Option<String>,
    pub display: Option<(u32, u32)>,
}

/// Constructed only from locked native metadata; no webview can supply these paths.
#[derive(Clone)]
pub struct Launch {
    pub root: PathBuf,
    pub emulator: PathBuf,
    pub discovery: PathBuf,
    pub device_id: String,
    pub avd_name: String,
    pub gpu: String,
    pub memory: u32,
    pub cores: u32,
    pub adb_port: u16,
}

pub type DispatchGuard = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

pub fn check_guard(guard: &Option<DispatchGuard>) -> Result<(), String> {
    if let Some(check) = guard {
        check()?;
    }
    Ok(())
}

type Reply = oneshot::Sender<Result<Status, String>>;
enum Message {
    Start(Reply, Option<DispatchGuard>),
    Stop {
        guard: Option<(String, DispatchGuard)>,
        force: bool,
        reply: Reply,
    },
    Connection(oneshot::Sender<Result<Arc<Connection>, String>>),
    ObservationGuest {
        generation: String,
        guard: DispatchGuard,
        reply: oneshot::Sender<Result<adb::Guest, String>>,
    },
    Log(oneshot::Sender<String>),
    Input {
        guard: Option<DispatchGuard>,
        generation: String,
        command: super::input::Command,
        reply: oneshot::Sender<Result<super::input::Reply, String>>,
    },
    LaunchApp {
        generation: String,
        package: String,
        activity: Option<String>,
        guard: DispatchGuard,
        reply: oneshot::Sender<Result<String, String>>,
    },
    InstallApk {
        guard: Option<DispatchGuard>,
        generation: String,
        file: fs::File,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// The actor, not its callers, owns the Child. Dropping an invoke future cannot
/// lose a process, credentials or a pending Stop. An idle actor has no timer.
#[derive(Clone)]
pub struct DeviceRuntime {
    messages: mpsc::Sender<Message>,
    status: watch::Receiver<Status>,
    queued_starts: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
}

impl DeviceRuntime {
    pub async fn diagnostic_log(&self) -> Result<String, String> {
        let (reply, receive) = oneshot::channel();
        self.messages.try_send(Message::Log(reply)).map_err(|_| {
            "Android is busy; retry exporting diagnostics after the current operation."
        })?;
        tokio::time::timeout(Duration::from_secs(3), receive)
            .await
            .map_err(|_| "Android diagnostics timed out; retry after the current operation.")?
            .map_err(|_| "Android runtime closed")
            .map_err(String::from)
    }
    pub fn spawn(plan: Launch, directory: Arc<Mutex<storage::Directory>>) -> Result<Self, String> {
        plan.validate()?;
        if directory
            .lock()
            .map_err(|_| "Android directory lock failed")?
            .root
            .canonicalize()
            .map_err(|e| e.to_string())?
            != plan.root.canonicalize().map_err(|e| e.to_string())?
        {
            return Err("Android process must retain its managed directory owner".into());
        }
        let status = Status {
            device_id: plan.device_id.clone(),
            generation: None,
            phase: Phase::Stopped,
            process_alive: false,
            serial: None,
            error: None,
            display: None,
        };
        let (sender, receiver) = watch::channel(status);
        let (messages, inbox) = mpsc::channel(32);
        let queued_starts = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        tokio::spawn(
            Actor {
                plan,
                _directory: directory,
                status: sender,
                inbox,
                process: None,
                operation: None,
                cancel: cancel.clone(),
                starters: Vec::new(),
                stoppers: Vec::new(),
                stopping_deadline: None,
                queued_starts: queued_starts.clone(),
            }
            .run(),
        );
        Ok(Self {
            messages,
            status: receiver,
            queued_starts,
            cancel,
        })
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    #[cfg(test)]
    pub async fn start(&self) -> Result<Status, String> {
        self.request_start()?
            .await
            .map_err(|_| "Android start response ended")?
    }

    pub fn is_busy(&self) -> bool {
        self.queued_starts.load(Ordering::Acquire) != 0 || self.status.borrow().process_alive
    }

    #[cfg(test)]
    pub fn request_start(&self) -> Result<oneshot::Receiver<Result<Status, String>>, String> {
        self.request_start_guarded(None)
    }

    pub fn request_start_guarded(
        &self,
        guard: Option<DispatchGuard>,
    ) -> Result<oneshot::Receiver<Result<Status, String>>, String> {
        check_guard(&guard)?;
        let (send, receive) = oneshot::channel();
        self.queued_starts.fetch_add(1, Ordering::AcqRel);
        if self.messages.try_send(Message::Start(send, guard)).is_err() {
            self.queued_starts.fetch_sub(1, Ordering::AcqRel);
            return Err(
                "Android runtime is busy or unavailable. Retry after its current operation.".into(),
            );
        }
        Ok(receive)
    }

    pub async fn stop(&self, force: bool) -> Result<Status, String> {
        self.cancel.store(true, Ordering::Release);
        let (send, receive) = oneshot::channel();
        self.messages
            .send(Message::Stop {
                guard: None,
                force,
                reply: send,
            })
            .await
            .map_err(|_| "Android runtime ended")?;
        receive.await.map_err(|_| "Android stop response ended")?
    }

    pub async fn stop_guarded(
        &self,
        generation: String,
        guard: DispatchGuard,
    ) -> Result<Status, String> {
        guard()?;
        let (reply, result) = oneshot::channel();
        self.messages
            .try_send(Message::Stop {
                guard: Some((generation, guard)),
                force: false,
                reply,
            })
            .map_err(|_| "Android runtime is busy or unavailable")?;
        result.await.map_err(|_| "Android stop response ended")?
    }

    pub async fn connection(&self) -> Result<Arc<Connection>, String> {
        let (send, receive) = oneshot::channel();
        self.messages
            .send(Message::Connection(send))
            .await
            .map_err(|_| "Android runtime ended")?;
        receive
            .await
            .map_err(|_| "Android connection response ended")?
    }

    pub async fn input(
        &self,
        generation: String,
        command: super::input::Command,
    ) -> Result<super::input::Reply, String> {
        self.input_guarded(generation, command, None).await
    }

    pub async fn observation_guest(
        &self,
        generation: String,
        guard: DispatchGuard,
    ) -> Result<adb::Guest, String> {
        guard()?;
        let (reply, result) = oneshot::channel();
        self.messages
            .try_send(Message::ObservationGuest {
                generation,
                guard,
                reply,
            })
            .map_err(|_| "Android observation queue is busy")?;
        result.await.map_err(|_| "Android observation ended")?
    }

    pub async fn input_guarded(
        &self,
        generation: String,
        command: super::input::Command,
        guard: Option<DispatchGuard>,
    ) -> Result<super::input::Reply, String> {
        check_guard(&guard)?;
        let (reply, result) = oneshot::channel();
        self.messages
            .try_send(Message::Input {
                guard,
                generation,
                command,
                reply,
            })
            .map_err(|_| "Android control is busy or unavailable")?;
        result.await.map_err(|_| "Android input response ended")?
    }

    pub async fn launch_app(
        &self,
        generation: String,
        package: String,
        activity: Option<String>,
        guard: DispatchGuard,
    ) -> Result<String, String> {
        guard()?;
        let (reply, result) = oneshot::channel();
        self.messages
            .try_send(Message::LaunchApp {
                generation,
                package,
                activity,
                guard,
                reply,
            })
            .map_err(|_| "Android control queue is busy")?;
        result.await.map_err(|_| "Android launch response ended")?
    }

    pub async fn install_apk(&self, generation: String, file: fs::File) -> Result<(), String> {
        self.install_apk_guarded(generation, file, None).await
    }
    pub async fn install_apk_guarded(
        &self,
        generation: String,
        file: fs::File,
        guard: Option<DispatchGuard>,
    ) -> Result<(), String> {
        if let Some(guard) = &guard {
            guard()?;
        }
        let (reply, result) = oneshot::channel();
        self.messages
            .try_send(Message::InstallApk {
                guard,
                generation,
                file,
                reply,
            })
            .map_err(|_| "Android control is busy or unavailable")?;
        result
            .await
            .map_err(|_| "Android APK installation response ended")?
    }
}

struct Ports {
    console: u16,
    grpc: u16,
    reservations: Vec<TcpListener>,
}
impl Ports {
    fn reserve() -> Result<Self, String> {
        for console in (5554..=5682).step_by(2) {
            let Ok(a) = TcpListener::bind(("127.0.0.1", console)) else {
                continue;
            };
            let Ok(b) = TcpListener::bind(("127.0.0.1", console + 1)) else {
                continue;
            };
            let grpc = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
            return Ok(Self {
                console,
                grpc: grpc.local_addr().map_err(|e| e.to_string())?.port(),
                reservations: vec![a, b, grpc],
            });
        }
        Err("No free Android console/ADB port pair. Stop an unused emulator and retry.".into())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    version: u32,
    device_id: String,
    generation: String,
    generation_key: String,
    process: Identity,
    #[serde(default)]
    launching: bool,
    console_port: u16,
    grpc_port: u16,
    #[serde(default = "default_adb_port")]
    adb_port: u16,
}

impl Record {
    fn current_process(&self, root: &Path) -> Result<Option<Identity>, String> {
        if self.process.matches_or_exited()? {
            return Ok(Some(self.process.clone()));
        }
        if self.launching {
            if let Ok(current) = Identity::read(self.process.pid) {
                if current.created == self.process.created && current.boot == self.process.boot {
                    let engine = root
                        .join("sdk/emulator")
                        .canonicalize()
                        .map_err(|e| e.to_string())?;
                    if self.process.executable.starts_with(&engine)
                        && current.executable.starts_with(&engine)
                    {
                        // The Unix launcher execs the bundled QEMU in the same
                        // process. Creation/boot identity must survive that exec.
                        return Ok(Some(current));
                    }
                    return Err("The launching Android process changed executable outside managed tools. Its recovery record was preserved.".into());
                }
            }
        }
        Ok(None)
    }
}

fn default_adb_port() -> u16 {
    5037
}

#[cfg(any(feature = "android-probe", feature = "mcp-probe"))]
pub(super) fn fixture_guest(root: &Path, device: &str) -> Result<adb::Guest, String> {
    if !storage::valid_id(device) {
        return Err("Invalid fixture device".into());
    }
    let record = previous(&root.join("runtime").join(format!("{device}.json")), device)?
        .ok_or("Missing fixture identity")?;
    if !record.process.still_matches()? {
        return Err("The fixture process changed".into());
    }
    Ok(adb::Guest {
        server: adb::Server {
            port: super::fixture::adb_port(),
        },
        console_port: record.console_port,
        device_id: device.into(),
        generation_key: record.generation_key,
    })
}

fn previous(path: &Path, device_id: &str) -> Result<Option<Record>, String> {
    let old = storage::read::<Record>(path)?;
    if let Some(old) = &old {
        if old.version != 1
            || old.device_id != device_id
            || !storage::valid_id(&old.generation)
            || !storage::valid_id(&old.generation_key)
            || !(5554..=5682).contains(&old.console_port)
            || !old.console_port.is_multiple_of(2)
            || old.grpc_port < 1024
            || old.adb_port < 1024
        {
            return Err("Invalid Android recovery record. Preserve it and export diagnostics before recovery.".into());
        }
    }
    Ok(old)
}

pub fn recovered_statuses(
    root: &Path,
    known: &std::collections::BTreeSet<String>,
) -> Result<Vec<Status>, String> {
    let runtime = super::installation::checked_path(root, Path::new("runtime"))?;
    if !runtime.try_exists().map_err(|e| e.to_string())? {
        return Ok(vec![]);
    }
    let mut result = Vec::new();
    for (index, entry) in fs::read_dir(runtime)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if index >= 4096 {
            return Err("Android runtime directory exceeds its recovery limit".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| storage::valid_id(id) && !known.contains(*id))
        else {
            continue;
        };
        if let Some(record) = previous(&entry.path(), id)? {
            if record.current_process(root)?.is_some() {
                result.push(Status { device_id: id.into(), generation: Some(record.generation), phase: Phase::Failed, process_alive: true, serial: Some(format!("emulator-{}", record.console_port)), display: None, error: Some("Android survived a previous Lomi exit. Choose Stop to recover it safely, then Start again. Device data is preserved.".into()) });
            }
        }
    }
    Ok(result)
}

pub async fn stop_recovered(
    root: PathBuf,
    device_id: String,
    force: bool,
) -> Result<Status, String> {
    let path = super::installation::checked_path(&root, Path::new("runtime"))?
        .join(format!("{device_id}.json"));
    let stopped = || Status {
        device_id: device_id.clone(),
        generation: None,
        phase: Phase::Stopped,
        process_alive: false,
        serial: None,
        error: None,
        display: None,
    };
    let Some(record) = previous(&path, &device_id)? else {
        return Ok(stopped());
    };
    let Some(process) = record.current_process(&root)? else {
        return Ok(stopped());
    };
    let engine = root
        .join("sdk/emulator")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !process.executable.starts_with(engine) {
        return Err(
            "The recovery process is not the managed Android executable. No signal was sent."
                .into(),
        );
    }
    let discovery = super::avd::discovery(&root)?;
    let published = super::avd::read_ini(&discovery.join(format!("pid_{}.ini", process.pid)))?;
    let avd = super::installation::checked_path(
        &root,
        &PathBuf::from("avd").join(format!("sb_{device_id}.avd")),
    )?
    .canonicalize()
    .map_err(|e| e.to_string())?;
    if published.get("avd.id") != Some(&format!("sb_{device_id}"))
        || published
            .get("avd.dir")
            .map(PathBuf::from)
            .and_then(|path| path.canonicalize().ok())
            .as_ref()
            != Some(&avd)
    {
        return Err(
            "The recovered process does not own this managed AVD. No signal was sent.".into(),
        );
    }
    let connection = Connection::register(
        &process,
        &discovery,
        record.grpc_port,
        Arc::new(Authority::new(auth::new_id()?)?),
    )
    .await?;
    if force {
        let request = connection.request(
            "setVmState",
            crate::android_protocol::VmRunState {
                state: crate::android_protocol::vm_run_state::RunState::Shutdown as i32,
            },
        )?;
        connection
            .client()
            .set_vm_state(request)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        let guest = adb::Guest {
            server: adb::Server {
                port: record.adb_port,
            },
            console_port: record.console_port,
            device_id: device_id.clone(),
            generation_key: record.generation_key.clone(),
        };
        tokio::task::spawn_blocking(move || guest.request_shutdown())
            .await
            .map_err(|e| e.to_string())??;
    }
    let deadline = Instant::now() + STOP_TIMEOUT;
    while process.matches_or_exited()? {
        if Instant::now() >= deadline {
            return Err("Recovered Android has not exited. Its identity and AVD are retained; retry Stop or explicitly Force stop.".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if previous(&path, &device_id)?.is_some_and(|current| current.generation == record.generation) {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(stopped())
}

pub fn require_no_recovered_process(root: &Path) -> Result<(), String> {
    let runtime = super::installation::checked_path(root, Path::new("runtime"))?;
    if !runtime.try_exists().map_err(|e| e.to_string())? {
        return Ok(());
    }
    let mut count = 0;
    for entry in fs::read_dir(runtime).map_err(|e| e.to_string())? {
        count += 1;
        if count > 4096 {
            return Err("Android runtime directory exceeds its recovery limit".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|id| storage::valid_id(id))
        else {
            continue;
        };
        check_previous(&entry.path(), id)?;
    }
    Ok(())
}

pub(super) fn check_previous(path: &Path, device_id: &str) -> Result<(), String> {
    if let Some(old) = previous(path, device_id)? {
        let root = path
            .parent()
            .and_then(Path::parent)
            .ok_or("Invalid Android recovery path")?;
        if old.current_process(root)?.is_some() {
            return Err("A previous Lomi instance still owns an Android process. Recover or stop it before changing its files or starting another process.".into());
        }
    }
    Ok(())
}

struct Process {
    child: Child,
    ports: Ports,
    generation: String,
    guest: adb::Guest,
    connection: Option<Arc<Connection>>,
    input: super::input::State,
    claimed: Arc<AtomicBool>,
    stopping_requested: bool,
    output: Arc<Mutex<VecDeque<u8>>>,
    readers: Vec<JoinHandle<()>>,
    record: PathBuf,
    allowlist: PathBuf,
}
impl Process {
    fn spawn(plan: &Launch) -> Result<Self, String> {
        adb::Server {
            port: plan.adb_port,
        }
        .preflight()?;
        let generation = auth::new_id()?;
        let generation_key = auth::new_id()?;
        for name in [
            "runtime",
            "logs",
            "tmp",
            "user",
            "user/cli-home",
            "emulator-home",
        ] {
            fs::create_dir_all(plan.root.join(name)).map_err(|e| e.to_string())?;
        }
        let record = plan
            .root
            .join("runtime")
            .join(format!("{}.json", plan.device_id));
        check_previous(&record, &plan.device_id)?;
        let mut ports = Ports::reserve()?;
        let authority = Authority::new(generation.clone())?;
        let allowlist = plan
            .root
            .join("runtime")
            .join(format!("{}-allowlist.json", plan.device_id));
        storage::write(&allowlist, &authority.allowlist())?;
        let mut command = environment::command(
            &plan.emulator,
            &plan.root,
            &plan.root.join("sdk"),
            None,
            plan.adb_port,
        )?;
        command
            .args([
                "-avd",
                &plan.avd_name,
                "-no-window",
                "-no-audio",
                "-no-boot-anim",
                "-camera-back",
                "none",
                "-camera-front",
                "none",
                "-gpu",
                &plan.gpu,
                "-cores",
                &plan.cores.to_string(),
                "-memory",
                &plan.memory.to_string(),
                "-vsync-rate",
                "30",
                "-port",
                &ports.console.to_string(),
                "-grpc",
                &ports.grpc.to_string(),
                "-grpc-use-jwt",
                "-no-snapshot",
                "-no-metrics",
                "-crash-report-mode",
                "disabled",
                "-append-userspace-opt",
                &format!("androidboot.lomi.device={}", plan.device_id),
                "-adb-path",
            ])
            .arg(plan.root.join("runtime/no-external-adb"))
            .arg("-grpc-allowlist")
            .arg(&allowlist)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Emulator has no socket-activation API. Reserve the whole pair until spawn;
        // a cross-application bind race fails authentication/boot, never selects a peer.
        ports.reservations.clear();
        let mut child = tokio::process::Command::from(command)
            .spawn()
            .map_err(|e| format!("Cannot start Android: {e}. Repair tools in Android settings."))?;
        let output = Arc::new(Mutex::new(VecDeque::with_capacity(LOG_LIMIT)));
        let mut readers = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            readers.push(drain(stdout, output.clone()));
        }
        if let Some(stderr) = child.stderr.take() {
            readers.push(drain(stderr, output.clone()));
        }
        Ok(Self {
            child,
            guest: adb::Guest {
                server: adb::Server {
                    port: plan.adb_port,
                },
                console_port: ports.console,
                device_id: plan.device_id.clone(),
                generation_key,
            },
            ports,
            generation,
            connection: None,
            input: super::input::State::default(),
            claimed: Arc::new(AtomicBool::new(false)),
            stopping_requested: false,
            output,
            readers,
            record,
            allowlist,
        })
    }

    fn log(&self) -> String {
        self.output
            .lock()
            .map(|bytes| {
                String::from_utf8_lossy(&bytes.iter().copied().collect::<Vec<_>>()).into_owned()
            })
            .unwrap_or_default()
    }

    fn completed(&mut self, root: &Path, device: &str) {
        for reader in self.readers.drain(..) {
            reader.abort();
        }
        // One current bounded log per device. No credentials enter emulator stdout.
        if let Ok(parent) = super::installation::checked_path(root, Path::new("logs")) {
            let _ =
                storage::write_bytes(&parent.join(format!("{device}.log")), self.log().as_bytes());
        }
        let _ = super::maintenance::prune_logs(root);
        if storage::read::<Record>(&self.record)
            .ok()
            .flatten()
            .is_some_and(|record| record.generation == self.generation)
        {
            let _ = fs::remove_file(&self.record);
        }
        let _ = fs::remove_file(&self.allowlist);
    }
}

impl Launch {
    fn validate(&self) -> Result<(), String> {
        if !storage::valid_id(&self.device_id)
            || !self.root.is_absolute()
            || !self.emulator.is_absolute()
            || !self.discovery.is_absolute()
            || !self.avd_name.starts_with("sb_")
            || self.avd_name.len() > 64
            || !self
                .avd_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || !["auto", "host", "swiftshader"].contains(&self.gpu.as_str())
            || !(512..=32768).contains(&self.memory)
            || !(1..=32).contains(&self.cores)
            || self.adb_port < 1024
        {
            return Err("Invalid managed Android launch configuration".into());
        }
        let emulator = self.emulator.canonicalize().map_err(|e| e.to_string())?;
        if !emulator.starts_with(
            self.root
                .join("sdk/emulator")
                .canonicalize()
                .map_err(|e| e.to_string())?,
        ) {
            return Err("Android executable escaped managed tools".into());
        }
        let avd = self.root.join("avd").join(format!("{}.avd", self.avd_name));
        if avd
            .symlink_metadata()
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
            || avd.canonicalize().map_err(|e| e.to_string())?.parent()
                != Some(
                    self.root
                        .join("avd")
                        .canonicalize()
                        .map_err(|e| e.to_string())?
                        .as_path(),
                )
        {
            return Err("Android AVD escaped the managed directory".into());
        }
        Ok(())
    }
}

fn drain(
    mut pipe: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    output: Arc<Mutex<VecDeque<u8>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut bytes = [0; 8192];
        while let Ok(length) = pipe.read(&mut bytes).await {
            if length == 0 {
                break;
            }
            if let Ok(mut output) = output.lock() {
                let discard = output
                    .len()
                    .saturating_add(length)
                    .saturating_sub(LOG_LIMIT);
                output.drain(..discard);
                output.extend(&bytes[..length]);
            }
        }
    })
}

enum Operation {
    Ready(Result<(Arc<Connection>, (u32, u32)), String>),
    Shutdown(Result<(), String>),
}
struct Actor {
    plan: Launch,
    _directory: Arc<Mutex<storage::Directory>>,
    status: watch::Sender<Status>,
    inbox: mpsc::Receiver<Message>,
    process: Option<Process>,
    operation: Option<JoinHandle<Operation>>,
    cancel: Arc<AtomicBool>,
    starters: Vec<Reply>,
    stoppers: Vec<Reply>,
    stopping_deadline: Option<Instant>,
    queued_starts: Arc<AtomicUsize>,
}

impl Actor {
    fn publish(&self, phase: Phase, error: Option<String>) {
        self.status.send_modify(|status| {
            status.phase = phase;
            status.error = error;
            status.process_alive = self.process.is_some();
        });
    }

    fn resolve(&self, replies: Vec<Reply>, error: Option<String>) {
        for reply in replies {
            let _ = reply.send(match &error {
                Some(error) => Err(error.clone()),
                None => Ok(self.status.borrow().clone()),
            });
        }
    }

    async fn cancel_operation(&mut self) {
        if let Some(operation) = self.operation.take() {
            self.cancel.store(true, Ordering::Release);
            // spawn_blocking cannot be aborted. Wait for its bounded ADB operation
            // before changing generations, stopping the guest or releasing ownership.
            let _ = operation.await;
        }
    }

    fn start(&mut self, guard: Option<DispatchGuard>) {
        if let Err(error) = check_guard(&guard) {
            let replies = std::mem::take(&mut self.starters);
            self.resolve(replies, Some(error));
            return;
        }
        match Process::spawn(&self.plan) {
            Ok(process) => {
                self.status.send_modify(|status| {
                    status.generation = Some(process.generation.clone());
                    status.serial = Some(format!("emulator-{}", process.ports.console));
                    status.display = None;
                });
                let plan = self.plan.clone();
                let pid = process.child.id().expect("a newly spawned child has a PID");
                let port = process.ports.grpc;
                let guest = process.guest.clone();
                let record = process.record.clone();
                let status = self.status.clone();
                self.cancel.store(false, Ordering::Release);
                let cancel = self.cancel.clone();
                let claimed = process.claimed.clone();
                let generation = process.generation.clone();
                let recorded = Identity::read(pid).and_then(|identity| {
                    storage::write(
                        &record,
                        &Record {
                            version: 1,
                            device_id: plan.device_id.clone(),
                            generation: generation.clone(),
                            generation_key: guest.generation_key.clone(),
                            process: identity,
                            launching: true,
                            console_port: guest.console_port,
                            grpc_port: port,
                            adb_port: plan.adb_port,
                        },
                    )
                });
                self.process = Some(process);
                if let Err(error) = recorded {
                    self.publish(Phase::Failed, Some(error.clone()));
                    let replies = std::mem::take(&mut self.starters);
                    self.resolve(replies, Some(error));
                    return;
                }
                self.publish(Phase::Starting, None);
                self.operation = Some(tokio::spawn(async move {
                    Operation::Ready(
                        ready(Boot {
                            guard,
                            plan,
                            pid,
                            port,
                            guest,
                            record,
                            status,
                            cancel,
                            claimed,
                            generation,
                        })
                        .await,
                    )
                }));
            }
            Err(error) => {
                self.publish(Phase::Failed, Some(error.clone()));
                let replies = std::mem::take(&mut self.starters);
                self.resolve(replies, Some(error));
            }
        }
    }

    async fn stop(&mut self, force: bool) {
        self.cancel_operation().await;
        let starters = std::mem::take(&mut self.starters);
        self.resolve(starters, Some("Android start was cancelled by Stop".into()));
        self.stopping_deadline = Some(Instant::now() + STOP_TIMEOUT);
        self.publish(Phase::Stopping, None);
        if let Some(process) = &mut self.process {
            process.stopping_requested = true;
            if !force {
                if let Some(connection) = &process.connection {
                    let _ = process.input.release(connection, &process.guest).await;
                }
            }
            process.connection = None;
            if force {
                if let Err(error) = process.child.start_kill() {
                    self.failed_stop(format!("Cannot force-stop Android: {error}. Retry Stop."));
                }
            } else {
                let guest = process.guest.clone();
                let claimed = process.claimed.clone();
                self.operation = Some(tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        if !claimed.load(Ordering::Acquire) {
                            guest.claim_generation()?;
                            claimed.store(true, Ordering::Release);
                        }
                        guest.request_shutdown()
                    })
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|result| result);
                    Operation::Shutdown(result)
                }));
            }
        }
    }

    fn failed_stop(&mut self, error: String) {
        self.stopping_deadline = None;
        self.publish(Phase::Failed, Some(error.clone()));
        let stoppers = std::mem::take(&mut self.stoppers);
        self.resolve(stoppers, Some(error.clone()));
        let starters = std::mem::take(&mut self.starters);
        self.resolve(starters, Some(error));
    }

    async fn exited(&mut self, exit: ExitStatus) {
        self.cancel_operation().await;
        let expected = self
            .process
            .as_ref()
            .is_some_and(|process| process.stopping_requested);
        if let Some(mut process) = self.process.take() {
            process.completed(&self.plan.root, &self.plan.device_id);
        }
        self.stopping_deadline = None;
        self.status.send_modify(|status| status.serial = None);
        let error = if expected {
            None
        } else {
            Some(format!(
                "Android exited ({exit}). Review diagnostics or retry Start."
            ))
        };
        self.publish(
            if expected {
                Phase::Stopped
            } else {
                Phase::Failed
            },
            error.clone(),
        );
        let replies = std::mem::take(&mut self.stoppers);
        self.resolve(replies, None);
        if expected && !self.starters.is_empty() {
            self.start(None);
        } else {
            let replies = std::mem::take(&mut self.starters);
            self.resolve(replies, error);
        }
    }

    async fn run(mut self) {
        let mut abandoned = false;
        let mut next_tick = Instant::now();
        loop {
            if self.inbox.is_closed() && !abandoned {
                abandoned = true;
                if self.process.is_some() {
                    self.stop(false).await;
                }
            }
            let message = if self.process.is_some() {
                tokio::select! { message = self.inbox.recv(), if !self.inbox.is_closed() => message, _ = tokio::time::sleep_until(next_tick) => { self.tick().await; next_tick = Instant::now() + Duration::from_millis(100); continue; } }
            } else {
                self.inbox.recv().await
            };
            let Some(message) = message else {
                if self.process.is_none() {
                    break;
                }
                // Retain the directory and Child while a live process remains, even if
                // every frontend request disappeared during application teardown.
                self.stop(false).await;
                continue;
            };
            match message {
                Message::Log(reply) => {
                    let _ = reply.send(self.process.as_ref().map(Process::log).unwrap_or_default());
                }
                Message::Start(reply, guard) => {
                    let phase = self.status.borrow().phase;
                    let checked = check_guard(&guard).and_then(|_| {
                        if guard.is_some() && phase == Phase::Stopping {
                            Err("Android is stopping; wait before starting it".into())
                        } else {
                            Ok(())
                        }
                    });
                    if let Err(error) = checked {
                        self.queued_starts.fetch_sub(1, Ordering::AcqRel);
                        let _ = reply.send(Err(error));
                        continue;
                    }
                    match phase {
                        Phase::Running => {
                            let _ = reply.send(Ok(self.status.borrow().clone()));
                        }
                        Phase::Failed if self.process.is_some() => {
                            let _ = reply.send(Err("The previous Android process is still alive. Retry Stop before starting another instance.".into()));
                        }
                        phase => {
                            self.starters.retain(|reply| !reply.is_closed());
                            if self.starters.len() >= 128 {
                                self.queued_starts.fetch_sub(1, Ordering::AcqRel);
                                let _ = reply.send(Err("Too many pending Android start requests. Wait for the current operation.".into()));
                                continue;
                            }
                            self.starters.push(reply);
                            if matches!(phase, Phase::Stopped | Phase::Failed) {
                                self.start(guard);
                            }
                        }
                    }
                    self.queued_starts.fetch_sub(1, Ordering::AcqRel);
                }
                Message::Stop {
                    force,
                    reply,
                    guard,
                } => {
                    if let Some((generation, check)) = guard {
                        let checked = check().and_then(|_| {
                            if self.status.borrow().generation.as_deref() != Some(&generation) {
                                Err("This Android stop belongs to an old generation".into())
                            } else {
                                Ok(())
                            }
                        });
                        if let Err(error) = checked {
                            let _ = reply.send(Err(error));
                            continue;
                        }
                    }
                    if self.process.is_none() {
                        self.publish(Phase::Stopped, None);
                        let _ = reply.send(Ok(self.status.borrow().clone()));
                    } else {
                        self.stoppers.retain(|reply| !reply.is_closed());
                        if self.stoppers.len() >= 128 {
                            let _ = reply.send(Err("Too many pending Android stop requests. Wait for the current operation.".into()));
                            continue;
                        }
                        self.stoppers.push(reply);
                        if self.status.borrow().phase != Phase::Stopping || force {
                            self.stop(force).await;
                        }
                    }
                }
                Message::Connection(reply) => {
                    let result = self
                        .process
                        .as_ref()
                        .and_then(|process| process.connection.clone())
                        .filter(|_| self.status.borrow().phase == Phase::Running)
                        .ok_or("Android is not running. Start or reconnect the phone.".into());
                    let _ = reply.send(result);
                }
                Message::ObservationGuest {
                    generation,
                    guard,
                    reply,
                } => {
                    let result = guard().and_then(|_| {
                        self.process
                            .as_ref()
                            .filter(|process| {
                                process.generation == generation
                                    && self.status.borrow().phase == Phase::Running
                            })
                            .map(|process| process.guest.clone())
                            .ok_or_else(|| {
                                "Android observation belongs to a stopped or old generation".into()
                            })
                    });
                    let _ = reply.send(result);
                }
                Message::Input {
                    guard,
                    generation,
                    command,
                    reply,
                } => {
                    if let Err(error) = check_guard(&guard) {
                        let _ = reply.send(Err(error));
                        continue;
                    }
                    let status = self.status.borrow().clone();
                    let result = if let Some(process) = &mut self.process {
                        if process.generation == generation && status.phase == Phase::Running {
                            if let (Some(connection), Some(display)) =
                                (&process.connection, status.display)
                            {
                                process
                                    .input
                                    .apply(command, connection, &process.guest, display, guard)
                                    .await
                            } else {
                                Err("Android input is not ready. Reconnect the phone.".into())
                            }
                        } else {
                            Err("Android input belongs to an old or stopped instance.".into())
                        }
                    } else {
                        Err("Android is stopped. Start the phone before sending input.".into())
                    };
                    let _ = reply.send(result);
                }
                Message::LaunchApp {
                    generation,
                    package,
                    activity,
                    guard,
                    reply,
                } => {
                    let result = if let Some(process) = &self.process {
                        if process.generation == generation
                            && self.status.borrow().phase == Phase::Running
                            && !self.cancel.load(Ordering::Acquire)
                            && guard().is_ok()
                        {
                            let guest = process.guest.clone();
                            let cancel = self.cancel.clone();
                            tokio::task::spawn_blocking(move || {
                                guest.launch_app(&package, activity.as_deref(), cancel, guard)
                            })
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r)
                        } else {
                            Err("Android launch authority ended".into())
                        }
                    } else {
                        Err("Android is stopped".into())
                    };
                    let _ = reply.send(result);
                }
                Message::InstallApk {
                    guard,
                    generation,
                    mut file,
                    reply,
                } => {
                    let status = self.status.borrow().clone();
                    let result = if let Some(process) = &self.process {
                        if process.generation == generation
                            && status.phase == Phase::Running
                            && !self.cancel.load(Ordering::Acquire)
                            && guard.as_ref().is_none_or(|g| g().is_ok())
                        {
                            let guest = process.guest.clone();
                            let cancel = self.cancel.clone();
                            tokio::task::spawn_blocking(move || match guard {
                                Some(guard) => {
                                    guest.install_apk_guarded(&mut file, &cancel, Some(guard))
                                }
                                None => guest.install_apk(&mut file, &cancel),
                            })
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|result| result)
                        } else {
                            Err("Android instance changed during APK selection. Choose the file again.".into())
                        }
                    } else {
                        Err("Android is stopped. Start the phone before installing an APK.".into())
                    };
                    let _ = reply.send(result);
                }
            }
        }
    }

    async fn tick(&mut self) {
        if let Some(process) = &mut self.process {
            match process.child.try_wait() {
                Ok(Some(exit)) => {
                    self.exited(exit).await;
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    self.failed_stop(format!(
                        "Cannot confirm Android process exit: {error}. Retry Stop."
                    ));
                    return;
                }
            }
        }
        if self.operation.as_ref().is_some_and(JoinHandle::is_finished) {
            let result = self.operation.take().unwrap().await;
            match result {
                Ok(Operation::Ready(Ok((connection, display)))) => {
                    if let Some(process) = &mut self.process {
                        process.connection = Some(connection);
                    }
                    self.status
                        .send_modify(|status| status.display = Some(display));
                    self.publish(Phase::Running, None);
                    let replies = std::mem::take(&mut self.starters);
                    self.resolve(replies, None);
                }
                Ok(Operation::Ready(Err(error))) => {
                    self.publish(Phase::Failed, Some(error.clone()));
                    let replies = std::mem::take(&mut self.starters);
                    self.resolve(replies, Some(error));
                }
                Ok(Operation::Shutdown(Ok(()))) => {}
                Ok(Operation::Shutdown(Err(error))) => self.failed_stop(format!(
                    "{error}. The process is retained; retry Stop or explicitly force-stop it."
                )),
                Err(error) => {
                    self.failed_stop(format!("Android operation failed: {error}. Retry Stop."))
                }
            }
        }
        if self
            .stopping_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.cancel_operation().await;
            self.failed_stop("Android has not exited within 30 seconds. Its process handle is retained; retry Stop or explicitly force-stop it.".into());
        }
    }
}

struct Boot {
    guard: Option<DispatchGuard>,
    plan: Launch,
    pid: u32,
    port: u16,
    guest: adb::Guest,
    record: PathBuf,
    status: watch::Sender<Status>,
    cancel: Arc<AtomicBool>,
    claimed: Arc<AtomicBool>,
    generation: String,
}

async fn ready(boot: Boot) -> Result<(Arc<Connection>, (u32, u32)), String> {
    let Boot {
        guard,
        plan,
        pid,
        port,
        guest,
        record,
        status,
        cancel,
        claimed,
        generation,
    } = boot;
    let deadline = Instant::now() + BOOT_TIMEOUT;
    let checkpoint = || -> Result<(), String> {
        check_guard(&guard)?;
        if cancel.load(Ordering::Acquire) {
            return Err("Android start was cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err(
                "Android boot timed out. The process is retained; Stop it before retrying Start."
                    .into(),
            );
        }
        Ok(())
    };
    let identity = loop {
        checkpoint()?;
        if plan.discovery.join(format!("pid_{pid}.ini")).is_file() {
            let identity = Identity::read(pid)?;
            if !identity.executable.starts_with(
                plan.root
                    .join("sdk/emulator")
                    .canonicalize()
                    .map_err(|e| e.to_string())?,
            ) {
                return Err("Android process executable changed unexpectedly".into());
            }
            break identity;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    storage::write(
        &record,
        &Record {
            version: 1,
            device_id: plan.device_id.clone(),
            generation: generation.clone(),
            generation_key: guest.generation_key.clone(),
            process: identity.clone(),
            launching: false,
            console_port: guest.console_port,
            grpc_port: port,
            adb_port: plan.adb_port,
        },
    )?;
    let authority = Arc::new(Authority::new(generation)?);
    let mut connection = Connection::register(&identity, &plan.discovery, port, authority).await?;
    checkpoint()?;
    status.send_modify(|status| {
        status.phase = Phase::Booting;
    });
    let display = Connection::display(&connection.status().await?)?;
    loop {
        checkpoint()?;
        if !identity.still_matches()? {
            return Err("Android exited while booting".into());
        }
        if connection.status().await.is_ok_and(|status| status.booted) {
            let boot_guest = guest.clone();
            let claimed = claimed.clone();
            let result = tokio::task::spawn_blocking(move || {
                boot_guest.claim_generation()?;
                claimed.store(true, Ordering::Release);
                boot_guest.booted()
            })
            .await
            .map_err(|e| e.to_string())?;
            checkpoint()?;
            if result == Ok(true) {
                let guest = guest.clone();
                let cancel = cancel.clone();
                tokio::task::spawn_blocking(move || guest.prepare_input(&cancel))
                    .await
                    .map_err(|e| e.to_string())??;
                checkpoint()?;
                return Ok((Arc::new(connection), display));
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn actor_rechecks_queued_start_and_stop_authority_before_process_effects() {
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Mutex::new(
            storage::Directory::acquire(root.path().into()).unwrap(),
        ));
        fs::create_dir_all(root.path().join("sdk/emulator")).unwrap();
        fs::create_dir_all(root.path().join("avd/sb_fixture.avd")).unwrap();
        let emulator = root.path().join("sdk/emulator/emulator");
        fs::write(&emulator, "unused").unwrap();
        let runtime = DeviceRuntime::spawn(
            Launch {
                root: root.path().into(),
                emulator,
                discovery: root.path().join("discovery"),
                device_id: "00000000-0000-0000-0000-000000000001".into(),
                avd_name: "sb_fixture".into(),
                gpu: "host".into(),
                memory: 2048,
                cores: 2,
                adb_port: 15047,
            },
            directory,
        )
        .unwrap();
        let active = Arc::new(AtomicBool::new(true));
        let flag = active.clone();
        let guard: DispatchGuard = Arc::new(move || {
            if flag.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err("revoked fixture dispatch".into())
            }
        });
        let queued = runtime.request_start_guarded(Some(guard.clone())).unwrap();
        active.store(false, Ordering::SeqCst);
        assert!(queued
            .await
            .unwrap()
            .unwrap_err()
            .contains("revoked fixture"));
        assert!(!runtime.status().process_alive);
        assert_eq!(runtime.status().phase, Phase::Stopped);
        assert_eq!(runtime.queued_starts.load(Ordering::Acquire), 0);
        active.store(true, Ordering::SeqCst);
        let stop = runtime
            .stop_guarded("00000000-0000-0000-0000-000000000002".into(), guard)
            .await;
        assert!(stop.unwrap_err().contains("old generation"));
        assert_eq!(runtime.status().phase, Phase::Stopped);
    }

    #[tokio::test]
    async fn recovery_never_signals_a_live_process_outside_the_owned_emulator_tree() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("runtime")).unwrap();
        fs::create_dir_all(root.path().join("sdk/emulator")).unwrap();
        let id = "00000000-0000-0000-0000-000000000001";
        let path = root.path().join("runtime").join(format!("{id}.json"));
        let process = Identity::read(std::process::id()).unwrap();
        storage::write(
            &path,
            &Record {
                version: 1,
                device_id: id.into(),
                generation: auth::new_id().unwrap(),
                generation_key: auth::new_id().unwrap(),
                process: process.clone(),
                launching: false,
                console_port: 5588,
                grpc_port: 19000,
                adb_port: 15047,
            },
        )
        .unwrap();
        let original = fs::read(&path).unwrap();
        assert!(check_previous(&path, id).is_err());
        for force in [false, true] {
            assert!(stop_recovered(root.path().into(), id.into(), force)
                .await
                .unwrap_err()
                .contains("not the managed"));
            assert!(process.still_matches().unwrap());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Starts a real emulator using the explicitly accepted isolated native trial"]
    fn native_runtime_coalesces_starts_retains_failed_stop_and_preserves_data() {
        let root = PathBuf::from(std::env::var("LOMI_ANDROID_PROBE_DIRECTORY").unwrap())
            .canonicalize()
            .unwrap();
        let consent: serde_json::Value = storage::read(&root.join("evidence/consent.json"))
            .unwrap()
            .unwrap();
        assert_eq!(consent["accepted"], true);
        assert!(root
            .to_string_lossy()
            .starts_with("/private/tmp/lomi-android-stage0-"));
        let directory = Arc::new(Mutex::new(
            storage::Directory::acquire(root.clone()).unwrap(),
        ));
        let plan = Launch {
            root: root.clone(),
            emulator: root.join("sdk/emulator/emulator"),
            discovery: crate::shell::home().join("Library/Caches/TemporaryItems/avd/running"),
            device_id: "00000000-0000-0000-0000-000000000001".into(),
            avd_name: "sb_stage0_small".into(),
            gpu: "host".into(),
            memory: 2048,
            cores: 2,
            adb_port: 15037,
        };
        struct PrivateAdb(Option<std::process::Child>);
        impl PrivateAdb {
            fn stop(&mut self) -> Result<(), String> {
                if let Some(mut child) = self.0.take() {
                    child.kill().map_err(|e| e.to_string())?;
                    child.wait().map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            fn start(&mut self, plan: &Launch) -> Result<(), String> {
                let reservation =
                    TcpListener::bind(("127.0.0.1", plan.adb_port)).map_err(|e| e.to_string())?;
                let mut command = environment::command(
                    &plan.root.join("sdk/platform-tools/adb"),
                    &plan.root,
                    &plan.root.join("sdk"),
                    None,
                    plan.adb_port,
                )?;
                command
                    .args(["-L", "tcp:15037", "server", "nodaemon"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                drop(reservation);
                self.0 = Some(command.spawn().map_err(|e| e.to_string())?);
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    if let Some(exit) = self
                        .0
                        .as_mut()
                        .unwrap()
                        .try_wait()
                        .map_err(|e| e.to_string())?
                    {
                        return Err(format!("Private ADB exited during startup: {exit}"));
                    }
                    if (adb::Server {
                        port: plan.adb_port,
                    })
                    .preflight()
                    .is_ok()
                    {
                        return Ok(());
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err("Private ADB startup timed out".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        impl Drop for PrivateAdb {
            fn drop(&mut self) {
                let _ = self.stop();
            }
        }
        let mut server = PrivateAdb(None);
        server.start(&plan).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let runtime = DeviceRuntime::spawn(plan.clone(), directory).unwrap();
            let mut changes = runtime.subscribe();
            let result: Result<serde_json::Value, String> = async {
                let started = Instant::now();
                let (a, b) = tokio::join!(runtime.start(), runtime.start());
                let (a, b) = (a?, b?);
                if a.phase != Phase::Running || a.generation != b.generation || a.serial != b.serial || a.display != Some((720, 1280)) { return Err("Concurrent native starts did not join the same running instance".into()); }
                changes.changed().await.map_err(|e| e.to_string())?;
                let boot_seconds = started.elapsed().as_secs_f64();
                let record_path = root.join("runtime").join(format!("{}.json", plan.device_id));
                let first: Record = storage::read(&record_path)?.ok_or("Missing runtime identity")?;
                if !first.process.still_matches()? { return Err("Native child identity changed".into()); }
                let connection = runtime.connection().await?;
                connection.client().get_status(connection.request("getStatus", ())?).await.map_err(|e| e.to_string())?;
                let guest = adb::Guest { server: adb::Server { port: plan.adb_port }, console_port: first.console_port, device_id: plan.device_id.clone(), generation_key: first.generation_key.clone() };
                let marker = auth::new_id()?;
                if guest.native_runtime_marker(Some(&marker))? != marker { return Err("Guest marker write failed".into()); }
                server.stop()?;
                if runtime.stop(false).await.is_ok() || !runtime.status().process_alive || runtime.status().phase != Phase::Failed { return Err("Failed Stop lost the living process".into()); }
                if runtime.start().await.is_ok() || !first.process.still_matches()? { return Err("Start duplicated a living failed instance".into()); }
                server.start(&plan)?;
                let deadline = Instant::now() + Duration::from_secs(20);
                while guest.booted() != Ok(true) {
                    if Instant::now() >= deadline { return Err("Private ADB reconnect failed".into()); }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                let stopped = runtime.stop(false).await?;
                if stopped.process_alive || stopped.phase != Phase::Stopped { return Err("Stop completed without reaping the process".into()); }
                let restarted = runtime.start().await?;
                let second: Record = storage::read(&record_path)?.ok_or("Missing restarted identity")?;
                if restarted.generation == a.generation || second.process == first.process { return Err("Restart reused the old live instance identity".into()); }
                let guest = adb::Guest { server: adb::Server { port: plan.adb_port }, console_port: second.console_port, device_id: plan.device_id.clone(), generation_key: second.generation_key.clone() };
                if guest.native_runtime_marker(None)? != marker { return Err("Native shutdown lost guest data".into()); }
                runtime.stop(false).await?;
                Ok(serde_json::json!({"coalescedStarts":true,"failedStopRetainsProcess":true,"startBlockedUntilExit":true,"restartChangesGeneration":true,"guestDataRetained":true,"bootSeconds":boot_seconds,"firstPid":first.process.pid,"secondPid":second.process.pid,"finalStatus":runtime.status()}))
            }.await;
            if runtime.status().process_alive { let _ = runtime.stop(true).await; }
            match result {
                Ok(report) => { storage::write(&root.join("evidence/native-runtime-lifecycle.json"), &report).unwrap(); println!("{report}"); },
                Err(error) => panic!("Native Android lifecycle failed: {error}"),
            }
        });
    }
}
