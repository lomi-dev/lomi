//! Native enrollment/stdio qualification. Compiled only with mcp-probe.
#[path = "mcp-browser-upload-support.rs"]
mod browser_upload_probe;
#[path = "mcp-performance-support.rs"]
mod performance_probe;
#[path = "mcp-routing-support.rs"]
mod routing_probe;
#[path = "mcp-terminal-control-support.rs"]
mod terminal_probe;
use crate::mcp_browser_probe::{evaluate, screenshot, wait_for};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{Emitter, Listener, Manager, Webview};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[path = "mcp-android-layout-support.rs"]
mod android_layout_probe;
#[path = "mcp-android-setup-support.rs"]
mod android_setup_probe;
#[path = "mcp-artifact-files-support.rs"]
mod artifact_files_probe;
#[path = "mcp-browser-download-support.rs"]
mod browser_download_probe;
#[path = "mcp-browser-frames-support.rs"]
mod browser_frames_probe;
#[path = "mcp-browser-logs-support.rs"]
mod browser_logs_probe;
#[path = "mcp-chat-support.rs"]
mod chat_probe;
#[path = "mcp-settings-support.rs"]
mod settings_probe;
#[path = "mcp-theme-support.rs"]
mod theme_probe;

pub(crate) fn record_close(value: Value) {
    use std::io::Write;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let Ok(_lock) = LOCK.lock() else {
        return;
    };
    let Some(directory) = std::env::var_os("LOMI_MCP_CONTROL_PROBE_DIRECTORY") else {
        return;
    };
    let path = PathBuf::from(directory).join("native-close-trace.jsonl");
    if path.metadata().is_ok_and(|m| m.len() > 256 * 1024) {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{value}");
    }
}

fn fixture_editor_text(xml: &str) -> Result<String, String> {
    fixture_node_text(xml, "android.widget.EditText", "lomi-test-editor")
}

fn fixture_node_text(xml: &str, class: &str, description: &str) -> Result<String, String> {
    use quick_xml::{events::Event, Reader, XmlVersion};
    let mut reader = Reader::from_str(xml);
    let mut text = None;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(node) | Event::Empty(node) if node.name().as_ref() == b"node" => {
                let attrs = node
                    .attributes()
                    .map(|attr| {
                        let attr = attr.map_err(|e| e.to_string())?;
                        Ok((
                            attr.key.as_ref().to_vec(),
                            attr.normalized_value(XmlVersion::Implicit1_0)
                                .map_err(|e| e.to_string())?
                                .into_owned(),
                        ))
                    })
                    .collect::<Result<std::collections::BTreeMap<_, _>, String>>()?;
                if attrs.get(b"class".as_slice()).map(String::as_str) == Some(class)
                    && attrs.get(b"content-desc".as_slice()).map(String::as_str)
                        == Some(description)
                {
                    if text.is_some() {
                        return Err("Ambiguous guest fixture node".into());
                    }
                    text = attrs.get(b"text".as_slice()).cloned();
                }
            }
            Event::Eof => return text.ok_or_else(|| "Guest fixture node is missing".into()),
            Event::DocType(_) => return Err("Unexpected guest fixture XML DTD".into()),
            _ => {}
        }
    }
}

