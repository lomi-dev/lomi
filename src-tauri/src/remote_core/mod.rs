//! Native host of the shared Remote core (plan-1.0 P2.19), compiled only with the
//! `remote-host` feature and active only when the build pins `LOMI_REMOTE_CONTRACT_HASH`.
//!
//! One actor owns the sans-I/O [`engine::Engine`] and performs its I/O: the session
//! exchange over the existing desktop HTTP stack with the desktop bearer (which never
//! leaves Rust), the relay link over `tokio-tungstenite`, and the encrypted directory in
//! `app_data` with its key in the OS keychain. The webview only receives state and timing
//! marks over a Tauri channel, so Rust is the single source of truth for the Host list.

mod engine;
mod vault;

use crate::auth::controller::CoreIdentity;
use engine::{CoreState, Engine, Exchange, Ids, NameHint, Out, Status, RETRY_MS};
use futures_util::{SinkExt, StreamExt};
use lomi_remote_core::directory::Directory;
use lomi_remote_core::link::Now;
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{ipc::Channel as IpcChannel, Manager, State, Webview, Window};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use vault::{FileVault, KeyringStore, Vault};

const WIPE_RETRY: Duration = Duration::from_secs(5);
/// Least time between two session exchanges that the user or a repeated start asks for.
const USER_SESSION_INTERVAL: Duration = Duration::from_secs(30);
/// A wipe command reports failure when the vault does not confirm in this time.
const WIPE_CONFIRM: Duration = Duration::from_secs(10);
const MAX_NAME_HINTS: usize = 512;
const MAX_NAME_BYTES: usize = 256;

/// The server's `REMOTE_SESSION_CONTRACT_HASH` this build speaks; 64 lowercase hex.
fn contract_hash() -> Option<&'static str> {
    option_env!("LOMI_REMOTE_CONTRACT_HASH").filter(|h| valid_hash(h))
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The legacy channel endpoint (`/v2/relay`) and the device link (`/relay`) share an origin.
fn link_url(endpoint: &str) -> Option<String> {
    let mut url = tauri::Url::parse(endpoint).ok()?;
    if url.scheme() != "wss" {
        return None;
    }
    url.set_path("/relay");
    url.set_query(None);
    Some(url.to_string())
}

