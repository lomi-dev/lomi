//! Two real helper sessions, independent workspaces and in-flight disconnect.
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
async fn wait_output(wire: &mut Wire, target: &Value, marker: &str) -> Result<Value, String> {
    for _ in 0..100 {
        let read=wire.tool("lomi_terminal_read",json!({"workspaceId":target["workspaceId"],"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"maxBytes":8192})).await?;
        if data(&read)["text"]
            .as_str()
            .is_some_and(|s| s.lines().any(|l| l.trim() == marker))
        {
            return Ok(read);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("Missing exact native output line: {marker}"))
}
async fn ready(wire: &mut Wire, target: &Value) -> Result<(), String> {
    for _ in 0..100 {
        let read=wire.tool("lomi_terminal_read",json!({"workspaceId":target["workspaceId"],"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"]})).await?;
        if data(&read)["prompt"] == "ready" {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err("Native terminal did not become ready".into())
}
async fn run_command(
    wire: &mut Wire,
    target: &Value,
    epoch: &Value,
    command: &str,
    key: &str,
) -> Result<Value, String> {
    wire.tool("lomi_terminal_run",json!({"workspaceId":target["workspaceId"],"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"retryEpoch":epoch,"requestKey":key,"command":command})).await
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    first: &mut Wire,
    settings: &Webview,
    first_child: &mut tokio::process::Child,
    helper: &Value,
    directory: &Path,
    context: &Value,
) -> Result<(), String> {
    let first_pid = first_child.id().ok_or("Missing first helper")?;
    let mut child =
        tokio::process::Command::new(helper["command"].as_str().ok_or("Missing helper")?)
            .args(
                serde_json::from_value::<Vec<String>>(helper["args"].clone())
                    .map_err(|e| e.to_string())?,
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(
                std::fs::File::create(directory.join("second-helper.log"))
                    .map_err(|e| e.to_string())?,
            )
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;
    let second_pid = child.id().ok_or("Missing second helper")?;
    let mut second = Wire {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
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
        .ok_or("Missing distinct pairing request")?;
    require(
        second.tool("lomi_workspace_list", json!({})).await?["structuredContent"]["code"]
            == "PAIRING_REQUIRED",
        "Matching clientInfo silently shared authorization",
    )?;
    routing_probe::approve(
        settings,
        request,
        &json!("foreign-workspace"),
        &Value::Null,
        "terminal-interrupt",
        directory,
    )
    .await?;
    activate_main(app).await?;
    let connected = second
        .tool("lomi_connect", json!({"workspaceId":"foreign-workspace"}))
        .await?;
    let second_epoch = &data(&connected)["retryEpoch"];
    require(second_epoch.is_string(), "Second helper did not connect")?;
    let a_list = first.tool("lomi_workspace_list", json!({})).await?;
    let b_list = second.tool("lomi_workspace_list", json!({})).await?;
    require(
        data(&a_list)["items"]
            .as_array()
            .is_some_and(|a| a.len() == 1 && a[0]["id"] == context["workspaceId"]),
        "First workspace scope expanded",
    )?;
    require(
        data(&b_list)["items"]
            .as_array()
            .is_some_and(|a| a.len() == 1 && a[0]["id"] == "foreign-workspace"),
        "Second workspace scope leaked",
    )?;
    let (_,a)=layout_call(first,"lomi_terminal_create",json!({"workspaceId":context["workspaceId"],"cwdRelative":".","title":"Client A","retryEpoch":context["retryEpoch"],"requestKey":"isolation-terminal-a"})).await?;
    let a = data(&a)["result"].clone();
    let (_,b)=layout_call(&mut second,"lomi_terminal_create",json!({"workspaceId":"foreign-workspace","cwdRelative":".","title":"Client B","retryEpoch":second_epoch,"requestKey":"isolation-terminal-b"})).await?;
    let b = data(&b)["result"].clone();
    ready(first, &a).await?;
    ready(&mut second, &b).await?;
    require(
        a["panelId"] != b["panelId"] && a["terminalSessionId"] != b["terminalSessionId"],
        "Clients shared a native PTY",
    )?;
    // Focus B while A runs: execution must follow explicit identity.
    layout_call(&mut second,"lomi_panel_focus",json!({"workspaceId":"foreign-workspace","panelId":b["panelId"],"terminalSessionId":b["terminalSessionId"],"retryEpoch":second_epoch,"requestKey":"focus-b"})).await?;
    let a_run = run_command(
        first,
        &a,
        &context["retryEpoch"],
        "printf 'once\\n' >> client-once.txt; printf 'CLIENT_A_RUNNING\\n'; sleep 120",
        "client-a-long",
    )
    .await?;
    let operation = data(&a_run)["operationId"]
        .as_str()
        .ok_or("First command was rejected")?;
    wait_output(first, &a, "CLIENT_A_RUNNING").await?;
    let b_run = run_command(
        &mut second,
        &b,
        second_epoch,
        "printf 'CLIENT_B_READY\\n'",
        "client-b-first",
    )
    .await?;
    let b_done = second
        .settled(
            data(&b_run)["operationId"]
                .as_str()
                .ok_or("Second command rejected")?,
        )
        .await?;
    require(
        data(&b_done)["result"]["observation"]["exitCode"] == 0,
        "Second command failed",
    )?;
    wait_output(&mut second, &b, "CLIENT_B_READY").await?;
    let mut denied = Vec::new();
    for (wire, target, workspace) in [
        (&mut *first, &b, &context["workspaceId"]),
        (&mut second, &a, &json!("foreign-workspace")),
    ] {
        let result=wire.tool("lomi_terminal_read",json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"]})).await?;
        require(
            result["structuredContent"]["code"] == "TARGET_NOT_FOUND",
            format!("Foreign terminal disclosed: {result}"),
        )?;
        denied.push(result);
    }
    let main = app.get_webview("main").ok_or("Missing main")?;
    let native_before=javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');return {}.map(t=>{{const r=m.runningTerminal(t.panelId);return {{panelId:t.panelId,sessionId:r?.sessionId,status:r?.getSnapshot().status,text:[...Array(r.terminal.buffer.active.length)].map((_,i)=>r.terminal.buffer.active.getLine(i)?.translateToString()).join('\\n')}};}});",json!([a,b]))).await?;
    let screen = native_before.as_array().ok_or("Missing native screens")?;
    for (index, expected, foreign, target) in [
        (0, "CLIENT_A_RUNNING", "CLIENT_B_READY", &a),
        (1, "CLIENT_B_READY", "CLIENT_A_RUNNING", &b),
    ] {
        require(
            screen[index]["sessionId"] == target["terminalSessionId"]
                && screen[index]["text"].as_str().is_some_and(|s| {
                    s.lines().any(|l| l.trim() == expected)
                        && !s.lines().any(|l| l.trim() == foreign)
                }),
            "Actual xterm output crossed client workspaces",
        )?;
    }
    let running = first
        .tool("lomi_operation_get", json!({"operationId":operation}))
        .await?;
    require(
        data(&running)["state"] == "running",
        "Disconnect was not inside an active command",
    )?;
    first_child.kill().await.map_err(|e| e.to_string())?;
    let database = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("agent-control/control.sqlite3");
    let mut durable = None;
    for _ in 0..100 {
        let db = rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;
        let row: (String, String) = db
            .query_row(
                "SELECT state,effect FROM receipts WHERE id=?1",
                [operation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if row.0 == "outcome_unknown" {
            durable = Some(row);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    require(
        durable.as_ref().is_some_and(|r| r.1 == "unknown"),
        "Disconnect did not durably disclose unknown command effects",
    )?;
    ready(&mut second, &b).await?;
    let after = run_command(
        &mut second,
        &b,
        second_epoch,
        "printf 'CLIENT_B_AFTER_DISCONNECT\\n'",
        "client-b-after",
    )
    .await?;
    let after = second
        .settled(
            data(&after)["operationId"]
                .as_str()
                .ok_or("B lost authorization after A disconnected")?,
        )
        .await?;
    require(
        data(&after)["result"]["observation"]["exitCode"] == 0,
        "A disconnect affected B command",
    )?;
    wait_output(&mut second, &b, "CLIENT_B_AFTER_DISCONNECT").await?;
    require(
        std::fs::read(directory.join("project/client-once.txt")).map_err(|e| e.to_string())?
            == b"once\n",
        "Disconnected command effect repeated",
    )?;
    let retained=javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});return {{sessionId:r?.sessionId,running:Boolean(r&&!r.atPrompt),status:r?.getSnapshot().status}};",a["panelId"])).await?;
    require(
        retained["sessionId"] == a["terminalSessionId"] && retained["running"] == true,
        "Disconnect killed or replaced the user's retained command",
    )?;
    // Explicit fixture cleanup through ordinary human input, after observations.
    javascript(&main,&format!("await window.__TAURI_INTERNALS__.invoke('write_terminal',{{id:{},data:'\\u0003'}});return true;",a["terminalSessionId"])).await?;
    for _ in 0..100 {
        if javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});return Boolean(r?.atPrompt&&!r?.activeBlock);",a["panelId"])).await?==true {break;}
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    require(javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});return Boolean(r?.atPrompt&&!r?.activeBlock);",a["panelId"])).await?==true,"Fixture command did not stop")?;
    child.kill().await.map_err(|e| e.to_string())?;
    std::fs::write(directory.join("client-isolation.json"),serde_json::to_vec_pretty(&json!({"passed":true,"helpers":[first_pid,second_pid],"workspaces":[context["workspaceId"],"foreign-workspace"],"terminals":[a,b],"native":native_before,"foreignReadsDenied":denied,"disconnectedOperation":operation,"durableState":durable,"retainedAfterDisconnect":retained,"effectCount":1,"secondClientContinued":true,"fixtureCommandsStopped":true})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
