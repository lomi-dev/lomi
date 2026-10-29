#[cfg(target_os = "macos")]
use security_framework::{
    base::Error,
    os::macos::keychain::{KeychainUserInteractionLock, SecKeychain},
};
#[cfg(target_os = "macos")]
use std::sync::OnceLock;

// Keychain interaction is process-global. Retain the guard for the entire process
// so overlapping operations cannot reenable password or access-control dialogs.
#[cfg(target_os = "macos")]
static NO_KEYCHAIN_UI: OnceLock<Result<KeychainUserInteractionLock, Error>> = OnceLock::new();

#[cfg(target_os = "macos")]
fn disable_keychain_ui() -> keyring::Result<()> {
    if let Err(error) = NO_KEYCHAIN_UI.get_or_init(SecKeychain::disable_user_interaction) {
        return Err(keyring::Error::NoStorageAccess(Box::new(*error)));
    }
    Ok(())
}

pub(crate) const CURSOR_SERVICE: &str = "cursor-access-token";
pub(crate) const CURSOR_ACCOUNT: &str = "cursor-user";
const CURSOR_AUTHORIZATION_ARG: &str = "--authorize-cursor-usage-keychain";

fn authorization_arguments(args: &[std::ffi::OsString]) -> Option<bool> {
    (args
        .first()
        .is_some_and(|arg| arg == CURSOR_AUTHORIZATION_ARG))
    .then_some(args.len() == 1)
}

// This entry point must run before any application service disables interaction.
// Only the fixed Cursor item is read; no secret is returned to the parent.
pub(crate) fn cursor_authorization_entry() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let valid = authorization_arguments(&args)?;
    if !valid {
        return Some(2);
    }
    #[cfg(target_os = "macos")]
    {
        if NO_KEYCHAIN_UI.get().is_some()
            || !matches!(SecKeychain::user_interaction_allowed(), Ok(true))
        {
            return Some(1);
        }
        if !start_authorization_watchdog() {
            return Some(1);
        }
        Some(
            match security_framework::os::macos::passwords::find_generic_password(
                None,
                CURSOR_SERVICE,
                CURSOR_ACCOUNT,
            ) {
                // Security owns this buffer and frees it on drop. Never copy it into
                // a String, IPC response, log, or process argument.
                Ok((password, item)) => {
                    drop(password);
                    drop(item);
                    0
                }
                Err(_) => 1,
            },
        )
    }
    #[cfg(not(target_os = "macos"))]
    Some(1)
}

#[cfg(target_os = "macos")]
fn authorization_child_should_exit(
    parent: libc::pid_t,
    current_parent: libc::pid_t,
    elapsed: std::time::Duration,
) -> bool {
    parent <= 1 || current_parent != parent || elapsed >= std::time::Duration::from_secs(120)
}

#[cfg(target_os = "macos")]
fn start_authorization_watchdog() -> bool {
    // Parent process::exit bypasses Rust destructors, including kill_on_drop.
    // Bound the dialog in the helper even when the GUI exits abruptly.
    let parent = unsafe { libc::getppid() };
    if parent <= 1 {
        return false;
    }
    std::thread::Builder::new()
        .name("lomi-keychain-lifetime".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            loop {
                if authorization_child_should_exit(
                    parent,
                    unsafe { libc::getppid() },
                    started.elapsed(),
                ) {
                    std::process::exit(1);
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        })
        .is_ok()
}

pub(crate) fn is_keychain_permission_denied(error: &keyring::Error) -> bool {
    #[cfg(target_os = "macos")]
    if let keyring::Error::PlatformFailure(cause) | keyring::Error::NoStorageAccess(cause) = error {
        return cause
            .downcast_ref::<Error>()
            .is_some_and(|error| matches!(error.code(), -25293 | -25308));
    }
    let _ = error;
    false
}

#[cfg(target_os = "macos")]
pub(crate) async fn authorize_cursor() -> Result<bool, &'static str> {
    let executable =
        std::env::current_exe().map_err(|_| "Cannot locate Lomi to request Keychain access.")?;
    let child = tokio::process::Command::new(executable)
        .arg(CURSOR_AUTHORIZATION_ARG)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Cannot start the Keychain access request. Try again.")?;
    wait_for_authorization_child(child, std::time::Duration::from_secs(120)).await
}

