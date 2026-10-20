use super::{auth::Authority, process_identity::Identity};
use crate::android_protocol::{
    emulator_controller_client::EmulatorControllerClient, EmulatorStatus,
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tonic::{transport::Channel, Request};

pub const MAX_PIXELS: u32 = 720 * 1280;
pub const MAX_DISPLAY_EDGE: u32 = 4096;
pub const MAX_DISPLAY_PIXELS: u32 = 3840 * 2160;
pub const MAX_SCREENSHOT_BYTES: usize = MAX_DISPLAY_PIXELS as usize * 4 + 1024 * 1024;
const MAX_MESSAGE: usize = 9 * 1024 * 1024;

/// A connection belongs to an OS process and its published random JWK directory.
/// Keep this owner in the instance, never in a React view or serialized UI state.
pub struct Connection {
    client: EmulatorControllerClient<Channel>,
    authority: Arc<Authority>,
    registration: PathBuf,
    public_key: Vec<u8>,
}

fn bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    if path
        .symlink_metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Android discovery data must not be a symbolic link".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Android discovery data exceeds its bound".into());
    }
    Ok(bytes)
}

fn publish_registration(registration: &Path, public_key: &[u8]) -> Result<(), String> {
    let parent = registration
        .parent()
        .ok_or("Missing Android JWK directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary.write_all(public_key).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    {
        // The emulator's Linux JWK watcher observes CREATE/MODIFY/DELETE, but not
        // MOVED_TO. renameat2-based persist_noclobber can therefore go unnoticed.
        // Link the complete, synced public key atomically without overwriting; an
        // unsupported hard link fails closed. Dropping the temporary removes its name.
        fs::hard_link(temporary.path(), registration).map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "linux"))]
    temporary
        .persist_noclobber(registration)
        .map_err(|e| e.to_string())?;
    Ok(())
}

