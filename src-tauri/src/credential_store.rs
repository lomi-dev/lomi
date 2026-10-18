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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn bounded_reader_preserves_success_missing_and_denied_results() {
        let worker = start_reader(
            |_, account, _| match account {
                "present" => Ok("fake-secret".into()),
                "missing" => Err(keyring::Error::NoEntry),
                _ => Err(reader_unavailable()),
            },
            1,
        )
        .unwrap();
        let read = |account| {
            read_with_worker(
                &worker,
                "fixture",
                account,
                1024,
                std::time::Duration::from_secs(1),
            )
        };
        assert_eq!(read("present").unwrap(), "fake-secret");
        assert!(matches!(read("missing"), Err(keyring::Error::NoEntry)));
        assert!(matches!(
            read("denied"),
            Err(keyring::Error::NoStorageAccess(_))
        ));
    }

    #[test]
    fn stalled_reader_times_out_and_caps_pending_work() {
        use std::{
            sync::{
                atomic::{AtomicUsize, Ordering},
                mpsc, Arc,
            },
            time::{Duration, Instant},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let (started, running) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let worker = start_reader(
            move |_, _, _| {
                worker_calls.fetch_add(1, Ordering::SeqCst);
                started.send(()).unwrap();
                blocked.recv().unwrap();
                Ok("fake-secret".into())
            },
            1,
        )
        .unwrap();
        let timeout = Duration::from_millis(50);
        let first_worker = worker.clone();
        let first = std::thread::spawn(move || {
            read_with_worker(&first_worker, "fixture", "blocked", 1024, timeout)
        });
        running.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(first.join().unwrap().is_err());
        let before = Instant::now();
        assert!(read_with_worker(&worker, "fixture", "queued", 1024, timeout).is_err());
        assert!(before.elapsed() < Duration::from_secs(1));
        let before = Instant::now();
        for _ in 0..32 {
            assert!(read_with_worker(&worker, "fixture", "full", 1024, timeout).is_err());
        }
        assert!(before.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        drop(worker);
        assert!(matches!(
            running.recv_timeout(Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        // The expired queued request never calls the native reader.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[ignore = "creates and locks only a temporary, isolated macOS Keychain"]
    fn locked_isolated_keychain_denies_reads_without_interaction() {
        use security_framework::os::macos::{
            keychain::CreateOptions, passwords::find_generic_password,
        };
        use std::{
            process::Command,
            time::{Duration, Instant},
        };

        let keychain_state = || {
            ["list-keychains", "default-keychain"].map(|operation| {
                let output = Command::new("/usr/bin/security")
                    .args([operation, "-d", "user"])
                    .output()
                    .unwrap();
                assert!(output.status.success());
                output.stdout
            })
        };
        let original_state = keychain_state();
        disable_keychain_ui().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lomi-no-dialog.keychain");
        let mut random = [0_u8; 24];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random).unwrap();
        let password = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut keychain = CreateOptions::new()
            .password(&password)
            .prompt_user(false)
            .create(&path)
            .unwrap();
        let service = "dev.lomi.test.locked-no-dialog";
        let account = "isolated-fixture";
        keychain
            .set_generic_password(service, account, b"isolated-secret")
            .unwrap();
        let untrusted_service = "dev.lomi.test.untrusted-no-dialog";
        let untrusted_account = "isolated-untrusted";
        let created = Command::new("/usr/bin/security")
            .args([
                "add-generic-password",
                "-a",
                untrusted_account,
                "-s",
                untrusted_service,
                "-w",
                "fake-fixture-secret",
                "-T",
                "",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(created.status.success());
        let untrusted_keychain = keychain.clone();
        let (untrusted_result, untrusted_elapsed) = std::thread::spawn(move || {
            disable_keychain_ui().unwrap();
            let started = Instant::now();
            let result = find_generic_password(
                Some(&[untrusted_keychain]),
                untrusted_service,
                untrusted_account,
            )
            .map(|_| ())
            .map_err(|error| {
                eprintln!("isolated untrusted Keychain read status: {}", error.code());
                keyring::macos::decode_error(error)
            });
            (result, started.elapsed())
        })
        .join()
        .unwrap();
        assert!(Command::new("/usr/bin/security")
            .arg("lock-keychain")
            .arg(&path)
            .status()
            .unwrap()
            .success());

        assert_eq!(keychain_state(), original_state);
        let locked = keychain.clone();
        let (result, elapsed, interaction) = std::thread::spawn(move || {
            disable_keychain_ui().unwrap();
            let started = Instant::now();
            let result = find_generic_password(Some(&[locked]), service, account)
                .map(|_| ())
                .map_err(|error| {
                    eprintln!("isolated locked Keychain read status: {}", error.code());
                    keyring::macos::decode_error(error)
                });
            (
                result,
                started.elapsed(),
                SecKeychain::user_interaction_allowed().unwrap(),
            )
        })
        .join()
        .unwrap();

        // Clean up the private fixture before asserting the denied read.
        keychain.unlock(Some(&password)).unwrap();
        assert!(Command::new("/usr/bin/security")
            .arg("delete-keychain")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        assert_eq!(keychain_state(), original_state);
        assert!(untrusted_result.is_err());
        assert!(!matches!(untrusted_result, Err(keyring::Error::NoEntry)));
        assert!(untrusted_elapsed < Duration::from_secs(3));
        assert!(result.is_err());
        assert!(!matches!(result, Err(keyring::Error::NoEntry)));
        assert!(elapsed < Duration::from_secs(3));
        assert!(!interaction);
        assert!(!SecKeychain::user_interaction_allowed().unwrap());
    }

    #[test]
    fn concurrent_entries_keep_keychain_interaction_disabled() {
        let workers: Vec<_> = (0..16)
            .map(|index| {
                std::thread::spawn(move || {
                    // Constructing an entry does not read or mutate any credentials.
                    let entry = entry("dev.lomi.test.no-keychain-ui", &index.to_string()).unwrap();
                    assert!(!SecKeychain::user_interaction_allowed().unwrap());
                    drop(entry);
                    assert!(!SecKeychain::user_interaction_allowed().unwrap());
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(!SecKeychain::user_interaction_allowed().unwrap());
    }
}

#[cfg(test)]
mod authorization_tests {
    use super::*;

    #[test]
    fn authorization_entry_requires_the_exact_fixed_arguments() {
        let args = |items: &[&str]| {
            items
                .iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        };
        assert_eq!(authorization_arguments(&args(&[])), None);
        assert_eq!(authorization_arguments(&args(&["--other"])), None);
        assert_eq!(
            authorization_arguments(&args(&[CURSOR_AUTHORIZATION_ARG])),
            Some(true)
        );
        assert_eq!(
            authorization_arguments(&args(&[CURSOR_AUTHORIZATION_ARG, "extra"])),
            Some(false)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn authorization_watchdog_exits_when_parent_dies_or_deadline_passes() {
        use std::time::Duration;
        assert!(!authorization_child_should_exit(
            42,
            42,
            Duration::from_secs(119)
        ));
        assert!(authorization_child_should_exit(
            42,
            42,
            Duration::from_secs(120)
        ));
        assert!(authorization_child_should_exit(42, 1, Duration::ZERO));
        assert!(authorization_child_should_exit(42, 43, Duration::ZERO));
        assert!(authorization_child_should_exit(1, 1, Duration::ZERO));
    }

    #[cfg(target_os = "macos")]
    fn fixture_child(program: &str, args: &[&str]) -> tokio::process::Child {
        tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }

    #[cfg(target_os = "macos")]
    fn process_exists(pid: u32) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn authorization_wait_checks_exit_status_and_reaps_timeout() {
        let timeout = std::time::Duration::from_secs(2);
        assert_eq!(
            wait_for_authorization_child(fixture_child("/usr/bin/true", &[]), timeout).await,
            Ok(true)
        );
        assert_eq!(
            wait_for_authorization_child(fixture_child("/usr/bin/false", &[]), timeout).await,
            Ok(false)
        );
        let child = fixture_child("/bin/sleep", &["30"]);
        let pid = child.id().unwrap();
        assert!(
            wait_for_authorization_child(child, std::time::Duration::from_millis(10))
                .await
                .is_err()
        );
        assert!(!process_exists(pid));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn cancelling_authorization_wait_kills_the_child() {
        let child = fixture_child("/bin/sleep", &["30"]);
        let pid = child.id().unwrap();
        let task = tokio::spawn(wait_for_authorization_child(
            child,
            std::time::Duration::from_secs(120),
        ));
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        for _ in 0..100 {
            if !process_exists(pid) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("authorization child survived cancellation");
    }

    #[test]
    fn permission_classification_excludes_missing_busy_and_oversized() {
        for error in [
            keyring::Error::NoEntry,
            keyring::Error::TooLong("fixture".into(), 1),
            keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("busy"))),
        ] {
            assert!(!is_keychain_permission_denied(&error));
        }
        #[cfg(target_os = "macos")]
        for code in [-25293, -25308, -25291, -25300, -50] {
            for error in [
                keyring::Error::NoStorageAccess(Box::new(Error::from_code(code))),
                keyring::Error::PlatformFailure(Box::new(Error::from_code(code))),
            ] {
                assert_eq!(
                    is_keychain_permission_denied(&error),
                    matches!(code, -25293 | -25308)
                );
            }
        }
    }
}
