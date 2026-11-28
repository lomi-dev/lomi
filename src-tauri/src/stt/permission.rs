//! Microphone permission state per platform.
//!
//! macOS feeds silence instead of failing when access is denied, so the state
//! is read explicitly and an undecided user is asked before the stream opens.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Granted,
    Denied,
}

/// "granted", "denied" or "undetermined" (macOS asks on the first dictation).
#[cfg(target_os = "macos")]
pub fn state() -> &'static str {
    mac::state()
}

#[cfg(windows)]
pub fn state() -> &'static str {
    if win::denied() {
        "denied"
    } else {
        "granted"
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn state() -> &'static str {
    "granted"
}

/// Blocks until the user decided when macOS has not asked yet.
#[cfg(target_os = "macos")]
pub fn ensure() -> Access {
    mac::ensure()
}

/// Windows streams silence when a privacy switch is off; read them instead.
#[cfg(windows)]
pub fn ensure() -> Access {
    if win::denied() {
        Access::Denied
    } else {
        Access::Granted
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn ensure() -> Access {
    Access::Granted
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
    };

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    /// The consent value ("Allow" or "Deny") of one switch, if set.
    fn value(root: HKEY, key: &str) -> Option<String> {
        let (key, name) = (wide(key), wide("Value"));
        let mut buffer = [0u16; 16];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        // SAFETY: both names are NUL-terminated and `size` is the buffer's byte length.
        let status = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        let chars = (size as usize / 2).saturating_sub(1).min(buffer.len());
        (status == 0).then(|| String::from_utf16_lossy(&buffer[..chars]))
    }

    /// Device-wide access, the user's access, or access for desktop apps is off.
    pub fn denied() -> bool {
        [
            (HKEY_LOCAL_MACHINE, KEY.to_string()),
            (HKEY_CURRENT_USER, KEY.to_string()),
            (HKEY_CURRENT_USER, format!(r"{KEY}\NonPackaged")),
        ]
        .iter()
        .any(|(root, key)| value(*root, key).as_deref() == Some("Deny"))
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::mpsc;
    use std::sync::Mutex;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;

    use super::Access;

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeAudio: &'static NSString;
    }

    // AVAuthorizationStatus values.
    const NOT_DETERMINED: isize = 0;
    const AUTHORIZED: isize = 3;

    fn status() -> isize {
        unsafe {
            msg_send![
                class!(AVCaptureDevice),
                authorizationStatusForMediaType: AVMediaTypeAudio
            ]
        }
    }

    pub fn state() -> &'static str {
        match status() {
            AUTHORIZED => "granted",
            NOT_DETERMINED => "undetermined",
            _ => "denied",
        }
    }

    pub fn ensure() -> Access {
        match status() {
            AUTHORIZED => Access::Granted,
            NOT_DETERMINED => request(),
            _ => Access::Denied,
        }
    }

    fn request() -> Access {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(Some(tx));
        let done = RcBlock::new(move |granted: Bool| {
            if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
                let _ = tx.send(granted.as_bool());
            }
        });
        unsafe {
            let _: () = msg_send![
                class!(AVCaptureDevice),
                requestAccessForMediaType: AVMediaTypeAudio,
                completionHandler: &*done
            ];
        }
        // The system dialog waits for the user; give up after two minutes.
        match rx.recv_timeout(Duration::from_secs(120)) {
            Ok(true) => Access::Granted,
            _ => Access::Denied,
        }
    }
}