fn enabled_link_url() -> Option<String> {
    contract_hash()?;
    link_url(option_env!("LOMI_HOST_RELAY_ENDPOINT")?)
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum CoreEvent {
    State {
        state: CoreState,
    },
    Mark {
        name: &'static str,
        detail: Option<serde_json::Value>,
    },
}

enum Cmd {
    Subscribe(IpcChannel<CoreEvent>),
    Start { expected_account: Option<String> },
    AuthChanged,
    Names(Vec<NameHint>),
    Renew,
    Wipe(oneshot::Sender<bool>),
}

enum Io {
    Open(u64),
    Message(u64, Vec<u8>),
    Closed(u64),
    /// Session exchange result: (epoch, user, status, body) or a local error code.
    Session(u64, Result<(String, u16, Vec<u8>), String>),
    Vault(VaultEvent),
}

/// Keychain and file calls can block (a Secret Service prompt, a locked keychain), so one
/// worker thread performs them in order and the actor never waits for it.
enum VaultJob {
    Load(String),
    Persist(Ids, Directory),
    Wipe,
}

enum VaultEvent {
    Loaded(Result<Option<(Ids, Directory)>, ()>),
    Wiped(bool),
}

fn vault_worker(
    mut vault: Box<dyn Vault>,
    jobs: std::sync::mpsc::Receiver<VaultJob>,
    io: mpsc::UnboundedSender<Io>,
) {
    while let Ok(job) = jobs.recv() {
        let event = match job {
            VaultJob::Load(user) => Some(VaultEvent::Loaded(
                vault::open_for(&mut *vault, &user).map_err(|_| ()),
            )),
            VaultJob::Persist(ids, dir) => {
                // Best effort: the next change seals again.
                let _ = vault::persist(&mut *vault, &ids, &dir);
                None
            }
            VaultJob::Wipe => Some(VaultEvent::Wiped(vault.wipe().is_ok())),
        };
        if let Some(event) = event {
            let _ = io.send(Io::Vault(event));
        }
    }
}

enum SockCmd {
    Send(Vec<u8>),
    Close,
}

#[derive(Default)]
pub(crate) struct RemoteCore {
    tx: Mutex<Option<mpsc::UnboundedSender<Cmd>>>,
}

impl RemoteCore {
    fn sender(
        &self,
        app: &tauri::AppHandle,
        auth: &crate::auth::AuthController,
    ) -> Result<mpsc::UnboundedSender<Cmd>, String> {
        let mut slot = self.tx.lock().map_err(|_| "Remote core is unavailable.")?;
        if let Some(tx) = slot.as_ref() {
            return Ok(tx.clone());
        }
        let link = enabled_link_url().ok_or("Remote core is not configured for this build.")?;
        let hash = contract_hash().ok_or("Remote core is not configured for this build.")?;
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|_| "Remote core storage is unavailable.")?
            .join("remote-core")
            .join("directory.bin");
        let vault = FileVault::new(dir, KeyringStore::new(keyring_service()));
        let (tx, rx) = mpsc::unbounded_channel();
        let actor = Actor::new(auth.clone(), Box::new(vault), hash, link);
        tauri::async_runtime::spawn(actor.run(rx));
        *slot = Some(tx.clone());
        Ok(tx)
    }

    fn notify(&self) {
        if let Some(tx) = self.tx.lock().ok().and_then(|slot| slot.clone()) {
            let _ = tx.send(Cmd::AuthChanged);
        }
    }
}

/// Called whenever the local account state changes (sign-in, sign-out, another user).
pub(crate) fn auth_changed(app: &tauri::AppHandle) {
    if let Some(core) = app.try_state::<RemoteCore>() {
        core.notify();
    }
}

fn keyring_service() -> String {
    use sha2::{Digest, Sha256};
    let origin = option_env!("LOMI_HOST_IDENTITY_ORIGIN").unwrap_or_default();
    let digest = Sha256::digest(origin.as_bytes());
    let tag: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("dev.lomi.desktop.remote-core.directory.v1.{tag}")
}

struct Actor {
    auth: crate::auth::AuthController,
    engine: Engine,
    vault: std::sync::mpsc::Sender<VaultJob>,
    link: String,
    subs: Vec<IpcChannel<CoreEvent>>,
    clock: Instant,
    io_tx: mpsc::UnboundedSender<Io>,
    io_rx: Option<mpsc::UnboundedReceiver<Io>>,
    socket: Option<(u64, mpsc::UnboundedSender<SockCmd>)>,
    socket_gen: u64,
    started: bool,
    cycle: bool,
    inflight: bool,
    epoch: u64,
    attempt: usize,
    last_cycle: Option<Instant>,
    cache_requested: bool,
    wipe_inflight: bool,
    wipe_queued: bool,
    wipe_waiters: Vec<oneshot::Sender<bool>>,
    retry_at: Option<Instant>,
    wipe_retry_at: Option<Instant>,
}

impl Actor {
    fn new(
        auth: crate::auth::AuthController,
        vault: Box<dyn Vault>,
        hash: &str,
        link: String,
    ) -> Self {
        use ring::rand::SecureRandom;
        let mut seed = [0u8; 8];
        let _ = ring::rand::SystemRandom::new().fill(&mut seed);
        let (io_tx, io_rx) = mpsc::unbounded_channel();
        let (vault_tx, vault_rx) = std::sync::mpsc::channel();
        let worker_io = io_tx.clone();
        std::thread::spawn(move || vault_worker(vault, vault_rx, worker_io));
        Self {
            auth,
            engine: Engine::new(hash, u64::from_le_bytes(seed)),
            vault: vault_tx,
            link,
            subs: Vec::new(),
            clock: Instant::now(),
            io_tx,
            io_rx: Some(io_rx),
            socket: None,
            socket_gen: 0,
            started: false,
            cycle: false,
            inflight: false,
            epoch: 0,
            attempt: 0,
            last_cycle: None,
            cache_requested: false,
            wipe_inflight: false,
            wipe_queued: false,
            wipe_waiters: Vec::new(),
            retry_at: None,
            wipe_retry_at: None,
        }
    }

