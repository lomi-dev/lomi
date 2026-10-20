//! In-process GTK event dispatch to the owned main WebKit widget, including Wayland.
//! This is native widget dispatch, not compositor/global input injection.
use gtk::{
    gdk,
    glib::{self, translate::*},
    prelude::*,
};
use tauri::{Manager, Window};
use webkit2gtk::WebViewExt;

pub(super) async fn with_main<R: Send + 'static>(
    window: &Window,
    operation: impl FnOnce(webkit2gtk::WebView) -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    let view = window
        .app_handle()
        .get_webview("main")
        .ok_or("Missing main WebKit widget")?;
    let (send, receive) = tokio::sync::oneshot::channel();
    let owner = window.clone();
    view.with_webview(move |platform| {
        let widget = platform.inner();
        let result = (|| {
            let main = owner.gtk_window().map_err(|e| e.to_string())?;
            let main_window = main.clone().upcast::<gtk::Window>();
            if widget.toplevel().as_ref() != Some(main.upcast_ref::<gtk::Widget>()) {
                return Err("Main WebKit widget is not owned by the main GTK window".into());
            }
            let modal_open = gtk::Window::list_toplevels().into_iter().any(|candidate| {
                candidate
                    .downcast::<gtk::Window>()
                    .ok()
                    .is_some_and(|native| {
                        if !native.is_visible() || native == main_window {
                            return false;
                        }
                        let owned_android_chooser = native
                            .clone()
                            .downcast::<gtk::FileChooserDialog>()
                            .ok()
                            .is_some_and(|dialog| {
                                let title = dialog.title().unwrap_or_default();
                                let qualified = (title == "Install APK on Android"
                                    && dialog.action() == gtk::FileChooserAction::Open)
                                    || (title == "Save Android screenshot"
                                        && dialog.action() == gtk::FileChooserAction::Save);
                                qualified
                                    && dialog
                                        .transient_for()
                                        .is_none_or(|parent| parent == main_window)
                            });
                        native.is_modal() || owned_android_chooser
                    })
            });
            if modal_open {
                Err("An owned native dialog blocks main WebKit input".into())
            } else if !main.is_active() {
                // Check before pointer-down can grab focus; a background event
                // simulator must not reactivate the owned window implicitly.
                Err("Main GTK window is not active for native input".into())
            } else if widget.is_mapped() && widget.is_sensitive() {
                operation(widget)
            } else {
                Err("Main WebKit widget is not mapped and sensitive".into())
            }
        })();
        let _ = send.send(result);
    })
    .map_err(|e| e.to_string())?;
    receive.await.map_err(|e| e.to_string())?
}

pub async fn text(window: &Window, action: String, text: String) -> Result<(), String> {
    if action != "commit" {
        return Err(
            "Linux native text fixture supports committed UTF-8 only; preedit is unsupported"
                .into(),
        );
    }
    if text.contains('\0') {
        return Err("Native text cannot contain NUL".into());
    }
    with_main(window, move |widget| {
        if !widget.has_focus() {
            return Err("Main WebKit widget does not own native keyboard focus".into());
        }
        let context = widget
            .input_method_context()
            .ok_or("WebKit has no native input method context")?;
        // The same WebKit IM-context signal used by the platform IME. Never evaluate JS,
        // change DOM values, or inject global keys as evidence of committed native text.
        context.emit_by_name::<()>("committed", &[&text]);
        Ok(())
    })
    .await
}

pub async fn pointer(window: &Window, phase: String, x: f64, y: f64) -> Result<(), String> {
    with_main(window, move |widget| {
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= f64::from(widget.allocated_width())
            || y >= f64::from(widget.allocated_height())
        {
            return Err("Native pointer is outside its own WebKit widget".into());
        }
        let native = widget
            .window()
            .ok_or("Main WebKit widget has no GDK window")?;
        let device = native
            .display()
            .default_seat()
            .and_then(|seat| seat.pointer())
            .ok_or("Main WebKit display has no native pointer device")?;
        let kind = match phase.as_str() {
            "down" => gdk::EventType::ButtonPress,
            "up" => gdk::EventType::ButtonRelease,
            "drag" | "move" => gdk::EventType::MotionNotify,
            "wheel-up" | "wheel-down" => gdk::EventType::Scroll,
            _ => return Err("Invalid native pointer phase".into()),
        };
        let mut event = gdk::Event::new(kind);
        event.set_device(Some(&device));
        let state = if matches!(phase.as_str(), "up" | "drag") {
            gdk::ModifierType::BUTTON1_MASK.into_glib()
        } else {
            0
        };
        let time = (glib::monotonic_time() / 1000) as u32;
        // GDK's event union has no Rust setters for coordinates. The allocated variant
        // matches each accessed union member; the event owns its retained window ref.
        unsafe {
            let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
            (*raw).any.window = native.to_glib_full();
            (*raw).any.send_event = 1;
            match kind {
                gdk::EventType::ButtonPress | gdk::EventType::ButtonRelease => {
                    let value = &mut (*raw).button;
                    value.time = time;
                    value.x = x;
                    value.y = y;
                    value.state = state;
                    value.button = 1;
                }
                gdk::EventType::MotionNotify => {
                    let value = &mut (*raw).motion;
                    value.time = time;
                    value.x = x;
                    value.y = y;
                    value.state = state;
                }
                gdk::EventType::Scroll => {
                    let value = &mut (*raw).scroll;
                    value.time = time;
                    value.x = x;
                    value.y = y;
                    value.state = state;
                    value.direction = if phase == "wheel-up" {
                        gdk::ffi::GDK_SCROLL_UP
                    } else {
                        gdk::ffi::GDK_SCROLL_DOWN
                    };
                }
                _ => unreachable!(),
            }
        }
        if phase == "down" {
            widget.grab_focus();
        }
        // Enter GTK's normal event routing for the owned GDK window. This handles
        // propagation and grabs; an unhandled widget signal is not a dispatch error.
        // Never enqueue into gdk_event_put/the global event queue.
        gtk::main_do_event(&mut event);
        Ok(())
    })
    .await
}
