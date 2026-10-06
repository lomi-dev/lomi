//! Host-pinned native execution boundary.
//! Native execution is held until its kernel resource coalition is durable.
//! A sealed, previously validated coalition's ESRCH is the only retirement
//! receipt. Stream EOF, task counters and PID-list emptiness are not receipts.

use super::process_identity::Identity;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
mod crypto;
mod kernel;
mod pty;
mod registry;
pub(crate) use registry::{registered_scope, EffectGuard, EffectScope};
mod sandbox;
pub(crate) use kernel::Member;
pub(crate) use sandbox::Policy;
#[cfg(test)]
mod tests;
const MAX_RECORD: u64 = 65536;
const MAX_SPEC: usize = 2 * 1024 * 1024;
const WAIT: Duration = Duration::from_secs(30);
const WORKER_FLAG: &str = "--agent-runtime-host-worker";
const NATIVE_FLAG: &str = "--agent-runtime-host-native-child";
fn failure() -> String {
    "The native host boundary cannot prove ownership or retirement.".into()
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Purpose {
    Attempt,
    VersionProbe,
    Resolver,
    Collector,
    AccountTerminal,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Scope {
    pub operation_id: String,
    #[serde(default)]
    pub parent_operation_id: Option<String>,
    #[serde(default)]
    pub physical_account_root: Option<PathBuf>,
    #[serde(default)]
    pub storage_root: Option<PathBuf>,
    pub account_id: String,
    pub auth_revision: u64,
    pub task_id: Option<String>,
    pub attempt_id: Option<String>,
    pub generation: Option<u64>,
    pub project_root: PathBuf,
    pub purpose: Purpose,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NativeSpec {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub policy: sandbox::Policy,
    #[serde(default)]
    pub io: NativeIo,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub(crate) enum NativeIo {
    #[default]
    Pipes,
    Pty {
        rows: u16,
        cols: u16,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Intent,
    Held,
    ReleaseIntent,
    Released,
    Sealed,
    Retired,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    schema: u32,
    scope: Scope,
    host: kernel::HostWitness,
    label: String,
    domain: String,
    worker: PathBuf,
    worker_digest: String,
    spec_digest: String,
    phase: Phase,
    held: Option<kernel::Member>,
    seal_confirmed: bool,
    #[serde(default)]
    native_exit: Option<i32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Held {
    schema: u32,
    operation_id: String,
    spec_digest: String,
    member: kernel::Member,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Release {
    operation_id: String,
    spec_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RetiredReceipt {
    pub operation_id: String,
    pub boot: String,
    pub resource_coalition: u64,
    pub sealed_job: String,
}
pub(crate) struct Transport {
    pub stdin: fs::File,
    pub stdout: fs::File,
    pub stderr: fs::File,
}
pub(crate) struct Boundary {
    root: PathBuf,
    record: Record,
    _lock: fs::File,
    transport: Option<Transport>,
    effects: std::sync::Arc<EffectScope>,
    resize_lock: std::sync::Mutex<()>,
}
fn private_directory(path: &Path) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|_| failure())?;
    if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
        return Err(failure());
    }
    Ok(())
}
fn private_file(path: &Path) -> Result<fs::File, String> {
    let f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| failure())?;
    let m = f.metadata().map_err(|_| failure())?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.nlink() != 1
        || m.len() > MAX_RECORD
    {
        return Err(failure());
    }
    Ok(f)
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let mut bytes = Vec::new();
    private_file(path)?
        .take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() > MAX_RECORD as usize {
        return Err(failure());
    }
    serde_json::from_slice(&bytes).map_err(|_| failure())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| failure())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|_| failure())?;
    fs::File::open(path.parent().ok_or_else(failure)?)
        .and_then(|f| f.sync_all())
        .map_err(|_| failure())
}
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| failure())?;
    if bytes.len() > MAX_RECORD as usize {
        return Err(failure());
    }
    write_new(path, &bytes)
}
fn update(root: &Path, record: &Record) -> Result<(), String> {
    let path = root.join(format!("record-{}.tmp", super::new_id()?));
    write_json(&path, record)?;
    fs::rename(path, root.join("record.json")).map_err(|_| failure())?;
    fs::File::open(root)
        .and_then(|f| f.sync_all())
        .map_err(|_| failure())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn binary_digest(path: &Path) -> Result<String, String> {
    let admitted = super::process_owner::executable(path)?;
    let mut f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&admitted.0)
        .map_err(|_| failure())?;
    if f.metadata().map_err(|_| failure())?.len() > 1024 * 1024 * 1024 {
        return Err(failure());
    }
    let mut h = crypto::Hasher::new()?;
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b).map_err(|_| failure())?;
        if n == 0 {
            break;
        }
        h.update(&b[..n])?;
    }
    if super::process_owner::executable(path)? != admitted {
        return Err(failure());
    }
    h.finish()
}
pub(crate) fn staged_worker(storage_root: &Path) -> Result<PathBuf, String> {
    private_directory(storage_root)?;
    stage_worker()
}
fn stage_worker() -> Result<PathBuf, String> {
    let source = std::env::current_exe()
        .map_err(|_| failure())?
        .canonicalize()
        .map_err(|_| failure())?;
    let expected = binary_digest(&source)?;
    let base = PathBuf::from(format!("/private/tmp/lomi-host-workers-{}", unsafe {
        libc::geteuid()
    }));
    match fs::DirBuilder::new().mode(0o700).create(&base) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(failure()),
    }
    private_directory(&base)?;
    let published = base.join(&expected);
    let worker = published.join("worker");
    let verify = || -> Result<(), String> {
        private_directory(&published)?;
        let metadata = fs::symlink_metadata(&worker).map_err(|_| failure())?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o277 != 0
            || metadata.nlink() != 1
            || binary_digest(&worker)? != expected
        {
            return Err(failure());
        }
        Ok(())
    };
    if published.try_exists().map_err(|_| failure())? {
        verify()?;
        return Ok(worker);
    }
    let staging = base.join(format!("staging-{}", super::new_id()?));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&staging)
        .map_err(|_| failure())?;
    let target_path = staging.join("worker");
    let mut source_file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&source)
        .map_err(|_| failure())?;
    let mut target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o500)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&target_path)
        .map_err(|_| failure())?;
    std::io::copy(&mut source_file, &mut target).map_err(|_| failure())?;
    target.sync_all().map_err(|_| failure())?;
    fs::File::open(&staging)
        .and_then(|f| f.sync_all())
        .map_err(|_| failure())?;
    if binary_digest(&source)? != expected || binary_digest(&target_path)? != expected {
        return Err(failure());
    }
    match fs::rename(&staging, &published) {
        Ok(()) => {}
        Err(_) if published.try_exists().map_err(|_| failure())? => {
            verify()?;
            fs::remove_dir_all(&staging).map_err(|_| failure())?;
        }
        Err(_) => return Err(failure()),
    }
    fs::File::open(&base)
        .and_then(|f| f.sync_all())
        .map_err(|_| failure())?;
    verify()?;
    Ok(worker)
}
fn fifo(root: &Path, name: &str, read: bool, write: bool) -> Result<fs::File, String> {
    use std::os::unix::ffi::OsStrExt;
    let path = root.join(name);
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| failure())?;
    if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 {
        return Err(failure());
    }
    open_fifo(&path, read, write)
}
fn open_fifo(path: &Path, read: bool, write: bool) -> Result<fs::File, String> {
    let f = fs::OpenOptions::new()
        .read(read)
        .write(write)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| failure())?;
    let m = f.metadata().map_err(|_| failure())?;
    if m.mode() & u32::from(libc::S_IFMT) != u32::from(libc::S_IFIFO)
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
    {
        return Err(failure());
    }
    Ok(f)
}