    fn now(&self) -> Now {
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(1);
        Now {
            mono_ms: self.clock.elapsed().as_millis() as u64 + 1,
            wall_ms: wall,
        }
    }

    fn instant(&self, mono_ms: u64) -> Instant {
        self.clock + Duration::from_millis(mono_ms.saturating_sub(1))
    }

    async fn run(mut self, mut cmds: mpsc::UnboundedReceiver<Cmd>) {
        let Some(mut io) = self.io_rx.take() else {
            return;
        };
        loop {
            let deadline = [
                self.engine.next_deadline().map(|ms| self.instant(ms)),
                self.retry_at,
                self.wipe_retry_at,
            ]
            .into_iter()
            .flatten()
            .min();
            let sleep = async {
                match deadline {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                cmd = cmds.recv() => match cmd {
                    Some(cmd) => self.command(cmd),
                    None => return,
                },
                Some(event) = io.recv() => self.io(event),
                _ = sleep => self.timer(),
            }
        }
    }

    fn timer(&mut self) {
        let at = Instant::now();
        if self.retry_at.is_some_and(|r| r <= at) {
            self.retry_at = None;
            self.attempt_session();
        }
        if self.wipe_retry_at.is_some_and(|r| r <= at) {
            self.wipe_retry_at = None;
            self.wipe_vault();
        }
        let outs = self.engine.tick(self.now());
        self.apply(outs);
    }

    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Subscribe(channel) => {
                let _ = channel.send(CoreEvent::State {
                    state: self.engine.state().clone(),
                });
                self.subs.push(channel);
            }
            Cmd::Start { expected_account } => {
                self.engine.set_expected_account(expected_account);
                if !self.started {
                    self.started = true;
                    self.load_cache();
                }
                if matches!(
                    self.engine.state().status,
                    Status::Idle | Status::SignedOut | Status::LinkDown | Status::Cached
                ) {
                    self.user_session();
                }
            }
            Cmd::AuthChanged => self.auth_changed(),
            Cmd::Names(hints) => {
                let outs = self.engine.set_names(&hints, self.now());
                self.apply(outs);
            }
            Cmd::Renew => self.user_session(),
            Cmd::Wipe(reply) => {
                self.wipe(Status::SignedOut);
                self.wipe_waiters.push(reply);
            }
        }
    }

    fn auth_changed(&mut self) {
        if !self.started {
            return;
        }
        match self.auth.remote_core_identity() {
            CoreIdentity::SignedOut => self.wipe(Status::SignedOut),
            CoreIdentity::User(user) => {
                if self.engine.ids().is_some_and(|ids| ids.user != user) {
                    self.wipe(Status::Idle);
                }
                if !self.cache_requested {
                    self.load_cache();
                }
                if matches!(self.engine.state().status, Status::Idle | Status::SignedOut) {
                    self.request_session(None);
                }
            }
            CoreIdentity::Unknown => {}
        }
    }

    /// Asks the vault worker for the directory of the signed-in user; a signed-out device
    /// has its leftovers wiped. Repeated when the identity first becomes known.
    fn load_cache(&mut self) {
        match self.auth.remote_core_identity() {
            CoreIdentity::User(user) => {
                self.cache_requested = true;
                let _ = self.vault.send(VaultJob::Load(user));
            }
            CoreIdentity::SignedOut => {
                self.cache_requested = true;
                self.wipe_vault();
            }
            CoreIdentity::Unknown => {}
        }
    }

    fn wipe(&mut self, status: Status) {
        self.epoch += 1;
        self.inflight = false;
        self.cycle = false;
        self.retry_at = None;
        self.last_cycle = None;
        self.cache_requested = false;
        let outs = self.engine.wipe(status, self.now());
        self.apply(outs);
    }

    fn wipe_vault(&mut self) {
        self.wipe_retry_at = None;
        if self.wipe_inflight {
            self.wipe_queued = true;
        } else {
            self.wipe_inflight = true;
            let _ = self.vault.send(VaultJob::Wipe);
        }
    }

    fn vault_event(&mut self, event: VaultEvent) {
        match event {
            VaultEvent::Loaded(Ok(Some((ids, dir)))) => {
                // Signed out or another user while the key was being read: show nothing.
                let current = matches!(
                    self.auth.remote_core_identity(),
                    CoreIdentity::User(ref user) if *user == ids.user
                );
                if current {
                    let outs = self.engine.load_cache(dir, ids, self.now());
                    self.apply(outs);
                } else {
                    self.wipe_vault();
                }
            }
            VaultEvent::Loaded(Ok(None)) => {}
            // Unreadable store: never keep what cannot be read.
            VaultEvent::Loaded(Err(())) => self.wipe_vault(),
            VaultEvent::Wiped(ok) => {
                self.wipe_inflight = false;
                if ok && self.wipe_queued {
                    self.wipe_queued = false;
                    return self.wipe_vault();
                }
                self.wipe_queued = false;
                for waiter in self.wipe_waiters.drain(..) {
                    let _ = waiter.send(ok);
                }
                if !ok {
                    // Something may remain (keychain locked): never leave it, retry.
                    self.wipe_retry_at = Some(Instant::now() + WIPE_RETRY);
                }
            }
        }
    }

    // ---- session exchange and token renewal ----

    /// A start or renew asked by the UI: spaced like the core spaces its own renewals, so
    /// a page that keeps asking cannot hammer the server.
    fn user_session(&mut self) {
        if self
            .last_cycle
            .is_some_and(|at| at.elapsed() < USER_SESSION_INTERVAL)
        {
            return;
        }
        self.request_session(None);
    }

    /// Single flight; the core already spaces its own renewal requests.
    fn request_session(&mut self, delay: Option<Duration>) {
        if self.cycle {
            return;
        }
        self.cycle = true;
        self.last_cycle = Some(Instant::now());
        self.attempt = 0;
        let outs = self.engine.begin_exchange(self.now());
        self.apply(outs);
        match delay {
            Some(delay) => self.retry_at = Some(Instant::now() + delay),
            None => self.attempt_session(),
        }
    }

    fn attempt_session(&mut self) {
        if !self.cycle || self.inflight {
            return;
        }
        self.inflight = true;
        let (auth, body, epoch, tx) = (
            self.auth.clone(),
            self.engine.session_body(),
            self.epoch,
            self.io_tx.clone(),
        );
        tauri::async_runtime::spawn(async move {
            let result = auth
                .remote_core_session(body)
                .await
                .map(|(binding, status, bytes)| (binding.user_id, status, bytes))
                .map_err(|e| e.code);
            let _ = tx.send(Io::Session(epoch, result));
        });
    }

    fn session_done(&mut self, result: Result<(String, u16, Vec<u8>), String>) {
        self.inflight = false;
        let exchange = match result {
            Ok((user, status, body)) => {
                let (exchange, outs) = self.engine.on_session(status, &body, &user, self.now());
                self.apply(outs);
                exchange
            }
            Err(code) => match code.as_str() {
                // The account changed or is signed out: the auth change path decides what is next.
                "SESSION_CHANGED"
                | "AUTHORIZATION_REQUIRED"
                | "REMOTE_DISABLED"
                | "IDENTITY_MISMATCH" => Exchange::Final,
                _ => Exchange::Retry,
            },
        };
        if exchange == Exchange::Retry {
            let ms = RETRY_MS[self.attempt.min(RETRY_MS.len() - 1)];
            self.attempt += 1;
            self.retry_at = Some(Instant::now() + Duration::from_millis(ms));
        } else {
            self.cycle = false;
            let outs = self.engine.end_exchange(self.now());
            self.apply(outs);
        }
    }

    // ---- I/O events and actions ----

    fn io(&mut self, event: Io) {
        let now = self.now();
        let outs = match event {
            Io::Session(epoch, result) => {
                if epoch == self.epoch && self.inflight {
                    self.session_done(result);
                }
                return;
            }
            Io::Vault(event) => return self.vault_event(event),
            Io::Open(gen) if self.socket.as_ref().is_some_and(|s| s.0 == gen) => {
                self.engine.on_open(now)
            }
            Io::Message(gen, bytes) if self.socket.as_ref().is_some_and(|s| s.0 == gen) => {
                self.engine.on_message(&bytes, now)
            }
            Io::Closed(gen) if self.socket.as_ref().is_some_and(|s| s.0 == gen) => {
                self.socket = None;
                self.engine.on_close(now)
            }
            _ => return,
        };
        self.apply(outs);
    }

    fn apply(&mut self, outs: Vec<Out>) {
        for out in outs {
            match out {
                Out::Connect => self.connect(),
                Out::Send(bytes) => {
                    if let Some((_, tx)) = &self.socket {
                        let _ = tx.send(SockCmd::Send(bytes));
                    }
                }
                Out::Close => {
                    if let Some((_, tx)) = self.socket.take() {
                        let _ = tx.send(SockCmd::Close);
                    }
                }
                Out::Persist => self.persist(),
                Out::WipeVault => self.wipe_vault(),
                Out::NeedToken => self.request_session(None),
                Out::NeedTokenLater(ms) => {
                    self.request_session(Some(Duration::from_millis(ms)));
                }
                Out::State(state) => self.broadcast(CoreEvent::State { state }),
                Out::Mark(name, detail) => self.broadcast(CoreEvent::Mark {
                    name,
                    detail: detail.map(|(k, v)| serde_json::json!({ k: v })),
                }),
            }
        }
    }

    fn broadcast(&mut self, event: CoreEvent) {
        self.subs
            .retain(|channel| channel.send(event.clone()).is_ok());
    }

    fn persist(&mut self) {
        if let Some((ids, dir)) = self.engine.snapshot() {
            let _ = self.vault.send(VaultJob::Persist(ids, dir));
        }
    }

    fn connect(&mut self) {
        self.socket_gen += 1;
        let gen = self.socket_gen;
        let (tx, rx) = mpsc::unbounded_channel();
        self.socket = Some((gen, tx));
        tauri::async_runtime::spawn(socket_task(self.link.clone(), gen, self.io_tx.clone(), rx));
    }
}

