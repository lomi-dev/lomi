//! Both approved shells through stdio MCP, the native PTY and retained xterm.
use super::*;
use base64::Engine;

fn data(value: &Value) -> &Value {
    &value["structuredContent"]["data"]
}
fn require(ok: bool, message: impl Into<String>) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
async fn observe(
    wire: &mut Wire,
    args: &Value,
    predicate: impl Fn(&Value) -> bool,
) -> Result<Value, String> {
    let mut last = Value::Null;
    for _ in 0..400 {
        last = wire.tool("lomi_terminal_read", args.clone()).await?;
        if predicate(data(&last)) {
            return Ok(last);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(format!("Terminal observation timed out: {last}"))
}
async fn ready(wire: &mut Wire, args: &Value) -> Result<Value, String> {
    observe(wire, args, |d| {
        d["prompt"] == "ready" && d["streamSequence"] == d["parsedSequence"]
    })
    .await
}
async fn resize_and_restore(wire: &mut Wire, main: &Webview, read: &Value) -> Result<(), String> {
    let mut screen = read.clone();
    screen["mode"] = json!("screen");
    let dimensions = wire.tool("lomi_terminal_read", screen).await?;
    let rows = data(&dimensions)["rows"]
        .as_u64()
        .ok_or("Missing terminal rows")?;
    let columns = data(&dimensions)["columns"]
        .as_u64()
        .ok_or("Missing terminal columns")?;
    for cols in [40, 100, columns] {
        javascript(main,&format!("await window.__TAURI_INTERNALS__.invoke('resize_terminal',{{id:{},cols:{cols},rows:{rows}}});return true;",read["terminalSessionId"])).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
async fn run(
    wire: &mut Wire,
    base: &Value,
    key: &str,
    command: &str,
) -> Result<(Value, String), String> {
    let mut args = base.clone();
    args["requestKey"] = json!(key);
    args["command"] = json!(command);
    let result = wire.tool("lomi_terminal_run", args.clone()).await?;
    let id = data(&result)["operationId"]
        .as_str()
        .ok_or_else(|| format!("Run {key}: {result}"))?
        .to_owned();
    Ok((args, id))
}
async fn completed(wire: &mut Wire, id: &str, exit: i64) -> Result<Value, String> {
    let result = wire.settled(id).await?;
    require(
        data(&result)["result"]["observation"]["exitCode"] == exit,
        format!("Wrong command result: {result}"),
    )?;
    require(
        data(&result)["result"]["observation"]["startedObserved"] == true,
        "Command start was not observed",
    )?;
    Ok(result)
}
async fn input(
    wire: &mut Wire,
    base: &Value,
    sequence: &mut u64,
    payload: Value,
) -> Result<(Value, Value), String> {
    *sequence += 1;
    let mut args = base.clone();
    args.as_object_mut().unwrap().remove("retryEpoch");
    args["inputSequence"] = json!(sequence.to_string());
    args["input"] = payload;
    let result = wire.tool("lomi_terminal_input", args.clone()).await?;
    require(
        data(&result)["dispatch"] == "dispatched",
        format!("Input failed: {result}"),
    )?;
    Ok((args, result))
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    epoch: &Value,
    directory: &Path,
) -> Result<(), String> {
    activate_main(app).await?;
    let shell = std::env::var("LOMI_MCP_TERMINAL_ONLY").map_err(|e| e.to_string())?;
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(directory.join("terminal-fixture.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let node = fixture["node"].as_str().ok_or("Missing fixture Node")?;
    let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
    let node = quote(node);
    let listed = wire.tool("lomi_workspace_list", json!({})).await?;
    let denied = wire.tool("lomi_terminal_create", json!({"workspaceId":workspace,"cwdRelative":".","profileId":if shell == "bash" {"local:zsh"} else {"local:bash"},"title":"Wrong shell","expectedRevision":data(&listed)["domainRevision"],"retryEpoch":epoch,"requestKey":"wrong-shell"})).await?;
    require(
        denied["structuredContent"]["code"] == "SCOPE_DENIED",
        format!("Other shell was permitted: {denied}"),
    )?;
    let (_, terminal) = layout_call(wire,"lomi_terminal_create",json!({"workspaceId":workspace,"cwdRelative":".","title":format!("MCP {shell} qualification"),"retryEpoch":epoch,"requestKey":"terminal-qualified"})).await?;
    let target = data(&terminal)["result"].clone();
    let read = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"maxBytes":65536});
    let base = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"retryEpoch":epoch});
    ready(wire, &read).await?;
    let mut checks = vec!["Settings-selected shell and denied alternative profile"];
    let mut evidence = Vec::new();
    let (args, id) = run(
        wire,
        &base,
        "unicode-once",
        "printf x >> terminal-once; printf 'MCP:%s\\n' 'Zażółć 🙂'",
    )
    .await?;
    evidence.push(completed(wire, &id, 0).await?);
    require(
        data(&wire.tool("lomi_terminal_run", args).await?)["operationId"] == id,
        "Run replay changed operation",
    )?;
    require(
        std::fs::read(directory.join("project/terminal-once")).map_err(|e| e.to_string())? == b"x",
        "Run executed twice",
    )?;
    let value = ready(wire, &read).await?;
    require(
        data(&value)["text"]
            .as_str()
            .unwrap_or("")
            .contains("MCP:Zażółć 🙂"),
        "Unicode output missing",
    )?;
    checks.push("Unicode, observed completion and once-only command retry");
    let (_, id) = run(wire, &base, "nonzero", "false").await?;
    evidence.push(completed(wire, &id, 1).await?);
    ready(wire, &read).await?;
    checks.push("Observed nonzero exit status");
    let (_, id) = run(wire, &base, "silent", "sleep 30").await?;
    observe(wire, &read, |d| d["prompt"] == "running").await?;
    tokio::time::sleep(Duration::from_millis(350)).await;
    require(
        data(
            &wire
                .tool("lomi_operation_get", json!({"operationId":id}))
                .await?,
        )["state"]
            == "running",
        "Silence falsely completed command",
    )?;
    let mut interrupt = base.clone();
    interrupt["operationId"] = json!(id);
    interrupt["requestKey"] = json!("stop-silent");
    let stop = wire
        .tool("lomi_terminal_interrupt", interrupt.clone())
        .await?;
    let stopped = wire
        .settled(
            data(&stop)["operationId"]
                .as_str()
                .ok_or_else(|| stop.to_string())?,
        )
        .await?;
    require(
        data(&stopped)["state"] == "succeeded",
        format!("Interrupt failed: {stopped}"),
    )?;
    evidence.push(completed(wire, &id, 130).await?);
    require(
        data(&wire.tool("lomi_terminal_interrupt", interrupt).await?)["operationId"]
            == data(&stop)["operationId"],
        "Interrupt retry changed operation",
    )?;
    ready(wire, &read).await?;
    checks.push("Silence remains running; targeted Ctrl+C and retry");
    let mut sequence = 0;
    let (packet, ack) = input(
        wire,
        &base,
        &mut sequence,
        json!({"type":"text","text":"printf p >> terminal-input-once"}),
    )
    .await?;
    require(
        wire.tool("lomi_terminal_input", packet.clone()).await? == ack,
        "Input retry changed receipt",
    )?;
    let mut changed = packet;
    changed["input"]["text"] = json!("wrong");
    require(
        wire.tool("lomi_terminal_input", changed).await?["structuredContent"]["code"]
            == "IDEMPOTENCY_CONFLICT",
        "Changed input sequence accepted",
    )?;
    let mut unsafe_run = base.clone();
    resize_and_restore(wire, main, &read).await?;
    unsafe_run["command"] = json!("echo WRONG");
    unsafe_run["requestKey"] = json!("partial-run");
    let partial = wire.tool("lomi_terminal_run", unsafe_run).await?;
    let partial = wire
        .settled(
            data(&partial)["operationId"]
                .as_str()
                .ok_or_else(|| partial.to_string())?,
        )
        .await?;
    require(
        data(&partial)["state"] == "failed"
            && data(&partial)["effectState"] == "none"
            && data(&partial)["result"]["code"] == "PROMPT_STATE_UNKNOWN",
        format!("Partial input response: {partial}"),
    )?;
    evidence.push(partial);
    input(
        wire,
        &base,
        &mut sequence,
        json!({"type":"key","key":"enter"}),
    )
    .await?;
    ready(wire, &read).await?;
    require(
        std::fs::read(directory.join("project/terminal-input-once")).map_err(|e| e.to_string())?
            == b"p",
        "Input executed twice",
    )?;
    checks.push("Partial input and resize block run; ordered input retry does not duplicate");
    let (_, id) = run(wire, &base, "node-repl", &format!("{node} --interactive")).await?;
    observe(wire, &read, |d| {
        d["text"]
            .as_str()
            .unwrap_or("")
            .contains("Welcome to Node.js")
    })
    .await?;
    input(
        wire,
        &base,
        &mut sequence,
        json!({"type":"text","text":"console.log('REPL:' + 'Zażółć 🙂')\r"}),
    )
    .await?;
    observe(wire, &read, |d| {
        d["text"].as_str().unwrap_or("").contains("REPL:Zażółć 🙂")
    })
    .await?;
    input(
        wire,
        &base,
        &mut sequence,
        json!({"type":"text","text":".exit\r"}),
    )
    .await?;
    evidence.push(completed(wire, &id, 0).await?);
    ready(wire, &read).await?;
    checks.push("Real Node REPL receives Unicode input and returns to shell");
    let (_, id) = run(
        wire,
        &base,
        "vim-tui",
        "/usr/bin/vi -u NONE -i NONE -n -- terminal-tui.txt",
    )
    .await?;
    let mut screen = read.clone();
    screen["mode"] = json!("screen");
    observe(wire, &screen, |d| {
        d["buffer"] == "alternate" && d["parserPending"] == false
    })
    .await?;
    input(
        wire,
        &base,
        &mut sequence,
        json!({"type":"text","text":"iZażółć 🙂"}),
    )
    .await?;
    input(
        wire,
        &base,
        &mut sequence,
        json!({"type":"key","key":"escape"}),
    )
    .await?;
    let visible = observe(wire, &screen, |d| {
        d["buffer"] == "alternate"
            && d["parserPending"] == false
            && d["text"].as_str().unwrap_or("").contains("Zażółć 🙂")
    })
    .await?;
    evidence.push(visible);
    // Parser ACK precedes xterm's scheduled visual frame.
    javascript(main,"await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));return true;").await?;
    screenshot(main, directory.join("terminal-tui.png")).await?;
    input(
        wire,
        &base,
        &mut sequence,
        json!({"type":"text","text":":wq\r"}),
    )
    .await?;
    evidence.push(completed(wire, &id, 0).await?);
    ready(wire, &read).await?;
    require(
        std::fs::read_to_string(directory.join("project/terminal-tui.txt"))
            .map_err(|e| e.to_string())?
            == "Zażółć 🙂\n",
        "TUI did not save exact input",
    )?;
    checks.push("Real Vim alternate screen, Unicode input and file assertion");
    let (_, id) = run(wire, &base, "binary", r"printf 'BINARY:\000\377\001:END\n'").await?;
    evidence.push(completed(wire, &id, 0).await?);
    ready(wire, &read).await?;
    let mut raw = read.clone();
    raw["mode"] = json!("raw");
    let value = wire.tool("lomi_terminal_read", raw).await?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(
            data(&value)["base64"]
                .as_str()
                .ok_or("Missing raw terminal bytes")?,
        )
        .map_err(|e| e.to_string())?;
    require(
        bytes.windows(14).any(|b| b == b"BINARY:\0\xff\x01:END"),
        "Raw read lost binary bytes",
    )?;
    checks.push("Raw terminal output preserves NUL and invalid UTF-8");
    let (_,id) = run(wire,&base,"flood",&format!("{node} -e {}",quote("process.stdout.write(Buffer.alloc(2*1024*1024,120));process.stdout.write('FLOOD_END\\n')"))).await?;
    // Deliberately stop MCP reads while the native channel and xterm drain.
    tokio::time::sleep(Duration::from_secs(1)).await;
    evidence.push(completed(wire, &id, 0).await?);
    let observed = ready(wire, &read).await?;
    let mut old = read.clone();
    old["cursor"] = json!("0");
    let gap = wire.tool("lomi_terminal_read", old).await?;
    require(
        data(&gap)["gap"] == true,
        "Slow reader did not report trimmed ring output",
    )?;
    evidence.push(observed);
    evidence.push(gap);
    checks.push("2 MiB flood drains xterm without MCP reads; bounded ring reports gap");
    let (_, id) = run(
        wire,
        &base,
        "background",
        "sleep 1 & printf 'BACKGROUND_DONE\\n'",
    )
    .await?;
    evidence.push(completed(wire, &id, 0).await?);
    ready(wire, &read).await?;
    checks.push("Background job does not hold the foreground command receipt");
    let (_,other)=layout_call(wire,"lomi_terminal_create",json!({"workspaceId":workspace,"cwdRelative":".","title":"EOF qualification","retryEpoch":epoch,"requestKey":"eof-terminal"})).await?;
    let other = data(&other)["result"].clone();
    let other_read = json!({"workspaceId":workspace,"panelId":other["panelId"],"terminalSessionId":other["terminalSessionId"]});
    ready(wire, &other_read).await?;
    let other_base = json!({"workspaceId":workspace,"panelId":other["panelId"],"terminalSessionId":other["terminalSessionId"],"leaseId":other["leaseId"],"retryEpoch":epoch});
    let (_, id) = run(wire, &other_base, "eof", "exit 7").await?;
    let eof = wire.settled(&id).await?;
    require(
        data(&eof)["state"] == "outcome_unknown",
        format!("EOF fabricated a shell-hook completion: {eof}"),
    )?;
    evidence.push(eof);
    checks.push("Shell EOF without completion marker remains outcome unknown");
    layout_call(wire,"lomi_panel_focus",json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"retryEpoch":epoch,"requestKey":"focus-takeover"})).await?;
    javascript(main,&format!("await window.__TAURI_INTERNALS__.invoke('write_terminal',{{id:{},data:'human-owned'}});return true;",target["terminalSessionId"])).await?;
    let mut refused = base.clone();
    refused["command"] = json!("echo WRONG");
    refused["requestKey"] = json!("after-human");
    let refused = wire.tool("lomi_terminal_run", refused).await?;
    require(
        data(&refused)["state"] == "failed"
            && data(&refused)["effectState"] == "none"
            && data(&refused)["result"]["code"] == "CONTROL_REVOKED",
        format!("Human input retained agent execution: {refused}"),
    )?;
    evidence.push(refused);
    require(
        data(&wire.tool("lomi_terminal_read", read.clone()).await?)["leaseId"].is_null(),
        "Human input retained lease",
    )?;
    checks.push("Native human input revokes execution and lease");
    let claim = wire.tool("lomi_panel_control",json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"action":"claim","retryEpoch":epoch,"requestKey":"reclaim-partial-terminal"})).await?;
    require(
        data(&claim)["state"] == "awaiting_user",
        format!("Claim skipped Settings approval: {claim}"),
    )?;
    let claim_id = data(&claim)["operationId"]
        .as_str()
        .ok_or("Missing claim ID")?;
    let settings = app
        .get_webview("settings")
        .ok_or("Missing terminal Settings")?;
    wait_for(
        &settings,
        "document.body.textContent.includes('Terminal input requests')",
    )
    .await?;
    click(&settings, "Allow input").await?;
    let mut granted = Value::Null;
    for _ in 0..400 {
        granted = wire.settled(claim_id).await?;
        if data(&granted)["state"] != "awaiting_user" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    require(
        data(&granted)["state"] == "succeeded",
        format!("Claim failed: {granted}"),
    )?;
    let lease = data(&granted)["result"]["leaseId"].clone();
    require(
        lease.is_string() && lease != target["leaseId"],
        "Claim reused revoked lease",
    )?;
    let mut claimed = base.clone();
    claimed["leaseId"] = lease;
    resize_and_restore(wire, main, &read).await?;
    let (_, blocked) = run(wire, &claimed, "claimed-partial", "echo WRONG").await?;
    let blocked = wire.settled(&blocked).await?;
    require(
        data(&blocked)["state"] == "failed"
            && data(&blocked)["effectState"] == "none"
            && data(&blocked)["result"]["code"] == "PROMPT_STATE_UNKNOWN",
        format!("Claim/resize authorized an existing partial line: {blocked}"),
    )?;
    let mut fresh_sequence = 0;
    input(
        wire,
        &claimed,
        &mut fresh_sequence,
        json!({"type":"key","key":"ctrl_c"}),
    )
    .await?;
    ready(wire, &read).await?;
    let (_, id) = run(
        wire,
        &claimed,
        "claimed-fresh",
        "printf 'FRESH_CLAIM_OK\\n'",
    )
    .await?;
    evidence.push(completed(wire, &id, 0).await?);
    ready(wire, &read).await?;
    evidence.push(granted);
    evidence.push(blocked);
    checks.push("Settings reclaim creates a new lease and preserves partial input through resize");
    screenshot(main, directory.join("terminal-final.png")).await?;
    std::fs::write(
        directory.join("terminals.json"),
        serde_json::to_vec_pretty(
            &json!({"shell":shell,"checks":checks,"evidence":evidence,"terminal":terminal}),
        )
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
