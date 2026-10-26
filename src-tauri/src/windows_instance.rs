use std::{
    ffi::{c_void, OsStr},
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::PathBuf,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tauri::Manager;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions},
    time,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, LocalFree, ERROR_ALREADY_EXISTS, WAIT_OBJECT_0},
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    System::{
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            CreateEventW, CreateMutexW, GetCurrentProcessId, SetEvent, WaitForSingleObject,
            INFINITE,
        },
    },
};

const FORWARD_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PATHS: usize = 16_384;
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
const FRAME_MAGIC: &[u8; 4] = b"LFI1";
const ACK_MAGIC: &[u8; 4] = b"LOK1";
const ACK_RECEIVED: &[u8; 4] = b"LRC1";
const ERROR_PIPE_BUSY: u32 = 231;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_SEM_TIMEOUT: u32 = 121;

struct ForwardState {
    app: Option<tauri::AppHandle>,
    pending: Vec<PathBuf>,
}

pub(crate) struct Instance {
    // The mutex remains alive through native Exit cleanup, independently of HWNDs.
    mutex: Mutex<Option<OwnedHandle>>,
    reopen: Arc<OwnedHandle>,
    forwarding: Arc<Mutex<ForwardState>>,
    pipe_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    listening: AtomicBool,
}

impl Instance {
    pub(crate) fn acquire(identifier: &str) -> io::Result<Option<Self>> {
        Self::acquire_with_paths(identifier, || {
            Ok(crate::external_files::arguments(
                std::env::args_os().skip(1),
                &std::env::current_dir()?,
            ))
        })
    }

