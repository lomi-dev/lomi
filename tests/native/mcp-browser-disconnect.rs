//! A real child-webview click outlives its disconnected helper without replay.
use super::*;
fn data(v: &Value) -> &Value {
    &v["structuredContent"]["data"]
}
fn require(ok: bool, message: impl Into<String>) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn receipt(app: &tauri::AppHandle, operation: &str) -> Result<(String, String), String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("agent-control/control.sqlite3");
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
    db.query_row(
        "SELECT state,effect FROM receipts WHERE id=?1",
        [operation],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(|e| e.to_string())
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    settings: &Webview,
    child: &mut tokio::process::Child,
    helper: &Value,
    directory: &Path,
    context: &Value,
) -> Result<(), String> {
    activate_main(app).await?;
    let workspace = &context["workspaceId"];
    let epoch = &context["retryEpoch"];
    let origin = &context["origin"];
    let (_,opened)=layout_call(wire,"lomi_browser_open",json!({"workspaceId":workspace,"url":origin,"retryEpoch":epoch,"requestKey":"disconnect-open"})).await?;
    let target = &data(&opened)["result"];
    let browser = app
        .get_webview(&format!(
            "browser-{}",
            target["panelId"].as_str().ok_or("Missing browser panel")?
        ))
        .ok_or("Missing native browser")?;
    wait_for(
        &browser,
        "document.title==='Disconnect fixture'&&document.readyState==='complete'",
    )
    .await?;
    let snapshot=wire.tool("lomi_browser_snapshot",json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"maxNodes":20,"maxBytes":4096})).await?;
    let snap = data(&snapshot);
    let reference = snap["elements"]
        .as_array()
        .and_then(|e| {
            e.iter()
                .find(|e| e["role"] == "button" && e["name"] == "Record once")
        })
        .map(|e| e["elementRef"].clone())
        .ok_or("Missing actual button reference")?;
    let arguments = json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"navigationId":snap["navigationId"],"snapshotId":snap["snapshotId"],"elementRef":reference,"leaseId":target["leaseId"],"retryEpoch":epoch,"requestKey":"disconnect-click"});
    let accepted = wire.tool("lomi_browser_click", arguments.clone()).await?;
    let operation = data(&accepted)["operationId"]
        .as_str()
        .ok_or_else(|| format!("Click rejected: {accepted}"))?;
    let effect_path = directory.join("browser-disconnect-effect.json");
    let started = tokio::time::Instant::now();
    while !effect_path.exists() && started.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    require(
        effect_path.exists(),
        "Real page did not submit its side effect",
    )?;
    let before = receipt(app, operation)?;
    require(
        before.0 == "running",
        format!("Click was already terminal before disconnect: {before:?}"),
    )?;
    let first_pid = child.id().ok_or("Missing first helper PID")?;
    child.kill().await.map_err(|e| e.to_string())?;
    let mut disconnected = receipt(app, operation)?;
    for _ in 0..100 {
        if disconnected.0 == "outcome_unknown" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        disconnected = receipt(app, operation)?;
    }
    require(
        disconnected.0 == "outcome_unknown" && disconnected.1 == "unknown",
        format!("Disconnected click hid uncertainty: {disconnected:?}"),
    )?;
    wait_for(
        &browser,
        "document.querySelector('#result')?.textContent==='effect:1'",
    )
    .await?;
    require(
        receipt(app, operation)? == disconnected,
        "Late native ACK changed the disconnected receipt",
    )?;
    let mut replacement =
        tokio::process::Command::new(helper["command"].as_str().ok_or("Missing helper path")?)
            .args(
                serde_json::from_value::<Vec<String>>(helper["args"].clone())
                    .map_err(|e| e.to_string())?,
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(
                std::fs::File::create(directory.join("replacement-helper.log"))
                    .map_err(|e| e.to_string())?,
            )
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;
    let second_pid = replacement.id().ok_or("Missing replacement helper")?;
    let mut second = Wire {
        input: replacement.stdin.take().unwrap(),
        output: BufReader::new(replacement.stdout.take().unwrap()),
        id: 0,
    };
    second.call("initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"native-lomi-fixture","version":"1"}})).await?;
    second
        .input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .map_err(|e| e.to_string())?;
    let status = second.tool("lomi_status", json!({})).await?;
    let request = data(&status)["pairingRequestId"]
        .as_str()
        .ok_or("Missing fresh pairing")?;
    require(
        second.tool("lomi_workspace_list", json!({})).await?["structuredContent"]["code"]
            == "PAIRING_REQUIRED",
        "Replacement inherited old grant",
    )?;
    routing_probe::approve(settings, request, workspace, origin, "form", directory).await?;
    let connected = second
        .tool("lomi_connect", json!({"workspaceId":workspace}))
        .await?;
    require(
        connected["structuredContent"]["status"] == "ok",
        format!("Replacement connection failed: {connected}"),
    )?;
    let old_receipt = second
        .tool("lomi_operation_get", json!({"operationId":operation}))
        .await?;
    require(
        old_receipt["structuredContent"]["code"] == "TARGET_NOT_FOUND",
        format!("Replacement receipt read was not scoped as expected: {old_receipt}"),
    )?;
    let old_retry = second.tool("lomi_browser_click", arguments).await?;
    require(
        old_retry["structuredContent"]["status"] == "error"
            && data(&old_retry)["operationId"].is_null(),
        "Old click authority was replayed",
    )?;
    let effects: Value =
        serde_json::from_slice(&std::fs::read(effect_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    require(effects["effects"] == 1, "Click side effect repeated")?;
    let page = evaluate(
        &browser,
        "({title:document.title,text:document.body.innerText})",
    )
    .await?;
    replacement.kill().await.map_err(|e| e.to_string())?;
    std::fs::write(directory.join("browser-disconnect.json"),serde_json::to_vec_pretty(&json!({"passed":true,"helpers":[first_pid,second_pid],"target":target,"snapshot":snapshot,"accepted":accepted,"before":before,"disconnected":disconnected,"afterLateAck":receipt(app,operation)?,"page":page,"effects":effects,"oldReceipt":old_receipt,"oldRetry":old_retry})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
