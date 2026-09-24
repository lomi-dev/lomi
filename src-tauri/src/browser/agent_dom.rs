//! Bounded native DOM adapter; never accepts caller-supplied JavaScript.
use lomi_control_core::browser::BrowserControl;
use lomi_control_protocol::{control::*, ErrorCode};
use serde_json::{json, Value};
use std::{
    sync::{mpsc::sync_channel, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

thread_local! {
    // WebKit caches named worlds only while a strong native reference exists.
    // The world is shared by name; its JavaScript globals remain per document/view.
    static WORLD: std::cell::RefCell<Option<objc2::rc::Retained<objc2_web_kit::WKContentWorld>>> = const { std::cell::RefCell::new(None) };
}

fn world(marker: objc2::MainThreadMarker) -> objc2::rc::Retained<objc2_web_kit::WKContentWorld> {
    WORLD.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| unsafe {
                objc2_web_kit::WKContentWorld::worldWithName(
                    &objc2_foundation::NSString::from_str("LomiAgentDomV1"),
                    marker,
                )
            })
            .clone()
    })
}

pub(super) fn install_logs(native: &objc2_web_kit::WKWebView, marker: objc2::MainThreadMarker) {
    use objc2::MainThreadOnly;
    use objc2_web_kit::{WKUserScript, WKUserScriptInjectionTime};
    let script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(marker),
            &objc2_foundation::NSString::from_str(include_str!("agent-logs.js")),
            WKUserScriptInjectionTime::AtDocumentStart,
            true,
            &world(marker),
        )
    };
    unsafe {
        let controller = native.configuration().userContentController();
        controller.addUserScript(&script);
        let page_logs = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(marker),
            &objc2_foundation::NSString::from_str(include_str!("agent-page-logs.js")),
            WKUserScriptInjectionTime::AtDocumentStart,
            true,
            &objc2_web_kit::WKContentWorld::pageWorld(marker),
        );
        controller.addUserScript(&page_logs);
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LogBatch {
    capture_started_at_millis: u64,
    entries: Vec<BrowserLogEntry>,
    dropped: u64,
    has_more: bool,
    through: u64,
}
pub fn logs(
    app: &AppHandle,
    control: Arc<BrowserControl>,
    input: BrowserLogsInput,
    navigation: String,
    after: u64,
    deadline: Instant,
) -> Result<BrowserLogs, ErrorCode> {
    let value = execute(
        app,
        control.clone(),
        &navigation,
        json!({"action":if input.log_kind == BrowserLogKind::JavascriptError { "logs" } else { "page_logs" }, "logKind":input.log_kind, "after":after, "limit":input.limit}),
        None,
        deadline,
    )?;
    if let Some(error) = value.get("error") {
        return Err(serde_json::from_value(error.clone()).unwrap_or(ErrorCode::OutcomeUnknown));
    }
    let batch: LogBatch = serde_json::from_value(value).map_err(|_| ErrorCode::OutcomeUnknown)?;
    let url = control.document_url()?;
    if !control.permits(&url) {
        return Err(ErrorCode::ScopeDenied);
    }
    let origin = lomi_control_protocol::browser::address(&url)
        .map_err(|_| ErrorCode::ScopeDenied)?
        .origin()
        .ascii_serialization();
    Ok(BrowserLogs {
        workspace_id: input.workspace_id, panel_id: input.panel_id,
        browser_generation: input.browser_generation.clone(), navigation_id: navigation.clone(),
        frame_id: "main".into(), log_kind: input.log_kind, origin,
        capture_started_at_millis: batch.capture_started_at_millis,
        coverage: vec![
            match input.log_kind {
                BrowserLogKind::JavascriptError => "Document-start main-frame error events in an isolated world.",
                BrowserLogKind::Console => "Document-start main-frame log/info/warn/error/debug wrappers in the page world. Replaced console methods bypass later collection.",
                BrowserLogKind::PromiseRejection => "Main-frame PromiseRejectionEvent reports in the page world; primitive reasons only. Includes synthetic reports. eventTrusted preserves the engine flag; WKWebView marks genuine rejections false too.",
            }.into(),
            "Partial coverage; all messages are untrusted page data. Object properties/coercion, stacks, network, headers, cookies, child frames and source URLs are not collected.".into(),
            "64 recent entries per kind; each message is truncated to 256 UTF-16 units before retention. Navigation expires cursors.".into()],
        entries: batch.entries, next_cursor: format!("{}{}", input.log_kind.cursor_prefix(&input.browser_generation, &navigation), batch.through), dropped: batch.dropped, has_more: batch.has_more,
    })
}