    fn acquire_with_paths(
        identifier: &str,
        secondary_paths: impl FnOnce() -> io::Result<Vec<PathBuf>>,
    ) -> io::Result<Option<Self>> {
        let name = |suffix: &str| {
            format!("Local\\{identifier}.{suffix}")
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        // The auto-reset event retains reopen requests made before listen starts.
        let event_name = name("reopen");
        let event = unsafe { CreateEventW(ptr::null(), 0, 0, event_name.as_ptr()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        let reopen = Arc::new(unsafe { OwnedHandle::from_raw_handle(event) });
        let mutex_name = name("instance");
        let mutex = unsafe { CreateMutexW(ptr::null(), 0, mutex_name.as_ptr()) };
        if mutex.is_null() {
            return Err(io::Error::last_os_error());
        }
        let existing = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let mutex = unsafe { OwnedHandle::from_raw_handle(mutex) };
        if existing {
            // Signal the existing reopen behavior even if file forwarding fails.
            if unsafe { SetEvent(reopen.as_raw_handle()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let paths = secondary_paths()?;
            if !paths.is_empty() {
                send_paths_with_delay(&pipe_name(identifier)?, &paths, Duration::ZERO)?;
            }
            return Ok(None);
        }

        let pipe_name = pipe_name(identifier)?;
        let server = create_server_pipe(&pipe_name)?;
        let forwarding = Arc::new(Mutex::new(ForwardState {
            app: None,
            pending: Vec::new(),
        }));
        let task = tauri::async_runtime::spawn(run_pipe_server(server, forwarding.clone()));
        Ok(Some(Self {
            mutex: Mutex::new(Some(mutex)),
            reopen,
            forwarding,
            pipe_task: Mutex::new(Some(task)),
            listening: AtomicBool::new(false),
        }))
    }

    pub(crate) fn release(&self) {
        // Stop accepting requests before releasing ownership to a restart process.
        self.stop_pipe_server();
        self.mutex.lock().unwrap().take();
    }

    pub(crate) fn listen(&self, app: &tauri::AppHandle) -> io::Result<()> {
        if self.listening.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        {
            let mut forwarding = self.forwarding.lock().unwrap();
            forwarding.app = Some(app.clone());
            let pending = std::mem::take(&mut forwarding.pending);
            if !pending.is_empty() {
                crate::external_files::enqueue(app, pending);
            }
        }

        let reopen = self.reopen.clone();
        let app = app.clone();
        let result = thread::Builder::new()
            .name("lomi-reopen".into())
            .spawn(move || loop {
                if unsafe { WaitForSingleObject(reopen.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0
                {
                    eprintln!(
                        "Cannot wait for Lomi reopen: {}",
                        io::Error::last_os_error()
                    );
                    break;
                }
                if let Some(window) = app.get_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            });
        if let Err(error) = result {
            self.listening.store(false, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    fn stop_pipe_server(&self) {
        if let Some(task) = self.pipe_task.lock().unwrap().take() {
            task.abort();
            let _ = tauri::async_runtime::block_on(task);
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stop_pipe_server();
    }
}

fn pipe_name(identifier: &str) -> io::Result<String> {
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(r"\\.\pipe\{identifier}.{session}.files"))
}

fn create_server_pipe(name: &str) -> io::Result<NamedPipeServer> {
    // OWNER RIGHTS restricts the pipe DACL to the creating user's token owner.
    let sddl = OsStr::new("D:P(A;;GA;;;OW)")
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let result = tauri::async_runtime::block_on(async {
        let mut options = ServerOptions::new();
        options.first_pipe_instance(true).max_instances(1);
        unsafe {
            options.create_with_security_attributes_raw(
                name,
                &mut attributes as *mut SECURITY_ATTRIBUTES as *mut c_void,
            )
        }
    });
    unsafe { LocalFree(descriptor) };
    result
}

fn send_paths_with_delay(name: &str, paths: &[PathBuf], ack_delay: Duration) -> io::Result<()> {
    let frame = encode_paths(paths)?;
    tauri::async_runtime::block_on(async {
        time::timeout(FORWARD_TIMEOUT, async {
            loop {
                match ClientOptions::new().open(name) {
                    Ok(mut pipe) => {
                        pipe.write_all(&frame).await?;
                        time::sleep(ack_delay).await;
                        let mut ack = [0; 4];
                        pipe.read_exact(&mut ack).await?;
                        if &ack != ACK_MAGIC {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "invalid Lomi file-forwarding acknowledgment",
                            ));
                        }
                        pipe.write_all(ACK_RECEIVED).await?;
                        return Ok(());
                    }
                    Err(error)
                        if error.raw_os_error().is_some_and(|code| {
                            matches!(
                                code as u32,
                                ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY | ERROR_SEM_TIMEOUT
                            )
                        }) =>
                    {
                        time::sleep(Duration::from_millis(20)).await
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timed out forwarding file paths"))?
    })
}

fn encode_paths(paths: &[PathBuf]) -> io::Result<Vec<u8>> {
    let payload = serde_json::to_vec(paths).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid file path: {error}"),
        )
    })?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Lomi request is too large",
        ));
    }
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(FRAME_MAGIC);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

async fn read_paths(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<PathBuf>> {
    let mut header = [0; 8];
    reader.read_exact(&mut header).await?;
    if &header[..4] != FRAME_MAGIC {
        return Err(invalid_data("invalid request"));
    }
    let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    if size == 0 || size > MAX_PAYLOAD_BYTES {
        return Err(invalid_data("invalid request size"));
    }
    let mut payload = vec![0; size];
    reader.read_exact(&mut payload).await?;
    let paths: Vec<PathBuf> = serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if paths.is_empty() || paths.len() > MAX_PATHS {
        return Err(invalid_data("invalid path count"));
    }
    Ok(paths)
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

async fn run_pipe_server(mut pipe: NamedPipeServer, forwarding: Arc<Mutex<ForwardState>>) {
    loop {
        if let Err(error) = pipe.connect().await {
            eprintln!("Cannot accept a Lomi file-forwarding request: {error}");
            return;
        }
        let request = async {
            let paths = read_paths(&mut pipe).await?;
            enqueue_paths(&forwarding, paths);
            pipe.write_all(ACK_MAGIC).await?;
            let mut receipt = [0; 4];
            pipe.read_exact(&mut receipt).await?;
            if &receipt != ACK_RECEIVED {
                return Err(invalid_data("invalid acknowledgment receipt"));
            }
            Ok(())
        };
        let result = time::timeout(FORWARD_TIMEOUT, request).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("Ignoring invalid Lomi file-forwarding request: {error}"),
            Err(_) => eprintln!("Timed out reading a Lomi file-forwarding request"),
        }
        let _ = pipe.disconnect();
    }
}

fn enqueue_paths(forwarding: &Mutex<ForwardState>, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let mut forwarding = forwarding.lock().unwrap();
    if let Some(app) = forwarding.app.as_ref() {
        crate::external_files::enqueue(app, paths);
    } else {
        forwarding.pending.extend(paths);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::Command, time::SystemTime};
    use windows_sys::Win32::Foundation::WAIT_TIMEOUT;

    fn identifier() -> String {
        format!(
            "dev.lomi.instance-test.{}.{:?}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    fn wait_for_pending(instance: &Instance, expected: &[PathBuf]) {
        tauri::async_runtime::block_on(async {
            time::timeout(Duration::from_secs(6), async {
                loop {
                    if instance.forwarding.lock().unwrap().pending == expected {
                        return;
                    }
                    time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("timed out waiting for forwarded paths");
        });
    }

    #[test]
    fn secondary_signals_before_the_primary_starts_listening() {
        let identifier = identifier();
        let primary = Instance::acquire(&identifier).unwrap().unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "windows_instance::tests::secondary_launcher",
            ])
            .env("LOMI_INSTANCE_TEST_IDENTIFIER", &identifier)
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        assert_eq!(
            unsafe { WaitForSingleObject(primary.reopen.as_raw_handle(), 0) },
            WAIT_OBJECT_0
        );
        assert_eq!(
            unsafe { WaitForSingleObject(primary.reopen.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
    }

    #[test]
    fn secondary_forwards_multiple_unicode_paths_before_frontend_readiness() {
        let identifier = identifier();
        let primary = Instance::acquire(&identifier).unwrap().unwrap();
        let first = PathBuf::from(r"C:\Users\test\Project with spaces\one.txt");
        let second = PathBuf::from(r"C:\Users\test\项目\résumé.md");
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "windows_instance::tests::secondary_file_launcher",
            ])
            .env("LOMI_INSTANCE_TEST_IDENTIFIER", &identifier)
            .env("LOMI_INSTANCE_TEST_PATH_ONE", first.as_os_str())
            .env("LOMI_INSTANCE_TEST_PATH_TWO", second.as_os_str())
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        wait_for_pending(&primary, &[first, second]);
    }

    #[test]
    fn path_frame_round_trips_spaces_and_unicode() {
        let expected = vec![
            PathBuf::from(r"C:\Project folder\hello.txt"),
            PathBuf::from(r"C:\项目\café\😀.md"),
        ];
        tauri::async_runtime::block_on(async {
            let frame = encode_paths(&expected).unwrap();
            let (mut writer, mut reader) = tokio::io::duplex(frame.len());
            writer.write_all(&frame).await.unwrap();
            drop(writer);
            assert_eq!(read_paths(&mut reader).await.unwrap(), expected);
        });
    }

    #[test]
    fn closing_the_owner_allows_a_fresh_instance() {
        let identifier = identifier();
        let primary = Instance::acquire(&identifier).unwrap().unwrap();
        assert!(Instance::acquire_with_paths(&identifier, || Ok(Vec::new()))
            .unwrap()
            .is_none());
        drop(primary);
        assert!(Instance::acquire(&identifier).unwrap().is_some());
    }

    #[test]
    fn exit_cleanup_releases_the_mutex_before_process_restart() {
        let identifier = identifier();
        let primary = Instance::acquire(&identifier).unwrap().unwrap();
        primary.release();
        let replacement = Instance::acquire(&identifier).unwrap().unwrap();
        assert!(Instance::acquire_with_paths(&identifier, || Ok(Vec::new()))
            .unwrap()
            .is_none());
        drop(replacement);
        drop(primary);
        assert!(Instance::acquire(&identifier).unwrap().is_some());
    }

    #[test]
    #[ignore = "subprocess helper for the interprocess instance test"]
    fn secondary_launcher() {
        let identifier = std::env::var("LOMI_INSTANCE_TEST_IDENTIFIER").unwrap();
        assert!(Instance::acquire_with_paths(&identifier, || Ok(Vec::new()))
            .unwrap()
            .is_none());
    }

    #[test]
    #[ignore = "subprocess helper for the interprocess file-forwarding test"]
    fn secondary_file_launcher() {
        let identifier = std::env::var("LOMI_INSTANCE_TEST_IDENTIFIER").unwrap();
        let paths = vec![
            PathBuf::from(std::env::var_os("LOMI_INSTANCE_TEST_PATH_ONE").unwrap()),
            PathBuf::from(std::env::var_os("LOMI_INSTANCE_TEST_PATH_TWO").unwrap()),
        ];
        send_paths_with_delay(
            &pipe_name(&identifier).unwrap(),
            &paths,
            Duration::from_millis(300),
        )
        .unwrap();
    }
}
