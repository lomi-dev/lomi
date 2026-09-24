//! Real stdio/broker/WKContentWorld frame qualification, without public eval tools.
use super::*;

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn element(snapshot: &Value, name: &str, role: &str) -> Result<Value, String> {
    snapshot["elements"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|e| e["name"] == name && e["role"] == role)
        })
        .map(|e| e["elementRef"].clone())
        .ok_or_else(|| format!("Missing {role} {name}: {snapshot}"))
}
async fn snapshot(wire: &mut Wire, args: &Value) -> Result<Value, String> {
    let value = wire.tool("lomi_browser_snapshot", args.clone()).await?;
    require(
        value["structuredContent"]["status"] == "ok",
        &format!("Frame snapshot failed: {value}"),
    )?;
    Ok(value["structuredContent"]["data"].clone())
}
async fn interact(
    wire: &mut Wire,
    tool: &str,
    args: Value,
    expected: &str,
) -> Result<Value, String> {
    let value = wire.tool(tool, args.clone()).await?;
    let result = if let Some(operation) = value["structuredContent"]["data"]["operationId"].as_str()
    {
        wire.settled(operation).await?
    } else {
        value
    };
    let data = &result["structuredContent"]["data"];
    require(
        if expected == "succeeded" {
            data["state"] == expected
        } else {
            data["result"]["code"] == expected || result["structuredContent"]["code"] == expected
        },
        &format!("Expected {expected} from {tool}: {result}"),
    )?;
    let replay = wire.tool(tool, args).await?;
    require(
        replay["structuredContent"] == result["structuredContent"],
        "Browser frame exact retry changed its receipt",
    )?;
    Ok(result)
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
    let (_, opened) = layout_call(wire,"lomi_browser_open",json!({"workspaceId":workspace,"url":format!("{}/frames",fixture["origin"].as_str().unwrap()),"retryEpoch":epoch,"requestKey":"frames-open"})).await?;
    let target = &opened["structuredContent"]["data"]["result"];
    let browser = app
        .get_webview(&format!(
            "browser-{}",
            target["panelId"].as_str().ok_or("Missing browser panel")?
        ))
        .ok_or("Missing frame child browser")?;
    wait_for(&browser,"Boolean(document.querySelector('#child')?.contentDocument?.querySelector('#nested')?.contentDocument?.querySelector('#save'))").await?;
    let args = json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"maxNodes":100,"maxBytes":16384});
    let initial = snapshot(wire, &args).await?;
    require(
        initial["frames"].as_array().map(Vec::len) == Some(3) && initial["omittedFrames"] == 3,
        &format!("Wrong native frame coverage: {initial}"),
    )?;
    require(
        !initial.to_string().contains("private-"),
        "Frame snapshot disclosed private fields or omitted content",
    )?;
    let mut checks = vec![
        "native isolated world reads three same-origin frames",
        "opaque, srcdoc and hidden frames omitted",
        "private field values omitted",
    ];
    let input = element(&initial, "Nested", "textbox")?;
    let button = element(&initial, "Save Nested", "button")?;
    let base = json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"navigationId":initial["navigationId"],"snapshotId":initial["snapshotId"],"leaseId":target["leaseId"],"retryEpoch":epoch});
    evaluate(&browser, "(()=>{const w=document.querySelector('#child').contentDocument.querySelector('#nested').contentWindow;for(const key of ['HTMLInputElement','InputEvent','KeyboardEvent'])w[key]=function(){throw Error('page-world-constructor-used')};w.__lomiAgentDomV1={snapshot:'page-forged'};return true})()").await?;
    checks.push("page-world frame constructors and globals cannot replace the isolated dispatcher");
    let mut fill = base.clone();
    fill["elementRef"] = input.clone();
    fill["text"] = json!("Zażółć 🙂");
    fill["requestKey"] = json!("frames-fill");
    interact(wire, "lomi_browser_fill", fill, "succeeded").await?;
    checks.push("nested Unicode fill with exact retry");
    let mut key = base.clone();
    key["elementRef"] = input;
    key["key"] = json!("Enter");
    key["requestKey"] = json!("frames-key");
    interact(wire, "lomi_browser_key", key, "succeeded").await?;
    wait_for(&browser,"document.querySelector('#child').contentDocument.querySelector('#nested').contentDocument.querySelector('#name').dataset.key === 'Enter'").await?;
    checks.push("key delivered only to focused nested document");
    let mut click_args = base.clone();
    click_args["elementRef"] = button;
    click_args["requestKey"] = json!("frames-click");
    interact(wire, "lomi_browser_click", click_args.clone(), "succeeded").await?;
    wait_for(&browser,"(()=>{const d=document.querySelector('#child').contentDocument.querySelector('#nested').contentDocument;return d.querySelector('#result').textContent==='Saved: Zażółć 🙂' && d.querySelector('#save').dataset.count==='1'})()").await?;
    checks.push("nested click confirmed once in actual frame");
    screenshot(&browser, directory.join("browser-frames.png")).await?;
    evaluate(&browser,"document.querySelector('#child').contentDocument.querySelector('#nested').src='/nested?replacement';true").await?;
    wait_for(&browser,"document.querySelector('#child').contentDocument.querySelector('#nested').contentWindow.location.search==='?replacement' && document.querySelector('#child').contentDocument.querySelector('#nested').contentDocument.querySelector('#name').value===''").await?;
    click_args["requestKey"] = json!("frames-stale-child");
    interact(wire, "lomi_browser_click", click_args, "STALE_SNAPSHOT").await?;
    checks.push("child document replacement expires old refs");
    let fresh = snapshot(wire, &args).await?;
    let child_button = element(&fresh, "Save Child", "button")?;
    let mut action = base.clone();
    action["navigationId"] = fresh["navigationId"].clone();
    action["snapshotId"] = fresh["snapshotId"].clone();
    action["elementRef"] = child_button;
    action["requestKey"] = json!("frames-covered");
    evaluate(&browser,"(()=>{const r=document.querySelector('#child').getBoundingClientRect();const e=document.createElement('div');e.id='cover';e.style.cssText=`position:fixed;left:${r.left}px;top:${r.top}px;width:620px;height:80px;z-index:100;background:white`;document.body.append(e);return true})()").await?;
    interact(
        wire,
        "lomi_browser_click",
        action.clone(),
        "PANEL_NOT_RENDERABLE",
    )
    .await?;
    evaluate(&browser,"document.querySelector('#cover').remove();document.querySelector('#child').contentDocument.querySelector('#name').focus();true").await?;
    checks.push("parent occlusion prevents hidden child input");
    let mut scroll = action.clone();
    scroll.as_object_mut().unwrap().remove("elementRef");
    scroll["viewportRef"] = json!("f1-viewport");
    scroll["requestKey"] = json!("frames-scroll");
    scroll["deltaX"] = json!(0);
    scroll["deltaY"] = json!(200);
    interact(wire, "lomi_browser_scroll", scroll, "succeeded").await?;
    wait_for(
        &browser,
        "scrollY===0 && document.querySelector('#child').contentWindow.scrollY===200",
    )
    .await?;
    checks.push("scroll changes only requested frame viewport");
    evaluate(&browser, "document.querySelector('#child').remove();true").await?;
    action["requestKey"] = json!("frames-detached");
    interact(wire, "lomi_browser_click", action, "STALE_SNAPSHOT").await?;
    checks.push("detached frame cannot receive input");
    let mut foreign = args.clone();
    foreign["workspaceId"] = json!("foreign-workspace");
    require(
        wire.tool("lomi_browser_snapshot", foreign).await?["structuredContent"]["code"]
            == "TARGET_NOT_FOUND",
        "Foreign workspace disclosed frame DOM",
    )?;
    checks.push("foreign workspace scope denied");
    let (_, closed)=layout_call(wire,"lomi_panel_close",json!({"workspaceId":workspace,"panelId":target["panelId"],"retryEpoch":epoch,"requestKey":"frames-close","browserGeneration":target["browserGeneration"]})).await?;
    require(
        closed["structuredContent"]["data"]["state"] == "succeeded",
        "Frame browser did not close",
    )?;
    checks.push("native child cleanup");
    screenshot(main, directory.join("browser-frames-cleanup.png")).await?;
    std::fs::write(
        directory.join("browser-frames.json"),
        serde_json::to_vec_pretty(&json!({"checks":checks,"initial":initial,"closed":closed}))
            .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
