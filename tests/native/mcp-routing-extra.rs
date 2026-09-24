//! Independent disk, PTY and native-browser postconditions for distinct tasks.
use super::*;
use base64::Engine;

fn calls(report: &Value) -> impl Iterator<Item = &Value> {
    report["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["type"] == "mcpToolCall" && i["server"] == "lomi_probe")
}
fn saw(report: &Value, tool: &str, text: &str) -> bool {
    calls(report).any(|i| {
        i["tool"] == tool
            && i["result"]["structuredContent"]["status"] == "ok"
            && i["result"]["structuredContent"]["data"]
                .to_string()
                .contains(text)
    })
}
pub(super) async fn files(directory: &Path, case: &str, report: &Value) -> Result<Value, String> {
    let project = directory.join("project");
    if report["competingActions"]
        .as_array()
        .is_none_or(|a| !a.is_empty())
    {
        return Err("Read/file-only routing used a competing execution tool".into());
    }
    match case {
        "files-search" => {
            let expected = "Routing needle: CYAN_ORBIT_482\nPreserve Zażółć 🙂\n";
            if std::fs::read_to_string(project.join("notes/routing-note.txt"))
                .map_err(|e| e.to_string())?
                != expected
                || !saw(report, "lomi_files_search", "notes/routing-note.txt")
                || !saw(report, "lomi_files_read", "CYAN_ORBIT_482")
                || !saw(report, "lomi_files_read", "Zażółć 🙂")
            {
                return Err("Model did not find/read the exact unchanged Unicode fixture".into());
            }
        }
        "files-rename" => {
            if project.join("routing-rename.txt").exists()
                || std::fs::read(project.join("routing-renamed.txt")).map_err(|e| e.to_string())?
                    != "Rename preserves CRLF\r\nZażółć 🙂\r\n".as_bytes()
                || !calls(report).any(|i| {
                    i["tool"] == "lomi_files_read"
                        && i["arguments"]["relativePath"] == "routing-renamed.txt"
                        && i["result"]["structuredContent"]["status"] == "ok"
                })
            {
                return Err(
                    "Rename did not preserve exact bytes and the model's final read".into(),
                );
            }
        }
        "git-review" => {
            let baseline = read_json(&directory.join("routing-extra-baseline.json"))?;
            let head = tokio::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&project)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            let index = tokio::process::Command::new("git")
                .args(["diff", "--cached", "--exit-code", "--quiet"])
                .current_dir(&project)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            if !head.status.success()
                || String::from_utf8_lossy(&head.stdout).trim()
                    != baseline["gitHead"]
                        .as_str()
                        .ok_or("Missing baseline HEAD")?
                || !index.status.success()
                || std::fs::read_to_string(project.join("routing-review.txt"))
                    .map_err(|e| e.to_string())?
                    != "Updated routing review: GREEN_REVIEW_731\n"
                || !saw(report, "lomi_git_diff", "GREEN_REVIEW_731")
                || !saw(report, "lomi_git_status", "routing-review.txt")
            {
                return Err(
                    "Git review did not observe the actual change or modified HEAD/index/file"
                        .into(),
                );
            }
        }
        _ => return Err("Unknown file routing case".into()),
    }
    Ok(json!({"passed":true,"case":case,"nativeDiskPostcondition":true}))
}

