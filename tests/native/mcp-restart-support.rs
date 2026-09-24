//! Actual native process restart preserves layout, never command authority.
use super::*;
fn data(v: &Value) -> &Value {
    &v["structuredContent"]["data"]
}
fn require(ok: bool, detail: impl Into<String>) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(detail.into())
    }
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    workspace: &Value,
    connected: &Value,
    directory: &Path,
) -> Result<(), String> {
    let phase = std::env::var("LOMI_MCP_RESTART_PHASE").map_err(|e| e.to_string())?;
    let main = app.get_webview("main").ok_or("Missing main")?;
    let baseline = directory.join("restart-baseline.json");
    if phase == "first" {
        let (_,created)=layout_call(wire,"lomi_terminal_create",json!({"workspaceId":workspace,"cwdRelative":".","title":"Restart fixture","retryEpoch":data(connected)["retryEpoch"],"requestKey":"restart-terminal"})).await?;
        let target = data(&created)["result"].clone();
        for _ in 0..100 {
            let read=wire.tool("lomi_terminal_read",json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"]})).await?;
            if data(&read)["prompt"] == "ready" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let args = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"retryEpoch":data(connected)["retryEpoch"],"requestKey":"restart-command","command":"printf 'once\\n' >> restart-once.txt; printf 'RESTART_ONCE_WRITTEN\\n'"});
        let started = wire.tool("lomi_terminal_run", args.clone()).await?;
        let operation = data(&started)["operationId"]
            .as_str()
            .ok_or("Restart command rejected")?;
        let done = wire.settled(operation).await?;
        require(
            data(&done)["result"]["observation"]["exitCode"] == 0,
            "First command did not finish",
        )?;
        let native=javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});return {{sessionId:r?.sessionId,promptReady:Boolean(r?.atPrompt&&!r?.activeBlock)}};",target["panelId"])).await?;
        require(
            native["sessionId"] == target["terminalSessionId"] && native["promptReady"] == true,
            "Original retained PTY not ready",
        )?;
        require(
            std::fs::read(directory.join("project/restart-once.txt")).map_err(|e| e.to_string())?
                == b"once\n",
            "Initial effect not singular",
        )?;
        let panels = wire
            .tool("lomi_panel_list", json!({"workspaceId":workspace}))
            .await?;
        let tab_id = data(&panels)["items"]
            .as_array()
            .and_then(|items| items.iter().find(|p| p["id"] == target["panelId"]))
            .map(|p| p["tabId"].clone())
            .ok_or("Missing retained terminal tab")?;
        std::fs::write(&baseline,serde_json::to_vec_pretty(&json!({"pid":std::process::id(),"connected":connected,"target":target,"tabId":tab_id,"arguments":args,"operationId":operation,"native":native})).unwrap()).map_err(|e|e.to_string())?;
        return Ok(());
    }
    require(phase == "second", "Invalid restart phase")?;
    let before: Value =
        serde_json::from_slice(&std::fs::read(baseline).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    require(
        before["pid"] != std::process::id()
            && data(&before["connected"])["instanceId"] != data(connected)["instanceId"]
            && data(&before["connected"])["retryEpoch"] != data(connected)["retryEpoch"],
        "Restart retained process or broker authority",
    )?;
    let target = &before["target"];
    // Restored terminals are human-owned and may be lazy; use the ordinary tab
    // action, not a broker focus request that correctly forbids lazy starts.
    javascript(&main,&format!("const e=[...document.querySelectorAll('[data-tab-id]')].find(e=>e.dataset.tabId==={});if(!e)throw Error('Missing restored tab');e.click();return true;",before["tabId"])).await?;
    let mut native = Value::Null;
    for _ in 0..100 {
        native=javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});return {{sessionId:r?.sessionId,promptReady:Boolean(r?.atPrompt&&!r?.activeBlock),text:r?[...Array(r.terminal.buffer.active.length)].map((_,i)=>r.terminal.buffer.active.getLine(i)?.translateToString()).join('\\n'):null}};",target["panelId"])).await?;
        if native["promptReady"] == true {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    require(
        native["sessionId"].is_string()
            && native["sessionId"] != target["terminalSessionId"]
            && native["promptReady"] == true,
        "Restored terminal did not get a fresh ready runtime",
    )?;
    require(
        !native["text"]
            .as_str()
            .unwrap_or("")
            .contains("RESTART_ONCE_WRITTEN"),
        "Historical command output was replayed",
    )?;
    let receipt = wire
        .tool(
            "lomi_operation_get",
            json!({"operationId":before["operationId"]}),
        )
        .await?;
    require(
        receipt["structuredContent"]["code"] == "TARGET_NOT_FOUND",
        "Fresh helper acquired an old receipt",
    )?;
    let replay = wire
        .tool("lomi_terminal_run", before["arguments"].clone())
        .await?;
    require(
        replay["structuredContent"]["status"] == "error" && data(&replay)["operationId"].is_null(),
        format!("Old command authority was accepted: {replay}"),
    )?;
    require(
        std::fs::read(directory.join("project/restart-once.txt")).map_err(|e| e.to_string())?
            == b"once\n",
        "App restart or stale retry repeated a command",
    )?;
    let list = wire
        .tool("lomi_panel_list", json!({"workspaceId":workspace}))
        .await?;
    require(
        data(&list)["items"].as_array().is_some_and(|a| {
            a.iter()
                .any(|p| p["id"] == target["panelId"] && p["ownership"] == "human_or_unassigned")
        }),
        "Restart restored a terminal lease",
    )?;
    std::fs::write(directory.join("restart-proof.json"),serde_json::to_vec_pretty(&json!({"passed":true,"first":before,"second":{"pid":std::process::id(),"connected":connected,"native":native},"oldReceipt":receipt,"oldRetry":replay,"effectCount":1,"freshApprovalRequired":true,"leaseRestored":false})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
