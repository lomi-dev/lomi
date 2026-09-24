//! Native MCP latency and bounded-resource measurements; never a release feature.
use super::*;
use std::time::Instant;

fn data(value: &Value) -> &Value {
    &value["structuredContent"]["data"]
}
fn require(ok: bool, detail: impl Into<String>) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(detail.into())
    }
}
fn stats(values: &[f64], budget: f64) -> Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let p95 = sorted[((sorted.len() as f64 * 0.95).ceil() as usize).saturating_sub(1)];
    json!({"count":sorted.len(),"p95Ms":p95,"maxMs":sorted.last(),"budgetMs":budget,"passed":p95<=budget,"samplesMs":values})
}
async fn timed(
    wire: &mut Wire,
    name: &str,
    args: Value,
    values: &mut Vec<f64>,
) -> Result<Value, String> {
    let start = Instant::now();
    let result = wire.tool(name, args).await?;
    values.push(start.elapsed().as_secs_f64() * 1000.);
    require(
        result["structuredContent"]["status"] == "ok",
        format!("Performance call {name}: {result}"),
    )?;
    Ok(result)
}
async fn processes(helper: u32) -> Result<Value, String> {
    let result = tokio::process::Command::new("/bin/ps")
        .args([
            "-p",
            &format!("{},{}", std::process::id(), helper),
            "-o",
            "pid=,rss=,time=",
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    require(
        result.status.success(),
        "Cannot sample native/helper processes",
    )?;
    let mut rows = Vec::new();
    for line in String::from_utf8(result.stdout)
        .map_err(|e| e.to_string())?
        .lines()
    {
        let cells: Vec<_> = line.split_whitespace().collect();
        require(cells.len() == 3, "Unexpected ps sample")?;
        let cpu = cells[2]
            .split(':')
            .try_fold(0., |seconds, part| {
                part.parse::<f64>().map(|part| seconds * 60. + part)
            })
            .map_err(|e| e.to_string())?;
        rows.push(json!({"pid":cells[0].parse::<u32>().map_err(|e|e.to_string())?,"rssKiB":cells[1].parse::<u64>().map_err(|e|e.to_string())?,"cpuSeconds":cpu}));
    }
    require(rows.len() == 2, "A measured process disappeared")?;
    Ok(json!(rows))
}
pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    workspace: &Value,
    epoch: &Value,
    directory: &Path,
    fixture: &Value,
    helper: u32,
) -> Result<(), String> {
    let duration: u64 = std::env::var("LOMI_MCP_PERFORMANCE_SECONDS")
        .unwrap_or("1800".into())
        .parse()
        .map_err(|_| "Invalid performance duration")?;
    require(
        matches!(duration, 60 | 1800),
        "Use 60 seconds for fixture validation or 1800 for qualification",
    )?;
    let main = app.get_webview("main").ok_or("Missing main")?;
    activate_main(app).await?;
    let (_, terminal) = layout_call(wire,"lomi_terminal_create",json!({"workspaceId":workspace,"cwdRelative":".","title":"Performance fixture","retryEpoch":epoch,"requestKey":"perf-terminal"})).await?;
    let terminal = data(&terminal)["result"].clone();
    let read = json!({"workspaceId":workspace,"panelId":terminal["panelId"],"terminalSessionId":terminal["terminalSessionId"],"maxBytes":4096});
    let run = json!({"workspaceId":workspace,"panelId":terminal["panelId"],"terminalSessionId":terminal["terminalSessionId"],"leaseId":terminal["leaseId"],"retryEpoch":epoch,"command":"sleep 0.1"});
    for _ in 0..100 {
        if data(&wire.tool("lomi_terminal_read", read.clone()).await?)["prompt"] == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut admission = Vec::new();
    for index in 0..23 {
        let mut args = run.clone();
        args["requestKey"] = json!(format!("perf-command-{index}"));
        let result = timed(wire, "lomi_terminal_run", args, &mut admission).await?;
        let settled = wire
            .settled(
                data(&result)["operationId"]
                    .as_str()
                    .ok_or("Missing performance receipt")?,
            )
            .await?;
        require(
            data(&settled)["state"] == "succeeded"
                && data(&settled)["result"]["observation"]["exitCode"] == 0,
            format!("Performance command failed: {settled}"),
        )?;
    }
    admission.drain(..3);
    let (_, opened) = layout_call(wire,"lomi_browser_open",json!({"workspaceId":workspace,"url":fixture["origin"],"retryEpoch":epoch,"requestKey":"perf-browser"})).await?;
    let target = data(&opened)["result"].clone();
    let browser = app
        .get_webview(&format!(
            "browser-{}",
            target["panelId"].as_str().ok_or("Missing browser panel")?
        ))
        .ok_or("Missing native browser")?;
    wait_for(&browser, "document.readyState==='complete'").await?;
    evaluate(&browser,"document.title='MCP performance';document.body.replaceChildren();for(let i=0;i<200;i++){const b=document.createElement('button');b.textContent='Fixture '+i;document.body.append(b)}for(let i=0;i<90;i++)document.body.append(document.createElement('span'));true").await?;
    let snapshot_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"maxNodes":500,"maxBytes":49152});
    let mut status = Vec::new();
    let mut panels = Vec::new();
    let mut snapshots = Vec::new();
    let mut screenshots = Vec::new();
    let mut image_args = Value::Null;
    let mut image_metadata = Value::Null;
    for index in 0..23 {
        timed(wire, "lomi_status", json!({}), &mut status).await?;
        timed(
            wire,
            "lomi_panel_list",
            json!({"workspaceId":workspace}),
            &mut panels,
        )
        .await?;
        let snapshot = timed(
            wire,
            "lomi_browser_snapshot",
            snapshot_args.clone(),
            &mut snapshots,
        )
        .await?;
        require(
            data(&snapshot)["elements"]
                .as_array()
                .is_some_and(|nodes| nodes.len() >= 200),
            "Performance DOM did not include its fixture controls",
        )?;
        image_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"browserGeneration":target["browserGeneration"],"navigationId":data(&snapshot)["navigationId"]});
        let image = timed(
            wire,
            "lomi_browser_screenshot",
            image_args.clone(),
            &mut screenshots,
        )
        .await?;
        require(
            image["content"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["type"] == "image")),
            "Screenshot omitted MCP image",
        )?;
        image_metadata = data(&image)["artifact"]["image"].clone();
        if index == 22 {
            let encoded = image["content"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["type"] == "image")
                .unwrap()["data"]
                .as_str()
                .ok_or("Missing MCP image bytes")?;
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                .map_err(|e| e.to_string())?;
            std::fs::write(directory.join("performance-browser.png"), bytes)
                .map_err(|e| e.to_string())?;
        }
        if index == 2 {
            status.clear();
            panels.clear();
            snapshots.clear();
            screenshots.clear();
        }
    }
    screenshot(&main, directory.join("performance-main.png")).await?;
    let warmup = if duration == 1800 { 120 } else { 2 };
    tokio::time::sleep(Duration::from_secs(warmup)).await;
    let start = Instant::now();
    let idle_seconds = duration / 3;
    let mut samples =
        vec![json!({"elapsedSeconds":0,"phase":"idle","processes":processes(helper).await?})];
    let step = if duration == 1800 { 30 } else { 5 };
    let mut cycle = 0;
    while start.elapsed().as_secs() < duration {
        tokio::time::sleep(Duration::from_secs(step)).await;
        let elapsed = start.elapsed().as_secs_f64();
        let phase = if elapsed <= idle_seconds as f64 + 1. {
            "idle"
        } else {
            "mixed"
        };
        samples.push(
            json!({"elapsedSeconds":elapsed,"phase":phase,"processes":processes(helper).await?}),
        );
        std::fs::write(
            directory.join("performance-progress.json"),
            serde_json::to_vec_pretty(&json!({"durationSeconds":duration,"samples":samples}))
                .unwrap(),
        )
        .map_err(|e| e.to_string())?;
        if phase == "mixed" {
            timed(wire, "lomi_status", json!({}), &mut status).await?;
            timed(
                wire,
                "lomi_panel_list",
                json!({"workspaceId":workspace}),
                &mut panels,
            )
            .await?;
            timed(
                wire,
                "lomi_browser_snapshot",
                snapshot_args.clone(),
                &mut snapshots,
            )
            .await?;
            if cycle % 4 == 0 {
                timed(
                    wire,
                    "lomi_browser_screenshot",
                    image_args.clone(),
                    &mut screenshots,
                )
                .await?;
            }
            cycle += 1;
        }
    }
    let cpu_total = |sample: &Value| {
        sample["processes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["cpuSeconds"].as_f64().unwrap())
            .sum::<f64>()
    };
    let idle_end = samples
        .iter()
        .rev()
        .find(|s| s["phase"] == "idle")
        .ok_or("Missing idle window")?;
    let idle_cpu = (cpu_total(idle_end) - cpu_total(&samples[0]))
        / idle_end["elapsedSeconds"].as_f64().unwrap()
        * 100.;
    let latency = json!({"status":stats(&status,200.),"panels":stats(&panels,200.),"admission":stats(&admission,500.),"snapshot":stats(&snapshots,1000.),"screenshot":stats(&screenshots,1500.)});
    let passed = latency
        .as_object()
        .unwrap()
        .values()
        .all(|s| s["passed"] == true);
    std::fs::write(directory.join("performance.json"),serde_json::to_vec_pretty(&json!({"qualifiedDuration":duration==1800,"durationSeconds":start.elapsed().as_secs_f64(),"warmupSeconds":warmup,"build":"debug mcp-probe + Vite","scope":"Native app and helper CPU/RSS; excludes renderer, emulator and model time","idleSeconds":idle_end["elapsedSeconds"],"idleCpuPercentOneCore":idle_cpu,"latency":latency,"image":image_metadata,"processSamples":samples,"latencyPassed":passed})).unwrap()).map_err(|e|e.to_string())?;
    require(
        passed,
        "Performance budget exceeded; inspect performance.json",
    )?;
    Ok(())
}