fn native_stdio(file: &fs::File) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) } != 0 {
        return Err(failure());
    }
    Ok(())
}

fn write_stream(file: &mut fs::File, bytes: &[u8]) -> Result<(), String> {
    let deadline = Instant::now() + WAIT;
    let mut at = 0;
    while at < bytes.len() {
        match file.write(&bytes[at..]) {
            Ok(0) => return Err(failure()),
            Ok(n) => at += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err(failure()),
        }
        if Instant::now() > deadline {
            return Err(failure());
        }
    }
    Ok(())
}
fn read_stream(file: &mut fs::File, len: usize) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + WAIT;
    let mut bytes = vec![0; len];
    let mut at = 0;
    while at < len {
        match file.read(&mut bytes[at..]) {
            Ok(0) => std::thread::sleep(Duration::from_millis(5)),
            Ok(n) => at += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err(failure()),
        }
        if Instant::now() > deadline {
            return Err(failure());
        }
    }
    Ok(bytes)
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn launchctl(args: &[&str]) -> Result<i32, String> {
    use std::process::{Command, Stdio};
    let child = Command::new("/bin/launchctl")
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| failure())?;
    let mut child = super::process_owner::OwnedChild::new(child);
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.try_wait().map_err(|_| failure())? {
            return Ok(status.code().unwrap_or(-1));
        }
        if Instant::now() > deadline {
            let _ = child.stop_and_wait();
            return Err(failure());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn job_status(job: &str) -> Result<String, String> {
    use std::os::fd::AsRawFd;
    let child = std::process::Command::new("/bin/launchctl")
        .args(["print", job])
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| failure())?;
    let mut child = super::process_owner::OwnedChild::new(child);
    let mut output = child.stdout.take().ok_or_else(failure)?;
    let flags = unsafe { libc::fcntl(output.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(output.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(failure());
    }
    let deadline = Instant::now() + WAIT;
    let mut bytes = Vec::new();
    let mut status = None;
    loop {
        let mut chunk = [0; 4096];
        match output.read(&mut chunk) {
            Ok(0) if status.is_some() => break,
            Ok(n) => {
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.len() > MAX_RECORD as usize {
                    let _ = child.stop_and_wait();
                    return Err(failure());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return Err(failure()),
        }
        if status.is_none() {
            status = child.try_wait().map_err(|_| failure())?;
        }
        if Instant::now() >= deadline {
            let _ = child.stop_and_wait();
            return Err(failure());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !status.ok_or_else(failure)?.success() {
        return Err(failure());
    }
    String::from_utf8(bytes).map_err(|_| failure())
}
fn parse_job_exit(text: &str) -> Result<Option<i32>, String> {
    let field = |name: &str| -> Result<Option<&str>, String> {
        let prefix = format!("{name} = ");
        let mut values = text
            .lines()
            .filter(|line| !line.starts_with("\t\t"))
            .filter_map(|line| line.trim().strip_prefix(&prefix));
        let value = values.next();
        if values.next().is_some() {
            return Err(failure());
        }
        Ok(value)
    };
    if field("state")? == Some("running") {
        return Ok(None);
    }
    if field("runs")? != Some("1") {
        return Ok(None);
    }
    match field("last exit code")? {
        Some(value) => match value.parse::<i32>() {
            Ok(code) if (0..=255).contains(&code) => Ok(Some(code)),
            _ => Err(failure()),
        },
        None => Ok(None),
    }
}
impl Boundary {
    pub(crate) fn create(base: &Path, scope: Scope, spec: NativeSpec) -> Result<Self, String> {
        private_directory(base)?;
        kernel::HostWitness::admitted()?;
        if scope.operation_id.len() != 32
            || !scope.operation_id.bytes().all(|b| b.is_ascii_hexdigit())
            || scope.account_id.is_empty()
            || scope.account_id.len() > 200
            || scope.account_id.chars().any(char::is_control)
            || scope.auth_revision == 0
        {
            return Err(failure());
        }
        spec.validate()
            .map_err(|error| format!("Native specification validation failed: {error}"))?;
        if scope.parent_operation_id.as_ref().is_some_and(|parent| {
            parent.is_empty()
                || parent == &scope.operation_id
                || parent.len() > 200
                || parent.chars().any(char::is_control)
        }) || scope
            .physical_account_root
            .as_ref()
            .is_some_and(|root| root != &spec.policy.account_root)
            || scope.storage_root.as_ref().is_some_and(|root| {
                let Some(parent) = scope.parent_operation_id.as_deref() else {
                    return true;
                };
                base != root
                    .join("host-boundaries/parents")
                    .join(digest(parent.as_bytes()))
            })
            || scope.project_root != spec.policy.project_root
        {
            return Err(failure());
        }
        let root = base.join(&scope.operation_id);
        fs::create_dir(&root).map_err(|_| failure())?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|_| failure())?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("owner.lock"))
            .map_err(|_| failure())?;
        if unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&lock),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        } != 0
        {
            return Err(failure());
        }
        let worker = stage_worker()?;
        let record = Record {
            schema: 1,
            scope,
            host: kernel::HostWitness::admitted()?,
            label: String::new(),
            domain: format!("gui/{}", unsafe { libc::getuid() }),
            worker_digest: binary_digest(&worker)?,
            worker,
            spec_digest: digest(&serde_json::to_vec(&spec).map_err(|_| failure())?),
            phase: Phase::Intent,
            held: None,
            seal_confirmed: false,
            native_exit: None,
        };
        let mut result = Self {
            root,
            record,
            _lock: lock,
            transport: None,
            effects: EffectScope::new(),
            resize_lock: std::sync::Mutex::new(()),
        };
        result.record.label = format!("dev.lomi.agent.host.{}", result.record.scope.operation_id);
        write_json(&result.root.join("record.json"), &result.record)?;
        let stdin = fifo(&result.root, "stdin", true, true)?;
        let stdout = fifo(&result.root, "stdout", true, false)?;
        let stderr = fifo(&result.root, "stderr", true, false)?;
        let mut config = fifo(&result.root, "config", true, true)?;
        if matches!(spec.io, NativeIo::Pty { .. }) {
            drop(fifo(&result.root, "resize", true, true)?);
            drop(fifo(&result.root, "resize-ack", true, true)?);
        }
        result.transport = Some(Transport {
            stdin,
            stdout,
            stderr,
        });
        #[allow(unused_mut)]
        let mut args = vec![
            result.record.worker.to_string_lossy().into_owned(),
            WORKER_FLAG.into(),
            result.root.to_string_lossy().into_owned(),
        ];
        #[cfg(test)]
        {
            args = vec![
                result.record.worker.to_string_lossy().into_owned(),
                "--exact".into(),
                "agent_runtime::host_boundary::tests::compiled_worker_entry".into(),
                "--nocapture".into(),
            ];
        }
        let mut plist=format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>RunAtLoad</key><true/><key>KeepAlive</key><false/><key>ProcessType</key><string>Background</string><key>ProgramArguments</key><array>",xml(&result.record.label));
        for arg in args {
            plist.push_str(&format!("<string>{}</string>", xml(&arg)));
        }
        plist.push_str("</array>");
        #[cfg(test)] plist.push_str(&format!("<key>EnvironmentVariables</key><dict><key>LOMI_HOST_BOUNDARY_TEST_ROOT</key><string>{}</string></dict>",xml(&result.root.to_string_lossy())));
        plist.push_str("</dict></plist>");
        write_new(&result.root.join("job.plist"), plist.as_bytes())?;
        if launchctl(&[
            "bootstrap",
            &result.record.domain,
            result.root.join("job.plist").to_str().ok_or_else(failure)?,
        ])? != 0
        {
            return Err(failure());
        }
        let mut bytes = serde_json::to_vec(&spec).map_err(|_| failure())?;
        if bytes.len() > MAX_SPEC {
            return Err(failure());
        }
        write_stream(&mut config, &(bytes.len() as u32).to_le_bytes())?;
        let sent = write_stream(&mut config, &bytes);
        bytes.fill(0);
        sent?;
        let deadline = Instant::now() + WAIT;
        let held: Held = loop {
            match read_json(&result.root.join("held.json")) {
                Ok(held) => break held,
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => {
                    #[cfg(test)]
                    eprintln!(
                        "worker stages: {:?}",
                        std::fs::read_dir(&result.root)
                            .unwrap()
                            .filter_map(Result::ok)
                            .map(|e| e.file_name())
                            .collect::<Vec<_>>()
                    );
                    return Err(failure());
                }
            }
        };
        if held.schema != 1
            || held.operation_id != result.record.scope.operation_id
            || held.spec_digest != result.record.spec_digest
        {
            return Err(failure());
        }
        let observed = kernel::member(held.member.identity.pid)?;
        let own = kernel::member(std::process::id())?;
        if observed != held.member
            || observed.identity.executable != result.record.worker
            || observed.resource_coalition <= 1
            || observed.resource_coalition == own.resource_coalition
            || kernel::usage(observed.resource_coalition)? != kernel::Usage::Alive
        {
            return Err(failure());
        }
        result.record.held = Some(observed);
        result.record.phase = Phase::Held;
        update(&result.root, &result.record)?;
        Ok(result)
    }
    #[cfg(test)]
    pub(crate) fn release(&mut self) -> Result<(), String> {
        self.release_with_gate(|_| Ok(()))
    }
    pub(crate) fn release_with_gate(
        &mut self,
        before_release: impl FnOnce(&Boundary) -> Result<(), String>,
    ) -> Result<(), String> {
        self.ensure_boot()?;
        if self.record.phase != Phase::Held {
            return Err(failure());
        }
        let held = self.record.held.as_ref().ok_or_else(failure)?;
        if kernel::member(held.identity.pid)? != *held
            || binary_digest(&self.record.worker)? != self.record.worker_digest
        {
            return Err(failure());
        }
        before_release(self)?;
        self.record.phase = Phase::ReleaseIntent;
        update(&self.root, &self.record)?;
        self.record.phase = Phase::Released;
        update(&self.root, &self.record)?;
        registry::register(
            &self.record.scope,
            self.record.held.as_ref().ok_or_else(failure)?,
            &self.effects,
        )?;
        self.effects.release()?;
        // Native admission is the last publication, after durable ownership and
        // host-effect registration. A failed publication stays recovery-held.
        write_json(
            &self.root.join("release.json"),
            &Release {
                operation_id: self.record.scope.operation_id.clone(),
                spec_digest: self.record.spec_digest.clone(),
            },
        )
    }
    fn ensure_boot(&self) -> Result<(), String> {
        if self.record.host != kernel::HostWitness::admitted()? {
            return Err(failure());
        }
        Ok(())
    }
    pub(crate) fn take_transport(&mut self) -> Result<Transport, String> {
        self.transport.take().ok_or_else(failure)
    }
    pub(crate) fn stop(&mut self) -> Result<RetiredReceipt, String> {
        self.retire_using(kernel::enumerate)
    }
    fn retire_using(
        &mut self,
        discover: impl Fn() -> Result<Vec<u32>, String>,
    ) -> Result<RetiredReceipt, String> {
        self.ensure_boot()?;
        self.effects.cancel();
        self.recover_held()?;
        let cid = self
            .record
            .held
            .as_ref()
            .ok_or_else(failure)?
            .resource_coalition;
        if cid <= 1 {
            return Err(failure());
        }
        let job = format!("{}/{}", self.record.domain, self.record.label);
        if !self.record.seal_confirmed {
            let code = launchctl(&["bootout", &job])?;
            // A prior successful durable bootout can be retried. Never infer a
            // first seal from unknown launchctl failures or an absent PID.
            if code != 0 {
                return Err(failure());
            }
            self.record.seal_confirmed = true;
            self.record.phase = Phase::Sealed;
            update(&self.root, &self.record)?;
        }
        let deadline = Instant::now() + WAIT;
        loop {
            match kernel::usage(cid)? {
                kernel::Usage::Retired => {
                    self.effects.cancel_and_wait(WAIT)?;
                    self.record.phase = Phase::Retired;
                    update(&self.root, &self.record)?;
                    self.effects.release_resources()?;
                    return Ok(RetiredReceipt {
                        operation_id: self.record.scope.operation_id.clone(),
                        boot: self.record.host.boot.clone(),
                        resource_coalition: cid,
                        sealed_job: job,
                    });
                }
                kernel::Usage::Alive => {}
            }
            for pid in discover()? {
                let member = match kernel::member(pid) {
                    Ok(member) => member,
                    Err(_) if kernel::gone(pid) => continue,
                    Err(error) => return Err(error),
                };
                if member.resource_coalition == cid {
                    kernel::signal_member(&member, cid)?;
                }
            }
            if Instant::now() > deadline {
                return Err(failure());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pub(crate) fn restore(root: &Path) -> Result<Self, String> {
        private_directory(root)?;
        let record: Record = read_json(&root.join("record.json"))?;
        if record.schema != 1
            || record.host != kernel::HostWitness::admitted()?
            || root.file_name().and_then(|s| s.to_str()) != Some(record.scope.operation_id.as_str())
            || record.label != format!("dev.lomi.agent.host.{}", record.scope.operation_id)
            || record.domain != format!("gui/{}", unsafe { libc::getuid() })
        {
            return Err(failure());
        }
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join("owner.lock"))
            .map_err(|_| failure())?;
        let m = lock.metadata().map_err(|_| failure())?;
        if !m.is_file()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o077 != 0
            || m.nlink() != 1
            || unsafe {
                libc::flock(
                    std::os::fd::AsRawFd::as_raw_fd(&lock),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            } != 0
        {
            return Err(failure());
        }
        let effects = registry::retained_effects(&record.scope)?.unwrap_or_else(EffectScope::new);
        Ok(Self {
            root: root.into(),
            record,
            _lock: lock,
            transport: None,
            effects,
            resize_lock: std::sync::Mutex::new(()),
        })
    }
    pub(crate) fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        self.ensure_boot()?;
        if rows == 0 || cols == 0 || self.record.phase != Phase::Released {
            return Err(failure());
        }
        let _lock = self.resize_lock.lock().map_err(|_| failure())?;
        let mut ack = open_fifo(&self.root.join("resize-ack"), true, true)?;
        let mut channel = open_fifo(&self.root.join("resize"), false, true)?;
        let bytes = [rows.to_le_bytes(), cols.to_le_bytes()].concat();
        write_stream(&mut channel, &bytes)?;
        if read_stream(&mut ack, 4)? != bytes {
            return Err(failure());
        }
        Ok(())
    }
    pub(crate) fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        use std::os::unix::process::ExitStatusExt;
        self.ensure_boot()?;
        if let Some(code) = self.record.native_exit {
            return Ok(Some(std::process::ExitStatus::from_raw(code << 8)));
        }
        if self.record.phase != Phase::Released {
            return Err(failure());
        }
        let held = self.record.held.as_ref().ok_or_else(failure)?;
        match kernel::member(held.identity.pid) {
            Ok(current) if current == *held => return Ok(None),
            Ok(_) => {}
            Err(_) => {}
        }
        let job = format!("{}/{}", self.record.domain, self.record.label);
        let text = job_status(&job)?;
        #[cfg(test)]
        if parse_job_exit(&text).is_err() {
            eprintln!("Owned job status parse failed: {text}");
        }
        if let Some(code) = parse_job_exit(&text)? {
            self.record.native_exit = Some(code);
            update(&self.root, &self.record)?;
            return Ok(Some(std::process::ExitStatus::from_raw(code << 8)));
        }
        Ok(None)
    }
    pub(crate) fn retain_retirement_resource<T: Send + Sync + 'static>(
        &self,
        resource: std::sync::Arc<T>,
    ) -> Result<(), String> {
        self.ensure_boot()?;
        if self.record.phase != Phase::Held {
            return Err(failure());
        }
        let held = self.record.held.as_ref().ok_or_else(failure)?;
        if kernel::member(held.identity.pid)? != *held {
            return Err(failure());
        }
        // Register while admission is still closed so a failed release can
        // restore the same scope and retained resources after its adapter drops.
        registry::register(&self.record.scope, held, &self.effects)?;
        self.effects.retain_resource(resource)
    }
    pub(crate) fn effects(&self) -> std::sync::Arc<EffectScope> {
        self.effects.clone()
    }
    pub(crate) fn identity(&self) -> Result<&Identity, String> {
        Ok(&self.record.held.as_ref().ok_or_else(failure)?.identity)
    }
    pub(crate) fn signal(&self, signal: i32) -> Result<(), String> {
        self.ensure_boot()?;
        let expected = self.record.held.as_ref().ok_or_else(failure)?;
        if ![libc::SIGKILL, libc::SIGTERM, libc::SIGINT].contains(&signal) {
            return Err(failure());
        }
        for pid in kernel::enumerate()? {
            if pid == expected.identity.pid {
                continue;
            }
            let member = match kernel::member(pid) {
                Ok(member) => member,
                Err(_) if kernel::gone(pid) => continue,
                Err(error) => return Err(error),
            };
            if member.resource_coalition == expected.resource_coalition {
                kernel::signal_member_with(&member, expected.resource_coalition, signal)?;
            }
        }
        Ok(())
    }
    fn recover_held(&mut self) -> Result<(), String> {
        if self.record.held.is_some() {
            return Ok(());
        }
        let held: Held = read_json(&self.root.join("held.json"))?;
        if held.schema != 1
            || held.operation_id != self.record.scope.operation_id
            || held.spec_digest != self.record.spec_digest
        {
            return Err(failure());
        }
        let observed = kernel::member(held.member.identity.pid)?;
        let own = kernel::member(std::process::id())?;
        if observed != held.member
            || observed.resource_coalition == own.resource_coalition
            || observed.identity.executable != self.record.worker
            || binary_digest(&self.record.worker)? != self.record.worker_digest
        {
            return Err(failure());
        }
        self.record.held = Some(observed);
        self.record.phase = Phase::Held;
        update(&self.root, &self.record)
    }
}
pub(crate) fn prepare_parent(storage: &Path, parent: &str) -> Result<PathBuf, String> {
    private_directory(storage)?;
    if parent.is_empty() || parent.len() > 200 || parent.chars().any(char::is_control) {
        return Err(failure());
    }
    let base = storage.join("host-boundaries");
    let parents = base.join("parents");
    let indexed = parents.join(digest(parent.as_bytes()));
    for path in [&base, &parents, &indexed] {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(failure()),
        }
        private_directory(path)?;
    }
    Ok(indexed)
}
fn parent_records(storage: &Path, parent: &str) -> Result<Vec<PathBuf>, String> {
    private_directory(storage)?;
    let base = storage.join("host-boundaries");
    if !base.try_exists().map_err(|_| failure())? {
        return Ok(Vec::new());
    }
    private_directory(&base)?;
    let parents = base.join("parents");
    let indexed = parents.join(digest(parent.as_bytes()));
    let mut roots = Vec::new();
    if parents.try_exists().map_err(|_| failure())? {
        private_directory(&parents)?;
        if indexed.try_exists().map_err(|_| failure())? {
            private_directory(&indexed)?;
            for (index, entry) in fs::read_dir(&indexed).map_err(|_| failure())?.enumerate() {
                if index >= 4096 {
                    return Err(failure());
                }
                let root = entry.map_err(|_| failure())?.path();
                private_directory(&root)?;
                let record: Record = read_json(&root.join("record.json"))?;
                if record.scope.parent_operation_id.as_deref() != Some(parent) {
                    return Err(failure());
                }
                roots.push(root);
            }
        }
    }
    // Schema-one fixture and older flat journals remain conservatively readable.
    // New parent-indexed journals never count toward this legacy scan bound.
    let mut legacy_count = 0;
    for entry in fs::read_dir(&base).map_err(|_| failure())? {
        let root = entry.map_err(|_| failure())?.path();
        if root == parents {
            continue;
        }
        legacy_count += 1;
        if legacy_count > 4096 {
            return Err(failure());
        }
        private_directory(&root)?;
        let record: Record = read_json(&root.join("record.json"))?;
        if record.scope.parent_operation_id.as_deref() == Some(parent) {
            roots.push(root);
        }
    }
    roots.sort();
    Ok(roots)
}
pub(crate) fn parent_scopes(storage: &Path, parent: &str) -> Result<Vec<Scope>, String> {
    let mut scopes = Vec::new();
    for root in parent_records(storage, parent)? {
        let record: Record = read_json(&root.join("record.json"))?;
        if record.schema != 1
            || record.host != kernel::HostWitness::admitted()?
            || root.file_name().and_then(|s| s.to_str()) != Some(record.scope.operation_id.as_str())
            || record.label != format!("dev.lomi.agent.host.{}", record.scope.operation_id)
            || record.domain != format!("gui/{}", unsafe { libc::getuid() })
        {
            return Err(failure());
        }
        if record.phase == Phase::Retired
            && (!record.seal_confirmed
                || kernel::usage(record.held.as_ref().ok_or_else(failure)?.resource_coalition)?
                    != kernel::Usage::Retired)
        {
            return Err(failure());
        }
        scopes.push(record.scope);
    }
    Ok(scopes)
}
pub(crate) fn settle_parent(storage: &Path, parent: &str) -> Result<Vec<RetiredReceipt>, String> {
    let mut receipts = Vec::new();
    for root in parent_records(storage, parent)? {
        receipts.push(Boundary::restore(&root)?.stop()?);
    }
    Ok(receipts)
}
pub(crate) fn parent_completed(storage: &Path, parent: &str) -> Result<bool, String> {
    for root in parent_records(storage, parent)? {
        let record: Record = read_json(&root.join("record.json"))?;
        if record.schema != 1
            || record.host != kernel::HostWitness::admitted()?
            || root.file_name().and_then(|s| s.to_str()) != Some(record.scope.operation_id.as_str())
            || record.label != format!("dev.lomi.agent.host.{}", record.scope.operation_id)
            || record.domain != format!("gui/{}", unsafe { libc::getuid() })
        {
            return Err(failure());
        }
        if record.phase != Phase::Retired || !record.seal_confirmed {
            return Ok(false);
        }
        let held = record.held.as_ref().ok_or_else(failure)?;
        if kernel::usage(held.resource_coalition)? != kernel::Usage::Retired {
            return Ok(false);
        }
    }
    Ok(true)
}
impl Drop for Boundary {
    fn drop(&mut self) {
        self.effects.cancel();
    }
}
impl NativeSpec {
    fn validate(&self) -> Result<(), String> {
        if matches!(
            self.io,
            NativeIo::Pty { rows: 0, .. } | NativeIo::Pty { cols: 0, .. }
        ) {
            return Err(failure());
        }
        if self.program.canonicalize().map_err(|_| failure())? != self.program
            || self.cwd.canonicalize().map_err(|_| failure())? != self.cwd
            || self.arguments.len() > 64
            || self
                .arguments
                .iter()
                .any(|a| a.len() > 65536 || a.contains('\0'))
            || self.environment.len() > 64
            || self.environment.iter().any(|(k, v)| {
                k.is_empty() || k.contains(['=', '\0']) || v.len() > 65536 || v.contains('\0')
            })
        {
            return Err(failure());
        }
        super::process_owner::executable(&self.program)?;
        self.policy.validate()?;
        Ok(())
    }
}
pub(crate) fn entry() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    let flag = args.get(1)?.to_str()?;
    if flag != WORKER_FLAG && flag != NATIVE_FLAG {
        return None;
    }
    if args.len() != 3 {
        return Some(2);
    }
    Some(
        if flag == NATIVE_FLAG {
            native_runner(Path::new(&args[2]))
        } else {
            worker(Path::new(&args[2]))
        }
        .unwrap_or(2),
    )
}
fn native_runner(root: &Path) -> Result<i32, String> {
    #[cfg(test)]
    unsafe {
        if libc::dup2(4, 1) < 0 || libc::dup2(5, 2) < 0 {
            return Err(failure());
        }
        libc::close(4);
        libc::close(5);
    }

    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    };
    unsafe {
        libc::umask(0o077);
    }
    private_directory(root)?;
    let record: Record = read_json(&root.join("record.json"))?;
    if record.schema != 1
        || record.phase != Phase::Released
        || record.host != kernel::HostWitness::admitted()?
    {
        return Err(failure());
    }
    let own = kernel::member(std::process::id())?;
    if own.resource_coalition != record.held.as_ref().ok_or_else(failure)?.resource_coalition
        || own.identity.executable != record.worker
        || binary_digest(&record.worker)? != record.worker_digest
    {
        return Err(failure());
    }
    let mut config = unsafe { fs::File::from_raw_fd(3) };
    let flags = unsafe { libc::fcntl(config.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(config.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(failure());
    }
    let length = u32::from_le_bytes(
        read_stream(&mut config, 4)?
            .try_into()
            .map_err(|_| failure())?,
    ) as usize;
    if length > MAX_SPEC {
        return Err(failure());
    }
    let mut bytes = read_stream(&mut config, length)?;
    if digest(&bytes) != record.spec_digest {
        bytes.fill(0);
        return Err(failure());
    }
    let parsed = serde_json::from_slice::<NativeSpec>(&bytes);
    bytes.fill(0);
    drop(config);
    let spec = parsed.map_err(|_| failure())?;
    spec.validate()?;
    if spec.policy.project_root != record.scope.project_root
        || record
            .scope
            .physical_account_root
            .as_ref()
            .is_some_and(|r| r != &spec.policy.account_root)
    {
        return Err(failure());
    }
    let terminal = if matches!(spec.io, NativeIo::Pty { .. }) {
        let mut path = [0i8; 128];
        if unsafe { libc::ttyname_r(0, path.as_mut_ptr(), path.len()) } != 0 {
            return Err(failure());
        }
        Some(PathBuf::from(
            unsafe { std::ffi::CStr::from_ptr(path.as_ptr()) }
                .to_str()
                .map_err(|_| failure())?,
        ))
    } else {
        None
    };
    // The fresh runner applies the sandbox without forking. No Rust allocator,
    // filesystem validation or sandbox library runs inside a pre_exec callback.
    spec.policy.apply_terminal(root, terminal.as_deref())?;
    for fd in 3..unsafe { libc::getdtablesize() } {
        unsafe {
            libc::close(fd);
        }
    }
    let mut command = std::process::Command::new(&spec.program);
    command
        .args(spec.arguments)
        .env_clear()
        .envs(spec.environment)
        .current_dir(spec.cwd);
    Err(format!(
        "Native execution was refused inside its sandbox: {}",
        command.exec().kind()
    ))
}
#[cfg(test)]
fn stage(root: &Path, name: &str) {
    let _ = std::fs::write(root.join(format!("stage-{name}")), b"");
}
fn worker(root: &Path) -> Result<i32, String> {
    unsafe {
        libc::umask(0o077);
    }
    #[cfg(test)]
    stage(root, "entered");
    use std::os::unix::process::CommandExt;
    private_directory(root)?;
    let record: Record = read_json(&root.join("record.json"))?;
    if record.schema != 1
        || record.phase != Phase::Intent
        || record.host != kernel::HostWitness::admitted()?
    {
        return Err(failure());
    }
    #[cfg(test)]
    stage(root, "host-admitted");
    #[cfg(test)]
    stage(root, "worker-hash-start");
    #[cfg(test)]
    let hash_start = Instant::now();
    let observed_digest = binary_digest(&std::env::current_exe().map_err(|_| failure())?)?;
    #[cfg(test)]
    let _ = fs::write(
        root.join("worker-hash-millis"),
        hash_start.elapsed().as_millis().to_string(),
    );
    #[cfg(test)]
    stage(root, "worker-hash-complete");
    if observed_digest != record.worker_digest {
        return Err(failure());
    }
    #[cfg(test)]
    stage(root, "verified");
    let mut config = open_fifo(&root.join("config"), true, false)?;
    let size = read_stream(&mut config, 4)?;
    let len = u32::from_le_bytes(size.try_into().map_err(|_| failure())?) as usize;
    if len > MAX_SPEC {
        return Err(failure());
    }
    let mut bytes = read_stream(&mut config, len)?;
    if digest(&bytes) != record.spec_digest {
        bytes.fill(0);
        return Err(failure());
    }
    let parsed = serde_json::from_slice::<NativeSpec>(&bytes);
    bytes.fill(0);
    let spec = parsed.map_err(|_| failure())?;
    spec.validate()?;
    drop(config);
    #[cfg(test)]
    stage(root, "parsed");
    let stdin = open_fifo(&root.join("stdin"), true, false)?;
    let stdout = open_fifo(&root.join("stdout"), false, true)?;
    let stderr = open_fifo(&root.join("stderr"), false, true)?;
    // Bootstrap and parent transport polling need nonblocking opens, but native
    // stdio must backpressure writers rather than lose output with EAGAIN.
    // These worker opens have separate file descriptions from parent readers.
    for file in [&stdin, &stdout, &stderr] {
        native_stdio(file)?;
    }
    #[cfg(test)]
    stage(root, "stdio");
    let terminal = match spec.io {
        NativeIo::Pipes => None,
        NativeIo::Pty { rows, cols } => Some(pty::Pty::create(root, rows, cols)?),
    };
    let member = kernel::member(std::process::id())?;
    let coalition = member.resource_coalition;
    write_json(
        &root.join("held.json"),
        &Held {
            schema: 1,
            operation_id: record.scope.operation_id.clone(),
            spec_digest: record.spec_digest.clone(),
            member,
        },
    )?;
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(release) = read_json::<Release>(&root.join("release.json")) {
            let latest: Record = read_json(&root.join("record.json"))?;
            if !matches!(latest.phase, Phase::ReleaseIntent | Phase::Released)
                || release.operation_id != record.scope.operation_id
                || release.spec_digest != record.spec_digest
            {
                return Err(failure());
            }
            break;
        }
        if Instant::now() > deadline {
            return Err(failure());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    #[cfg(test)]
    stage(root, "released");
    #[cfg(test)]
    let mut diagnostic = stderr.try_clone().map_err(|_| failure())?;
    let mut descriptors = [-1; 2];
    if unsafe { libc::pipe(descriptors.as_mut_ptr()) } != 0 {
        return Err(failure());
    }
    use std::os::fd::{AsRawFd, FromRawFd};
    let control = unsafe { fs::File::from_raw_fd(descriptors[0]) };
    let mut control_writer = unsafe { fs::File::from_raw_fd(descriptors[1]) };
    for file in [&control, &control_writer] {
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(failure());
        }
    }
    let mut command = std::process::Command::new(&record.worker);
    command
        .args([std::ffi::OsStr::new(NATIVE_FLAG), root.as_os_str()])
        .env_clear();
    #[cfg(test)]
    {
        command = std::process::Command::new(&record.worker);
        command
            .args([
                "--exact",
                "agent_runtime::host_boundary::tests::compiled_native_runner_entry",
                "--nocapture",
            ])
            .env_clear()
            .env("LOMI_HOST_NATIVE_CHILD_ROOT", root);
    }
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(terminal) = &terminal {
        command
            .stdin(terminal.slave.try_clone().map_err(|_| failure())?)
            .stdout(terminal.slave.try_clone().map_err(|_| failure())?)
            .stderr(terminal.slave.try_clone().map_err(|_| failure())?);
    }
    let control_fd = control.as_raw_fd();
    let use_terminal = terminal.is_some();
    unsafe {
        command.pre_exec(move || {
            kernel::inherited_coalition(coalition)?;
            if use_terminal && (libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY.into(), 0) < 0)
            {
                return Err(std::io::Error::last_os_error());
            }
            if libc::dup2(control_fd, 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            #[cfg(test)]
            {
                if libc::dup2(1, 4) < 0
                    || libc::dup2(2, 5) < 0
                    || libc::fcntl(4, libc::F_SETFD, 0) < 0
                    || libc::fcntl(5, libc::F_SETFD, 0) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
                if null < 0 || libc::dup2(null, 1) < 0 || libc::dup2(null, 2) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(null);
            }
            let max = libc::getdtablesize();
            #[cfg(test)]
            let first = 6;
            #[cfg(not(test))]
            let first = 4;
            for fd in first..max {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 && libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|error| {
        #[cfg(test)]
        let _ = writeln!(diagnostic, "native spawn failed: {error}");
        format!(
            "Native spawn failed inside the held boundary: {}",
            error.kind()
        )
    })?;
    drop(command);
    drop(control);
    let mut bytes = serde_json::to_vec(&spec).map_err(|_| failure())?;
    let flags = unsafe { libc::fcntl(control_writer.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe {
            libc::fcntl(
                control_writer.as_raw_fd(),
                libc::F_SETFL,
                flags | libc::O_NONBLOCK,
            )
        } < 0
    {
        return Err(failure());
    }
    write_stream(&mut control_writer, &(bytes.len() as u32).to_le_bytes())?;
    let sent = write_stream(&mut control_writer, &bytes);
    bytes.fill(0);
    sent?;
    drop(control_writer);
    #[cfg(test)]
    let _ = writeln!(
        diagnostic,
        "native child pid={} coalition={coalition}",
        child.id()
    );
    let status = if let Some(terminal) = terminal {
        drop(stderr);
        let output = terminal.relay(stdin, stdout)?;
        let status = child.wait().map_err(|_| failure())?;
        output.join().map_err(|_| failure())??;
        status
    } else {
        let mut child_stdin = child.stdin.take().ok_or_else(failure)?;
        let mut child_stdout = child.stdout.take().ok_or_else(failure)?;
        let mut child_stderr = child.stderr.take().ok_or_else(failure)?;
        std::thread::spawn(move || std::io::copy(&mut { stdin }, &mut child_stdin));
        let output = std::thread::spawn(move || std::io::copy(&mut child_stdout, &mut { stdout }));
        let errors = std::thread::spawn(move || std::io::copy(&mut child_stderr, &mut { stderr }));
        let status = child.wait().map_err(|_| failure())?;
        output
            .join()
            .map_err(|_| failure())?
            .map_err(|_| failure())?;
        errors
            .join()
            .map_err(|_| failure())?
            .map_err(|_| failure())?;
        status
    };
    use std::os::unix::process::ExitStatusExt;
    #[cfg(test)]
    let _ = writeln!(
        diagnostic,
        "native exit: code={:?} signal={:?}",
        status.code(),
        status.signal()
    );
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

#[cfg(test)]
mod driver_tests {
    use super::*;
    #[test]
    fn only_completed_owned_job_status_yields_an_outcome() {
        assert_eq!(
            parse_job_exit("state = running\nruns = 1\nlast exit code = 0").unwrap(),
            None
        );
        assert_eq!(
            parse_job_exit("state = waiting\nruns = 0\nlast exit code = 0").unwrap(),
            None
        );
        assert_eq!(
            parse_job_exit("state = waiting\nruns = 1\nlast exit code = 7").unwrap(),
            Some(7)
        );
        assert!(parse_job_exit("runs = 1\nlast exit code = 7\nlast exit code = 0").is_err());
        assert!(parse_job_exit("runs = 1\nlast exit code = 256").is_err());
    }
    #[test]
    fn legacy_scope_and_spec_keep_locked_down_defaults() {
        let scope: Scope = serde_json::from_str(r#"{"operationId":"a","accountId":"fixture","authRevision":1,"taskId":null,"attemptId":null,"generation":null,"projectRoot":"/private/tmp/project","purpose":"attempt"}"#).unwrap();
        assert!(scope.parent_operation_id.is_none());
        assert!(scope.physical_account_root.is_none());
        assert!(scope.storage_root.is_none());
        assert!(matches!(NativeIo::default(), NativeIo::Pipes));
    }
}

#[cfg(test)]
pub(crate) fn test_effect_scope() -> std::sync::Arc<EffectScope> {
    let scope = EffectScope::new();
    scope.release().unwrap();
    scope
}

#[cfg(test)]
mod journal_tests {
    use super::*;
    #[test]
    fn parent_indexes_do_not_count_unrelated_parent_directories() {
        let storage = tempfile::tempdir().unwrap();
        let storage = storage.path().canonicalize().unwrap();
        fs::set_permissions(&storage, fs::Permissions::from_mode(0o700)).unwrap();
        let selected = prepare_parent(&storage, "selected").unwrap();
        let parents = selected.parent().unwrap();
        for i in 0..4100 {
            fs::create_dir(parents.join(format!("unrelated-{i}"))).unwrap();
        }
        assert!(parent_records(&storage, "selected").unwrap().is_empty());
        assert!(parent_records(&storage, "new-parent").unwrap().is_empty());
    }
}
