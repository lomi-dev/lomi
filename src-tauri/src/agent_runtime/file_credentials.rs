//! Select pinned Claude's own OAuth plaintext fallback without Keychain access.
//! The trusted helper never reads/writes credentials or invokes securityd.
use std::{
    fs,
    io::{Read, Write},
    os::fd::RawFd,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const FLAG: &str = "--agent-runtime-host-file-keychain";
const ABSENT: &str = "security: The specified item could not be found in the keychain.\n";
const WRITE_UNAVAILABLE: &str =
    "security: Keychain storage is unavailable in this managed account.\n";

pub(super) fn entry() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(FLAG)) {
        return None;
    }
    Some(dispatch(
        &std::env::args().skip(2).collect::<Vec<_>>(),
        &mut std::io::stderr(),
        || drain_input(0, Duration::from_secs(2)),
    ))
}
fn dispatch(args: &[String], errors: &mut impl Write, drain: impl FnOnce() -> bool) -> i32 {
    if args.len() > 32 || args.iter().map(String::len).sum::<usize>() > 256 * 1024 {
        return 1;
    }
    let Some(command) = args.first().map(String::as_str) else {
        return 1;
    };
    match command {
        "find-generic-password" | "delete-generic-password" => {
            let _ = errors.write_all(ABSENT.as_bytes());
            44
        }
        "add-generic-password" => {
            let _ = errors.write_all(WRITE_UNAVAILABLE.as_bytes());
            1
        }
        "-i" => {
            // The byte and monotonic time bounds also cover a writer that
            // keeps stdin open. Never parse, store or echo secret bytes.
            let _ = drain();
            let _ = errors.write_all(WRITE_UNAVAILABLE.as_bytes());
            1
        }
        _ => 1,
    }
}

fn drain_input(fd: RawFd, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut bytes = [0u8; 4096];
    let mut total = 0usize;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let wait = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
        let result = unsafe { libc::poll(&mut descriptor, 1, wait) };
        if result < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        if result == 0 || descriptor.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return false;
        }
        // This early standalone entry has the sole reader of stdin. poll
        // guarantees data or EOF before the fixed-size unbuffered read.
        let count = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) };
        if count == 0 {
            return true;
        }
        if count < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        total = total.saturating_add(count as usize);
        bytes.fill(0);
        if total > 256 * 1024 {
            return false;
        }
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(super) struct FileCredentials {
    pub directory: PathBuf,
    pub reads: Vec<PathBuf>,
    worker: PathBuf,
    script: PathBuf,
    bytes: Vec<u8>,
    storage: PathBuf,
}
impl FileCredentials {
    pub(super) fn prepare(storage: &Path) -> Result<Self, String> {
        let worker = super::host_boundary::staged_worker(storage)?;
        let directory = worker
            .parent()
            .ok_or("Missing private worker directory.")?
            .to_owned();
        let script = directory.join("security");
        let bytes = format!(
            "#!/bin/sh\nexec {} {FLAG} \"$@\"\n",
            shell_quote(worker.to_str().ok_or("Worker path is not UTF-8.")?)
        )
        .into_bytes();
        if !script
            .try_exists()
            .map_err(|_| "Cannot inspect native file-store selector.")?
        {
            let staged = directory.join(format!("security-{}", super::new_id()?));
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(libc::O_NOFOLLOW)
                .mode(0o500)
                .open(&staged)
                .map_err(|_| "Cannot stage native file-store selector.")?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Cannot persist native file-store selector.")?;
            fs::rename(&staged, &script)
                .map_err(|_| "Cannot publish native file-store selector.")?;
            fs::File::open(&directory)
                .and_then(|f| f.sync_all())
                .map_err(|_| "Cannot sync native file-store selector.")?;
        }
        let value = Self {
            directory,
            reads: vec![worker.clone(), script.clone(), PathBuf::from("/bin/sh")],
            worker,
            script,
            bytes,
            storage: storage.to_owned(),
        };
        value.recheck()?;
        Ok(value)
    }
    pub(super) fn recheck(&self) -> Result<(), String> {
        if super::host_boundary::staged_worker(&self.storage)? != self.worker {
            return Err("Native file-store selector worker changed.".into());
        }
        let metadata = fs::symlink_metadata(&self.script)
            .map_err(|_| "Cannot inspect native file-store selector.")?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o777 != 0o500
            || metadata.len() != self.bytes.len() as u64
            || self
                .script
                .canonicalize()
                .map_err(|_| "Cannot resolve native file-store selector.")?
                != self.script
        {
            return Err("Native file-store selector is not trusted.".into());
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.script)
            .map_err(|_| "Cannot open native file-store selector.")?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(self.bytes.len() as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot verify native file-store selector.")?;
        if bytes != self.bytes {
            return Err("Native file-store selector changed.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_script_quotes_literal_paths_without_substitution() {
        for path in [
            "/private/tmp/worker",
            "/private/tmp/a b/c'd",
            "/private/tmp/$(exit 7)`exit 8`\"\nworker",
        ] {
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", &format!("printf '%s' {}", shell_quote(path))])
                .env_clear()
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, path.as_bytes());
        }
    }
    #[test]
    fn selector_returns_exact_native_fallback_codes_without_echoing_secrets() {
        for command in ["find-generic-password", "delete-generic-password"] {
            let mut error = Vec::new();
            assert_eq!(
                dispatch(
                    &[command.into(), "fixture-secret".into()],
                    &mut error,
                    || true
                ),
                44
            );
            assert_eq!(error, ABSENT.as_bytes());
        }
        let mut input = &b"add-generic-password -X fixture-secret\n"[..];
        let mut error = Vec::new();
        assert_eq!(
            dispatch(&["-i".into()], &mut error, || {
                std::io::copy(&mut input, &mut std::io::sink()).is_ok()
            }),
            1
        );
        assert!(input.is_empty());
        assert_eq!(error, WRITE_UNAVAILABLE.as_bytes());
        assert!(!String::from_utf8(error).unwrap().contains("fixture-secret"));
        assert_eq!(dispatch(&["unknown".into()], &mut Vec::new(), || true), 1);
    }
    #[test]
    fn input_deadline_covers_open_and_partial_command_pipes() {
        use std::os::fd::{AsRawFd, FromRawFd};
        for partial in [false, true] {
            let mut fds = [-1; 2];
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            let reader = unsafe { fs::File::from_raw_fd(fds[0]) };
            let mut writer = unsafe { fs::File::from_raw_fd(fds[1]) };
            if partial {
                writer.write_all(b"owned-fixture-secret").unwrap();
            }
            let mut error = Vec::new();
            let started = Instant::now();
            assert_eq!(
                dispatch(&["-i".into()], &mut error, || {
                    drain_input(reader.as_raw_fd(), Duration::from_millis(100))
                }),
                1
            );
            assert!(started.elapsed() >= Duration::from_millis(90));
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(error, WRITE_UNAVAILABLE.as_bytes());
            assert!(!String::from_utf8(error)
                .unwrap()
                .contains("owned-fixture-secret"));
            drop(writer);
        }
    }
}
