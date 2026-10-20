//! Main-WebView-only capture for the opt-in Android product fixture.
use std::path::PathBuf;
use tauri::{Manager, Window};
use webview2_com::{
    CapturePreviewCompletedHandler,
    Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
};
use windows::Win32::{
    Foundation::HGLOBAL,
    System::Com::{
        IStream, StructuredStorage::CreateStreamOnHGlobal, STATFLAG_NONAME, STATSTG,
        STREAM_SEEK_SET,
    },
};

const MAX_PNG_BYTES: u64 = 32 * 1024 * 1024;

fn read_png(stream: &IStream) -> Result<Vec<u8>, String> {
    // The stream and all COM operations stay on WebView2's UI apartment.
    let mut stat = STATSTG::default();
    unsafe { stream.Stat(&mut stat, STATFLAG_NONAME) }.map_err(|e| e.to_string())?;
    if !(8..=MAX_PNG_BYTES).contains(&stat.cbSize) {
        return Err("WebView screenshot is empty or exceeds 32 MiB".into());
    }
    let mut bytes = vec![0; stat.cbSize as usize];
    let mut read = 0;
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }.map_err(|e| e.to_string())?;
    unsafe {
        stream
            .Read(
                bytes.as_mut_ptr().cast(),
                bytes.len() as u32,
                Some(&mut read),
            )
            .ok()
    }
    .map_err(|e| e.to_string())?;
    if read as usize != bytes.len() {
        return Err("WebView screenshot stream was truncated".into());
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("WebView screenshot is not a PNG".into());
    }
    Ok(bytes)
}

fn publish(path: PathBuf, result: Result<Vec<u8>, String>) {
    // Only owned bytes or an error leave the COM apartment. File I/O does not
    // hold up the application's UI message loop.
    tauri::async_runtime::spawn_blocking(move || {
        let result =
            result.and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
        if let Err(error) = result {
            let _ = std::fs::write(path.with_extension("error.txt"), error);
        }
    });
}

pub fn capture(window: &Window, path: PathBuf) -> Result<(), String> {
    crate::files::main_window(window)?;
    let target = window.app_handle().get_webview("main").ok_or_else(|| {
        let error = "Missing main product WebView".to_owned();
        publish(path.clone(), Err(error.clone()));
        error
    })?;
    let error_path = path.clone();
    let scheduled = target.with_webview(move |platform| {
        let capture = (|| -> Result<(), String> {
            let webview =
                unsafe { platform.controller().CoreWebView2() }.map_err(|e| e.to_string())?;
            let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) }
                .map_err(|e| e.to_string())?;
            let callback_stream = stream.clone();
            let callback_path = path.clone();
            let handler = CapturePreviewCompletedHandler::create(Box::new(move |status| {
                let result = status
                    .map_err(|e| e.to_string())
                    .and_then(|()| read_png(&callback_stream));
                publish(callback_path, result);
                Ok(())
            }));
            unsafe {
                webview.CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &handler,
                )
            }
            .map_err(|e| e.to_string())
        })();
        if let Err(error) = capture {
            publish(path, Err(error));
        }
    });
    scheduled.map_err(|error| {
        let error = error.to_string();
        publish(error_path, Err(error.clone()));
        error
    })
}