impl Connection {
    /// The caller derives these values from its owned Child and native port allocator.
    /// None of the paths, endpoint or authority are accepted from a webview.
    pub async fn register(
        process: &Identity,
        discovery: &Path,
        port: u16,
        authority: Arc<Authority>,
    ) -> Result<Self, String> {
        if port < 1024 || !process.still_matches()? {
            return Err("The managed Android process changed before connection".into());
        }
        let discovery = discovery.canonicalize().map_err(|e| e.to_string())?;
        let bytes = bounded(&discovery.join(format!("pid_{}.ini", process.pid)), 65536)?;
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let field = |name: &str| -> Result<&str, String> {
            let values: Vec<_> = text
                .lines()
                .filter_map(|line| line.split_once('='))
                .filter(|(key, _)| *key == name)
                .map(|(_, value)| value.trim())
                .collect();
            match values.as_slice() {
                [value] => Ok(*value),
                _ => Err(format!(
                    "Missing or ambiguous Android discovery field {name}"
                )),
            }
        };
        if field("grpc.port")?.parse::<u16>().ok() != Some(port) {
            return Err("Android discovery port does not match the owned instance".into());
        }
        let parent = discovery.join(process.pid.to_string()).join("jwks");
        if parent
            .symlink_metadata()
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Android JWK parent must not be a symbolic link".into());
        }
        let parent = parent.canonicalize().map_err(|e| e.to_string())?;
        if !parent.starts_with(&discovery) {
            return Err("Android JWK parent escaped discovery".into());
        }
        let jwks = PathBuf::from(field("grpc.jwks")?)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if jwks.parent() != Some(parent.as_path()) {
            return Err("Android JWK directory does not belong to the owned process".into());
        }
        if !process.still_matches()? {
            return Err("Android exited during credential registration".into());
        }
        let key = authority.public_jwk();
        let id = key["keys"][0]["kid"]
            .as_str()
            .ok_or("Missing authority identity")?;
        let registration = jwks.join(format!("lomi-{id}.jwk"));
        let public_key = key.to_string().into_bytes();
        publish_registration(&registration, &public_key)?;
        // Install the cleanup owner before any fallible connection or authentication work.
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://127.0.0.1:{port}"))
            .map_err(|e| e.to_string())?
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5));
        let mut connection = Self {
            client: EmulatorControllerClient::new(endpoint.connect_lazy())
                .max_decoding_message_size(MAX_MESSAGE),
            authority,
            registration,
            public_key,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if !process.still_matches()? {
                return Err("Android exited before authentication completed".into());
            }
            if connection.status().await.is_ok() {
                return Ok(connection);
            }
            if Instant::now() >= deadline {
                return Err(
                    "Android did not accept its private credentials. Retry connection.".into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub fn request<T>(&self, method: &str, body: T) -> Result<Request<T>, String> {
        let mut request = Request::new(body);
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", self.authority.token(method)?)
                .parse()
                .map_err(|_| "Invalid Android credential metadata")?,
        );
        Ok(request)
    }

    pub fn client(&self) -> EmulatorControllerClient<Channel> {
        self.client.clone()
    }

    pub async fn status(&mut self) -> Result<EmulatorStatus, String> {
        let request = self.request("getStatus", ())?;
        self.client
            .get_status(request)
            .await
            .map(|reply| reply.into_inner())
            .map_err(|e| e.to_string())
    }

    pub fn display(status: &EmulatorStatus) -> Result<(u32, u32), String> {
        // The pinned 37.1.11 binary still returns an empty platformConfig on AOSP API 36.
        #[allow(deprecated)]
        let legacy = status.hardware_config.as_ref();
        let dimension = |name: &str| {
            status
                .platform_config
                .get(name)
                .or_else(|| {
                    legacy
                        .and_then(|config| config.entry.iter().find(|entry| entry.key == name))
                        .map(|entry| &entry.value)
                })
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|value| (1..=MAX_DISPLAY_EDGE).contains(value))
        };
        let width = dimension("hw.lcd.width").ok_or("The Android display width is unsupported")?;
        let height =
            dimension("hw.lcd.height").ok_or("The Android display height is unsupported")?;
        if width * height > MAX_DISPLAY_PIXELS {
            return Err("This phone exceeds the supported hardware display size. Choose another phone profile.".into());
        }
        Ok((width, height))
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Never remove a replacement belonging to another instance or a user's repair.
        if bounded(&self.registration, 65536).is_ok_and(|bytes| bytes == self.public_key) {
            let _ = fs::remove_file(&self.registration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn public_key() -> Vec<u8> {
        Authority::new("00000000-0000-0000-0000-000000000001".into())
            .unwrap()
            .public_jwk()
            .to_string()
            .into_bytes()
    }

    #[test]
    fn registration_publishes_only_the_complete_public_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("final.jwk");
        let key = public_key();
        publish_registration(&path, &key).unwrap();
        assert_eq!(fs::read(&path).unwrap(), key);
        let value: serde_json::Value = serde_json::from_slice(&key).unwrap();
        assert_eq!(value["keys"][0]["kty"], "EC");
        assert!(value["keys"][0].get("d").is_none());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn registration_does_not_overwrite_or_leak_on_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("final.jwk");
        let existing = b"existing registration";
        fs::write(&path, existing).unwrap();
        assert!(publish_registration(&path, &public_key()).is_err());
        assert_eq!(fs::read(&path).unwrap(), existing);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(publish_registration(&path, &public_key()).is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn registration_notifies_the_emulators_linux_jwk_watcher() {
        use std::{
            ffi::CString,
            os::{
                fd::{AsRawFd, FromRawFd, OwnedFd},
                unix::ffi::OsStrExt,
            },
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("final.jwk");
        let key = public_key();
        let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(
            raw >= 0,
            "inotify_init1: {}",
            std::io::Error::last_os_error()
        );
        // The owned descriptor removes the inotify watch on every exit/panic path.
        let watcher = unsafe { OwnedFd::from_raw_fd(raw) };
        let name = CString::new(directory.path().as_os_str().as_bytes()).unwrap();
        let mask = libc::IN_CREATE | libc::IN_MODIFY | libc::IN_DELETE;
        let watch = unsafe { libc::inotify_add_watch(watcher.as_raw_fd(), name.as_ptr(), mask) };
        assert!(
            watch >= 0,
            "inotify_add_watch: {}",
            std::io::Error::last_os_error()
        );
        publish_registration(&path, &key).unwrap();
        let mut ready = libc::pollfd {
            fd: watcher.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(
            unsafe { libc::poll(&mut ready, 1, 2000) },
            1,
            "No JWK watcher event"
        );
        let mut bytes = [0u8; 8192];
        let length =
            unsafe { libc::read(watcher.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
        assert!(
            length > 0,
            "inotify read: {}",
            std::io::Error::last_os_error()
        );
        let mut offset = 0;
        let mut published = false;
        while offset + std::mem::size_of::<libc::inotify_event>() <= length as usize {
            // inotify records need not align with a Rust byte buffer's address.
            let event = unsafe {
                std::ptr::read_unaligned(bytes.as_ptr().add(offset).cast::<libc::inotify_event>())
            };
            let start = offset + std::mem::size_of::<libc::inotify_event>();
            let end = start + event.len as usize;
            assert!(end <= length as usize);
            let name = bytes[start..end].split(|byte| *byte == 0).next().unwrap();
            if event.wd == watch
                && name == b"final.jwk"
                && event.mask & (libc::IN_CREATE | libc::IN_MODIFY) != 0
            {
                let observed = fs::read(&path).unwrap();
                assert_eq!(observed, key);
                let value: serde_json::Value = serde_json::from_slice(&observed).unwrap();
                assert_eq!(value["keys"][0]["crv"], "P-256");
                published = true;
            }
            offset = end;
        }
        assert!(
            published,
            "The emulator's watcher did not observe the complete final JWK"
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn hardware_display_is_independent_of_the_stream_budget() {
        let mut status = EmulatorStatus::default();
        status
            .platform_config
            .insert("hw.lcd.width".into(), "720".into());
        status
            .platform_config
            .insert("hw.lcd.height".into(), "1280".into());
        assert_eq!(Connection::display(&status).unwrap(), (720, 1280));
        status
            .platform_config
            .insert("hw.lcd.width".into(), "1080".into());
        status
            .platform_config
            .insert("hw.lcd.height".into(), "1920".into());
        assert_eq!(Connection::display(&status).unwrap(), (1080, 1920));
        status
            .platform_config
            .insert("hw.lcd.width".into(), "1344".into());
        status
            .platform_config
            .insert("hw.lcd.height".into(), "2992".into());
        assert_eq!(Connection::display(&status).unwrap(), (1344, 2992));
        status
            .platform_config
            .insert("hw.lcd.width".into(), "4096".into());
        status
            .platform_config
            .insert("hw.lcd.height".into(), "4096".into());
        assert!(Connection::display(&status).is_err());
        status
            .platform_config
            .insert("hw.lcd.width".into(), "4294967295".into());
        assert!(Connection::display(&status).is_err());
    }
}
