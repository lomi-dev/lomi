//! Process-local admission for Lomi project writes. External editors and PTYs
//! do not participate; effect CAS still requires the filesystem broker's proof.
use std::{
    collections::HashMap,
    fs::{self, File},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

#[cfg(target_os = "macos")]
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
use lomi_remote_protocol::ipc::{
    LocalCall, LocalError, LocalNativeParentOperationId, LocalResult, LocalRootPath,
    NativeBindJobRequest, NativeIntentId, NativeIntentRequest, NativeIntentScope,
    NativeIntentTarget, NativeLease, NativeWriterState,
};

const STOP_RUN: &str = "Stop the active coding run before changing files in this project.";
const BUSY: &str = "Wait for the current file operation to finish before starting this coding run.";
const UNAVAILABLE: &str = "Project write admission is unavailable.";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectId {
    device: u64,
    inode: u64,
}
#[cfg(unix)]
fn object(meta: &fs::Metadata) -> ObjectId {
    use std::os::unix::fs::MetadataExt;
    ObjectId {
        device: meta.dev(),
        inode: meta.ino(),
    }
}
#[cfg(not(unix))]
fn object(_: &fs::Metadata) -> ObjectId {
    ObjectId {
        device: 0,
        inode: 0,
    }
}
struct Root {
    path: PathBuf,
    file: File,
    identity: ObjectId,
    ancestry: Vec<ObjectId>,
}
impl Root {
    fn open(path: &Path) -> Result<Self, String> {
        #[cfg(not(unix))]
        {
            let _ = path;
            return Err("Coding project leases are unavailable on this platform.".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let path = path
                .canonicalize()
                .map_err(|_| "Cannot resolve the coding project directory.")?;
            let mut options = fs::OpenOptions::new();
            options
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
            let file = options
                .open(&path)
                .map_err(|_| "Cannot retain the coding project directory.")?;
            let meta = file.metadata().map_err(|_| UNAVAILABLE)?;
            if !meta.is_dir() {
                return Err("The coding project must be a directory.".into());
            }
            let identity = object(&meta);
            let ancestry = directories(&path)?;
            let root = Self {
                path,
                file,
                identity,
                ancestry,
            };
            root.check()?;
            Ok(root)
        }
    }
    fn check(&self) -> Result<(), String> {
        let current = fs::symlink_metadata(&self.path).map_err(|_| STOP_RUN)?;
        let retained = self.file.metadata().map_err(|_| STOP_RUN)?;
        if !current.is_dir()
            || current.file_type().is_symlink()
            || object(&current) != self.identity
            || object(&retained) != self.identity
        {
            return Err(STOP_RUN.into());
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Scope {
    path: PathBuf,
    directories: Vec<ObjectId>,
}
impl Scope {
    fn resolve(path: &Path) -> Result<Self, String> {
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        {
            return Err("File write destinations must have an absolute resolved path.".into());
        }
        let mut existing = path;
        let mut suffix = Vec::new();
        loop {
            match fs::symlink_metadata(existing) {
                Ok(_) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    suffix.push(existing.file_name().ok_or(UNAVAILABLE)?.to_owned());
                    existing = existing.parent().ok_or(UNAVAILABLE)?;
                }
                Err(_) => return Err(UNAVAILABLE.into()),
            }
        }
        // A dangling or cyclic symlink is an error, not an unowned future path.
        let canonical = existing
            .canonicalize()
            .map_err(|_| "Cannot establish file write destination ownership.")?;
        if !suffix.is_empty() && !canonical.is_dir() {
            return Err("The file write parent is not a directory.".into());
        }
        let mut resolved = canonical.clone();
        for part in suffix.iter().rev() {
            resolved.push(part);
        }
        let ancestor = if canonical.is_dir() {
            canonical.as_path()
        } else {
            canonical.parent().ok_or(UNAVAILABLE)?
        };
        Ok(Self {
            path: resolved,
            directories: directories(ancestor)?,
        })
    }
    fn overlaps(&self, root: &Root) -> bool {
        overlap(&self.path, &root.path)
            || self.directories.contains(&root.identity)
            || fs::metadata(&self.path)
                .is_ok_and(|m| m.is_dir() && root.ancestry.contains(&object(&m)))
    }
}
fn directories(path: &Path) -> Result<Vec<ObjectId>, String> {
    path.ancestors()
        .map(|ancestor| {
            let meta = fs::metadata(ancestor).map_err(|_| UNAVAILABLE)?;
            if !meta.is_dir() {
                return Err(UNAVAILABLE.into());
            }
            Ok(object(&meta))
        })
        .collect()
}
fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
struct ActiveLease {
    root: Arc<Root>,
    run: String,
    generation: u64,
}
#[derive(Default)]
struct Registry {
    sequence: u64,
    recovery_ceilings: usize,
    leases: HashMap<u64, ActiveLease>,
    writes: HashMap<u64, Option<Vec<Scope>>>,
}
type Shared = Arc<Mutex<Registry>>;
static REGISTRY: OnceLock<Shared> = OnceLock::new();
fn shared() -> Shared {
    REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(Registry::default())))
        .clone()
}

#[cfg(target_os = "macos")]
impl NativeIntentHandle {
    fn register(operation_id: &str, store_root: &Path) -> Result<Self, String> {
        let id = native_intent_id(operation_id)?;
        let store_root = store_root
            .to_str()
            .ok_or("Native runtime storage path is not valid UTF-8.")?;
        if store_root.is_empty() || store_root.chars().any(char::is_control) {
            return Err("Native runtime storage path is invalid.".into());
        }
        let profile = canonical_host_profile()?;
        let mut client = connect_host(&profile)?;
        match local_result(
            &mut client,
            LocalCall::NativeIntent(NativeIntentRequest {
                intent_id: id,
                scope: NativeIntentScope::Unscoped {},
            }),
        )? {
            LocalResult::NativeIntent(returned) if returned == id => {}
            _ => return Err("The canonical host did not register the native Attempt.".into()),
        }
        Ok(Self {
            operation_id: operation_id.to_owned(),
            inner: Arc::new(Mutex::new(NativeIntentSession {
                id,
                profile,
                client: Some(client),
                phase: NativeIntentPhase::Registered,
            })),
        })
    }

    fn recovered(operation_id: &str) -> Result<Self, String> {
        let id = native_intent_id(operation_id)?;
        Ok(Self {
            operation_id: operation_id.to_owned(),
            inner: Arc::new(Mutex::new(NativeIntentSession {
                id,
                profile: canonical_host_profile()?,
                client: None,
                phase: NativeIntentPhase::Unknown,
            })),
        })
    }

    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub(crate) fn bind_job(&self, store_root: &Path, operation_id: &str) -> Result<(), String> {
        if operation_id != self.operation_id {
            return Err("The native Attempt does not match its registered host intent.".into());
        }
        let root = store_root
            .to_str()
            .ok_or("Native runtime storage path is not valid UTF-8.")?;
        let store_root = LocalRootPath::try_from(root.to_owned())
            .map_err(|_| "Native runtime storage path is invalid.")?;
        let parent_operation_id = LocalNativeParentOperationId::try_from(operation_id.to_owned())
            .map_err(|_| "Native Attempt operation identity is invalid.")?;
        let mut session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
        if !matches!(
            &session.phase,
            NativeIntentPhase::Registered | NativeIntentPhase::BindPending
        ) || session.client.is_none()
        {
            return Err(
                "The native host intent is not available for its first Attempt bind.".into(),
            );
        }
        session.phase = NativeIntentPhase::BindPending;
        let call = LocalCall::NativeBindJob(NativeBindJobRequest {
            intent_id: session.id,
            store_root,
            parent_operation_id,
        });
        let result = match local_response(session.client.as_mut().unwrap(), call) {
            Ok(Ok(LocalResult::NativeLease(lease))) => Ok(lease),
            Ok(Err(LocalError::NativeBindRejected)) => {
                // This dedicated response is emitted only after the Host has
                // atomically sealed the still-Registered intent as an
                // unbound Finished tombstone.
                session.phase = NativeIntentPhase::BindRejected;
                session.client = None;
                return Err("The held native Attempt could not be independently captured.".into());
            }
            Ok(Err(_)) | Ok(Ok(_)) | Err(_) => {
                Err("The canonical host could not bind the native Attempt.".to_owned())
            }
        };
        match result {
            Ok(lease) if valid_lease(&lease, session.id, NativeWriterState::Active) => {
                session.phase = NativeIntentPhase::Active {
                    fence: lease.fence.0,
                    expires_at_ms: lease.expires_at_ms.0,
                    check_at: Instant::now() + Duration::from_secs(30),
                };
                Ok(())
            }
            _ => {
                session.phase = NativeIntentPhase::Unknown;
                session.client = None;
                Err("The native Attempt bind outcome is unknown; the workload remains held.".into())
            }
        }
    }

    pub(crate) fn mark_bind_attempted(&self) -> Result<(), String> {
        let mut session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
        match &session.phase {
            NativeIntentPhase::Registered if session.client.is_some() => {
                session.phase = NativeIntentPhase::BindPending;
                Ok(())
            }
            NativeIntentPhase::BindPending if session.client.is_some() => Ok(()),
            _ => Err("The native Host intent cannot enter its first bind.".into()),
        }
    }

    pub(crate) fn before_helper_release(
        &self,
        purpose: &crate::agent_runtime::NativeHostPurpose,
    ) -> Result<(), String> {
        let registered_read_only_helper = matches!(
            purpose,
            crate::agent_runtime::NativeHostPurpose::VersionProbe
                | crate::agent_runtime::NativeHostPurpose::Resolver
        );
        {
            let session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
            if matches!(&session.phase, NativeIntentPhase::Registered)
                && registered_read_only_helper
            {
                return Ok(());
            }
        }
        self.check_active(true)
    }

    pub(crate) fn check_for_execution(&self) -> Result<(), String> {
        let session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
        let active = matches!(&session.phase, NativeIntentPhase::Active { .. });
        let result = match &session.phase {
            NativeIntentPhase::Registered
            | NativeIntentPhase::Finished
            | NativeIntentPhase::BindRejected
            | NativeIntentPhase::Closed => Ok(()),
            NativeIntentPhase::BindPending
            | NativeIntentPhase::Quarantined
            | NativeIntentPhase::Unknown => Err(
                "The native host lease is uncertain; execution remains fenced for recovery.".into(),
            ),
            NativeIntentPhase::Active { .. } => Ok(()),
        };
        drop(session);
        if active {
            self.check_active(false)
        } else {
            result
        }
    }

    fn check_active(&self, force: bool) -> Result<(), String> {
        let mut session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
        let (fence, expires_at_ms, check_at) = match &session.phase {
            NativeIntentPhase::Active {
                fence,
                expires_at_ms,
                check_at,
            } => (*fence, *expires_at_ms, *check_at),
            _ => {
                return Err(
                    "A bound native host lease is required before this helper can start.".into(),
                );
            }
        };
        if !force && Instant::now() < check_at {
            return Ok(());
        }
        let now = unix_time_ms()?;
        if now >= expires_at_ms {
            session.phase = NativeIntentPhase::Unknown;
            session.client = None;
            return Err("The native host lease expired; execution remains fenced.".into());
        }
        let id = session.id;
        let checked = session
            .client
            .as_mut()
            .ok_or_else(|| "The native host connection was lost.".to_owned())
            .and_then(|client| {
                local_result(
                    client,
                    LocalCall::NativeCheck(NativeIntentTarget { intent_id: id }),
                )
            });
        let mut lease = match checked {
            Ok(LocalResult::NativeLease(lease))
                if valid_lease(&lease, id, NativeWriterState::Active) && lease.fence.0 == fence =>
            {
                lease
            }
            _ => {
                session.phase = NativeIntentPhase::Unknown;
                session.client = None;
                return Err(
                    "The canonical host no longer confirms the active native lease.".into(),
                );
            }
        };
        let renew_margin = 60_000;
        if lease.expires_at_ms.0.saturating_sub(now) <= renew_margin {
            let renewed = session
                .client
                .as_mut()
                .ok_or_else(|| "The native host connection was lost.".to_owned())
                .and_then(|client| {
                    local_result(
                        client,
                        LocalCall::NativeRenew(NativeIntentTarget { intent_id: id }),
                    )
                });
            lease = match renewed {
                Ok(LocalResult::NativeLease(lease))
                    if valid_lease(&lease, id, NativeWriterState::Active)
                        && lease.fence.0 == fence
                        && lease.expires_at_ms.0 > expires_at_ms =>
                {
                    lease
                }
                _ => {
                    session.phase = NativeIntentPhase::Unknown;
                    session.client = None;
                    return Err(
                        "The canonical host could not renew the active native lease.".into(),
                    );
                }
            };
        }
        session.phase = NativeIntentPhase::Active {
            fence,
            expires_at_ms: lease.expires_at_ms.0,
            check_at: Instant::now() + Duration::from_secs(30),
        };
        Ok(())
    }

    pub(crate) fn finish_after_settlement(&self) -> Result<(), String> {
        let mut session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
        match &session.phase {
            NativeIntentPhase::Finished
            | NativeIntentPhase::BindRejected
            | NativeIntentPhase::Closed => return Ok(()),
            NativeIntentPhase::Registered => {
                // Once the durable caller marker says a bind may have been
                // sent, registration alone cannot prove a harmless stop.
                session.client = None;
                session.phase = NativeIntentPhase::Unknown;
            }
            NativeIntentPhase::Active { .. }
            | NativeIntentPhase::BindPending
            | NativeIntentPhase::Quarantined
            | NativeIntentPhase::Unknown => {}
        }

        if matches!(&session.phase, NativeIntentPhase::Active { .. }) {
            let id = session.id;
            let finished = session
                .client
                .as_mut()
                .ok_or_else(|| "The native host connection was lost.".to_owned())
                .and_then(|client| {
                    local_result(
                        client,
                        LocalCall::NativeRequestFinish(NativeIntentTarget { intent_id: id }),
                    )
                });
            match finished {
                Ok(LocalResult::NativeLease(lease))
                    if valid_lease(&lease, id, NativeWriterState::Finished) =>
                {
                    session.client = None;
                    session.phase = NativeIntentPhase::Finished;
                    return Ok(());
                }
                _ => {
                    session.phase = NativeIntentPhase::Unknown;
                    session.client = None;
                }
            }
        } else {
            // Reconcile is explicitly a fresh-connection operation.
            session.client = None;
        }

        let id = session.id;
        let profile = session.profile.clone();
        let mut client = match connect_host(&profile) {
            Ok(client) => client,
            Err(_) => {
                session.phase = NativeIntentPhase::Unknown;
                return Err("The canonical host could not reconcile native completion.".into());
            }
        };
        let reconciled = local_result(
            &mut client,
            LocalCall::NativeReconcile(NativeIntentTarget { intent_id: id }),
        );
        match reconciled {
            Ok(LocalResult::NativeLease(lease))
                if valid_lease(&lease, id, NativeWriterState::Finished) =>
            {
                session.phase = NativeIntentPhase::Finished;
                session.client = None;
                Ok(())
            }
            _ => {
                session.phase = NativeIntentPhase::Unknown;
                session.client = None;
                Err("Independent native completion evidence remains unknown; project writes stay fenced.".into())
            }
        }
    }

    fn requires_recovery(&self) -> bool {
        let Ok(session) = self.inner.lock() else {
            return true;
        };
        matches!(
            &session.phase,
            NativeIntentPhase::BindPending
                | NativeIntentPhase::Active { .. }
                | NativeIntentPhase::Quarantined
                | NativeIntentPhase::Unknown
        )
    }

    /// Reconcile a recovered intent through a fresh Host connection. A
    /// missing row or transport error is deliberately left Unknown.
    pub(crate) fn reconcile_recovered(&self) -> Result<NativeWriterState, String> {
        let (id, profile) = {
            let mut session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
            if matches!(&session.phase, NativeIntentPhase::Finished) {
                return Err("The native Host intent is not a recovered unresolved lease.".into());
            }
            session.client = None;
            (session.id, session.profile.clone())
        };
        let reconciled = connect_host(&profile).and_then(|mut client| {
            local_result(
                &mut client,
                LocalCall::NativeReconcile(NativeIntentTarget { intent_id: id }),
            )
        });
        let state = match reconciled {
            Ok(LocalResult::NativeLease(lease)) if lease.intent_id == id => lease.state,
            _ => {
                if let Ok(mut session) = self.inner.lock() {
                    session.phase = NativeIntentPhase::Unknown;
                    session.client = None;
                }
                return Err("The canonical Host could not reconcile the native writer.".into());
            }
        };
        let mut session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
        session.phase = match state {
            NativeWriterState::Finished => NativeIntentPhase::Finished,
            NativeWriterState::Quarantine => NativeIntentPhase::Quarantined,
            NativeWriterState::Registered | NativeWriterState::Active => NativeIntentPhase::Unknown,
        };
        session.client = None;
        Ok(state)
    }

    pub(crate) fn recovery_reconcile_needed(&self) -> bool {
        self.inner.lock().is_ok_and(|session| {
            matches!(
                &session.phase,
                NativeIntentPhase::Registered
                    | NativeIntentPhase::BindPending
                    | NativeIntentPhase::Active { .. }
                    | NativeIntentPhase::Quarantined
                    | NativeIntentPhase::Unknown
            )
        })
    }

    pub(crate) fn definitively_unbound(&self) -> bool {
        let Ok(session) = self.inner.lock() else {
            return false;
        };
        matches!(
            &session.phase,
            NativeIntentPhase::Registered
                | NativeIntentPhase::BindRejected
                | NativeIntentPhase::Closed
        )
    }

    pub(crate) fn host_finished_observed(&self) -> bool {
        let Ok(session) = self.inner.lock() else {
            return false;
        };
        matches!(
            &session.phase,
            NativeIntentPhase::Finished | NativeIntentPhase::BindRejected
        )
    }

    pub(crate) fn bind_rejected_observed(&self) -> bool {
        let Ok(session) = self.inner.lock() else {
            return false;
        };
        matches!(&session.phase, NativeIntentPhase::BindRejected)
    }

    pub(crate) fn confirm_bind_rejected_from_durable_proof(&self) -> Result<(), String> {
        let mut session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
        match &session.phase {
            NativeIntentPhase::BindRejected => Ok(()),
            NativeIntentPhase::Registered
            | NativeIntentPhase::BindPending
            | NativeIntentPhase::Quarantined
            | NativeIntentPhase::Unknown
            | NativeIntentPhase::Closed => {
                session.client = None;
                session.phase = NativeIntentPhase::BindRejected;
                Ok(())
            }
            NativeIntentPhase::Finished => {
                session.client = None;
                session.phase = NativeIntentPhase::BindRejected;
                Ok(())
            }
            NativeIntentPhase::Active { .. } => Err(
                "The native Host handle conflicts with its durable bind rejection proof.".into(),
            ),
        }
    }

    pub(crate) fn locally_unbound(&self) -> bool {
        let Ok(session) = self.inner.lock() else {
            return false;
        };
        matches!(
            &session.phase,
            NativeIntentPhase::Registered | NativeIntentPhase::Closed
        )
    }

    pub(crate) fn close_confirmed_prebind(&self) -> Result<(), String> {
        let mut session = self.inner.lock().map_err(|_| UNAVAILABLE)?;
        match &session.phase {
            NativeIntentPhase::Registered | NativeIntentPhase::Unknown => {
                session.client = None;
                session.phase = NativeIntentPhase::Closed;
                Ok(())
            }
            NativeIntentPhase::Closed => Ok(()),
            NativeIntentPhase::BindPending
            | NativeIntentPhase::Quarantined
            | NativeIntentPhase::Active { .. }
            | NativeIntentPhase::Finished
            | NativeIntentPhase::BindRejected => {
                Err("The native Host intent is not in a source-confirmed pre-bind state.".into())
            }
        }
    }

    fn close_if_unbound(&self) {
        if let Ok(mut session) = self.inner.lock() {
            if matches!(&session.phase, NativeIntentPhase::Registered) {
                session.client = None;
                session.phase = NativeIntentPhase::Closed;
            }
        }
    }

    fn disconnect_without_completion(&self) {
        if let Ok(mut session) = self.inner.lock() {
            match &session.phase {
                NativeIntentPhase::BindPending | NativeIntentPhase::Active { .. } => {
                    session.phase = NativeIntentPhase::Unknown;
                    session.client = None;
                }
                NativeIntentPhase::Registered
                | NativeIntentPhase::Quarantined
                | NativeIntentPhase::BindRejected
                | NativeIntentPhase::Unknown
                | NativeIntentPhase::Finished
                | NativeIntentPhase::Closed => {
                    session.client = None;
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn native_intent_id(operation_id: &str) -> Result<NativeIntentId, String> {
    if operation_id.len() != 32 || !operation_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("The durable native Attempt operation ID is not an admission ID.".into());
    }
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&operation_id[index * 2..index * 2 + 2], 16)
            .map_err(|_| "The durable native Attempt operation ID is invalid.".to_owned())?;
    }
    NativeIntentId::from_bytes(bytes)
        .map_err(|_| "The durable native Attempt operation ID is invalid.".to_owned())
}

#[cfg(target_os = "macos")]
fn canonical_host_profile() -> Result<PathBuf, String> {
    let profile = lomi_platform::default_profile()
        .ok_or("The canonical local Host profile is unavailable.")?;
    if !profile.is_absolute()
        || profile.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err("The canonical local Host profile path is invalid.".into());
    }
    Ok(profile)
}

#[cfg(target_os = "macos")]
fn connect_host(profile: &Path) -> Result<lomi_platform::unix::LocalClient, String> {
    crate::shared_host::connect_or_start_local_host(profile)
}

#[cfg(target_os = "macos")]
fn local_result(
    client: &mut lomi_platform::unix::LocalClient,
    call: LocalCall,
) -> Result<LocalResult, String> {
    local_response(client, call)?
        .map_err(|_| "The canonical local Host refused the native lease request.".into())
}

#[cfg(target_os = "macos")]
fn local_response(
    client: &mut lomi_platform::unix::LocalClient,
    call: LocalCall,
) -> Result<Result<LocalResult, lomi_remote_protocol::ipc::LocalError>, String> {
    client
        .request(call)
        .map(|response| response.result)
        .map_err(|_| "The canonical local Host IPC request failed.".into())
}

#[cfg(target_os = "macos")]
fn valid_lease(lease: &NativeLease, id: NativeIntentId, expected: NativeWriterState) -> bool {
    lease.intent_id == id
        && lease.state == expected
        && (expected != NativeWriterState::Active
            || lease.fence.0 > 0
                && lease.expires_at_ms.0 > 0
                && lease.expires_at_ms.0 > unix_time_ms().unwrap_or(u64::MAX))
}

#[cfg(target_os = "macos")]
fn unix_time_ms() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System time cannot qualify the native lease.".to_owned())?
        .as_millis();
    u64::try_from(millis).map_err(|_| "System time exceeds the native lease clock.".to_owned())
}
impl Registry {
    fn token(&mut self) -> Result<u64, String> {
        self.sequence = self.sequence.checked_add(1).ok_or(UNAVAILABLE)?;
        Ok(self.sequence)
    }
    fn check_roots(&self) -> Result<(), String> {
        // Namespace drift cannot be safely attributed to one new pathname. Keep
        // all Lomi mutations fenced until the owning run is explicitly stopped.
        for active in self.leases.values() {
            active.root.check()?;
        }
        Ok(())
    }
}
pub(crate) struct Lease {
    registry: Shared,
    token: u64,
    root: Arc<Root>,
    run: String,
    generation: u64,
    #[cfg(target_os = "macos")]
    native_intent: Option<NativeIntentHandle>,
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
pub(crate) struct NativeIntentHandle {
    operation_id: String,
    inner: Arc<Mutex<NativeIntentSession>>,
}

#[cfg(target_os = "macos")]
impl std::fmt::Debug for NativeIntentHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeIntentHandle")
            .field("operation_id", &self.operation_id)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "macos")]
struct NativeIntentSession {
    id: NativeIntentId,
    profile: PathBuf,
    client: Option<lomi_platform::unix::LocalClient>,
    phase: NativeIntentPhase,
}

#[cfg(target_os = "macos")]
enum NativeIntentPhase {
    Registered,
    BindPending,
    Active {
        fence: u64,
        expires_at_ms: u64,
        check_at: Instant,
    },
    Unknown,
    Quarantined,
    Finished,
    BindRejected,
    Closed,
}

#[cfg(target_os = "macos")]
type NativeIntentRegistry = Arc<Mutex<HashMap<String, NativeIntentHandle>>>;
#[cfg(target_os = "macos")]
static NATIVE_INTENTS: OnceLock<NativeIntentRegistry> = OnceLock::new();

#[cfg(target_os = "macos")]
fn native_intents() -> NativeIntentRegistry {
    NATIVE_INTENTS
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}
pub(crate) struct WriteAdmission {
    registry: Shared,
    token: u64,
}

/// Blocks all local writes and managed project leases while a persisted Host
/// writer participant remains unresolved.
pub(crate) struct RecoveryCeiling {
    registry: Shared,
}

impl Drop for RecoveryCeiling {
    fn drop(&mut self) {
        if let Ok(mut state) = self.registry.lock() {
            state.recovery_ceilings = state.recovery_ceilings.saturating_sub(1);
        }
    }
}

pub(crate) fn install_recovery_ceiling() -> Result<RecoveryCeiling, String> {
    install_recovery_ceiling_in(shared())
}
fn install_recovery_ceiling_in(registry: Shared) -> Result<RecoveryCeiling, String> {
    let mut state = registry.lock().map_err(|_| UNAVAILABLE)?;
    state.recovery_ceilings = state.recovery_ceilings.checked_add(1).ok_or(UNAVAILABLE)?;
    drop(state);
    Ok(RecoveryCeiling { registry })
}

impl Drop for Lease {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(intent) = &self.native_intent {
            if intent.requires_recovery() {
                // A dropped Rust owner is not evidence that the native job
                // stopped. Leave the process-local fence and intent lookup in
                // place; the host lease will expire into quarantine.
                intent.disconnect_without_completion();
                return;
            }
            let registry = native_intents();
            if let Ok(mut entries) = registry.lock() {
                if entries
                    .get(&intent.operation_id)
                    .is_some_and(|registered| Arc::ptr_eq(&registered.inner, &intent.inner))
                {
                    entries.remove(&intent.operation_id);
                }
            };
        };
        if let Ok(mut state) = self.registry.lock() {
            state.leases.remove(&self.token);
        }
    }
}
impl Drop for WriteAdmission {
    fn drop(&mut self) {
        if let Ok(mut state) = self.registry.lock() {
            state.writes.remove(&self.token);
        }
    }
}
pub(crate) fn activate(root: &Path, run: &str, generation: u64) -> Result<Lease, String> {
    activate_in(shared(), root, run, generation)
}
fn activate_in(registry: Shared, path: &Path, run: &str, generation: u64) -> Result<Lease, String> {
    if run.is_empty() || run.len() > 256 || generation == 0 {
        return Err("Invalid coding lease owner.".into());
    }
    let mut state = registry.lock().map_err(|_| UNAVAILABLE)?;
    if state.recovery_ceilings > 0 {
        return Err(STOP_RUN.into());
    }
    state.check_roots()?;
    let root = Arc::new(Root::open(path)?);
    if state.leases.values().any(|active| {
        overlap(&root.path, &active.root.path)
            || root.ancestry.contains(&active.root.identity)
            || active.root.ancestry.contains(&root.identity)
    }) {
        return Err(STOP_RUN.into());
    }
    if state.writes.values().any(|scopes| {
        scopes
            .as_ref()
            .is_none_or(|scopes| scopes.iter().any(|scope| scope.overlaps(&root)))
    }) {
        return Err(BUSY.into());
    }
    let token = state.token()?;
    state.leases.insert(
        token,
        ActiveLease {
            root: root.clone(),
            run: run.into(),
            generation,
        },
    );
    drop(state);
    Ok(Lease {
        registry,
        token,
        root,
        run: run.into(),
        generation,
        #[cfg(target_os = "macos")]
        native_intent: None,
    })
}

