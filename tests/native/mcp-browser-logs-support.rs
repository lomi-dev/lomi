//! Qualified native console/error/rejection reads over the real helper connection.
use super::*;
fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn read(wire: &mut Wire, args: &Value, kind: &str) -> Result<Value, String> {
    let mut args = args.clone();
    args["logKind"] = json!(kind);
    let value = wire.tool("lomi_browser_logs", args).await?;
    require(
        value["structuredContent"]["status"] == "ok",
        &format!("Native {kind} read failed: {value}"),
    )?;
    Ok(value["structuredContent"]["data"].clone())
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    epoch: &Value,
    directory: &Path,
    fixture: &Value,
) -> Result<(), String> {
    activate_main(app).await?;
    let (_,opened)=layout_call(wire,"lomi_browser_open",json!({"workspaceId":workspace,"url":format!("{}/frames",fixture["origin"].as_str().unwrap()),"retryEpoch":epoch,"requestKey":"logs-open"})).await?;
    let target = &opened["structuredContent"]["data"]["result"];
    let browser = app
        .get_webview(&format!(
            "browser-{}",
            target["panelId"]
                .as_str()
                .ok_or("Missing log browser panel")?
        ))
        .ok_or("Missing native log browser")?;
    wait_for(&browser, "Boolean(document.querySelector('#save'))").await?;
    let args = json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"limit":64});
    evaluate(&browser,"(()=>{window.logGetterCalls=0;const object={get secret(){logGetterCalls++;throw Error('getter-used')},toString(){logGetterCalls++;throw Error('coercion-used')}};console.log('native-console Zażółć 🙂',42,true);console.info('native-info');console.warn('native-warn',object);window.nativeConsoleCalls=0;document.querySelector('#child').contentWindow.console.warn('baseline',{get secret(){nativeConsoleCalls++;throw Error('getter-used')},toString(){nativeConsoleCalls++;throw Error('coercion-used')}});console.error('native-console-error');console.debug('native-debug');document.querySelector('#rejections').click();dispatchEvent(new PromiseRejectionEvent('unhandledrejection',{promise:Promise.resolve(),reason:'synthetic-rejection'}));setTimeout(()=>{throw Error('native-js-error')},0);return true})()").await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let console = read(wire, &args, "console").await?;
    let rejection = read(wire, &args, "promise_rejection").await?;
    let errors = read(wire, &args, "javascript_error").await?;
    require(
        console
            .to_string()
            .contains("native-console Zażółć 🙂 42 true")
            && ["log", "info", "warn", "error", "debug"]
                .iter()
                .all(|level| {
                    console["entries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|e| e["level"] == *level)
                }),
        "Console levels or Unicode omitted",
    )?;
    require(
        console.to_string().contains("[object omitted]")
            && !console.to_string().contains("getter-used")
            && evaluate(
                &browser,
                "window.logGetterCalls === window.nativeConsoleCalls",
            )
            .await?
                == true,
        &format!(
            "Console collector changed native object handling: console={console}; counters={}",
            evaluate(
                &browser,
                "({wrapped:window.logGetterCalls,native:window.nativeConsoleCalls})"
            )
            .await?
        ),
    )?;
    require(
        rejection.to_string().contains("native-rejection")
            && rejection.to_string().contains("[object omitted]")
            && rejection.to_string().contains("synthetic-rejection")
            && rejection["entries"]
                .as_array()
                .is_some_and(|entries| entries.iter().all(|entry| entry["eventTrusted"] == false)),
        &format!(
            "Wrong rejection coverage: {rejection}; pageEvents={}",
            evaluate(&browser, "window.rejectionEvents").await?
        ),
    )?;
    require(
        errors.to_string().contains("native-js-error")
            && !errors.to_string().contains("native-rejection"),
        "Isolated JavaScript errors changed",
    )?;
    let mut checks = vec![
        "five console levels and Unicode",
        "objects omitted without coercion",
        "primitive and opaque Promise reasons",
        "Promise event trust flags reported without authenticating page messages",
        "isolated error stream preserved",
    ];
    let mut cursor = args.clone();
    cursor["cursor"] = console["nextCursor"].clone();
    let empty = read(wire, &cursor, "console").await?;
    require(
        empty["entries"].as_array().is_some_and(|e| e.is_empty()),
        "Console cursor replayed messages",
    )?;
    cursor["logKind"] = json!("promise_rejection");
    require(
        wire.tool("lomi_browser_logs", cursor.clone()).await?["structuredContent"]["code"]
            == "CURSOR_EXPIRED",
        "Cursor crossed log kinds",
    )?;
    checks.push("pagination and kind-bound cursors");
    evaluate(&browser,"(()=>{const original={parse:JSON.parse,stringify:JSON.stringify,push:Array.prototype.push,shift:Array.prototype.shift,slice:String.prototype.slice,now:Date.now};try{JSON.parse=JSON.stringify=Array.prototype.push=Array.prototype.shift=String.prototype.slice=()=>{throw Error('page-override')};Date.now=()=>Infinity;console.log('captured-intrinsics');}finally{JSON.parse=original.parse;JSON.stringify=original.stringify;Array.prototype.push=original.push;Array.prototype.shift=original.shift;String.prototype.slice=original.slice;Date.now=original.now;}let denied=false;try{Object.defineProperty(globalThis,'__lomiAgentPageLogsV1',{value:()=>'{forged}'})}catch{denied=true}return denied})()").await?.as_bool().filter(|v|*v).ok_or("Page replaced private log reader")?;
    require(
        read(wire, &args, "console")
            .await?
            .to_string()
            .contains("captured-intrinsics"),
        "Page replaced collector intrinsics",
    )?;
    checks.push("page mutation cannot replace collector or captured intrinsics");
    evaluate(&browser,"(()=>{for(let i=0;i<90;i++)console.log('bounded-'+i+'x'.repeat(10000));console.error('edge-'+String.fromCharCode(0xd800));return true})()").await?;
    let mut bounded = args.clone();
    bounded["limit"] = json!(10);
    let first = read(wire, &bounded, "console").await?;
    require(
        first["entries"].as_array().map(Vec::len) == Some(10)
            && first["dropped"].as_u64().unwrap_or(0) >= 27
            && first["hasMore"] == true
            && first.to_string().len() < 12000,
        "Console overflow budget failed",
    )?;
    bounded["cursor"] = first["nextCursor"].clone();
    bounded["limit"] = json!(64);
    let rest = read(wire, &bounded, "console").await?;
    require(
        rest["entries"][0]["sequence"].as_u64() > first["entries"][9]["sequence"].as_u64()
            && rest.to_string().contains("edge-�"),
        "Console continuation or Unicode normalization failed",
    )?;
    checks.push("bounded overflow, gap reporting and UTF-16 normalization");
    let mut foreign = args.clone();
    foreign["workspaceId"] = json!("foreign-workspace");
    foreign["logKind"] = json!("console");
    require(
        wire.tool("lomi_browser_logs", foreign).await?["structuredContent"]["code"]
            == "TARGET_NOT_FOUND",
        "Foreign workspace disclosed console",
    )?;
    checks.push("foreign workspace denied");
    let navigated=wire.tool("lomi_browser_navigate",json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"leaseId":target["leaseId"],"url":format!("{}/frames?next",fixture["origin"].as_str().unwrap()),"retryEpoch":epoch,"requestKey":"logs-navigation"})).await?;
    let op = navigated["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| navigated.to_string())?;
    require(
        wire.settled(op).await?["structuredContent"]["data"]["state"] == "succeeded",
        "Logs browser navigation failed",
    )?;
    cursor["logKind"] = json!("console");
    require(
        wire.tool("lomi_browser_logs", cursor).await?["structuredContent"]["code"]
            == "CURSOR_EXPIRED",
        "Navigation retained old log cursor",
    )?;
    require(
        read(wire, &args, "console").await?["entries"]
            .as_array()
            .is_some_and(|e| e.is_empty()),
        "Navigation retained old messages",
    )?;
    checks.push("navigation resets collection and expires cursors");
    let (_,closed)=layout_call(wire,"lomi_panel_close",json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"retryEpoch":epoch,"requestKey":"logs-close"})).await?;
    checks.push("native browser cleanup");
    screenshot(main, directory.join("browser-logs-cleanup.png")).await?;
    std::fs::write(directory.join("browser-expanded-logs.json"),serde_json::to_vec_pretty(&json!({"checks":checks,"console":console,"rejection":rejection,"errors":errors,"bounded":first,"rest":rest,"closed":closed})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
