//! Native-only fixture; external pages receive no probe IPC permissions.
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{Manager, Webview};
async fn evaluate(view: &Webview, script: &str) -> Result<Value, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let send = Mutex::new(Some(send));
    view.eval_with_callback(script, move |text| {
        if let Some(send) = send.lock().unwrap().take() {
            let result = if text.len() > 65536 {
                Err("Fixture result too large".into())
            } else {
                serde_json::from_str(&text).map_err(|e| e.to_string())
            };
            let _ = send.send(result);
        }
    })
    .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(5), receive)
        .await
        .map_err(|_| "Fixture JS timeout")?
        .map_err(|_| "Fixture callback dropped")?
}
async fn snapshot(
    app: &tauri::AppHandle,
    directory: &std::path::Path,
    case: &str,
    index: usize,
) -> Value {
    let app = app.clone();
    match tauri::async_runtime::spawn_blocking(move || {
        crate::browser::remote_probe::capture(
            &app,
            "remote-capture-fixture",
            Instant::now() + Duration::from_secs(3),
        )
    })
    .await
    {
        Ok(Ok(frame)) => {
            let _ = std::fs::write(directory.join(format!("{case}-{index}.jpg")), frame.jpeg);
            json!({"metrics":frame.metrics,"center":frame.center,"clockPixel":frame.clock_pixel,"contentPixels":frame.content_pixels})
        }
        Ok(Err(error)) => json!({"error":error}),
        Err(error) => json!({"error":error.to_string()}),
    }
}
async fn run(app: &tauri::AppHandle, directory: &std::path::Path) -> Result<Value, String> {
    let window = app.get_window("main").ok_or("Main missing")?;
    let fixture_url =
        std::env::var("LOMI_REMOTE_BROWSER_FIXTURE_URL").map_err(|_| "Fixture URL missing")?;
    let parsed = tauri::Url::parse(&fixture_url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "http" || parsed.host_str() != Some("127.0.0.1") {
        return Err("Fixture requires loopback HTTP".into());
    }
    let slot = serde_json::from_value(json!({"id":"remote-capture-fixture","url":fixture_url,"bounds":{"x":10.,"y":100.,"width":800.,"height":600.}})).map_err(|e| e.to_string())?;
    crate::browser::sync_browsers(
        window.clone(),
        vec!["remote-capture-fixture".into()],
        vec![slot],
    )
    .await?;
    let mut view = None;
    for _ in 0..300 {
        if let Some(v) = app.get_webview("browser-remote-capture-fixture") {
            if evaluate(&v, "Boolean(window.fixtureReady)")
                .await
                .unwrap_or(Value::Null)
                == true
            {
                view = Some(v);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let view = view.ok_or("Ordinary fixture page did not load")?;
    let initial = evaluate(
        &view,
        "({cookie:document.cookie,counter:window.fixtureCounter,href:location.href,webgl:window.fixtureWebgl,rafTicks:window.fixtureRafTicks})",
    )
    .await?;
    if !initial["cookie"]
        .as_str()
        .is_some_and(|s| s.contains("remote_capture_fixture="))
    {
        return Err("Fixture cookie missing".into());
    }
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    let active_slot = serde_json::from_value(json!({"id":"remote-capture-fixture","url":initial["href"],"bounds":{"x":10.,"y":100.,"width":800.,"height":600.}})).map_err(|e| e.to_string())?;
    crate::browser::sync_browsers(
        window.clone(),
        vec!["remote-capture-fixture".into()],
        vec![active_slot],
    )
    .await?;
    let mut cases = Vec::new();
    for case in [
        "active",
        "hidden-native",
        "inactive-tab",
        "inactive-workspace",
        "window-hidden",
        "minimized",
    ] {
        match case {
            "hidden-native" => view.hide().map_err(|e| e.to_string())?,
            "inactive-tab" | "inactive-workspace" => {
                crate::browser::sync_browsers(
                    window.clone(),
                    vec!["remote-capture-fixture".into()],
                    vec![],
                )
                .await?;
            }
            "window-hidden" => {
                view.show().map_err(|e| e.to_string())?;
                window.hide().map_err(|e| e.to_string())?;
            }
            "minimized" => {
                window.show().map_err(|e| e.to_string())?;
                window.minimize().map_err(|e| e.to_string())?;
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let case_start_ticks = evaluate(&view, "window.fixtureTicks").await?;
        let mut frames = Vec::new();
        for index in 0..2 {
            // A native fixture stimulus updates the same ordinary DOM without app IPC.
            let stimulus=evaluate(&view,&format!("window.fixturePaint({}); ({{counter:window.fixtureCounter,color:window.fixtureColor,cookie:document.cookie,clock:performance.now(),ticks:window.fixtureTicks}})",if index==0 {40} else {180})).await?;
            tokio::time::sleep(Duration::from_millis(1200)).await;
            let content_before = evaluate(&view, "window.fixtureRafTicks").await?;
            let before = evaluate(&view, "window.fixtureTicks").await?;
            let frame = snapshot(app, directory, case, index).await;
            let after = evaluate(&view, "window.fixtureTicks").await?;
            let content_after = evaluate(&view, "window.fixtureRafTicks").await?;
            let clock_pixel = frame["clockPixel"][0].as_u64();
            let clock_fresh = match (before.as_u64(), after.as_u64(), clock_pixel) {
                (Some(a), Some(b), Some(p)) if b - a < 250 => {
                    (a..=b).any(|n| (n % 256).abs_diff(p) <= 2)
                }
                _ => false,
            };
            let expected = stimulus["color"].as_array();
            let actual = frame["center"].as_array();
            let fresh = match (expected, actual) {
                (Some(a), Some(b)) => a.iter().zip(b).all(|(x, y)| {
                    (x.as_i64().unwrap_or(-999) - y.as_i64().unwrap_or(999)).abs() <= 4
                }),
                _ => false,
            };
            let content_fresh = [
                ("canvas2d", 90, 150),
                ("webgl", 120, 120),
                ("rafDom", 60, 180),
            ]
            .into_iter()
            .map(|(kind, g, b)| {
                let pixel = &frame["contentPixels"][kind];
                let fresh = match (
                    content_before.as_u64(),
                    content_after.as_u64(),
                    pixel[0].as_u64(),
                    pixel[1].as_i64(),
                    pixel[2].as_i64(),
                ) {
                    (Some(start), Some(end), Some(red), Some(got_g), Some(got_b))
                        if end - start < 200 =>
                    {
                        (start..=end).any(|n| (30 + n % 200).abs_diff(red) <= 2)
                            && (got_g - g).abs() <= 2
                            && (got_b - b).abs() <= 2
                    }
                    _ => false,
                };
                (kind.to_string(), json!({"freshPixels":fresh,"pixel":pixel}))
            })
            .collect::<serde_json::Map<String, Value>>();
            let same_session = stimulus["cookie"] == initial["cookie"];
            frames.push(json!({"contentBefore":content_before,"contentAfter":content_after,"contentFreshness":content_fresh,"sameSessionCookie":same_session,"stimulus":stimulus,"frame":frame,"freshPixels":fresh,"clockFreshPixels":clock_fresh,"clockBefore":before,"clockAfter":after}));
        }
        let clock_advanced = frames
            .last()
            .and_then(|f| f["clockAfter"].as_u64())
            .zip(case_start_ticks.as_u64())
            .is_some_and(|(end, start)| end > start);
        let visibility_matches = frames.iter().all(|f| match case {
            "active" => {
                f["frame"]["metrics"]["visibility"]["windowVisible"] == true
                    && f["frame"]["metrics"]["visibility"]["hidden"] == false
                    && f["frame"]["metrics"]["visibility"]["registeredVisible"] == true
            }
            "hidden-native" => f["frame"]["metrics"]["visibility"]["hidden"] == true,
            "inactive-tab" | "inactive-workspace" => {
                f["frame"]["metrics"]["visibility"]["registeredVisible"] == false
                    && f["frame"]["metrics"]["visibility"]["hidden"] == true
            }
            "window-hidden" => f["frame"]["metrics"]["visibility"]["windowVisible"] == false,
            "minimized" => f["frame"]["metrics"]["visibility"]["minimized"] == true,
            _ => true,
        });
        let fresh = visibility_matches
            && clock_advanced
            && frames.iter().all(|f| {
                f["freshPixels"] == true
                    && f["clockFreshPixels"] == true
                    && f["sameSessionCookie"] == true
            });
        let autonomous_raf_advanced = frames
            .last()
            .and_then(|f| f["contentAfter"].as_u64())
            .zip(frames.first().and_then(|f| f["contentBefore"].as_u64()))
            .is_some_and(|(end, start)| end > start);
        let content_fresh = ["canvas2d","webgl","rafDom"].into_iter().map(|kind| (kind.to_string(),json!({"freshPixels":frames.iter().all(|f|f["contentFreshness"][kind]["freshPixels"]==true),"pixelsChanged":frames.first().map(|f|&f["frame"]["contentPixels"][kind]) != frames.last().map(|f|&f["frame"]["contentPixels"][kind]),"autonomousRafAdvanced":autonomous_raf_advanced}))).collect::<serde_json::Map<String,Value>>();
        cases.push(json!({"case":case,"fresh":fresh,"contentQualification":content_fresh,"visibilityMatchesCase":visibility_matches,"autonomousClockAdvanced":clock_advanced,"frames":frames}));
    }
    let retained_host_experiments = host_experiments(app, &view, directory).await?;
    window.unminimize().map_err(|e| e.to_string())?;
    // Start then close immediately; a settled frame must not survive native identity removal.
    let capture_app = app.clone();
    let pending = tauri::async_runtime::spawn_blocking(move || {
        crate::browser::remote_probe::capture(
            &capture_app,
            "remote-capture-fixture",
            Instant::now() + Duration::from_secs(3),
        )
    });
    crate::browser::sync_browsers(window, vec![], vec![]).await?;
    let close_result = match pending.await {
        Ok(Ok(_)) => "frame returned; ordering relative to close fence is unobserved".to_string(),
        Ok(Err(e)) => e,
        Err(e) => e.to_string(),
    };
    let go = cases.iter().all(|case| case["fresh"] == true);
    let background_content_candidate = cases.iter().all(|case| {
        ["canvas2d", "webgl", "rafDom"].into_iter().all(|kind| {
            case["contentQualification"][kind]["freshPixels"] == true
                && case["contentQualification"][kind]["autonomousRafAdvanced"] == true
                && case["contentQualification"][kind]["pixelsChanged"] == true
        })
    });
    Ok(
        json!({"retainedHostExperiments":retained_host_experiments,"backgroundContentCandidate":background_content_candidate,"fullBrowserBackgroundDecision":"NO-GO: lifecycle races, long-run budgets, video/DRM/cross-origin and full UI transitions remain unqualified","g1BrowserDecision":if go {"GO for tested DOM visibility cases only"}else{"NO-GO"},"initial":initial,"cases":cases,"closeRace":close_result,"closeQualification":"inconclusive: no native submission/identity removal barrier","limitations":["Controlled fixture main creates an ordinary Page through the native factory; inactive tab/workspace reproduce sync retained-hidden transitions, not full product UI switching","Autonomous timer DOM, RAF DOM, canvas2D and WebGL clear pixels are measured; native-controlled DOM colors are separate stimuli","Close callback ordering, split browser identity, occlusion, menus/drag, lock/sleep, zoom/navigation callback races and 30 minute budgets remain unqualified","Video/DRM and cross-origin frames remain unqualified; per-case canvas/WebGL/RAF results must be read separately from DOM visibility","No signed grant, relay, Worker decode or production remote admission is enabled"]}),
    )
}
pub fn start(app: tauri::AppHandle) {
    let Some(directory) =
        std::env::var_os("LOMI_REMOTE_BROWSER_CAPTURE_DIRECTORY").map(PathBuf::from)
    else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let result = run(&app, &directory).await;
        if let Some(view) = app.get_webview("browser-remote-capture-fixture") {
            let _ = render_host(&view, None).await;
        }
        let failed = result.is_err();
        let report = match result {
            Ok(data) => json!({"stage":"completed","data":data}),
            Err(error) => json!({"stage":"failed","error":error}),
        };
        let _ = std::fs::write(
            directory.join("result.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        );
        app.exit(if failed { 1 } else { 0 });
    });
}

struct RetainedHost {
    window: objc2::rc::Retained<objc2_app_kit::NSWindow>,
    view: objc2::rc::Retained<objc2_web_kit::WKWebView>,
    parent: objc2::rc::Retained<objc2_app_kit::NSView>,
    frame: objc2_foundation::NSRect,
    hidden: bool,
}
thread_local! { static HOST: std::cell::RefCell<Option<RetainedHost>> = const { std::cell::RefCell::new(None) }; }

async fn render_host(view: &Webview, variant: Option<&str>) -> Result<Value, String> {
    let variant = variant.map(str::to_owned);
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
        use objc2_app_kit::{NSApplication, NSBackingStoreType, NSScreen, NSView, NSWindow, NSWindowAnimationBehavior, NSWindowOcclusionState, NSWindowStyleMask};
        use objc2_foundation::{NSPoint, NSRect};
        use objc2_web_kit::WKWebView;
        let main=MainThreadMarker::new().unwrap();
        let native=unsafe { &*platform.inner().cast::<WKWebView>() };
        let nsview: &NSView=native;
        let application=NSApplication::sharedApplication(main);
        let key_before=application.keyWindow().map(|w|w.windowNumber());
        let main_before=application.mainWindow().map(|w|w.windowNumber());
        let result=(|| {
            HOST.with(|slot| {
                let mut slot=slot.borrow_mut();
                let mut restored=json!({"restored":true});
                if let Some(old)=slot.take() {
                    old.view.removeFromSuperview(); old.parent.addSubview(&old.view);
                    old.view.setFrame(old.frame); old.view.setHidden(old.hidden);
                    let restored_parent=unsafe { old.view.superview() }.as_ref().is_some_and(|p|std::ptr::eq(&**p,&*old.parent));
                    restored=json!({"restored":true,"parentMatches":restored_parent,"frameMatches":old.view.frame()==old.frame,"hiddenMatches":old.view.isHidden()==old.hidden,"restoredWindowNumber":old.view.window().map(|w|w.windowNumber())});
                    old.window.orderOut(None); old.window.close();
                }
                let Some(variant)=variant.as_deref() else { return Ok(restored); };
                let parent=unsafe { nsview.superview() }.ok_or("Source parent missing")?;
                let source_window=nsview.window().ok_or("Source window missing")?;
                let frame=nsview.frame(); let bounds=nsview.bounds(); let hidden=nsview.isHidden();
                let source_scale=source_window.backingScaleFactor();
                let screens=NSScreen::screens(main);
                let screen=source_window.screen().or_else(||screens.firstObject()).ok_or("Screen missing")?;
                let screen_frame=screen.frame();
                let max_x=screens.iter().map(|s|s.frame().origin.x+s.frame().size.width).fold(f64::NEG_INFINITY,f64::max);
                let origin=if variant=="offscreen" {NSPoint::new(max_x+100.,screen_frame.origin.y+100.)} else {NSPoint::new(screen_frame.origin.x+100.,screen_frame.origin.y+100.)};
                let host=unsafe { NSWindow::initWithContentRect_styleMask_backing_defer_screen(NSWindow::alloc(main),NSRect::new(origin,bounds.size),NSWindowStyleMask::Borderless,NSBackingStoreType::Buffered,false,Some(&screen)) };
                if variant=="offscreen" && host.frame().origin.x < max_x { return Err("Refuse on-screen opaque offscreen fixture host".into()); }
                unsafe { host.setReleasedWhenClosed(false); }
                host.setAnimationBehavior(NSWindowAnimationBehavior::None);
                host.setIgnoresMouseEvents(true); host.setHidesOnDeactivate(false); host.setCanHide(false);
                if variant=="transparent-onscreen" { host.setAlphaValue(0.); }
                let source=unsafe {Retained::retain(native as *const WKWebView as *mut WKWebView)}.ok_or("Cannot retain source")?;
                let host_content=host.contentView().ok_or("Host content view missing")?;
                nsview.removeFromSuperview();
                host_content.addSubview(nsview);
                nsview.setFrame(NSRect::new(NSPoint::new(0.,0.),bounds.size)); nsview.setHidden(false);
                // Public ordering preserves key/main window. The transparent variant
                // must be front-ordered to measure its actual occlusion visibility.
                if variant=="transparent-onscreen" {host.orderFront(None);} else {host.orderBack(None);}
                let report=json!({"variant":variant,"sourceScaleBefore":source_scale,"sourceScaleAfter":host.backingScaleFactor(),"cssWidthBefore":bounds.size.width,"cssHeightBefore":bounds.size.height,"cssWidthAfter":nsview.bounds().size.width,"cssHeightAfter":nsview.bounds().size.height,"hostOrigin":[host.frame().origin.x,host.frame().origin.y],"alpha":host.alphaValue(),"ignoresMouseEvents":host.ignoresMouseEvents(),"canBecomeKey":host.canBecomeKeyWindow(),"canBecomeMain":host.canBecomeMainWindow(),"visible":host.isVisible(),"occlusionVisible":host.occlusionState().contains(NSWindowOcclusionState::Visible)});
                *slot=Some(RetainedHost {window:host,view:source,parent,frame,hidden});
                Ok(report)
            })
        })();
        let result=result.map(|mut data:Value| { data["keyBefore"]=json!(key_before);data["keyAfter"]=json!(application.keyWindow().map(|w|w.windowNumber()));data["mainBefore"]=json!(main_before);data["mainAfter"]=json!(application.mainWindow().map(|w|w.windowNumber()));data });
        let _=send.send(result);
    }).map_err(|e|e.to_string())?;
    tokio::time::timeout(Duration::from_secs(5), receive)
        .await
        .map_err(|_| "Render host callback timeout")?
        .map_err(|_| "Render host callback dropped")?
}

async fn host_experiments(
    app: &tauri::AppHandle,
    view: &Webview,
    directory: &std::path::Path,
) -> Result<Value, String> {
    let window = app.get_window("main").ok_or("Main missing")?;
    let mut variants = Vec::new();
    for variant in ["offscreen", "transparent-onscreen"] {
        window.unminimize().map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        let state_before=evaluate(view,"({cookie:document.cookie,url:location.href,counter:window.fixtureCounter,form:document.querySelector('#draft')?.value,dpr:devicePixelRatio,width:innerWidth,height:innerHeight})").await?;
        let setup = render_host(view, Some(variant)).await?;
        let mut cases = Vec::new();
        for parent_case in ["normal", "parent-hidden", "parent-minimized"] {
            match parent_case {
                "parent-hidden" => window.hide().map_err(|e| e.to_string())?,
                "parent-minimized" => {
                    window.show().map_err(|e| e.to_string())?;
                    window.minimize().map_err(|e| e.to_string())?;
                }
                _ => {}
            }
            let mut frames = Vec::new();
            for index in 0..2 {
                tokio::time::sleep(Duration::from_millis(1200)).await;
                let before = evaluate(view, "window.fixtureRafTicks").await?;
                let frame = snapshot(
                    app,
                    directory,
                    &format!("host-{variant}-{parent_case}"),
                    index,
                )
                .await;
                let after = evaluate(view, "window.fixtureRafTicks").await?;
                let fresh = [
                    ("canvas2d", 90, 150),
                    ("webgl", 120, 120),
                    ("rafDom", 60, 180),
                ]
                .into_iter()
                .map(|(kind, g, b)| {
                    let p = &frame["contentPixels"][kind];
                    let valid = match (
                        before.as_u64(),
                        after.as_u64(),
                        p[0].as_u64(),
                        p[1].as_i64(),
                        p[2].as_i64(),
                    ) {
                        (Some(a), Some(z), Some(r), Some(pg), Some(pb)) if z - a < 200 => {
                            (a..=z).any(|n| (30 + n % 200).abs_diff(r) <= 2)
                                && (pg - g).abs() <= 2
                                && (pb - b).abs() <= 2
                        }
                        _ => false,
                    };
                    (kind.to_string(), json!(valid))
                })
                .collect::<serde_json::Map<String, Value>>();
                let parent_visible = window.is_visible().map_err(|e| e.to_string())?;
                let parent_minimized = window.is_minimized().map_err(|e| e.to_string())?;
                let parent_matches = match parent_case {
                    "normal" => parent_visible && !parent_minimized,
                    "parent-hidden" => !parent_visible && !parent_minimized,
                    "parent-minimized" => parent_minimized,
                    _ => false,
                };
                frames.push(json!({"originalParentState":{"visible":parent_visible,"minimized":parent_minimized,"matchesCase":parent_matches},"rafBefore":before,"rafAfter":after,"freshPixels":fresh,"frame":frame}));
            }
            let advanced = frames
                .last()
                .and_then(|f| f["rafAfter"].as_u64())
                .zip(frames.first().and_then(|f| f["rafBefore"].as_u64()))
                .is_some_and(|(b, a)| b > a);
            let pixels_changed = ["canvas2d", "webgl", "rafDom"].into_iter().all(|kind| {
                frames.first().map(|f| &f["frame"]["contentPixels"][kind])
                    != frames.last().map(|f| &f["frame"]["contentPixels"][kind])
            });
            let transition_observed = frames
                .iter()
                .all(|f| f["originalParentState"]["matchesCase"] == true);
            let fresh = transition_observed
                && advanced
                && pixels_changed
                && frames.iter().all(|f| {
                    ["canvas2d", "webgl", "rafDom"]
                        .into_iter()
                        .all(|kind| f["freshPixels"][kind] == true)
                });
            cases.push(json!({"parentCase":parent_case,"parentTransitionObserved":transition_observed,"autonomousRafAdvanced":advanced,"pixelsChanged":pixels_changed,"fresh":fresh,"frames":frames}));
        }
        let state_after=evaluate(view,"({cookie:document.cookie,url:location.href,counter:window.fixtureCounter,form:document.querySelector('#draft')?.value,dpr:devicePixelRatio,width:innerWidth,height:innerHeight})").await?;
        let restore = render_host(view, None).await?;
        let restoration_preserved = restore["parentMatches"] == true
            && restore["frameMatches"] == true
            && restore["hiddenMatches"] == true;
        let focus_preserved =
            setup["keyBefore"] == setup["keyAfter"] && setup["mainBefore"] == setup["mainAfter"];
        let state_preserved =
            state_before == state_after && setup["sourceScaleBefore"] == setup["sourceScaleAfter"];
        variants.push(json!({"variant":variant,"setup":setup,"stateBefore":state_before,"stateAfter":state_after,"sourceStatePreserved":state_preserved,"keyAndMainWindowsPreservedAtReparent":focus_preserved,"cases":cases,"restore":restore,"candidate":state_preserved&&focus_preserved&&restoration_preserved&&cases.iter().all(|c|c["fresh"]==true),"limitations":["Only key/main window IDs are checked; first responder, DOM editing focus, selection, IME and undo history remain unqualified. Reparent may clear editing undo commands","On-screen transparent ordered window is an owned native window; hit testing and focus flags are measured, lifecycle and Spaces policy require separate qualification","No video/DRM/cross-origin qualification"]}));
    }
    Ok(json!(variants))
}
