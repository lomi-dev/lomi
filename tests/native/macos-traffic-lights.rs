//! Run on macOS with `cargo run --manifest-path src-tauri/Cargo.toml --locked
//! --example macos-traffic-lights`. Pass `-- --without-fix` to verify the failure.

#[cfg(target_os = "macos")]
#[path = "../../src-tauri/src/macos/traffic_lights.rs"]
mod traffic_lights;

#[cfg(target_os = "macos")]
fn main() {
    use objc2_app_kit::{NSBitmapImageFileType, NSView, NSWindow, NSWindowButton};
    use objc2_foundation::{NSDictionary, NSString};
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        thread,
        time::Duration,
    };
    use tauri::{
        LogicalPosition, LogicalSize, Manager, TitleBarStyle, WebviewUrl, WebviewWindowBuilder,
    };

    fn measure(app: &tauri::AppHandle, label: &'static str, stage: &str) -> Vec<(f64, f64)> {
        thread::sleep(Duration::from_millis(400));
        let (tx, rx) = mpsc::channel();
        let handle = app.clone();
        let screenshot = (stage == "fullscreen exit")
            .then(|| std::env::var_os("LOMI_TRAFFIC_LIGHT_SCREENSHOTS"))
            .flatten()
            .map(|directory| std::path::PathBuf::from(directory).join(format!("{label}.png")));
        app.run_on_main_thread(move || {
            assert!(objc2::MainThreadMarker::new().is_some());
            let window = handle.get_webview_window(label).unwrap();
            // Tauri owns the NSWindow throughout this main-thread callback.
            let native = unsafe { &*window.ns_window().unwrap().cast::<NSWindow>() };
            let positions = [
                NSWindowButton::CloseButton,
                NSWindowButton::MiniaturizeButton,
                NSWindowButton::ZoomButton,
            ]
            .into_iter()
            .map(|kind| {
                let button = native.standardWindowButton(kind).unwrap();
                let rect = button.convertRect_toView(NSView::bounds(&button), None);
                (
                    rect.origin.x,
                    native.frame().size.height - rect.origin.y - rect.size.height,
                )
            })
            .collect::<Vec<_>>();
            if let Some(path) = screenshot {
                let frame = unsafe {
                    native
                        .standardWindowButton(NSWindowButton::CloseButton)
                        .unwrap()
                        .superview()
                        .unwrap()
                        .superview()
                        .unwrap()
                };
                let bounds = frame.bounds();
                let bitmap = frame.bitmapImageRepForCachingDisplayInRect(bounds).unwrap();
                frame.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
                let png = unsafe {
                    bitmap.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                }
                .unwrap();
                assert!(
                    png.writeToFile_atomically(&NSString::from_str(path.to_str().unwrap()), true)
                );
            }
            tx.send(positions).unwrap();
        })
        .unwrap();
        let positions = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        println!("{label} / {stage}: {positions:?}");
        positions
    }

    let mut context = tauri::generate_context!();
    for window in &mut context.config_mut().app.windows {
        window.create = false;
    }
    let app = tauri::Builder::default().build(context).unwrap();
    let fix = !std::env::args().any(|arg| arg == "--without-fix");
    let passed = Arc::new(AtomicBool::new(false));
    let result = passed.clone();
    let handle = app.handle().clone();
    thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Create after the event loop starts, as with the preloaded settings window.
            for label in ["main", "settings"] {
                let position = if label == "main" {
                    let config = handle
                        .config()
                        .app
                        .windows
                        .iter()
                        .find(|window| window.label == label)
                        .unwrap();
                    let inset = config.traffic_light_position.as_ref().unwrap();
                    LogicalPosition::new(inset.x, inset.y)
                } else {
                    traffic_lights::SETTINGS_POSITION
                };
                let window = WebviewWindowBuilder::new(
                    &handle,
                    label,
                    WebviewUrl::External("about:blank".parse().unwrap()),
                )
                .title("Traffic lights regression")
                .inner_size(920.0, 680.0)
                .visible(false)
                .decorations(true)
                .transparent(true)
                .title_bar_style(TitleBarStyle::Overlay)
                .hidden_title(true)
                .traffic_light_position(position)
                .build()
                .unwrap();
                let css = include_str!("../../src/theme/baseline.css");
                let html = format!("<style>{css} body{{margin:0;background:var(--color-surface);color:var(--color-surface-text);font:13px system-ui}} header{{height:43px;border-bottom:1px solid var(--color-outline);padding-left:100px;line-height:43px}} p{{padding:24px}}</style><header>Lomi</header><p>Native macOS traffic light regression</p>");
                window
                    .eval(format!(
                        "document.documentElement.innerHTML={}",
                        serde_json::to_string(&html).unwrap()
                    ))
                    .unwrap();
                window.show().unwrap();
                window.set_focus().unwrap();
                let baseline = measure(&handle, label, "first show");
                assert_eq!(
                    baseline[0],
                    (14.0, 15.0),
                    "Configured inset must match the titlebar"
                );
                for index in 0..10 {
                    window
                        .set_title(&format!("Project — workspace {index} — Lomi"))
                        .unwrap();
                    assert_eq!(measure(&handle, label, "title changed"), baseline);
                }
                window.set_size(LogicalSize::new(800.0, 500.0)).unwrap();
                assert_eq!(measure(&handle, label, "resize"), baseline);
                window.maximize().unwrap();
                assert_eq!(measure(&handle, label, "maximize"), baseline);
                window.set_fullscreen(true).unwrap();
                thread::sleep(Duration::from_secs(2));
                assert!(window.is_fullscreen().unwrap());
                window.set_fullscreen(false).unwrap();
                thread::sleep(Duration::from_secs(2));
                assert!(!window.is_fullscreen().unwrap());
                assert_eq!(measure(&handle, label, "fullscreen exit"), baseline);
                window.unmaximize().unwrap();
                assert_eq!(measure(&handle, label, "unmaximize"), baseline);
                window.minimize().unwrap();
                thread::sleep(Duration::from_secs(1));
                window.unminimize().unwrap();
                thread::sleep(Duration::from_secs(1));
                assert_eq!(measure(&handle, label, "unminimize"), baseline);
                window.hide().unwrap();
                thread::sleep(Duration::from_millis(200));
                window.show().unwrap();
                window.set_focus().unwrap();
                assert_eq!(measure(&handle, label, "show again"), baseline);
                window.destroy().unwrap();
            }
        }));
        result.store(outcome.is_ok(), Ordering::Relaxed);
        handle.exit(0);
    });
    app.run_return(move |app, event| {
        if let tauri::RunEvent::WindowEvent {
            ref label,
            event: tauri::WindowEvent::Destroyed,
            ..
        } = event
        {
            traffic_lights::forget(label);
        }
        if fix && matches!(event, tauri::RunEvent::MainEventsCleared) {
            traffic_lights::refresh(app);
        }
        if let tauri::RunEvent::ExitRequested {
            api, code: None, ..
        } = event
        {
            api.prevent_exit();
        }
    });
    if !passed.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
    println!("PASS: main and settings preserve native traffic light positions");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This native regression test requires macOS.");
    std::process::exit(1);
}
