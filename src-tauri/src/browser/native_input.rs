//! Observe AppKit input before dispatch to the child WKWebView. Page scripts
//! cannot forge this path; DOM events only reach the synthetic action adapter.
use lomi_control_core::browser::BrowserControl;
use objc2::{
    rc::{Retained, Weak},
    runtime::AnyObject,
    MainThreadMarker,
};
use objc2_app_kit::{NSEvent, NSEventMask, NSEventType, NSView};
use std::{cell::RefCell, collections::HashMap, ptr::NonNull, sync::Arc};
use tauri::{Manager, Webview};

thread_local! {
    static MONITORS: RefCell<HashMap<String, Retained<AnyObject>>> = RefCell::new(HashMap::new());
}

#[cfg(feature = "mcp-probe")]
pub fn probe_count() -> usize {
    assert!(MainThreadMarker::new().is_some());
    MONITORS.with(|monitors| monitors.borrow().len())
}

fn forget_on_main(label: &str) {
    if let Some(monitor) = MONITORS.with(|monitors| monitors.borrow_mut().remove(label)) {
        // The token is produced only by addLocalMonitorForEventsMatchingMask.
        unsafe { NSEvent::removeMonitor(&monitor) };
    }
}

pub fn forget(app: &tauri::AppHandle, label: String) {
    let _ = app.run_on_main_thread(move || forget_on_main(&label));
}

pub fn clear(app: &tauri::AppHandle) {
    let _ = app.run_on_main_thread(|| {
        let monitors = MONITORS.with(|monitors| std::mem::take(&mut *monitors.borrow_mut()));
        for monitor in monitors.into_values() {
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    });
}

pub async fn install(view: &Webview, control: Arc<BrowserControl>) -> Result<(), String> {
    let label = view.label().to_owned();
    let app = view.app_handle().clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| {
            let _main = MainThreadMarker::new()
                .ok_or("Browser input monitor requires the native main thread.")?;
            // Tauri's macOS child handle is a WKWebView, a subclass of NSView.
            let native = unsafe { Retained::retain(platform.inner().cast::<NSView>()) }
                .ok_or("Missing native browser view.")?;
            let weak = Weak::new(&*native);
            forget_on_main(&label);
            let monitor_label = label.clone();
            let block = block2::RcBlock::new(move |pointer: NonNull<NSEvent>| {
                let Some(main) = MainThreadMarker::new() else {
                    return pointer.as_ptr();
                };
                let Some(native) = weak.load() else {
                    forget_on_main(&monitor_label);
                    return pointer.as_ptr();
                };
                if !control.authorized() {
                    super::update(&app, &control.panel_id, |_| {});
                    forget_on_main(&monitor_label);
                    return pointer.as_ptr();
                }
                let event = unsafe { pointer.as_ref() };
                let related = (|| {
                    if native.isHiddenOrHasHiddenAncestor() {
                        return false;
                    }
                    let Some(window) = native.window() else {
                        return false;
                    };
                    let Some(event_window) = event.window(main) else {
                        return false;
                    };
                    if Retained::as_ptr(&window) != Retained::as_ptr(&event_window) {
                        return false;
                    }
                    if event.r#type() == NSEventType::KeyDown {
                        return window
                            .firstResponder()
                            .and_then(|responder| responder.downcast::<NSView>().ok())
                            .is_some_and(|view| view.isDescendantOf(&native));
                    }
                    let point = native.convertPoint_fromView(event.locationInWindow(), None);
                    let bounds = native.bounds();
                    point.x >= bounds.origin.x
                        && point.y >= bounds.origin.y
                        && point.x < bounds.origin.x + bounds.size.width
                        && point.y < bounds.origin.y + bounds.size.height
                })();
                if related {
                    control.take_over();
                    super::update(&app, &control.panel_id, |_| {});
                    forget_on_main(&monitor_label);
                }
                pointer.as_ptr()
            });
            let mask = NSEventMask::KeyDown
                | NSEventMask::LeftMouseDown
                | NSEventMask::RightMouseDown
                | NSEventMask::OtherMouseDown
                | NSEventMask::ScrollWheel;
            let monitor =
                unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) }
                    .ok_or("Cannot observe native browser input.")?;
            MONITORS.with(|monitors| monitors.borrow_mut().insert(label, monitor));
            Ok::<_, String>(())
        })();
        let _ = send.send(result);
    })
    .map_err(|e| e.to_string())?;
    receive
        .await
        .map_err(|_| "Native browser input monitor did not register.".to_owned())?
}
