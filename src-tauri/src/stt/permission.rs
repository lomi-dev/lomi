//! Microphone permission state per platform.
//!
//! macOS feeds silence instead of failing when access is denied, so the state
//! is read explicitly and an undecided user is asked before the stream opens.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Granted,
    Denied,
}

/// Blocks until the user decided when macOS has not asked yet.
#[cfg(target_os = "macos")]
pub fn ensure() -> Access {
    mac::ensure()
}

// shortcut: Windows privacy switches are only detected by the digital-silence
// check and stream errors; read the ConsentStore registry keys in E2-R (R8).
#[cfg(not(target_os = "macos"))]
pub fn ensure() -> Access {
    Access::Granted
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