/// Install the process-local project fence for a managed native Attempt.
/// Shared Host participation is an optional additional protocol; the local
/// project lease remains authoritative for Lomi-owned writes in either mode.
pub(crate) fn activate_attempt(root: &Path, run: &str, generation: u64) -> Result<Lease, String> {
    #[cfg(target_os = "macos")]
    {
        activate_attempt_in(shared(), root, run, generation)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (root, run, generation);
        Err("Managed native task execution requires the supported macOS host boundary.".into())
    }
}
fn activate_attempt_in(
    registry: Shared,
    root: &Path,
    run: &str,
    generation: u64,
) -> Result<Lease, String> {
    activate_in(registry, root, run, generation)
}

#[cfg(target_os = "macos")]
pub(crate) fn recovered_native_intent(operation_id: &str) -> Result<NativeIntentHandle, String> {
    let registry = native_intents();
    if let Some(existing) = registry
        .lock()
        .map_err(|_| UNAVAILABLE)?
        .get(operation_id)
        .cloned()
    {
        return Ok(existing);
    }
    let recovered = NativeIntentHandle::recovered(operation_id)?;
    let mut entries = registry.lock().map_err(|_| UNAVAILABLE)?;
    Ok(entries
        .entry(operation_id.to_owned())
        .or_insert(recovered)
        .clone())
}