pub fn snapshot(
    app: &AppHandle,
    control: Arc<BrowserControl>,
    input: BrowserSnapshotInput,
    snapshot: String,
    navigation: String,
    deadline: Instant,
) -> Result<BrowserSnapshot, ErrorCode> {
    let value = execute(
        app,
        control,
        &navigation,
        json!({"action":"snapshot", "workspaceId":input.workspace_id,
        "panelId":input.panel_id,"browserGeneration":input.browser_generation,"maxNodes":input.max_nodes,
        "maxBytes":input.max_bytes,"snapshotId":snapshot,"navigationId":navigation}),
        None,
        deadline,
    )?;
    if let Some(error) = value.get("error") {
        return Err(serde_json::from_value(error.clone()).unwrap_or(ErrorCode::OutcomeUnknown));
    }
    serde_json::from_value(value).map_err(|_| ErrorCode::OutcomeUnknown)
}

fn execute(
    app: &AppHandle,
    control: Arc<BrowserControl>,
    navigation: &str,
    mut arguments: Value,
    permit: Option<lomi_control_core::broker::NativePermit>,
    deadline: Instant,
) -> Result<Value, ErrorCode> {
    let page_logs = arguments["action"] == "page_logs";
    control.check_document(navigation)?;
    let label = super::label(&control.panel_id).map_err(|_| ErrorCode::TargetNotFound)?;
    {
        let pages = app.state::<super::Browsers>();
        let pages = pages.pages.lock().map_err(|_| ErrorCode::AppUnavailable)?;
        let page = pages
            .get(&control.panel_id)
            .ok_or(ErrorCode::TargetNotFound)?;
        if page
            .control
            .as_ref()
            .is_none_or(|c| !Arc::ptr_eq(c, &control))
        {
            return Err(ErrorCode::StaleGeneration);
        }
    }
    let view = app.get_webview(&label).ok_or(ErrorCode::TargetNotFound)?;
    let native_guard = control.begin_native_dom()?;
    let (answer, receive) = sync_channel(1);
    let navigation = navigation.to_owned();

    view.with_webview(move |platform| {
        use objc2::{runtime::AnyObject, MainThreadMarker};
        use objc2_foundation::{NSDictionary, NSError, NSString};
        use objc2_web_kit::WKWebView;
        let native = unsafe { &*platform.inner().cast::<WKWebView>() };
        let mut dispatch = || -> Result<_, ErrorCode> {
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            if let Some(permit) = &permit {
                permit.check()?;
            }
            control.check_document(&navigation)?;
            if arguments["action"] == "interact" {
                control.require_renderable()?;
                use objc2_app_kit::NSView;
                let native_view: &NSView = native;
                if native_view.isHiddenOrHasHiddenAncestor() {
                    return Err(ErrorCode::PanelNotRenderable);
                }
            }
            let url = unsafe { native.URL() }
                .and_then(|u| u.absoluteString())
                .ok_or(ErrorCode::StaleSnapshot)?
                .to_string();
            if !control.permits(&url) {
                return Err(ErrorCode::ScopeDenied);
            }
            let parsed = lomi_control_protocol::browser::address(&url)
                .map_err(|_| ErrorCode::ScopeDenied)?;
            let clock = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| ErrorCode::AppUnavailable)?;
            arguments["deadlineEpochMs"] =
                ((clock + deadline.saturating_duration_since(Instant::now())).as_millis() as u64)
                    .into();
            arguments["origin"] = parsed.origin().ascii_serialization().into();
            arguments["url"] = url.into();
            Ok(NSString::from_str(&arguments.to_string()))
        };
        let payload = match dispatch() {
            Ok(p) => p,
            Err(e) => {
                let _ = answer.send(Err(e));
                return;
            }
        };
        let answer = Mutex::new(Some(answer));
        let native_guard = Mutex::new(Some(native_guard));
        let callback = block2::RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            // WebKit retains this block while JavaScript is pending. Keep the
            // permit even after the caller times out; never enqueue unbounded work.
            let guard = native_guard.lock().ok().and_then(|mut g| g.take());
            let result = (|| {
                control.check_document(&navigation)?;
                if !error.is_null() || value.is_null() {
                    return Err(ErrorCode::OutcomeUnknown);
                }
                let string = unsafe { &*value }
                    .downcast_ref::<NSString>()
                    .ok_or(ErrorCode::OutcomeUnknown)?;
                if string.length() > 65536 {
                    return Err(ErrorCode::ResourceExhausted);
                }
                let value = string.to_string();
                if value.len() > 65536 {
                    return Err(ErrorCode::ResourceExhausted);
                }
                let value: Value =
                    serde_json::from_str(&value).map_err(|_| ErrorCode::OutcomeUnknown)?;
                Ok(value)
            })();
            drop(guard);
            if let Some(answer) = answer.lock().ok().and_then(|mut a| a.take()) {
                let _ = answer.send(result);
            }
        });
        let key = NSString::from_str("payload");
        let arguments = NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[&payload]);
        let marker = MainThreadMarker::new().unwrap();
        let world = if page_logs {
            unsafe { objc2_web_kit::WKContentWorld::pageWorld(marker) }
        } else {
            world(marker)
        };
        // Only bounded log-read metadata enters the page world. DOM dispatch and
        // snapshot references remain in the private content world.
        let script = if page_logs {
            "return globalThis.__lomiAgentPageLogsV1(payload);"
        } else {
            include_str!("agent-dom.js")
        };
        unsafe {
            native.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                &NSString::from_str(script),
                Some(&arguments),
                None,
                &world,
                Some(&callback),
            );
        }
    })
    .map_err(|_| ErrorCode::AppUnavailable)?;
    receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| ErrorCode::DeadlineExceeded)?
}

