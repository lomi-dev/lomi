use objc2::{
    rc::{Retained, Weak},
    MainThreadMarker,
};
use objc2_app_kit::{NSView, NSWindow, NSWindowButton, NSWindowStyleMask};
use std::{cell::RefCell, collections::HashMap};
use tauri::{AppHandle, LogicalPosition, Manager};

thread_local! {
    static WINDOWS: RefCell<HashMap<String, Weak<NSWindow>>> = RefCell::new(HashMap::new());
}

pub fn forget(label: &str) {
    WINDOWS.with(|windows| windows.borrow_mut().remove(label));
}

pub fn refresh(app: &AppHandle) {
    let Some(_main_thread) = MainThreadMarker::new() else {
        return;
    };
    // Tao queues title changes on the main dispatch queue. Restore the inset
    // after native events, even when AppKit does not redraw the content view.
    let label = "main";
    let native = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        if let Some(native) = windows.get(label).and_then(Weak::load) {
            return Some(native);
        }
        // Cloning Tauri's window dispatcher or querying its raw handle wakes the
        // event loop. Cache weak native references to avoid self-triggered refresh.
        let window = app.get_window(label)?;
        let pointer = window.ns_window().ok()?;
        let native = unsafe { Retained::retain(pointer.cast::<NSWindow>()) }?;
        windows.insert(label.to_string(), Weak::new(&native));
        Some(native)
    });
    let Some(native) = native else {
        return;
    };
    let position = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == label)
        .and_then(|window| window.traffic_light_position.as_ref())
        .map(|position| LogicalPosition::new(position.x, position.y));
    if let Some(position) = position {
        position_buttons(&native, position);
    }
}

fn position_buttons(window: &NSWindow, position: LogicalPosition<f64>) {
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }
    let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    let Some(minimize) = window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
        return;
    };
    let Some(container) = (unsafe { close.superview().and_then(|view| view.superview()) }) else {
        return;
    };
    let close_frame = NSView::frame(&close);
    let spacing = NSView::frame(&minimize).origin.x - close_frame.origin.x;
    let mut frame = container.frame();
    frame.size.height = close_frame.size.height + position.y;
    frame.origin.y = window.frame().size.height - frame.size.height;
    // Avoid scheduling another native update when the layout is already correct.
    if container.frame() != frame {
        container.setFrame(frame);
    }
    let zoom = window.standardWindowButton(NSWindowButton::ZoomButton);
    for (index, button) in [Some(close), Some(minimize), zoom].into_iter().enumerate() {
        if let Some(button) = button {
            let mut origin = NSView::frame(&button).origin;
            let x = position.x + index as f64 * spacing;
            if origin.x != x {
                origin.x = x;
                button.setFrameOrigin(origin);
            }
        }
    }
}