#[cfg(target_os = "macos")]
async fn wait_for_authorization_child(
    mut child: tokio::process::Child,
    timeout: std::time::Duration,
) -> Result<bool, &'static str> {
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => Ok(status.success()),
        Ok(Err(_)) => Err("The Keychain access request could not finish. Try again."),
        Err(_) => {
            let _ = child.kill().await;
            Err("The Keychain access request timed out. Try again.")
        }
    }
}

pub(crate) fn entry(service: &str, account: &str) -> keyring::Result<keyring::Entry> {
    #[cfg(target_os = "macos")]
    disable_keychain_ui()?;
    keyring::Entry::new(service, account)
}

pub(crate) fn get_password(
    service: &str,
    account: &str,
    max_bytes: usize,
) -> keyring::Result<String> {
    #[cfg(target_os = "macos")]
    {
        disable_keychain_ui()?;
        let worker = CLI_READER.get_or_init(|| start_reader(native_cli_password, 128));
        let worker = worker.as_ref().map_err(|_| reader_unavailable())?;
        read_with_worker(
            worker,
            service,
            account,
            max_bytes,
            std::time::Duration::from_secs(3),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = max_bytes;
        entry(service, account)?.get_password()
    }
}

#[cfg(target_os = "macos")]
struct ReadRequest {
    service: String,
    account: String,
    max_bytes: usize,
    deadline: std::time::Instant,
    response: std::sync::mpsc::Sender<keyring::Result<String>>,
}

#[cfg(target_os = "macos")]
static CLI_READER: OnceLock<Result<std::sync::mpsc::SyncSender<ReadRequest>, std::io::Error>> =
    OnceLock::new();

#[cfg(target_os = "macos")]
fn reader_unavailable() -> keyring::Error {
    keyring::Error::NoStorageAccess(Box::new(std::io::Error::other(
        "The system credential store is busy or unavailable.",
    )))
}

#[cfg(target_os = "macos")]
fn start_reader(
    read: impl Fn(&str, &str, usize) -> keyring::Result<String> + Send + 'static,
    capacity: usize,
) -> Result<std::sync::mpsc::SyncSender<ReadRequest>, std::io::Error> {
    // A stalled Security framework call owns the only worker. The bounded queue
    // covers the maximum CLI batch; saturation never spawns additional threads.
    let (sender, receiver) = std::sync::mpsc::sync_channel::<ReadRequest>(capacity);
    std::thread::Builder::new()
        .name("lomi-cli-keychain".into())
        .spawn(move || {
            while let Ok(request) = receiver.recv() {
                if std::time::Instant::now() >= request.deadline {
                    continue;
                }
                let result = read(&request.service, &request.account, request.max_bytes);
                let _ = request.response.send(result);
            }
        })?;
    Ok(sender)
}

#[cfg(target_os = "macos")]
fn read_with_worker(
    worker: &std::sync::mpsc::SyncSender<ReadRequest>,
    service: &str,
    account: &str,
    max_bytes: usize,
    timeout: std::time::Duration,
) -> keyring::Result<String> {
    let (response, receiver) = std::sync::mpsc::channel();
    worker
        .try_send(ReadRequest {
            service: service.into(),
            account: account.into(),
            max_bytes,
            deadline: std::time::Instant::now() + timeout,
            response,
        })
        .map_err(|_| reader_unavailable())?;
    receiver
        .recv_timeout(timeout)
        .map_err(|_| reader_unavailable())?
}

#[cfg(target_os = "macos")]
fn native_cli_password(service: &str, account: &str, max_bytes: usize) -> keyring::Result<String> {
    disable_keychain_ui()?;
    // Preserve the CLI reader's default search list, rather than limiting it
    // to the user-domain default keychain selected by keyring's Entry.
    let (password, _item) =
        security_framework::os::macos::passwords::find_generic_password(None, service, account)
            .map_err(keyring::macos::decode_error)?;
    if password.len() > max_bytes {
        return Err(keyring::Error::TooLong(
            "CLI credential".into(),
            max_bytes as u32,
        ));
    }
    keyring::error::decode_password(password.to_owned())
}