async fn socket_task(
    url: String,
    gen: u64,
    io: mpsc::UnboundedSender<Io>,
    mut cmds: mpsc::UnboundedReceiver<SockCmd>,
) {
    let opened = tokio::time::timeout(Duration::from_secs(10), connect_async(url.as_str())).await;
    let Ok(Ok((ws, _))) = opened else {
        let _ = io.send(Io::Closed(gen));
        return;
    };
    let _ = io.send(Io::Open(gen));
    let (mut sink, mut stream) = ws.split();
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(SockCmd::Send(bytes)) => {
                    if sink.send(Message::Binary(bytes.into())).await.is_err() { break; }
                }
                // Asked by the core: it must not hear about this close.
                Some(SockCmd::Close) | None => { let _ = sink.close().await; return; }
            },
            message = stream.next() => match message {
                Some(Ok(Message::Binary(bytes))) => { let _ = io.send(Io::Message(gen, bytes.to_vec())); }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => break,
            },
        }
    }
    let _ = io.send(Io::Closed(gen));
}

// ---- commands ----

fn trusted(window: &Window, webview: &Webview) -> Result<(), String> {
    crate::shared_host::trusted_full_window(window, webview)?;
    crate::desktop_runtime::require_shared_host()
}

/// Subscribes this window to the core state and starts the core. `false`: the core is not
/// configured in this build and the UI keeps the inventory path.
#[tauri::command]
pub(crate) fn remote_v2_core_start(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    core: State<'_, RemoteCore>,
    auth: State<'_, crate::auth::AuthController>,
    expected_account: Option<String>,
    events: IpcChannel<CoreEvent>,
) -> Result<bool, String> {
    trusted(&window, &webview)?;
    if enabled_link_url().is_none() {
        return Ok(false);
    }
    let tx = core.sender(&app, &auth)?;
    let _ = tx.send(Cmd::Subscribe(events));
    let _ = tx.send(Cmd::Start { expected_account });
    Ok(true)
}

