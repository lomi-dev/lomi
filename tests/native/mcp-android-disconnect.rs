//! Disconnect an approved real APK stream after its first partial transfer.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn transfer_checkpoint(
    device: &str,
    generation_key: &str,
    port: u16,
    total: u64,
    sent: u64,
) -> Result<(), String> {
    static TRANSFERS: AtomicUsize = AtomicUsize::new(0);
    let count = TRANSFERS.fetch_add(1, Ordering::SeqCst) + 1;
    let directory = PathBuf::from(
        std::env::var_os("LOMI_MCP_CONTROL_PROBE_DIRECTORY")
            .ok_or("Missing native probe directory")?,
    );
    let marker = directory.join("android-transfer-started.json");
    let temporary = marker.with_extension("json.tmp");
    std::fs::write(&temporary,serde_json::to_vec_pretty(&json!({"transfers":count,"deviceId":device,"generationKey":generation_key,"consolePort":port,"totalBytes":total,"sentBytes":sent})).unwrap()).map_err(|e|e.to_string())?;
    std::fs::rename(temporary, marker).map_err(|e| e.to_string())?;
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while !directory.join("android-transfer-release").exists() {
        if std::time::Instant::now() >= until {
            return Err("Native APK transfer fault-injection barrier timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
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
fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
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
    let workspace = &context["workspaceId"];
    let epoch = &context["retryEpoch"];
    let device = read_json(&directory.join("android-fixture.json"))?;
    let build_command = read_json(&directory.join("apk-build-command.json"))?;
    let (_,created)=layout_call(wire,"lomi_terminal_create",json!({"workspaceId":workspace,"cwdRelative":".","title":"APK disconnect fixture","retryEpoch":epoch,"requestKey":"apk-disconnect-terminal"})).await?;
    let terminal = &data(&created)["result"];
    let read_args = json!({"workspaceId":workspace,"panelId":terminal["panelId"],"terminalSessionId":terminal["terminalSessionId"],"maxBytes":65536});
    for _ in 0..100 {
        if data(&wire.tool("lomi_terminal_read", read_args.clone()).await?)["prompt"] == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let build=wire.tool("lomi_terminal_run",json!({"workspaceId":workspace,"panelId":terminal["panelId"],"terminalSessionId":terminal["terminalSessionId"],"leaseId":terminal["leaseId"],"command":build_command["command"],"retryEpoch":epoch,"requestKey":"apk-disconnect-build"})).await?;
    let built = wire
        .settled_with_limit(
            data(&build)["operationId"]
                .as_str()
                .ok_or_else(|| format!("APK build rejected: {build}"))?,
            4800,
        )
        .await?;
    require(
        data(&built)["state"] == "succeeded"
            && data(&built)["result"]["observation"]["exitCode"] == 0,
        format!("APK build failed: {built}"),
    )?;
    let output = wire.tool("lomi_terminal_read", read_args).await?;
    let metadata: Value = data(&output)["text"]
        .as_str()
        .ok_or("Missing real build output")?
        .lines()
        .filter_map(|l| {
            l.find("LOMI_APK_RESULT=")
                .and_then(|i| serde_json::from_str(l[i + 16..].trim()).ok())
        })
        .next_back()
        .ok_or("Missing APK metadata")?;
    require(
        metadata["byteLength"].as_u64().is_some_and(|n| n > 1024),
        "APK is too small for partial transfer",
    )?;
    let (_,imported)=layout_call(wire,"lomi_artifact_import",json!({"workspaceId":workspace,"relativePath":metadata["relativePath"],"kind":"android_apk","expectedByteLength":metadata["byteLength"],"expectedSha256":metadata["sha256"],"retryEpoch":epoch,"requestKey":"apk-disconnect-import"})).await?;
    let artifact = &data(&imported)["result"];
    require(
        artifact["sha256"] == metadata["sha256"],
        "Imported APK hash mismatch",
    )?;
    let (_,opened)=layout_call(wire,"lomi_android_open",json!({"workspaceId":workspace,"deviceId":device["deviceId"],"retryEpoch":epoch,"requestKey":"apk-disconnect-open"})).await?;
    let panel = &data(&opened)["result"]["panelId"];
    let panels = wire
        .tool("lomi_panel_list", json!({"workspaceId":workspace}))
        .await?;
    let started=wire.tool("lomi_android_start",json!({"workspaceId":workspace,"panelId":panel,"deviceId":device["deviceId"],"expectedRevision":data(&panels)["domainRevision"],"retryEpoch":epoch,"requestKey":"apk-disconnect-start"})).await?;
    let running = wire
        .settled_with_limit(
            data(&started)["operationId"]
                .as_str()
                .ok_or("Android start rejected")?,
            7200,
        )
        .await?;
    require(
        data(&running)["state"] == "succeeded" && data(&running)["result"]["ready"] == true,
        format!("Owned Android did not boot: {running}"),
    )?;
    let generation = &data(&running)["result"]["generation"];
    let root = crate::android::fixture::directory()?.ok_or("Missing licensed fixture")?;
    let guest = crate::android::fixture::guest(
        &root,
        device["deviceId"].as_str().ok_or("Missing device")?,
    )?;
    let before_guest = guest.clone();
    let before =
        tauri::async_runtime::spawn_blocking(move || before_guest.native_apk_fixture_hash())
            .await
            .map_err(|e| e.to_string())??;
    let arguments = json!({"workspaceId":workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"artifactId":artifact["artifactId"],"sha256":metadata["sha256"],"retryEpoch":epoch,"requestKey":"apk-disconnect-install"});
    let accepted = wire
        .tool("lomi_android_install_apk", arguments.clone())
        .await?;
    require(
        data(&accepted)["state"] == "awaiting_user",
        "Install skipped exact Settings approval",
    )?;
    let operation = data(&accepted)["operationId"]
        .as_str()
        .ok_or("Install omitted its receipt")?;
    wait_for(
        settings,
        &format!(
            "document.body.textContent.includes({})&&document.body.textContent.includes({})",
            metadata["sha256"], generation
        ),
    )
    .await?;
    let approval=evaluate(settings,"[...document.querySelectorAll('button')].find(e=>e.textContent==='Install this APK')?.closest('article').innerText").await?;
    require(
        approval.as_str().is_some_and(|s| {
            s.contains(metadata["sha256"].as_str().unwrap())
                && s.contains(device["deviceId"].as_str().unwrap())
        }),
        "Wrong one-use install approval",
    )?;
    click(settings, "Install this APK").await?;
    let marker = directory.join("android-transfer-started.json");
    for _ in 0..500 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let transfer = read_json(&marker)?;
    require(
        transfer["transfers"] == 1
            && transfer["deviceId"] == device["deviceId"]
            && transfer["generationKey"] == guest.generation_key
            && transfer["consolePort"] == guest.console_port
            && transfer["totalBytes"] == metadata["byteLength"]
            && transfer["sentBytes"] == 1024,
        "Transfer did not enter the exact owned APK stream",
    )?;
    let before_disconnect = receipt(app, operation)?;
    require(
        before_disconnect.0 == "running",
        "Install completed before the actual disconnect",
    )?;
    let first_pid = child.id().ok_or("Missing helper")?;
    child.kill().await.map_err(|e| e.to_string())?;
    let mut disconnected = receipt(app, operation)?;
    for _ in 0..100 {
        if disconnected.0 == "outcome_unknown" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        disconnected = receipt(app, operation)?;
    }
    // Release only the artificial timing gate; production transport checks the
    // revoked connection permit before sending the next APK bytes.
    std::fs::write(directory.join("android-transfer-release"), b"release")
        .map_err(|e| e.to_string())?;
    require(
        disconnected.0 == "outcome_unknown" && disconnected.1 == "unknown",
        format!("Install concealed uncertainty: {disconnected:?}"),
    )?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after_guest = guest.clone();
    let after = tauri::async_runtime::spawn_blocking(move || after_guest.native_apk_fixture_hash())
        .await
        .map_err(|e| e.to_string())??;
    require(
        before == after,
        "Interrupted partial APK replaced the installed package",
    )?;
    require(
        receipt(app, operation)? == disconnected,
        "Late install completion changed unknown receipt",
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
    routing_probe::approve(settings, request, workspace, &Value::Null, "apk", directory).await?;
    second
        .tool("lomi_connect", json!({"workspaceId":workspace}))
        .await?;
    let old_receipt = second
        .tool("lomi_operation_get", json!({"operationId":operation}))
        .await?;
    require(
        old_receipt["structuredContent"]["code"] == "TARGET_NOT_FOUND",
        "Replacement acquired old receipt",
    )?;
    let old_retry = second.tool("lomi_android_install_apk", arguments).await?;
    require(
        old_retry["structuredContent"]["status"] == "error"
            && data(&old_retry)["operationId"].is_null(),
        "Old installation authority was replayed",
    )?;
    require(
        read_json(&marker)?["transfers"] == 1,
        "Reconnect started a second APK stream",
    )?;
    replacement.kill().await.map_err(|e| e.to_string())?;
    let final_hash = tauri::async_runtime::spawn_blocking(move || guest.native_apk_fixture_hash())
        .await
        .map_err(|e| e.to_string())??;
    require(final_hash == before, "Reconnect changed the installed APK")?;
    std::fs::write(directory.join("android-disconnect.json"),serde_json::to_vec_pretty(&json!({"passed":true,"helpers":[first_pid,second_pid],"build":built,"metadata":metadata,"artifact":artifact,"deviceId":device["deviceId"],"generation":generation,"approval":approval,"transfer":transfer,"accepted":accepted,"beforeDisconnect":before_disconnect,"disconnected":disconnected,"afterLateCompletion":receipt(app,operation)?,"beforePackage":before,"afterPackage":after,"finalPackage":final_hash,"oldReceipt":old_receipt,"oldRetry":old_retry})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