pub fn interact(
    app: &AppHandle,
    control: Arc<BrowserControl>,
    operation: &str,
    command: BrowserDomCommand,
    permit: lomi_control_core::broker::NativePermit,
) -> Result<BrowserInteractionResult, ErrorCode> {
    let _guard = control.begin_dom()?;
    control.check_snapshot(&command.snapshot_id, &command.navigation_id)?;
    let mut arguments = serde_json::to_value(&command).map_err(|_| ErrorCode::AppUnavailable)?;
    arguments["action"] = "interact".into();
    let value = execute(
        app,
        control.clone(),
        &command.navigation_id,
        arguments,
        Some(permit),
        Instant::now() + Duration::from_secs(3),
    )?;
    if let Some(error) = value.get("error") {
        let code = serde_json::from_value(error.clone()).unwrap_or(ErrorCode::OutcomeUnknown);
        if value["noEffect"] == true {
            control.record_interaction_rejection(operation, code)?;
        }
        return Err(code);
    }
    let result: BrowserInteractionResult =
        serde_json::from_value(value).map_err(|_| ErrorCode::OutcomeUnknown)?;
    if result.workspace_id != command.workspace_id
        || result.panel_id != command.panel_id
        || result.browser_generation != command.browser_generation
        || result.navigation_id != command.navigation_id
        || result.snapshot_id != command.snapshot_id
        || result.element_ref != command.element_ref
        || !result.dispatched
        || result.input_mode != "synthetic_dom"
    {
        return Err(ErrorCode::OutcomeUnknown);
    }
    control.record_interaction(operation, result.clone())?;
    Ok(result)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Geometry {
    pub url: String,
    pub width: f64,
    pub height: f64,
    pub device_scale_factor: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
}
pub(super) fn geometry(
    app: &AppHandle,
    control: Arc<BrowserControl>,
    navigation: &str,
    deadline: Instant,
) -> Result<Geometry, ErrorCode> {
    let value = execute(
        app,
        control,
        navigation,
        json!({"action":"geometry"}),
        None,
        deadline,
    )?;
    if let Some(error) = value.get("error") {
        return Err(serde_json::from_value(error.clone()).unwrap_or(ErrorCode::OutcomeUnknown));
    }
    serde_json::from_value(value).map_err(|_| ErrorCode::OutcomeUnknown)
}