#[tauri::command]
pub(crate) fn remote_v2_core_names(
    window: Window,
    webview: Webview,
    core: State<'_, RemoteCore>,
    hosts: Vec<NameHint>,
) -> Result<(), String> {
    trusted(&window, &webview)?;
    let hints: Vec<NameHint> = hosts
        .into_iter()
        .take(MAX_NAME_HINTS)
        .map(|mut h| {
            h.name = h.name.filter(|n| n.len() <= MAX_NAME_BYTES);
            h.workspaces.truncate(MAX_NAME_HINTS);
            for w in &mut h.workspaces {
                w.name = w.name.take().filter(|n| n.len() <= MAX_NAME_BYTES);
            }
            h
        })
        .collect();
    if let Some(tx) = core.tx.lock().ok().and_then(|s| s.clone()) {
        let _ = tx.send(Cmd::Names(hints));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn remote_v2_core_renew(
    window: Window,
    webview: Webview,
    core: State<'_, RemoteCore>,
) -> Result<(), String> {
    trusted(&window, &webview)?;
    if let Some(tx) = core.tx.lock().ok().and_then(|s| s.clone()) {
        let _ = tx.send(Cmd::Renew);
    }
    Ok(())
}

/// Logout or revoke from the UI: link closed, directory wiped, key destroyed.
#[tauri::command]
pub(crate) async fn remote_v2_core_wipe(
    window: Window,
    webview: Webview,
    core: State<'_, RemoteCore>,
) -> Result<(), String> {
    trusted(&window, &webview)?;
    let tx = core.tx.lock().ok().and_then(|s| s.clone());
    let Some(tx) = tx else { return Ok(()) };
    let (reply, done) = oneshot::channel();
    tx.send(Cmd::Wipe(reply))
        .map_err(|_| "Remote core is unavailable.")?;
    match tokio::time::timeout(WIPE_CONFIRM, done).await {
        Ok(Ok(true)) => Ok(()),
        _ => Err("The Remote directory could not be fully erased yet.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_hash_must_be_lowercase_hex_of_64() {
        assert!(valid_hash(&"a".repeat(64)));
        assert!(!valid_hash(&"A".repeat(64)));
        assert!(!valid_hash(&"a".repeat(63)));
        assert!(!valid_hash(""));
    }

    #[test]
    fn link_url_reuses_the_relay_origin_with_the_device_path() {
        assert_eq!(
            link_url("wss://relay.lomi.dev/v2/relay").as_deref(),
            Some("wss://relay.lomi.dev/relay")
        );
        assert_eq!(
            link_url("wss://127.0.0.1:9443/v2/relay?x=1").as_deref(),
            Some("wss://127.0.0.1:9443/relay")
        );
        assert!(link_url("https://relay.lomi.dev/v2/relay").is_none());
        assert!(link_url("ws://relay.lomi.dev/v2/relay").is_none());
        assert!(link_url("not a url").is_none());
    }
}
