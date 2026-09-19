use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

const LIMIT: usize = 1024 * 1024;
const VERSION: &str = "0029";
const INPUT_METHOD: &str = "org.simplebench.input/.SimpleBenchInput";

struct Connection {
    socket: TcpStream,
    deadline: Instant,
    cancel: Option<Arc<AtomicBool>>,
}
impl Connection {
    fn remaining(&self) -> std::io::Result<Duration> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Acquire))
        {
            return Err(std::io::Error::other("Android operation cancelled"));
        }
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "ADB operation deadline exceeded",
                )
            })
    }
}
impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let remaining = self.remaining()?;
            self.socket
                .set_read_timeout(Some(if self.cancel.is_some() {
                    remaining.min(Duration::from_millis(250))
                } else {
                    remaining
                }))?;
            match self.socket.read(buffer) {
                Err(error)
                    if self.cancel.is_some()
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                {
                    continue
                }
                result => return result,
            }
        }
    }
}
impl Write for Connection {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        loop {
            let remaining = self.remaining()?;
            self.socket
                .set_write_timeout(Some(if self.cancel.is_some() {
                    remaining.min(Duration::from_millis(250))
                } else {
                    remaining
                }))?;
            match self.socket.write(bytes) {
                Err(error)
                    if self.cancel.is_some()
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                {
                    continue
                }
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
    }
}

/// Smart-socket access deliberately never starts or replaces an ADB server.
#[derive(Clone)]
pub struct Server {
    pub port: u16,
}

#[derive(Clone)]
pub struct Guest {
    pub server: Server,
    pub console_port: u16,
    pub device_id: String,
    pub generation_key: String,
}

fn error(error: impl std::fmt::Display) -> String {
    format!("Android ADB: {error}")
}

impl Server {
    fn connect(&self) -> Result<Connection, String> {
        let socket =
            TcpStream::connect_timeout(&([127, 0, 0, 1], self.port).into(), Duration::from_secs(2))
                .map_err(error)?;
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(error)?;
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(error)?;
        Ok(Connection {
            socket,
            deadline: Instant::now() + Duration::from_secs(5),
            cancel: None,
        })
    }

    pub fn preflight(&self) -> Result<(), String> {
        let mut socket = self.connect()?;
        request(&mut socket, "host:version")?;
        let version = read_string(&mut socket, 4)?;
        if version != VERSION {
            return Err(format!("ADB server protocol {version} is incompatible with the managed tools (expected {VERSION}). Stop or update it in the application that owns it, then retry. SimpleBench did not replace it."));
        }
        Ok(())
    }

    fn select(&self, port: u16) -> Result<Connection, String> {
        if !(5554..=5682).contains(&port) || !port.is_multiple_of(2) {
            return Err("Invalid managed emulator port".into());
        }
        self.preflight()?;
        let mut socket = self.connect()?;
        request(&mut socket, &format!("host:transport:emulator-{port}"))?;
        // A server replacement after preflight cannot trigger host:kill. Guest operations
        // verify device and generation on this selected transport before doing any work.
        Ok(socket)
    }
}

impl Guest {
    #[cfg(any(test, feature = "android-probe"))]
    pub fn native_input_fixture(&self, open: bool) -> Result<String, String> {
        self.shell(&(self.guard()? + if open { "am start -n org.simplebench.inputtest/.InputTest" } else { "uiautomator dump /data/local/tmp/simplebench-input-check.xml >/dev/null && cat /data/local/tmp/simplebench-input-check.xml && rm /data/local/tmp/simplebench-input-check.xml" }))
    }
    #[cfg(any(test, feature = "android-probe"))]
    pub fn native_runtime_marker(&self, write: Option<&str>) -> Result<String, String> {
        let command = match write {
            Some(value) if super::storage::valid_id(value) => format!("printf '%s' '{value}' > /data/local/tmp/simplebench-runtime-marker; cat /data/local/tmp/simplebench-runtime-marker"),
            Some(_) => return Err("Invalid native fixture marker".into()),
            None => "cat /data/local/tmp/simplebench-runtime-marker".into(),
        };
        self.shell(&(self.guard()? + &command))
    }

