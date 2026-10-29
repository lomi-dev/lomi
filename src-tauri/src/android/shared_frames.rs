//! One native mapping per frame. No COM interface leaves the WebView UI thread.
use super::{
    frames::{Delivery, Sink},
    storage::valid_id,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use tauri::{Manager, Window};
use tokio::sync::oneshot;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Environment12, ICoreWebView2SharedBuffer, ICoreWebView2_17,
    COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY,
};
use windows::core::{Interface, PCWSTR, PWSTR};

const MIN_PACKET: usize = 1024;
const MAX_PACKET: usize = 68 + 720 * 1280 * 4;
const E_NOINTERFACE: i32 = 0x80004002u32 as i32;

struct Queued {
    canceled: AtomicBool,
    packet: Mutex<Option<Vec<u8>>>,
}
impl Queued {
    fn new(packet: Vec<u8>) -> Result<Arc<Self>, String> {
        if !(MIN_PACKET..=MAX_PACKET).contains(&packet.len()) {
            return Err("Android shared frame exceeds the supported packet budget".into());
        }
        Ok(Arc::new(Self {
            canceled: AtomicBool::new(false),
            packet: Mutex::new(Some(packet)),
        }))
    }
    fn take(&self) -> Result<Vec<u8>, String> {
        self.packet
            .lock()
            .map_err(|_| "Android shared frame queue failed")?
            .take()
            .ok_or_else(|| "Android shared frame was canceled".into())
    }
    fn active(&self, sender: &oneshot::Sender<Result<bool, String>>) -> bool {
        !self.canceled.load(Ordering::Acquire) && !sender.is_closed()
    }
}
struct Cancel(Arc<Queued>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.canceled.store(true, Ordering::Release);
        // This guard belongs to the waiting future, not the queued UI callback.
        // Canceling the stream therefore frees queued pixels even if the UI stalls.
        if let Ok(mut packet) = self.0.packet.lock() {
            packet.take();
        }
    }
}
struct Buffer(ICoreWebView2SharedBuffer);
impl Drop for Buffer {
    fn drop(&mut self) {
        // WebView2 documents independent native/script mappings: Close does not
        // invalidate the script view; JS must separately call releaseBuffer.
        unsafe {
            let _ = self.0.Close();
        }
    }
}

