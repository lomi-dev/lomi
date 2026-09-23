//! Navigation dispatch and cancellation run on AppKit, independently of page JS.
use lomi_control_core::{broker::NativePermit, browser::BrowserControl};
use lomi_control_protocol::ErrorCode;
use std::{sync::Arc, time::Instant};

pub async fn start(
    view: &tauri::Webview,
    control: Arc<BrowserControl>,
    operation: String,
    url: String,
    permit: NativePermit,
    deadline: Instant,
) -> Result<(), ErrorCode> {
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| {
            permit.check()?;
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            if !control.permits(&url) || !control.may_stop_navigation(&operation) {
                return Err(ErrorCode::ControlRevoked);
            }
            let url =
                objc2_foundation::NSURL::URLWithString(&objc2_foundation::NSString::from_str(&url))
                    .ok_or(ErrorCode::ScopeDenied)?;
            let request = objc2_foundation::NSURLRequest::requestWithURL(&url);
            let native = unsafe { &*platform.inner().cast::<objc2_web_kit::WKWebView>() };
            unsafe { native.loadRequest(&request) }.ok_or(ErrorCode::OutcomeUnknown)?;
            Ok(())
        })();
        let _ = send.send(result);
    })
    .map_err(|_| ErrorCode::AppUnavailable)?;
    tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), receive)
        .await
        .map_err(|_| ErrorCode::DeadlineExceeded)?
        .map_err(|_| ErrorCode::AppUnavailable)?
}

pub fn stop(view: &tauri::Webview, control: Arc<BrowserControl>, operation: String) {
    // Never stop a replacement operation or a page already taken over by a human.
    let _ = view.with_webview(move |platform| {
        if control.may_stop_navigation(&operation) {
            let native = unsafe { &*platform.inner().cast::<objc2_web_kit::WKWebView>() };
            unsafe {
                native.stopLoading();
            }
        }
    });
}
