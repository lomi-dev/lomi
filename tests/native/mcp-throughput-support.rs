//! Same-build PTY delivery/parser comparison with and without MCP observation.
use super::*;

async fn parsed_prompt(wire: &mut Wire, target: &Value) -> Result<(), String> {
    for _ in 0..200 {
        let read = wire.tool("lomi_terminal_read", target.clone()).await?;
        let data = &read["structuredContent"]["data"];
        if data["prompt"] == "ready" && data["streamSequence"] == data["parsedSequence"] {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err("Throughput terminal did not reach a parsed prompt".into())
}

pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    workspace: &Value,
    epoch: &Value,
    directory: &Path,
) -> Result<(), String> {
    let main = app.get_webview("main").ok_or("Missing main")?;
    let ordinary = routing_probe::prepare_origin(app, wire, workspace).await?;
    let (_, operation) = layout_call(wire,"lomi_terminal_create",json!({"workspaceId":workspace,"cwdRelative":".","title":"Observed throughput","retryEpoch":epoch,"requestKey":"throughput-terminal"})).await?;
    let observed = operation["structuredContent"]["data"]["result"].clone();
    let read = json!({"workspaceId":workspace,"panelId":observed["panelId"],"terminalSessionId":observed["terminalSessionId"]});
    parsed_prompt(wire, &read).await?;
    layout_call(wire,"lomi_panel_focus",json!({"workspaceId":workspace,"panelId":"mcp-control-fixture","retryEpoch":epoch,"requestKey":"throughput-hide"})).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut geometry = Vec::new();
    for target in [&ordinary, &observed] {
        geometry.push(javascript(&main,&format!(r#"
const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});
if(!r||r.sessionId!=={})throw Error('Wrong throughput PTY');
if(r.terminal.element?.getClientRects().length)throw Error('Throughput terminal must be hidden');
r.terminal.resize(80,24);
await window.__TAURI_INTERNALS__.invoke('resize_terminal',{{id:r.sessionId,cols:80,rows:24}});
window.__mcpThroughput??={{handlers:[],samples:{{}}}};
let started=null;
const handler=r.terminal.parser.registerOscHandler(7777,text=>{{
  if(text.startsWith('start:'))started={{id:text.slice(6),time:performance.now()}};
  else if(text.startsWith('end:')&&started?.id===text.slice(4)){{
    window.__mcpThroughput.samples[started.id]=performance.now()-started.time;started=null;
  }}
  return true;
}});
window.__mcpThroughput.handlers.push(handler);
return {{panelId:{},sessionId:r.sessionId,rows:r.terminal.rows,columns:r.terminal.cols,renderer:r.getSnapshot().renderer,agentControlled:r.getSnapshot().agentControlled}};
"#,target["panelId"],target["terminalSessionId"],target["panelId"])).await?);
    }
    if geometry[0]["agentControlled"] != false || geometry[1]["agentControlled"] != true {
        return Err("Throughput comparison did not isolate observer ownership".into());
    }
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(directory.join("terminal-fixture.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let node = quote(fixture["node"].as_str().ok_or("Missing Node")?);
    let mut samples = Vec::new();
    for pair in 0..23 {
        let order = if pair % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        };
        for controlled in order {
            let id = format!(
                "{pair}-{}",
                if controlled { "observed" } else { "ordinary" }
            );
            let script = format!("process.stdout.write('\\x1b]7777;start:{id}\\x07');process.stdout.write(Buffer.alloc(2*1024*1024,120));process.stdout.write('\\x1b]7777;end:{id}\\x07\\n')");
            let command = format!("{node} -e {}", quote(&script));
            let operation = if controlled {
                let admitted=wire.tool("lomi_terminal_run",json!({"workspaceId":workspace,"panelId":observed["panelId"],"terminalSessionId":observed["terminalSessionId"],"leaseId":observed["leaseId"],"retryEpoch":epoch,"requestKey":id,"command":command})).await?;
                Some(
                    admitted["structuredContent"]["data"]["operationId"]
                        .as_str()
                        .ok_or_else(|| format!("Throughput admission failed: {admitted}"))?
                        .to_owned(),
                )
            } else {
                javascript(&main,&format!("await window.__TAURI_INTERNALS__.invoke('write_terminal',{{id:{},data:{}}});return true;",ordinary["terminalSessionId"],json!(format!("{command}\r")))).await?;
                None
            };
            let mut elapsed = None;
            for _ in 0..300 {
                let sample = javascript(
                    &main,
                    &format!(
                        "return window.__mcpThroughput.samples[{}]??null;",
                        json!(id)
                    ),
                )
                .await?;
                if let Some(ms) = sample.as_f64() {
                    elapsed = Some(ms);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let elapsed = elapsed.ok_or("Output did not reach the real xterm parser")?;
            if let Some(id) = operation {
                let completed = wire.settled(&id).await?;
                if completed["structuredContent"]["data"]["result"]["observation"]["exitCode"] != 0
                {
                    return Err(format!("Throughput command did not exit 0: {completed}"));
                }
                parsed_prompt(wire, &read).await?;
            } else {
                javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');window.__mcpThroughput.ordinary=m.runningTerminal({});return true;",ordinary["panelId"])).await?;
                wait_for(&main,&format!("(()=>{{const r=window.__mcpThroughput.ordinary;const b=r.getSnapshot().blocks.at(-1);return Boolean(r.promptEnd&&b?.finished&&b.exitCode===0&&b.command.includes({}));}})()",json!(id))).await?;
            }
            samples.push(
                json!({"pair":pair,"warmup":pair<3,"observed":controlled,"parserMs":elapsed}),
            );
        }
    }
    javascript(&main,"for(const h of window.__mcpThroughput.handlers)h.dispose();delete window.__mcpThroughput;return true;").await?;
    let summary = |controlled: bool| {
        let mut values: Vec<_> = samples
            .iter()
            .filter(|s| s["warmup"] == false && s["observed"] == controlled)
            .map(|s| s["parserMs"].as_f64().unwrap())
            .collect();
        values.sort_by(f64::total_cmp);
        json!({"count":values.len(),"medianMs":(values[9]+values[10])/2.0,"p95Ms":values[18],"maxMs":values[19]})
    };
    let ordinary_summary = summary(false);
    let observed_summary = summary(true);
    let ratio = observed_summary["medianMs"].as_f64().unwrap()
        / ordinary_summary["medianMs"].as_f64().unwrap();
    let proof = json!({"payloadBytes":2*1024*1024,"pairedWarmups":3,"measuredPairs":20,"geometry":geometry,"samples":samples,"ordinary":ordinary_summary,"observed":observed_summary,"medianRatio":ratio,"maximumMedianRatio":1.10,"endpoint":"OSC start/end callbacks in the actual retained xterm parser; identical current-build ordinary and MCP-observed PTYs; excludes command admission and Node startup"});
    std::fs::write(
        directory.join("terminal-throughput.json"),
        serde_json::to_vec_pretty(&proof).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    if ratio > 1.10 {
        return Err(format!(
            "MCP observer median throughput regression exceeds 10%: {proof}"
        ));
    }
    Ok(())
}
