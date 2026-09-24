//! Manual client restart requires a fresh approval and never replays a command.
use super::*;

fn calls(report: &Value) -> impl Iterator<Item = &Value> {
    report["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["type"] == "mcpToolCall" && i["server"] == "lomi_probe")
}
fn operation(report: &Value) -> Result<String, String> {
    calls(report)
        .find(|i| i["tool"] == "lomi_terminal_run")
        .and_then(|i| i["result"]["structuredContent"]["data"]["operationId"].as_str())
        .map(String::from)
        .ok_or("Missing original operation identity".into())
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    settings: &Webview,
    workspace: &Value,
    helper: &Value,
    directory: &Path,
    fixture: &Value,
) -> Result<Value, String> {
    let cases = read_json(&directory.join("project/routing-cases.json"))?;
    let mut first_fixture = fixture.clone();
    first_fixture["routingCase"] = json!("reconnect-write");
    first_fixture["routingPhase"] = json!("reconnect-first");
    first_fixture["routingTask"] = cases["reconnect"].clone();
    let first = qualify_once(
        app,
        wire,
        settings,
        workspace,
        helper,
        directory,
        &first_fixture,
    )
    .await?;
    let mut ended_processes = Vec::new();
    for field in ["pid", "appServerPid"] {
        let pid = first["clientProcess"][field]
            .as_u64()
            .ok_or("Missing previous process identity")?;
        let output = tokio::process::Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "pid="])
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if output.status.success() || !output.stdout.is_empty() {
            return Err("Previous model client remains alive at reconnect".into());
        }
        ended_processes.push(pid);
    }
    let old_operation = operation(&first)?;
    let mut next_fixture = fixture.clone();
    next_fixture["routingCase"] = json!("reconnect-read");
    next_fixture["routingTask"] = json!({"prompt":format!("The prior client session ended after a single command wrote routing-once.txt. This is a fresh helper session and new native approval. Try to read its old operation ID {old_operation} once through Lomi; if access is refused, report that limitation. Then read routing-once.txt through Lomi and report its exact content without running or repeating any command. Do not request broader permissions."),"expectedTools":["lomi_files_read"]});
    let mut second = qualify_once(
        app,
        wire,
        settings,
        workspace,
        helper,
        directory,
        &next_fixture,
    )
    .await?;
    let pairing = |r: &Value| {
        r["bootstrapStatus"]["structuredContent"]["data"]["pairingRequestId"]
            .as_str()
            .map(String::from)
    };
    if pairing(&first).is_none()
        || pairing(&first) == pairing(&second)
        || first["clientProcess"]["appServerPid"] == second["clientProcess"]["appServerPid"]
    {
        return Err("Reconnect did not create distinct native approvals/client processes".into());
    }
    let mut items = first["items"]
        .as_array()
        .ok_or("Missing first session trace")?
        .clone();
    items.extend(
        second["items"]
            .as_array()
            .ok_or("Missing next session trace")?
            .iter()
            .cloned(),
    );
    let mut competing = first["competingActions"]
        .as_array()
        .ok_or("Missing first competing actions")?
        .clone();
    competing.extend(
        second["competingActions"]
            .as_array()
            .ok_or("Missing next competing actions")?
            .iter()
            .cloned(),
    );
    second["items"] = json!(items);
    second["competingActions"] = json!(competing);
    second["modelTurns"] = json!(2);
    second["previousSession"] = json!({"clientProcess":first["clientProcess"],"pairingRequestId":pairing(&first),"operationId":old_operation,"nativePostconditionsVerified":first["nativePostconditionsVerified"]});
    second["nativeEvidence"] = json!({"passed":true,"case":"reconnect","first":first["nativeEvidence"],"second":second["nativeEvidence"],"freshApproval":true,"previousProcessesAbsent":ended_processes,"commandReplay":false});
    write_json(
        &directory.join("routing-native.json"),
        &second["nativeEvidence"],
    )?;
    write_json(&directory.join("routing-result.json"), &second)?;
    Ok(second)
}

pub(super) async fn verify(
    app: &tauri::AppHandle,
    directory: &Path,
    case: &str,
    report: &Value,
) -> Result<Value, String> {
    if std::fs::read(directory.join("project/routing-once.txt")).map_err(|e| e.to_string())?
        != b"once\n"
        || report["competingActions"]
            .as_array()
            .is_none_or(|a| !a.is_empty())
    {
        return Err("Reconnect changed the effect count or used a competing tool".into());
    }
    if case == "reconnect-write" {
        if calls(report)
            .filter(|i| i["tool"] == "lomi_terminal_run")
            .count()
            != 1
        {
            return Err("First client repeated its command".into());
        }
        let terminal = calls(report)
            .map(|i| &i["result"]["structuredContent"]["data"]["result"])
            .find(|r| r["terminalSessionId"].is_string())
            .ok_or("Missing original terminal")?;
        let main = app.get_webview("main").ok_or("Missing main")?;
        let native=javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});return {{sessionId:r?.sessionId,promptReady:Boolean(r?.atPrompt&&!r?.activeBlock),text:[...Array(r.terminal.buffer.active.length)].map((_,i)=>r.terminal.buffer.active.getLine(i)?.translateToString()).join('\\n')}};",terminal["panelId"])).await?;
        if native["sessionId"] != terminal["terminalSessionId"]
            || native["promptReady"] != true
            || !native["text"]
                .as_str()
                .unwrap_or("")
                .contains("ROUTING_ONCE_WRITTEN")
        {
            return Err("Original command not observed in retained native PTY".into());
        }
        return Ok(
            json!({"passed":true,"case":case,"terminal":terminal,"native":native,"effectCount":1}),
        );
    }
    let first = read_json(&directory.join("reconnect-first-result.json"))?;
    let old_operation = operation(&first)?;
    let denied = calls(report).any(|i| {
        i["tool"] == "lomi_operation_get"
            && i["arguments"]["operationId"] == old_operation
            && i["result"]["structuredContent"]["code"] == "TARGET_NOT_FOUND"
    });
    let read = calls(report).any(|i| {
        i["tool"] == "lomi_files_read"
            && i["arguments"]["relativePath"] == "routing-once.txt"
            && i["result"]["structuredContent"]["status"] == "ok"
            && i["result"]["structuredContent"]["data"]
                .to_string()
                .contains("once")
    });
    if !denied || !read || calls(report).any(|i| i["tool"] == "lomi_terminal_run") {
        return Err(
            "New client did not respect the old receipt boundary and read the existing effect"
                .into(),
        );
    }
    Ok(
        json!({"passed":true,"case":case,"oldReceiptDenied":true,"effectCount":1,"commandReplay":false}),
    )
}