pub(super) async fn terminal(
    app: &tauri::AppHandle,
    directory: &Path,
    case: &str,
    report: &Value,
    target: &Value,
    native: &Value,
) -> Result<Value, String> {
    if case == "external-playwright" {
        let external = read_json(&directory.join("project/routing-external-result.json"))?;
        let pid = external["browserPid"]
            .as_u64()
            .ok_or("Missing browser PID")?;
        let ps = tokio::process::Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "ppid=,comm="])
            .output()
            .await
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&ps.stdout);
        let mut fields = text.split_whitespace();
        let parent = fields.next().and_then(|s| s.parse::<u64>().ok());
        let executable = fields.collect::<Vec<_>>().join(" ");
        if !ps.status.success()
            || parent != external["pid"].as_u64()
            || !(executable.contains("Chromium")
                || executable.contains("Chrome")
                || executable.ends_with("/chrome-headless-shell"))
            || !external["userAgent"]
                .as_str()
                .unwrap_or("")
                .contains("HeadlessChrome")
            || !native["text"]
                .as_str()
                .unwrap_or("")
                .contains("ROUTING_EXTERNAL")
            || app
                .webviews()
                .keys()
                .any(|label| label.starts_with("browser-"))
        {
            return Err(
                "External Playwright evidence was not a distinct live Chromium process".into(),
            );
        }
        let screenshot = std::fs::read(directory.join("project/routing-external.png"))
            .map_err(|e| e.to_string())?;
        if !screenshot.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("External screenshot is not PNG".into());
        }
        return Ok(
            json!({"passed":true,"case":case,"external":external,"nativeTerminal":native,"browserExecutable":executable,"noLomiBrowserPanel":true,"modelFinalRequiresReview":true}),
        );
    }
    let main = app.get_webview("main").ok_or("Missing main")?;
    let shell=javascript(&main,&format!("const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal({});return {{promptReady:Boolean(r.atPrompt&&!r.activeBlock),blocks:r.getSnapshot().blocks.slice(-3).map(({{command,exitCode,finished}})=>({{command,exitCode,finished}})),status:r.getSnapshot().status}};",target["panelId"])).await?;
    if shell["promptReady"] != true || shell["status"] != "running" {
        return Err("Model did not leave the original shell ready".into());
    }
    let text = native["text"].as_str().unwrap_or("");
    match case {
        "terminal-repl" => {
            if !text.contains("\"sum\":42")
                || !text.contains("Zażółć 🙂")
                || !saw(report, "lomi_terminal_read", "sum")
            {
                return Err("REPL result was not observed in the actual terminal".into());
            }
        }
        "terminal-interrupt" => {
            if !text.contains("ROUTING_LONG_READY")
                || shell["blocks"]
                    .as_array()
                    .and_then(|b| b.last())
                    .is_none_or(|b| b["exitCode"] != 130)
            {
                return Err("Targeted interrupt did not return exit 130 to its shell".into());
            }
        }
        "terminal-exit" => {
            if calls(report)
                .filter(|i| i["tool"] == "lomi_terminal_run")
                .count()
                != 1
                || !text.contains("ROUTING_EXPECTED_FAILURE")
                || !calls(report).any(|i| {
                    i["result"]["structuredContent"]["data"]["result"]["observation"]["exitCode"]
                        == 7
                })
            {
                return Err("Model did not observe the single expected exit 7".into());
            }
        }
        "terminal-binary" => {
            let raw = calls(report)
                .filter(|i| i["tool"] == "lomi_terminal_read" && i["arguments"]["mode"] == "raw")
                .filter_map(|i| i["result"]["structuredContent"]["data"]["base64"].as_str());
            if !raw.into_iter().any(|s| {
                base64::engine::general_purpose::STANDARD
                    .decode(s)
                    .is_ok_and(|b| b.windows(11).any(|v| v == b"BIN:\0\xff\x01:END"))
            }) {
                return Err("Model did not read the actual binary output".into());
            }
        }
        _ => return Err("Unknown terminal routing case".into()),
    }
    Ok(json!({"passed":true,"case":case,"terminal":native,"shell":shell}))
}

pub(super) fn browser(
    case: &str,
    report: &Value,
    page: &Value,
    state: &Value,
) -> Result<(), String> {
    match case {
        "browser-select" => {
            let value: Value = serde_json::from_str(
                state["preferenceValue"]
                    .as_str()
                    .ok_or("Missing saved preferences")?,
            )
            .map_err(|e| e.to_string())?;
            if state["preferences"] != 1
                || value != json!({"color":"violet","alerts":true})
                || !page["text"].as_str().unwrap_or("").contains("Saved:")
            {
                return Err("Preferences were not submitted once with the exact choices".into());
            }
        }
        "browser-editable" => {
            if state["drafts"] != 1
                || state["draftValue"] != "Draft Zażółć 🙂"
                || !page["text"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Saved: Draft Zażółć 🙂")
            {
                return Err("Contenteditable submission did not preserve exact Unicode".into());
            }
        }
        "browser-navigation" => {
            if state["details"].as_u64().unwrap_or(0) == 0
                || !saw(report, "lomi_browser_snapshot", "Details")
            {
                return Err("Model did not observe the actual details navigation".into());
            }
        }
        _ => return Err("Unknown browser routing case".into()),
    }
    Ok(())
}
