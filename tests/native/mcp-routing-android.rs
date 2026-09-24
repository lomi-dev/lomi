//! Actual-model APK workflow against the explicitly licensed disposable device.
use super::*;

async fn check(settings: &Webview, article: &str, label: &str) -> Result<(), String> {
    let input = format!("[...({article}).querySelectorAll('label')].find(e=>e.textContent.includes({})).querySelector('input')", json!(label));
    wait_for(settings, &format!("!({input}).disabled")).await?;
    evaluate(
        settings,
        &format!("(()=>{{const e={input};if(!e.checked)e.click();return true;}})()"),
    )
    .await?;
    Ok(())
}

pub(super) async fn grant(
    settings: &Webview,
    article: &str,
    directory: &Path,
) -> Result<(), String> {
    let device = read_json(&directory.join("android-fixture.json"))?;
    check(settings, article, "Allow reading project files").await?;
    check(
        settings,
        article,
        "Allow importing APK files from this project",
    )
    .await?;
    check(
        settings,
        article,
        "Allow reading selected Android device status",
    )
    .await?;
    let select = format!("({article}).querySelector('select[id^=control-android-]')");
    wait_for(
        settings,
        &format!(
            "[...({select}?.options??[])].some(o=>o.value==={})",
            device["deviceId"]
        ),
    )
    .await?;
    evaluate(settings, &format!("(()=>{{const s={select};s.value={};s.dispatchEvent(new Event('change',{{bubbles:true}}));return true;}})()", device["deviceId"])).await?;
    for label in [
        "Allow starting and stopping this Android device",
        "Allow touch, keys and text in this Android device",
        "Allow reading screen content in this Android device",
        "Allow screenshots of this Android device",
        "Allow requesting APK installation on this Android device",
        "Allow launching approved Android apps",
    ] {
        check(settings, article, label).await?;
    }
    evaluate(settings, &format!("(()=>{{const e=({article}).querySelector('textarea[id^=control-packages-]');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,'org.lomi.inputtest');e.dispatchEvent(new Event('input',{{bubbles:true}}));return true;}})()")).await?;
    Ok(())
}

pub(super) async fn approve_install(
    app: &tauri::AppHandle,
    settings: &Webview,
    directory: &Path,
    workspace: &Value,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let approved = directory.join("routing-install-approval.json");
    if approved.exists() {
        return Ok(());
    }
    let pending = evaluate(settings, "Boolean([...document.querySelectorAll('button')].find(b=>b.textContent==='Install this APK'&&!b.disabled))").await?;
    if pending != true {
        return Ok(());
    }
    let bytes =
        std::fs::read(directory.join("project/mcp-input-test.apk")).map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let device = read_json(&directory.join("android-fixture.json"))?;
    let detail = evaluate(settings, "[...document.querySelectorAll('button')].find(b=>b.textContent==='Install this APK').closest('article').innerText").await?;
    let text = detail.as_str().ok_or("Missing install approval detail")?;
    if !text.contains(&hash)
        || !text.contains(device["deviceId"].as_str().ok_or("Missing device")?)
        || !text.contains("mcp-input-test.apk")
        || !text.contains("Workspace: Visible workspace")
    {
        return Err("Model installation request differs from the authorized fixture".into());
    }
    write_json(
        &approved,
        &json!({"sha256":hash,"byteLength":bytes.len(),"deviceId":device["deviceId"],"workspaceId":workspace,"detail":detail}),
    )?;
    click(settings, "Install this APK").await?;
    activate_main(app).await?;
    Ok(())
}

pub(super) async fn verify(
    app: &tauri::AppHandle,
    directory: &Path,
    report: &Value,
    terminal: &Value,
) -> Result<Value, String> {
    let calls = report["items"].as_array().ok_or("Missing model calls")?;
    let results: Vec<_> = calls
        .iter()
        .filter(|c| c["type"] == "mcpToolCall" && c["server"] == "lomi_probe")
        .map(|c| &c["result"]["structuredContent"]["data"]["result"])
        .collect();
    let installed = results
        .iter()
        .find(|r| {
            r["kind"] == "android_install"
                && r["installed"] == true
                && r["packageName"] == "org.lomi.inputtest"
        })
        .ok_or("Model did not observe successful fixture installation")?;
    let approval = read_json(&directory.join("routing-install-approval.json"))?;
    if installed["sha256"] != approval["sha256"]
        || installed["deviceId"] != approval["deviceId"]
        || installed["workspaceId"] != approval["workspaceId"]
    {
        return Err(
            "Completed installation differs from the approved artifact/device/workspace".into(),
        );
    }
    if !terminal["text"]
        .as_str()
        .unwrap_or("")
        .contains("LOMI_APK_RESULT=")
    {
        return Err("APK build did not render its result in the actual terminal".into());
    }
    let main = app.get_webview("main").ok_or("Missing main")?;
    let native = javascript(&main,"const state=await window.__TAURI_INTERNALS__.invoke('android_state');return {streams:state.streams,panels:[...document.querySelectorAll('[data-android-pane-id]')].map(e=>e.getAttribute('data-android-pane-id'))};").await?;
    if native["streams"].as_array().is_none_or(|streams| {
        !streams.iter().any(|s| {
            s["deviceId"] == installed["deviceId"]
                && s["generation"] == installed["generation"]
                && s["phase"] == "streaming"
        })
    }) {
        return Err("Installed generation is not the native streaming device".into());
    }
    for call in calls.iter().filter(|c| {
        c["type"] == "mcpToolCall"
            && c["server"] == "lomi_probe"
            && matches!(
                c["tool"].as_str(),
                Some("lomi_android_input" | "lomi_android_screenshot")
            )
    }) {
        if call["arguments"]["deviceId"] != installed["deviceId"]
            || call["arguments"]["generation"] != installed["generation"]
            || native["panels"]
                .as_array()
                .is_none_or(|panels| !panels.contains(&call["arguments"]["panelId"]))
        {
            return Err(
                "Model input/capture did not target the visible installed generation".into(),
            );
        }
    }
    let root = crate::android::fixture::directory()?.ok_or("Missing licensed fixture")?;
    let guest = crate::android::fixture::guest(
        &root,
        installed["deviceId"].as_str().ok_or("Missing device ID")?,
    )?;
    let xml = tauri::async_runtime::spawn_blocking(move || guest.native_input_fixture(false))
        .await
        .map_err(|e| e.to_string())??;
    std::fs::write(directory.join("routing-android.xml"), &xml).map_err(|e| e.to_string())?;
    screenshot(&main, directory.join("routing-android.png")).await?;
    if fixture_editor_text(&xml)? != "ROUTING Zażółć 🙂"
        || fixture_node_text(&xml, "android.widget.TextView", "lomi-test-result:1")?
            != "Submitted: ROUTING Zażółć 🙂"
    {
        return Err(
            "Independent guest hierarchy did not confirm exact Unicode input and one submission"
                .into(),
        );
    }
    Ok(
        json!({"passed":true,"case":"apk","terminal":terminal,"installed":installed,"native":native,"oneSubmission":true,"exactUnicode":true}),
    )
}
