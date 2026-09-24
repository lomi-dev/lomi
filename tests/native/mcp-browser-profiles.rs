//! Product-created browser profiles across two projects and an ordinary view.
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
async fn state(view: &Webview, seed: Option<&str>) -> Result<Value, String> {
    let seed = if let Some(value) = seed {
        format!("document.cookie='lomi_s07='+{}+'; SameSite=Strict';localStorage.setItem('lomi_s07',{});await caches.open('lomi-s07-'+{});await navigator.serviceWorker.register('/worker.js');await navigator.serviceWorker.ready;",json!(value),json!(value),json!(value))
    } else {
        String::new()
    };
    evaluate(view,&format!("(()=>{{window.__profileProbe=null;void(async()=>{{{seed}return {{cookie:document.cookie.split('; ').find(s=>s.startsWith('lomi_s07='))??null,storage:localStorage.getItem('lomi_s07'),caches:(await caches.keys()).filter(k=>k.startsWith('lomi-s07-')),workers:(await navigator.serviceWorker.getRegistrations()).filter(r=>(r.active??r.waiting??r.installing)?.scriptURL.endsWith('/worker.js')).length}};}})().then(value=>window.__profileProbe={{ok:true,value}},error=>window.__profileProbe={{ok:false,error:String(error)}});return true;}})()")).await?;
    wait_for(view, "Boolean(window.__profileProbe)").await?;
    let observed = evaluate(view, "window.__profileProbe").await?;
    require(
        observed["ok"] == true,
        format!("Native profile observation failed: {observed}"),
    )?;
    Ok(observed["value"].clone())
}
fn empty(s: &Value) -> bool {
    s["cookie"].is_null() && s["storage"].is_null() && s["caches"] == json!([]) && s["workers"] == 0
}
fn marked(s: &Value, name: &str) -> bool {
    s["cookie"] == format!("lomi_s07={name}")
        && s["storage"] == name
        && s["caches"] == json!([format!("lomi-s07-{name}")])
        && s["workers"] == 1
}
async fn open(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    workspace: &Value,
    epoch: &Value,
    origin: &Value,
    key: &str,
) -> Result<(Value, Webview), String> {
    let (_, opened) = layout_call(
        wire,
        "lomi_browser_open",
        json!({"workspaceId":workspace,"url":origin,"retryEpoch":epoch,"requestKey":key}),
    )
    .await?;
    let target = data(&opened)["result"].clone();
    let view = app
        .get_webview(&format!(
            "browser-{}",
            target["panelId"].as_str().ok_or("Missing browser panel")?
        ))
        .ok_or("Missing product browser")?;
    wait_for(
        &view,
        "document.title==='Disconnect fixture'&&document.readyState==='complete'",
    )
    .await?;
    Ok((target, view))
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    first: &mut Wire,
    settings: &Webview,
    helper: &Value,
    directory: &Path,
    context: &Value,
) -> Result<(), String> {
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(directory.join("browser-profiles-fixture.json"))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    activate_main(app).await?;
    let main = app.get_webview("main").ok_or("Missing main")?;
    javascript(&main,"const e=document.querySelector('[data-tab-id=\"mcp-profile-human\"]');if(!e)throw Error('Missing ordinary browser tab');e.querySelector('[role=tab]').click();return true;").await?;
    for _ in 0..100 {
        if app.get_webview("browser-mcp-profile-human").is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let human = app
        .get_webview("browser-mcp-profile-human")
        .ok_or("Missing ordinary browser runtime")?;
    wait_for(
        &human,
        "document.title==='Disconnect fixture'&&document.readyState==='complete'",
    )
    .await?;
    let human_before = state(&human, None).await?;
    require(
        empty(&human_before),
        "Ordinary fixture profile already contains test state",
    )?;
    let human_seeded = state(&human, Some("human")).await?;
    require(
        marked(&human_seeded, "human"),
        "Ordinary fixture seed was not retained",
    )?;
    let (a, a_view) = open(
        app,
        first,
        &context["workspaceId"],
        &context["retryEpoch"],
        &context["origin"],
        "profiles-a",
    )
    .await?;
    let a_before = state(&a_view, None).await?;
    require(
        empty(&a_before),
        "First MCP profile shared the ordinary browser state",
    )?;
    let a_seeded = state(&a_view, Some("a")).await?;
    require(
        marked(&a_seeded, "a"),
        "First MCP browser seed was not retained",
    )?;
    let mut child =
        tokio::process::Command::new(helper["command"].as_str().ok_or("Missing helper path")?)
            .args(
                serde_json::from_value::<Vec<String>>(helper["args"].clone())
                    .map_err(|e| e.to_string())?,
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(
                std::fs::File::create(directory.join("profiles-helper.log"))
                    .map_err(|e| e.to_string())?,
            )
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;
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
        .ok_or("Missing second pairing")?;
    routing_probe::approve(
        settings,
        request,
        &fixture["workspaceId"],
        &context["origin"],
        "form",
        directory,
    )
    .await?;
    let connected = second
        .tool(
            "lomi_connect",
            json!({"workspaceId":fixture["workspaceId"]}),
        )
        .await?;
    require(
        connected["structuredContent"]["status"] == "ok"
            && data(&connected)["projectId"] == fixture["projectId"],
        "Second client did not select the other project",
    )?;
    activate_main(app).await?;
    let (b, b_view) = open(
        app,
        &mut second,
        &fixture["workspaceId"],
        &data(&connected)["retryEpoch"],
        &context["origin"],
        "profiles-b",
    )
    .await?;
    let b_before = state(&b_view, None).await?;
    require(
        empty(&b_before),
        "Second project's profile inherited another identity's storage",
    )?;
    let b_seeded = state(&b_view, Some("b")).await?;
    require(
        marked(&b_seeded, "b"),
        "Second MCP browser seed was not retained",
    )?;
    let final_human = state(&human, None).await?;
    let final_a = state(&a_view, None).await?;
    let final_b = state(&b_view, None).await?;
    require(
        marked(&final_human, "human") && marked(&final_a, "a") && marked(&final_b, "b"),
        "Storage crossed an ordinary or MCP profile",
    )?;
    let denied=second.tool("lomi_browser_snapshot",json!({"workspaceId":context["workspaceId"],"panelId":a["panelId"],"browserGeneration":a["browserGeneration"]})).await?;
    require(
        denied["structuredContent"]["code"] == "TARGET_NOT_FOUND",
        "Foreign project browser was readable",
    )?;
    let panels = first
        .tool(
            "lomi_panel_list",
            json!({"workspaceId":context["workspaceId"]}),
        )
        .await?;
    let second_panels = second
        .tool(
            "lomi_panel_list",
            json!({"workspaceId":fixture["workspaceId"]}),
        )
        .await?;
    child.kill().await.map_err(|e| e.to_string())?;
    std::fs::write(directory.join("browser-profiles.json"),serde_json::to_vec_pretty(&json!({"passed":true,"distinctProjects":true,"origin":context["origin"],"first":a,"second":b,"before":[human_before,a_before,b_before],"after":[final_human,final_a,final_b],"foreignDenied":denied,"panels":[panels,second_panels],"checks":["ordinary native profile preserved","two Settings-approved identities in distinct projects","real product-created child webviews","same origin with independent cookies and localStorage","independent CacheStorage and service-worker registries","foreign project snapshot denied"]})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