#[cfg(target_os = "macos")]
pub(crate) fn acknowledge_native_intent(operation_id: &str) -> Result<(), String> {
    let id = native_intent_id(operation_id)?;
    let profile = canonical_host_profile()?;
    let mut client = connect_host(&profile)?;
    match local_result(
        &mut client,
        LocalCall::NativeAcknowledge(NativeIntentTarget { intent_id: id }),
    )? {
        LocalResult::NativeAcknowledged(acknowledged) if acknowledged == id => Ok(()),
        _ => Err("The canonical Host did not acknowledge the exact native writer ID.".into()),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn native_intent_for_parent(operation_id: &str) -> Option<NativeIntentHandle> {
    native_intents().lock().ok()?.get(operation_id).cloned()
}

#[cfg(target_os = "macos")]
pub(crate) fn forget_native_intent(operation_id: &str) {
    if let Ok(mut intents) = native_intents().lock() {
        intents.remove(operation_id);
    }
}
/// Hold this guard across all staging, replacement, rename and durability I/O.
pub(crate) fn admit(paths: &[&Path]) -> Result<WriteAdmission, String> {
    admit_in(shared(), paths, None, false)
}
/// Broker operations with opaque destinations conservatively fence every project.
pub(crate) fn admit_unscoped() -> Result<WriteAdmission, String> {
    admit_in(shared(), &[], None, true)
}
/// Call while holding the native runtime owner, before committing a new
/// attempt. Its active entry then prevents later MCP effect admission until
/// the worker acquires the retained project lease.
pub(crate) fn ensure_no_unscoped_effects() -> Result<(), String> {
    let registry = shared();
    let state = registry.lock().map_err(|_| UNAVAILABLE)?;
    if state.writes.values().any(Option::is_none) {
        return Err(BUSY.into());
    }
    Ok(())
}
fn admit_in(
    registry: Shared,
    paths: &[&Path],
    owner: Option<u64>,
    unscoped: bool,
) -> Result<WriteAdmission, String> {
    if !unscoped && (paths.is_empty() || paths.len() > 128) {
        return Err("Invalid file write admission scope.".into());
    }
    let mut state = registry.lock().map_err(|_| UNAVAILABLE)?;
    if state.recovery_ceilings > 0 {
        return Err(STOP_RUN.into());
    }
    state.check_roots()?;
    let scopes = if unscoped {
        None
    } else {
        Some(
            paths
                .iter()
                .map(|path| Scope::resolve(path))
                .collect::<Result<Vec<_>, _>>()?,
        )
    };
    if let Some(owner) = owner {
        let active = state.leases.get(&owner).ok_or(STOP_RUN)?;
        if scopes.as_ref().is_none_or(|scopes| {
            scopes.iter().any(|scope| {
                !scope.path.starts_with(&active.root.path)
                    || !scope.directories.contains(&active.root.identity)
            })
        }) {
            return Err("The coding effect is outside its retained project lease.".into());
        }
    }
    if state.leases.iter().any(|(id, active)| {
        Some(*id) != owner
            && scopes
                .as_ref()
                .is_none_or(|scopes| scopes.iter().any(|s| s.overlaps(&active.root)))
    }) {
        return Err(STOP_RUN.into());
    }
    let token = state.token()?;
    state.writes.insert(token, scopes);
    drop(state);
    Ok(WriteAdmission { registry, token })
}
impl Lease {
    #[cfg(target_os = "macos")]
    pub(crate) fn register_native_intent(
        &mut self,
        operation_id: &str,
        store_root: &Path,
    ) -> Result<NativeIntentHandle, String> {
        if self.native_intent.is_some() {
            return Err("A Host writer intent already owns this project lease.".into());
        }
        let intent = NativeIntentHandle::register(operation_id, store_root)?;
        let registry = native_intents();
        let mut entries = registry.lock().map_err(|_| UNAVAILABLE)?;
        if entries.contains_key(operation_id) {
            return Err("A native host intent already owns this Attempt.".into());
        }
        entries.insert(operation_id.to_owned(), intent.clone());
        self.native_intent = Some(intent.clone());
        Ok(intent)
    }

    pub(crate) fn check(&self, run: &str, generation: u64) -> Result<(), String> {
        let state = self.registry.lock().map_err(|_| UNAVAILABLE)?;
        let active = state.leases.get(&self.token).ok_or(STOP_RUN)?;
        if run != self.run
            || generation != self.generation
            || active.run != run
            || active.generation != generation
        {
            return Err(STOP_RUN.into());
        }
        drop(state);
        self.root.check()?;
        #[cfg(target_os = "macos")]
        if let Some(intent) = &self.native_intent {
            intent.check_for_execution()?;
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn native_intent(&self) -> Option<NativeIntentHandle> {
        self.native_intent.clone()
    }

    pub(crate) fn close_unbound_native_intent(&self) {
        #[cfg(target_os = "macos")]
        if let Some(intent) = &self.native_intent {
            intent.close_if_unbound();
        }
    }

    pub(crate) fn requires_recovery(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.native_intent
                .as_ref()
                .is_some_and(NativeIntentHandle::requires_recovery)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn registry() -> Shared {
        Arc::new(Mutex::new(Registry::default()))
    }

    #[test]
    fn managed_attempt_lease_blocks_local_writes_and_releases_after_drop() {
        let project = tempfile::tempdir().unwrap();
        let project_path = project.path();
        let path = project_path.join("managed.txt");
        let registry = registry();

        let lease = activate_attempt_in(registry.clone(), project_path, "task-local", 1).unwrap();
        assert!(lease.check("task-local", 1).is_ok());
        assert!(admit_in(registry.clone(), &[&path], None, false).is_err());

        drop(lease);
        assert!(admit_in(registry, &[&path], None, false).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn managed_attempt_entry_point_uses_the_local_project_fence() {
        let project = tempfile::tempdir().unwrap();
        let path = project.path().join("managed.txt");

        let lease = activate_attempt(project.path(), "task-local-entry", 1).unwrap();
        assert!(admit(&[&path]).is_err());

        drop(lease);
        assert!(admit(&[&path]).is_ok());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn managed_attempt_entry_point_denies_unqualified_platforms() {
        let project = tempfile::tempdir().unwrap();
        assert!(activate_attempt(project.path(), "task-unsupported", 1).is_err());
    }

    #[test]
    fn recovery_ceiling_blocks_attempt_leases_and_local_writes() {
        let project = tempfile::tempdir().unwrap();
        let project_path = project.path();
        let path = project_path.join("managed.txt");
        let registry = registry();
        let ceiling = install_recovery_ceiling_in(registry.clone()).unwrap();

        assert!(activate_attempt_in(registry.clone(), project_path, "task-local", 1).is_err());
        assert!(admit_in(registry, &[&path], None, false).is_err());

        drop(ceiling);
    }
}
