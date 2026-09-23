//! Automation profiles do not inherit Wry's file chooser or automatic media grant.
//! Installed on a blank view before the first external navigation.
use objc2::{
    define_class, msg_send, rc::Retained, runtime::ProtocolObject, ClassType, MainThreadMarker,
    MainThreadOnly,
};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSURL};
use objc2_web_kit::{
    WKFrameInfo, WKMediaCaptureType, WKOpenPanelParameters, WKPermissionDecision, WKSecurityOrigin,
    WKUIDelegate, WKWebView,
};
use std::cell::RefCell;
#[cfg(feature = "mcp-probe")]
static MEDIA_DENIALS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "mcp-probe")]
pub fn media_denials() -> usize {
    MEDIA_DENIALS.load(std::sync::atomic::Ordering::SeqCst)
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct AgentUIDelegate;
    unsafe impl NSObjectProtocol for AgentUIDelegate {}
    unsafe impl WKUIDelegate for AgentUIDelegate {
        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn deny_file_panel(
            &self,
            _view: &WKWebView,
            _parameters: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            completion: &block2::Block<dyn Fn(*const NSArray<NSURL>)>,
        ) {
            completion.call((std::ptr::null(),));
        }
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn deny_media(
            &self,
            _view: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            _kind: WKMediaCaptureType,
            completion: &block2::Block<dyn Fn(WKPermissionDecision)>,
        ) {
            #[cfg(feature = "mcp-probe")]
            MEDIA_DENIALS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            completion.call((WKPermissionDecision::Deny,));
        }
        #[unsafe(method(webView:requestDeviceOrientationAndMotionPermissionForOrigin:initiatedByFrame:decisionHandler:))]
        fn deny_motion(
            &self,
            _view: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            completion: &block2::Block<dyn Fn(WKPermissionDecision)>,
        ) {
            completion.call((WKPermissionDecision::Deny,));
        }
    }
);
thread_local! {
    static DELEGATE: RefCell<Option<Retained<AgentUIDelegate>>> = const {RefCell::new(None)};
}
pub async fn install(view: &tauri::Webview, expected_profile: [u8; 16]) -> Result<(), String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let marker = MainThreadMarker::new().expect("WebKit dispatcher runs on AppKit");
        let delegate = DELEGATE.with(|slot| {
            slot.borrow_mut()
                .get_or_insert_with(|| {
                    let allocated = AgentUIDelegate::alloc(marker).set_ivars(());
                    unsafe { msg_send![super(allocated), init] }
                })
                .clone()
        });
        let native = unsafe { &*platform.inner().cast::<WKWebView>() };
        unsafe {
            native.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        let profile_matches = unsafe { native.configuration().websiteDataStore().identifier() }
            .is_some_and(|id| id.as_bytes() == expected_profile);
        super::agent_dom::install_logs(native, marker);
        let installed = profile_matches
            && unsafe { native.UIDelegate() }
                .is_some_and(|delegate| delegate.isKindOfClass(AgentUIDelegate::class()));
        let _ = send.send(installed);
    })
    .map_err(|_| "Cannot install browser permission policy.")?;
    let installed = tokio::time::timeout(std::time::Duration::from_secs(3), receive)
        .await
        .map_err(|_| "Browser permission setup timed out.")?
        .map_err(|_| "Browser permission setup did not complete.")?;
    if !installed {
        return Err("Browser permission delegate did not attach.".into());
    }
    Ok(())
}
