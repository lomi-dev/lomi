use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::Manager;
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ALREADY_EXISTS, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject, INFINITE},
};

pub(crate) struct Instance {
    // The mutex remains alive through native Exit cleanup, independently of HWNDs.
    mutex: Mutex<Option<OwnedHandle>>,
    reopen: Arc<OwnedHandle>,
    listening: AtomicBool,
}

impl Instance {
    pub(crate) fn acquire(identifier: &str) -> io::Result<Option<Self>> {
        let name = |suffix: &str| {
            format!("Local\\{identifier}.{suffix}")
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        // Publish the event before election. Auto-reset retains an early reopen
        // request until the primary's frontend is ready to show its window.
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
            if unsafe { SetEvent(reopen.as_raw_handle()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            return Ok(None);
        }
        Ok(Some(Self {
            mutex: Mutex::new(Some(mutex)),
            reopen,
            listening: AtomicBool::new(false),
        }))
    }

    pub(crate) fn release(&self) {
        // Tauri spawns its replacement process before exiting on restart.
        // Release only after application cleanup, so the replacement can own it.
        self.mutex.lock().unwrap().take();
    }

    pub(crate) fn listen(&self, app: &tauri::AppHandle) -> io::Result<()> {
        if self.listening.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let reopen = self.reopen.clone();
        let app = app.clone();
        let result = std::thread::Builder::new()
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
        assert!(Instance::acquire(&identifier).unwrap().is_none());
    }

    #[test]
    fn closing_the_owner_allows_a_fresh_instance() {
        let identifier = identifier();
        let primary = Instance::acquire(&identifier).unwrap().unwrap();
        assert!(Instance::acquire(&identifier).unwrap().is_none());
        drop(primary);
        assert!(Instance::acquire(&identifier).unwrap().is_some());
    }

    #[test]
    fn exit_cleanup_releases_the_mutex_before_process_restart() {
        let identifier = identifier();
        let primary = Instance::acquire(&identifier).unwrap().unwrap();
        primary.release();
        let replacement = Instance::acquire(&identifier).unwrap().unwrap();
        assert!(Instance::acquire(&identifier).unwrap().is_none());
        drop(replacement);
        drop(primary);
        assert!(Instance::acquire(&identifier).unwrap().is_some());
    }

    #[test]
    #[ignore = "subprocess helper for the interprocess instance test"]
    fn secondary_launcher() {
        let identifier = std::env::var("LOMI_INSTANCE_TEST_IDENTIFIER").unwrap();
        assert!(Instance::acquire(&identifier).unwrap().is_none());
    }
}