async fn git_fixture_command(repository: &Path, args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("/Library/Developer/CommandLineTools/usr/bin/git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", repository)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Fixture Git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}
async fn qualify_git_observations(
    wire: &mut Wire,
    directory: &Path,
    workspace: &str,
) -> Result<(), String> {
    let repository = directory.join("project/mcp-git-read");
    git_fixture_command(&repository, &["add", "--", "a.txt", "b.txt"]).await?;
    git_fixture_command(
        &repository,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "Native history 🙂\n\nExact fixture body\n",
        ],
    )
    .await?;
    let commit = git_fixture_command(&repository, &["rev-parse", "HEAD"])
        .await?
        .trim()
        .to_string();
    std::fs::write(repository.join("a.txt"), "staged 🙂\n").map_err(|e| e.to_string())?;
    git_fixture_command(&repository, &["add", "--", "a.txt"]).await?;
    std::fs::write(repository.join("a.txt"), "working Zażółć 🙂\n").map_err(|e| e.to_string())?;
    git_fixture_command(&repository, &["config", "remote.origin.url", "https://fixture-user:fixture-secret@example.invalid/private-location?private=value#hidden"]).await?;
    let base = json!({"workspaceId": workspace,"repositoryRelative":"mcp-git-read"});
    let mut args = base.clone();
    args["relativePath"] = json!("a.txt");
    args["comparison"] = json!("worktree");
    args["maxChars"] = json!(17);
    let first = wire.tool("lomi_git_diff", args.clone()).await?;
    let mut page = first.clone();
    let mut patch = String::new();
    for _ in 0..100 {
        let data = &page["structuredContent"]["data"];
        patch.push_str(
            data["patch"]
                .as_str()
                .ok_or_else(|| format!("Git diff: {page}"))?,
        );
        if data["nextUtf16"].is_null() {
            break;
        }
        args["startUtf16"] = data["nextUtf16"].clone();
        args["expectedObservationRevision"] = data["observationRevision"].clone();
        page = wire.tool("lomi_git_diff", args.clone()).await?;
    }
    if !patch.contains("+working Zażółć 🙂\n")
        || !patch.contains("-staged 🙂\n")
        || !page["structuredContent"]["data"]["nextUtf16"].is_null()
    {
        return Err(format!("Native Git patch pagination failed: {patch}"));
    }
    std::fs::write(repository.join("a.txt"), "changed after observation\n")
        .map_err(|e| e.to_string())?;
    let conflict = wire.tool("lomi_git_diff", args.clone()).await?;
    if conflict["structuredContent"]["code"] != "REVISION_CONFLICT" {
        return Err(format!("Git diff changed revision: {conflict}"));
    }
    args["comparison"] = json!("staged");
    args["startUtf16"] = json!(0);
    args["maxChars"] = json!(8192);
    args["expectedObservationRevision"] = Value::Null;
    let staged = wire.tool("lomi_git_diff", args.clone()).await?;
    if !staged["structuredContent"]["data"]["patch"]
        .as_str()
        .is_some_and(|p| p.contains("+staged 🙂\n") && p.contains("-fixture"))
    {
        return Err(format!("Native staged diff failed: {staged}"));
    }
    args["relativePath"] = json!(".env");
    let denied = wire.tool("lomi_git_diff", args).await?;
    if denied["structuredContent"]["status"] != "error" {
        return Err("Git diff disclosed secret file".into());
    }
    let history = wire.tool("lomi_git_history", base.clone()).await?;
    if history["structuredContent"]["data"]["startCommit"] != commit
        || history["structuredContent"]["data"]["commits"][0]["subject"] != "Native history 🙂"
    {
        return Err(format!("Native history failed: {history}"));
    }
    let mut args = base.clone();
    args["commit"] = json!(commit);
    let details = wire.tool("lomi_git_commit", args).await?;
    if details["structuredContent"]["data"]["message"]
        != "Native history 🙂\n\nExact fixture body\n"
    {
        return Err(format!("Native commit details failed: {details}"));
    }
    let remotes = wire.tool("lomi_git_remotes", base).await?;
    let serialized = remotes.to_string();
    for secret in [
        "fixture-user",
        "fixture-secret",
        "private-location",
        "private=value",
        "hidden",
    ] {
        if serialized.contains(secret) {
            return Err("Native remote metadata exposed a private URL component".into());
        }
    }
    if remotes["structuredContent"]["data"]["remotes"][0]["host"] != "example.invalid"
        || remotes["structuredContent"]["data"]["remotes"][0]["locationRedacted"] != true
    {
        return Err(format!("Native remotes failed: {remotes}"));
    }
    let result = json!({"firstDiffPage":first,"patch":patch,"conflict":conflict,"staged":staged,"denied":denied,"history":history,"commit":details,"remotes":remotes});
    std::fs::write(
        directory.join("git-observations.json"),
        serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn qualify_git_pulls(
    wire: &mut Wire,
    main: &Webview,
    directory: &Path,
    workspace: &str,
    connected: &Value,
) -> Result<(), String> {
    let mut results = Vec::new();
    for case in ["cancel", "ff", "remote-changed", "rebase", "conflict"] {
        let relative = format!("mcp-pull-{case}");
        let repository = directory.join("project").join(&relative);
        let remote = directory.join(format!("pull-{case}.git"));
        let peer = directory.join(format!("pull-{case}-peer"));
        std::fs::create_dir(&repository).map_err(|e| e.to_string())?;
        git_fixture_command(&repository, &["init", "-q", "--initial-branch=main"]).await?;
        for (key, value) in [
            ("user.name", "Native Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgSign", "false"),
        ] {
            git_fixture_command(&repository, &["config", key, value]).await?;
        }
        std::fs::write(repository.join("a.txt"), "base\n").map_err(|e| e.to_string())?;
        git_fixture_command(&repository, &["add", "."]).await?;
        git_fixture_command(&repository, &["commit", "-qm", "base"]).await?;
        let remote_path = remote.to_str().ok_or("Non-UTF8 pull fixture")?;
        let peer_path = peer.to_str().ok_or("Non-UTF8 pull fixture")?;
        git_fixture_command(directory, &["init", "-q", "--bare", remote_path]).await?;
        git_fixture_command(&repository, &["remote", "add", "fixture", remote_path]).await?;
        git_fixture_command(
            &repository,
            &["push", "-q", "fixture", "HEAD:refs/heads/main"],
        )
        .await?;
        git_fixture_command(
            directory,
            &["clone", "-q", "--branch=main", remote_path, peer_path],
        )
        .await?;
        for (key, value) in [
            ("user.name", "Native Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgSign", "false"),
        ] {
            git_fixture_command(&peer, &["config", key, value]).await?;
        }
        let rebasing = matches!(case, "rebase" | "conflict");
        if rebasing {
            let name = if case == "conflict" {
                "a.txt"
            } else {
                "local.txt"
            };
            std::fs::write(repository.join(name), "local committed work\n")
                .map_err(|e| e.to_string())?;
            git_fixture_command(&repository, &["add", "--", name]).await?;
            git_fixture_command(&repository, &["commit", "-qm", "local work"]).await?;
        }
        std::fs::write(peer.join("a.txt"), "remote committed work\n").map_err(|e| e.to_string())?;
        git_fixture_command(&peer, &["commit", "-qam", "remote work"]).await?;
        git_fixture_command(&peer, &["push", "-q", "origin", "HEAD:refs/heads/main"]).await?;
        git_fixture_command(&repository, &["fetch", "-q", "fixture"]).await?;
        let source = git_fixture_command(&repository, &["rev-parse", "HEAD"])
            .await?
            .trim()
            .to_string();
        let incoming = git_fixture_command(&peer, &["rev-parse", "HEAD"])
            .await?
            .trim()
            .to_string();
        let projection = preview_status(wire, main).await?;
        let args = json!({"workspaceId":workspace,"repositoryRelative":relative,"operation":"pull","paths":[],
            "remote":"fixture","reference":"refs/heads/main","sourceCommit":source,"expectedRemoteCommit":incoming,
            "pullMode":if rebasing {"rebase"} else {"ff_only"},"expectedRevision":projection["structuredContent"]["data"]["domainRevision"],
            "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("native-pull-{case}")});
        let started = wire.tool("lomi_git_mutate", args.clone()).await?;
        let op = started["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| started.to_string())?;
        wait_for(main, "[...document.querySelectorAll('dialog[open]')].some(e=>e.classList.contains('agent-git-approval') && e.textContent.includes('Pull this branch'))").await?;
        let awaiting = wire
            .tool("lomi_operation_get", json!({"operationId":op}))
            .await?;
        if awaiting["structuredContent"]["data"]["state"] != "awaiting_user" {
            return Err(format!("Pull did not require approval: {awaiting}"));
        }
        if case == "rebase" {
            screenshot(main, directory.join("git-pull-approval.png")).await?;
        }
        if case == "remote-changed" {
            std::fs::write(peer.join("a.txt"), "later remote work\n").map_err(|e| e.to_string())?;
            git_fixture_command(&peer, &["commit", "-qam", "another client"]).await?;
            git_fixture_command(&peer, &["push", "-q", "origin", "HEAD:refs/heads/main"]).await?;
        }
        let label = if case == "cancel" {
            "Cancel"
        } else {
            "Pull this branch"
        };
        javascript(main, &format!("const d=document.querySelector('dialog.agent-git-approval[open]');const b=[...d.querySelectorAll('button')].find(e=>e.textContent.trim()==={});if(!b)throw Error('Missing pull decision');b.click();return true;",json!(label))).await?;
        let receipt = wire.settled(op).await?;
        let data = &receipt["structuredContent"]["data"];
        let (state, effect, outcome) = match case {
            "cancel" => ("cancelled", "none", None),
            "remote-changed" => ("failed", "partial", Some("remote_changed")),
            "conflict" => ("failed", "partial", Some("conflicted")),
            _ => ("succeeded", "complete", Some("applied")),
        };
        if data["state"] != state
            || data["effectState"] != effect
            || outcome.is_some_and(|expected| data["result"]["pull"]["outcome"] != expected)
        {
            return Err(format!("Native pull {case} receipt mismatch: {receipt}"));
        }
        let bytes = std::fs::read_to_string(repository.join("a.txt")).map_err(|e| e.to_string())?;
        match case {
            "cancel" | "remote-changed" => {
                if bytes != "base\n"
                    || git_fixture_command(&repository, &["rev-parse", "HEAD"])
                        .await?
                        .trim()
                        != source
                {
                    return Err("Pull integrated an unapproved commit".into());
                }
            }
            "conflict" => {
                if !repository.join(".git/rebase-merge").is_dir()
                    || !bytes.contains("local committed work")
                    || git_fixture_command(&repository, &["ls-files", "--unmerged"])
                        .await?
                        .is_empty()
                {
                    return Err("Rebase conflict was not preserved".into());
                }
            }
            _ => {
                if bytes != "remote committed work\n" {
                    return Err("Pull did not integrate exact remote bytes".into());
                }
                if rebasing
                    && std::fs::read_to_string(repository.join("local.txt"))
                        .map_err(|e| e.to_string())?
                        != "local committed work\n"
                {
                    return Err("Rebase lost local committed work".into());
                }
            }
        }
        let repeated = wire.tool("lomi_git_mutate", args).await?;
        if repeated["structuredContent"]["data"]["operationId"] != op
            || repeated["structuredContent"]["data"]["state"] != state
        {
            return Err("Pull retry changed its durable receipt".into());
        }
        results.push(json!({"case":case,"receipt":receipt,"repeat":repeated}));
    }
    std::fs::write(
        directory.join("git-pulls.json"),
        serde_json::to_vec_pretty(&results).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn qualify_git_mutations(
    wire: &mut Wire,
    main: &Webview,
    directory: &Path,
    workspace: &str,
    connected: &Value,
) -> Result<(), String> {
    let repository = directory.join("project/mcp-git-read");
    let path = repository.join("approval.txt");
    std::fs::write(&path, "Native approved bytes 🙂\n").map_err(|e| e.to_string())?;
    let mut receipts = Vec::new();
    let mut push_expected = None;
    for case in [
        "cancel",
        "stale",
        "cancel-tool",
        "stage",
        "unstage",
        "commit-cancel",
        "commit",
        "fetch-cancel",
        "fetch",
        "push-cancel",
        "push",
        "discard-cancel",
        "discard-stale",
        "discard",
    ] {
        let discarding = case.starts_with("discard");
        if discarding {
            std::fs::write(&path, "New native disk version 🙂\n").map_err(|e| e.to_string())?;
            git_fixture_command(&repository, &["add", "--", "approval.txt"]).await?;
            std::fs::write(&path, "Unstaged native work to discard 🙂\n")
                .map_err(|e| e.to_string())?;
        }
        let pushing = case.starts_with("push");
        if case == "push-cancel" {
            push_expected = Some(
                git_fixture_command(&repository, &["rev-parse", "HEAD"])
                    .await?
                    .trim()
                    .to_string(),
            );
            std::fs::write(
                repository.join("push-proof.txt"),
                "Native publication fixture only\n",
            )
            .map_err(|e| e.to_string())?;
            git_fixture_command(&repository, &["add", "--", "push-proof.txt"]).await?;
            git_fixture_command(
                &repository,
                &["commit", "-qm", "Native push fixture advance"],
            )
            .await?;
        }
        let fetching = case.starts_with("fetch");
        if case == "fetch-cancel" {
            let remote = directory.join("git-fetch-remote.git");
            std::fs::create_dir(&remote).map_err(|e| e.to_string())?;
            git_fixture_command(&remote, &["init", "-q", "--bare"]).await?;
            git_fixture_command(
                &repository,
                &[
                    "push",
                    "-q",
                    remote.to_str().ok_or("Non-UTF8 fixture remote")?,
                    "HEAD:refs/heads/native-fixture",
                ],
            )
            .await?;
            git_fixture_command(
                &repository,
                &[
                    "remote",
                    "add",
                    "fixture",
                    remote.to_str().ok_or("Non-UTF8 fixture remote")?,
                ],
            )
            .await?;
        }
        let committing = case.starts_with("commit");
        if committing {
            git_fixture_command(&repository, &["config", "user.name", "Native Fixture"]).await?;
            git_fixture_command(
                &repository,
                &["config", "user.email", "native-fixture@example.invalid"],
            )
            .await?;
            git_fixture_command(&repository, &["add", "--", "approval.txt"]).await?;
        }
        let status = preview_status(wire, main).await?;
        let mut args = json!({"workspaceId":workspace,"repositoryRelative":"mcp-git-read","operation":if case == "unstage" { "unstage" } else { "stage" },"paths":["approval.txt"],"expectedRevision":status["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("git-mutate-{case}")});
        if discarding {
            args["operation"] = json!("discard");
        }
        if committing {
            args["operation"] = json!("commit");
            args["paths"] = json!(["a.txt", "approval.txt"]);
            args["message"] = json!("  Native approved commit 🙂  \n\nExact body  \n");
        }
        if fetching {
            args["operation"] = json!("fetch");
            args["paths"] = json!([]);
            args["remote"] = json!("fixture");
            args["reference"] = json!("refs/heads/native-fixture");
        }
        if pushing {
            args["operation"] = json!("push");
            args["paths"] = json!([]);
            args["remote"] = json!("fixture");
            args["reference"] = json!("refs/heads/native-fixture");
            args["sourceCommit"] = json!(git_fixture_command(&repository, &["rev-parse", "HEAD"])
                .await?
                .trim());
            args["expectedRemoteCommit"] = json!(push_expected);
        }
        let start = wire.tool("lomi_git_mutate", args.clone()).await?;
        let op = start["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| start.to_string())?;
        wait_for(main, "[...document.querySelectorAll('dialog[open], [role=dialog]')].some(e=>e.classList.contains('agent-git-approval'))").await?;
        let awaiting = wire
            .tool("lomi_operation_get", json!({"operationId":op}))
            .await?;
        if awaiting["structuredContent"]["data"]["state"] != "awaiting_user" {
            return Err(format!(
                "Git did not await an exact main-window decision: {awaiting}"
            ));
        }
        if case == "discard" {
            screenshot(main, directory.join("git-discard-approval.png")).await?;
        }
        if case == "push" {
            screenshot(main, directory.join("git-push-approval.png")).await?;
        }
        if case == "fetch" {
            screenshot(main, directory.join("git-fetch-approval.png")).await?;
        }
        if case == "commit" {
            screenshot(main, directory.join("git-commit-approval.png")).await?;
        }
        if case == "stage" {
            screenshot(main, directory.join("git-mutation-approval.png")).await?;
        }
        if matches!(case, "stale" | "discard-stale") {
            std::fs::write(&path, "New native disk version 🙂\n").map_err(|e| e.to_string())?;
        }
        if case == "cancel-tool" {
            let _ = wire
                .tool("lomi_operation_cancel", json!({"operationId":op}))
                .await?;
        } else {
            let label = if matches!(
                case,
                "cancel" | "commit-cancel" | "fetch-cancel" | "push-cancel" | "discard-cancel"
            ) {
                "Cancel"
            } else if discarding {
                "Discard working changes"
            } else if case == "push" {
                "Push this commit"
            } else if case == "fetch" {
                "Fetch this branch"
            } else if case == "commit" {
                "Create this commit"
            } else if case == "unstage" {
                "Unstage these files"
            } else {
                "Stage these files"
            };
            javascript(main, &format!("const dialog=[...document.querySelectorAll('dialog[open], [role=dialog]')].find(e=>e.classList.contains('agent-git-approval'));const button=[...dialog.querySelectorAll('button')].find(e=>e.textContent.trim()==={});if(!button)throw Error('Missing Git decision');button.click();return true;", serde_json::to_string(label).unwrap())).await?;
        }
        wait_for(main, "![...document.querySelectorAll('dialog[open], [role=dialog]')].some(e=>e.classList.contains('agent-git-approval'))").await?;
        let receipt = wire.settled(op).await?;
        let expected = match case {
            "cancel" | "cancel-tool" | "commit-cancel" | "fetch-cancel" | "push-cancel"
            | "discard-cancel" => "cancelled",
            "stale" | "discard-stale" => "failed",
            _ => "succeeded",
        };
        if receipt["structuredContent"]["data"]["state"] != expected {
            return Err(format!("Git mutation {case} failed: {receipt}"));
        }
        if matches!(case, "stale" | "discard-stale")
            && receipt["structuredContent"]["data"]["result"]["code"] != "REVISION_CONFLICT"
        {
            return Err(format!("Git changed bytes were not rejected: {receipt}"));
        }
        let indexed = git_fixture_command(&repository, &["ls-files", "--", "approval.txt"]).await?;
        if indexed.contains("approval.txt")
            != matches!(
                case,
                "stage"
                    | "commit-cancel"
                    | "commit"
                    | "fetch-cancel"
                    | "fetch"
                    | "push-cancel"
                    | "push"
                    | "discard-cancel"
                    | "discard-stale"
                    | "discard"
            )
        {
            return Err(format!("Unexpected index after {case}: {indexed}"));
        }
        if case == "stage"
            && git_fixture_command(&repository, &["show", ":approval.txt"]).await?
                != "New native disk version 🙂\n"
        {
            return Err("Staged bytes differ from the approved disk version".into());
        }
        if case == "commit" {
            let object = git_fixture_command(&repository, &["cat-file", "commit", "HEAD"]).await?;
            if object.split_once("\n\n").map(|v| v.1) != args["message"].as_str()
                || git_fixture_command(&repository, &["show", "HEAD:approval.txt"]).await?
                    != "New native disk version 🙂\n"
                || git_fixture_command(&repository, &["show", "HEAD:a.txt"]).await? != "staged 🙂\n"
                || !receipt["structuredContent"]["data"]["result"]["commit"]["messageSha256"]
                    .is_string()
            {
                return Err(
                    "Native commit did not preserve approved message and staged bytes".into(),
                );
            }
        }
        if case == "fetch-cancel"
            && !git_fixture_command(&repository, &["for-each-ref", "refs/remotes/fixture/"])
                .await?
                .is_empty()
        {
            return Err("Cancelled fetch contacted the remote".into());
        }
        if case == "fetch" {
            let fetched = &receipt["structuredContent"]["data"]["result"]["fetch"];
            let observed = git_fixture_command(
                &repository,
                &["rev-parse", "refs/remotes/fixture/native-fixture"],
            )
            .await?;
            if fetched["commit"].as_str() != Some(observed.trim()) || fetched["remote"] != "fixture"
            {
                return Err("Fetch receipt did not verify the exact tracking commit".into());
            }
        }
        if pushing {
            let remote = git_fixture_command(
                &directory.join("git-fetch-remote.git"),
                &["rev-parse", "refs/heads/native-fixture"],
            )
            .await?;
            let expected = if case == "push-cancel" {
                args["expectedRemoteCommit"].as_str()
            } else {
                args["sourceCommit"].as_str()
            };
            if Some(remote.trim()) != expected {
                return Err("Native push changed an unapproved remote commit".into());
            }
            if case == "push"
                && receipt["structuredContent"]["data"]["result"]["push"]["verification"]
                    != "server_acknowledgement"
            {
                return Err("Missing native server acknowledgement".into());
            }
        }
        if discarding {
            let expected = if case == "discard-cancel" {
                "Unstaged native work to discard 🙂\n"
            } else {
                "New native disk version 🙂\n"
            };
            if std::fs::read_to_string(&path).map_err(|e| e.to_string())? != expected
                || git_fixture_command(&repository, &["show", ":approval.txt"]).await?
                    != "New native disk version 🙂\n"
            {
                return Err("Discard lost unapproved bytes or changed staged content".into());
            }
        }
        let repeated = wire.tool("lomi_git_mutate", args).await?;
        if repeated["structuredContent"]["data"]["operationId"] != op
            || repeated["structuredContent"]["data"]["state"] != expected
        {
            return Err(format!(
                "Git mutation retry changed its receipt: {repeated}"
            ));
        }
        receipts.push(json!({"case":case,"awaiting":awaiting,"receipt":receipt,"repeat":repeated}));
    }
    if std::fs::read_to_string(&path).map_err(|e| e.to_string())? != "New native disk version 🙂\n"
    {
        return Err("Unstage changed the native working file".into());
    }
    std::fs::write(
        directory.join("git-mutations.json"),
        serde_json::to_vec_pretty(&receipts).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn qualify_git_views(
    wire: &mut Wire,
    main: &Webview,
    directory: &Path,
    workspace: &str,
    connected: &Value,
) -> Result<(), String> {
    let repository = directory.join("project/mcp-git-read");
    let commit = git_fixture_command(&repository, &["rev-parse", "HEAD"])
        .await?
        .trim()
        .to_string();
    javascript(main, "window.gitRawReads=[];window.gitOldInvoke=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(command,args,...rest)=>{if(['git_diff','git_commit_details','git_commit_diff'].includes(command))window.gitRawReads.push(command);return window.gitOldInvoke(command,args,...rest);};return true;").await?;
    let mut receipts = Vec::new();
    for (index, view) in [
        json!({"type":"diff","relativePath":"a.txt","staged":false}),
        json!({"type":"commit","commit":commit}),
    ]
    .into_iter()
    .enumerate()
    {
        let status = preview_status(wire, main).await?;
        let args = json!({"workspaceId":workspace,"repositoryRelative":"mcp-git-read","view":view,"expectedRevision":status["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("git-open-{index}")});
        let start = wire.tool("lomi_git_open", args.clone()).await?;
        let op = start["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| start.to_string())?;
        let receipt = wire.settled(op).await?;
        if receipt["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Native Git view failed: {receipt}"));
        }
        let panel = receipt["structuredContent"]["data"]["result"]["panelId"].clone();
        let repeated = wire.tool("lomi_git_open", args).await?;
        if repeated["structuredContent"]["data"]["operationId"] != op {
            return Err("Git view retry created a new operation".into());
        }
        wait_for(main, if index == 0 { "document.querySelector('.working-file-diff .commit-patch')?.textContent.includes('changed after observation')" } else { "document.querySelector('.commit-details .commit-patch')?.textContent.includes('fixture')&&document.querySelector('.commit-details h1')?.textContent==='Native history 🙂'" }).await?;
        let dom = evaluate(main, "({text:document.querySelector('.working-file-diff,.commit-details')?.textContent,ordinaryReads:window.gitRawReads})").await?;
        screenshot(main, directory.join(format!("git-view-{index}.png"))).await?;
        if dom["ordinaryReads"]
            .as_array()
            .is_none_or(|r| !r.is_empty())
        {
            return Err(format!("Git view called an ordinary Git reader: {dom}"));
        }
        let status = preview_status(wire, main).await?;
        let close = wire.tool("lomi_panel_close", json!({"workspaceId":workspace,"panelId":panel,"expectedRevision":status["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("git-view-close-{index}")})).await?;
        let closed = wire
            .settled(
                close["structuredContent"]["data"]["operationId"]
                    .as_str()
                    .ok_or_else(|| close.to_string())?,
            )
            .await?;
        if closed["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Git view close failed: {closed}"));
        }
        receipts.push(json!({"opened":receipt,"repeated":repeated,"dom":dom,"closed":closed}));
    }
    javascript(
        main,
        "window.__TAURI_INTERNALS__.invoke=window.gitOldInvoke;return true;",
    )
    .await?;
    std::fs::write(
        directory.join("git-views.json"),
        serde_json::to_vec_pretty(&receipts).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn qualify_git_status(
    wire: &mut Wire,
    directory: &Path,
    workspace: &str,
) -> Result<(), String> {
    let project = directory.join("project");
    let repository = project.join("mcp-git-read");
    std::fs::create_dir(&repository).map_err(|e| e.to_string())?;
    let init = tokio::process::Command::new("/Library/Developer/CommandLineTools/usr/bin/git")
        .args(["init", "-q"])
        .arg(&repository)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !init.status.success() {
        return Err("Cannot prepare the isolated Git status fixture".into());
    }
    for name in ["a.txt", "b.txt", ".env"] {
        std::fs::write(repository.join(name), b"fixture").map_err(|e| e.to_string())?;
    }
    std::os::unix::fs::symlink("a.txt", repository.join("link.txt")).map_err(|e| e.to_string())?;
    let args = json!({"workspaceId":workspace,"repositoryRelative":"mcp-git-read","limit":1});
    let first = wire.tool("lomi_git_status", args.clone()).await?;
    let data = &first["structuredContent"]["data"];
    if data["source"] != "git"
        || data["consistency"] != "per_command_snapshot"
        || data["changes"][0]["relativePath"] != "mcp-git-read/a.txt"
        || data["omittedEntries"] != 2
        || !data["nextCursor"].is_string()
        || !data["gitVersion"]
            .as_str()
            .is_some_and(|s| s.starts_with("git version "))
    {
        return Err(format!("Git status first page failed: {first}"));
    }
    let mut next_args = args.clone();
    next_args["cursor"] = data["nextCursor"].clone();
    next_args["limit"] = json!(200);
    std::fs::write(repository.join("c.txt"), b"later fixture").map_err(|e| e.to_string())?;
    let next = wire.tool("lomi_git_status", next_args.clone()).await?;
    let tail = &next["structuredContent"]["data"];
    if tail["observationRevision"] != data["observationRevision"]
        || tail["changes"].as_array().map(Vec::len) != Some(1)
        || tail["changes"][0]["relativePath"] != "mcp-git-read/b.txt"
        || !tail["nextCursor"].is_null()
    {
        return Err(format!("Git status cursor lost its observation: {next}"));
    }
    next_args["workspaceId"] = json!("foreign");
    let foreign = wire.tool("lomi_git_status", next_args).await?;
    if foreign["structuredContent"]["status"] != "error" {
        return Err("Git status disclosed a foreign cursor".into());
    }
    let mut fresh = args;
    fresh["limit"] = json!(200);
    let fresh = wire.tool("lomi_git_status", fresh).await?;
    if fresh["structuredContent"]["data"]["observationRevision"] == data["observationRevision"]
        || fresh["structuredContent"]["data"]["changes"]
            .as_array()
            .map(Vec::len)
            != Some(3)
    {
        return Err(format!("New Git status omitted a new file: {fresh}"));
    }
    let result = json!({"first":first,"next":next,"foreign":foreign,"fresh":fresh});
    if result
        .to_string()
        .contains(project.to_str().ok_or("Non-UTF8 Git fixture path")?)
    {
        return Err("Git status exposed an absolute project path".into());
    }
    std::fs::write(
        directory.join("git-status.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn preview_status(wire: &mut Wire, main: &Webview) -> Result<Value, String> {
    if evaluate(main, "document.visibilityState === 'visible'").await? != true {
        activate_main(main.app_handle()).await?;
        wait_for_human_focus(main, "preview").await?;
        wait_for(main, "document.visibilityState === 'visible'").await?;
    }
    wait_for(
        main,
        "document.querySelectorAll('.editor-loading').length===0",
    )
    .await?;
    javascript(
        main,
        "await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));return true;",
    )
    .await?;
    let mut previous = Value::Null;
    for _ in 0..20 {
        let current = wire.tool("lomi_workspace_list", json!({})).await?;
        if current["structuredContent"]["data"]["domainRevision"] == previous {
            return Ok(current);
        }
        previous = current["structuredContent"]["data"]["domainRevision"].clone();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("Preview layout revision did not settle".into())
}

async fn wait_for_human_focus(main: &Webview, stage: &str) -> Result<(), String> {
    if std::env::var("LOMI_MCP_WAIT_FOR_FOCUS").as_deref() != Ok("1") {
        return Ok(());
    }
    let window = main
        .app_handle()
        .get_window("main")
        .ok_or("Missing main window")?;
    eprintln!("MCP_FOCUS_WAIT: {stage}: click the test Lomi window and keep it in front");
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let mut samples = 0;
    while std::time::Instant::now() < deadline {
        if window.is_focused().unwrap_or(false)
            && evaluate(
                main,
                "document.visibilityState === 'visible' && document.hasFocus()",
            )
            .await?
                == true
        {
            samples += 1;
            if samples == 3 {
                eprintln!("MCP_FOCUS_READY: {stage}");
                return Ok(());
            }
        } else {
            samples = 0;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!(
        "Native focus was not available within 120 seconds for {stage}"
    ))
}

async fn qualify_previews(
    wire: &mut Wire,
    main: &Webview,
    directory: &Path,
    workspace: &str,
    connected: &Value,
) -> Result<(), String> {
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    let project = directory.join("project");
    let mut png = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 3, Rgba([220, 50, 40, 128])))
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    std::fs::write(project.join("preview-pixel.png"), png.get_ref()).map_err(|e| e.to_string())?;
    let mut gif = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
        encoder
            .encode_frame(image::Frame::new(RgbaImage::from_pixel(
                2,
                3,
                Rgba([255, 255, 0, 255]),
            )))
            .map_err(|e| e.to_string())?;
        encoder
            .encode_frame(image::Frame::new(RgbaImage::from_pixel(
                2,
                3,
                Rgba([0, 0, 255, 255]),
            )))
            .map_err(|e| e.to_string())?;
    }
    std::fs::write(project.join("preview-animation.gif"), gif).map_err(|e| e.to_string())?;
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(directory.join("browser-fixture.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let blocked = fixture["blocked"]
        .as_str()
        .ok_or("Missing preview tracker")?;
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="48" onload="fetch('{blocked}/svg-onload')"><script>fetch('{blocked}/svg-script');parent.previewEscaped=true;</script><rect width="64" height="48" fill="#00ff00"/><image href="{blocked}/svg-image" width="1" height="1" x="50"/><foreignObject x="50" width="1" height="1"><img xmlns="http://www.w3.org/1999/xhtml" src="{blocked}/foreign-image"/></foreignObject></svg>"##
    );
    std::fs::write(project.join("preview-vector.svg"), svg).map_err(|e| e.to_string())?;
    std::fs::write(project.join(".env.png"), png.get_ref()).map_err(|e| e.to_string())?;
    std::os::unix::fs::symlink(
        project.join("preview-pixel.png"),
        project.join("preview-link.png"),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(project.join("preview-document.md"), format!("# MCP preview Zażółć 🙂\n\n![Local pixel](preview-pixel.png)\n![Local vector](preview-vector.svg)\n![Secret image](.env.png)\n![Linked image](preview-link.png)\n![External tracker]({blocked}/markdown-image)\n")).map_err(|e|e.to_string())?;
    javascript(main, "window.previewRawReads=[];window.previewOldInvoke=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(command,args,...rest)=>{if(['read_image_file','read_markdown_image'].includes(command))window.previewRawReads.push(command);return window.previewOldInvoke(command,args,...rest);};return true;").await?;
    let retry = &connected["structuredContent"]["data"]["retryEpoch"];
    let mut receipts = Vec::new();
    for (index, relative) in [
        "preview-pixel.png",
        "preview-animation.gif",
        "preview-vector.svg",
        "preview-document.md",
    ]
    .iter()
    .enumerate()
    {
        let status = preview_status(wire, main).await?;
        let args = json!({"workspaceId":workspace,"relativePath":relative,"presentation":"preview","expectedRevision":status["structuredContent"]["data"]["domainRevision"],"retryEpoch":retry,"requestKey":format!("preview-{index}")});
        let started = wire.tool("lomi_editor_open", args.clone()).await?;
        let operation = started["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| started.to_string())?;
        let receipt = wire.settled(operation).await?;
        if receipt["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Preview {relative} failed: {receipt}"));
        }
        let result = &receipt["structuredContent"]["data"]["result"];
        let panel = result["panelId"]
            .as_str()
            .ok_or_else(|| receipt.to_string())?;
        let repeated = wire.tool("lomi_editor_open", args).await?;
        if repeated["structuredContent"]["data"]["operationId"] != operation {
            return Err("Preview retry created a new operation".into());
        }
        if index < 3 {
            wait_for(main, &format!("[...document.querySelectorAll('.image-canvas img')].some(i=>i.alt==={}&&i.complete&&i.naturalWidth>0&&i.getBoundingClientRect().width>0)", json!(relative))).await?;
            let pixels = evaluate(main,&format!("(()=>{{const i=[...document.querySelectorAll('.image-canvas img')].find(i=>i.alt==={});const c=document.createElement('canvas');c.width=i.naturalWidth;c.height=i.naturalHeight;const x=c.getContext('2d');x.drawImage(i,0,0);return {{width:i.naturalWidth,height:i.naturalHeight,pixel:[...x.getImageData(0,0,1,1).data]}};}})()",json!(relative))).await?;
            let expected = if index == 0 {
                json!([219, 50, 40, 128])
            } else if index == 1 {
                json!([255, 255, 0, 255])
            } else {
                json!([0, 255, 0, 255])
            };
            // Browser canvas premultiplication may round the translucent red channel by one.
            let valid_pixel = pixels["pixel"] == expected
                || (index == 0 && pixels["pixel"] == json!([220, 50, 40, 128]));
            if !valid_pixel
                || (index < 2
                    && (result["kind"] != "editor_previewed"
                        || result["width"] != 2
                        || result["height"] != 3))
            {
                return Err(format!(
                    "Wrong preview pixels or metadata {relative}: {pixels}; {result}"
                ));
            }
            std::fs::write(
                directory.join(format!("preview-pixels-{index}.json")),
                serde_json::to_vec_pretty(&pixels).unwrap(),
            )
            .map_err(|e| e.to_string())?;
        } else {
            wait_for(main, "[...document.querySelectorAll('.markdown-preview h1')].some(e=>e.textContent==='MCP preview Zażółć 🙂') && document.querySelectorAll('.markdown-preview img').length===2 && [...document.querySelectorAll('.markdown-preview img')].every(i=>i.complete&&i.naturalWidth>0) && document.querySelectorAll('.markdown-image-error').length===2").await?;
            let images = evaluate(main,"({loaded:[...document.querySelectorAll('.markdown-preview img')].map(i=>i.alt),denied:[...document.querySelectorAll('.markdown-image-error')].map(i=>i.textContent),external:[...document.querySelectorAll('.markdown-image-link')].map(i=>i.textContent)})").await?;
            std::fs::write(
                directory.join("preview-markdown-images.json"),
                serde_json::to_vec_pretty(&images).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            let read = wire.tool("lomi_editor_read", json!({"workspaceId":workspace,"panelId":panel,"relativePath":relative,"maxChars":8192})).await?;
            let text = &read["structuredContent"]["data"];
            let status = preview_status(wire, main).await?;
            let edited = wire.tool("lomi_editor_apply_edits",json!({"workspaceId":workspace,"panelId":panel,"relativePath":relative,"documentId":text["documentId"],"expectedBufferRevision":text["bufferRevision"],"expectedDiskRevision":text["diskRevision"],"edits":[{"fromUtf16":0,"toUtf16":0,"insert":"# Unsaved preview\n\n"}],"expectedRevision":status["structuredContent"]["data"]["domainRevision"],"retryEpoch":retry,"requestKey":"preview-edit"})).await?;
            let edited = wire
                .settled(
                    edited["structuredContent"]["data"]["operationId"]
                        .as_str()
                        .ok_or_else(|| edited.to_string())?,
                )
                .await?;
            if edited["structuredContent"]["data"]["state"] != "succeeded" {
                return Err(format!("Preview edit failed: {edited}"));
            }
            wait_for(main,"[...document.querySelectorAll('.markdown-preview h1')].some(e=>e.textContent==='Unsaved preview')").await?;
            let status = preview_status(wire, main).await?;
            let split = wire.tool("lomi_editor_open",json!({"workspaceId":workspace,"relativePath":relative,"presentation":"split","expectedRevision":status["structuredContent"]["data"]["domainRevision"],"retryEpoch":retry,"requestKey":"preview-split"})).await?;
            let split = wire
                .settled(
                    split["structuredContent"]["data"]["operationId"]
                        .as_str()
                        .ok_or_else(|| split.to_string())?,
                )
                .await?;
            let reused = &split["structuredContent"]["data"]["result"];
            if reused["panelId"] != panel
                || reused["documentId"] != text["documentId"]
                || reused["dirty"] != true
                || reused["presentation"] != "split"
            {
                return Err(format!("Preview split replaced dirty document: {split}"));
            }
            wait_for(
                main,
                "Boolean(document.querySelector('.editor-content.is-split .cm-content'))",
            )
            .await?;
            screenshot(main, directory.join("preview-markdown-split.png")).await?;
            javascript(main,"const m=await import('/src/editor-runtime.ts');const d=m.documents().find(d=>d.location.relative==='preview-document.md');d.command('undo');for(let i=0;i<40;i++){if(!d.dirty)return true;await new Promise(r=>setTimeout(r,25));}throw Error('Preview undo did not restore clean buffer');").await?;
        }
        screenshot(main, directory.join(format!("preview-{index}.png"))).await?;
        let status = preview_status(wire, main).await?;
        let close = wire.tool("lomi_panel_close",json!({"workspaceId":workspace,"panelId":panel,"expectedRevision":status["structuredContent"]["data"]["domainRevision"],"retryEpoch":retry,"requestKey":format!("preview-close-{index}")})).await?;
        let closed = wire
            .settled(
                close["structuredContent"]["data"]["operationId"]
                    .as_str()
                    .ok_or_else(|| close.to_string())?,
            )
            .await?;
        if closed["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Preview close failed: {closed}"));
        }
        receipts.push(receipt);
    }
    let raw = javascript(main,"window.__TAURI_INTERNALS__.invoke=window.previewOldInvoke;return {reads:window.previewRawReads,escaped:window.previewEscaped===true};").await?;
    if raw["reads"].as_array().is_none_or(|v| !v.is_empty()) || raw["escaped"] == true {
        return Err(format!(
            "Preview bypassed scoped reads or escaped SVG: {raw}"
        ));
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let requests = std::fs::read_to_string(directory.join("browser-denied-requests.json"))
        .map_err(|e| e.to_string())?;
    if requests.trim() != "0" {
        return Err(format!("Preview loaded external content: {requests}"));
    }
    std::fs::write(
        directory.join("editor-previews.json"),
        serde_json::to_vec_pretty(
            &json!({"receipts":receipts,"rawReads":raw,"externalRequests":0}),
        )
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn qualify_trash(
    wire: &mut Wire,
    main: &Webview,
    directory: &Path,
    workspace: &str,
    connected: &Value,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let name = format!(
        "{}-trash.txt",
        directory.file_name().unwrap().to_string_lossy()
    );
    let file = directory.join("project").join(&name);
    std::fs::write(&file, b"Native Trash fixture\r\n").map_err(|e| e.to_string())?;
    let revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let opened = wire.tool("lomi_editor_open", json!({"workspaceId":workspace,"relativePath":name,
        "expectedRevision":revision["structuredContent"]["data"]["domainRevision"],
        "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"trash-open"})).await?;
    let opened = wire
        .settled(
            opened["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| opened.to_string())?,
        )
        .await?;
    let panel = opened["structuredContent"]["data"]["result"]["panelId"]
        .as_str()
        .ok_or_else(|| opened.to_string())?
        .to_owned();
    let read_args = json!({"workspaceId":workspace,"panelId":panel,"relativePath":name});
    let mut results = Vec::new();
    for (key, choice) in [
        ("cancel", "Cancel"),
        ("mcp-cancel", ""),
        ("save", "Save changes"),
        ("discard", "Discard changes"),
    ] {
        let mut buffer = wire.tool("lomi_editor_read", read_args.clone()).await?;
        if buffer["structuredContent"]["data"]["dirty"] != true {
            let data = &buffer["structuredContent"]["data"];
            let revision = wire.tool("lomi_workspace_list", json!({})).await?;
            let edited = wire.tool("lomi_editor_apply_edits", json!({"workspaceId":workspace,"panelId":panel,"relativePath":name,
                "documentId":data["documentId"],"expectedBufferRevision":data["bufferRevision"],"expectedDiskRevision":data["diskRevision"],
                "edits":[{"fromUtf16":0,"toUtf16":0,"insert":format!("Unsaved {key} Zażółć 🙂\n")}],
                "expectedRevision":revision["structuredContent"]["data"]["domainRevision"],
                "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("trash-edit-{key}")})).await?;
            let edited = wire
                .settled(
                    edited["structuredContent"]["data"]["operationId"]
                        .as_str()
                        .ok_or_else(|| edited.to_string())?,
                )
                .await?;
            if edited["structuredContent"]["data"]["state"] != "succeeded" {
                return Err(format!("Trash edit failed: {edited}"));
            }
            buffer = wire.tool("lomi_editor_read", read_args.clone()).await?;
        }
        let disk = wire
            .tool(
                "lomi_files_read",
                json!({"workspaceId":workspace,"relativePath":name}),
            )
            .await?;
        let parent = wire
            .tool("lomi_files_list", json!({"workspaceId":workspace}))
            .await?;
        let revision = wire.tool("lomi_workspace_list", json!({})).await?;
        let args = json!({"workspaceId":workspace,"operation":{"type":"trash","relativePath":name,"kind":"file",
            "expectedEntryRevision":disk["structuredContent"]["data"]["diskRevision"],
            "expectedParentRevision":parent["structuredContent"]["data"]["directoryRevision"]},
            "expectedRevision":revision["structuredContent"]["data"]["domainRevision"],
            "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("trash-{key}")});
        let pending = wire.tool("lomi_files_mutate", args.clone()).await?;
        let id = pending["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| pending.to_string())?
            .to_owned();
        let pending = wire.settled(&id).await?;
        if pending["structuredContent"]["data"]["state"] != "awaiting_user" || !file.exists() {
            return Err(format!(
                "Trash bypassed the dirty-buffer decision: {pending}"
            ));
        }
        wait_for(main, "!!document.querySelector('.editor-close-dialog')").await?;
        if key == "cancel" {
            screenshot(main, directory.join("files-trash-dialog.png")).await?;
        }
        let bytes_before = std::fs::read(&file).map_err(|e| e.to_string())?;
        let inode = std::fs::metadata(&file).map_err(|e| e.to_string())?.ino();
        if key == "mcp-cancel" {
            wire.tool("lomi_operation_cancel", json!({"operationId":id}))
                .await?;
        } else {
            evaluate(main,&format!("(()=>{{const b=[...document.querySelectorAll('.editor-close-dialog button')].find(b=>b.textContent.trim()==={});if(!b||b.disabled)throw Error('Missing Trash decision');b.click();return true;}})()",serde_json::to_string(choice).unwrap())).await?;
        }
        let mut settled = Value::Null;
        for _ in 0..160 {
            settled = wire
                .tool("lomi_operation_get", json!({"operationId":id}))
                .await?;
            if !matches!(
                settled["structuredContent"]["data"]["state"].as_str(),
                Some("queued" | "running" | "cancelling" | "awaiting_user")
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        wait_for(main, "!document.querySelector('.editor-close-dialog')").await?;
        let expected = if key == "discard" {
            "succeeded"
        } else {
            "cancelled"
        };
        if settled["structuredContent"]["data"]["state"] != expected {
            return Err(format!("Trash {key} did not reach {expected}: {settled}"));
        }
        let after = if key != "discard" {
            let after = wire.tool("lomi_editor_read", read_args.clone()).await?;
            if after["structuredContent"]["data"]["content"]
                != buffer["structuredContent"]["data"]["content"]
                || after["structuredContent"]["data"]["dirty"] != (key != "save")
            {
                return Err(format!("Trash {key} damaged the buffer: {after}"));
            }
            let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
            if (key != "save" && bytes != bytes_before) || (key == "save" && bytes == bytes_before)
            {
                return Err(format!("Trash {key} changed the wrong disk state"));
            }
            after
        } else {
            if file.exists() {
                return Err("Trashed fixture is still at its project path".into());
            }
            let trash = PathBuf::from(std::env::var_os("HOME").ok_or("Missing fixture home")?)
                .join(".Trash")
                .join(&name);
            if std::fs::read(&trash)
                .map_err(|e| format!("Cannot verify native Trash fixture: {e}"))?
                != bytes_before
                || std::fs::metadata(&trash).map_err(|e| e.to_string())?.ino() != inode
            {
                return Err(
                    "System Trash did not preserve the original fixture bytes and inode".into(),
                );
            }
            // Remove only this invocation's UUID-named fixture after exact identity/bytes checks.
            std::fs::remove_file(&trash).map_err(|e| e.to_string())?;
            let panels = wire
                .tool("lomi_panel_list", json!({"workspaceId":workspace}))
                .await?;
            if panels.to_string().contains(&panel) {
                return Err("Trash left its removed editor panel in the layout".into());
            }
            json!({"nativeTrashBytesAndInodePreserved":true,"fixtureRemovedFromTrash":true,"panels":panels})
        };
        let retry = wire.tool("lomi_files_mutate", args).await?;
        if retry["structuredContent"]["data"]["operationId"] != id {
            return Err(format!("Trash {key} replayed the effect"));
        }
        results.push(
            json!({"choice":key,"pending":pending,"settled":settled,"after":after,"retry":retry}),
        );
    }
    std::fs::write(
        directory.join("files-trash.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn activate_main(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app.get_window("main").ok_or("Missing main window")?;
    if let Some(settings) = app.get_window("settings") {
        settings.hide().map_err(|e| e.to_string())?;
    }
    window.unminimize().map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    app.show().map_err(|e| e.to_string())?;
    let target = window.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            use objc2::{class, msg_send, runtime::AnyObject};
            use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior as Behavior};
            let result = target
                .ns_window()
                .map_err(|e| e.to_string())
                .map(|pointer| {
                    let native = unsafe { &*pointer.cast::<AnyObject>() };
                    let native_window = unsafe { &*pointer.cast::<NSWindow>() };
                    let mut behavior = native_window.collectionBehavior();
                    behavior.remove(
                        Behavior::MoveToActiveSpace
                            | Behavior::FullScreenPrimary
                            | Behavior::FullScreenNone
                            | Behavior::Primary
                            | Behavior::Auxiliary,
                    );
                    behavior.insert(
                        Behavior::CanJoinAllSpaces
                            | Behavior::FullScreenAuxiliary
                            | Behavior::CanJoinAllApplications,
                    );
                    native_window.setCollectionBehavior(behavior);
                    unsafe {
                        let _: () = msg_send![native, setLevel: 25isize];
                        let application: *mut AnyObject =
                            msg_send![class!(NSApplication), sharedApplication];
                        let _: () = msg_send![application, activateIgnoringOtherApps: true];
                        let _: () =
                            msg_send![native, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
                        let _: () = msg_send![native, orderFrontRegardless];
                    }
                });
            let _ = send.send(result);
        })
        .map_err(|e| e.to_string())?;
    receive.await.map_err(|e| e.to_string())?
}

async fn clean_android_fixture(
    app: &tauri::AppHandle,
    directory: &Path,
    stage: &str,
) -> Result<(), String> {
    let Some(root) = crate::android::fixture::directory()? else {
        return Ok(());
    };
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(directory.join("android-fixture.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let id = fixture["deviceId"]
        .as_str()
        .ok_or("Missing Android fixture UUID")?;
    let manager = app.state::<crate::android::manager::Android>().get(root)?;
    let valid = manager
        .directory
        .lock()
        .map_err(|_| "Android fixture directory failed")?
        .devices()?
        .devices
        .iter()
        .any(|device| device.id == id && device.name == "MCP qualification");
    if !valid {
        return Err("Android fixture does not identify the isolated qualification device".into());
    }
    let graceful = manager.stop(id, false).await;
    let (status, forced) = match graceful {
        Ok(status) if !status.process_alive => (status, false),
        _ => (manager.stop(id, true).await?, true),
    };
    if status.process_alive {
        return Err("The isolated Android fixture did not exit".into());
    }
    if stage == "after" && std::env::var_os("LOMI_MCP_ANDROID_SETUP_ONLY").is_some() {
        let name = format!(
            "MCP setup {}",
            directory.file_name().unwrap().to_string_lossy()
        );
        let metadata = manager
            .directory
            .lock()
            .map_err(|_| "Fixture metadata unavailable")?
            .devices()?;
        if let Some(device) = metadata.devices.iter().find(|d| d.name == name) {
            manager.stop(&device.id, false).await?;
            let action=serde_json::from_value(json!({"type":"delete","expectedRevision":metadata.revision,"deviceId":device.id,"confirmation":name})).map_err(|e|e.to_string())?;
            manager.installer.manage(&manager, action)?;
            manager.installer.settle(false).await?;
            let result =
                serde_json::to_value(manager.installer.progress()).map_err(|e| e.to_string())?;
            if result["phase"] != "succeeded" {
                return Err(format!("Cannot clean owned management fixture: {result}"));
            }
        }
    }
    let cleanup = manager.clone();
    tauri::async_runtime::spawn_blocking(move || cleanup.stop_private_adb_fixture())
        .await
        .map_err(|e| e.to_string())??;
    std::fs::write(
        directory.join(format!("android-cleanup-{stage}.json")),
        serde_json::to_vec_pretty(
            &json!({"deviceId":id,"processAlive":false,"forced":forced,"privateAdbStopped":true}),
        )
        .unwrap(),
    )
    .map_err(|e| e.to_string())
}

async fn qualify_settings_update(
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    directory: &Path,
    context: Value,
) -> Result<(), String> {
    layout_call(wire,"lomi_panel_focus",json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","retryEpoch":context["retryEpoch"],"requestKey":"settings-update-fixture-focus"})).await?;
    javascript(main,"const m=await import('/src/editor-runtime.ts');window.__settingsUpdateDocument=m.documents().find(d=>d.location.relative==='fixture.txt');return true;").await?;
    wait_for(main, "Boolean(window.__settingsUpdateDocument?.view?.dom.isConnected && window.__settingsUpdateDocument.view.dom.getClientRects().length)").await?;
    let data = main
        .app_handle()
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    let path = data.join("editor-preferences.json");
    if path.exists() {
        return Err("Settings creation fixture must begin with absent preferences".into());
    }
    let other_files = [
        "terminal-preferences.json",
        "keybindings.json",
        "theme-settings.json",
    ];
    let originals: Vec<_> = other_files
        .iter()
        .map(|f| std::fs::read(data.join(f)).ok())
        .collect();
    let before = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    let editor_args = json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt"});
    let editor = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    javascript(main, "const m=await import('/src/editor-runtime.ts');const d=m.documents().find(d=>d.location.relative==='fixture.txt');window.__settingsUpdateDocument=d;d.dispatch({changes:{from:0,to:d.state.doc.length,insert:'Settings update retained draft 🙂\\n'}});return true;").await?;
    let forbidden = javascript(main, "try{await window.__TAURI_INTERNALS__.invoke('agent_control_settings_decide',{operationId:'forged',approve:true});return false;}catch{return true;}").await?;
    if forbidden != true {
        return Err("Main can approve preference changes".into());
    }
    let mut evidence = Vec::new();
    for (index, case) in ["create", "replace", "reject", "cancel", "stale", "recovery"]
        .iter()
        .enumerate()
    {
        let recovery = *case == "recovery";
        let concurrent = br#"{"version":1,"tabSize":12,"insertSpaces":false}"#;
        let corrupt = b"PRIVATE_FIXTURE malformed settings";
        if recovery {
            std::fs::write(&path, corrupt).map_err(|e| e.to_string())?;
            main.app_handle()
                .emit("editor-preferences-changed", ())
                .map_err(|e| e.to_string())?;
        }
        let mut snapshot = Value::Null;
        for _ in 0..80 {
            snapshot = wire
                .tool(
                    "lomi_settings_read",
                    json!({"workspaceId":context["anchor"],"section":"editor"}),
                )
                .await?;
            let s = &snapshot["structuredContent"]["data"];
            let desired = match index {
                0 => 4,
                1..=4 => 8,
                _ => 12,
            };
            if s["readiness"]
                == if recovery {
                    "recovery_required"
                } else {
                    "ready"
                }
                && s["values"]["tabSize"] == desired
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let s = &snapshot["structuredContent"]["data"];
        if s["readiness"]
            != if recovery {
                "recovery_required"
            } else {
                "ready"
            }
        {
            return Err(format!(
                "Preference providers not ready for {case}: {snapshot}"
            ));
        }
        let domain = wire.tool("lomi_workspace_list", json!({})).await?;
        let patch = if *case == "replace" {
            json!({"type":"editor_insert_spaces","value":false})
        } else {
            json!({"type":"editor_tab_size","value":if *case=="create" {8} else {2}})
        };
        let args = json!({"workspaceId":context["anchor"],"patch":patch,"expectedSettingsRevision":s["revision"],"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":context["retryEpoch"],"requestKey":format!("settings-update-{case}")});
        let queued = wire.tool("lomi_settings_update", args.clone()).await?;
        let operation = queued["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| queued.to_string())?;
        let original = std::fs::read(&path).ok();
        if !recovery {
            let selector = format!("[data-settings-operation='{operation}']");
            wait_for(
                settings,
                &format!("!!document.querySelector({})", json!(selector)),
            )
            .await?;
            let pending = javascript(
                settings,
                "return await window.__TAURI_INTERNALS__.invoke('agent_control_state');",
            )
            .await?;
            let request = pending["broker"]["pendingSettingsUpdates"]
                .as_array()
                .and_then(|a| a.iter().find(|p| p["operationId"] == operation))
                .ok_or("Missing native preference plan")?;
            if request["before"]["tabSize"] != s["values"]["tabSize"]
                || request["before"]["insertSpaces"] != s["values"]["insertSpaces"]
            {
                return Err("Native plan does not match effective settings".into());
            }
            if std::fs::read(&path).ok() != original {
                return Err("Preferences changed before approval".into());
            }
            if *case == "create" {
                screenshot(settings, directory.join("settings-update-approval.png")).await?;
            }
            match *case {
                "cancel" => {
                    wire.tool("lomi_operation_cancel", json!({"operationId":operation}))
                        .await?;
                }
                "stale" => {
                    // Simulates another writer after the exact plan is visible.
                    std::fs::write(&path, concurrent).map_err(|e| e.to_string())?;
                    main.app_handle()
                        .emit("editor-preferences-changed", ())
                        .map_err(|e| e.to_string())?;
                    click(settings, "Apply change").await?;
                }
                "reject" => click(settings, "Reject change").await?,
                _ => click(settings, "Apply change").await?,
            }
        }
        let mut settled = Value::Null;
        for _ in 0..120 {
            settled = wire
                .tool("lomi_operation_get", json!({"operationId":operation}))
                .await?;
            if !matches!(
                settled["structuredContent"]["data"]["state"].as_str(),
                Some("queued" | "running" | "awaiting_user" | "cancelling")
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let result = &settled["structuredContent"]["data"];
        let (state, effect) = match *case {
            "create" | "replace" => ("succeeded", "complete"),
            "stale" | "recovery" => ("failed", "none"),
            _ => ("cancelled", "none"),
        };
        if result["state"] != state || result["effectState"] != effect {
            return Err(format!("Settings update {case}: {settled}"));
        }
        if matches!(*case, "create" | "replace") {
            let stored: Value =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if stored["tabSize"] != 8
                || stored["insertSpaces"] != (*case == "create")
                || result["result"]["kind"] != "settings_updated"
            {
                return Err(format!("Wrong preference write {case}: {stored}"));
            }
            if *case == "create" && !result["result"]["previousStoredRevision"].is_null() {
                return Err("Create did not report absent original file".into());
            }
        } else {
            let expected = match *case {
                "stale" => Some(concurrent.to_vec()),
                "recovery" => Some(corrupt.to_vec()),
                _ => original,
            };
            if std::fs::read(&path).ok() != expected {
                return Err(format!("Failed or rejected {case} changed preferences"));
            }
        }
        let retry = wire.tool("lomi_settings_update", args).await?;
        if retry["structuredContent"] != settled["structuredContent"] {
            return Err(format!("Preference retry differs: {case}"));
        }
        evidence.push(json!({"case":case,"snapshot":snapshot,"result":settled,"retry":retry}));
    }
    let dirty = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    if dirty["structuredContent"]["data"]["content"] != "Settings update retained draft 🙂\n"
        || dirty["structuredContent"]["data"]["dirty"] != true
        || dirty["structuredContent"]["data"]["documentId"]
            != editor["structuredContent"]["data"]["documentId"]
    {
        return Err("Preference changes replaced dirty editor state".into());
    }
    let identical = javascript(main,"const m=await import('/src/editor-runtime.ts');return m.documents().find(d=>d.location.relative==='fixture.txt')===window.__settingsUpdateDocument;").await?;
    if identical != true {
        return Err("Preference changes replaced retained document instance".into());
    }
    javascript(
        main,
        "window.__settingsUpdateDocument.command('undo');return true;",
    )
    .await?;
    let undone = wire.tool("lomi_editor_read", editor_args).await?;
    if undone["structuredContent"]["data"]["content"]
        != editor["structuredContent"]["data"]["content"]
    {
        return Err(format!("Preference update destroyed Undo: {undone}"));
    }
    let after = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    if before != after {
        return Err("Preference update changed native terminals".into());
    }
    for (file, original) in other_files.iter().zip(&originals) {
        if std::fs::read(data.join(file)).ok().as_ref() != original.as_ref() {
            return Err(format!("Editor update changed unrelated {file}"));
        }
    }
    std::fs::write(directory.join("settings-update.json"),serde_json::to_vec_pretty(&json!({"cases":evidence,"dirty":dirty,"undo":undone,"retainedDocument":true,"unrelatedFilesUnchanged":true,"terminalsUnchanged":true,"mainCannotApprove":true})).unwrap()).map_err(|e|e.to_string())?;
    // Only the unique private fixture's preferences are restored for normal exit.
    javascript(settings,"await window.__TAURI_INTERNALS__.invoke('save_editor_preferences',{data:{version:1,tabSize:4,insertSpaces:true}});return true;").await?;
    Ok(())
}

async fn qualify_settings_read(
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    directory: &Path,
    context: Value,
) -> Result<(), String> {
    wait_for(main, "!!document.querySelector('.cm-content')").await?;
    let data = main
        .app_handle()
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    let files = [
        "editor-preferences.json",
        "terminal-preferences.json",
        "keybindings.json",
        "theme-settings.json",
    ];
    let originals: Vec<_> = files
        .iter()
        .map(|f| std::fs::read(data.join(f)).ok())
        .collect();
    let before = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    let mut reads = Vec::new();
    for section in ["editor", "terminal", "themes", "keybinds"] {
        let read = wire
            .tool(
                "lomi_settings_read",
                json!({"workspaceId":context["anchor"],"section":section,"limit":2}),
            )
            .await?;
        if read["structuredContent"]["status"] != "ok"
            || read["structuredContent"]["data"]["readiness"] != "ready"
            || read["structuredContent"]["data"]["values"]["section"] != section
        {
            return Err(format!("Settings read {section}: {read}"));
        }
        reads.push(read);
    }
    let keys = &reads[3]["structuredContent"]["data"];
    let second=wire.tool("lomi_settings_read",json!({"workspaceId":context["anchor"],"section":"keybinds","limit":2,"offset":keys["values"]["nextOffset"],"expectedRevision":keys["revision"]})).await?;
    if second["structuredContent"]["data"]["revision"] != keys["revision"]
        || second["structuredContent"]["data"]["values"]["items"][0]["action"]
            == keys["values"]["items"][0]["action"]
    {
        return Err("Shortcut page/revision mismatch".into());
    }
    let forbidden = wire
        .tool(
            "lomi_settings_read",
            json!({"workspaceId":"foreign-workspace","section":"editor"}),
        )
        .await?;
    if forbidden["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err("Settings read accepted foreign workspace".into());
    }
    for (file, original) in files.iter().zip(&originals) {
        if std::fs::read(data.join(file)).ok().as_ref() != original.as_ref() {
            return Err(format!("Settings read changed {file}"));
        }
    }
    let editor=wire.tool("lomi_editor_read",json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt"})).await?;
    javascript(main,"const m=await import('/src/editor-runtime.ts');const d=m.documents().find(d=>d.location.relative==='fixture.txt');d.dispatch({changes:{from:0,to:d.state.doc.length,insert:'Settings read retained draft 🙂\\n'}});return true;").await?;
    javascript(settings,"await window.__TAURI_INTERNALS__.invoke('save_editor_preferences',{data:{version:1,tabSize:8,insertSpaces:false}});return true;").await?;
    let mut changed = Value::Null;
    for _ in 0..60 {
        changed = wire
            .tool(
                "lomi_settings_read",
                json!({"workspaceId":context["anchor"],"section":"editor"}),
            )
            .await?;
        if changed["structuredContent"]["data"]["values"]["tabSize"] == 8 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if changed["structuredContent"]["data"]["values"]["tabSize"] != 8
        || changed["structuredContent"]["data"]["values"]["insertSpaces"] != false
    {
        return Err("MCP did not observe Settings provider changes".into());
    }
    let stale=wire.tool("lomi_settings_read",json!({"workspaceId":context["anchor"],"section":"editor","expectedRevision":reads[0]["structuredContent"]["data"]["revision"]})).await?;
    if stale["structuredContent"]["code"] != "REVISION_CONFLICT" {
        return Err("Settings read accepted old revision".into());
    }
    let corrupt = b"PRIVATE_FIXTURE malformed settings";
    std::fs::write(data.join("editor-preferences.json"), corrupt).map_err(|e| e.to_string())?;
    main.app_handle()
        .emit("editor-preferences-changed", ())
        .map_err(|e| e.to_string())?;
    let mut recovery = Value::Null;
    for _ in 0..60 {
        recovery = wire
            .tool(
                "lomi_settings_read",
                json!({"workspaceId":context["anchor"],"section":"editor"}),
            )
            .await?;
        if recovery["structuredContent"]["data"]["readiness"] == "recovery_required" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if recovery["structuredContent"]["data"]["readiness"] != "recovery_required"
        || recovery["structuredContent"]["data"]["values"]["tabSize"] != 8
        || recovery.to_string().contains("PRIVATE_FIXTURE")
        || recovery.to_string().contains("editor-preferences.json")
        || std::fs::read(data.join("editor-preferences.json")).map_err(|e| e.to_string())?
            != corrupt
    {
        return Err("Settings recovery leaked source or replaced its invalid file".into());
    }
    let dirty=wire.tool("lomi_editor_read",json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt"})).await?;
    if dirty["structuredContent"]["data"]["content"] != "Settings read retained draft 🙂\n"
        || dirty["structuredContent"]["data"]["dirty"] != true
        || dirty["structuredContent"]["data"]["documentId"]
            != editor["structuredContent"]["data"]["documentId"]
    {
        return Err("Settings read changed dirty editor".into());
    }
    let after = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    if before != after {
        return Err("Settings reads changed terminals".into());
    }
    std::fs::write(directory.join("settings-read.json"),serde_json::to_vec_pretty(&json!({"reads":reads,"second":second,"changed":changed,"stale":stale,"recovery":recovery,"dirty":dirty,"before":before,"after":after,"readOnlyFiles":true,"invalidFilePreserved":true})).unwrap()).map_err(|e|e.to_string())?;
    // Restore only this isolated fixture's preference and save its test buffer.
    javascript(settings,"await window.__TAURI_INTERNALS__.invoke('save_editor_preferences',{data:{version:1,tabSize:4,insertSpaces:true}});return true;").await?;
    layout_call(wire,"lomi_editor_save",json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt","documentId":dirty["structuredContent"]["data"]["documentId"],"expectedBufferRevision":dirty["structuredContent"]["data"]["bufferRevision"],"expectedDiskRevision":dirty["structuredContent"]["data"]["diskRevision"],"retryEpoch":context["retryEpoch"],"requestKey":"settings-read-fixture-cleanup"})).await?;
    Ok(())
}

async fn qualify_settings_open(
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    directory: &Path,
    context: Value,
) -> Result<(), String> {
    wait_for(main, "!!document.querySelector('.cm-content')").await?;
    let read = json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt"});
    let initial = wire.tool("lomi_editor_read", read.clone()).await?;
    javascript(main,"const m=await import('/src/editor-runtime.ts');const d=m.documents().find(d=>d.location.relative==='fixture.txt');d.dispatch({changes:{from:0,to:d.state.doc.length,insert:'Settings retained draft 🙂\\n'}});return true;").await?;
    let before = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    let data = main
        .app_handle()
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    let preference_files = [
        "editor-preferences.json",
        "terminal-preferences.json",
        "keybindings.json",
        "theme-settings.json",
    ];
    let original: Vec<_> = preference_files
        .iter()
        .map(|f| std::fs::read(data.join(f)).ok())
        .collect();
    let layout = wire
        .tool("lomi_panel_list", json!({"workspaceId":context["anchor"]}))
        .await?;
    let mut pages = Vec::new();
    for (page, label) in [
        ("keybinds", "Keybinds"),
        ("themes", "Themes"),
        ("plugins", "Plugins"),
        ("terminal", "Terminal"),
        ("chat-ai", "Chat AI"),
        ("android", "Android"),
        ("about", "About"),
        ("agent-control", "Agent control"),
        ("editor", "Editor"),
    ] {
        main.app_handle()
            .get_window("settings")
            .ok_or("Settings window missing")?
            .hide()
            .map_err(|e| e.to_string())?;
        let (args, result) = layout_call(wire,"lomi_settings_open",json!({"workspaceId":context["anchor"],"page":page,"retryEpoch":context["retryEpoch"],"requestKey":format!("settings-{page}")})).await?;
        if result["structuredContent"]["data"]["result"]["page"] != page
            || result["structuredContent"]["data"]["result"]["requested"] != true
        {
            return Err(format!(
                "Settings request missing native completion: {result}"
            ));
        }
        wait_for(settings,&format!("document.querySelector('.settings-nav-item[aria-current=page]')?.textContent.trim()==={}",serde_json::to_string(label).unwrap())).await?;
        if !main
            .app_handle()
            .get_window("settings")
            .ok_or("Settings window missing")?
            .is_visible()
            .map_err(|e| e.to_string())?
        {
            return Err(format!("Settings page {page} remained hidden"));
        }
        let retry = wire.tool("lomi_settings_open", args).await?;
        if retry["structuredContent"] != result["structuredContent"] {
            return Err("Settings retry changed receipt".into());
        }
        pages.push(json!({"page":page,"result":result["structuredContent"]["data"]}));
    }
    screenshot(settings, directory.join("settings-open-editor.png")).await?;
    let dirty = wire.tool("lomi_editor_read", read).await?;
    if dirty["structuredContent"]["data"]["content"] != "Settings retained draft 🙂\n"
        || dirty["structuredContent"]["data"]["dirty"] != true
        || dirty["structuredContent"]["data"]["documentId"]
            != initial["structuredContent"]["data"]["documentId"]
    {
        return Err("Settings opening changed the retained editor buffer".into());
    }
    let after = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    if before != after {
        return Err("Settings opening changed native terminals".into());
    }
    let current = wire
        .tool("lomi_panel_list", json!({"workspaceId":context["anchor"]}))
        .await?;
    if current["structuredContent"]["data"]["items"] != layout["structuredContent"]["data"]["items"]
    {
        return Err("Settings opening changed the layout".into());
    }
    for (file, old) in preference_files.iter().zip(original) {
        if std::fs::read(data.join(file)).ok() != old {
            return Err(format!("Opening changed {file}"));
        }
    }
    std::fs::write(directory.join("settings-open.json"),serde_json::to_vec_pretty(&json!({"pages":pages,"dirty":dirty,"before":before,"after":after,"preferencesUnchanged":true,"layoutUnchanged":true})).unwrap()).map_err(|e|e.to_string())?;
    layout_call(wire,"lomi_editor_save",json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt","documentId":dirty["structuredContent"]["data"]["documentId"],"expectedBufferRevision":dirty["structuredContent"]["data"]["bufferRevision"],"expectedDiskRevision":dirty["structuredContent"]["data"]["diskRevision"],"retryEpoch":context["retryEpoch"],"requestKey":"settings-fixture-cleanup"})).await?;
    Ok(())
}

async fn qualify_project_open(
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    directory: &Path,
    context: Value,
) -> Result<(), String> {
    wait_for(main, "!!document.querySelector('.cm-content')").await?;
    let read = json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt"});
    let initial = wire.tool("lomi_editor_read", read.clone()).await?;
    javascript(main,"const m=await import('/src/editor-runtime.ts');const d=m.documents().find(d=>d.location.relative==='fixture.txt');d.dispatch({changes:{from:0,to:d.state.doc.length,insert:'Project open retained draft 🙂\\n'}});return true;").await?;
    let before = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    let mut cases = Vec::new();
    let mut approved = Vec::new();
    for case in ["reject", "cancel", "replaced", "first", "second"] {
        let folder = directory.join(format!("project-open-{case}"));
        std::fs::create_dir(&folder).map_err(|e| e.to_string())?;
        std::fs::write(folder.join("only-this-project.txt"), case).map_err(|e| e.to_string())?;
        let (anchor, anchor_project) = if case == "second" {
            let first: &Value = &approved[0];
            (
                first["result"]["workspaceId"].clone(),
                first["result"]["projectId"].clone(),
            )
        } else {
            (context["anchor"].clone(), context["projectId"].clone())
        };
        let current = wire.tool("lomi_workspace_list", json!({})).await?;
        let key = if matches!(case, "first" | "second") {
            "open-exact"
        } else {
            case
        };
        let args = json!({"workspaceId":anchor,"projectPath":folder,"name":format!("Opened {case}"),"expectedRevision":current["structuredContent"]["data"]["domainRevision"],"retryEpoch":context["retryEpoch"],"requestKey":key});
        let queued = wire.tool("lomi_project_open", args.clone()).await?;
        let operation = queued["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| format!("Project open admission {case}: {queued}"))?
            .to_string();
        wait_for(
            settings,
            "[...document.querySelectorAll('button')].some(b=>b.textContent==='Approve folder')",
        )
        .await?;
        let pending = javascript(
            settings,
            "return await window.__TAURI_INTERNALS__.invoke('agent_control_state');",
        )
        .await?;
        if pending["broker"]["pendingProjectOpens"]
            .as_array()
            .is_none_or(|items| {
                items.len() != 1
                    || items[0]["operationId"] != operation
                    || items[0]["projectPath"]
                        != folder.canonicalize().unwrap().to_string_lossy().as_ref()
                    || items[0]["workspaceName"] != args["name"]
            })
        {
            return Err(format!("Wrong project approval {case}: {pending}"));
        }
        let proposal = queued["structuredContent"]["data"]["result"].clone();
        let denied = wire
            .tool(
                "lomi_files_list",
                json!({"workspaceId":proposal["workspaceId"]}),
            )
            .await?;
        if denied["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
            return Err("Pending folder request granted reads".into());
        }
        if case == "reject" {
            evaluate(settings,"[...document.querySelectorAll('button')].find(b=>b.textContent==='Approve folder').closest('article').scrollIntoView({block:'center'});true").await?;
            javascript(settings,"await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));return true;").await?;
            screenshot(settings, directory.join("project-open-approval.png")).await?;
            click(settings, "Reject folder").await?;
        } else if case == "cancel" {
            wire.tool("lomi_operation_cancel", json!({"operationId":operation}))
                .await?;
        } else if case == "replaced" {
            std::fs::rename(&folder, directory.join("project-open-original-replaced"))
                .map_err(|e| e.to_string())?;
            std::fs::create_dir(&folder).map_err(|e| e.to_string())?;
        } else {
            click(settings, "Approve folder").await?;
        }
        let mut settled = Value::Null;
        for _ in 0..160 {
            settled = wire
                .tool("lomi_operation_get", json!({"operationId":operation}))
                .await?;
            if !matches!(
                settled["structuredContent"]["data"]["state"].as_str(),
                Some("queued" | "awaiting_user" | "running" | "cancelling")
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let data = &settled["structuredContent"]["data"];
        if matches!(case, "first" | "second") {
            if data["state"] != "succeeded"
                || data["result"]["kind"] != "project_opened"
                || data["result"]["opened"] != true
            {
                return Err(format!("Project open {case}: {settled}"));
            }
            let files = wire
                .tool(
                    "lomi_files_list",
                    json!({"workspaceId":data["result"]["workspaceId"]}),
                )
                .await?;
            if files["structuredContent"]["data"]["entries"]
                .as_array()
                .is_none_or(|v| v.len() != 1 || v[0]["relativePath"] != "only-this-project.txt")
            {
                return Err(format!("Opened project root mismatch: {files}"));
            }
            let lookup=wire.tool("lomi_operation_get",json!({"projectId":anchor_project,"retryEpoch":context["retryEpoch"],"tool":"lomi_project_open","requestKey":key})).await?;
            if lookup["structuredContent"]["data"]["operationId"] != operation {
                return Err("Project-bound open receipt mismatch".into());
            }
            approved.push(data.clone());
        } else if data["effectState"] != "none"
            || !matches!(data["state"].as_str(), Some("cancelled" | "failed"))
        {
            return Err(format!(
                "Rejected project request changed resources: {case}: {settled}"
            ));
        }
        let retry = wire.tool("lomi_project_open", args.clone()).await?;
        if retry["structuredContent"]["data"]["operationId"] != operation {
            return Err(format!("Project open replay {case}: {retry}"));
        }
        cases.push(json!({"case":case,"anchorProjectId":anchor_project,"args":args,"settled":settled,"retry":retry}));
        wait_for(
            settings,
            "![...document.querySelectorAll('button')].some(b=>b.textContent==='Approve folder')",
        )
        .await?;
    }
    if approved[0]["operationId"] == approved[1]["operationId"] {
        return Err("Two projects shared an open receipt".into());
    }
    let ambiguous=wire.tool("lomi_operation_get",json!({"retryEpoch":context["retryEpoch"],"tool":"lomi_project_open","requestKey":"open-exact"})).await?;
    if ambiguous["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err("Ambiguous multi-project lookup selected a project".into());
    }
    let dirty = wire.tool("lomi_editor_read", read).await?;
    if dirty["structuredContent"]["data"]["content"] != "Project open retained draft 🙂\n"
        || dirty["structuredContent"]["data"]["dirty"] != true
        || dirty["structuredContent"]["data"]["documentId"]
            != initial["structuredContent"]["data"]["documentId"]
    {
        return Err(format!(
            "Opening project replaced original dirty document: {dirty}"
        ));
    }
    let after = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    if before != after {
        return Err("Opening a project started or changed a terminal".into());
    }
    let panels = wire
        .tool(
            "lomi_panel_list",
            json!({"workspaceId":approved[1]["result"]["workspaceId"]}),
        )
        .await?;
    if panels["structuredContent"]["data"]["items"]
        .as_array()
        .is_none_or(|p| p.len() != 1 || p[0]["kind"] != "file")
    {
        return Err(format!("Project did not open a neutral editor: {panels}"));
    }
    screenshot(main, directory.join("project-opened.png")).await?;
    std::fs::write(directory.join("project-open.json"),serde_json::to_vec_pretty(&json!({"cases":cases,"approved":approved,"dirty":dirty,"before":before,"after":after,"panels":panels,"ambiguous":ambiguous})).unwrap()).map_err(|e|e.to_string())?;
    // Only after the retained-draft assertions, save this fixture-owned file so
    // the real application close guard can finish without process termination.
    let saved=layout_call(wire,"lomi_editor_save",json!({"workspaceId":context["anchor"],"panelId":"mcp-control-fixture","relativePath":"fixture.txt","documentId":dirty["structuredContent"]["data"]["documentId"],"expectedBufferRevision":dirty["structuredContent"]["data"]["bufferRevision"],"expectedDiskRevision":dirty["structuredContent"]["data"]["diskRevision"],"retryEpoch":context["retryEpoch"],"requestKey":"project-open-fixture-cleanup"})).await?;
    if std::fs::read_to_string(directory.join("project/fixture.txt")).map_err(|e| e.to_string())?
        != "Project open retained draft 🙂\n"
    {
        return Err("Project-open fixture cleanup saved different bytes".into());
    }
    std::fs::write(
        directory.join("project-open-cleanup.json"),
        serde_json::to_vec_pretty(&saved.1).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn qualify_project_close(
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    directory: &Path,
    context: Value,
) -> Result<(), String> {
    wait_for(main, "!!document.querySelector('.cm-content')").await?;
    let mut terminals = Vec::new();
    for (index, workspace) in [context["anchor"].clone(), json!("foreign-workspace")]
        .into_iter()
        .enumerate()
    {
        let (_, terminal) = layout_call(wire, "lomi_terminal_create", json!({"workspaceId":workspace,"cwdRelative":".","title":"Project close idle PTY","retryEpoch":context["retryEpoch"],"requestKey":format!("project-idle-{index}")})).await?;
        let target = &terminal["structuredContent"]["data"]["result"];
        let read = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"]});
        let mut ready = false;
        for _ in 0..100 {
            if wire.tool("lomi_terminal_read", read.clone()).await?["structuredContent"]["data"]
                ["prompt"]
                == "ready"
            {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !ready {
            return Err("Project close terminal did not reach an idle prompt".into());
        }
        terminals.push(terminal);
    }
    let (_, browser) = layout_call(wire, "lomi_browser_open", json!({"workspaceId":"foreign-workspace","url":context["origin"],"visible":false,"retryEpoch":context["retryEpoch"],"requestKey":"project-close-browser"})).await?;
    let before = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    qualify_workspace_close(wire, main, directory, context.clone()).await?;
    let after = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    let mut expected = before.clone();
    for terminal in &terminals {
        let generation = terminal["structuredContent"]["data"]["result"]["terminalSessionId"]
            .as_str()
            .ok_or("Missing project PTY generation")?;
        if expected
            .as_object_mut()
            .ok_or("No terminal contexts")?
            .remove(generation)
            .is_none()
        {
            return Err("Project-close proof lacked its native PTY".into());
        }
    }
    if after != expected {
        return Err("Project close stopped or created an unrelated terminal".into());
    }
    let remaining = javascript(settings, "return (await window.__TAURI_INTERNALS__.invoke('agent_control_state')).broker.workspaces;").await?;
    if !remaining.as_array().is_some_and(|workspaces| {
        workspaces.len() == 1
            && workspaces[0]["name"] == "Retained other project"
            && workspaces[0]["projectId"] != context["projectId"]
    }) {
        return Err(format!(
            "Project close changed an unrelated project: {remaining}"
        ));
    }
    wait_for(main, "document.body.textContent.includes('Welcome to Lomi') && !document.querySelector('.terminal-layout')").await?;
    screenshot(main, directory.join("project-closed.png")).await?;
    std::fs::write(directory.join("project-close-native.json"), serde_json::to_vec_pretty(&json!({"before":before,"after":after,"terminals":terminals,"browser":browser,"remaining":remaining})).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

async fn qualify_workspace_close(
    wire: &mut Wire,
    main: &Webview,
    directory: &Path,
    context: Value,
) -> Result<(), String> {
    let retry = &context["retryEpoch"];
    let project_close = context["projectClose"] == true;
    let tool = if project_close {
        "lomi_project_close"
    } else {
        "lomi_workspace_update"
    };
    let current = wire.tool("lomi_workspace_list", json!({})).await?;
    let protected = if project_close {
        Value::Null
    } else {
        wire.tool("lomi_workspace_update",json!({"action":"close","workspaceId":context["protectedWorkspace"],"expectedRevision":current["structuredContent"]["data"]["domainRevision"],"retryEpoch":retry,"requestKey":"close-protected-workspace"})).await?
    };
    if !project_close
        && !matches!(
            protected["structuredContent"]["code"].as_str(),
            Some("CONTROL_REVOKED" | "PROTECTED_ORIGIN_TERMINAL" | "TARGET_BUSY")
        )
    {
        return Err(format!(
            "Ancestor close did not preserve busy/human resources: {protected}"
        ));
    }
    let (_,created)=layout_call(wire,"lomi_workspace_create",json!({"workspaceId":context["anchor"],"name":"MCP guarded closure","retryEpoch":retry,"requestKey":"close-workspace-create"})).await?;
    let workspace = &created["structuredContent"]["data"]["result"]["workspaceId"];
    let relative = "mcp-workspace-close.txt";
    std::fs::write(
        directory.join("project").join(relative),
        "Close disk bytes 🙂\n",
    )
    .map_err(|e| e.to_string())?;
    let (_,opened)=layout_call(wire,"lomi_editor_open",json!({"workspaceId":workspace,"relativePath":relative,"retryEpoch":retry,"requestKey":"close-editor-open"})).await?;
    let panel = &opened["structuredContent"]["data"]["result"]["panelId"];
    let read_args = json!({"workspaceId":workspace,"panelId":panel,"relativePath":relative});
    let mut outcomes = Vec::new();
    for (case, choice) in [
        ("cancel", "Cancel"),
        ("mcp-cancel", ""),
        ("save", "Save changes"),
        ("discard", "Discard changes"),
    ] {
        let read = wire.tool("lomi_editor_read", read_args.clone()).await?;
        let text = &read["structuredContent"]["data"];
        if text["dirty"] != true {
            layout_call(wire,"lomi_editor_apply_edits",json!({"workspaceId":workspace,"panelId":panel,"relativePath":relative,"documentId":text["documentId"],"expectedBufferRevision":text["bufferRevision"],"expectedDiskRevision":text["diskRevision"],"edits":[{"fromUtf16":0,"toUtf16":0,"insert":format!("Unsaved {case} 🙂\n")}],"retryEpoch":retry,"requestKey":format!("close-draft-{case}")})).await?;
        }
        let dirty = wire.tool("lomi_editor_read", read_args.clone()).await?;
        let listed = wire.tool("lomi_workspace_list", json!({})).await?;
        let mut args = json!({"action":"close","workspaceId":workspace,"expectedRevision":listed["structuredContent"]["data"]["domainRevision"],"retryEpoch":retry,"requestKey":format!("close-workspace-{case}")});
        if project_close {
            args.as_object_mut().unwrap().remove("action");
            args["projectId"] = context["projectId"].clone();
        }
        let requested = wire.tool(tool, args.clone()).await?;
        let operation = requested["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| requested.to_string())?;
        wait_for(main,"[...document.querySelectorAll('dialog[open]')].some(d=>d.textContent.includes('Save changes before closing?'))").await?;
        if case == "cancel" {
            screenshot(
                main,
                directory.join(if project_close {
                    "project-close-guard.png"
                } else {
                    "workspace-close-guard.png"
                }),
            )
            .await?;
        }
        if choice.is_empty() {
            wire.tool("lomi_operation_cancel", json!({"operationId":operation}))
                .await?;
        } else {
            click(main, choice).await?;
        }
        let result = wire.settled(operation).await?;
        let data = &result["structuredContent"]["data"];
        let expected = if case == "save" {
            "partial"
        } else if case == "discard" {
            "complete"
        } else {
            "none"
        };
        if data["effectState"] != expected
            || (case == "discard"
                && (data["state"] != "succeeded" || data["result"]["closed"] != true))
        {
            return Err(format!("Workspace {case} receipt: {result}"));
        }
        let repeated = wire.tool(tool, args).await?;
        if repeated["structuredContent"]["data"]["operationId"] != operation {
            return Err(format!("Workspace {case} replayed: {repeated}"));
        }
        if case == "cancel" || case == "mcp-cancel" {
            let after = wire.tool("lomi_editor_read", read_args.clone()).await?;
            if after["structuredContent"]["data"] != dirty["structuredContent"]["data"] {
                return Err("Cancelled ancestor close changed the buffer".into());
            }
        }
        if case == "save" {
            let saved = wire.tool("lomi_editor_read", read_args.clone()).await?;
            if saved["structuredContent"]["data"]["dirty"] != false
                || data["result"]["closed"] != false
            {
                return Err("Save did not retain the workspace".into());
            }
            let disk = std::fs::read_to_string(directory.join("project").join(relative))
                .map_err(|e| e.to_string())?;
            if json!(disk) != dirty["structuredContent"]["data"]["content"] {
                return Err("Workspace guard did not save exact bytes".into());
            }
        }
        outcomes.push(result);
    }
    let closed = wire.tool("lomi_editor_read", read_args).await?;
    if closed["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err(format!(
            "Closed workspace still disclosed buffers: {closed}"
        ));
    }
    let listed = wire.tool("lomi_workspace_list", json!({})).await?;
    if !listed["structuredContent"]["data"]["items"]
        .as_array()
        .is_some_and(|items| {
            if project_close {
                return items.is_empty();
            }
            !items.iter().any(|w| w["id"] == *workspace)
                && items.iter().any(|w| w["id"] == context["anchor"])
                && items
                    .iter()
                    .any(|w| w["id"] == context["protectedWorkspace"])
        })
    {
        return Err("Ancestor close removed a different workspace".into());
    }
    let disk = std::fs::read_to_string(directory.join("project").join(relative))
        .map_err(|e| e.to_string())?;
    if disk != "Unsaved cancel 🙂\nClose disk bytes 🙂\n" {
        return Err(format!("Discard changed saved disk bytes: {disk:?}"));
    }
    std::fs::write(directory.join(if project_close { "project-close.json" } else { "workspace-close.json" }),serde_json::to_vec_pretty(&json!({"guarded":outcomes,"protected":protected,"closedRead":closed,"remaining":listed,"disk":disk})).unwrap()).map_err(|e|e.to_string())?;
    if project_close {
        Ok(())
    } else {
        qualify_idle_workspace_close(wire, main, directory, &context).await
    }
}

async fn qualify_idle_workspace_close(
    wire: &mut Wire,
    main: &Webview,
    directory: &Path,
    context: &Value,
) -> Result<(), String> {
    let retry = &context["retryEpoch"];
    for round in 0..6 {
        // A separate idle terminal descendant exercises the native stop commit.
        let (_,created)=layout_call(wire,"lomi_workspace_create",json!({"workspaceId":context["anchor"],"name":"MCP idle closure","retryEpoch":retry,"requestKey":format!("close-idle-workspace-{round}")})).await?;
        let workspace = &created["structuredContent"]["data"]["result"]["workspaceId"];
        let (_,terminal)=layout_call(wire,"lomi_terminal_create",json!({"workspaceId":workspace,"cwdRelative":".","title":"MCP closing idle PTY","retryEpoch":retry,"requestKey":format!("close-idle-terminal-{round}")})).await?;
        let target = &terminal["structuredContent"]["data"]["result"];
        let read = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"]});
        let mut ready = false;
        for _ in 0..100 {
            if wire.tool("lomi_terminal_read", read.clone()).await?["structuredContent"]["data"]
                ["prompt"]
                == "ready"
            {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !ready {
            return Err("Closure PTY did not reach its idle prompt".into());
        }
        let (_,browser)=layout_call(wire,"lomi_browser_open",json!({"workspaceId":workspace,"url":context["origin"],"visible":false,"retryEpoch":retry,"requestKey":format!("close-owned-browser-{round}")})).await?;
        let before = javascript(
            main,
            "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
        )
        .await?;
        let (args, closed)=layout_call(wire,"lomi_workspace_update",json!({"action":"close","workspaceId":workspace,"retryEpoch":retry,"requestKey":format!("close-idle-runtime-{round}")})).await?;
        let retry_result = wire.tool("lomi_workspace_update", args).await?;
        if retry_result["structuredContent"]["data"]["operationId"]
            != closed["structuredContent"]["data"]["operationId"]
        {
            return Err("Idle workspace close replayed".into());
        }
        let after = javascript(
            main,
            "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
        )
        .await?;
        let mut expected = before.clone();
        expected
            .as_object_mut()
            .ok_or("No native terminal contexts")?
            .remove(target["terminalSessionId"].as_str().unwrap());
        if after != expected {
            return Err("Ancestor close replaced or stopped another terminal".into());
        }
        std::fs::write(directory.join(format!("workspace-close-runtime-{round}.json")),serde_json::to_vec_pretty(&json!({"closed":closed,"terminal":terminal,"browser":browser,"before":before,"after":after})).unwrap()).map_err(|e|e.to_string())?;
    }
    Ok(())
}

async fn layout_call(
    wire: &mut Wire,
    tool: &str,
    mut args: Value,
) -> Result<(Value, Value), String> {
    let key = args["requestKey"]
        .as_str()
        .ok_or("Missing layout key")?
        .to_owned();
    for attempt in 0..3 {
        let current = wire.tool("lomi_workspace_list", json!({})).await?;
        args["expectedRevision"] = current["structuredContent"]["data"]["domainRevision"].clone();
        args["requestKey"] = json!(format!("{key}-{attempt}"));
        let reply = wire.tool(tool, args.clone()).await?;
        let operation = reply["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| reply.to_string())?;
        let result = wire
            .settled_with_limit(
                operation,
                if std::env::var_os("LOMI_MCP_ANDROID_LAYOUT_ONLY").is_some() {
                    7200
                } else {
                    900
                },
            )
            .await?;
        let data = &result["structuredContent"]["data"];
        if data["state"] == "succeeded" {
            return Ok((args, result));
        }
        if data["effectState"] != "none" || data["result"]["code"] != "REVISION_CONFLICT" {
            return Err(format!("Layout fixture {tool}: {result}"));
        }
    }
    Err("Layout did not stabilize within three no-effect revision attempts".into())
}
async fn qualify_panel_moves(
    wire: &mut Wire,
    main: &Webview,
    browser: &Webview,
    directory: &Path,
    context: Value,
) -> Result<(), String> {
    let workspace = &context["workspaceId"];
    let retry = &context["retryEpoch"];
    let server = &context["server"];
    let browser_target = &context["browser"];
    let relative = "mcp-layout-buffer.txt";
    std::fs::write(
        directory.join("project").join(relative),
        "Layout disk bytes 🙂\n",
    )
    .map_err(|e| e.to_string())?;
    let retained = evaluate(browser, "document.querySelector('#name').value").await?;
    let (_, opened) = layout_call(wire, "lomi_editor_open", json!({"workspaceId":workspace,"relativePath":relative,"retryEpoch":retry,"requestKey":"layout-file-open"})).await?;
    let file = &opened["structuredContent"]["data"]["result"]["panelId"];
    let read_args = json!({"workspaceId":workspace,"panelId":file,"relativePath":relative});
    let read = wire.tool("lomi_editor_read", read_args.clone()).await?;
    let text = &read["structuredContent"]["data"];
    layout_call(wire, "lomi_editor_apply_edits", json!({"workspaceId":workspace,"panelId":file,"relativePath":relative,"documentId":text["documentId"],"expectedBufferRevision":text["bufferRevision"],"expectedDiskRevision":text["diskRevision"],"edits":[{"fromUtf16":0,"toUtf16":0,"insert":"Unsaved layout draft 🙂\n"}],"retryEpoch":retry,"requestKey":"layout-draft"})).await?;
    let dirty = wire.tool("lomi_editor_read", read_args.clone()).await?;
    layout_call(wire,"lomi_panel_focus",json!({"workspaceId":workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"retryEpoch":retry,"requestKey":"layout-focus-browser"})).await?;
    let panels = wire
        .tool("lomi_panel_list", json!({"workspaceId":workspace}))
        .await?;
    let items = panels["structuredContent"]["data"]["items"]
        .as_array()
        .ok_or("Missing layout panels")?;
    let server_tab = items
        .iter()
        .find(|p| p["id"] == server["panelId"])
        .ok_or("Missing server tab")?["tabId"]
        .clone();
    let browser_tab = items
        .iter()
        .find(|p| p["id"] == browser_target["panelId"])
        .ok_or("Missing browser tab")?["tabId"]
        .clone();
    let file_tab = items
        .iter()
        .find(|p| p["id"] == *file)
        .ok_or("Missing file tab")?["tabId"]
        .clone();
    let base = json!({"workspaceId":workspace,"retryEpoch":retry});
    let mut hidden = base.clone();
    hidden["expectedRevision"] = panels["structuredContent"]["data"]["domainRevision"].clone();
    hidden["requestKey"] = json!("layout-hidden");
    hidden["movement"] =
        json!({"type":"dock_tab","tabId":file_tab,"targetTabId":server_tab,"side":"right"});
    let denied = wire.tool("lomi_panel_move", hidden).await?;
    if denied["structuredContent"]["code"] != "PANEL_NOT_RENDERABLE" {
        return Err(format!("Hidden docking accepted: {denied}"));
    }
    let mut reorder = base.clone();
    reorder["requestKey"] = json!("layout-reorder");
    reorder["movement"] = json!({"type":"reorder_tab","tabId":file_tab,"beforeTabId":server_tab});
    let (args, reordered) = layout_call(wire, "lomi_panel_move", reorder).await?;
    let repeated = wire.tool("lomi_panel_move", args).await?;
    if repeated["structuredContent"]["data"]["operationId"]
        != reordered["structuredContent"]["data"]["operationId"]
    {
        return Err("Layout retry created another operation".into());
    }
    javascript(main,"const dialog=document.createElement('dialog');dialog.id='layout-fixture-modal';dialog.textContent='Layout guard qualification';document.body.append(dialog);dialog.showModal();return true;").await?;
    let current = wire.tool("lomi_workspace_list", json!({})).await?;
    let blocked=wire.tool("lomi_panel_move",json!({"workspaceId":workspace,"retryEpoch":retry,"requestKey":"layout-modal","expectedRevision":current["structuredContent"]["data"]["domainRevision"],"movement":{"type":"reorder_tab","tabId":file_tab,"beforeTabId":null}})).await?;
    let blocked = wire
        .settled(
            blocked["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| blocked.to_string())?,
        )
        .await?;
    javascript(
        main,
        "document.querySelector('#layout-fixture-modal').remove();return true;",
    )
    .await?;
    if blocked["structuredContent"]["data"]["result"]["code"] != "TARGET_BUSY"
        || blocked["structuredContent"]["data"]["effectState"] != "none"
    {
        return Err(format!("Modal allowed layout mutation: {blocked}"));
    }
    layout_call(wire,"lomi_panel_focus",json!({"workspaceId":workspace,"panelId":server["panelId"],"terminalSessionId":server["terminalSessionId"],"retryEpoch":retry,"requestKey":"layout-focus-server"})).await?;
    wait_for(
        main,
        "document.querySelector('.terminal-layout')?.clientWidth > 480",
    )
    .await?;
    let mut results = vec![reordered];
    for (name, movement) in [
        (
            "browser",
            json!({"type":"dock_tab","tabId":browser_tab,"targetTabId":server_tab,"side":"right"}),
        ),
        (
            "editor",
            json!({"type":"dock_tab","tabId":file_tab,"targetTabId":server_tab,"side":"bottom"}),
        ),
        (
            "pane",
            json!({"type":"move_pane","panelId":file,"targetPanelId":server["panelId"],"side":"top"}),
        ),
    ] {
        let mut request = base.clone();
        request["requestKey"] = json!(format!("layout-{name}"));
        request["movement"] = movement;
        let (args, result) = layout_call(wire, "lomi_panel_move", request).await?;
        let repeated = wire.tool("lomi_panel_move", args).await?;
        if repeated["structuredContent"]["data"]["operationId"]
            != result["structuredContent"]["data"]["operationId"]
        {
            return Err("Docking repeated".into());
        }
        results.push(result);
    }
    let after = wire.tool("lomi_editor_read", read_args.clone()).await?;
    if after["structuredContent"]["data"] != dirty["structuredContent"]["data"]
        || after["structuredContent"]["data"]["dirty"] != true
    {
        return Err(format!(
            "Docking changed the shared dirty document: {after}"
        ));
    }
    let (_, destination) = layout_call(wire,"lomi_workspace_create",json!({"workspaceId":workspace,"name":"MCP transfer destination","retryEpoch":retry,"requestKey":"layout-transfer-workspace"})).await?;
    let destination_id = &destination["structuredContent"]["data"]["result"]["workspaceId"];
    let current = wire.tool("lomi_workspace_list", json!({})).await?;
    let denied_transfer = wire.tool("lomi_panel_move",json!({"workspaceId":workspace,"retryEpoch":retry,"requestKey":"layout-foreign-transfer","expectedRevision":current["structuredContent"]["data"]["domainRevision"],"movement":{"type":"transfer_tab","tabId":server_tab,"targetWorkspaceId":"foreign-workspace","beforeTabId":null}})).await?;
    if denied_transfer["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err(format!(
            "Foreign workspace transfer accepted: {denied_transfer}"
        ));
    }
    let (args, transferred) = layout_call(wire,"lomi_panel_move",json!({"workspaceId":workspace,"retryEpoch":retry,"requestKey":"layout-transfer","movement":{"type":"transfer_tab","tabId":server_tab,"targetWorkspaceId":destination_id,"beforeTabId":null}})).await?;
    if wire.tool("lomi_panel_move", args).await?["structuredContent"]["data"]["operationId"]
        != transferred["structuredContent"]["data"]["operationId"]
    {
        return Err("Transfer retry replayed".into());
    }
    let mut destination_read = read_args.clone();
    destination_read["workspaceId"] = destination_id.clone();
    let transferred_buffer = wire.tool("lomi_editor_read", destination_read).await?;
    for field in ["documentId", "bufferRevision", "content", "dirty"] {
        if transferred_buffer["structuredContent"]["data"][field]
            != dirty["structuredContent"]["data"][field]
        {
            return Err(format!(
                "Transfer changed buffer {field}: {transferred_buffer}"
            ));
        }
    }
    let stale_source = wire.tool("lomi_editor_read", read_args.clone()).await?;
    if stale_source["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err(format!(
            "Old workspace retained moved buffer authority: {stale_source}"
        ));
    }
    let transferred_terminal = wire.tool("lomi_terminal_read",json!({"workspaceId":destination_id,"panelId":server["panelId"],"terminalSessionId":server["terminalSessionId"],"operationId":context["serverOperation"],"maxBytes":8192})).await?;
    if transferred_terminal["structuredContent"]["data"]["command"]["operationId"]
        != context["serverOperation"]
        || transferred_terminal["structuredContent"]["data"]["terminalSessionId"]
            != server["terminalSessionId"]
    {
        return Err(format!("Transfer lost live PTY: {transferred_terminal}"));
    }
    let replayed_run = wire
        .tool("lomi_terminal_run", context["serverRun"].clone())
        .await?;
    if replayed_run["structuredContent"]["data"]["operationId"] != context["serverOperation"] {
        return Err(format!(
            "Transferred run did not return its original receipt: {replayed_run}"
        ));
    }
    let stale_terminal = wire.tool("lomi_terminal_read",json!({"workspaceId":workspace,"panelId":server["panelId"],"terminalSessionId":server["terminalSessionId"]})).await?;
    if stale_terminal["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err(format!(
            "Old workspace retained moved terminal authority: {stale_terminal}"
        ));
    }
    layout_call(wire,"lomi_panel_focus",json!({"workspaceId":destination_id,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"retryEpoch":retry,"requestKey":"layout-transfer-focus"})).await?;
    if evaluate(browser, "document.querySelector('#name').value").await? != retained {
        return Err("Workspace transfer recreated the native browser".into());
    }
    screenshot(main, directory.join("workspace-transfer.png")).await?;
    screenshot(browser, directory.join("workspace-transfer-browser.png")).await?;
    let (_, returned) = layout_call(wire,"lomi_panel_move",json!({"workspaceId":destination_id,"retryEpoch":retry,"requestKey":"layout-transfer-back","movement":{"type":"transfer_tab","tabId":server_tab,"targetWorkspaceId":workspace,"beforeTabId":null}})).await?;
    wait_for(main, "!document.querySelector('.terminal-layout') && document.body.textContent.includes('Welcome to Lomi')").await?;
    layout_call(wire,"lomi_panel_focus",json!({"workspaceId":workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"retryEpoch":retry,"requestKey":"layout-transfer-return-focus"})).await?;
    std::fs::write(directory.join("workspace-transfer.json"),serde_json::to_vec_pretty(&json!({"transfer":transferred,"return":returned,"dirty":transferred_buffer,"terminal":transferred_terminal,"replayedRun":replayed_run,"foreign":denied_transfer,"oldSource":stale_source,"oldTerminal":stale_terminal})).unwrap()).map_err(|e|e.to_string())?;
    javascript(main,"const m=await import('/src/editor-runtime.ts');m.documents().find(d=>d.location.relative==='mcp-layout-buffer.txt').command('undo');return true;").await?;
    let undone = wire.tool("lomi_editor_read", read_args).await?;
    if undone["structuredContent"]["data"]["content"] != text["content"]
        || undone["structuredContent"]["data"]["documentId"] != text["documentId"]
        || undone["structuredContent"]["data"]["dirty"] != false
    {
        return Err("Docking lost editor undo".into());
    }
    let terminal=wire.tool("lomi_terminal_read",json!({"workspaceId":workspace,"panelId":server["panelId"],"terminalSessionId":server["terminalSessionId"],"operationId":context["serverOperation"],"maxBytes":8192})).await?;
    if terminal["structuredContent"]["data"]["terminalSessionId"] != server["terminalSessionId"]
        || terminal["structuredContent"]["data"]["command"]["operationId"]
            != context["serverOperation"]
    {
        return Err(format!("Docking lost running PTY: {terminal}"));
    }
    layout_call(wire,"lomi_panel_focus",json!({"workspaceId":workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"retryEpoch":retry,"requestKey":"layout-return-browser"})).await?;
    if evaluate(browser, "document.querySelector('#name').value").await? != retained {
        return Err("Docking reloaded the native browser".into());
    }
    screenshot(main, directory.join("mixed-layout.png")).await?;
    screenshot(browser, directory.join("mixed-layout-browser.png")).await?;
    std::fs::write(directory.join("panel-moves.json"),serde_json::to_vec_pretty(&json!({"moves":results,"modal":blocked,"hidden":denied,"dirty":after,"undo":undone,"terminal":terminal})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}

async fn native_browser_pointer(view: &Webview) -> Result<(), String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView};
        let result = (|| {
            let main = MainThreadMarker::new().ok_or("Not on native event thread")?;
            let native = unsafe { &*platform.inner().cast::<NSView>() };
            let window = native.window().ok_or("Missing browser window")?;
            let mut point = native.bounds().origin;
            point.x += 24.; point.y += 24.;
            let point = native.convertPoint_toView(point, None);
            let app = NSApplication::sharedApplication(main);
            for (number, kind) in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp].into_iter().enumerate() {
                let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(kind, point, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, number as isize, 1, 1.).ok_or("Cannot create native fixture input")?;
                app.postEvent_atStart(&event, false);
            }
            Ok::<_, String>(())
        })();
        let _ = send.send(result);
    }).map_err(|e|e.to_string())?;
    receive.await.map_err(|e| e.to_string())?
}

async fn native_browser_hidden(view: &Webview) -> Result<bool, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let native = unsafe { &*platform.inner().cast::<objc2_app_kit::NSView>() };
        let _ = send.send(native.isHiddenOrHasHiddenAncestor());
    })
    .map_err(|e| e.to_string())?;
    receive.await.map_err(|e| e.to_string())
}

async fn browser_monitor_count(app: &tauri::AppHandle) -> Result<usize, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = send.send(crate::browser::native_input::probe_count());
    })
    .map_err(|e| e.to_string())?;
    receive.await.map_err(|e| e.to_string())
}

async fn javascript(view: &Webview, script: &str) -> Result<Value, String> {
    evaluate(view,&format!("window.controlProbeResult=null; Promise.resolve().then(async()=>{{ {script} }}).then(value=>window.controlProbeResult={{value}},error=>window.controlProbeResult={{error:String(error)}});true")).await?;
    wait_for(view, "window.controlProbeResult !== null").await?;
    let result = evaluate(view, "window.controlProbeResult").await?;
    if let Some(error) = result.get("error") {
        return Err(error.to_string());
    }
    Ok(result["value"].clone())
}
async fn click(view: &Webview, label: &str) -> Result<(), String> {
    let expression = format!(
        "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==={})",
        json!(label)
    );
    wait_for(
        view,
        &format!("Boolean(({expression}) && !({expression}).disabled)"),
    )
    .await?;
    evaluate(view, &format!("({expression}).click();true")).await?;
    Ok(())
}
struct Wire {
    input: tokio::process::ChildStdin,
    output: BufReader<tokio::process::ChildStdout>,
    id: u64,
}
impl Wire {
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let response_timeout = if params["name"] == "lomi_android_setup_plan" {
            50
        } else {
            10
        };
        self.id += 1;
        let frame = format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params})
        );
        self.input
            .write_all(frame.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(response_timeout),
            self.output.read_line(&mut line),
        )
        .await
        .map_err(|_| "Helper timed out")?
        .map_err(|e| e.to_string())?;
        if line.len() > 1024 * 1024 {
            return Err("Helper response too large".into());
        }
        let value: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if value["id"] != self.id || value["jsonrpc"] != "2.0" {
            return Err("Invalid helper JSON-RPC response".into());
        }
        Ok(value)
    }
    async fn settled(&mut self, operation: &str) -> Result<Value, String> {
        self.settled_with_limit(operation, 900).await
    }
    async fn settled_with_limit(
        &mut self,
        operation: &str,
        attempts: usize,
    ) -> Result<Value, String> {
        for _ in 0..attempts {
            let value = self
                .tool("lomi_operation_get", json!({"operationId":operation}))
                .await?;
            let state = value["structuredContent"]["data"]["state"]
                .as_str()
                .unwrap_or("");
            if !matches!(state, "queued" | "running" | "cancelling") {
                return Ok(value);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Err(format!("Operation did not settle: {operation}"))
    }
    async fn android_packet(
        &mut self,
        base: &Value,
        sequence: u64,
        event: Value,
    ) -> Result<Value, String> {
        let mut args = base.clone();
        args["inputSequence"] = json!(sequence.to_string());
        args["event"] = event;
        let receipt = self.tool("lomi_android_input", args).await?;
        let receipt = self
            .settled(
                receipt["structuredContent"]["data"]["operationId"]
                    .as_str()
                    .ok_or_else(|| format!("Android sequence {sequence}: {receipt}"))?,
            )
            .await?;
        if receipt["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Android sequence {sequence} failed: {receipt}"));
        }
        Ok(receipt)
    }
    async fn tool(&mut self, name: &str, args: Value) -> Result<Value, String> {
        let result = self
            .call("tools/call", json!({"name":name,"arguments":args}))
            .await?;
        result
            .get("result")
            .cloned()
            .ok_or_else(|| result.to_string())
    }
}

async fn run(app: &tauri::AppHandle, directory: &Path) -> Result<Value, String> {
    let browser_fixture: Value = serde_json::from_slice(
        &std::fs::read(directory.join("browser-fixture.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let main = app.get_webview("main").ok_or("Missing main")?;
    wait_for(&main, "Boolean(document.querySelector('.app-shell'))").await?;
    javascript(&main, r#"const m=await import('/src/agent-workspace.ts');window.__domainConflicts=[];m.observeAgentDomainConflicts((left,right)=>{const changes=[];const scan=(a,b,path)=>{if(changes.length>=32||JSON.stringify(a)===JSON.stringify(b))return;if(a&&b&&typeof a==='object'&&typeof b==='object'){for(const key of new Set([...Object.keys(a),...Object.keys(b)]))scan(a[key],b[key],path+'.'+key);}else changes.push({path,left:JSON.stringify(a)?.slice(0,160),right:JSON.stringify(b)?.slice(0,160)});};scan(left,right,'session');window.__domainConflicts.push({time:Date.now(),changes});window.__domainConflicts=window.__domainConflicts.slice(-32);});return true;"#).await?;

    javascript(&main,"await window.__TAURI_INTERNALS__.invoke('open_settings',{page:'agent-control'});return true;").await?;
    let settings = app.get_webview("settings").ok_or("Missing settings")?;
    click(&settings, "Enable for this Lomi session").await?;
    wait_for(&settings,"document.body.textContent.includes('Ready for pairing') && Boolean(document.querySelector('#control-config'))").await?;
    let config = evaluate(
        &settings,
        "JSON.parse(document.querySelector('#control-config').value)",
    )
    .await?;
    let helper = &config["mcpServers"]["lomi"];
    let args: Vec<String> =
        serde_json::from_value(helper["args"].clone()).map_err(|e| e.to_string())?;
    let stderr = std::fs::File::create(directory.join("helper.log")).map_err(|e| e.to_string())?;
    let mut child =
        tokio::process::Command::new(helper["command"].as_str().ok_or("Missing helper path")?)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(stderr)
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;
    let mut wire = Wire {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
        id: 0,
    };
    let initialized=wire.call("initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"native-lomi-fixture","version":"1"}})).await?;
    if initialized["result"]["serverInfo"]["name"] != "lomi-mcp" {
        return Err("Wrong MCP helper".into());
    }
    wire.input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .map_err(|e| e.to_string())?;
    let catalog = wire.call("tools/list", json!({})).await?;
    wait_for(
        &settings,
        "Boolean(document.querySelector('.agent-control-request select'))",
    )
    .await?;
    let denied = wire.tool("lomi_workspace_list", json!({})).await?;
    if denied["structuredContent"]["code"] != "PAIRING_REQUIRED" {
        return Err(format!("Unapproved read was not denied: {denied}"));
    }
    let settings_denied=javascript(&main,"try {await window.__TAURI_INTERNALS__.invoke('agent_control_approve',{requestId:'forged',workspaceIds:[],scopes:['workspace.read']});return false;}catch{return true;}").await?;
    if settings_denied != true {
        return Err("Main can approve pairing".into());
    }
    let workspace=evaluate(&settings,"(() => {const s=document.querySelector('.agent-control-request select');s.value=Array.from(s.options).find(o=>o.textContent.includes('Visible workspace')).value;s.dispatchEvent(new Event('change',{bubbles:true}));return s.value;})()").await?;
    evaluate(
        &settings,
        "document.querySelectorAll('.agent-control-request input[type=checkbox]').forEach(e=>{if (!e.closest('fieldset') && !/browser|Android|Chat AI/.test(e.closest('label').textContent)) e.click();});true",
    )
    .await?;
    if std::env::var_os("LOMI_MCP_SETTINGS_UPDATE_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_SETTINGS_TERMINAL_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_SETTINGS_KEYBINDS_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_SETTINGS_THEMES_ONLY").is_some()
    {
        wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting application preference changes')).querySelector('input').disabled").await?;
        evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting application preference changes')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    }
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting workspace closure')).querySelector('input').disabled").await?;
    if std::env::var_os("LOMI_MCP_PROJECT_OPEN_ONLY").is_some() {
        wait_for(&settings,"![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting access to new project folders')).querySelector('input').disabled").await?;
        evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting access to new project folders')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    }

    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting workspace closure')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow rearranging existing panels')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow rearranging existing panels')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow importing APK files')).querySelector('input').disabled").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow reading unsaved editor')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow reading Git information')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting Git changes')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting Git changes')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting contact with configured Git remotes')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting contact with configured Git remotes')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting discard of working Git changes')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting discard of working Git changes')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting Git pulls')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting Git pulls')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting Git pushes')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting Git pushes')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow reading unsaved editor')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow editing loaded buffers')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow editing loaded buffers')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;

    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow saving editor files')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow saving editor files')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow creating project files')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow creating project files')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow renaming and moving project files')).querySelector('input').disabled").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow renaming and moving project files')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow moving project files and folders to Trash')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow importing APK files')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
    if std::env::var_os("LOMI_MCP_ARTIFACT_FILES_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_BROWSER_DOWNLOAD_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_BROWSER_UPLOAD_ONLY").is_some()
    {
        for label in [
            "Allow importing project files as artifacts",
            "Allow exporting artifacts to new project files",
        ] {
            evaluate(&settings, &format!("(()=>{{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes({})).querySelector('input');if(e.disabled)throw Error('Disabled artifact scope');if(!e.checked)e.click();e.scrollIntoView({{block:'center'}});return true;}})()",json!(label))).await?;
        }
        screenshot(&settings, directory.join("artifact-permissions.png")).await?;
    }
    evaluate(&settings, "[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow opening and navigating isolated browser panels')).querySelector('input').click();true").await?;
    wait_for(
        &settings,
        "Boolean(document.querySelector('.agent-control-request textarea'))",
    )
    .await?;
    evaluate(&settings, &format!("(()=>{{const e=document.querySelector('.agent-control-request textarea');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,{});e.dispatchEvent(new Event('input',{{bubbles:true}}));return true;}})()",browser_fixture["origin"])).await?;
    evaluate(&settings, "[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow reading page text, form structure and browser logs')).querySelector('input').click();true").await?;
    evaluate(&settings, "[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow clicking, typing and scrolling in pages')).querySelector('input').click();true").await?;
    evaluate(&settings,"[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow screenshots of pages')).querySelector('input').click();true").await?;
    if std::env::var_os("LOMI_MCP_BROWSER_UPLOAD_ONLY").is_some() {
        evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting file uploads')).querySelector('input');if(e.disabled)throw Error('Upload permission unavailable');e.click();e.scrollIntoView({block:'center'});return true;})()").await?;
        screenshot(&settings, directory.join("browser-upload-permission.png")).await?;
    }
    if std::env::var_os("LOMI_MCP_BROWSER_DOWNLOAD_ONLY").is_some() {
        evaluate(&settings,"(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow downloading page files')).querySelector('input');if(e.disabled)throw Error('Download permission unavailable');e.click();e.scrollIntoView({block:'center'});return true;})()").await?;
        screenshot(&settings, directory.join("browser-download-permission.png")).await?;
    }

    let android_fixture = std::fs::read(directory.join("android-fixture.json"))
        .ok()
        .map(|bytes| serde_json::from_slice::<Value>(&bytes))
        .transpose()
        .map_err(|e| e.to_string())?;
    if let Some(device) = &android_fixture {
        evaluate(&settings, "[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow reading selected Android device status')).querySelector('input').click();true").await?;

        if std::env::var_os("LOMI_MCP_ANDROID_SETUP_ONLY").is_some() {
            for text in [
                "Allow Android SDK setup, recovery and cache cleanup requests",
                "Allow creating devices and changing the selected device",
            ] {
                evaluate(&settings,&format!("[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes({})).querySelector('input').click();true",json!(text))).await?;
            }
        }
        let selector = "document.querySelector('select[id^=control-android-]')";
        wait_for(
            &settings,
            &format!(
                "[...({selector}?.options ?? [])].some(o=>o.value==={})",
                device["deviceId"]
            ),
        )
        .await?;
        evaluate(&settings, &format!("(()=>{{const s={selector};s.value={};s.dispatchEvent(new Event('change',{{bubbles:true}}));return true;}})()", device["deviceId"])).await?;
        evaluate(&settings, "[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow starting and stopping this Android device')).querySelector('input').click();true").await?;
        wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow touch, keys and text in this Android device')).querySelector('input').disabled").await?;
        evaluate(&settings, "[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow touch, keys and text in this Android device')).querySelector('input').click();true").await?;
        evaluate(&settings, "[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow reading screen content in this Android device')).querySelector('input').click();true").await?;
        evaluate(&settings, "[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow screenshots of this Android device')).querySelector('input').click();true").await?;
        wait_for(&settings,"![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting APK installation')).querySelector('input').disabled").await?;
        evaluate(&settings,"[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting APK installation')).querySelector('input').click();true").await?;
        evaluate(&settings,"[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow launching approved Android apps')).querySelector('input').click();true").await?;
        evaluate(&settings,"[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow reading logs from approved Android apps')).querySelector('input').click();true").await?;
        wait_for(
            &settings,
            "Boolean(document.querySelector('textarea[id^=control-packages-]'))",
        )
        .await?;
        evaluate(&settings,"(()=>{const e=document.querySelector('textarea[id^=control-packages-]');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,'org.lomi.inputtest');e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()").await?;
    }
    if std::env::var_os("LOMI_MCP_PROJECT_CLOSE_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_ANDROID_LAYOUT_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some()
    {
        evaluate(&settings, "[...document.querySelectorAll('.agent-control-request fieldset label')].find(e=>e.textContent.includes('Private workspace')).querySelector('input').click();true").await?;
        wait_for(&settings, "![...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting project closure')).querySelector('input').disabled").await?;
        evaluate(&settings, "(()=>{const e=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Allow requesting project closure')).querySelector('input');if(!e.checked)e.click();return true;})()").await?;
        screenshot(&settings, directory.join("project-close-permissions.png")).await?;
    }
    if std::env::var_os("LOMI_MCP_CHAT_READ_ONLY").is_some()
        || (std::env::var_os("LOMI_MCP_CHAT_OPEN_ONLY").is_some()
            || std::env::var_os("LOMI_MCP_CHAT_DRAFT_ONLY").is_some()
            || std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some())
    {
        chat_probe::prepare(app, &main, &settings, &workspace, directory).await?;
    }
    if let Ok(shell) = std::env::var("LOMI_MCP_TERMINAL_ONLY") {
        evaluate(&settings, &format!("(()=>{{const s=[...document.querySelectorAll('.agent-control-request label')].find(e=>e.textContent.includes('Approved terminal shell')).querySelector('select');s.value={};s.dispatchEvent(new Event('change',{{bubbles:true}}));s.scrollIntoView({{block:'center'}});return true;}})()",json!(format!("local:{shell}")))).await?;
        screenshot(&settings, directory.join("terminal-permission.png")).await?;
    }
    click(&settings, "Approve session").await?;
    let mut listed = Value::Null;
    for _ in 0..50 {
        listed = wire
            .tool("lomi_workspace_list", json!({"limit":100}))
            .await?;
        if listed["structuredContent"]["status"] == "ok" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let android_list = wire
        .tool("lomi_android_list", json!({"workspaceId":workspace}))
        .await?;
    if let Some(device) = &android_fixture {
        let devices = &android_list["structuredContent"]["data"]["devices"];
        if devices["items"]
            .as_array()
            .is_none_or(|items| items.len() != 1)
            || devices["items"][0]["deviceId"] != device["deviceId"]
            || devices["items"][0]["phase"] != "stopped"
            || devices["items"][0]["processAlive"] != false
        {
            return Err(format!("Wrong selected Android metadata: {android_list}"));
        }
        let foreign = wire
            .tool("lomi_android_list", json!({"workspaceId":"foreign"}))
            .await?;
        if foreign["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
            return Err("Foreign workspace disclosed Android".into());
        }
        std::fs::write(
            directory.join("android-list.json"),
            serde_json::to_vec_pretty(&json!({"devices":android_list,"foreign":foreign})).unwrap(),
        )
        .map_err(|e| e.to_string())?;
    } else if android_list["structuredContent"]["code"] != "SCOPE_DENIED" {
        return Err("Unapproved Android metadata disclosed".into());
    }
    let items = listed["structuredContent"]["data"]["items"]
        .as_array()
        .ok_or_else(|| listed.to_string())?;
    let project_close = std::env::var_os("LOMI_MCP_PROJECT_CLOSE_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_ANDROID_LAYOUT_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some();
    if project_close {
        if items.len() != 2
            || !items.iter().any(|w| w["id"] == workspace)
            || !items.iter().any(|w| w["id"] == "foreign-workspace")
        {
            return Err("Project-close workspace enrollment mismatch".into());
        }
    } else if items.len() != 1
        || items[0]["id"] != workspace
        || items[0]["name"] != "Visible workspace"
    {
        return Err("Workspace authorization leaked or omitted a resource".into());
    }
    let connected = wire
        .tool("lomi_connect", json!({"workspaceId":workspace}))
        .await?;
    if connected["structuredContent"]["status"] != "ok" {
        return Err("Cannot select approved workspace".into());
    }
    if std::env::var_os("LOMI_MCP_ROUTING_ONLY").is_some() {
        routing_probe::qualify(
            app,
            &mut wire,
            &settings,
            &workspace,
            helper,
            directory,
            &browser_fixture,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"routing-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_PERFORMANCE_ONLY").is_some() {
        performance_probe::qualify(
            app,
            &mut wire,
            &workspace,
            &connected["structuredContent"]["data"]["retryEpoch"],
            directory,
            &browser_fixture,
            child.id().ok_or("Missing helper PID")?,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"performance-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_TERMINAL_ONLY").is_some() {
        terminal_probe::qualify(
            app,
            &mut wire,
            &main,
            &workspace,
            &connected["structuredContent"]["data"]["retryEpoch"],
            directory,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"terminal-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_BROWSER_UPLOAD_ONLY").is_some() {
        browser_upload_probe::qualify(
            app,
            &mut wire,
            &main,
            &workspace,
            &connected["structuredContent"]["data"]["retryEpoch"],
            directory,
            &browser_fixture,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"browser-upload-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_BROWSER_DOWNLOAD_ONLY").is_some() {
        browser_download_probe::qualify(
            app,
            &mut wire,
            &main,
            &workspace,
            &connected["structuredContent"]["data"]["retryEpoch"],
            directory,
            &browser_fixture,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"browser-download-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_ARTIFACT_FILES_ONLY").is_some() {
        artifact_files_probe::qualify(
            app,
            &mut wire,
            &main,
            &workspace,
            &connected["structuredContent"]["data"]["retryEpoch"],
            directory,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"artifact-files-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_BROWSER_LOGS_ONLY").is_some() {
        browser_logs_probe::qualify(
            app,
            &mut wire,
            &main,
            &workspace,
            &connected["structuredContent"]["data"]["retryEpoch"],
            directory,
            &browser_fixture,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"browser-logs-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_BROWSER_FRAMES_ONLY").is_some() {
        browser_frames_probe::qualify(
            app,
            &mut wire,
            &main,
            &workspace,
            &connected["structuredContent"]["data"]["retryEpoch"],
            directory,
            &browser_fixture,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"browser-frames-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_ANDROID_LAYOUT_ONLY").is_some() {
        android_layout_probe::qualify(
            app,
            &mut wire,
            &main,
            &workspace,
            &connected["structuredContent"]["data"]["retryEpoch"],
            directory,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"android-layout-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_ANDROID_SETUP_ONLY").is_some() {
        android_setup_probe::qualify(
            app,
            &mut wire,
            &main,
            &settings,
            &workspace,
            &connected["structuredContent"]["data"]["retryEpoch"],
            directory,
        )
        .await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"android-setup-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_CHAT_READ_ONLY").is_some()
        || (std::env::var_os("LOMI_MCP_CHAT_OPEN_ONLY").is_some()
            || std::env::var_os("LOMI_MCP_CHAT_DRAFT_ONLY").is_some()
            || std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some())
    {
        chat_probe::qualify(app, &mut wire, &main, &settings, &workspace, directory).await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":if std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some(){"chat-send-only"}else if std::env::var_os("LOMI_MCP_CHAT_DRAFT_ONLY").is_some(){"chat-draft-only"}else if std::env::var_os("LOMI_MCP_CHAT_OPEN_ONLY").is_some(){"chat-open-only"}else{"chat-read-only"},"catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_PROJECT_OPEN_ONLY").is_some() {
        qualify_project_open(&mut wire,&main,&settings,directory,json!({"anchor":workspace,"projectId":items[0]["projectId"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"]})).await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"project-open-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_SETTINGS_OPEN_ONLY").is_some() {
        qualify_settings_open(&mut wire,&main,&settings,directory,json!({"anchor":workspace,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"]})).await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"settings-open-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_SETTINGS_TERMINAL_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_SETTINGS_KEYBINDS_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_SETTINGS_THEMES_ONLY").is_some()
    {
        settings_probe::qualify_terminal_preferences(&mut wire,&main,&settings,directory,json!({"anchor":workspace,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"]})).await?;
        qualify_settings_update(&mut wire,&main,&settings,directory,json!({"anchor":workspace,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"]})).await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":if std::env::var_os("LOMI_MCP_SETTINGS_THEMES_ONLY").is_some() { "settings-themes-only" } else if std::env::var_os("LOMI_MCP_SETTINGS_KEYBINDS_ONLY").is_some() { "settings-keybinds-only" } else { "settings-terminal-only" },"catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_SETTINGS_UPDATE_ONLY").is_some() {
        qualify_settings_update(&mut wire,&main,&settings,directory,json!({"anchor":workspace,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"]})).await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"settings-update-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_SETTINGS_READ_ONLY").is_some() {
        qualify_settings_read(&mut wire,&main,&settings,directory,json!({"anchor":workspace,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"]})).await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"settings-read-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if project_close {
        let project_id = &items.iter().find(|w| w["id"] == workspace).unwrap()["projectId"];
        qualify_project_close(&mut wire, &main, &settings, directory, json!({"anchor":workspace,"projectId":project_id,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"origin":browser_fixture["origin"],"projectClose":true})).await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"project-close-only","catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    if std::env::var_os("LOMI_MCP_CLOSE_STRESS_ONLY").is_some() {
        qualify_idle_workspace_close(&mut wire, &main, directory, &json!({"anchor":workspace,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"origin":browser_fixture["origin"]})).await?;
        child.kill().await.map_err(|e| e.to_string())?;
        return Ok(
            json!({"profile":"workspace-close-stress-only","rounds":6,"catalogCount":catalog["result"]["tools"].as_array().map(Vec::len)}),
        );
    }
    wait_for(&main, "!!document.querySelector('.cm-content')").await?;
    let editor_args = json!({"workspaceId":workspace,"panelId":"mcp-control-fixture","relativePath":"fixture.txt"});
    let editor_initial = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    let editor_data = &editor_initial["structuredContent"]["data"];
    if editor_data["source"] != "buffer"
        || editor_data["dirty"] != false
        || editor_data["content"] != "MCP native control qualification\n"
    {
        return Err(format!(
            "Shared editor initial read failed: {editor_initial}"
        ));
    }
    // Inject a competing user transaction into the real retained CodeMirror runtime.
    javascript(&main, "const m=await import('/src/editor-runtime.ts');const d=m.documents().find(d=>d.location.relative==='fixture.txt');d.dispatch({changes:{from:0,to:d.state.doc.length,insert:'Unsaved Zażółć 🙂\\n'}});return true;").await?;
    let mut stale_editor_args = editor_args.clone();
    stale_editor_args["documentId"] = editor_data["documentId"].clone();
    stale_editor_args["expectedBufferRevision"] = editor_data["bufferRevision"].clone();
    let stale_editor = wire.tool("lomi_editor_read", stale_editor_args).await?;
    if stale_editor["structuredContent"]["code"] != "REVISION_CONFLICT" {
        return Err("Editor accepted stale buffer revision".into());
    }
    let editor_dirty = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    let dirty_data = &editor_dirty["structuredContent"]["data"];
    if dirty_data["dirty"] != true
        || dirty_data["content"] != "Unsaved Zażółć 🙂\n"
        || dirty_data["documentId"] != editor_data["documentId"]
    {
        return Err(format!(
            "Shared editor did not disclose exact dirty buffer: {editor_dirty}"
        ));
    }
    let disk_while_dirty = wire
        .tool(
            "lomi_files_read",
            json!({"workspaceId":workspace,"relativePath":"fixture.txt"}),
        )
        .await?;
    if disk_while_dirty["structuredContent"]["data"]["content"]
        != "MCP native control qualification\n"
    {
        return Err("Buffer read mutated disk".into());
    }
    javascript(&main, "const m=await import('/src/editor-runtime.ts');m.documents().find(d=>d.location.relative==='fixture.txt').command('undo');return true;").await?;
    let editor_undo = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    if editor_undo["structuredContent"]["data"]["content"] != editor_data["content"]
        || editor_undo["structuredContent"]["data"]["dirty"] != false
    {
        return Err(format!("Editor history was not preserved: {editor_undo}"));
    }
    let edit_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let edit_input = json!({"workspaceId":workspace,"panelId":"mcp-control-fixture","relativePath":"fixture.txt",
        "documentId":editor_undo["structuredContent"]["data"]["documentId"],
        "expectedBufferRevision":editor_undo["structuredContent"]["data"]["bufferRevision"],
        "expectedDiskRevision":editor_undo["structuredContent"]["data"]["diskRevision"],
        "expectedRevision":edit_revision["structuredContent"]["data"]["domainRevision"],
        "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"editor-native-edits",
        "edits":[{"fromUtf16":0,"toUtf16":0,"insert":"Agent🙂\n"},{"fromUtf16":editor_undo["structuredContent"]["data"]["totalUtf16"],"toUtf16":editor_undo["structuredContent"]["data"]["totalUtf16"],"insert":"tail\n"}]});
    let edited = wire
        .tool("lomi_editor_apply_edits", edit_input.clone())
        .await?;
    let edited_id = edited["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("No edit receipt: {edited}"))?
        .to_string();
    let edited_receipt = wire.settled(&edited_id).await?;
    if edited_receipt["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Buffer edit failed: {edited_receipt}"));
    }
    let edited_buffer = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    if edited_buffer["structuredContent"]["data"]["content"]
        != "Agent🙂\nMCP native control qualification\ntail\n"
    {
        return Err(format!("Wrong edited buffer: {edited_buffer}"));
    }
    let edit_retry = wire
        .tool("lomi_editor_apply_edits", edit_input.clone())
        .await?;
    if edit_retry["structuredContent"]["data"]["operationId"] != edited_id {
        return Err("Edit retry duplicated operation".into());
    }
    let mut invalid = edit_input.clone();
    invalid["requestKey"] = json!("editor-native-invalid-range");
    invalid["expectedBufferRevision"] =
        edited_buffer["structuredContent"]["data"]["bufferRevision"].clone();
    invalid["edits"] = json!([{"fromUtf16":0,"toUtf16":1,"insert":"Z"},{"fromUtf16":6,"toUtf16":7,"insert":"broken"}]);
    let rejected = wire.tool("lomi_editor_apply_edits", invalid).await?;
    let rejected_id = rejected["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| rejected.to_string())?;
    let rejected_receipt = wire.settled(rejected_id).await?;
    if rejected_receipt["structuredContent"]["data"]["state"] != "failed"
        || rejected_receipt["structuredContent"]["data"]["effectState"] != "none"
        || rejected_receipt["structuredContent"]["data"]["result"]["code"] != "REVISION_CONFLICT"
    {
        return Err(format!(
            "Invalid edit range was not rejected: {rejected_receipt}"
        ));
    }
    let after_rejected = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    if after_rejected["structuredContent"]["data"] != edited_buffer["structuredContent"]["data"] {
        return Err("Invalid batch partially changed the document".into());
    }
    screenshot(&main, directory.join("editor-edits.png")).await?;
    javascript(&main, "const m=await import('/src/editor-runtime.ts');m.documents().find(d=>d.location.relative==='fixture.txt').command('undo');return true;").await?;
    let undone_edit = wire.tool("lomi_editor_read", editor_args.clone()).await?;
    if undone_edit["structuredContent"]["data"]["content"] != editor_data["content"]
        || undone_edit["structuredContent"]["data"]["dirty"] != false
    {
        return Err("Agent edits did not undo together".into());
    }
    std::fs::write(directory.join("editor-edits.json"), serde_json::to_vec_pretty(&json!({"receipt":edited_receipt,"buffer":edited_buffer,"retry":edit_retry,"rejected":rejected_receipt,"unchanged":after_rejected,"undo":undone_edit})).unwrap()).map_err(|e| e.to_string())?;
    let mut foreign_editor = editor_args;
    foreign_editor["workspaceId"] = json!("foreign-workspace");
    let foreign_editor = wire.tool("lomi_editor_read", foreign_editor).await?;
    if foreign_editor["structuredContent"]["status"] != "error" {
        return Err("Editor crossed workspace scope".into());
    }
    std::fs::write(directory.join("editor-read.json"), serde_json::to_vec_pretty(&json!({"initial":editor_initial,"dirty":editor_dirty,"stale":stale_editor,"disk":disk_while_dirty,"undo":editor_undo,"foreign":foreign_editor})).unwrap()).map_err(|e|e.to_string())?;
    let open_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let mut open_input = json!({"workspaceId":workspace,"relativePath":"mcp-read-fixture.txt",
        "expectedRevision":open_revision["structuredContent"]["data"]["domainRevision"],
        "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"editor-native-open"});
    let opened = wire.tool("lomi_editor_open", open_input.clone()).await?;
    let opened_id = opened["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("No editor open receipt: {opened}"))?
        .to_string();
    let opened = wire.settled(&opened_id).await?;
    if opened["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Editor open failed: {opened}"));
    }
    let opened_meta = &opened["structuredContent"]["data"]["result"];
    let opened_read = json!({"workspaceId":workspace,"panelId":opened_meta["panelId"],"relativePath":"mcp-read-fixture.txt"});
    let opened_buffer = wire.tool("lomi_editor_read", opened_read.clone()).await?;
    let opened_data = &opened_buffer["structuredContent"]["data"];
    if opened_data["content"] != "Disk Zażółć 🙂\nsecond line\n"
        || opened_data["lineEndings"] != "cr_lf"
        || opened_data["dirty"] != false
        || opened_data["documentId"] != opened_meta["documentId"]
    {
        return Err(format!(
            "Opened editor bytes/identity changed: {opened_buffer}"
        ));
    }
    let opened_retry = wire.tool("lomi_editor_open", open_input.clone()).await?;
    if opened_retry["structuredContent"]["data"]["operationId"] != opened_id {
        return Err("Editor open retry created another operation".into());
    }
    let open_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let draft = wire.tool("lomi_editor_apply_edits", json!({"workspaceId":workspace,
        "panelId":opened_meta["panelId"],"relativePath":"mcp-read-fixture.txt",
        "documentId":opened_data["documentId"],"expectedBufferRevision":opened_data["bufferRevision"],
        "expectedDiskRevision":opened_data["diskRevision"],
        "expectedRevision":open_revision["structuredContent"]["data"]["domainRevision"],
        "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"editor-open-draft",
        "edits":[{"fromUtf16":0,"toUtf16":0,"insert":"Draft🙂\n"}]})).await?;
    let draft = wire
        .settled(
            draft["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| draft.to_string())?,
        )
        .await?;
    if draft["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Opened editor draft failed: {draft}"));
    }
    let draft_buffer = wire.tool("lomi_editor_read", opened_read.clone()).await?;
    let open_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    open_input["expectedRevision"] =
        open_revision["structuredContent"]["data"]["domainRevision"].clone();
    open_input["requestKey"] = json!("editor-native-reopen-dirty");
    let reopened = wire.tool("lomi_editor_open", open_input.clone()).await?;
    let reopened = wire
        .settled(
            reopened["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| reopened.to_string())?,
        )
        .await?;
    let preserved = wire.tool("lomi_editor_read", opened_read.clone()).await?;
    if reopened["structuredContent"]["data"]["state"] != "succeeded"
        || reopened["structuredContent"]["data"]["result"]["panelId"] != opened_meta["panelId"]
        || preserved["structuredContent"]["data"] != draft_buffer["structuredContent"]["data"]
        || preserved["structuredContent"]["data"]["dirty"] != true
    {
        return Err(format!(
            "Reopening changed the shared dirty buffer: {reopened} {preserved}"
        ));
    }
    let panels = wire
        .tool("lomi_panel_list", json!({"workspaceId":workspace}))
        .await?;
    if panels["structuredContent"]["data"]["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|p| p["id"] == opened_meta["panelId"])
                .count()
        })
        != Some(1)
    {
        return Err(format!("Editor open duplicated its panel: {panels}"));
    }
    screenshot(&main, directory.join("editor-open.png")).await?;
    javascript(&main, "const m=await import('/src/editor-runtime.ts');m.documents().find(d=>d.location.relative==='mcp-read-fixture.txt').command('undo');return true;").await?;
    let open_undo = wire.tool("lomi_editor_read", opened_read.clone()).await?;
    if open_undo["structuredContent"]["data"]["content"] != opened_data["content"]
        || open_undo["structuredContent"]["data"]["dirty"] != false
    {
        return Err("Reopening reset the shared undo history".into());
    }
    javascript(&main, "const m=await import('/src/editor-runtime.ts');m.documents().find(d=>d.location.relative==='mcp-read-fixture.txt').command('redo');return true;").await?;
    let save_buffer = wire.tool("lomi_editor_read", opened_read.clone()).await?;
    let save_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let save_input = json!({"workspaceId":workspace,"panelId":opened_meta["panelId"],"relativePath":"mcp-read-fixture.txt",
        "documentId":save_buffer["structuredContent"]["data"]["documentId"],
        "expectedBufferRevision":save_buffer["structuredContent"]["data"]["bufferRevision"],
        "expectedDiskRevision":save_buffer["structuredContent"]["data"]["diskRevision"],
        "expectedRevision":save_revision["structuredContent"]["data"]["domainRevision"],
        "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"editor-native-save"});
    let saved = wire.tool("lomi_editor_save", save_input.clone()).await?;
    let saved_id = saved["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Save has no operation: {saved}"))?
        .to_string();
    let saved = wire.settled(&saved_id).await?;
    if saved["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Editor save failed: {saved}"));
    }
    let saved_buffer = wire.tool("lomi_editor_read", opened_read.clone()).await?;
    let saved_disk = wire
        .tool(
            "lomi_files_read",
            json!({"workspaceId":workspace,"relativePath":"mcp-read-fixture.txt"}),
        )
        .await?;
    if saved_buffer["structuredContent"]["data"]["dirty"] != false
        || saved_buffer["structuredContent"]["data"]["bufferRevision"]
            != save_input["expectedBufferRevision"]
        || saved_buffer["structuredContent"]["data"]["diskRevision"]
            != saved["structuredContent"]["data"]["result"]["diskRevision"]
        || saved_disk["structuredContent"]["data"]["content"]
            != "Draft🙂\r\nDisk Zażółć 🙂\r\nsecond line\r\n"
        || std::fs::read(directory.join("project/mcp-read-fixture.txt"))
            .map_err(|e| e.to_string())?
            != "Draft🙂\r\nDisk Zażółć 🙂\r\nsecond line\r\n".as_bytes()
    {
        return Err(format!(
            "Save did not preserve exact bytes/revisions: {saved_buffer} {saved_disk}"
        ));
    }
    let save_retry = wire.tool("lomi_editor_save", save_input.clone()).await?;
    if save_retry["structuredContent"]["data"]["operationId"] != saved_id {
        return Err("Save retry created a second writer".into());
    }
    javascript(&main, "const m=await import('/src/editor-runtime.ts');m.documents().find(d=>d.location.relative==='mcp-read-fixture.txt').command('undo');return true;").await?;
    let save_undo = wire.tool("lomi_editor_read", opened_read.clone()).await?;
    if save_undo["structuredContent"]["data"]["content"] != opened_data["content"]
        || save_undo["structuredContent"]["data"]["dirty"] != true
        || std::fs::read(directory.join("project/mcp-read-fixture.txt"))
            .map_err(|e| e.to_string())?
            != "Draft🙂\r\nDisk Zażółć 🙂\r\nsecond line\r\n".as_bytes()
    {
        return Err(format!(
            "Save reset history or undo changed disk: {save_undo}"
        ));
    }
    javascript(&main, "const m=await import('/src/editor-runtime.ts');m.documents().find(d=>d.location.relative==='mcp-read-fixture.txt').command('redo');return true;").await?;
    let save_redo = wire.tool("lomi_editor_read", opened_read.clone()).await?;
    if save_redo["structuredContent"]["data"]["dirty"] != false {
        return Err(format!("Redo did not return to saved state: {save_redo}"));
    }
    screenshot(&main, directory.join("editor-save.png")).await?;
    std::fs::write(directory.join("editor-save.json"), serde_json::to_vec_pretty(&json!({"saved":saved,"buffer":saved_buffer,"disk":saved_disk,"retry":save_retry,"undo":save_undo,"redo":save_redo})).unwrap()).map_err(|e|e.to_string())?;
    // Restore this shared fixture through the same MCP save path for later disk-read checks.
    javascript(&main, "const m=await import('/src/editor-runtime.ts');m.documents().find(d=>d.location.relative==='mcp-read-fixture.txt').command('undo');return true;").await?;
    let restore_buffer = wire.tool("lomi_editor_read", opened_read).await?;
    let restore_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let mut restore_input = save_input;
    restore_input["requestKey"] = json!("editor-native-save-restore");
    restore_input["expectedRevision"] =
        restore_revision["structuredContent"]["data"]["domainRevision"].clone();
    restore_input["expectedBufferRevision"] =
        restore_buffer["structuredContent"]["data"]["bufferRevision"].clone();
    restore_input["expectedDiskRevision"] =
        restore_buffer["structuredContent"]["data"]["diskRevision"].clone();
    let restored = wire.tool("lomi_editor_save", restore_input).await?;
    let restored = wire
        .settled(
            restored["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| restored.to_string())?,
        )
        .await?;
    if restored["structuredContent"]["data"]["state"] != "succeeded"
        || std::fs::read(directory.join("project/mcp-read-fixture.txt"))
            .map_err(|e| e.to_string())?
            != "Disk Zażółć 🙂\r\nsecond line\r\n".as_bytes()
    {
        return Err(format!("Fixture restore save failed: {restored}"));
    }
    std::fs::write(
        directory.join("editor-save-restore.json"),
        serde_json::to_vec_pretty(&restored).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    open_input["relativePath"] = json!(".env.fixture");
    open_input["requestKey"] = json!("editor-open-secret");
    let secret_open = wire.tool("lomi_editor_open", open_input).await?;
    if secret_open["structuredContent"]["code"] != "SCOPE_DENIED" {
        return Err("Editor open accepted a secret path".into());
    }
    std::fs::write(directory.join("editor-open.json"), serde_json::to_vec_pretty(&json!({"opened":opened,"buffer":opened_buffer,"retry":opened_retry,"draft":draft,"reopened":reopened,"preserved":preserved,"undo":open_undo,"secret":secret_open})).unwrap()).map_err(|e|e.to_string())?;
    javascript(&main, "const m=await import('/src/editor-runtime.ts');for(let n=0;n<40;n++){if(!m.documents().find(d=>d.location.relative==='mcp-read-fixture.txt')?.dirty)return true;await new Promise(r=>setTimeout(r,25));}throw Error('Undo dirty state did not settle');").await?;
    let open_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let focus_original = wire.tool("lomi_panel_focus", json!({"workspaceId":workspace,
        "panelId":"mcp-control-fixture","expectedRevision":open_revision["structuredContent"]["data"]["domainRevision"],
        "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"editor-open-focus-original"})).await?;
    let focus_original = wire
        .settled(
            focus_original["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| focus_original.to_string())?,
        )
        .await?;
    if focus_original["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Original editor focus failed: {focus_original}"));
    }
    let open_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let close_opened = wire.tool("lomi_panel_close", json!({"workspaceId":workspace,
        "panelId":opened_meta["panelId"],"expectedRevision":open_revision["structuredContent"]["data"]["domainRevision"],
        "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"editor-open-close"})).await?;
    let close_opened = wire
        .settled(
            close_opened["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| close_opened.to_string())?,
        )
        .await?;
    if close_opened["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!(
            "Opened clean editor could not close: {close_opened}"
        ));
    }
    let file_args =
        json!({"workspaceId":workspace,"relativePath":"mcp-read-fixture.txt","maxChars":8});
    let first_file = wire.tool("lomi_files_read", file_args.clone()).await?;
    let file = &first_file["structuredContent"]["data"];
    if file["source"] != "disk"
        || file["content"] != "Disk Zaż"
        || file["lineEndings"] != "cr_lf"
        || file["truncated"] != true
    {
        return Err(format!("Disk text slice failed: {first_file}"));
    }
    let next_file = wire.tool("lomi_files_read", json!({"workspaceId":workspace,"relativePath":"mcp-read-fixture.txt","startUtf16":file["nextUtf16"],"expectedDiskRevision":file["diskRevision"]})).await?;
    if next_file["structuredContent"]["data"]["content"] != "ółć 🙂\r\nsecond line\r\n" {
        return Err(format!(
            "Disk UTF-16 continuation changed bytes: {next_file}"
        ));
    }
    let secret_file = wire
        .tool(
            "lomi_files_read",
            json!({"workspaceId":workspace,"relativePath":".env.fixture"}),
        )
        .await?;
    if secret_file["structuredContent"]["code"] != "SCOPE_DENIED"
        || secret_file.to_string().contains("PRIVATE_FIXTURE_VALUE")
    {
        return Err("Secret file was disclosed".into());
    }
    let mut wrong_revision = file_args.clone();
    wrong_revision["expectedDiskRevision"] = json!("0".repeat(64));
    let conflict = wire.tool("lomi_files_read", wrong_revision).await?;
    if conflict["structuredContent"]["code"] != "REVISION_CONFLICT" {
        return Err("Disk revision guard failed".into());
    }
    let mut foreign_file = file_args;
    foreign_file["workspaceId"] = json!("foreign");
    let foreign_file = wire.tool("lomi_files_read", foreign_file).await?;
    if foreign_file["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err("File read crossed workspace scope".into());
    }
    std::fs::write(directory.join("files-read.json"), serde_json::to_vec_pretty(&json!({"first":first_file,"next":next_file,"secret":secret_file,"conflict":conflict,"foreign":foreign_file})).unwrap()).map_err(|e|e.to_string())?;
    let list_args = json!({"workspaceId":workspace,"limit":1});
    let first_list = wire.tool("lomi_files_list", list_args.clone()).await?;
    let page = &first_list["structuredContent"]["data"];
    let cursor = page["nextCursor"]
        .as_str()
        .ok_or_else(|| format!("Missing file-list cursor: {first_list}"))?;
    if page["entries"].as_array().map(Vec::len) != Some(1) || page["filtered"] != true {
        return Err("Directory page is not bounded/filtered".into());
    }
    let mut next_args = list_args;
    next_args["cursor"] = json!(cursor);
    let second_list = wire.tool("lomi_files_list", next_args.clone()).await?;
    if second_list["structuredContent"]["data"]["entries"]
        .as_array()
        .map(Vec::len)
        != Some(1)
        || second_list["structuredContent"]["data"]["directoryRevision"]
            != page["directoryRevision"]
        || second_list["structuredContent"]["data"]["entries"][0]["name"]
            == page["entries"][0]["name"]
    {
        return Err(format!("Directory continuation is invalid: {second_list}"));
    }
    let listing = wire
        .tool(
            "lomi_files_list",
            json!({"workspaceId":workspace,"limit":200}),
        )
        .await?;
    let entries = listing["structuredContent"]["data"]["entries"]
        .as_array()
        .ok_or_else(|| listing.to_string())?;
    if !entries.iter().any(|e| e["name"] == "mcp-read-fixture.txt")
        || entries
            .iter()
            .any(|e| e["name"] == ".env.fixture" || e.get("path").is_some())
    {
        return Err("Directory listing leaked hidden or absolute metadata".into());
    }
    std::fs::write(
        directory.join("project/mcp-list-added.txt"),
        b"external directory change",
    )
    .map_err(|e| e.to_string())?;
    let expired = wire.tool("lomi_files_list", next_args).await?;
    if expired["structuredContent"]["code"] != "CURSOR_EXPIRED" {
        return Err("Directory change did not expire its cursor".into());
    }
    std::fs::write(
        directory.join("files-list.json"),
        serde_json::to_vec_pretty(
            &json!({"first":first_list,"second":second_list,"all":listing,"changed":expired}),
        )
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let mut created_entries = Vec::new();
    for (relative, kind, parent) in [
        ("mcp-created", "directory", ""),
        ("mcp-created/empty.txt", "file", "mcp-created"),
    ] {
        let parent_state = wire
            .tool(
                "lomi_files_list",
                json!({"workspaceId":workspace,"relativeDirectory":parent}),
            )
            .await?;
        let revision = wire.tool("lomi_workspace_list", json!({})).await?;
        let args = json!({"workspaceId":workspace,"operation":{"type":"create","relativePath":relative,"kind":kind,
            "expectedParentRevision":parent_state["structuredContent"]["data"]["directoryRevision"]},
            "expectedRevision":revision["structuredContent"]["data"]["domainRevision"],
            "retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("files-create-{kind}")});
        let created = wire.tool("lomi_files_mutate", args.clone()).await?;
        let id = created["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| created.to_string())?
            .to_string();
        let created = wire.settled(&id).await?;
        if created["structuredContent"]["data"]["state"] != "succeeded"
            || created["structuredContent"]["data"]["result"]["newPath"] != relative
            || !directory.join("project").join(relative).exists()
        {
            return Err(format!("Native file creation failed: {created}"));
        }
        let retry = wire.tool("lomi_files_mutate", args).await?;
        if retry["structuredContent"]["data"]["operationId"] != id {
            return Err("Creation retry duplicated its operation".into());
        }
        created_entries.push(json!({"created":created,"retry":retry}));
    }
    let created_path = directory.join("project/mcp-created/empty.txt");
    if std::fs::read(&created_path).map_err(|e| e.to_string())? != b"" {
        return Err("Created file is not empty".into());
    }
    std::fs::write(&created_path, b"preserved user bytes").map_err(|e| e.to_string())?;
    let parent_state = wire
        .tool(
            "lomi_files_list",
            json!({"workspaceId":workspace,"relativeDirectory":"mcp-created"}),
        )
        .await?;
    let revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let existing = wire.tool("lomi_files_mutate",json!({"workspaceId":workspace,"operation":{"type":"create","relativePath":"mcp-created/empty.txt","kind":"file",
        "expectedParentRevision":parent_state["structuredContent"]["data"]["directoryRevision"]},
        "expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"files-create-existing"})).await?;
    let existing = wire
        .settled(
            existing["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| existing.to_string())?,
        )
        .await?;
    if existing["structuredContent"]["data"]["state"] != "failed"
        || existing["structuredContent"]["data"]["effectState"] != "none"
        || existing["structuredContent"]["data"]["result"]["code"] != "REVISION_CONFLICT"
        || std::fs::read(&created_path).map_err(|e| e.to_string())? != b"preserved user bytes"
    {
        return Err(format!("Creation overwrote an existing entry: {existing}"));
    }
    std::fs::write(
        directory.join("files-create.json"),
        serde_json::to_vec_pretty(&json!({"entries":created_entries,"existing":existing})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let move_open = wire.tool("lomi_editor_open",json!({"workspaceId":workspace,"relativePath":"mcp-created/empty.txt","expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"move-open"})).await?;
    let move_open = wire
        .settled(
            move_open["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| move_open.to_string())?,
        )
        .await?;
    if move_open["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Move editor open failed: {move_open}"));
    }
    let move_panel = move_open["structuredContent"]["data"]["result"]["panelId"].clone();
    let mut read_move = json!({"workspaceId":workspace,"panelId":move_panel,"relativePath":"mcp-created/empty.txt"});
    let initial_move = wire.tool("lomi_editor_read", read_move.clone()).await?;
    let revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let draft_move = wire.tool("lomi_editor_apply_edits",json!({"workspaceId":workspace,"panelId":move_panel,"relativePath":"mcp-created/empty.txt",
        "documentId":initial_move["structuredContent"]["data"]["documentId"],"expectedBufferRevision":initial_move["structuredContent"]["data"]["bufferRevision"],"expectedDiskRevision":initial_move["structuredContent"]["data"]["diskRevision"],
        "expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"move-draft","edits":[{"fromUtf16":0,"toUtf16":0,"insert":"Dirty Zażółć 🙂\n"}]})).await?;
    let draft_move = wire
        .settled(
            draft_move["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| draft_move.to_string())?,
        )
        .await?;
    if draft_move["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Move editor edit failed: {draft_move}"));
    }
    let mut moves = Vec::new();
    for (source, target, kind, parent) in [
        (
            "mcp-created/empty.txt",
            "mcp-created/renamed.txt",
            "rename",
            "mcp-created",
        ),
        ("mcp-created/renamed.txt", "mcp-moved.txt", "move", ""),
    ] {
        let parent_state = wire
            .tool(
                "lomi_files_list",
                json!({"workspaceId":workspace,"relativeDirectory":parent}),
            )
            .await?;
        let revision = wire.tool("lomi_workspace_list", json!({})).await?;
        let mut operation = json!({"type":kind,"relativePath":source,"expectedDiskRevision":initial_move["structuredContent"]["data"]["diskRevision"],"expectedParentRevision":parent_state["structuredContent"]["data"]["directoryRevision"]});
        if kind == "rename" {
            operation["newName"] = json!("renamed.txt");
        } else {
            operation["targetRelativePath"] = json!(target);
        }
        let args = json!({"workspaceId":workspace,"operation":operation,"expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("files-native-{kind}")});
        let moved = wire.tool("lomi_files_mutate", args.clone()).await?;
        let id = moved["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| moved.to_string())?
            .to_string();
        let moved = wire.settled(&id).await?;
        if moved["structuredContent"]["data"]["state"] != "succeeded"
            || directory.join("project").join(source).exists()
            || std::fs::read(directory.join("project").join(target)).map_err(|e| e.to_string())?
                != b"preserved user bytes"
        {
            return Err(format!("Native {kind} failed: {moved}"));
        }
        read_move["relativePath"] = json!(target);
        let buffer = wire.tool("lomi_editor_read", read_move.clone()).await?;
        if buffer["structuredContent"]["data"]["documentId"]
            != initial_move["structuredContent"]["data"]["documentId"]
            || buffer["structuredContent"]["data"]["dirty"] != true
            || buffer["structuredContent"]["data"]["content"]
                != "Dirty Zażółć 🙂\npreserved user bytes"
        {
            return Err(format!("{kind} lost the shared dirty buffer: {buffer}"));
        }
        let retry = wire.tool("lomi_files_mutate", args).await?;
        if retry["structuredContent"]["data"]["operationId"] != id {
            return Err(format!("{kind} replay failed: {retry}"));
        }
        moves.push(json!({"receipt":moved,"buffer":buffer,"retry":retry}));
    }
    let parent_state = wire
        .tool("lomi_files_list", json!({"workspaceId":workspace}))
        .await?;
    let revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let container = wire.tool("lomi_files_mutate",json!({"workspaceId":workspace,"operation":{"type":"create","relativePath":"mcp-container","kind":"directory","expectedParentRevision":parent_state["structuredContent"]["data"]["directoryRevision"]},"expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"directory-move-container"})).await?;
    let container = wire
        .settled(
            container["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| container.to_string())?,
        )
        .await?;
    if container["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Directory move container failed: {container}"));
    }
    // Put the retained dirty file into the directory that will be renamed twice.
    let parent_state = wire
        .tool(
            "lomi_files_list",
            json!({"workspaceId":workspace,"relativeDirectory":"mcp-created"}),
        )
        .await?;
    let revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let nested = wire.tool("lomi_files_mutate",json!({"workspaceId":workspace,"operation":{"type":"move","relativePath":"mcp-moved.txt","targetRelativePath":"mcp-created/final.txt","expectedDiskRevision":initial_move["structuredContent"]["data"]["diskRevision"],"expectedParentRevision":parent_state["structuredContent"]["data"]["directoryRevision"]},"expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"directory-move-nest-file"})).await?;
    let nested = wire
        .settled(
            nested["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| nested.to_string())?,
        )
        .await?;
    if nested["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Nesting file failed: {nested}"));
    }
    for (source, target, kind, parent) in [
        ("mcp-created", "mcp-renamed", "rename_directory", ""),
        (
            "mcp-renamed",
            "mcp-container/mcp-renamed",
            "move_directory",
            "mcp-container",
        ),
    ] {
        let source_state = wire
            .tool(
                "lomi_files_list",
                json!({"workspaceId":workspace,"relativeDirectory":source}),
            )
            .await?;
        let parent_state = wire
            .tool(
                "lomi_files_list",
                json!({"workspaceId":workspace,"relativeDirectory":parent}),
            )
            .await?;
        let revision = wire.tool("lomi_workspace_list", json!({})).await?;
        let mut operation = json!({"type":kind,"relativePath":source,"expectedDirectoryRevision":source_state["structuredContent"]["data"]["directoryRevision"],"expectedParentRevision":parent_state["structuredContent"]["data"]["directoryRevision"]});
        if kind == "rename_directory" {
            operation["newName"] = json!("mcp-renamed");
        } else {
            operation["targetRelativePath"] = json!(target);
        }
        let args = json!({"workspaceId":workspace,"operation":operation,"expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("native-{kind}")});
        let moved = wire.tool("lomi_files_mutate", args.clone()).await?;
        let id = moved["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| moved.to_string())?
            .to_owned();
        let moved = wire.settled(&id).await?;
        if moved["structuredContent"]["data"]["state"] != "succeeded"
            || moved["structuredContent"]["data"]["result"]["entryKind"] != "directory"
            || directory.join("project").join(source).exists()
            || std::fs::read(directory.join("project").join(target).join("final.txt"))
                .map_err(|e| e.to_string())?
                != b"preserved user bytes"
        {
            return Err(format!("{kind} failed: {moved}"));
        }
        read_move["relativePath"] = json!(format!("{target}/final.txt"));
        let buffer = wire.tool("lomi_editor_read", read_move.clone()).await?;
        if buffer["structuredContent"]["data"]["documentId"]
            != initial_move["structuredContent"]["data"]["documentId"]
            || buffer["structuredContent"]["data"]["dirty"] != true
            || buffer["structuredContent"]["data"]["content"]
                != "Dirty Zażółć 🙂\npreserved user bytes"
        {
            return Err(format!("{kind} changed the shared document: {buffer}"));
        }
        let retry = wire.tool("lomi_files_mutate", args).await?;
        if retry["structuredContent"]["data"]["operationId"] != id {
            return Err(format!("{kind} retry lost receipt: {retry}"));
        }
        moves.push(json!({"receipt":moved,"buffer":buffer,"retry":retry}));
    }
    wait_for(
        &main,
        "!document.querySelector('.file-tree')?.textContent.includes('mcp-created')",
    )
    .await?;
    screenshot(&main, directory.join("files-move.png")).await?;
    javascript(&main,"const m=await import('/src/editor-runtime.ts');m.documents().find(d=>d.location.relative==='mcp-container/mcp-renamed/final.txt').command('undo');return true;").await?;
    let move_undo = wire.tool("lomi_editor_read", read_move).await?;
    if move_undo["structuredContent"]["data"]["content"] != "preserved user bytes"
        || move_undo["structuredContent"]["data"]["dirty"] != false
    {
        return Err(format!("Move reset undo: {move_undo}"));
    }
    std::fs::write(
        directory.join("files-move.json"),
        serde_json::to_vec_pretty(&json!({"moves":moves,"undo":move_undo})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    javascript(&main,"const m=await import('/src/editor-runtime.ts');for(let n=0;n<40;n++){if(!m.documents().find(d=>d.location.relative==='mcp-container/mcp-renamed/final.txt')?.dirty)return true;await new Promise(r=>setTimeout(r,25));}throw Error('Moved document dirty state did not settle');").await?;
    let revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let focused = wire.tool("lomi_panel_focus",json!({"workspaceId":workspace,"panelId":"mcp-control-fixture","expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"move-focus-original"})).await?;
    let focused = wire
        .settled(
            focused["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| focused.to_string())?,
        )
        .await?;
    if focused["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Move focus failed: {focused}"));
    }
    let revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let closed = wire.tool("lomi_panel_close",json!({"workspaceId":workspace,"panelId":move_panel,"expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"move-close"})).await?;
    let closed = wire
        .settled(
            closed["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| closed.to_string())?,
        )
        .await?;
    if closed["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Move close failed: {closed}"));
    }
    qualify_trash(
        &mut wire,
        &main,
        directory,
        workspace.as_str().ok_or("Missing trash workspace")?,
        &connected,
    )
    .await?;
    qualify_git_status(
        &mut wire,
        directory,
        workspace.as_str().ok_or("Missing Git workspace")?,
    )
    .await?;
    qualify_git_observations(
        &mut wire,
        directory,
        workspace.as_str().ok_or("Missing Git workspace")?,
    )
    .await?;
    qualify_git_views(
        &mut wire,
        &main,
        directory,
        workspace.as_str().ok_or("Missing Git workspace")?,
        &connected,
    )
    .await?;
    qualify_git_mutations(
        &mut wire,
        &main,
        directory,
        workspace.as_str().ok_or("Missing Git workspace")?,
        &connected,
    )
    .await?;
    qualify_git_pulls(
        &mut wire,
        &main,
        directory,
        workspace.as_str().ok_or("Missing Git workspace")?,
        &connected,
    )
    .await?;
    qualify_previews(
        &mut wire,
        &main,
        directory,
        workspace.as_str().ok_or("Missing preview workspace")?,
        &connected,
    )
    .await?;
    std::fs::write(
        directory.join("project/mcp-search.txt"),
        "🙂 ZAŻÓŁĆ\r\nZażółć\r",
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(directory.join("project/.env.search"), "Zażółć PRIVATE")
        .map_err(|e| e.to_string())?;
    let search_args =
        json!({"workspaceId":workspace,"query":{"text":"zażółć","include":"*.txt"},"limit":1});
    let search_first = wire.tool("lomi_files_search", search_args.clone()).await?;
    let search_data = &search_first["structuredContent"]["data"];
    if search_data["matches"].as_array().map(Vec::len) != Some(1)
        || !search_data["nextCursor"].is_string()
        || search_data["consistency"] != "per_file_snapshot"
    {
        return Err(format!("Project search first page failed: {search_first}"));
    }
    let mut search_next_args = search_args.clone();
    search_next_args["cursor"] = search_data["nextCursor"].clone();
    search_next_args["limit"] = json!(200);
    let search_next = wire
        .tool("lomi_files_search", search_next_args.clone())
        .await?;
    let matches = search_next["structuredContent"]["data"]["matches"]
        .as_array()
        .ok_or("Missing search matches")?;
    if matches.len() != 2
        || matches[0]["relativePath"] != "mcp-search.txt"
        || matches[0]["column"] != 4
        || matches[0]["length"] != 6
        || matches[1]["line"] != 2
        || matches
            .iter()
            .any(|m| m["preview"].as_str().unwrap_or("").contains("PRIVATE"))
    {
        return Err(format!(
            "Project search Unicode/filter mismatch: {search_next}"
        ));
    }
    search_next_args["query"]["caseSensitive"] = json!(true);
    let search_changed = wire.tool("lomi_files_search", search_next_args).await?;
    if search_changed["structuredContent"]["code"] != "CURSOR_EXPIRED" {
        return Err("Search cursor accepted changed query".into());
    }
    let mut foreign_search = search_args;
    foreign_search["workspaceId"] = json!("foreign");
    let foreign_search = wire.tool("lomi_files_search", foreign_search).await?;
    if foreign_search["structuredContent"]["status"] != "error" {
        return Err("Search leaked foreign workspace".into());
    }
    std::fs::write(directory.join("files-search.json"), serde_json::to_vec_pretty(&json!({"first":search_first,"next":search_next,"changedQuery":search_changed,"foreign":foreign_search})).unwrap()).map_err(|e| e.to_string())?;
    let mut rename = Value::Null;
    let mut operation_id = String::new();
    let mut receipt = Value::Null;
    for attempt in 0..3 {
        listed = wire.tool("lomi_workspace_list", json!({})).await?;
        rename = json!({"workspaceId":workspace,"name":"Renamed by MCP","expectedRevision":listed["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("native-rename-{attempt}")});
        let operation = wire.tool("lomi_workspace_update", rename.clone()).await?;
        operation_id = operation["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| operation.to_string())?
            .to_string();
        receipt = wire.settled(&operation_id).await?;
        if receipt["structuredContent"]["data"]["state"] == "succeeded" {
            break;
        }
        if receipt["structuredContent"]["data"]["state"] != "failed"
            || receipt["structuredContent"]["data"]["effectState"] != "none"
            || receipt["structuredContent"]["data"]["result"]["code"] != "REVISION_CONFLICT"
        {
            return Err(format!("Native rename failed: {receipt}"));
        }
    }
    if receipt["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Native rename remained stale: {receipt}"));
    }
    let rename_key = rename["requestKey"].clone();
    let retry = wire.tool("lomi_workspace_update", rename).await?;
    if retry["structuredContent"]["data"]["operationId"] != operation_id
        || retry["structuredContent"]["data"]["state"] != "succeeded"
    {
        return Err("Native rename retry was not deduplicated".into());
    }
    let renamed = wire.tool("lomi_workspace_list", json!({})).await?;
    let lookup=wire.tool("lomi_operation_get",json!({"tool":"lomi_workspace_update","retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":rename_key})).await?;
    if lookup["structuredContent"]["data"]["operationId"] != operation_id {
        return Err("Cannot recover the receipt by request key".into());
    }
    if renamed["structuredContent"]["data"]["items"][0]["name"] != "Renamed by MCP" {
        return Err("Native workspace domain did not change".into());
    }
    let create = json!({"workspaceId":workspace,"cwdRelative":".","title":"MCP terminal fixture","expectedRevision":renamed["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-terminal"});
    let created = wire.tool("lomi_terminal_create", create.clone()).await?;
    let terminal_op = created["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Terminal create rejected: {created}"))?
        .to_string();
    let mut terminal = Value::Null;
    for _ in 0..400 {
        terminal = wire
            .tool("lomi_operation_get", json!({"operationId":terminal_op}))
            .await?;
        let state = &terminal["structuredContent"]["data"]["state"];
        if state != "queued" && state != "running" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if terminal["structuredContent"]["data"]["state"] != "succeeded"
        || !terminal["structuredContent"]["data"]["result"]["leaseId"].is_string()
    {
        return Err(format!("Native terminal failed: {terminal}"));
    }
    let repeated = wire.tool("lomi_terminal_create", create).await?;
    if repeated["structuredContent"]["data"]["operationId"] != terminal_op {
        return Err("Terminal create replayed".into());
    }
    wait_for(&main,"document.body.textContent.includes('MCP terminal fixture') && document.querySelectorAll('.xterm').length === 1").await?;
    let target = &terminal["structuredContent"]["data"]["result"];
    let read_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"maxBytes":65536});
    let mut observed = Value::Null;
    for _ in 0..200 {
        observed = wire.tool("lomi_terminal_read", read_args.clone()).await?;
        if observed["structuredContent"]["data"]["prompt"] == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if observed["structuredContent"]["data"]["prompt"] != "ready" {
        return Err(format!("No native prompt: {observed}"));
    }
    let run_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"command":"printf 'MCP Unicode: Zażółć 🙂\\n'; printf x >> mcp-once.txt","retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-run"});
    let run = wire.tool("lomi_terminal_run", run_args.clone()).await?;
    let run_id = run["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Run rejected: {run}"))?
        .to_string();
    let mut completed = Value::Null;
    for _ in 0..200 {
        completed = wire
            .tool("lomi_operation_get", json!({"operationId":run_id}))
            .await?;
        let state = &completed["structuredContent"]["data"]["state"];
        if state != "queued" && state != "running" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if completed["structuredContent"]["data"]["state"] != "succeeded"
        || completed["structuredContent"]["data"]["result"]["observation"]["exitCode"] != 0
    {
        return Err(format!("Native command did not complete: {completed}"));
    }
    let again = wire.tool("lomi_terminal_run", run_args).await?;
    if again["structuredContent"]["data"]["operationId"] != run_id {
        return Err("Native command was replayed".into());
    }
    let project = listed["structuredContent"]["data"]["items"][0]["projectPath"]
        .as_str()
        .ok_or("Missing fixture path")?;
    if std::fs::read(std::path::Path::new(project).join("mcp-once.txt"))
        .map_err(|e| e.to_string())?
        != b"x"
    {
        return Err("Expected exactly one command side effect".into());
    }
    observed = wire.tool("lomi_terminal_read", read_args.clone()).await?;
    if !observed["structuredContent"]["data"]["text"]
        .as_str()
        .unwrap_or("")
        .contains("MCP Unicode: Zażółć 🙂")
    {
        return Err(format!("Unicode output missing: {observed}"));
    }
    for _ in 0..200 {
        observed = wire.tool("lomi_terminal_read", read_args.clone()).await?;
        if observed["structuredContent"]["data"]["streamSequence"]
            == observed["structuredContent"]["data"]["parsedSequence"]
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if observed["structuredContent"]["data"]["streamSequence"]
        != observed["structuredContent"]["data"]["parsedSequence"]
    {
        return Err("xterm parser watermark did not catch up".into());
    }
    std::fs::write(
        directory.join("terminal-command.json"),
        serde_json::to_vec_pretty(&json!({"receipt":completed,"output":observed})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    screenshot(&main, directory.join("terminal.png")).await?;
    std::fs::write(
        directory.join("terminal.json"),
        serde_json::to_vec_pretty(&terminal).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let target = &terminal["structuredContent"]["data"]["result"];
    let packet = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"inputSequence":"1","input":{"type":"text","text":"printf y >> mcp-input-once.txt\r"}});
    let input = wire.tool("lomi_terminal_input", packet.clone()).await?;
    if input["structuredContent"]["data"]["dispatch"] != "dispatched" {
        return Err(format!("Input rejected: {input}"));
    }
    let repeat = wire.tool("lomi_terminal_input", packet.clone()).await?;
    if input != repeat {
        return Err("Input ACK was not deduplicated".into());
    }
    let mut changed = packet.clone();
    changed["input"]["text"] = json!("different");
    let conflict = wire.tool("lomi_terminal_input", changed).await?;
    if conflict["structuredContent"]["code"] != "IDEMPOTENCY_CONFLICT" {
        return Err("Changed input sequence accepted".into());
    }
    for _ in 0..200 {
        let observed = wire.tool("lomi_terminal_read", read_args.clone()).await?;
        if observed["structuredContent"]["data"]["prompt"] == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if std::fs::read(std::path::Path::new(project).join("mcp-input-once.txt"))
        .map_err(|e| e.to_string())?
        != b"y"
    {
        return Err("Input packet executed more than once".into());
    }
    let quiet_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"command":"sleep 30","retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-silent"});
    let quiet = wire.tool("lomi_terminal_run", quiet_args).await?;
    let quiet_id = quiet["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Silent command rejected: {quiet}"))?;
    tokio::time::sleep(Duration::from_millis(350)).await;
    let still_running = wire
        .tool("lomi_operation_get", json!({"operationId":quiet_id}))
        .await?;
    if still_running["structuredContent"]["data"]["state"] != "running" {
        return Err(format!(
            "Silence falsely completed a command: {still_running}"
        ));
    }
    let mut interrupt = packet.clone();
    interrupt["inputSequence"] = json!("2");
    interrupt["input"] = json!({"type":"key","key":"ctrl_c"});
    let interrupted = wire.tool("lomi_terminal_input", interrupt).await?;
    if interrupted["structuredContent"]["data"]["dispatch"] != "dispatched" {
        return Err(format!("Ctrl+C rejected: {interrupted}"));
    }
    let mut interrupt_observed = false;
    for _ in 0..200 {
        let observed = wire
            .tool("lomi_operation_get", json!({"operationId":quiet_id}))
            .await?;
        if observed["structuredContent"]["data"]["state"] == "failed"
            && observed["structuredContent"]["data"]["result"]["observation"]["exitCode"] == 130
        {
            interrupt_observed = true;
            break;
        }
        if observed["structuredContent"]["data"]["state"] != "running" {
            return Err(format!("Wrong interruption observation: {observed}"));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if !interrupt_observed {
        return Err("Ctrl+C did not produce an observed exit 130".into());
    }
    let quiet_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"command":"sleep 30","retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-targeted-interrupt"});
    let quiet = wire.tool("lomi_terminal_run", quiet_args.clone()).await?;
    let quiet_id = quiet["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or("Missing running operation")?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let interrupt_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"operationId":quiet_id,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-targeted-stop"});
    let stop = wire
        .tool("lomi_terminal_interrupt", interrupt_args.clone())
        .await?;
    let stop_id = stop["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Interrupt rejected: {stop}"))?;
    let stopped = wire.settled(stop_id).await?;
    if stopped["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Interrupt dispatch failed: {stopped}"));
    }
    let stopped = wire.settled(quiet_id).await?;
    if stopped["structuredContent"]["data"]["result"]["observation"]["exitCode"] != 130 {
        return Err(format!("Interrupt did not stop its command: {stopped}"));
    }
    let mut cancellable = quiet_args;
    cancellable["requestKey"] = json!("native-cancel-command");
    let cancellable = wire.tool("lomi_terminal_run", cancellable).await?;
    let cancel_id = cancellable["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or("Missing cancellable operation")?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let replay = wire.tool("lomi_terminal_interrupt", interrupt_args).await?;
    if replay["structuredContent"]["data"]["operationId"] != stop_id {
        return Err("Interrupt retry was replayed".into());
    }
    let running = wire
        .tool("lomi_operation_get", json!({"operationId":cancel_id}))
        .await?;
    if running["structuredContent"]["data"]["state"] != "running" {
        return Err("Old interrupt affected a new command".into());
    }
    wire.tool("lomi_operation_cancel", json!({"operationId":cancel_id}))
        .await?;
    let cancelled = wire.settled(cancel_id).await?;
    if cancelled["structuredContent"]["data"]["state"] != "cancelled"
        || cancelled["structuredContent"]["data"]["result"]["observation"]["exitCode"] != 130
    {
        return Err(format!("Operation cancellation failed: {cancelled}"));
    }
    let command_read = wire.tool("lomi_terminal_read", json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"mode":"command","operationId":run_id})).await?;
    let command_text = command_read["structuredContent"]["data"]["text"]
        .as_str()
        .ok_or("Missing command output")?;
    if !command_text.contains("MCP Unicode: Zażółć 🙂") || command_text.contains("sleep 30") {
        return Err("Command block included later terminal output".into());
    }
    let alternate_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":target["leaseId"],"command":r"printf '\033[?1049h\033[2J\033[HAlternate MCP 🙂'; sleep 30","retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-alternate-screen"});
    let alternate = wire
        .tool("lomi_terminal_run", alternate_args.clone())
        .await?;
    let alternate_id = alternate["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Alternate command rejected: {alternate}"))?;
    let mut screen = Value::Null;
    for _ in 0..100 {
        screen = wire.tool("lomi_terminal_read", json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"mode":"screen"})).await?;
        let data = &screen["structuredContent"]["data"];
        if data["buffer"] == "alternate"
            && data["parserPending"] == false
            && data["text"]
                .as_str()
                .unwrap_or("")
                .contains("Alternate MCP 🙂")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if screen["structuredContent"]["data"]["buffer"] != "alternate"
        || !screen["structuredContent"]["data"]["text"]
            .as_str()
            .unwrap_or("")
            .contains("Alternate MCP 🙂")
    {
        return Err(format!("Retained alternate screen read failed: {screen}"));
    }
    std::fs::write(
        directory.join("terminal-screen.json"),
        serde_json::to_vec_pretty(&screen).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    screenshot(&main, directory.join("terminal-alternate.png")).await?;
    let raw = wire.tool("lomi_terminal_read", json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"mode":"raw","maxBytes":65536})).await?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(
            raw["structuredContent"]["data"]["base64"]
                .as_str()
                .ok_or("Missing raw bytes")?,
        )
        .map_err(|e| e.to_string())?;
    if !bytes.windows(8).any(|b| b == b"\x1b[?1049h") {
        return Err("Raw read lost actual ANSI bytes".into());
    }
    wire.tool("lomi_operation_cancel", json!({"operationId":alternate_id}))
        .await?;
    let cancelled = wire.settled(alternate_id).await?;
    if cancelled["structuredContent"]["data"]["result"]["observation"]["exitCode"] != 130 {
        return Err(format!("Alternate command did not stop: {cancelled}"));
    }
    let mut restore = alternate_args;
    restore["command"] = json!(r"printf '\033[?1049l'");
    restore["requestKey"] = json!("native-restore-screen");
    let restore = wire.tool("lomi_terminal_run", restore).await?;
    let restored = wire
        .settled(
            restore["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or("Restore rejected")?,
        )
        .await?;
    if restored["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Normal screen restoration failed: {restored}"));
    }
    javascript(&main,&format!("await window.__TAURI_INTERNALS__.invoke('write_terminal',{{id:{},data:'human-owned'}});return true;",serde_json::to_string(&target["terminalSessionId"]).unwrap())).await?;
    let mut after_human = packet;
    after_human["inputSequence"] = json!("3");
    let refused = wire.tool("lomi_terminal_input", after_human).await?;
    if refused["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err(format!("Human takeover failed: {refused}"));
    }
    let after_human = wire.tool("lomi_terminal_read", read_args).await?;
    if !after_human["structuredContent"]["data"]["leaseId"].is_null() {
        return Err("Human takeover retained the lease".into());
    }
    let panels = wire
        .tool(
            "lomi_panel_list",
            json!({"workspaceId":workspace,"limit":1}),
        )
        .await?;
    let panel_cursor = panels["structuredContent"]["data"]["nextCursor"]
        .as_str()
        .ok_or("Missing panel cursor")?;
    let second = wire
        .tool(
            "lomi_panel_list",
            json!({"workspaceId":workspace,"limit":1,"cursor":panel_cursor}),
        )
        .await?;
    let mut listed_panels = panels["structuredContent"]["data"]["items"]
        .as_array()
        .ok_or("Missing first panel page")?
        .clone();
    let mut page = second;
    for _ in 0..100 {
        listed_panels.extend(
            page["structuredContent"]["data"]["items"]
                .as_array()
                .ok_or_else(|| page.to_string())?
                .iter()
                .cloned(),
        );
        let Some(cursor) = page["structuredContent"]["data"]["nextCursor"].as_str() else {
            break;
        };
        page = wire
            .tool(
                "lomi_panel_list",
                json!({"workspaceId":workspace,"limit":1,"cursor":cursor}),
            )
            .await?;
    }
    if page["structuredContent"]["data"]["nextCursor"].is_string() {
        return Err("Panel pagination exceeded the fixture bound".into());
    }
    let ids: std::collections::HashSet<_> =
        listed_panels.iter().map(|p| p["id"].as_str()).collect();
    if ids.len() != listed_panels.len() {
        return Err("Panel pagination duplicated an identity".into());
    }
    let panel = listed_panels
        .iter()
        .find(|p| p["id"] == target["panelId"])
        .ok_or("Owned terminal missing from panel pages")?;
    if panel["id"] != target["panelId"]
        || panel["terminalSessionId"] != target["terminalSessionId"]
        || panel["ownership"] != "this_session"
        || panel["inputControlled"] != false
    {
        return Err(format!("Wrong native panel identity: {panel}"));
    }
    let wrong_cursor = wire
        .tool("lomi_workspace_list", json!({"cursor":panel_cursor}))
        .await?;
    if wrong_cursor["structuredContent"]["code"] != "CURSOR_EXPIRED" {
        return Err("Panel cursor accepted as workspace cursor".into());
    }
    let list = wire.tool("lomi_workspace_list", json!({})).await?;
    let create_workspace = json!({"workspaceId":workspace,"name":"MCP scratch workspace","expectedRevision":list["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-workspace"});
    let created = wire
        .tool("lomi_workspace_create", create_workspace.clone())
        .await?;
    let created_id = created["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Workspace create rejected: {created}"))?;
    let created = wire.settled(created_id).await?;
    if created["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Workspace creation failed: {created}"));
    }
    let new_workspace = &created["structuredContent"]["data"]["result"]["workspaceId"];
    let repeat = wire.tool("lomi_workspace_create", create_workspace).await?;
    if repeat["structuredContent"]["data"]["operationId"]
        != created["structuredContent"]["data"]["operationId"]
    {
        return Err("Workspace create replayed".into());
    }
    let new_panels = wire
        .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
        .await?;
    let new_panels = new_panels["structuredContent"]["data"]["items"]
        .as_array()
        .ok_or("Created workspace is not authorized")?;
    if new_panels.len() != 1 || new_panels[0]["kind"] != "file" {
        return Err("Workspace did not start with one scratch file".into());
    }
    let contexts = javascript(
        &main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    if contexts.as_object().is_none_or(|map| {
        map.len() != 1 || !map.contains_key(target["terminalSessionId"].as_str().unwrap())
    }) {
        return Err("Workspace creation started or replaced a terminal".into());
    }
    let mut focus_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"]});
    for attempt in 0..3 {
        let listed = wire.tool("lomi_workspace_list", json!({})).await?;
        focus_args["expectedRevision"] =
            listed["structuredContent"]["data"]["domainRevision"].clone();
        focus_args["requestKey"] = json!(format!("native-focus-{attempt}"));
        let focused = wire.tool("lomi_panel_focus", focus_args.clone()).await?;
        let focused_id = focused["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| format!("Focus rejected: {focused}"))?;
        let focused = wire.settled(focused_id).await?;
        if focused["structuredContent"]["data"]["state"] == "succeeded" {
            break;
        }
        // A real terminal title update can race the optimistic layout revision.
        // Retry a newly inspected revision only after a proven no-effect failure.
        if attempt == 2
            || focused["structuredContent"]["data"]["effectState"] != "none"
            || focused["structuredContent"]["data"]["result"]["code"] != "REVISION_CONFLICT"
        {
            return Err(format!("Focus failed: {focused}"));
        }
    }
    wait_for(&main,"document.querySelectorAll('.xterm').length===1 && document.body.textContent.includes('MCP terminal fixture')").await?;
    let mut human_close = focus_args;
    human_close["requestKey"] = json!("human-owned-close");
    let human_close = wire.tool("lomi_panel_close", human_close).await?;
    if human_close["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err(format!("Human-owned terminal was closable: {human_close}"));
    }
    let mut claim_args = json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"action":"claim","retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-claim-denied"});
    let pending = wire.tool("lomi_panel_control", claim_args.clone()).await?;
    let pending_id = pending["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Claim rejected: {pending}"))?;
    if pending["structuredContent"]["data"]["state"] != "awaiting_user" {
        return Err("Claim did not require explicit approval".into());
    }
    let again = wire.tool("lomi_panel_control", claim_args.clone()).await?;
    if again["structuredContent"]["data"]["operationId"] != pending_id {
        return Err("Claim duplicated its approval request".into());
    }
    let denied = javascript(&main, &format!("try {{await window.__TAURI_INTERNALS__.invoke('agent_control_decide_terminal',{{operationId:{},approve:true}});return false;}} catch{{return true;}}",serde_json::to_string(pending_id).unwrap())).await?;
    if denied != true {
        return Err("Main approved terminal takeover".into());
    }
    wait_for(
        &settings,
        "document.body.textContent.includes('Terminal input requests')",
    )
    .await?;
    click(&settings, "Deny input").await?;
    let denied = wire.settled(pending_id).await?;
    if denied["structuredContent"]["data"]["state"] != "cancelled" {
        return Err("Denied claim remained active".into());
    }
    claim_args["requestKey"] = json!("native-claim-approved");
    let pending = wire.tool("lomi_panel_control", claim_args.clone()).await?;
    let pending_id = pending["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or("Missing claim operation")?;
    wait_for(
        &settings,
        "document.body.textContent.includes('Terminal input requests')",
    )
    .await?;
    evaluate(
        &settings,
        "document.querySelector('[data-control-operation]').scrollIntoView({block:'center'});true",
    )
    .await?;
    screenshot(&settings, directory.join("terminal-approval.png")).await?;
    click(&settings, "Allow input").await?;
    let granted = wire.settled(pending_id).await?;
    let lease = granted["structuredContent"]["data"]["result"]["leaseId"]
        .as_str()
        .ok_or_else(|| format!("Terminal claim failed: {granted}"))?;
    if lease == target["leaseId"] {
        return Err("Takeover restored an invalidated lease".into());
    }
    let observed = wire.tool("lomi_terminal_read",json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"mode":"screen"})).await?;
    if observed["structuredContent"]["data"]["kind"] != "terminal_screen" {
        return Err(format!("Reattached screen sequence lost: {observed}"));
    }
    let entered = wire.tool("lomi_terminal_input",json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":lease,"inputSequence":"1","input":{"type":"key","key":"enter"}})).await?;
    if entered["structuredContent"]["data"]["dispatch"] != "dispatched" {
        return Err(format!("Approved input failed: {entered}"));
    }
    let mut released = claim_args;
    released["action"] = json!("release");
    released["requestKey"] = json!("native-release");
    let released = wire.tool("lomi_panel_control", released).await?;
    if released["structuredContent"]["data"]["state"] != "succeeded"
        || !released["structuredContent"]["data"]["result"]["leaseId"].is_null()
    {
        return Err(format!("Input release failed: {released}"));
    }
    let refused=wire.tool("lomi_terminal_input",json!({"workspaceId":workspace,"panelId":target["panelId"],"terminalSessionId":target["terminalSessionId"],"leaseId":lease,"inputSequence":"2","input":{"type":"text","text":"forbidden"}})).await?;
    if refused["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err("Released lease accepted input".into());
    }
    javascript(&main,&format!("await window.__TAURI_INTERNALS__.invoke('write_terminal',{{id:{},data:' '}});return true;",target["terminalSessionId"])).await?;
    let current_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let close_file = json!({"workspaceId":new_workspace,"panelId":new_panels[0]["id"],"terminalSessionId":null,"expectedRevision":current_revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"close-scratch"});
    let closed = wire.tool("lomi_panel_close", close_file.clone()).await?;
    let closed_id = closed["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Scratch close rejected: {closed}"))?;
    let closed = wire.settled(closed_id).await?;
    if closed["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Scratch close failed: {closed}"));
    }
    let retry = wire.tool("lomi_panel_close", close_file).await?;
    if retry["structuredContent"]["data"]["operationId"] != closed_id {
        return Err("Closed panel receipt was lost on retry".into());
    }
    let current_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let disposable=wire.tool("lomi_terminal_create",json!({"workspaceId":new_workspace,"cwdRelative":".","title":"Disposable MCP terminal","expectedRevision":current_revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"disposable-terminal"})).await?;
    let disposable_id = disposable["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Disposable terminal rejected: {disposable}"))?;
    let disposable = wire.settled(disposable_id).await?;
    let disposable = &disposable["structuredContent"]["data"]["result"];
    for _ in 0..200 {
        let read=wire.tool("lomi_terminal_read",json!({"workspaceId":new_workspace,"panelId":disposable["panelId"],"terminalSessionId":disposable["terminalSessionId"]})).await?;
        let data = &read["structuredContent"]["data"];
        if data["prompt"] == "ready" && data["streamSequence"] == data["parsedSequence"] {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let current_revision = wire.tool("lomi_workspace_list", json!({})).await?;
    let close_args = json!({"workspaceId":new_workspace,"panelId":disposable["panelId"],"terminalSessionId":disposable["terminalSessionId"],"expectedRevision":current_revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"close-disposable"});
    let closed = wire.tool("lomi_panel_close", close_args.clone()).await?;
    let closed_id = closed["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Terminal close rejected: {closed}"))?;
    let settled = wire.settled(closed_id).await?;
    if settled["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Terminal close failed: {settled}"));
    }
    let retry = wire.tool("lomi_panel_close", close_args).await?;
    if retry["structuredContent"]["data"]["operationId"] != closed_id {
        return Err("Terminal close retry lost its receipt".into());
    }
    let contexts = javascript(
        &main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    if contexts.as_object().is_none_or(|map| {
        map.len() != 1 || !map.contains_key(target["terminalSessionId"].as_str().unwrap())
    }) {
        return Err(
            "Panel close affected the wrong terminal or started a replacement shell".into(),
        );
    }
    // Start an ordinary user terminal through the actual Workbench menu.
    evaluate(
        &main,
        "document.querySelector('button[title^=\"New tab\"]').click();true",
    )
    .await?;
    click(&main, "New terminal").await?;
    let mut human_panel = Value::Null;
    for _ in 0..100 {
        let panels = wire
            .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
            .await?;
        if let Some(panel) = panels["structuredContent"]["data"]["items"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|p| p["kind"] == "terminal" && p["terminalSessionId"].is_string())
            })
        {
            human_panel = panel.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if human_panel["ownership"] != "human_or_unassigned" {
        return Err(format!(
            "User terminal was assigned without approval: {human_panel}"
        ));
    }
    // A PTY identity is published before the user's shell startup completes.
    // Observe the parsed prompt before sending this fixture's human input.
    let mut prompt_ready = false;
    for _ in 0..100 {
        let ready = javascript(&main, &format!("const m=await import('/src/terminal-runtime.ts');return Boolean(m.runningTerminal({})?.promptEnd);", human_panel["id"])).await?;
        if ready == true {
            prompt_ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !prompt_ready {
        return Err("Ordinary terminal shell did not reach its initial parsed prompt".into());
    }
    javascript(&main,&format!("await window.__TAURI_INTERNALS__.invoke('write_terminal',{{id:{},data: \"printf 'USER RETAINED %s\\\\n' 'PTY'\\r\"}});return true;",human_panel["terminalSessionId"])).await?;
    let mut retained_output = false;
    for _ in 0..100 {
        let screen=javascript(&main,&format!("const runtime=await import('/src/terminal-runtime.ts');return runtime.runningTerminal({})?.controlScreen(65536)?.text ?? '';",human_panel["id"])).await?;
        if screen.as_str().unwrap_or("").contains("USER RETAINED PTY") {
            retained_output = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if !retained_output {
        return Err(
            "Ordinary terminal did not produce its own output before MCP attachment".into(),
        );
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let original_processes = app.state::<crate::terminal::Terminals>().smoke_sessions();
    let claim=wire.tool("lomi_panel_control",json!({"workspaceId":new_workspace,"panelId":human_panel["id"],"terminalSessionId":human_panel["terminalSessionId"],"action":"claim","retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-user-terminal"})).await?;
    let claim_id = claim["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Human claim rejected: {claim}"))?;
    if claim["structuredContent"]["data"]["state"] != "awaiting_user" {
        return Err("Human terminal was silently claimed".into());
    }
    wait_for(
        &settings,
        "document.body.textContent.includes('Terminal input requests')",
    )
    .await?;
    click(&settings, "Allow input").await?;
    let claimed = wire.settled(claim_id).await?;
    let lease = claimed["structuredContent"]["data"]["result"]["leaseId"]
        .as_str()
        .ok_or_else(|| format!("Human terminal attach failed: {claimed}"))?;
    let screen=wire.tool("lomi_terminal_read",json!({"workspaceId":new_workspace,"panelId":human_panel["id"],"terminalSessionId":human_panel["terminalSessionId"],"mode":"screen"})).await?;
    if screen["structuredContent"]["data"]["kind"] != "terminal_screen" {
        return Err(format!(
            "Human terminal screen/generation changed: {screen}"
        ));
    }
    if !screen["structuredContent"]["data"]["text"]
        .as_str()
        .unwrap_or("")
        .contains("USER RETAINED PTY")
        || screen["structuredContent"]["data"]["streamSequence"] == "0"
    {
        return Err("Attachment lost the existing parsed screen".into());
    }
    let processes = app.state::<crate::terminal::Terminals>().smoke_sessions();
    if processes != original_processes {
        return Err("Claim restarted or replaced a terminal process".into());
    }
    wait_for(
        &main,
        "document.body.textContent.includes('Agent input · Take control')",
    )
    .await?;
    app.get_window("settings")
        .ok_or("Missing settings window")?
        .hide()
        .map_err(|e| e.to_string())?;
    app.show().map_err(|e| e.to_string())?;
    let main_window = app.get_window("main").ok_or("Missing main window")?;
    main_window.show().map_err(|e| e.to_string())?;
    main_window.unminimize().map_err(|e| e.to_string())?;
    main_window.set_focus().map_err(|e| e.to_string())?;
    main.set_focus().map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_millis(350)).await;
    let painted=javascript(&main,"const surfaces=[...document.querySelectorAll('.terminal-host, .xterm-screen, .xterm-screen canvas')].map(e=>({tag:e.tagName,className:e.className,rect:JSON.stringify(e.getBoundingClientRect()),display:getComputedStyle(e).display,opacity:getComputedStyle(e).opacity,visibility:getComputedStyle(e).visibility}));const m=await import('/src/terminal-runtime.ts');const r=m.runningTerminal(document.querySelector('[data-pane-id]').dataset.paneId);const frame=await Promise.race([new Promise(resolve=>requestAnimationFrame(()=>resolve(true))),new Promise(resolve=>setTimeout(()=>resolve(false),1500))]);return {surfaces,frame,visibility:document.visibilityState,focus:document.hasFocus(),ready:r.rendererReady,received:r.receivedOutput,attached:r.attached,paused:r.terminal._core._renderService._isPaused,refresh:r.terminal._core._renderService._needsFullRefresh};").await?;
    std::fs::write(
        directory.join("terminal-user-screen.json"),
        serde_json::to_vec_pretty(&json!({"screen":screen,"surfaces":painted})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    if painted["frame"] == true {
        wait_for(&main, "[...document.querySelectorAll('.terminal-host')].some(e => getComputedStyle(e).opacity === '1' && e.getBoundingClientRect().width > 0)").await?;
    }
    screenshot(&main, directory.join("terminal-ownership.png")).await?;
    app.get_window("settings")
        .ok_or("Missing settings window")?
        .show()
        .map_err(|e| e.to_string())?;
    click(&main, "Agent input · Take control").await?;
    wait_for(
        &main,
        "!document.body.textContent.includes('Agent input · Take control')",
    )
    .await?;
    let refused=wire.tool("lomi_terminal_input",json!({"workspaceId":new_workspace,"panelId":human_panel["id"],"terminalSessionId":human_panel["terminalSessionId"],"leaseId":lease,"inputSequence":"1","input":{"type":"text","text":"must not type"}})).await?;
    if refused["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err("Take control button left agent input enabled".into());
    }
    evaluate(
        &main,
        &format!(
            "document.querySelector('[data-tab-id=\"{}\"] .tab-close').click();true",
            human_panel["tabId"].as_str().unwrap()
        ),
    )
    .await?;
    wait_for(
        &main,
        &format!(
            "!document.querySelector('[data-tab-id=\"{}\"]')",
            human_panel["tabId"].as_str().unwrap()
        ),
    )
    .await?;
    let server_revision = wire
        .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
        .await?;
    let server_created = wire.tool("lomi_terminal_create", json!({"workspaceId":new_workspace,"cwdRelative":".","title":"MCP browser dev server","expectedRevision":server_revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"browser-server-terminal"})).await?;
    let server_terminal = wire
        .settled(
            server_created["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| server_created.to_string())?,
        )
        .await?;
    let server_target = &server_terminal["structuredContent"]["data"]["result"];
    if server_terminal["structuredContent"]["data"]["state"] != "succeeded"
        || !server_target["leaseId"].is_string()
    {
        return Err(format!("Server terminal failed: {server_terminal}"));
    }
    let server_read_args = json!({"workspaceId":new_workspace,"panelId":server_target["panelId"],"terminalSessionId":server_target["terminalSessionId"],"maxBytes":16384});
    let mut server_prompt = Value::Null;
    for _ in 0..200 {
        server_prompt = wire
            .tool("lomi_terminal_read", server_read_args.clone())
            .await?;
        if server_prompt["structuredContent"]["data"]["prompt"] == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if server_prompt["structuredContent"]["data"]["prompt"] != "ready" {
        return Err("Dev-server PTY never reached a qualified prompt".into());
    }
    let server_run_args = json!({"workspaceId":new_workspace,"panelId":server_target["panelId"],"terminalSessionId":server_target["terminalSessionId"],"leaseId":server_target["leaseId"],"command":browser_fixture["command"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"start-browser-fixture-server"});
    let server_run = wire
        .tool("lomi_terminal_run", server_run_args.clone())
        .await?;
    let server_operation = server_run["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| server_run.to_string())?;
    let mut command_read_args = server_read_args.clone();
    command_read_args["mode"] = json!("command");
    command_read_args["operationId"] = json!(server_operation);
    let mut server_output = Value::Null;
    let mut observed_origin = None;
    for _ in 0..200 {
        server_output = wire
            .tool("lomi_terminal_read", command_read_args.clone())
            .await?;
        observed_origin = server_output["structuredContent"]["data"]["text"]
            .as_str()
            .and_then(|text| {
                text.lines().find_map(|line| {
                    line.strip_prefix("LOMI_FIXTURE_READY ")
                        .map(|s| s.trim().to_owned())
                })
            });
        if observed_origin.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let observed_origin = observed_origin
        .ok_or_else(|| format!("No dev-server address in its command output: {server_output}"))?;
    if observed_origin != browser_fixture["origin"]
        || server_output["structuredContent"]["data"]["command"]["operationId"] != server_operation
    {
        return Err("Dev-server URL lacked its terminal operation provenance".into());
    }
    let server_status = wire
        .tool(
            "lomi_operation_get",
            json!({"operationId":server_operation}),
        )
        .await?;
    if server_status["structuredContent"]["data"]["state"] != "running" {
        return Err("Ready dev server was incorrectly completed".into());
    }
    let repeated_server = wire
        .tool("lomi_terminal_run", server_run_args.clone())
        .await?;
    if repeated_server["structuredContent"]["data"]["operationId"] != server_operation {
        return Err("Dev-server retry started a second process".into());
    }
    std::fs::write(directory.join("dev-server-provenance.json"), serde_json::to_vec_pretty(&json!({"workspaceId":new_workspace,"terminal":server_target,"operationId":server_operation,"observedOrigin":observed_origin,"source":"command_output","confidence":"reported_by_owned_command; confirmed by native browser response below","output":server_output,"status":server_status})).unwrap()).map_err(|e|e.to_string())?;
    let mut workspace_selections = Vec::new();
    for (name, destination) in [
        ("previous", workspace.as_str().unwrap()),
        ("dev-server", new_workspace.as_str().unwrap()),
    ] {
        let current = wire.tool("lomi_workspace_list", json!({})).await?;
        let args = json!({"action":"select","workspaceId":destination,"expectedRevision":current["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("select-workspace-{name}")});
        let selected = wire.tool("lomi_workspace_update", args.clone()).await?;
        let operation = selected["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| selected.to_string())?;
        let selected = wire.settled(operation).await?;
        if selected["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Workspace selection failed: {selected}"));
        }
        let retry = wire.tool("lomi_workspace_update", args).await?;
        if retry["structuredContent"]["data"]["operationId"] != operation {
            return Err("Workspace selection repeated its effect".into());
        }
        workspace_selections.push(selected);
    }
    let retained_server = wire
        .tool("lomi_terminal_read", command_read_args.clone())
        .await?;
    if retained_server["structuredContent"]["data"]["command"]["operationId"] != server_operation
        || retained_server["structuredContent"]["data"]["terminalSessionId"]
            != server_target["terminalSessionId"]
    {
        return Err("Workspace selection lost its retained dev server".into());
    }
    std::fs::write(
        directory.join("workspace-selection.json"),
        serde_json::to_vec_pretty(
            &json!({"selections":workspace_selections,"retainedServer":retained_server}),
        )
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let browser_revision = wire
        .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
        .await?;
    let browser_args = json!({"workspaceId":new_workspace,"url":observed_origin,"expectedRevision":browser_revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-browser-open"});
    let opened = wire.tool("lomi_browser_open", browser_args.clone()).await?;
    let browser_operation = opened["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Browser open rejected: {opened}"))?;
    let ready = wire.settled(browser_operation).await?;
    if ready["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Browser did not become ready: {ready}"));
    }
    let browser_target = &ready["structuredContent"]["data"]["result"];
    if browser_target["kind"] != "browser"
        || browser_target["ready"] != true
        || browser_target["engine"] != "WKWebView"
        || browser_target["networkIsolation"] != "none"
        || !browser_target["leaseId"].is_string()
    {
        return Err(format!("Invalid native browser result: {ready}"));
    }
    let browser = app
        .get_webview(&format!(
            "browser-{}",
            browser_target["panelId"].as_str().unwrap()
        ))
        .ok_or("No actual child browser")?;
    wait_for(
        &browser,
        "Boolean(window.fixtureReady && document.querySelector('#save'))",
    )
    .await?;
    wait_for(
        &browser,
        "Boolean(window.fixtureMedia && window.fixtureMedia !== 'pending')",
    )
    .await?;
    let media_status = evaluate(&browser, "window.fixtureMedia").await?;
    if media_status != "unavailable"
        && (media_status != "NotAllowedError"
            || crate::browser::agent_permissions::media_denials() == 0)
    {
        return Err("Automation media request was not denied by the native policy".into());
    }
    std::fs::write(directory.join("browser-permissions.json"),serde_json::to_vec_pretty(&json!({"delegateAttached":true,"mediaApi":media_status,"nativeMediaDenials":crate::browser::agent_permissions::media_denials()})).unwrap()).map_err(|e|e.to_string())?;
    let snapshot_args = json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"maxNodes":100,"maxBytes":16384});
    let snapshot = wire
        .tool("lomi_browser_snapshot", snapshot_args.clone())
        .await?;
    let snap = &snapshot["structuredContent"]["data"];
    if snap["kind"] != "browser_snapshot"
        || snap["snapshotKind"] != "dom"
        || !snap["elements"].as_array().is_some_and(|elements| {
            elements
                .iter()
                .any(|e| e["role"] == "button" && e["name"] == "Save")
        })
    {
        return Err(format!("Native DOM snapshot failed: {snapshot}"));
    }
    std::fs::write(
        directory.join("browser-snapshot.json"),
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    if snapshot.to_string().contains("fixture-password")
        || snapshot.to_string().contains("fixture-hidden")
        || snapshot.to_string().contains("Editable fixture")
    {
        return Err("Snapshot disclosed private form values".into());
    }
    let reference = |role: &str, name: &str| -> Result<Value, String> {
        snap["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["role"] == role && e["name"] == name)
            .map(|e| e["elementRef"].clone())
            .ok_or_else(|| format!("No {role} {name} reference"))
    };
    evaluate(
        &browser,
        "window.__lomiAgentDomV1={snapshot:'forged',refs:new Map()};true",
    )
    .await?;
    let action_args = json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"navigationId":snap["navigationId"],"snapshotId":snap["snapshotId"],"elementRef":reference("button","Save")?,"leaseId":browser_target["leaseId"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"browser-form-error"});
    let click_empty = wire.tool("lomi_browser_click", action_args.clone()).await?;
    let click_empty = wire
        .settled(
            click_empty["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| click_empty.to_string())?,
        )
        .await?;
    if click_empty["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Click failed: {click_empty}"));
    }
    wait_for(
        &browser,
        "document.querySelector('#result').textContent==='Name is required'",
    )
    .await?;
    let mut fill_args = action_args.clone();
    fill_args["elementRef"] = reference("textbox", "Name")?;
    fill_args["text"] = json!(r#"Zażółć 🙂 ' \ ${no_code}"#);
    fill_args["requestKey"] = json!("browser-fill-unicode");
    let filled = wire.tool("lomi_browser_fill", fill_args.clone()).await?;
    let filled = wire
        .settled(
            filled["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| filled.to_string())?,
        )
        .await?;
    if filled["structuredContent"]["data"]["state"] != "succeeded"
        || filled.to_string().contains("Zażółć")
    {
        return Err(format!("Fill failed or returned private text: {filled}"));
    }
    let mut key_args = fill_args.clone();
    key_args.as_object_mut().unwrap().remove("text");
    key_args["key"] = json!("Enter");
    key_args["requestKey"] = json!("browser-key-enter");
    let keyed = wire.tool("lomi_browser_key", key_args.clone()).await?;
    let keyed_id = keyed["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| keyed.to_string())?;
    let keyed_result = wire.settled(keyed_id).await?;
    if keyed_result["structuredContent"]["data"]["state"] != "succeeded"
        || keyed_result["structuredContent"]["data"]["result"]["defaultAction"] != false
    {
        return Err(format!("Synthetic key failed: {keyed_result}"));
    }
    let keyed_retry = wire.tool("lomi_browser_key", key_args.clone()).await?;
    if keyed_retry["structuredContent"]["data"]["operationId"] != keyed_id
        || evaluate(&browser, "window.fixtureKeys").await?
            != json!([{"key":"Enter","trusted":false}])
    {
        return Err("Keyboard input repeated or missed its focused target".into());
    }
    key_args["elementRef"] = reference("button", "Save")?;
    key_args["requestKey"] = json!("browser-key-wrong-focus");
    let wrong_focus = wire.tool("lomi_browser_key", key_args).await?;
    let wrong_focus = wire
        .settled(
            wrong_focus["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| wrong_focus.to_string())?,
        )
        .await?;
    if wrong_focus["structuredContent"]["data"]["state"] != "failed"
        || wrong_focus["structuredContent"]["data"]["result"]["code"] != "TARGET_BUSY"
    {
        return Err(format!(
            "Keyboard accepted a different focused target: {wrong_focus}"
        ));
    }
    let mut save_args = action_args.clone();
    save_args["requestKey"] = json!("browser-save-unicode");
    let saved = wire.tool("lomi_browser_click", save_args.clone()).await?;
    let saved_op = saved["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| saved.to_string())?;
    let saved = wire.settled(saved_op).await?;
    if saved["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Save failed: {saved}"));
    }
    wait_for(
        &browser,
        "document.querySelector('#result').textContent.startsWith('Saved: Zażółć 🙂')",
    )
    .await?;
    let saved_retry = wire.tool("lomi_browser_click", save_args).await?;
    if saved_retry["structuredContent"]["data"]["operationId"] != saved_op
        || evaluate(&browser, "window.fixtureSaveCount").await? != 2
    {
        return Err("Browser retry clicked twice".into());
    }
    if evaluate(&browser, "window.fixtureTrusted").await? != false {
        return Err("Synthetic input reported trusted".into());
    }
    for (role, name, text, selector) in [
        ("combobox", "Choice", "b", "#choice"),
        (
            "textbox",
            "Editable field",
            "Editable Zażółć 🙂",
            "#editable",
        ),
    ] {
        let mut args = action_args.clone();
        args["elementRef"] = reference(role, name)?;
        args["text"] = json!(text);
        args["requestKey"] = json!(format!("fill-{role}"));
        let changed = wire.tool("lomi_browser_fill", args).await?;
        let changed = wire
            .settled(
                changed["structuredContent"]["data"]["operationId"]
                    .as_str()
                    .ok_or_else(|| changed.to_string())?,
            )
            .await?;
        if changed["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Control fill failed: {changed}"));
        }
        let observed = evaluate(
            &browser,
            &format!(
                "(()=>{{const e=document.querySelector({});return e.value ?? e.textContent;}})()",
                json!(selector)
            ),
        )
        .await?;
        if observed != text {
            return Err("Native control retained a different value".into());
        }
    }
    let after = wire
        .tool("lomi_browser_snapshot", snapshot_args.clone())
        .await?;
    if !after.to_string().contains("Saved: Zażółć") || after.to_string().contains("Editable Zażółć")
    {
        return Err(format!("Form result not present in snapshot: {after}"));
    }
    fill_args["requestKey"] = json!("stale-browser-snapshot");
    let stale = wire.tool("lomi_browser_fill", fill_args).await?;
    if stale["structuredContent"]["code"] != "STALE_SNAPSHOT" {
        return Err(format!("Old snapshot accepted: {stale}"));
    }
    std::fs::write(
        directory.join("browser-form-result.json"),
        serde_json::to_vec_pretty(&after).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    evaluate(&browser, "(()=>{window.__lomiAgentLogsV1={entries:[{message:'page-forged-log'}]};dispatchEvent(new ErrorEvent('error',{message:'synthetic-forged-log'}));setTimeout(()=>{throw new Error('native-mcp-error')},0);Promise.reject('native-mcp-rejection');setTimeout(()=>{throw new Error('unicode-edge-'+String.fromCharCode(0xd800))},0);return true})()").await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let logs_args = json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"limit":64});
    let logs = wire.tool("lomi_browser_logs", logs_args.clone()).await?;
    let log_data = &logs["structuredContent"]["data"];
    if log_data["kind"] != "browser_logs"
        || !logs.to_string().contains("native-mcp-error")
        || logs.to_string().contains("native-mcp-rejection")
        || logs.to_string().contains("forged-log")
        || !logs.to_string().contains("unicode-edge-")
    {
        return Err(format!("Isolated native browser logs failed: {logs}"));
    }
    let mut cursor_args = logs_args.clone();
    cursor_args["cursor"] = log_data["nextCursor"].clone();
    let empty_logs = wire.tool("lomi_browser_logs", cursor_args.clone()).await?;
    if empty_logs["structuredContent"]["data"]["entries"]
        .as_array()
        .is_none_or(|e| !e.is_empty())
    {
        return Err(format!("Log cursor replayed entries: {empty_logs}"));
    }
    evaluate(&browser, "(()=>{for(let i=0;i<80;i++)setTimeout(()=>{throw new Error('bounded-error-'+i+'x'.repeat(10000))},0);return true})()").await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let mut page_logs = logs_args.clone();
    page_logs["limit"] = json!(10);
    let bounded_logs = wire.tool("lomi_browser_logs", page_logs.clone()).await?;
    let bounded_data = &bounded_logs["structuredContent"]["data"];
    if bounded_data["entries"].as_array().map(|v| v.len()) != Some(10)
        || bounded_data["hasMore"] != true
        || bounded_data["dropped"].as_u64().unwrap_or(0) < 16
        || bounded_logs.to_string().len() > 12000
    {
        return Err(format!("Browser log budget not enforced: {bounded_logs}"));
    }
    page_logs["cursor"] = bounded_data["nextCursor"].clone();
    let second_logs = wire.tool("lomi_browser_logs", page_logs).await?;
    if second_logs["structuredContent"]["data"]["entries"][0]["sequence"].as_u64()
        <= bounded_data["entries"][9]["sequence"].as_u64()
    {
        return Err("Browser log pagination repeated a sequence".into());
    }
    let mut invalid_logs = logs_args.clone();
    invalid_logs["cursor"] = json!("another-generation:1");
    if wire.tool("lomi_browser_logs", invalid_logs).await?["structuredContent"]["code"]
        != "CURSOR_EXPIRED"
    {
        return Err("Foreign log cursor accepted".into());
    }
    let mut foreign_logs = logs_args.clone();
    foreign_logs["workspaceId"] = json!("foreign-workspace");
    if wire.tool("lomi_browser_logs", foreign_logs).await?["structuredContent"]["code"]
        != "TARGET_NOT_FOUND"
    {
        return Err("Foreign workspace disclosed browser logs".into());
    }
    std::fs::write(
        directory.join("browser-logs.json"),
        serde_json::to_vec_pretty(
            &json!({"initial":logs,"bounded":bounded_logs,"secondPage":second_logs}),
        )
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let captured = wire.tool("lomi_browser_screenshot",json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"navigationId":after["structuredContent"]["data"]["navigationId"]})).await?;
    let artifact = &captured["structuredContent"]["data"]["artifact"];
    let captured_image = captured["content"]
        .as_array()
        .and_then(|content| content.iter().find(|c| c["type"] == "image"))
        .ok_or_else(|| format!("Native MCP image missing: {captured}"))?;
    if captured_image["mimeType"] != "image/png"
        || captured["structuredContent"]["data"].get("image").is_some()
    {
        return Err("MCP image format or metadata attachment separation is invalid".into());
    }
    let png = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        captured_image["data"].as_str().ok_or("Missing PNG data")?,
    )
    .map_err(|e| e.to_string())?;
    let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    if decoded.width() > 1280
        || decoded.width() != artifact["image"]["pixelWidth"].as_u64().unwrap_or(0) as u32
        || decoded.height() != artifact["image"]["pixelHeight"].as_u64().unwrap_or(0) as u32
        || png.len() > 1024 * 1024
    {
        return Err("PNG violated its dimensions or byte budget".into());
    }
    std::fs::write(directory.join("mcp-browser-capture.png"), &png).map_err(|e| e.to_string())?;
    std::fs::write(
        directory.join("mcp-browser-capture.json"),
        serde_json::to_vec_pretty(&captured["structuredContent"]).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let artifact_args = json!({"workspaceId":new_workspace,"artifactId":artifact["id"]});
    let reread = wire
        .tool("lomi_artifact_read", artifact_args.clone())
        .await?;
    if reread["content"]
        .as_array()
        .and_then(|content| content.iter().find(|c| c["type"] == "image"))
        != Some(captured_image)
    {
        return Err("Artifact did not preserve the exact native PNG bytes".into());
    }
    let denied_artifact = wire
        .tool(
            "lomi_artifact_read",
            json!({"workspaceId":"foreign-workspace","artifactId":artifact["id"]}),
        )
        .await?;
    if denied_artifact["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err("Artifact leaked outside its source workspace".into());
    }
    let retained_form_value = evaluate(&browser, "document.querySelector('#name').value").await?;
    let focus_list = wire
        .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
        .await?;
    let scratch = focus_list["structuredContent"]["data"]["items"]
        .as_array()
        .and_then(|p| p.iter().find(|p| p["kind"] == "file"))
        .ok_or("Missing scratch for browser focus fixture")?;
    let hide_args = json!({"workspaceId":new_workspace,"panelId":scratch["id"],"expectedRevision":focus_list["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"hide-browser-by-focusing-file"});
    let hidden = wire.tool("lomi_panel_focus", hide_args).await?;
    let hidden = wire
        .settled(
            hidden["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| hidden.to_string())?,
        )
        .await?;
    if hidden["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("File focus failed: {hidden}"));
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    let hidden_capture = wire.tool("lomi_browser_screenshot", json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"navigationId":after["structuredContent"]["data"]["navigationId"]})).await?;
    if hidden_capture["structuredContent"]["code"] != "PANEL_NOT_RENDERABLE" {
        return Err(format!(
            "Hidden browser capture accepted: {}",
            hidden_capture["structuredContent"]
        ));
    }
    let focus_list = wire
        .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
        .await?;
    let focus_browser = json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"expectedRevision":focus_list["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"focus-retained-browser"});
    let mut stale_focus = focus_browser.clone();
    stale_focus["browserGeneration"] = json!("stale-generation");
    stale_focus["requestKey"] = json!("focus-stale-browser");
    if wire.tool("lomi_panel_focus", stale_focus).await?["structuredContent"]["code"]
        != "STALE_GENERATION"
    {
        return Err("Focus accepted stale browser generation".into());
    }
    let focused_browser = wire.tool("lomi_panel_focus", focus_browser.clone()).await?;
    let focus_operation = focused_browser["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| focused_browser.to_string())?;
    let focused_browser = wire.settled(focus_operation).await?;
    if focused_browser["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Browser focus failed: {focused_browser}"));
    }
    let focus_retry = wire.tool("lomi_panel_focus", focus_browser).await?;
    if focus_retry["structuredContent"]["data"]["operationId"] != focus_operation
        || evaluate(&browser, "document.querySelector('#name').value").await? != retained_form_value
    {
        return Err("Browser focus did not retain form state or deduplicate".into());
    }
    qualify_panel_moves(&mut wire, &main, &browser, directory, json!({"workspaceId":new_workspace,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"server":server_target,"browser":browser_target,"serverOperation":server_operation,"serverRun":server_run_args})).await?;
    let mut replaced_args = action_args.clone();
    replaced_args["snapshotId"] = after["structuredContent"]["data"]["snapshotId"].clone();
    replaced_args["elementRef"] = after["structuredContent"]["data"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["role"] == "button" && e["name"] == "Save")
        .unwrap()["elementRef"]
        .clone();
    replaced_args["requestKey"] = json!("replaced-dom-node");
    evaluate(&browser,"(()=>{const e=document.querySelector('#save');e.replaceWith(e.cloneNode(true));return true;})()").await?;
    let replaced = wire.tool("lomi_browser_click", replaced_args).await?;
    let replaced = wire
        .settled(
            replaced["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| replaced.to_string())?,
        )
        .await?;
    if replaced["structuredContent"]["data"]["state"] != "failed"
        || replaced["structuredContent"]["data"]["effectState"] != "none"
        || replaced["structuredContent"]["data"]["result"]["code"] != "STALE_SNAPSHOT"
        || evaluate(&browser, "window.fixtureSaveCount").await? != 2
    {
        return Err(format!(
            "Replaced element was not rejected before input: {replaced}"
        ));
    }
    let mut scroll_args = action_args.clone();
    scroll_args.as_object_mut().unwrap().remove("elementRef");
    scroll_args["snapshotId"] = after["structuredContent"]["data"]["snapshotId"].clone();
    scroll_args["deltaX"] = json!(0);
    scroll_args["deltaY"] = json!(180);
    scroll_args["requestKey"] = json!("browser-scroll");
    let scrolled = wire
        .tool("lomi_browser_scroll", scroll_args.clone())
        .await?;
    let scrolled_id = scrolled["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| scrolled.to_string())?;
    let scroll_result = wire.settled(scrolled_id).await?;
    if scroll_result["structuredContent"]["data"]["state"] != "succeeded"
        || scroll_result["structuredContent"]["data"]["result"]["scrollPosition"]["y"] != 180.0
    {
        return Err(format!("Native viewport scroll failed: {scroll_result}"));
    }
    let scroll_retry = wire.tool("lomi_browser_scroll", scroll_args).await?;
    if scroll_retry["structuredContent"]["data"]["operationId"] != scrolled_id
        || evaluate(&browser, "scrollY").await? != 180
    {
        return Err("Scroll retry moved the viewport twice".into());
    }
    evaluate(&browser,"setTimeout(()=>document.querySelector('#result').textContent='Async result ready',700);true").await?;
    let wait_args = json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"condition":{"type":"text","text":"Async result ready"},"timeoutMs":3000});
    let waited = wire.tool("lomi_browser_wait", wait_args.clone()).await?;
    if waited["structuredContent"]["data"]["matched"] != true
        || !waited["structuredContent"]["data"]["snapshot"]
            .to_string()
            .contains("Async result ready")
    {
        return Err(format!("Wait did not observe asynchronous DOM: {waited}"));
    }
    let mut unmatched = wait_args.clone();
    unmatched["condition"]["text"] = json!("This text never appears");
    unmatched["timeoutMs"] = json!(200);
    let unmatched = wire.tool("lomi_browser_wait", unmatched).await?;
    if unmatched["structuredContent"]["data"]["matched"] != false
        || unmatched["structuredContent"]["data"]["elapsedMs"]
            .as_u64()
            .is_none_or(|ms| !(150..1500).contains(&ms))
    {
        return Err(format!("Wait timeout was not bounded: {unmatched}"));
    }
    let current_snapshot = &unmatched["structuredContent"]["data"]["snapshot"];
    let mut delayed_args = action_args.clone();
    delayed_args["snapshotId"] = current_snapshot["snapshotId"].clone();
    delayed_args["navigationId"] = current_snapshot["navigationId"].clone();
    delayed_args["elementRef"] = current_snapshot["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["role"] == "textbox" && e["name"] == "Name")
        .unwrap()["elementRef"]
        .clone();
    delayed_args["requestKey"] = json!("browser-delayed-fill");
    delayed_args["text"] = json!("This late input must not run");
    evaluate(&browser,"scrollTo(0,0);setTimeout(()=>{const until=performance.now()+4000;while(performance.now()<until){}},50);true").await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let delayed = wire.tool("lomi_browser_fill", delayed_args.clone()).await?;
    let delayed = wire
        .settled(
            delayed["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| delayed.to_string())?,
        )
        .await?;
    if delayed["structuredContent"]["data"]["result"]["code"] != "DEADLINE_EXCEEDED" {
        return Err(format!(
            "Busy browser did not expire native dispatch: {delayed}"
        ));
    }
    let delayed_value = evaluate(&browser, "document.querySelector('#name').value").await?;
    if !delayed_value
        .as_str()
        .is_some_and(|s| s.starts_with("Zażółć 🙂"))
    {
        return Err("Timed out input executed after the page resumed".into());
    }
    let mut spa_args = action_args.clone();
    spa_args["snapshotId"] = current_snapshot["snapshotId"].clone();
    spa_args["navigationId"] = current_snapshot["navigationId"].clone();
    spa_args["requestKey"] = json!("browser-spa-stale");
    spa_args["elementRef"] = current_snapshot["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["role"] == "button" && e["name"] == "Save")
        .unwrap()["elementRef"]
        .clone();
    evaluate(&browser, "history.pushState({},'', '#spa');true").await?;
    let mut spa = wire.tool("lomi_browser_click", spa_args).await?;
    if let Some(operation) = spa["structuredContent"]["data"]["operationId"]
        .as_str()
        .map(str::to_owned)
    {
        spa = wire.settled(&operation).await?;
    }
    if spa["structuredContent"]["code"] != "STALE_SNAPSHOT"
        && spa["structuredContent"]["data"]["result"]["code"] != "STALE_SNAPSHOT"
    {
        return Err(format!(
            "SPA URL change did not expire old references: {spa}"
        ));
    }
    let replay = wire.tool("lomi_browser_open", browser_args.clone()).await?;
    if replay["structuredContent"]["data"]["operationId"] != browser_operation {
        return Err("Browser retry created a new operation".into());
    }
    let mut foreign = browser_args;
    foreign["url"] = browser_fixture["blocked"].clone();
    foreign["requestKey"] = json!("native-browser-denied");
    let rejected = wire.tool("lomi_browser_open", foreign).await?;
    if rejected["structuredContent"]["code"] != "SCOPE_DENIED" {
        return Err(format!("Foreign origin accepted: {rejected}"));
    }
    let navigation_args = json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"leaseId":browser_target["leaseId"],"url":format!("{}/next",browser_fixture["origin"].as_str().unwrap()),"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-browser-navigate"});
    let navigating = wire
        .tool("lomi_browser_navigate", navigation_args.clone())
        .await?;
    let navigation_operation = navigating["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Navigation rejected: {navigating}"))?;
    let navigated = wire.settled(navigation_operation).await?;
    if navigated["structuredContent"]["data"]["state"] != "succeeded"
        || navigated["structuredContent"]["data"]["result"]["loaded"] != true
        || navigated["structuredContent"]["data"]["result"]["committed"] != true
    {
        return Err(format!("Navigation was not observed natively: {navigated}"));
    }
    if evaluate(&browser, "location.pathname").await? != "/next" {
        return Err("Navigation changed no native document".into());
    }
    let replayed = wire.tool("lomi_browser_navigate", navigation_args).await?;
    if replayed["structuredContent"]["data"]["operationId"] != navigation_operation {
        return Err("Navigation retry changed its operation".into());
    }
    std::fs::write(
        directory.join("browser-navigate.json"),
        serde_json::to_vec_pretty(&navigated).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let mut navigation_cancellation = Vec::new();
    for (index, cancel) in [true, false].into_iter().enumerate() {
        let slow_args = json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"leaseId":browser_target["leaseId"],"url":format!("{}/slow?case={index}",browser_fixture["origin"].as_str().unwrap()),"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("native-slow-navigation-{index}")});
        let start = std::time::Instant::now();
        let slow = wire
            .tool("lomi_browser_navigate", slow_args.clone())
            .await?;
        let slow_operation = slow["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| slow.to_string())?;
        let progress = || {
            std::fs::read(directory.join("browser-slow-navigation.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .unwrap_or(Value::Null)
        };
        for _ in 0..100 {
            if progress()["started"].as_u64().unwrap_or(0) >= (index + 1) as u64 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if progress()["started"] != (index + 1) as u64 {
            return Err("Native slow navigation never reached the fixture server".into());
        }
        if cancel {
            wire.tool(
                "lomi_operation_cancel",
                json!({"operationId":slow_operation}),
            )
            .await?;
        }
        let stopped = match wire.settled(slow_operation).await {
            Ok(value) => value,
            Err(error) => {
                let last = wire
                    .tool("lomi_operation_get", json!({"operationId":slow_operation}))
                    .await?;
                return Err(format!(
                    "Slow navigation {index} failed after {:?}: {error}; receipt={last}; server={}",
                    start.elapsed(),
                    progress()
                ));
            }
        };
        if stopped["structuredContent"]["data"]["state"] != "outcome_unknown"
            || stopped["structuredContent"]["data"]["effectState"] != "unknown"
            || stopped["structuredContent"]["data"]["result"]["code"]
                != if cancel {
                    "CONTROL_REVOKED"
                } else {
                    "DEADLINE_EXCEEDED"
                }
        {
            return Err(format!(
                "Navigation cancellation misreported effects: {stopped}"
            ));
        }
        for _ in 0..100 {
            if progress()["closed"].as_u64().unwrap_or(0) >= (index + 1) as u64 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if progress()["closed"] != (index + 1) as u64 {
            return Err("Navigation was reported stopped but its HTTP response stayed open".into());
        }
        let retried = wire.tool("lomi_browser_navigate", slow_args).await?;
        if retried["structuredContent"]["data"]["operationId"] != slow_operation {
            return Err("Cancelled navigation retry created another request".into());
        }
        navigation_cancellation.push(json!({"cancel":cancel,"elapsedMs":start.elapsed().as_millis(),"receipt":stopped,"server":progress()}));
        let recovery = wire.tool("lomi_browser_navigate", json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"leaseId":browser_target["leaseId"],"url":format!("{}/recovered-{index}",browser_fixture["origin"].as_str().unwrap()),"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("native-navigation-recovery-{index}")})).await?;
        let recovered = wire
            .settled(
                recovery["structuredContent"]["data"]["operationId"]
                    .as_str()
                    .ok_or_else(|| recovery.to_string())?,
            )
            .await?;
        if recovered["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!(
                "Navigation did not recover after stop: {recovered}"
            ));
        }
    }
    if wire.tool("lomi_browser_logs", cursor_args).await?["structuredContent"]["code"]
        != "CURSOR_EXPIRED"
    {
        return Err("Navigation retained an old log cursor".into());
    }
    std::fs::write(
        directory.join("browser-navigation-cancel.json"),
        serde_json::to_vec_pretty(&navigation_cancellation).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let profile = evaluate(&browser,"(()=>{localStorage.setItem('mcp-native-profile','isolated');return localStorage.getItem('mcp-native-profile')})()").await?;
    if profile != "isolated" {
        return Err("Browser storage unavailable".into());
    }
    evaluate(
        &browser,
        &format!(
            "location.assign({});true",
            json!(format!(
                "{}/redirect",
                browser_fixture["origin"].as_str().unwrap()
            ))
        ),
    )
    .await?;
    if let Err(error) = wait_for(
        &main,
        "document.body.textContent.includes('Navigation blocked by agent browser permissions.')",
    )
    .await
    {
        let ui = evaluate(&main, "document.body.textContent.slice(-4000)")
            .await
            .unwrap_or(Value::Null);
        let native = crate::browser::smoke_pages(app);
        let _ = screenshot(&main, directory.join("browser-denial-failure.png")).await;
        return Err(format!("{error}; native={native}; UI={ui}"));
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    let denied_requests: u32 = serde_json::from_slice(
        &std::fs::read(directory.join("browser-denied-requests.json"))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if denied_requests != 0 {
        return Err("Blocked browser redirect reached the foreign server".into());
    }
    std::fs::write(
        directory.join("browser-open.json"),
        serde_json::to_vec_pretty(&ready).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    screenshot(&browser, directory.join("browser-open.png")).await?;
    if browser_monitor_count(app).await? != 1 {
        return Err("Browser input monitor was not installed once".into());
    }
    native_browser_pointer(&browser).await?;
    // postEvent queues AppKit input; posting is not proof that its monitor ran.
    for _ in 0..100 {
        if browser_monitor_count(app).await? == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if browser_monitor_count(app).await? != 0 {
        return Err("AppKit did not consume the browser takeover event".into());
    }
    let revoked_logs = wire.tool("lomi_browser_logs", logs_args).await?;
    if revoked_logs["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err("Human takeover still disclosed browser logs".into());
    }
    let revoked_artifact = wire.tool("lomi_artifact_read", artifact_args).await?;
    if revoked_artifact["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err(format!(
            "Takeover still disclosed captured image: {revoked_artifact}"
        ));
    }
    wait_for(
        &main,
        "!document.body.textContent.includes('Agent · Take control')",
    )
    .await?;
    let human_navigation = wire.tool("lomi_browser_navigate",json!({"workspaceId":new_workspace,"panelId":browser_target["panelId"],"browserGeneration":browser_target["browserGeneration"],"leaseId":browser_target["leaseId"],"url":format!("{}/after-human",browser_fixture["origin"].as_str().unwrap()),"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-browser-after-human"})).await?;
    if human_navigation["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err(format!(
            "Native browser input left agent access enabled: {human_navigation}"
        ));
    }
    if browser_monitor_count(app).await? != 0 {
        return Err("Human takeover leaked a native input monitor".into());
    }
    let latest = wire
        .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
        .await?;
    let second = wire.tool("lomi_browser_open",json!({"workspaceId":new_workspace,"url":browser_fixture["origin"],"expectedRevision":latest["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-browser-second"})).await?;
    let second_operation = second["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Second browser rejected: {second}"))?;
    let second = wire.settled(second_operation).await?;
    let second_target = &second["structuredContent"]["data"]["result"];
    if second_target["kind"] != "browser" {
        return Err(format!("Second browser failed: {second}"));
    }
    wait_for(
        &main,
        "document.body.textContent.includes('Agent · Take control')",
    )
    .await?;
    screenshot(&main, directory.join("browser-ownership.png")).await?;
    click(&main, "Agent · Take control").await?;
    wait_for(
        &main,
        "!document.body.textContent.includes('Agent · Take control')",
    )
    .await?;
    let refused = wire.tool("lomi_browser_navigate",json!({"workspaceId":new_workspace,"panelId":second_target["panelId"],"browserGeneration":second_target["browserGeneration"],"leaseId":second_target["leaseId"],"url":browser_fixture["origin"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-browser-after-button"})).await?;
    if refused["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err("Browser takeover button did not revoke navigation".into());
    }

    let close_revision = wire
        .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
        .await?;
    let denied_browser_close = wire.tool("lomi_panel_close", json!({"workspaceId":new_workspace,"panelId":second_target["panelId"],"browserGeneration":second_target["browserGeneration"],"expectedRevision":close_revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"close-human-browser"})).await?;
    if denied_browser_close["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err(format!(
            "Close accepted human browser: {denied_browser_close}"
        ));
    }
    let disposable_browser = wire.tool("lomi_browser_open", json!({"workspaceId":new_workspace,"url":observed_origin,"expectedRevision":close_revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"visible":false,"requestKey":"disposable-browser"})).await?;
    let disposable_browser = wire
        .settled(
            disposable_browser["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| disposable_browser.to_string())?,
        )
        .await?;
    let disposable_target = &disposable_browser["structuredContent"]["data"]["result"];
    if disposable_browser["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Disposable browser failed: {disposable_browser}"));
    }
    let disposable_snapshot = wire.tool("lomi_browser_wait", json!({"workspaceId":new_workspace,"panelId":disposable_target["panelId"],"browserGeneration":disposable_target["browserGeneration"],"condition":{"type":"element","role":"heading","name":"Lomi native browser qualification"}})).await?;
    let disposable_native = app
        .get_webview(&format!(
            "browser-{}",
            disposable_target["panelId"].as_str().unwrap()
        ))
        .ok_or("Missing hidden native browser")?;
    if !native_browser_hidden(&disposable_native).await?
        || evaluate(
            &main,
            "document.querySelector('[role=tab][aria-selected=true]')?.closest('[data-tab-id]')?.dataset.tabId ?? null",
        )
        .await?
            != second_target["panelId"]
        || disposable_snapshot["structuredContent"]["data"]["matched"] != true
        || disposable_snapshot["structuredContent"]["data"]["snapshot"]["elements"]
            .as_array()
            .is_none_or(|n| n.is_empty())
    {
        return Err(format!(
            "Hidden page changed selection or lacked native DOM: {}",
            disposable_snapshot["structuredContent"]
        ));
    }
    let hidden_capture = wire.tool("lomi_browser_screenshot", json!({"workspaceId":new_workspace,"panelId":disposable_target["panelId"],"browserGeneration":disposable_target["browserGeneration"],"navigationId":disposable_snapshot["structuredContent"]["data"]["snapshot"]["navigationId"]})).await?;
    if hidden_capture["structuredContent"]["code"] != "PANEL_NOT_RENDERABLE" {
        return Err("New hidden page permitted image capture".into());
    }
    let mut revealed = Value::Null;
    let mut reveal_attempts = Vec::new();
    for attempt in 0..3 {
        let focus_revision = wire
            .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
            .await?;
        let request = wire.tool("lomi_panel_focus", json!({"workspaceId":new_workspace,"panelId":disposable_target["panelId"],"browserGeneration":disposable_target["browserGeneration"],"expectedRevision":focus_revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("reveal-hidden-browser-{attempt}")})).await?;
        revealed = wire
            .settled(
                request["structuredContent"]["data"]["operationId"]
                    .as_str()
                    .ok_or_else(|| request.to_string())?,
            )
            .await?;
        reveal_attempts.push(json!({"revision":focus_revision["structuredContent"]["data"]["domainRevision"],"receipt":revealed}));
        // A concurrent title/layout publication can invalidate discovery. Only a
        // definitive failure with no effect permits refreshing and a new key.
        let data = &revealed["structuredContent"]["data"];
        if data["state"] != "failed"
            || data["effectState"] != "none"
            || data["result"]["code"] != "REVISION_CONFLICT"
        {
            break;
        }
    }
    if revealed["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Reveal failed: {revealed}"));
    }
    for _ in 0..200 {
        if !native_browser_hidden(&disposable_native).await? {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if native_browser_hidden(&disposable_native).await? {
        return Err("Revealed view stayed hidden".into());
    }
    std::fs::write(directory.join("browser-hidden.json"), serde_json::to_vec_pretty(&json!({"snapshot":disposable_snapshot,"captureDenied":hidden_capture,"revealed":revealed,"revealAttempts":reveal_attempts})).unwrap()).map_err(|e|e.to_string())?;
    let closing_capture = wire.tool("lomi_browser_screenshot", json!({"workspaceId":new_workspace,"panelId":disposable_target["panelId"],"browserGeneration":disposable_target["browserGeneration"],"navigationId":disposable_snapshot["structuredContent"]["data"]["snapshot"]["navigationId"]})).await?;
    let closing_artifact = closing_capture["structuredContent"]["data"]["artifact"]["id"]
        .as_str()
        .ok_or_else(|| {
            format!(
                "Closing capture failed: {}",
                closing_capture["structuredContent"]
            )
        })?;
    let close_revision = wire
        .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
        .await?;
    let close_browser_args = json!({"workspaceId":new_workspace,"panelId":disposable_target["panelId"],"browserGeneration":disposable_target["browserGeneration"],"expectedRevision":close_revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"close-owned-browser"});
    let closed_browser = wire
        .tool("lomi_panel_close", close_browser_args.clone())
        .await?;
    let close_operation = closed_browser["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| closed_browser.to_string())?;
    let closed_browser = wire.settled(close_operation).await?;
    if closed_browser["structuredContent"]["data"]["state"] != "succeeded"
        || closed_browser["structuredContent"]["data"]["result"]["closed"] != true
    {
        return Err(format!("Owned browser close failed: {closed_browser}"));
    }
    let closed_replay = wire.tool("lomi_panel_close", close_browser_args).await?;
    if closed_replay["structuredContent"]["data"]["operationId"] != close_operation {
        return Err("Browser close replayed".into());
    }
    let closed_id = disposable_target["panelId"].as_str().unwrap();
    if app.get_webview(&format!("browser-{closed_id}")).is_some()
        || browser_monitor_count(app).await? != 0
    {
        return Err("Closed browser retained its native view or input monitor".into());
    }
    if app
        .get_webview(&format!(
            "browser-{}",
            second_target["panelId"].as_str().unwrap()
        ))
        .is_none()
    {
        return Err("Closing another browser removed the human-owned view".into());
    }
    let closed_artifact = wire
        .tool(
            "lomi_artifact_read",
            json!({"workspaceId":new_workspace,"artifactId":closing_artifact}),
        )
        .await?;
    if closed_artifact["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err(format!(
            "Closed browser artifact still disclosed: {}",
            closed_artifact["structuredContent"]
        ));
    }
    std::fs::write(directory.join("browser-close.json"), serde_json::to_vec_pretty(&json!({"closed":closed_browser,"deniedHuman":denied_browser_close,"artifact":closed_artifact})).unwrap()).map_err(|e|e.to_string())?;
    qualify_workspace_close(&mut wire,&main,directory,json!({"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"anchor":workspace,"protectedWorkspace":new_workspace,"origin":browser_fixture["origin"]})).await?;
    let stop_server = wire.tool("lomi_terminal_interrupt", json!({"workspaceId":new_workspace,"panelId":server_target["panelId"],"terminalSessionId":server_target["terminalSessionId"],"leaseId":server_target["leaseId"],"operationId":server_operation,"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"stop-browser-fixture-server"})).await?;
    let stop_server = wire
        .settled(
            stop_server["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or_else(|| stop_server.to_string())?,
        )
        .await?;
    let server_stopped = wire.settled(server_operation).await?;
    if stop_server["structuredContent"]["data"]["state"] != "succeeded"
        || server_stopped["structuredContent"]["data"]["result"]["observation"]["exitCode"] != 0
    {
        return Err(format!("PTY dev-server cleanup failed: {server_stopped}"));
    }
    std::fs::write(
        directory.join("dev-server-stopped.json"),
        serde_json::to_vec_pretty(&server_stopped).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    if let Some(device) = &android_fixture {
        let build_command: Value = serde_json::from_slice(
            &std::fs::read(directory.join("apk-build-command.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let build_args = json!({"workspaceId":new_workspace,"panelId":server_target["panelId"],"terminalSessionId":server_target["terminalSessionId"],"leaseId":server_target["leaseId"],"command":build_command["command"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"build-android-fixture"});
        let build = wire.tool("lomi_terminal_run", build_args.clone()).await?;
        let build_id = build["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| format!("APK build admission failed: {build}"))?;
        let built = wire.settled_with_limit(build_id, 4800).await?;
        if built["structuredContent"]["data"]["state"] != "succeeded"
            || built["structuredContent"]["data"]["result"]["observation"]["exitCode"] != 0
        {
            return Err(format!("MCP PTY APK build failed: {built}"));
        }
        let mut read_build = server_read_args.clone();
        read_build["maxBytes"] = json!(65536);
        let build_output = wire.tool("lomi_terminal_read", read_build).await?;
        let text = build_output["structuredContent"]["data"]["text"]
            .as_str()
            .ok_or("Missing build output")?;
        let fixture: Value = text
            .lines()
            .filter_map(|l| {
                l.find("LOMI_APK_RESULT=")
                    .and_then(|i| serde_json::from_str(l[i + 16..].trim()).ok())
            })
            .next_back()
            .ok_or_else(|| format!("No APK metadata in PTY output: {build_output}"))?;
        let replay_build = wire.tool("lomi_terminal_run", build_args).await?;
        if replay_build["structuredContent"]["data"]["operationId"] != build_id {
            return Err("APK build repeated".into());
        }
        std::fs::write(directory.join("apk-build.json"), serde_json::to_vec_pretty(&json!({"receipt":built,"output":build_output,"metadata":fixture,"retry":replay_build})).unwrap()).map_err(|e|e.to_string())?;
        let bad_build_args = json!({"workspaceId":new_workspace,"panelId":server_target["panelId"],"terminalSessionId":server_target["terminalSessionId"],"leaseId":server_target["leaseId"],"command":format!("{} --different-signer",build_command["command"].as_str().unwrap()),"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"build-incompatible-fixture"});
        let bad_build = wire.tool("lomi_terminal_run", bad_build_args).await?;
        let bad_build_id = bad_build["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| bad_build.to_string())?;
        let bad_built = wire.settled_with_limit(bad_build_id, 4800).await?;
        if bad_built["structuredContent"]["data"]["state"] != "succeeded"
            || bad_built["structuredContent"]["data"]["result"]["observation"]["exitCode"] != 0
        {
            return Err(format!("Incompatible fixture build failed: {bad_built}"));
        }
        let mut read_build = server_read_args.clone();
        read_build["maxBytes"] = json!(65536);
        let output = wire.tool("lomi_terminal_read", read_build).await?;
        let bad_fixture: Value = output["structuredContent"]["data"]["text"]
            .as_str()
            .ok_or("No incompatible build output")?
            .lines()
            .filter_map(|l| {
                l.find("LOMI_APK_RESULT=")
                    .and_then(|i| serde_json::from_str(l[i + 16..].trim()).ok())
            })
            .next_back()
            .ok_or("No incompatible APK metadata")?;
        if bad_fixture["relativePath"] != "mcp-incompatible.apk"
            || bad_fixture["sha256"] == fixture["sha256"]
        {
            return Err("Incompatible fixture did not use different bytes".into());
        }
        let revision = wire
            .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
            .await?;
        let import_args = json!({"workspaceId":new_workspace,"relativePath":fixture["relativePath"],"kind":"android_apk","expectedByteLength":fixture["byteLength"],"expectedSha256":fixture["sha256"],"expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-import-apk"});
        let imported = wire
            .tool("lomi_artifact_import", import_args.clone())
            .await?;
        let import_op = imported["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| imported.to_string())?;
        let imported = wire.settled(import_op).await?;
        let result = &imported["structuredContent"]["data"]["result"];
        if imported["structuredContent"]["data"]["state"] != "succeeded"
            || result["sha256"] != fixture["sha256"]
            || result["byteLength"] != fixture["byteLength"]
        {
            return Err(format!("APK import failed: {imported}"));
        }
        let replay = wire
            .tool("lomi_artifact_import", import_args.clone())
            .await?;
        if replay["structuredContent"]["data"]["operationId"] != import_op {
            return Err("APK import duplicated the copy".into());
        }
        let read_args = json!({"workspaceId":new_workspace,"artifactId":result["artifactId"]});
        let metadata = wire.tool("lomi_artifact_read", read_args.clone()).await?;
        if metadata["structuredContent"]["data"]["artifact"]["mediaType"]
            != "application/vnd.android.package-archive"
            || metadata["structuredContent"]["data"]["image"] != Value::Null
        {
            return Err("APK metadata leaked binary content or lost its type".into());
        }
        let source = directory.join("project/mcp-input-test.apk");
        let bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
        std::fs::write(&source, b"changed source after completed import")
            .map_err(|e| e.to_string())?;
        let reread = wire.tool("lomi_artifact_read", read_args).await?;
        if reread["structuredContent"] != metadata["structuredContent"] {
            return Err("Source change altered imported metadata".into());
        }
        std::fs::write(source, bytes).map_err(|e| e.to_string())?;
        let mut bad = import_args.clone();
        bad["requestKey"] = json!("bad-native-apk-hash");
        bad["expectedSha256"] = json!("0".repeat(64));
        let bad = wire.tool("lomi_artifact_import", bad).await?;
        let bad = wire
            .settled(
                bad["structuredContent"]["data"]["operationId"]
                    .as_str()
                    .ok_or_else(|| bad.to_string())?,
            )
            .await?;
        if bad["structuredContent"]["data"]["result"]["code"] != "REVISION_CONFLICT" {
            return Err(format!("Bad APK hash accepted: {bad}"));
        }
        let foreign = wire
            .tool(
                "lomi_artifact_read",
                json!({"workspaceId":workspace,"artifactId":result["artifactId"]}),
            )
            .await?;
        if foreign["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
            return Err("APK workspace classification lost".into());
        }
        std::fs::write(directory.join("apk-import.json"),serde_json::to_vec_pretty(&json!({"imported":imported,"metadata":metadata,"foreign":foreign,"badHash":bad,"sourceChangePreserved":true,"retryOnce":true})).unwrap()).map_err(|e|e.to_string())?;
        let projection = wire
            .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
            .await?;
        let args = json!({"workspaceId":new_workspace,"deviceId":device["deviceId"],"expectedRevision":projection["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-open-android"});
        let opened = wire.tool("lomi_android_open", args.clone()).await?;
        let operation = opened["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| opened.to_string())?;
        let opened = wire.settled(operation).await?;
        let panel = opened["structuredContent"]["data"]["result"]["panelId"]
            .as_str()
            .ok_or_else(|| opened.to_string())?;
        if opened["structuredContent"]["data"]["state"] != "succeeded"
            || opened["structuredContent"]["data"]["result"]["deviceId"] != device["deviceId"]
        {
            return Err(format!("Android open failed: {opened}"));
        }
        let ui = javascript(&main, "const state=await window.__TAURI_INTERNALS__.invoke('android_state');return {text:document.body.innerText,panels:[...document.querySelectorAll('[data-android-pane-id]')].map(e=>e.getAttribute('data-android-pane-id')),qualified:state.qualified,toolchainReady:state.toolchainReady,errors:state.errors};").await?;
        std::fs::write(
            directory.join("android-open-ui.json"),
            serde_json::to_vec_pretty(&ui).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        wait_for(&main, &format!("Boolean(document.querySelector('[data-android-pane-id=\"{panel}\"]')) && document.body.textContent.includes('Android is stopped. Its apps and data are kept.')")).await.map_err(|error| error.to_string())?;
        let repeated = wire.tool("lomi_android_open", args).await?;
        if repeated["structuredContent"]["data"]["operationId"]
            != opened["structuredContent"]["data"]["operationId"]
        {
            return Err("Android open retry created another panel".into());
        }
        let state = wire
            .tool("lomi_android_list", json!({"workspaceId":new_workspace}))
            .await?;
        if state["structuredContent"]["data"]["devices"]["items"][0]["phase"] != "stopped"
            || state["structuredContent"]["data"]["devices"]["items"][0]["processAlive"] != false
        {
            return Err("Opening the Android panel started the device".into());
        }
        let panels = wire
            .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
            .await?;
        if panels["structuredContent"]["data"]["items"]
            .as_array()
            .is_none_or(|items| {
                items
                    .iter()
                    .filter(|p| {
                        p["kind"] == "android" && p["androidDeviceId"] == device["deviceId"]
                    })
                    .count()
                    != 1
            })
        {
            return Err("Android panel projection duplicated or lost its device".into());
        }
        std::fs::write(
            directory.join("android-open.json"),
            serde_json::to_vec_pretty(
                &json!({"opened":opened,"repeated":repeated,"devices":state,"panels":panels}),
            )
            .unwrap(),
        )
        .map_err(|e| e.to_string())?;
        let args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"expectedRevision":panels["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-start-android"});
        let started = wire.tool("lomi_android_start", args.clone()).await?;
        let operation = started["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| started.to_string())?;
        let repeated = wire.tool("lomi_android_start", args.clone()).await?;
        if repeated["structuredContent"]["data"]["operationId"] != operation {
            return Err("Android boot retry spawned another operation".into());
        }
        let started = wire.settled_with_limit(operation, 7200).await?;
        let result = &started["structuredContent"]["data"]["result"];
        if started["structuredContent"]["data"]["state"] != "succeeded" || result["ready"] != true {
            return Err(format!("Android did not become ready: {started}"));
        }
        let generation = result["generation"]
            .as_str()
            .ok_or("Missing Android generation")?;
        let running = wire
            .tool("lomi_android_list", json!({"workspaceId":new_workspace}))
            .await?;
        if running["structuredContent"]["data"]["devices"]["items"][0]["generation"] != generation
            || running["structuredContent"]["data"]["devices"]["items"][0]["phase"] != "running"
        {
            return Err("Android readiness has no matching native generation".into());
        }

        let imported_artifact = &imported["structuredContent"]["data"]["result"];
        let mut install_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"artifactId":imported_artifact["artifactId"],"sha256":fixture["sha256"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-install-deny"});
        let awaiting = wire
            .tool("lomi_android_install_apk", install_args.clone())
            .await?;
        let install_op = awaiting["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| awaiting.to_string())?;
        if awaiting["structuredContent"]["data"]["state"] != "awaiting_user" {
            return Err(format!("APK install omitted Settings approval: {awaiting}"));
        }
        let main_denied=javascript(&main,&format!("try {{await window.__TAURI_INTERNALS__.invoke('agent_control_decide_install',{{operationId:{},approve:true}});return false;}}catch{{return true;}}",json!(install_op))).await?;
        if main_denied != true {
            return Err("Main could approve an APK installation".into());
        }
        click(&settings, "Deny installation").await?;
        let denied = wire.settled(install_op).await?;
        if denied["structuredContent"]["data"]["state"] != "cancelled" {
            return Err(format!("APK denial failed: {denied}"));
        }
        let repeat = wire
            .tool("lomi_android_install_apk", install_args.clone())
            .await?;
        if repeat["structuredContent"]["data"]["state"] != "cancelled" {
            return Err("Denied APK replay asked again".into());
        }
        install_args["requestKey"] = json!("native-install-approved");
        let awaiting = wire
            .tool("lomi_android_install_apk", install_args.clone())
            .await?;
        let install_op = awaiting["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| awaiting.to_string())?;
        wait_for(
            &settings,
            &format!(
                "document.body.textContent.includes({}) && document.body.textContent.includes({})",
                fixture["sha256"],
                json!(generation)
            ),
        )
        .await?;
        evaluate(&settings,"[...document.querySelectorAll('button')].find(e=>e.textContent==='Install this APK').closest('article').scrollIntoView({block:'center'});true").await?;
        screenshot(&settings, directory.join("apk-install-approval.png")).await?;
        let source = directory.join("project/mcp-input-test.apk");
        let bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
        std::fs::write(&source, b"changed source while private APK awaits approval")
            .map_err(|e| e.to_string())?;
        click(&settings, "Install this APK").await?;
        let installed = wire.settled_with_limit(install_op, 7200).await?;
        std::fs::write(source, bytes).map_err(|e| e.to_string())?;
        if installed["structuredContent"]["data"]["state"] != "succeeded"
            || installed["structuredContent"]["data"]["result"]["installed"] != true
            || installed["structuredContent"]["data"]["result"]["sha256"] != fixture["sha256"]
            || installed["structuredContent"]["data"]["result"]["packageName"]
                .as_str()
                .is_none()
        {
            return Err(format!("Native APK installation failed: {installed}"));
        }
        let repeat = wire.tool("lomi_android_install_apk", install_args).await?;
        if repeat["structuredContent"]["data"]["operationId"] != install_op {
            return Err("APK retry repeated installation".into());
        }
        std::fs::write(directory.join("apk-install.json"),serde_json::to_vec_pretty(&json!({"installed":installed,"denied":denied,"retry":repeat,"mainApprovalDenied":true,"sourceChangedBeforeInstall":true})).unwrap()).map_err(|e|e.to_string())?;

        let revision = wire
            .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
            .await?;
        let bad_import = wire.tool("lomi_artifact_import", json!({"workspaceId":new_workspace,"relativePath":bad_fixture["relativePath"],"kind":"android_apk","expectedByteLength":bad_fixture["byteLength"],"expectedSha256":bad_fixture["sha256"],"expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"import-incompatible-fixture"})).await?;
        let bad_imported = wire
            .settled(
                bad_import["structuredContent"]["data"]["operationId"]
                    .as_str()
                    .ok_or_else(|| bad_import.to_string())?,
            )
            .await?;
        if bad_imported["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Incompatible APK import failed: {bad_imported}"));
        }
        let bad_install_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"artifactId":bad_imported["structuredContent"]["data"]["result"]["artifactId"],"sha256":bad_fixture["sha256"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"install-incompatible-fixture"});
        let bad_install = wire
            .tool("lomi_android_install_apk", bad_install_args.clone())
            .await?;
        let bad_install_id = bad_install["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| bad_install.to_string())?;
        if bad_install["structuredContent"]["data"]["state"] != "awaiting_user" {
            return Err("Incompatible install skipped approval".into());
        }
        click(&settings, "Install this APK").await?;
        let rejected_install = wire.settled_with_limit(bad_install_id, 7200).await?;
        let rejected = &rejected_install["structuredContent"]["data"];
        if rejected["state"] != "failed"
            || rejected["effectState"] != "none"
            || rejected["result"]["installed"] != false
            || rejected["result"]["installerFailure"] != "INSTALL_FAILED_UPDATE_INCOMPATIBLE"
            || rejected["result"]["previousVersion"] != "1"
        {
            return Err(format!(
                "Signature mismatch did not preserve the existing installation: {rejected_install}"
            ));
        }
        let retry = wire
            .tool("lomi_android_install_apk", bad_install_args)
            .await?;
        if retry["structuredContent"]["data"]["operationId"] != bad_install_id
            || retry["structuredContent"]["data"]["state"] != "failed"
        {
            return Err("Rejected install replayed".into());
        }
        std::fs::write(directory.join("apk-signature-rejection.json"), serde_json::to_vec_pretty(&json!({"build":bad_built,"metadata":bad_fixture,"import":bad_imported,"rejected":rejected_install,"retry":retry})).unwrap()).map_err(|e|e.to_string())?;

        let fixture_root =
            crate::android::fixture::directory()?.ok_or("Missing licensed Android fixture")?;
        let guest = crate::android::fixture::guest(
            &fixture_root,
            device["deviceId"].as_str().ok_or("Missing device UUID")?,
        )?;
        let revision = wire
            .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
            .await?;
        let launch_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"packageName":"org.lomi.inputtest","activity":null,"expectedRevision":revision["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"launch-installed-fixture"});
        let launch = wire
            .tool("lomi_android_launch", launch_args.clone())
            .await?;
        let launch_id = launch["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| launch.to_string())?;
        let launched = wire.settled(launch_id).await?;
        if launched["structuredContent"]["data"]["state"] != "succeeded"
            || launched["structuredContent"]["data"]["result"]["intentDelivered"] != true
        {
            return Err(format!("Native MCP app launch failed: {launched}"));
        }
        let repeated = wire
            .tool("lomi_android_launch", launch_args.clone())
            .await?;
        if repeated["structuredContent"]["data"]["operationId"] != launch_id {
            return Err("Launch replayed".into());
        }
        let mut forbidden = launch_args;
        forbidden["packageName"] = json!("com.android.settings");
        forbidden["requestKey"] = json!("forbidden-launch");
        let denied_launch = wire.tool("lomi_android_launch", forbidden).await?;
        if denied_launch["structuredContent"]["code"] != "SCOPE_DENIED" {
            return Err("Launch escaped package permission".into());
        }
        let log_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"packageName":"org.lomi.inputtest","minPriority":"I","limit":64});
        let logs = wire.tool("lomi_android_logcat", log_args.clone()).await?;
        let lines = logs["structuredContent"]["data"]["lines"]
            .as_array()
            .ok_or_else(|| format!("No Android app logs: {logs}"))?;
        if !lines.iter().any(|l| {
            l.as_str()
                .is_some_and(|s| s.contains("MCP fixture started:"))
        }) || logs["structuredContent"]["data"]["complete"] != false
        {
            return Err(format!(
                "Native Android log marker missing or completeness overstated: {logs}"
            ));
        }
        let mut paged = log_args.clone();
        paged["limit"] = json!(1);
        let page = wire.tool("lomi_android_logcat", paged.clone()).await?;
        if let Some(cursor) = page["structuredContent"]["data"]["nextCursor"].as_str() {
            paged["cursor"] = json!(cursor);
            let next = wire.tool("lomi_android_logcat", paged.clone()).await?;
            if next["structuredContent"]["data"]["lines"]
                .as_array()
                .map(Vec::len)
                != Some(1)
            {
                return Err("Native Android log pagination failed".into());
            }
            paged["minPriority"] = json!("E");
            let wrong_filter = wire.tool("lomi_android_logcat", paged).await?;
            if wrong_filter["structuredContent"]["code"] != "CURSOR_EXPIRED" {
                return Err("Log cursor escaped its filter".into());
            }
        }
        let mut forbidden = log_args;
        forbidden["packageName"] = json!("com.android.settings");
        let denied_logs = wire.tool("lomi_android_logcat", forbidden).await?;
        if denied_logs["structuredContent"]["code"] != "SCOPE_DENIED" {
            return Err("Android logs escaped package permission".into());
        }
        std::fs::write(directory.join("android-apps.json"), serde_json::to_vec_pretty(&json!({"launch":launched,"retry":repeated,"deniedLaunch":denied_launch,"logs":logs,"page":page,"deniedLogs":denied_logs})).unwrap()).map_err(|e|e.to_string())?;
        let snapshot_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"maxNodes":250,"maxBytes":32768});
        let observed = wire
            .tool("lomi_android_snapshot", snapshot_args.clone())
            .await?;
        let nodes = observed["structuredContent"]["data"]["nodes"]
            .as_array()
            .ok_or_else(|| format!("No Android hierarchy: {observed}"))?;
        if !nodes.iter().any(|node| {
            node["text"]
                .as_str()
                .is_some_and(|s| s.contains("Native Unicode"))
        }) || !nodes.iter().any(|node| {
            node["class"]
                .as_str()
                .is_some_and(|s| s.ends_with("EditText"))
                && node["valueOmitted"] == true
                && node["text"] == ""
        }) {
            return Err(
                "Android hierarchy did not contain the real fixture or omitted-field marker".into(),
            );
        }
        let mut limited_args = snapshot_args.clone();
        limited_args["maxNodes"] = json!(1);
        let limited = wire.tool("lomi_android_snapshot", limited_args).await?;
        if limited["structuredContent"]["data"]["truncated"] != true
            || limited["structuredContent"]["data"]["nodes"]
                .as_array()
                .map(Vec::len)
                != Some(1)
        {
            return Err("Android hierarchy exceeded the requested node budget".into());
        }
        let mut foreign_args = snapshot_args.clone();
        foreign_args["workspaceId"] = json!("foreign");
        let foreign = wire.tool("lomi_android_snapshot", foreign_args).await?;
        if foreign["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
            return Err("Android hierarchy crossed workspace scope".into());
        }
        let mut stale_args = snapshot_args;
        stale_args["generation"] = json!("00000000-0000-0000-0000-000000000000");
        let stale_snapshot = wire.tool("lomi_android_snapshot", stale_args).await?;
        if stale_snapshot["structuredContent"]["code"] != "STALE_GENERATION" {
            return Err("Android hierarchy accepted an old generation".into());
        }
        std::fs::write(directory.join("android-snapshot.json"), serde_json::to_vec_pretty(&json!({"snapshot":observed,"limited":limited,"foreign":foreign,"stale":stale_snapshot})).unwrap()).map_err(|e|e.to_string())?;
        let capture_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"maxEdge":480,"maxBytes":262144});
        let captured_android = wire
            .tool("lomi_android_screenshot", capture_args.clone())
            .await?;
        let android_artifact = &captured_android["structuredContent"]["data"]["artifact"];
        let phone_image = captured_android["content"]
            .as_array()
            .and_then(|c| c.iter().find(|i| i["type"] == "image"))
            .ok_or_else(|| format!("Missing Android PNG: {captured_android}"))?;
        let phone_png = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            phone_image["data"]
                .as_str()
                .ok_or("Missing Android image bytes")?,
        )
        .map_err(|e| e.to_string())?;
        let decoded = image::load_from_memory_with_format(&phone_png, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        if decoded.width() > 480
            || decoded.height() > 480
            || phone_png.len() > 262144
            || android_artifact["source"]["deviceId"] != device["deviceId"]
            || android_artifact["source"]["generation"] != generation
            || android_artifact["image"]["hardwareDisplay"] != json!([720, 1280])
            || android_artifact["image"]["crop"] != "full_display"
        {
            return Err("Android capture has invalid identity, hardware geometry or budget".into());
        }
        let phone_read_args =
            json!({"workspaceId":new_workspace,"artifactId":android_artifact["id"]});
        let phone_read = wire
            .tool("lomi_artifact_read", phone_read_args.clone())
            .await?;
        if phone_read["content"]
            .as_array()
            .and_then(|c| c.iter().find(|i| i["type"] == "image"))
            != Some(phone_image)
        {
            return Err("Android artifact bytes changed on reread".into());
        }
        let foreign = wire
            .tool(
                "lomi_artifact_read",
                json!({"workspaceId":"foreign","artifactId":android_artifact["id"]}),
            )
            .await?;
        if foreign["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
            return Err("Android capture artifact crossed workspace scope".into());
        }
        let mut stale = capture_args.clone();
        stale["generation"] = json!("00000000-0000-0000-0000-000000000000");
        let stale = wire.tool("lomi_android_screenshot", stale).await?;
        if stale["structuredContent"]["code"] != "STALE_GENERATION" {
            return Err("Android capture accepted stale generation".into());
        }
        std::fs::write(directory.join("android-capture.png"), phone_png)
            .map_err(|e| e.to_string())?;
        std::fs::write(directory.join("android-capture.json"), serde_json::to_vec_pretty(&json!({"artifact":android_artifact,"foreign":foreign,"stale":stale,"rereadExactBytes":true})).unwrap()).map_err(|e|e.to_string())?;
        if std::env::var("LOMI_MCP_ANDROID_INPUT_MODE").as_deref() != Ok("skip-screen-locked") {
            let window = app.get_window("main").ok_or("Missing main window")?;
            activate_main(app).await?;
            wait_for_human_focus(&main, "android-input").await?;
            for _ in 0..100 {
                if window.is_focused().unwrap_or(false) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let ui = javascript(&main, "const state=await window.__TAURI_INTERNALS__.invoke('android_state');return {visibility:document.visibilityState,focus:document.hasFocus(),text:document.body.innerText,streams:state.streams};").await?;
            let ui = json!({"ui":ui,"nativeFocused":window.is_focused().unwrap_or(false),"nativeVisible":window.is_visible().unwrap_or(false),"nativeMinimized":window.is_minimized().unwrap_or(true)});
            std::fs::write(
                directory.join("android-running-ui.json"),
                serde_json::to_vec_pretty(&ui).unwrap(),
            )
            .map_err(|e| e.to_string())?;
            wait_for(&main, "Boolean(document.querySelector('.android-screen'))").await?;
            javascript(&main, &format!("const m=await import('/src/android/runtime.ts');for(let n=0;n<80;n++){{if(m.agentInputReady({},{}))return true;await new Promise(r=>setTimeout(r,100));}}throw Error('Phone has not decoded a visible frame for input');", json!(panel), json!(generation))).await?;
            activate_main(app).await?;
            let mut focus_samples = 0;
            for _ in 0..40 {
                if window.is_focused().unwrap_or(false)
                    && evaluate(&main, "document.hasFocus()").await? == true
                {
                    focus_samples += 1;
                    if focus_samples == 3 {
                        break;
                    }
                } else {
                    focus_samples = 0;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if focus_samples != 3 {
                return Err("Android qualification requires stable native main-window focus before claiming input".into());
            }
            let projection = wire
                .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
                .await?;
            let claim_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"action":"claim","expectedRevision":projection["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-android-input-claim"});
            let claimed = wire.tool("lomi_panel_control", claim_args).await?;
            std::fs::write(
                directory.join("android-claim-request.json"),
                serde_json::to_vec_pretty(
                    &json!({"reply":claimed,"nativeFocused":window.is_focused().unwrap_or(false)}),
                )
                .unwrap(),
            )
            .map_err(|e| e.to_string())?;
            let claimed = wire
                .settled(
                    claimed["structuredContent"]["data"]["operationId"]
                        .as_str()
                        .ok_or_else(|| claimed.to_string())?,
                )
                .await?;
            if !claimed["structuredContent"]["data"]["result"]["leaseId"].is_string() {
                let evidence = javascript(
                    &main,
                    "return {text:document.body.innerText,visibility:document.visibilityState,focused:document.hasFocus()};",
                )
                .await?;
                std::fs::write(
                    directory.join("android-input-failure.json"),
                    serde_json::to_vec_pretty(&evidence).unwrap(),
                )
                .map_err(|e| e.to_string())?;
            }
            let lease = claimed["structuredContent"]["data"]["result"]["leaseId"]
                .as_str()
                .ok_or_else(|| format!("Android claim has no lease: {claimed}"))?;
            wait_for(
                &main,
                "document.body.textContent.includes('Agent input · Take control')",
            )
            .await?;
            let input_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"leaseId":lease,"inputSequence":"1","event":{"type":"text","text":"MCP Zażółć gęślą jaźń 🙂"},"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"]});
            let typed = wire.tool("lomi_android_input", input_args.clone()).await?;
            std::fs::write(
                directory.join("android-first-input-request.json"),
                serde_json::to_vec_pretty(
                    &json!({"reply":typed,"nativeFocused":window.is_focused().unwrap_or(false)}),
                )
                .unwrap(),
            )
            .map_err(|e| e.to_string())?;
            let typed = wire
                .settled(
                    typed["structuredContent"]["data"]["operationId"]
                        .as_str()
                        .ok_or_else(|| typed.to_string())?,
                )
                .await?;
            if typed["structuredContent"]["data"]["state"] != "succeeded" {
                return Err(format!("Android input failed: {typed}"));
            }
            let repeated = wire.tool("lomi_android_input", input_args.clone()).await?;
            if repeated["structuredContent"]["data"]["operationId"]
                != typed["structuredContent"]["data"]["operationId"]
            {
                return Err("Android input retry created another operation".into());
            }
            let mut changed = input_args.clone();
            changed["event"]["text"] = json!("CONFLICT");
            let conflict = wire.tool("lomi_android_input", changed).await?;
            if conflict["structuredContent"]["code"] != "IDEMPOTENCY_CONFLICT" {
                return Err("Android input sequence accepted different bytes".into());
            }
            let inspect = guest.clone();
            let hierarchy =
                tauri::async_runtime::spawn_blocking(move || inspect.native_input_fixture(false))
                    .await
                    .map_err(|e| e.to_string())??;
            if hierarchy.matches("MCP Zażółć").count() != 1 || hierarchy.contains("CONFLICT") {
                return Err("Guest hierarchy did not confirm exactly one MCP Unicode input".into());
            }
            std::fs::write(directory.join("android-input-hierarchy.xml"), hierarchy)
                .map_err(|e| e.to_string())?;
            let form_snapshot_args = json!({"workspaceId":new_workspace,"panelId":panel,
                "deviceId":device["deviceId"],"generation":generation});
            let form = wire
                .tool("lomi_android_snapshot", form_snapshot_args.clone())
                .await?;
            let form_data = &form["structuredContent"]["data"];
            if form_data["rotation"] != 0 {
                return Err("Form fixture requires the observed portrait display".into());
            }
            let submit = form_data["nodes"]
                .as_array()
                .and_then(|nodes| {
                    nodes
                        .iter()
                        .find(|node| node["description"] == "lomi-test-submit")
                })
                .ok_or_else(|| format!("Form submit control not visible: {form}"))?;
            let bounds = submit["bounds"].as_array().ok_or("Submit bounds missing")?;
            let x = (bounds[0].as_i64().ok_or("Invalid bounds")?
                + bounds[2].as_i64().ok_or("Invalid bounds")?)
                / 2;
            let y = (bounds[1].as_i64().ok_or("Invalid bounds")?
                + bounds[3].as_i64().ok_or("Invalid bounds")?)
                / 2;
            let mut touches = Vec::new();
            for (sequence, phase) in [("2", "down"), ("3", "up")] {
                let mut tap = input_args.clone();
                tap["inputSequence"] = json!(sequence);
                tap["event"] = json!({"type":"touch","space":"hardware_display","identifier":0,"x":x,"y":y,"phase":phase});
                let dispatched = wire.tool("lomi_android_input", tap).await?;
                let settled = wire
                    .settled(
                        dispatched["structuredContent"]["data"]["operationId"]
                            .as_str()
                            .ok_or_else(|| dispatched.to_string())?,
                    )
                    .await?;
                if settled["structuredContent"]["data"]["state"] != "succeeded" {
                    return Err(format!("Form touch failed: {settled}"));
                }
                touches.push(settled);
            }
            let submitted = wire
                .tool("lomi_android_snapshot", form_snapshot_args.clone())
                .await?;
            if !submitted["structuredContent"]["data"]["nodes"]
                .as_array()
                .is_some_and(|nodes| {
                    nodes.iter().any(|node| {
                        node["description"] == "lomi-test-result:1"
                            && node["text"] == "Submitted: MCP Zażółć gęślą jaźń 🙂"
                    })
                })
            {
                return Err(format!(
                    "Guest did not confirm the submitted Unicode form: {submitted}"
                ));
            }
            std::fs::write(
                directory.join("android-form.json"),
                serde_json::to_vec_pretty(
                    &json!({"before":form,"touches":touches,"submitted":submitted}),
                )
                .unwrap(),
            )
            .map_err(|e| e.to_string())?;
            screenshot(&main, directory.join("android-controlled.png")).await?;
            let mut key_receipts = Vec::new();
            key_receipts.push(
                wire.android_packet(
                    &input_args,
                    4,
                    json!({"type":"key","key":"Backspace","down":true}),
                )
                .await?,
            );
            key_receipts.push(
                wire.android_packet(
                    &input_args,
                    5,
                    json!({"type":"key","key":"Backspace","down":false}),
                )
                .await?,
            );
            let inspect = guest.clone();
            let after_key =
                tauri::async_runtime::spawn_blocking(move || inspect.native_input_fixture(false))
                    .await
                    .map_err(|e| e.to_string())??;
            std::fs::write(directory.join("android-key-deleted.xml"), &after_key)
                .map_err(|e| e.to_string())?;
            if fixture_editor_text(&after_key)? != "MCP Zażółć gęślą jaźń " {
                return Err("Native Backspace did not remove the final guest emoji".into());
            }
            key_receipts.push(
                wire.android_packet(&input_args, 6, json!({"type":"text","text":"🙂"}))
                    .await?,
            );
            let mut restored_text = String::new();
            let mut text_observations = 0;
            for attempt in 0..3 {
                let inspect = guest.clone();
                restored_text = tauri::async_runtime::spawn_blocking(move || {
                    inspect.native_input_fixture(false)
                })
                .await
                .map_err(|e| e.to_string())??;
                std::fs::write(
                    directory.join(format!("android-key-restored-{attempt}.xml")),
                    &restored_text,
                )
                .map_err(|e| e.to_string())?;
                text_observations += 1;
                if fixture_editor_text(&restored_text)? == "MCP Zażółć gęślą jaźń 🙂" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            if fixture_editor_text(&restored_text)? != "MCP Zażółć gęślą jaźń 🙂" {
                return Err(format!(
                    "Unicode input after key release did not restore guest text: {restored_text}"
                ));
            }
            std::fs::write(directory.join("android-keys.json"),serde_json::to_vec_pretty(&json!({"receipts":key_receipts,"removedWholeEmoji":true,"restoredExactText":true,"textObservations":text_observations})).unwrap()).map_err(|e|e.to_string())?;
            let mut rotations = Vec::new();
            for (sequence, quarter) in [(7_u64, 1_u8), (8, 0)] {
                let receipt = wire
                    .android_packet(
                        &input_args,
                        sequence,
                        json!({"type":"rotate","quarterTurns":quarter}),
                    )
                    .await?;
                let mut observation = Value::Null;
                for _ in 0..5 {
                    observation = wire
                        .tool("lomi_android_snapshot", form_snapshot_args.clone())
                        .await?;
                    if observation["structuredContent"]["data"]["rotation"] == quarter {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                if observation["structuredContent"]["data"]["rotation"] != quarter {
                    return Err(format!(
                        "Guest orientation did not reach {quarter}: {observation}"
                    ));
                }
                let image = wire
                    .tool("lomi_android_screenshot", capture_args.clone())
                    .await?;
                let geometry = &image["structuredContent"]["data"]["artifact"]["image"];
                let width = geometry["pixelWidth"]
                    .as_u64()
                    .ok_or_else(|| image.to_string())?;
                let height = geometry["pixelHeight"]
                    .as_u64()
                    .ok_or_else(|| image.to_string())?;
                if geometry["rotation"] != quarter
                    || geometry["hardwareDisplay"] != json!([720, 1280])
                    || (quarter == 1 && width <= height)
                    || (quarter == 0 && width >= height)
                {
                    return Err(format!("Rotated capture metadata mismatch: {geometry}"));
                }
                rotations
                    .push(json!({"receipt":receipt,"snapshot":observation,"geometry":geometry}));
            }
            std::fs::write(
                directory.join("android-rotation.json"),
                serde_json::to_vec_pretty(&rotations).unwrap(),
            )
            .map_err(|e| e.to_string())?;

            let projection = wire
                .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
                .await?;
            let release_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":generation,"action":"release","expectedRevision":projection["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-android-input-release"});
            let released = wire.tool("lomi_panel_control", release_args).await?;
            let released = wire
                .settled(
                    released["structuredContent"]["data"]["operationId"]
                        .as_str()
                        .ok_or_else(|| released.to_string())?,
                )
                .await?;
            if released["structuredContent"]["data"]["state"] != "succeeded"
                || released["structuredContent"]["data"]["result"]["controlled"] != false
            {
                return Err(format!("Android input release failed: {released}"));
            }
            let denied = wire.tool("lomi_android_input", input_args).await?;
            if denied["structuredContent"]["code"] != "CONTROL_REVOKED" {
                return Err("Released Android lease accepted input".into());
            }
            std::fs::write(directory.join("android-input.json"), serde_json::to_vec_pretty(&json!({"claimed":claimed,"typed":typed,"repeat":repeated,"conflict":conflict,"release":released,"staleLease":denied})).unwrap()).map_err(|e|e.to_string())?;
            eprintln!("MCP_ANDROID_INPUT_PASSED: input and rotation complete; foreground is no longer required");
        } else {
            std::fs::write(directory.join("android-input-unqualified.json"), b"{\"status\":\"NOT_RUN\",\"reason\":\"Host screen locked; native focus required. Run again without LOMI_MCP_ANDROID_INPUT_MODE after unlocking.\"}").map_err(|e|e.to_string())?;
        }
        let projection = wire
            .tool("lomi_panel_list", json!({"workspaceId":new_workspace}))
            .await?;
        let mut stop_args = json!({"workspaceId":new_workspace,"panelId":panel,"deviceId":device["deviceId"],"generation":"00000000-0000-0000-0000-000000000000","expectedRevision":projection["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-stop-android"});
        let stale = wire.tool("lomi_android_stop", stop_args.clone()).await?;
        if stale["structuredContent"]["code"] != "STALE_GENERATION" {
            return Err(format!("Stale Android stop accepted: {stale}"));
        }
        stop_args["generation"] = json!(generation);
        let stopped = wire.tool("lomi_android_stop", stop_args.clone()).await?;
        let operation = stopped["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| stopped.to_string())?;
        let stopped = wire.settled_with_limit(operation, 2400).await?;
        if stopped["structuredContent"]["data"]["state"] != "succeeded"
            || stopped["structuredContent"]["data"]["result"]["stopped"] != true
        {
            return Err(format!("Android stop not confirmed: {stopped}"));
        }
        let repeated = wire.tool("lomi_android_stop", stop_args).await?;
        if repeated["structuredContent"]["data"]["operationId"]
            != stopped["structuredContent"]["data"]["operationId"]
        {
            return Err("Android stop retry did not retain its receipt".into());
        }
        let status = wire
            .tool("lomi_android_list", json!({"workspaceId":new_workspace}))
            .await?;
        if status["structuredContent"]["data"]["devices"]["items"][0]["processAlive"] != false {
            return Err("Android process survived confirmed Stop".into());
        }
        let stopped_artifact = wire.tool("lomi_artifact_read", phone_read_args).await?;
        if stopped_artifact["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
            return Err("Stopped Android artifact remained readable".into());
        }
        std::fs::write(
            directory.join("android-capture-after-stop.json"),
            serde_json::to_vec_pretty(&stopped_artifact).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(directory.join("android-runtime.json"), serde_json::to_vec_pretty(&json!({"started":started,"running":running,"stale":stale,"stopped":stopped,"devices":status})).unwrap()).map_err(|e|e.to_string())?;
    }
    let mut all_events = Vec::new();
    let mut cursor = Value::Null;
    let mut last_sequence = 0_u64;
    let mut empty = false;
    for _ in 0..32 {
        let page = wire
            .tool(
                "lomi_events_read",
                json!({"workspaceId":workspace,"limit":100,"cursor":cursor}),
            )
            .await?;
        let data = &page["structuredContent"]["data"];
        let items = data["items"]
            .as_array()
            .ok_or_else(|| format!("Events failed: {page}"))?;
        if data["gap"] != false {
            return Err("Qualification event history was truncated".into());
        }
        for event in items {
            let sequence = event["sequence"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or("Invalid event sequence")?;
            if sequence <= last_sequence || event["workspaceId"] != workspace {
                return Err("Event cursor replayed or crossed workspace scope".into());
            }
            last_sequence = sequence;
            all_events.push(event.clone());
        }
        cursor = data["nextCursor"].clone();
        if items.is_empty() {
            empty = true;
            break;
        }
    }
    if !empty
        || !all_events
            .iter()
            .any(|event| event["operationId"] == cancel_id && event["state"] == "cancelled")
    {
        return Err("Scoped operation events did not drain or omitted cancellation".into());
    }
    std::fs::write(
        directory.join("events.json"),
        serde_json::to_vec_pretty(&json!({"items":all_events,"nextCursor":cursor})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let foreign_events = wire
        .tool(
            "lomi_events_read",
            json!({"workspaceId":"foreign-workspace","cursor":cursor}),
        )
        .await?;
    if foreign_events["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err("Foreign events visible".into());
    }
    let foreign = wire
        .tool("lomi_connect", json!({"workspaceId":"foreign-workspace"}))
        .await?;
    if foreign["structuredContent"]["code"] != "TARGET_NOT_FOUND" {
        return Err("Foreign workspace accepted".into());
    }
    let invalid = wire
        .call(
            "tools/call",
            json!({"name":"lomi_status","arguments":{"approve":true}}),
        )
        .await?;
    if invalid["error"]["code"] != -32602 {
        return Err("Unknown input field accepted".into());
    }
    let image = screenshot(&settings, directory.join("settings.png")).await?;
    let (hang_send, hang_receive) = tokio::sync::oneshot::channel();
    let hang_listener = app.once("mcp-probe-hang-start", move |_| {
        let _ = hang_send.send(());
    });
    let hang_started = std::time::Instant::now();
    main.eval("{const until=performance.now()+3000;window.__TAURI_INTERNALS__.invoke('plugin:event|emit',{event:'mcp-probe-hang-start',payload:null});while(performance.now()<until){};window.mcpHangFinished=true;}").map_err(|e|e.to_string())?;
    tokio::time::timeout(Duration::from_secs(5), hang_receive)
        .await
        .map_err(|_| "Main hang fixture did not start")?
        .map_err(|e| e.to_string())?;
    app.unlisten(hang_listener);
    if hang_started.elapsed() > Duration::from_secs(1) {
        return Err("Hang marker arrived too late to verify native independence".into());
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let revoke_started = std::time::Instant::now();
    crate::agent_control::revoke(app);
    let revoked = wire.tool("lomi_workspace_list", json!({})).await?;
    if revoked["structuredContent"]["code"] != "CONTROL_REVOKED" {
        return Err("Revoked connection disclosed data".into());
    }
    if revoke_started.elapsed() > Duration::from_secs(1) {
        return Err("Native revoke waited for the hung renderer".into());
    }
    wait_for(&main, "window.mcpHangFinished === true").await?;
    drop(wire);
    let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .map_err(|_| "Helper did not exit")?
        .map_err(|e| e.to_string())?;
    if !exit.success() {
        return Err("Helper failed on clean EOF".into());
    }
    let codex_config = directory.join("codex-config.json");
    let approval_ready = directory.join("codex-approval-ready");
    std::fs::write(
        &codex_config,
        serde_json::to_vec(&json!({"helper":helper,"expectedWorkspaceId":workspace,"approvalReadyPath":approval_ready})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let codex_log =
        std::fs::File::create(directory.join("codex.log")).map_err(|e| e.to_string())?;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/mcp/codex-probe.mjs");
    let mut codex = tokio::process::Command::new("node")
        .arg(script)
        .arg("--control")
        .arg(codex_config)
        .stdout(codex_log.try_clone().map_err(|e| e.to_string())?)
        .stderr(codex_log)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    // The independent client first verifies an unapproved read before this click.
    for _ in 0..900 {
        if approval_ready.is_file() {
            break;
        }
        if codex.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("Codex stopped before approval; inspect codex.log".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !approval_ready.is_file() {
        return Err("Codex did not reach its unapproved-read assertion".into());
    }
    let ready: Value =
        serde_json::from_slice(&std::fs::read(&approval_ready).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let request = ready["pairingRequestId"]
        .as_str()
        .ok_or("Missing Codex pairing request ID")?;
    let article=format!("Array.from(document.querySelectorAll('.agent-control-request')).find(a=>Array.from(a.querySelectorAll('code')).some(c=>c.textContent==={}))",json!(request));
    wait_for(&settings, &format!("Boolean({article})")).await?;
    let pending_count = evaluate(
        &settings,
        "document.querySelectorAll('.agent-control-request select').length",
    )
    .await?;
    std::fs::write(
        directory.join("codex-pairing.json"),
        serde_json::to_vec(&json!({"requestId":request,"pendingRequests":pending_count})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    evaluate(&settings,&format!("(() => {{const s=({article}).querySelector('select');s.value=Array.from(s.options).find(o=>o.textContent.includes('Renamed by MCP')).value;s.dispatchEvent(new Event('change',{{bubbles:true}}));return true;}})()")).await?;
    let approve=format!("Array.from(({article}).querySelectorAll('button')).find(b=>b.textContent.trim()==='Approve session')");
    wait_for(&settings, &format!("!({approve}).disabled")).await?;
    evaluate(&settings, &format!("({approve}).click();true")).await?;
    let exit = tokio::time::timeout(Duration::from_secs(60), codex.wait())
        .await
        .map_err(|_| "Codex qualification timed out")?
        .map_err(|e| e.to_string())?;
    if !exit.success() {
        return Err("Real Codex control qualification failed; inspect codex.log".into());
    }
    click(&settings, "Disable agent control").await?;
    if browser_monitor_count(app).await? != 0 {
        return Err("Disabled agent control retains browser input monitors".into());
    }
    wait_for(
        &settings,
        "document.body.textContent.includes('Agent control is off')",
    )
    .await?;
    Ok(
        json!({"androidInputQualification":if android_fixture.is_none() {"NOT_CONFIGURED"} else if std::env::var("LOMI_MCP_ANDROID_INPUT_MODE").as_deref() == Ok("skip-screen-locked") {"NOT_RUN_SCREEN_LOCKED"} else {"RUN"},"tools":catalog["result"]["tools"],"snapshot":image,"workspaceId":workspace,"checks":["native Settings opt-in","generated configuration","production helper process","unapproved read denied","main cannot approve grants","native Settings approval","workspace filtering","connect retry epoch","native workspace rename","real retained terminal create","terminal create deduplication","RAM-only terminal lease","native shell command and exit observation","Unicode terminal output","native xterm parser watermark","retained alternate screen and Unicode","command bounded to its own output","opt-in raw ANSI bytes","command side effect exactly once","input sequence deduplication and conflict","silent command remains running","explicit Ctrl+C","targeted interrupt and deduplicated retry","native operation cancellation","human input revokes agent lease","explicit Settings approval for terminal reclaim","claim denial and one-use retry","release invalidates input lease","reattachment preserves xterm sequence","ordinary user terminal approved without restart","Take control button revokes lease","panel identity and scoped cursors","native panel focus","human-owned terminal close rejected","file and idle terminal close with deduplicated receipt","workspace create without shell execution","workspace select with exact revision, retained PTY and deduplicated receipt","hidden terminal retained across workspace switch","deduplicated mutation retry","receipt recovery by request key","foreign workspace rejected","unknown arguments rejected","native revoke disconnects helper","native revoke while main JS is hung","scoped operation event cursor","real isolated browser open","browser dev server runs in an owned Lomi PTY","browser URL comes from its exact command operation","dev-server stays running, deduplicates start and exits through targeted interrupt","browser open receipt deduplication","foreign browser origin rejected","native redirect denied before network request","native browser navigation commit and load","navigation retry does not issue a second request","native navigation cancel and timeout close the HTTP response","stopped navigation reports unknown effects and never replays","navigation recovers after stop; log cursors expire","AppKit browser pointer revokes agent navigation","browser Take control revokes without navigation","browser native input monitors removed","hidden browser native creation preserves selection and provides DOM; image requires reveal","owned browser native close and deduplicated retry","human browser close denied and closed artifact unavailable","native isolated semantic DOM snapshot","private form values omitted","page-world globals cannot forge references","React form validation and Unicode fill","synthetic click isTrusted false","click retry executes once","stale snapshot and replaced node rejected","select and contenteditable fill","focused synthetic key with deduplication","wrong focused target rejected","viewport scroll with deduplication","bounded wait for asynchronous DOM","native deny delegate attached; media API availability recorded","bounded unmatched wait","SPA invalidates old references","expired JavaScript queue does not execute late input","bounded native PNG through standard MCP image content","artifact exact-byte reread","retained browser focus with stale-generation denial and retry","hidden native browser screenshot denied","isolated JavaScript error capture with explicit Promise-rejection limitation","log cursor pagination and bounded overflow","page and synthetic events cannot forge logs","log workspace scope and takeover denial","artifact workspace scope and human takeover denial","clean EOF","real Codex 0.156.1 connection","disable"]}),
    )
}

pub fn start(app: tauri::AppHandle) {
    let Some(directory) = std::env::var_os("LOMI_MCP_CONTROL_PROBE_DIRECTORY").map(PathBuf::from)
    else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let result = match clean_android_fixture(&app, &directory, "before").await {
            Ok(()) => run(&app, &directory).await,
            Err(error) => Err(format!("Android fixture preflight failed: {error}")),
        };
        if let Some(main) = app.get_webview("main") {
            if let Ok(trace) = javascript(&main, "return window.__domainConflicts ?? [];").await {
                let _ = std::fs::write(
                    directory.join("domain-conflicts.json"),
                    serde_json::to_vec_pretty(&trace).unwrap(),
                );
            }
        }
        if result.is_err() {
            if let Some(main) = app.get_webview("main") {
                let ui = javascript(&main,"return {visibility:document.visibilityState,focused:document.hasFocus(),text:document.body.innerText.slice(-8000),active:document.activeElement?.tagName};").await;
                let window = app.get_window("main");
                let state = json!({"ui":ui.ok(),"nativeFocused":window.as_ref().and_then(|w|w.is_focused().ok()),"nativeVisible":window.as_ref().and_then(|w|w.is_visible().ok()),"nativeMinimized":window.as_ref().and_then(|w|w.is_minimized().ok())});
                let _ = std::fs::write(
                    directory.join("failure-ui.json"),
                    serde_json::to_vec_pretty(&state).unwrap(),
                );
                let _ = screenshot(&main, directory.join("failure.png")).await;
            }
        }
        let result = match (
            result,
            clean_android_fixture(&app, &directory, "after").await,
        ) {
            (result, Ok(())) => result,
            (Ok(_), Err(error)) => Err(format!("Android fixture cleanup failed: {error}")),
            (Err(error), Err(cleanup)) => Err(format!(
                "{error}; Android fixture cleanup failed: {cleanup}"
            )),
        };
        let failed = result.is_err();
        let result = match result {
            Ok(data) => json!({"stage":"passed","data":data}),
            Err(error) => json!({"stage":"failed","error":error}),
        };
        let _ = std::fs::write(
            directory.join("result.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        );
        app.exit(i32::from(failed));
    });
}