    fn identity(&self) -> Result<(), String> {
        if !super::storage::valid_id(&self.device_id)
            || !super::storage::valid_id(&self.generation_key)
        {
            return Err("Invalid managed Android identity".into());
        }
        Ok(())
    }

    fn guard(&self) -> Result<String, String> {
        self.identity()?;
        Ok(format!("[ \"$(getprop ro.boot.simplebench.device)\" = '{}' ] && [ \"$(settings get global simplebench_generation)\" = '{}' ] || exit 77; ", self.device_id, self.generation_key))
    }

    pub fn claim_generation(&self) -> Result<(), String> {
        self.identity()?;
        let command = format!("[ \"$(getprop ro.boot.simplebench.device)\" = '{}' ] || exit 77; settings put global simplebench_device '{}'; settings put global simplebench_generation '{}'", self.device_id, self.device_id, self.generation_key);
        self.shell(&command).map(|_| ())
    }

    pub fn booted(&self) -> Result<bool, String> {
        self.shell(&(self.guard()? + "getprop sys.boot_completed"))
            .map(|text| text.trim() == "1")
    }

    fn wait_for_input_registration(
        &self,
        cancel: &Arc<AtomicBool>,
        deadline: Instant,
    ) -> Result<(), String> {
        let command = self.guard()? + "ime list -a -s";
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Android input setup cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("Android has not registered the SimpleBench keyboard after installation. Stop and Start the phone to retry input setup; its apps and data are preserved.".into());
            }
            let methods = self.shell_until(&command, deadline, Some(cancel))?;
            if methods.lines().any(|method| method.trim() == INPUT_METHOD) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn enable_input(&self, cancel: &Arc<AtomicBool>) -> Result<(), String> {
        let guard = self.guard()?;
        // Package installation can finish before InputMethodManager processes its
        // package-change notification. Poll its registry before enabling the IME.
        self.wait_for_input_registration(cancel, Instant::now() + Duration::from_secs(15))?;
        self.shell_until(
            &(guard.clone() + "ime disable org.simplebench.input/.SimpleBenchInput"),
            Instant::now() + Duration::from_secs(5),
            Some(cancel),
        )?;
        // Package replacement disconnects the old service. Wait for Android's settings
        // observer before selecting the same component again, or it can keep a dead binding.
        let deadline = Instant::now() + Duration::from_secs(5);
        while self
            .shell_until(
                &(guard.clone() + "settings get secure default_input_method"),
                deadline,
                Some(cancel),
            )?
            .trim()
            == INPUT_METHOD
        {
            if Instant::now() >= deadline {
                return Err(
                    "Android has not released the previous keyboard service. Retry input setup."
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.shell_until(
            &(guard + "ime enable org.simplebench.input/.SimpleBenchInput && ime set org.simplebench.input/.SimpleBenchInput"),
            Instant::now() + Duration::from_secs(5),
            Some(cancel),
        )?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Android input setup cancelled".into());
            }
            match self.text("ping", "") {
                Ok(()) => return Ok(()),
                Err(error) if Instant::now() >= deadline => return Err(format!("The Android keyboard did not become ready: {error}. Stop and Start the phone to retry input setup.")),
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    pub fn open_settings(&self) -> Result<(), String> {
        self.shell(&(self.guard()? + "am start -a android.settings.SETTINGS"))
            .map(|_| ())
    }

    /// Requests Android's shutdown sequence; only the owner waiting on Child proves exit.
    pub fn request_shutdown(&self) -> Result<(), String> {
        let mut socket = self.server.select(self.console_port)?;
        socket.deadline = Instant::now() + Duration::from_secs(15);
        let command = self.guard()? + "sync && printf SBSD && svc power shutdown";
        request(&mut socket, &("shell,v2,raw:".to_string() + &command))?;
        let mut acknowledged = false;
        let mut output = Vec::new();
        loop {
            let mut header = [0; 5];
            match socket.read_exact(&mut header) {
                Ok(()) => {}
                // adbd exits during power-off, before it can return a shell exit packet.
                Err(error) if acknowledged && error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(());
                }
                Err(failure) => return Err(error(failure)),
            }
            let length = u32::from_le_bytes(header[1..].try_into().unwrap()) as usize;
            if length > LIMIT || output.len().saturating_add(length) > LIMIT {
                return Err("ADB shutdown output limit exceeded".into());
            }
            let mut bytes = vec![0; length];
            socket.read_exact(&mut bytes).map_err(error)?;
            match header[0] {
                1 => {
                    output.extend_from_slice(&bytes);
                    acknowledged = output.starts_with(b"SBSD");
                }
                2 => return Err(format!("Android shutdown: {}", String::from_utf8_lossy(&bytes))),
                3 if acknowledged && bytes.as_slice() == [0] => return Ok(()),
                3 => return Err("Android rejected the guarded shutdown request; keep its process handle and retry Stop".into()),
                _ => return Err("Unexpected ADB shutdown response".into()),
            }
        }
    }

    fn shell(&self, command: &str) -> Result<String, String> {
        self.shell_until(command, Instant::now() + Duration::from_secs(5), None)
    }

    fn shell_until(
        &self,
        command: &str,
        deadline: Instant,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<String, String> {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
            return Err("Android operation cancelled".into());
        }
        let mut socket = self.server.select(self.console_port)?;
        socket.deadline = deadline;
        socket.cancel = cancel.cloned();
        request(&mut socket, &("shell,v2,raw:".to_string() + command))?;
        let mut output = Vec::new();
        loop {
            let mut header = [0; 5];
            socket.read_exact(&mut header).map_err(error)?;
            let length = u32::from_le_bytes(header[1..].try_into().unwrap()) as usize;
            if length > LIMIT || output.len().saturating_add(length) > LIMIT {
                return Err("ADB output limit exceeded".into());
            }
            let mut bytes = vec![0; length];
            socket.read_exact(&mut bytes).map_err(error)?;
            match header[0] {
                1 | 2 => output.extend_from_slice(&bytes),
                3 if bytes.as_slice() == [0] => return String::from_utf8(output).map_err(error),
                3 if bytes.as_slice() == [77] => return Err("The ADB transport no longer belongs to this Android instance. Reconnect before retrying.".into()),
                3 => return Err(format!("Android command failed: {}", String::from_utf8_lossy(&output))),
                _ => return Err("Unexpected ADB shell packet".into()),
            }
        }
    }

    pub fn text(&self, operation: &str, text: &str) -> Result<(), String> {
        self.identity()?;
        if !["commit", "compose", "finish", "delete", "ping", "paste"].contains(&operation)
            || text.len() > 16384
        {
            return Err("Invalid Android text operation".into());
        }
        let mut socket = self.server.select(self.console_port)?;
        socket.deadline =
            Instant::now() + Duration::from_secs(if operation == "ping" { 2 } else { 5 });
        request(&mut socket, "localabstract:simplebench.input.v1")?;
        let data = serde_json::to_vec(&serde_json::json!({"version":1,"id":1,"deviceId":self.device_id,"generationKey":self.generation_key,"action":operation,"text":text})).map_err(error)?;
        if data.len() > 65536 {
            return Err("Encoded Android text exceeds the bridge message limit.".into());
        }
        socket
            .write_all(&(data.len() as u32).to_be_bytes())
            .map_err(error)?;
        socket.write_all(&data).map_err(error)?;
        let mut length = [0; 4];
        socket.read_exact(&mut length).map_err(error)?;
        let length = u32::from_be_bytes(length) as usize;
        if length > 4096 {
            return Err("Android input response exceeds 4 KiB".into());
        }
        let mut bytes = vec![0; length];
        socket.read_exact(&mut bytes).map_err(error)?;
        let response: serde_json::Value = serde_json::from_slice(&bytes).map_err(error)?;
        if response["version"] != 1 || response["id"] != 1 || response["ok"] != true {
            return Err(format!(
                "Android text input: {}",
                response["error"]
                    .as_str()
                    .unwrap_or("Invalid bridge response")
            ));
        }
        Ok(())
    }

    pub fn install_apk(
        &self,
        file: &mut std::fs::File,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        let metadata = file.metadata().map_err(error)?;
        if !metadata.is_file() {
            return Err("Select a regular APK file no larger than 2 GiB.".into());
        }
        self.install_bytes(file, metadata.len(), cancel)
    }

    pub fn prepare_input(&self, cancel: &Arc<AtomicBool>) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        let apk = include_bytes!("../../android-input/simplebench-input.apk");
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../android-input/artifact.json"))
                .map_err(error)?;
        let hash = format!("{:x}", Sha256::digest(apk));
        if manifest["apkSha256"].as_str() != Some(&hash) {
            return Err("The bundled Android keyboard is damaged. Reinstall SimpleBench.".into());
        }
        // The path is produced and quoted entirely inside the owned guest. No
        // frontend path, application filename or command enters this shell.
        let installed = self.shell(&(self.guard()? + "path=$(pm path org.simplebench.input); case \"$path\" in package:/data/app/*/base.apk) sha256sum \"${path#package:}\" ;; *) printf missing ;; esac"))?;
        if installed.split_whitespace().next() != Some(hash.as_str()) {
            self.install_bytes(&mut std::io::Cursor::new(apk), apk.len() as u64, cancel)?;
        }
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Android input setup cancelled".into());
        }
        self.enable_input(cancel)
    }

    fn install_bytes(
        &self,
        reader: &mut impl Read,
        length: u64,
        cancel: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        if !(4..=2 * 1024 * 1024 * 1024).contains(&length) {
            return Err("Select a regular APK file no larger than 2 GiB.".into());
        }
        let mut socket = self.server.select(self.console_port)?;
        socket.deadline = Instant::now() + Duration::from_secs(90);
        socket.cancel = Some(cancel.clone());
        // The only interpolated values are validated UUIDs and a native file size.
        // APK bytes are streamed over stdin; neither their name nor contents enter a shell.
        let command = format!(
            "exec:sh -c \"{}printf 'SBOK'; exec cmd package install -r -S {}\"",
            self.guard()?.replace('"', "\\\"").replace('$', "\\$"),
            length
        );
        request(&mut socket, &command)?;
        // Do not transmit an APK until this exact transport proves guest ownership.
        let mut ready = [0; 4];
        socket.read_exact(&mut ready).map_err(|_| {
            "Cannot verify the Android transport before APK transfer. Reconnect and retry."
                .to_string()
        })?;
        if &ready != b"SBOK" {
            return Err("Android transport rejected APK transfer.".into());
        }
        let mut remaining = length;
        let mut buffer = [0; 65536];
        while remaining > 0 {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err("Android APK installation cancelled".into());
            }
            let chunk = remaining.min(buffer.len() as u64) as usize;
            let count = reader.read(&mut buffer[..chunk]).map_err(error)?;
            if count == 0 {
                return Err("The selected APK changed during installation.".into());
            }
            socket.write_all(&buffer[..count]).map_err(error)?;
            remaining -= count as u64;
        }
        let mut output = Vec::new();
        socket.take(4097).read_to_end(&mut output).map_err(error)?;
        if output.len() > 4096
            || !String::from_utf8_lossy(&output)
                .lines()
                .any(|line| line.trim() == "Success")
        {
            return Err(format!(
                "APK installation failed: {}",
                String::from_utf8_lossy(&output)
            ));
        }
        Ok(())
    }
}

fn request(socket: &mut (impl Read + Write), service: &str) -> Result<(), String> {
    if service.len() > 65535 {
        return Err("ADB service request exceeds its limit".into());
    }
    socket
        .write_all(format!("{:04x}{service}", service.len()).as_bytes())
        .map_err(error)?;
    let mut response = [0; 4];
    socket.read_exact(&mut response).map_err(error)?;
    match &response {
        b"OKAY" => Ok(()),
        b"FAIL" => Err(format!("ADB: {}", read_string(socket, 4096)?)),
        _ => Err("Invalid ADB server response".into()),
    }
}
fn read_string(socket: &mut impl Read, limit: usize) -> Result<String, String> {
    let mut length = [0; 4];
    socket.read_exact(&mut length).map_err(error)?;
    let length =
        usize::from_str_radix(std::str::from_utf8(&length).map_err(error)?, 16).map_err(error)?;
    if length > limit {
        return Err("ADB response exceeds its limit".into());
    }
    let mut value = vec![0; length];
    socket.read_exact(&mut value).map_err(error)?;
    String::from_utf8(value).map_err(error)
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_keyboard_matches_its_reviewable_sources_and_artifact_manifest() {
        use sha2::{Digest, Sha256};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("android-input");
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../android-input/artifact.json")).unwrap();
        assert_eq!(
            manifest["apkSha256"],
            format!(
                "{:x}",
                Sha256::digest(include_bytes!("../../android-input/simplebench-input.apk"))
            )
        );
        for (path, expected) in manifest["sources"].as_object().unwrap() {
            assert_eq!(
                expected,
                &format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(root.join(path)).unwrap())
                ),
                "Rebuild the Android keyboard after changing {path}"
            );
        }
    }
    use super::*;
    use std::{net::TcpListener, thread};

    fn input_fixture(
        replies: Vec<(&'static str, &'static str, u8)>,
        ping: bool,
    ) -> (Guest, thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let guest = Guest {
            server: Server {
                port: listener.local_addr().unwrap().port(),
            },
            console_port: 5580,
            device_id: "00000000-0000-0000-0000-000000000001".into(),
            generation_key: "00000000-0000-0000-0000-000000000002".into(),
        };
        let guard = guest.guard().unwrap();
        let worker = thread::spawn(move || {
            let transport = || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                assert_eq!(read_string(&mut socket, 256).unwrap(), "host:version");
                socket.write_all(b"OKAY00040029").unwrap();
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                assert_eq!(
                    read_string(&mut socket, 256).unwrap(),
                    "host:transport:emulator-5580"
                );
                socket.write_all(b"OKAY").unwrap();
                socket
            };
            for (command, output, exit) in replies {
                let mut socket = transport();
                assert_eq!(
                    read_string(&mut socket, 2048).unwrap(),
                    format!("shell,v2,raw:{guard}{command}")
                );
                socket.write_all(b"OKAY").unwrap();
                socket.write_all(&[1]).unwrap();
                socket
                    .write_all(&(output.len() as u32).to_le_bytes())
                    .unwrap();
                socket.write_all(output.as_bytes()).unwrap();
                socket.write_all(&[3, 1, 0, 0, 0, exit]).unwrap();
            }
            if ping {
                let mut socket = transport();
                assert_eq!(
                    read_string(&mut socket, 256).unwrap(),
                    "localabstract:simplebench.input.v1"
                );
                socket.write_all(b"OKAY").unwrap();
                let mut length = [0; 4];
                socket.read_exact(&mut length).unwrap();
                let mut bytes = vec![0; u32::from_be_bytes(length) as usize];
                socket.read_exact(&mut bytes).unwrap();
                let message: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(message["action"], "ping");
                assert_eq!(message["deviceId"], "00000000-0000-0000-0000-000000000001");
                assert_eq!(
                    message["generationKey"],
                    "00000000-0000-0000-0000-000000000002"
                );
                let reply = br#"{"version":1,"id":1,"ok":true}"#;
                socket
                    .write_all(&(reply.len() as u32).to_be_bytes())
                    .unwrap();
                socket.write_all(reply).unwrap();
            }
        });
        (guest, worker)
    }

    #[test]
    fn input_setup_waits_for_registration_before_selecting_and_authenticating_keyboard() {
        let (guest, worker) = input_fixture(vec![
            ("ime list -a -s", "com.android.inputmethod.latin/.LatinIME\n", 0),
            ("ime list -a -s", "org.simplebench.input/.SimpleBenchInputOther\n", 0),
            ("ime list -a -s", "org.simplebench.input/.SimpleBenchInput\n", 0),
            ("ime disable org.simplebench.input/.SimpleBenchInput", "disabled", 0),
            ("settings get secure default_input_method", INPUT_METHOD, 0),
            ("settings get secure default_input_method", "com.android.inputmethod.latin/.LatinIME", 0),
            ("ime enable org.simplebench.input/.SimpleBenchInput && ime set org.simplebench.input/.SimpleBenchInput", "enabled and selected", 0),
        ], true);
        guest
            .enable_input(&Arc::new(AtomicBool::new(false)))
            .unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn input_registration_timeout_and_identity_failure_never_enable_keyboard() {
        for (exit, expected) in [(0, "has not registered"), (77, "no longer belongs")] {
            let (guest, worker) = input_fixture(vec![("ime list -a -s", "", exit)], false);
            let result = guest.wait_for_input_registration(
                &Arc::new(AtomicBool::new(false)),
                Instant::now() + Duration::from_millis(80),
            );
            assert!(result.unwrap_err().contains(expected));
            worker.join().unwrap();
        }
    }

    #[test]
    fn input_registration_can_be_cancelled_while_android_is_not_responding() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let guest = Guest {
            server: Server {
                port: listener.local_addr().unwrap().port(),
            },
            console_port: 5580,
            device_id: "00000000-0000-0000-0000-000000000001".into(),
            generation_key: "00000000-0000-0000-0000-000000000002".into(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let guard = guest.guard().unwrap();
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            assert_eq!(read_string(&mut socket, 256).unwrap(), "host:version");
            socket.write_all(b"OKAY00040029").unwrap();
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            assert_eq!(
                read_string(&mut socket, 256).unwrap(),
                "host:transport:emulator-5580"
            );
            socket.write_all(b"OKAY").unwrap();
            assert_eq!(
                read_string(&mut socket, 2048).unwrap(),
                format!("shell,v2,raw:{guard}ime list -a -s")
            );
            socket.write_all(b"OKAY").unwrap();
            stop.store(true, Ordering::Release);
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).unwrap(), 0);
        });
        let started = Instant::now();
        assert!(guest
            .enable_input(&cancel)
            .unwrap_err()
            .contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(2));
        worker.join().unwrap();
    }

    #[test]
    fn incompatible_server_receives_only_read_only_preflight() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let server = Server {
            port: listener.local_addr().unwrap().port(),
        };
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            assert_eq!(read_string(&mut socket, 100).unwrap(), "host:version");
            socket.write_all(b"OKAY00040028").unwrap();
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).unwrap(), 0);
        });
        assert!(server.preflight().unwrap_err().contains("incompatible"));
        worker.join().unwrap();
    }

    #[test]
    fn replacement_after_preflight_never_launches_a_client_or_retries_on_another_transport() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let server = Server {
            port: listener.local_addr().unwrap().port(),
        };
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in [
                "OKAY00040029",
                "FAIL0018Server transport changed",
                "OKAY00040028",
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                requests.push(read_string(&mut socket, 256).unwrap());
                socket.write_all(reply.as_bytes()).unwrap();
            }
            requests
        });
        let guest = Guest {
            server,
            console_port: 5580,
            device_id: "00000000-0000-0000-0000-000000000001".into(),
            generation_key: "00000000-0000-0000-0000-000000000002".into(),
        };
        assert!(guest.booted().unwrap_err().contains("transport changed"));
        assert!(guest.booted().unwrap_err().contains("incompatible"));
        assert_eq!(
            worker.join().unwrap(),
            [
                "host:version",
                "host:transport:emulator-5580",
                "host:version"
            ]
        );
    }

    #[test]
    fn shutdown_requires_same_transport_guard_and_ack_before_disconnect() {
        for (ack, exit, succeeds) in [
            (true, None, true),
            (false, None, false),
            (false, Some(77), false),
            (true, Some(1), false),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let server = Server {
                port: listener.local_addr().unwrap().port(),
            };
            let worker = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                assert_eq!(read_string(&mut socket, 256).unwrap(), "host:version");
                socket.write_all(b"OKAY00040029").unwrap();
                let (mut socket, _) = listener.accept().unwrap();
                assert_eq!(
                    read_string(&mut socket, 256).unwrap(),
                    "host:transport:emulator-5580"
                );
                socket.write_all(b"OKAY").unwrap();
                let command = read_string(&mut socket, 1024).unwrap();
                assert!(
                    command.starts_with("shell,v2,raw:[ \"$(getprop ro.boot.simplebench.device)\"")
                );
                assert!(command.contains("settings get global simplebench_generation"));
                assert!(command.ends_with("|| exit 77; sync && printf SBSD && svc power shutdown"));
                socket.write_all(b"OKAY").unwrap();
                if ack {
                    socket.write_all(b"\x01\x04\x00\x00\x00SBSD").unwrap();
                }
                if let Some(code) = exit {
                    socket.write_all(&[3, 1, 0, 0, 0, code]).unwrap();
                }
            });
            let guest = Guest {
                server,
                console_port: 5580,
                device_id: "00000000-0000-0000-0000-000000000001".into(),
                generation_key: "00000000-0000-0000-0000-000000000002".into(),
            };
            assert_eq!(guest.request_shutdown().is_ok(), succeeds);
            worker.join().unwrap();
        }
    }

    #[test]
    #[ignore = "Powers off only the isolated SIMPLEBENCH_ANDROID_PROBE_DIRECTORY fixture"]
    fn native_guarded_shutdown() {
        let root =
            std::path::PathBuf::from(std::env::var("SIMPLEBENCH_ANDROID_PROBE_DIRECTORY").unwrap());
        let consent: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("evidence/consent.json")).unwrap())
                .unwrap();
        assert_eq!(consent["accepted"], true);
        let guest = Guest {
            server: Server { port: 15037 },
            console_port: 5580,
            device_id: "00000000-0000-0000-0000-000000000001".into(),
            generation_key: "00000000-0000-0000-0000-000000000002".into(),
        };
        guest.claim_generation().unwrap();
        assert!(guest.booted().unwrap());
        guest.shell(&(guest.guard().unwrap() + "printf simplebench-rust-shutdown > /data/local/tmp/simplebench-rust-shutdown")).unwrap();
        guest.request_shutdown().unwrap();
        std::fs::write(root.join("evidence/native-rust-shutdown.json"),
            b"{\"requested\":true,\"exitVerified\":false,\"marker\":\"/data/local/tmp/simplebench-rust-shutdown\"}").unwrap();
    }

    #[test]
    #[ignore = "Requires SIMPLEBENCH_ANDROID_PROBE_DIRECTORY and the owned native fixture"]
    fn native_transport_identity_and_install() {
        let root = std::path::PathBuf::from(
            std::env::var("SIMPLEBENCH_ANDROID_PROBE_DIRECTORY")
                .expect("Start the isolated Android probe first"),
        );
        let consent: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("evidence/consent.json")).unwrap())
                .unwrap();
        assert_eq!(consent["accepted"], true);
        let guest = Guest {
            server: Server { port: 15037 },
            console_port: 5580,
            device_id: "00000000-0000-0000-0000-000000000001".into(),
            generation_key: "00000000-0000-0000-0000-000000000002".into(),
        };
        guest.claim_generation().unwrap();
        assert!(guest.booted().unwrap());
        for name in ["input.apk", "input-test.apk"] {
            let mut file =
                std::fs::File::open(root.join("development-tools/input-build").join(name)).unwrap();
            guest
                .install_apk(&mut file, &Arc::new(AtomicBool::new(false)))
                .unwrap();
        }
        guest
            .enable_input(&Arc::new(AtomicBool::new(false)))
            .unwrap();
        guest.open_settings().unwrap();
        guest
            .shell(&(guest.guard().unwrap() + "am start -n org.simplebench.inputtest/.InputTest"))
            .unwrap();
        std::thread::sleep(Duration::from_secs(1));
        guest
            .shell(&(guest.guard().unwrap() + "input tap 400 300"))
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        guest.text("commit", "Zażółć gęślą jaźń").unwrap();
        guest.text("compose", "に").unwrap();
        guest.text("compose", "日本").unwrap();
        guest.text("commit", "日本語").unwrap();
        guest.text("delete", "").unwrap();
        guest.text("commit", "語").unwrap();
        guest.text("finish", "").unwrap();
        let screen = guest.shell(&(guest.guard().unwrap() + "uiautomator dump /data/local/tmp/simplebench-input-check.xml >/dev/null && cat /data/local/tmp/simplebench-input-check.xml && rm /data/local/tmp/simplebench-input-check.xml")).unwrap();
        assert!(
            screen.contains("text=\"Zażółć gęślą jaźń日本語\""),
            "Guest editor did not preserve Unicode and composition: {screen}"
        );
        let stale = Guest {
            generation_key: "00000000-0000-0000-0000-000000000003".into(),
            ..guest
        };
        assert!(stale.booted().is_err());
        assert!(stale.text("commit", "WRONG TRANSPORT").is_err());
        let mut file =
            std::fs::File::open(root.join("development-tools/input-build/input-test.apk")).unwrap();
        assert!(stale
            .install_apk(&mut file, &Arc::new(AtomicBool::new(false)))
            .is_err());
        assert_eq!(std::io::Seek::stream_position(&mut file).unwrap(), 0);
    }
}
