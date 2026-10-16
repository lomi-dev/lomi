//! Run with `cargo run --manifest-path src-tauri/Cargo.toml --locked
//! --example macos-keyboard-exception`. Unpatched Tao aborts at `sendEvent:`.

#[cfg(target_os = "macos")]
mod probe {
    use objc2::{
        exception, msg_send,
        rc::Retained,
        runtime::{Bool, ClassBuilder, Sel},
        sel, ClassType, MainThreadMarker, MainThreadOnly,
    };
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSEvent, NSEventModifierFlags, NSEventType, NSResponder,
        NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSException, NSPoint, NSRect, NSSize, NSString};
    use std::{
        cell::{Cell, RefCell},
        ffi::CString,
        panic::AssertUnwindSafe,
        sync::mpsc,
        thread,
        time::Duration,
    };

    thread_local! {
        static THROW_NEXT: RefCell<Option<Retained<NSException>>> = const { RefCell::new(None) };
        static LOOP_EXIT: RefCell<Option<tauri::AppHandle>> = const { RefCell::new(None) };
        static LOOP_FAULT: Cell<bool> = const { Cell::new(false) };
        static LOOP_PASSED: Cell<bool> = const { Cell::new(false) };
        static RECEIVED: RefCell<Vec<(NSEventType, u16, NSEventModifierFlags)>> = const { RefCell::new(Vec::new()) };
    }

    extern "C-unwind" fn accepts_first_responder(_: &NSResponder, _: Sel) -> Bool {
        Bool::YES
    }

    extern "C-unwind" fn key_down(_: &NSResponder, _: Sel, event: &NSEvent) {
        record(event);
        let exception = THROW_NEXT.with(|slot| slot.borrow_mut().take());
        if let Some(exception) = exception {
            if LOOP_EXIT.with(|slot| slot.borrow().is_some()) {
                LOOP_FAULT.set(true);
            }
            // Preserve Objective-C unwinding through this test-owned responder.
            exception::throw(unsafe { Retained::cast_unchecked(exception) });
        }
        if event.keyCode() == 0 && LOOP_FAULT.get() {
            LOOP_EXIT.with(|slot| {
                if let Some(app) = slot.borrow_mut().take() {
                    LOOP_PASSED.set(true);
                    app.exit(0);
                }
            });
        }
    }

    extern "C-unwind" fn key_up(_: &NSResponder, _: Sel, event: &NSEvent) {
        record(event);
    }

    fn record(event: &NSEvent) {
        RECEIVED.with(|events| {
            events
                .borrow_mut()
                .push((event.r#type(), event.keyCode(), event.modifierFlags()));
        });
    }

    fn responder() -> Retained<NSResponder> {
        let name = CString::new("LomiKeyboardExceptionResponder").unwrap();
        let mut class = ClassBuilder::new(&name, NSResponder::class()).unwrap();
        unsafe {
            class.add_method(
                sel!(acceptsFirstResponder),
                accepts_first_responder as extern "C-unwind" fn(_, _) -> _,
            );
            class.add_method(sel!(keyDown:), key_down as extern "C-unwind" fn(_, _, _));
            class.add_method(sel!(keyUp:), key_up as extern "C-unwind" fn(_, _, _));
            msg_send![class.register(), new]
        }
    }

    fn event(
        window: &NSWindow,
        kind: NSEventType,
        flags: NSEventModifierFlags,
        characters: &str,
        code: u16,
    ) -> Retained<NSEvent> {
        let characters = NSString::from_str(characters);
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            kind,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            window.windowNumber(),
            None,
            &characters,
            &characters,
            false,
            code,
        )
        .unwrap()
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().unwrap();
        let mut context = tauri::generate_context!();
        for window in &mut context.config_mut().app.windows {
            window.create = false;
        }
        let app = tauri::Builder::default().build(context).unwrap();
        let native = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 160.0)),
                NSWindowStyleMask::Titled,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        let application = NSApplication::sharedApplication(mtm);
        let responder = responder();
        let window = tauri::WindowBuilder::new(&app, "keyboard-exception")
            .title("Keyboard exception regression")
            .inner_size(320.0, 160.0)
            .visible(false)
            .build()
            .unwrap();
        let (finished_tx, finished_rx) = mpsc::channel();
        thread::spawn(move || {
            if finished_rx.recv_timeout(Duration::from_secs(20)).is_err() {
                eprintln!("FAIL: native keyboard exception probe timed out");
                std::process::exit(1);
            }
        });
        let mut ready = false;
        let mut checked = false;
        let exit_code = app.run_return(move |handle, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                eprintln!("Native application ready");
                #[allow(deprecated)]
                application.activateIgnoringOtherApps(true);
                native.makeKeyAndOrderFront(None);
                eprintln!(
                    "Native window ready: active={}, visible={}, key={}",
                    application.isActive(),
                    native.isVisible(),
                    native.isKeyWindow()
                );
                ready = true;
            }
            if ready
                && !checked
                && matches!(event, tauri::RunEvent::MainEventsCleared)
                && native.isKeyWindow()
            {
                checked = true;
                // A base NSWindow isolates TaoApp's sendEvent: override first.
                eprintln!("Checking TaoApp dispatch through a base NSWindow");
                check_window(&application, &native, &responder);
                native.orderOut(None);

                // Keep the Tauri window alive until the posted-event stage exits.
                let tao_native = unsafe { &*window.ns_window().unwrap().cast::<NSWindow>() };
                eprintln!("Checking TaoApp dispatch through a TaoWindow");
                check_window(&application, tao_native, &responder);
                assert!(tao_native.makeFirstResponder(Some(&responder)));
                RECEIVED.with(|events| events.borrow_mut().clear());
                LOOP_EXIT.with(|slot| *slot.borrow_mut() = Some(handle.clone()));
                THROW_NEXT.with(|slot| *slot.borrow_mut() = Some(controlled_exception()));
                let throwing = event_for_loop(tao_native, "x", 7);
                let following = event_for_loop(tao_native, "a", 0);
                // AppKit's own NSApplication.run handler must handle this throw.
                // Queue order ensures the normal key is delivered after the fault.
                application.postEvent_atStart(&throwing, false);
                application.postEvent_atStart(&following, false);
            }
            if let tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } = event
            {
                api.prevent_exit();
            }
        });
        finished_tx.send(()).unwrap();
        assert_eq!(exit_code, 0);
        assert!(
            LOOP_PASSED.get(),
            "AppKit must dispatch the key after the exception"
        );
        RECEIVED.with(|events| {
            assert_eq!(
                *events.borrow(),
                [
                    (NSEventType::KeyDown, 7, NSEventModifierFlags::empty()),
                    (NSEventType::KeyDown, 0, NSEventModifierFlags::empty()),
                ]
            );
        });
        println!("PASS: direct dispatch preserves exceptions and AppKit continues its event loop");
    }

    fn event_for_loop(window: &NSWindow, characters: &str, code: u16) -> Retained<NSEvent> {
        event(
            window,
            NSEventType::KeyDown,
            NSEventModifierFlags::empty(),
            characters,
            code,
        )
    }

    fn controlled_exception() -> Retained<NSException> {
        unsafe {
            NSException::exceptionWithName_reason_userInfo(
                &NSString::from_str("LomiKeyboardProbeException"),
                Some(&NSString::from_str("Controlled keyDown exception")),
                None,
            )
        }
    }

    fn check_window(application: &NSApplication, native: &NSWindow, responder: &NSResponder) {
        RECEIVED.with(|events| events.borrow_mut().clear());
        native.makeKeyAndOrderFront(None);
        assert!(native.isKeyWindow());
        assert!(native.makeFirstResponder(Some(responder)));

        let expected = controlled_exception();
        let expected_pointer = Retained::as_ptr(&expected).cast::<()>();
        THROW_NEXT.with(|slot| *slot.borrow_mut() = Some(expected.clone()));
        let throwing = event(
            native,
            NSEventType::KeyDown,
            NSEventModifierFlags::empty(),
            "x",
            7,
        );
        // Catch the original exception outside the nested Tao sendEvent: overrides.
        let caught = exception::catch(AssertUnwindSafe(|| application.sendEvent(&throwing)))
            .expect_err("keyDown must propagate the controlled exception")
            .expect("the exception must not be nil");
        assert_eq!(Retained::as_ptr(&caught).cast::<()>(), expected_pointer);
        println!("Caught the original responder exception through NSApplication.sendEvent");

        let cases = [
            (NSEventType::KeyDown, NSEventModifierFlags::empty(), "a", 0),
            (NSEventType::KeyDown, NSEventModifierFlags::Shift, "\r", 36),
            (
                NSEventType::KeyDown,
                NSEventModifierFlags::empty(),
                "\u{f702}",
                123,
            ),
            (NSEventType::KeyUp, NSEventModifierFlags::Command, "a", 0),
        ];
        for (kind, flags, characters, code) in cases {
            let event = event(native, kind, flags, characters, code);
            exception::catch(AssertUnwindSafe(|| application.sendEvent(&event)))
                .expect("subsequent keyboard dispatch must succeed");
        }
        let mut expected_events = vec![(NSEventType::KeyDown, 7, NSEventModifierFlags::empty())];
        expected_events.extend(cases.map(|(kind, flags, _, code)| (kind, code, flags)));
        RECEIVED.with(|events| assert_eq!(*events.borrow(), expected_events));
        assert!(native.makeFirstResponder(None));
        println!(
            "PASS: normal, Shift+Enter, ArrowLeft and Command keyUp still reach the responder"
        );
    }
}

#[cfg(target_os = "macos")]
fn main() {
    probe::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This native regression test requires macOS.");
    std::process::exit(1);
}
