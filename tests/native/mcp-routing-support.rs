//! Model-selected MCP calls with independent native resource postconditions.
use super::*;

fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(value).unwrap())
        .map_err(|e| e.to_string())?;
    std::fs::rename(temporary, path).map_err(|e| e.to_string())
}
async fn approve(
    settings: &Webview,
    request: &str,
    workspace: &Value,
    origin: &Value,
    case: &str,
) -> Result<(), String> {
    let article = format!("[...document.querySelectorAll('.agent-control-request')].find(a=>[...a.querySelectorAll('code')].some(c=>c.textContent==={}))", json!(request));
    wait_for(settings, &format!("Boolean({article})")).await?;
    evaluate(settings, &format!("(()=>{{const e=({article}).querySelector('select');e.value={workspace};e.dispatchEvent(new Event('change',{{bubbles:true}}));return true;}})()")).await?;
    if case != "scope-denied" {
        let mut labels = vec![
            "Allow terminal creation, command execution and output reads",
            "Allow selecting and closing panels",
        ];
        if case == "fix-test" {
            labels.extend([
                "Allow reading project files",
                "Allow reading unsaved editor buffers",
                "Allow editing loaded buffers",
                "Allow saving editor files",
            ]);
        } else {
            labels.extend([
                "Allow opening and navigating isolated browser panels",
                "Allow reading page text, form structure and browser logs",
                "Allow clicking, typing and scrolling in pages",
                "Allow screenshots of pages",
            ]);
        }
        for label in labels {
            let input = format!("[...({article}).querySelectorAll('label')].find(e=>e.textContent.includes({})).querySelector('input')",json!(label));
            wait_for(settings, &format!("!({input}).disabled")).await?;
            evaluate(
                settings,
                &format!("(()=>{{const e={input};if(!e.checked)e.click();return true;}})()"),
            )
            .await?;
        }
        if case != "fix-test" {
            evaluate(settings,&format!("(()=>{{const e=({article}).querySelector('textarea');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,{origin});e.dispatchEvent(new Event('input',{{bubbles:true}}));return true;}})()")).await?;
        }
    }
    let button = format!("[...({article}).querySelectorAll('button')].find(b=>b.textContent.trim()==='Approve session')");
    wait_for(settings, &format!("!({button}).disabled")).await?;
    evaluate(settings, &format!("({button}).click();true")).await?;
    Ok(())
}

async fn verify(
    app: &tauri::AppHandle,
    directory: &Path,
    case: &str,
    report: &Value,
) -> Result<Value, String> {
    if report["modelTurnCompleted"] != true || report["expectedToolsObserved"] != true {
        return Err(format!(
            "Model routing did not complete: {}",
            report["error"]
        ));
    }
    let items = report["items"].as_array().ok_or("Missing model items")?;
    let calls: Vec<_> = items
        .iter()
        .filter(|i| i["type"] == "mcpToolCall" && i["server"] == "lomi_probe")
        .collect();
    if matches!(case, "unavailable" | "scope-denied") {
        if report["competingActions"]
            .as_array()
            .is_none_or(|a| !a.is_empty())
        {
            return Err("Model used a competing action after unavailability/denial".into());
        }
        let refused = calls.iter().any(|i| {
            let expected = if case == "unavailable" {
                "lomi_status"
            } else {
                "lomi_terminal_create"
            };
            if i["tool"] != expected {
                return false;
            }
            let result = &i["result"]["structuredContent"];
            if case == "unavailable" {
                result["data"]["connection"] == "app_unavailable"
            } else {
                result["code"] == "SCOPE_DENIED"
            }
        });
        if !refused {
            return Err("Missing actual unavailable/denied tool result".into());
        }
        return Ok(
            json!({"passed":true,"case":case,"noCompetingAction":true,"observedRefusal":true}),
        );
    }
    if report["expectedToolsSucceeded"] != true {
        return Err("A required model tool failed or omitted its screenshot image".into());
    }
    let main = app.get_webview("main").ok_or("Missing main")?;
    let resources: Vec<_> = calls
        .iter()
        .map(|i| &i["result"]["structuredContent"]["data"]["result"])
        .collect();
    let terminal = resources
        .iter()
        .find(|r| r["terminalSessionId"].is_string())
        .ok_or("No completed model-created terminal")?;
    let terminal_native = javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});if(!r||r.sessionId!=={})throw Error('Native terminal mismatch');const contexts=await window.__TAURI_INTERNALS__.invoke('terminal_contexts');return {{panelId:{},sessionId:r.sessionId,status:r.getSnapshot().status,context:contexts[r.sessionId],text:[...Array(r.terminal.buffer.active.length)].map((_,i)=>r.terminal.buffer.active.getLine(i)?.translateToString()).join('\\n')}};",terminal["panelId"],terminal["terminalSessionId"],terminal["panelId"])).await?;
    if case == "fix-test" {
        let expected = "import assert from 'node:assert/strict';\nimport { test } from 'node:test';\nimport { add } from './math.mjs';\ntest('addition', () => { assert.equal(add(2, 3), 5); assert.equal(add(-2, 3), 1); });\n";
        if std::fs::read_to_string(directory.join("project/math.test.mjs"))
            .map_err(|e| e.to_string())?
            != expected
        {
            return Err("Model changed the test instead of preserving its assertions".into());
        }
        let fixture = read_json(&directory.join("terminal-fixture.json"))?;
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::process::Command::new(fixture["node"].as_str().ok_or("Missing Node")?)
                .args(["--test", "math.test.mjs"])
                .current_dir(directory.join("project"))
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| "Independent fixed-test timeout")?
        .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("Independent fixed-test check failed".into());
        }
        let exits: Vec<_> = resources
            .iter()
            .filter_map(|r| r["observation"]["exitCode"].as_i64())
            .collect();
        if !exits.contains(&1) || !exits.contains(&0) {
            return Err("Model did not observe failure and successful rerun in Lomi".into());
        }
        return Ok(
            json!({"passed":true,"case":case,"terminal":terminal_native,"independentTestExit":0,"testFilePreserved":true}),
        );
    }
    let target = resources
        .iter()
        .find(|r| r["browserGeneration"].is_string())
        .ok_or("No completed model-created browser")?;
    let browser = app
        .get_webview(&format!(
            "browser-{}",
            target["panelId"].as_str().ok_or("Missing browser ID")?
        ))
        .ok_or("Model did not create a native browser")?;
    let page = evaluate(
        &browser,
        "({url:location.href,title:document.title,text:document.body.innerText})",
    )
    .await?;
    let fixture = read_json(&directory.join("browser-fixture.json"))?;
    if !page["url"]
        .as_str()
        .is_some_and(|url| url == format!("{}/", fixture["origin"].as_str().unwrap()))
        || page["title"] != "Routing fixture"
    {
        return Err(format!("Wrong native page: {page}"));
    }
    let state = read_json(&directory.join("project/routing-server-state.json"))?;
    if case == "form"
        && (state["submissions"] != 1
            || state["value"] != "Zażółć 🙂"
            || !page["text"]
                .as_str()
                .unwrap_or("")
                .contains("Saved: Zażółć 🙂"))
    {
        return Err("Native form submission postcondition failed".into());
    }
    if !terminal_native["text"]
        .as_str()
        .unwrap_or("")
        .contains("ROUTING_READY")
    {
        return Err("Server output was not rendered in the actual Lomi terminal".into());
    }
    screenshot(&browser, directory.join("routing-browser.png")).await?;
    Ok(
        json!({"passed":true,"case":case,"terminal":terminal_native,"browser":target,"page":page,"server":state}),
    )
}