pub(super) fn sink(
    window: &Window,
    token: String,
    device_id: String,
    generation: String,
    document: Arc<AtomicU64>,
    raw: Sink,
) -> Result<Sink, String> {
    if !valid_id(&token) || !valid_id(&device_id) || !valid_id(&generation) {
        return Err("Invalid Android shared frame identity".into());
    }
    crate::files::main_window(window)?;
    let webview = window
        .app_handle()
        .get_webview("main")
        .ok_or("Android main webview is unavailable")?;
    let document_generation = document.load(Ordering::Acquire);
    let development = tauri::is_dev();
    let dev_url = window.app_handle().config().build.dev_url.clone();
    let window = window.clone();
    let metadata: Arc<Vec<u16>> = Arc::new(
        serde_json::json!({
            "protocol": "lomi.android.frame.v1", "token": token,
            "deviceId": device_id, "generation": generation
        })
        .to_string()
        .encode_utf16()
        .chain(Some(0))
        .collect(),
    );
    let raw = Arc::new(raw);
    let unsupported = Arc::new(AtomicBool::new(false));
    Ok(Box::new(move |packet| -> Delivery {
        let document = document.clone();
        let dev_url = dev_url.clone();
        let webview = webview.clone();
        let window = window.clone();
        let metadata = metadata.clone();
        let raw = raw.clone();
        let unsupported = unsupported.clone();
        Box::pin(async move {
            let queued = Queued::new(packet)?;
            let _cancel = Cancel(queued.clone());
            if unsupported.load(Ordering::Acquire) {
                return raw(queued.take()?).await;
            }
            let (sender, receiver) = oneshot::channel();
            let ui_queued = queued.clone();
            webview
                .with_webview(move |platform| {
                    let result = (|| -> Result<bool, String> {
                        if !ui_queued.active(&sender) {
                            return Err("Android shared frame was canceled".into());
                        }
                        current_document(&document, document_generation)?;
                        crate::files::main_window(&window)?;
                        if !window.is_visible().map_err(|e| e.to_string())?
                            || window.is_minimized().map_err(|e| e.to_string())?
                        {
                            return Err("Android image is paused while the window is hidden".into());
                        }
                        let environment =
                            match platform.environment().cast::<ICoreWebView2Environment12>() {
                                Ok(value) => value,
                                Err(error) if error.code().0 == E_NOINTERFACE => return Ok(false),
                                Err(error) => return Err(error.to_string()),
                            };
                        let core = unsafe { platform.controller().CoreWebView2() }
                            .map_err(|e| e.to_string())?;
                        let core = match core.cast::<ICoreWebView2_17>() {
                            Ok(value) => value,
                            Err(error) if error.code().0 == E_NOINTERFACE => return Ok(false),
                            Err(error) => return Err(error.to_string()),
                        };
                        if !ui_queued.active(&sender) {
                            return Err("Android shared frame was canceled".into());
                        }
                        current_document(&document, document_generation)?;
                        let packet = ui_queued.take()?;
                        let buffer = Buffer(
                            unsafe { environment.CreateSharedBuffer(packet.len() as u64) }
                                .map_err(|e| e.to_string())?,
                        );
                        let mut target = std::ptr::null_mut();
                        unsafe { buffer.0.Buffer(&mut target) }.map_err(|e| e.to_string())?;
                        if target.is_null() {
                            return Err("Android shared frame mapping is unavailable".into());
                        }
                        unsafe {
                            std::ptr::copy_nonoverlapping(packet.as_ptr(), target, packet.len());
                        }
                        drop(packet);
                        if !ui_queued.active(&sender) {
                            return Err("Android shared frame was canceled".into());
                        }
                        // Read the native Source in this apartment; a Tauri URL
                        // getter here could synchronously redispatch to the UI.
                        let mut source = PWSTR::null();
                        let source_result = unsafe { core.Source(&mut source) };
                        let source = webview2_com::take_pwstr(source);
                        source_result.map_err(|e| e.to_string())?;
                        if !trusted_source(&source, development, dev_url.as_ref()) {
                            return Err(
                                "Android shared frames require the trusted app document".into()
                            );
                        }
                        current_document(&document, document_generation)?;
                        if !ui_queued.active(&sender) {
                            return Err("Android shared frame was canceled".into());
                        }
                        unsafe {
                            core.PostSharedBufferToScript(
                                &buffer.0,
                                COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY,
                                PCWSTR(metadata.as_ptr()),
                            )
                        }
                        .map_err(|e| e.to_string())?;
                        Ok(true)
                    })();
                    let _ = sender.send(result);
                })
                .map_err(|e| format!("Cannot schedule Android shared frame: {e}"))?;
            match completion(receiver).await? {
                true => Ok(()),
                false => {
                    unsupported.store(true, Ordering::Release);
                    raw(queued.take()?).await
                }
            }
        })
    }))
}

fn current_document(document: &AtomicU64, generation: u64) -> Result<(), String> {
    if document.load(Ordering::Acquire) == generation {
        Ok(())
    } else {
        Err("Android shared frame document changed. Reconnect the panel.".into())
    }
}

fn trusted_source(source: &str, development: bool, dev_url: Option<&tauri::Url>) -> bool {
    let Ok(source) = tauri::Url::parse(source) else {
        return false;
    };
    if !source.username().is_empty() || source.password().is_some() {
        return false;
    }
    if matches!(source.scheme(), "http" | "https")
        && source.host_str() == Some("tauri.localhost")
        && source.port().is_none()
        && matches!(source.path(), "/" | "/index.html")
        && source.query().is_none()
    {
        return true;
    }
    development
        && dev_url.is_some_and(|dev| {
            matches!(dev.scheme(), "http" | "https") && source.origin() == dev.origin()
        })
}

async fn completion(receiver: oneshot::Receiver<Result<bool, String>>) -> Result<bool, String> {
    completion_with_timeout(receiver, std::time::Duration::from_secs(2)).await
}

async fn completion_with_timeout(
    receiver: oneshot::Receiver<Result<bool, String>>,
    timeout: std::time::Duration,
) -> Result<bool, String> {
    tokio::time::timeout(timeout, receiver)
        .await
        .map_err(|_| {
            "Android shared frame UI delivery timed out. Reconnect the panel.".to_string()
        })?
        .map_err(|_| "Android shared frame UI callback ended".to_string())?
}