pub(super) async fn qualify(
    app: &tauri::AppHandle,
    settings: &Webview,
    workspace: &Value,
    helper: &Value,
    directory: &Path,
    fixture: &Value,
) -> Result<Value, String> {
    let case = std::env::var("LOMI_MCP_ROUTING_ONLY").map_err(|e| e.to_string())?;
    let cases = read_json(&directory.join("project/routing-cases.json"))?;
    let task = cases.get(&case).ok_or("Unknown routing case")?;
    let control_path = directory.join("routing-control.json");
    let ready_path = directory.join("routing-approval-ready.json");
    let result_path = directory.join("routing-result.json");
    let postconditions_path = directory.join("routing-native.json");
    let mut helper = helper.clone();
    if case == "unavailable" {
        helper["args"] = json!([]);
    }
    write_json(
        &control_path,
        &json!({"cwd":directory.join("project"),"helper":helper,"expectedWorkspaceId":workspace,"approvalReadyPath":ready_path,"resultPath":result_path,"postconditionsPath":postconditions_path,"expectUnavailable":case=="unavailable","prompt":task["prompt"],"expectedTools":task["expectedTools"],"clientApprovedTools":task["clientApprovedTools"]}),
    )?;
    let log = std::fs::File::create(directory.join("routing.log")).map_err(|e| e.to_string())?;
    let mut child = tokio::process::Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/mcp/codex-routing.mjs"))
        .arg(control_path)
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = std::time::Instant::now();
    let mut approved = case == "unavailable";
    while start.elapsed() < Duration::from_secs(300) {
        if !approved && ready_path.exists() {
            let request = read_json(&ready_path)?;
            approve(
                settings,
                request["pairingRequestId"]
                    .as_str()
                    .ok_or("Missing request ID")?,
                workspace,
                &fixture["origin"],
                &case,
            )
            .await?;
            activate_main(app).await?;
            approved = true;
        }
        if result_path.exists() {
            let report = read_json(&result_path)?;
            let evidence = verify(app, directory, &case, &report).await;
            write_json(
                &postconditions_path,
                &match &evidence {
                    Ok(value) => value.clone(),
                    Err(error) => json!({"passed":false,"error":error}),
                },
            )?;
            evidence?;
            let exit = tokio::time::timeout(Duration::from_secs(10), child.wait())
                .await
                .map_err(|_| "Routing client did not exit")?
                .map_err(|e| e.to_string())?;
            if !exit.success() {
                return Err("Routing client failed after native checks".into());
            }
            return read_json(&result_path);
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("Routing client exited before result; inspect routing.log".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("Routing client timed out".into())
}
