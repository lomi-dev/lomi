//! Chat through system CLIs: argv, JSONL stream parsers, status and model
//! probes, and the child process wrapper. Never starts a login flow.
use super::settings::Permissions;
use serde::Serialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdout, Command},
    task::JoinHandle,
};

pub(crate) const CLIS: [&str; 4] = ["claude", "codex", "kimi", "agy"];
/// Longest accepted stdout line; longer lines are dropped whole.
const LINE_CAP: usize = 1024 * 1024;
const STDERR_CAP: usize = 64 * 1024;
const STDERR_SHOWN: usize = 2000;
const MAX_MODELS: usize = 500;
/// kimi and agy take the prompt as an argument; bound it well below the OS argv limit.
const ARG_PROMPT_CAP: usize = 100 * 1024;

pub(crate) fn supported(cli: &str) -> bool {
    CLIS.contains(&cli)
}

pub(crate) fn display_name(cli: &str) -> &'static str {
    match cli {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "kimi" => "Kimi Code",
        _ => "Antigravity",
    }
}

/// Model and session IDs become argv values: reject anything an option parser could read as a flag.
pub(crate) fn valid_arg_id(id: &str, max: usize) -> bool {
    !id.is_empty()
        && id.chars().count() <= max
        && !id.starts_with('-')
        && !id.chars().any(|c| c.is_control() || c.is_whitespace())
}

pub(crate) fn valid_model(id: &str) -> Result<(), String> {
    if valid_arg_id(id, 256) {
        Ok(())
    } else {
        Err("Invalid CLI model ID.".into())
    }
}

pub(crate) fn valid_session(id: &str) -> bool {
    valid_arg_id(id, 128)
}

pub(crate) fn check_prompt(cli: &str, text: &str) -> Result<(), String> {
    if matches!(cli, "kimi" | "agy") && text.len() > ARG_PROMPT_CAP {
        return Err(format!(
            "This message is too long for {}.",
            display_name(cli)
        ));
    }
    Ok(())
}

// ---- argv ------------------------------------------------------------

pub(crate) struct Invocation<'a> {
    pub cli: &'a str,
    pub model: &'a str,
    pub cwd: &'a str,
    pub session: Option<&'a str>,
    pub system: &'a str,
    pub prompt: &'a str,
    /// Kimi read-only agent definition, bound when a session is created.
    pub agent_file: Option<&'a str>,
    pub permissions: Permissions,
    /// Reasoning effort; Kimi has no such option.
    pub effort: Option<&'a str>,
    /// Attachments folder to grant the CLI access to; set when files are attached.
    pub attach_dir: Option<&'a str>,
    /// Absolute paths of attached images (Codex `--image`).
    pub images: &'a [String],
}

/// Appends the attachment list to the prompt; `files` holds (absolute path, display name).
pub(crate) fn attach_prompt(text: &str, files: &[(String, String)]) -> String {
    if files.is_empty() {
        return text.to_string();
    }
    let sep = if text.is_empty() { "" } else { "\n\n" };
    let list: String = files
        .iter()
        .map(|(path, name)| format!("\n- {path} ({name})"))
        .collect();
    format!("{text}{sep}Attached files:{list}")
}

/// Returns the argument list and the stdin payload (when the CLI reads the prompt there).
pub(crate) fn argv(i: &Invocation) -> (Vec<String>, Option<String>) {
    let mut a: Vec<String> = Vec::new();
    let mut add = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    let stdin = match i.cli {
        "claude" => {
            add(&["-p", "--output-format", "stream-json", "--verbose"]);
            add(&["--include-partial-messages", "--model", i.model]);
            if let Some(d) = i.attach_dir {
                add(&[&format!("--add-dir={d}")]);
            }
            if let Some(sid) = i.session {
                add(&["--resume", sid]);
            }
            match i.permissions {
                Permissions::ReadOnly => {}
                Permissions::Edit => add(&["--permission-mode", "acceptEdits"]),
                Permissions::Full => add(&["--permission-mode", "bypassPermissions"]),
            }
            if let Some(e) = i.effort {
                add(&["--effort", e]);
            }
            if !i.system.is_empty() {
                add(&["--append-system-prompt", i.system]);
            }
            Some(i.prompt.to_string())
        }
        "codex" => {
            let mode = match i.permissions {
                Permissions::ReadOnly => Some("read-only"),
                Permissions::Edit => Some("workspace-write"),
                Permissions::Full => None,
            };
            match i.session {
                // `exec resume` has no `-s` or `-C`; the sandbox goes through config and the cwd through the process.
                Some(sid) => {
                    // `exec resume` has no `--add-dir`; the read-only sandbox still reads the files.
                    add(&["exec", "resume", sid, "--json", "--skip-git-repo-check"]);
                    if let Some(mode) = mode {
                        add(&["-c", &format!("sandbox_mode=\"{mode}\"")]);
                    }
                }
                None => {
                    add(&["exec", "--json", "--skip-git-repo-check"]);
                    if let Some(mode) = mode {
                        add(&["-s", mode]);
                    }
                    add(&[&format!("--cd={}", i.cwd)]);
                    if let Some(d) = i.attach_dir {
                        add(&[&format!("--add-dir={d}")]);
                    }
                }
            }
            for path in i.images {
                add(&[&format!("--image={path}")]);
            }
            if mode.is_none() {
                add(&["--dangerously-bypass-approvals-and-sandbox"]);
            }
            if let Some(e) = i.effort {
                add(&["-c", &format!("model_reasoning_effort=\"{e}\"")]);
            }
            add(&["-m", i.model, "-"]);
            Some(i.prompt.to_string())
        }
        "kimi" => {
            // `--prompt=` keeps a leading dash in the text from reading as a flag. Prompt mode
            // approves tools automatically and rejects `--plan`, so a new session binds a
            // read-only agent; resumed sessions restore that agent themselves.
            add(&[&format!("--prompt={}", i.prompt)]);
            add(&["--output-format", "stream-json", "-m", i.model]);
            if let Some(d) = i.attach_dir {
                add(&[&format!("--add-dir={d}")]);
            }
            match (i.session, i.agent_file) {
                (Some(sid), _) => add(&["-S", sid]),
                (None, Some(file)) => add(&[&format!("--agent-file={file}")]),
                (None, None) => {}
            }
            if i.permissions == Permissions::Full {
                add(&["--auto"]);
            }
            None
        }
        _ => {
            // A leading space keeps the text from reading as a flag.
            let prompt = if i.prompt.starts_with('-') {
                format!(" {}", i.prompt)
            } else {
                i.prompt.to_string()
            };
            add(&["-p", &prompt, "--output-format", "stream-json"]);
            add(&["--model", i.model]);
            if let Some(d) = i.attach_dir {
                add(&[&format!("--add-dir={d}")]);
            }
            if let Some(sid) = i.session {
                add(&["--conversation", sid]);
            }
            match i.permissions {
                Permissions::ReadOnly => add(&["--mode", "plan"]),
                Permissions::Edit => add(&["--mode", "accept-edits"]),
                Permissions::Full => add(&["--mode", "auto", "--dangerously-skip-permissions"]),
            }
            if let Some(e) = i.effort {
                add(&["--effort", e]);
            }
            None
        }
    };
    (a, stdin)
}

// ---- stream parsing --------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Ev {
    Text(String),
    Session(String),
    /// Complete answer; used only when no text streamed.
    Final(String),
    Error(String),
}

pub(crate) struct Parser {
    cli: String,
    had_text: bool,
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(k))?.as_str()
}

impl Parser {
    pub(crate) fn new(cli: &str) -> Self {
        Self {
            cli: cli.into(),
            had_text: false,
        }
    }

    /// Text separated from earlier text (new message or content block) starts a paragraph.
    fn text(&mut self, text: &str, separate: bool, out: &mut Vec<Ev>) {
        if text.is_empty() {
            return;
        }
        if separate && self.had_text {
            out.push(Ev::Text("\n\n".into()));
        }
        self.had_text = true;
        out.push(Ev::Text(text.into()));
    }

    /// Maps one stdout line; non-JSON and unknown lines are ignored.
    pub(crate) fn line(&mut self, line: &str) -> Vec<Ev> {
        let mut out = Vec::new();
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return out;
        };
        let kind = str_at(&v, &["type"]).unwrap_or("");
        match self.cli.as_str() {
            "claude" => match kind {
                "system" if str_at(&v, &["subtype"]) == Some("init") => {
                    out.extend(str_at(&v, &["session_id"]).map(|s| Ev::Session(s.into())));
                }
                // Sub-agent output is not part of the answer.
                "stream_event" if v["parent_tool_use_id"].is_null() => {
                    let e = &v["event"];
                    match str_at(e, &["type"]) {
                        Some("content_block_start")
                            if str_at(e, &["content_block", "type"]) == Some("text") =>
                        {
                            if self.had_text {
                                out.push(Ev::Text("\n\n".into()));
                                self.had_text = false; // one separator per block
                            }
                        }
                        Some("content_block_delta")
                            if str_at(e, &["delta", "type"]) == Some("text_delta") =>
                        {
                            let t = str_at(e, &["delta", "text"]).unwrap_or("");
                            self.text(t, false, &mut out);
                        }
                        _ => {}
                    }
                }
                "result" => {
                    out.extend(str_at(&v, &["session_id"]).map(|s| Ev::Session(s.into())));
                    let text = str_at(&v, &["result"]).unwrap_or("");
                    if v["is_error"] == true {
                        let detail = if text.is_empty() {
                            str_at(&v, &["subtype"]).unwrap_or("error")
                        } else {
                            text
                        };
                        out.push(Ev::Error(detail.into()));
                    } else {
                        out.push(Ev::Final(text.into()));
                    }
                }
                _ => {}
            },
            "codex" => match kind {
                "thread.started" => {
                    out.extend(str_at(&v, &["thread_id"]).map(|s| Ev::Session(s.into())));
                }
                "item.completed" if str_at(&v, &["item", "type"]) == Some("agent_message") => {
                    let t = str_at(&v, &["item", "text"]).unwrap_or("");
                    self.text(t, true, &mut out);
                }
                // Terminal event: the caller stops waiting for pipe EOF.
                "turn.completed" => out.push(Ev::Final(String::new())),
                "turn.failed" => out.push(Ev::Error(
                    str_at(&v, &["error", "message"])
                        .unwrap_or("The turn failed.")
                        .into(),
                )),
                "error" => out.push(Ev::Error(
                    str_at(&v, &["message"])
                        .unwrap_or("The CLI reported an error.")
                        .into(),
                )),
                _ => {}
            },
            "kimi" => {
                if str_at(&v, &["role"]) == Some("assistant") {
                    let t = str_at(&v, &["content"]).unwrap_or("");
                    self.text(t, true, &mut out);
                } else if str_at(&v, &["type"]) == Some("session.resume_hint") {
                    out.extend(str_at(&v, &["session_id"]).map(|s| Ev::Session(s.into())));
                }
            }
            _ => match str_at(&v, &["event"]) {
                Some("init") => {
                    out.extend(str_at(&v, &["conversation_id"]).map(|s| Ev::Session(s.into())));
                }
                Some("step_update")
                    if str_at(&v, &["step_update", "step_type"]) == Some("agent_response") =>
                {
                    let t = str_at(&v, &["step_update", "text_delta"]).unwrap_or("");
                    self.text(t, false, &mut out);
                }
                Some("result") => {
                    let r = &v["result"];
                    out.extend(str_at(r, &["conversation_id"]).map(|s| Ev::Session(s.into())));
                    match str_at(r, &["status"]) {
                        Some("SUCCESS") => {
                            out.push(Ev::Final(str_at(r, &["response"]).unwrap_or("").into()));
                        }
                        other => out.push(Ev::Error(format!(
                            "The CLI finished with status {}.",
                            other.unwrap_or("unknown")
                        ))),
                    }
                }
                _ => {}
            },
        }
        out
    }
}

/// Splits a byte stream into lines, dropping lines longer than `LINE_CAP`.
#[derive(Default)]
pub(crate) struct LineSplitter {
    buf: Vec<u8>,
    skipping: bool,
}

impl LineSplitter {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            let ends = part.last() == Some(&b'\n');
            if !self.skipping {
                self.buf.extend_from_slice(part);
                if self.buf.len() > LINE_CAP + 1 {
                    self.buf.clear();
                    self.skipping = !ends;
                    continue;
                }
            } else if !ends {
                continue;
            }
            if ends {
                if !self.skipping {
                    lines.push(self.take());
                }
                self.skipping = false;
            }
        }
        lines
    }

    pub(crate) fn finish(&mut self) -> Vec<String> {
        let rest = (!self.skipping && !self.buf.is_empty()).then(|| self.take());
        self.skipping = false;
        rest.into_iter().collect()
    }

    fn take(&mut self) -> String {
        let line = String::from_utf8_lossy(&self.buf).trim_end().to_string();
        self.buf.clear();
        line
    }
}

// ---- process ---------------------------------------------------------

pub(crate) struct Spec {
    pub bin: PathBuf,
    pub args: Vec<String>,
    pub stdin: Option<String>,
    pub cwd: PathBuf,
}

/// A running CLI in its own process group; dropping it kills the group.
pub(crate) struct Proc {
    pub child: Child,
    pub stdout: ChildStdout,
    stderr: JoinHandle<Vec<u8>>,
    /// Group id; kept after reaping because grandchildren can outlive the CLI.
    pgid: Option<u32>,
    reaped: bool,
}

/// GUI launches carry a minimal PATH; CLIs implemented as scripts need their runtime.
fn path_env(bin: &Path) -> Option<std::ffi::OsString> {
    let mut dirs: Vec<PathBuf> = bin.parent().map(Path::to_path_buf).into_iter().collect();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs.extend(std::env::split_paths(&std::env::var_os("PATH")?));
    std::env::join_paths(dirs).ok()
}

pub(crate) fn kill_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()).filter(|p| *p > 1) {
        // SAFETY: killpg only signals the group this process created with process_group(0).
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    let _ = pid;
}

impl Proc {
    pub(crate) fn spawn(spec: &Spec) -> Result<Self, String> {
        let mut cmd = Command::new(&spec.bin);
        cmd.args(&spec.args)
            .current_dir(&spec.cwd)
            .stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(path) = path_env(&spec.bin) {
            cmd.env("PATH", path);
        }
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "Cannot start the CLI. Check that it is installed.".to_string()
            } else {
                format!("Cannot start the CLI: {e}")
            }
        })?;
        let pgid = child.id();
        if let (Some(mut stdin), Some(text)) = (child.stdin.take(), spec.stdin.clone()) {
            // Written concurrently so a CLI that answers before reading everything cannot deadlock us.
            tokio::spawn(async move {
                let _ = stdin.write_all(text.as_bytes()).await;
            });
        }
        let stdout = child.stdout.take().ok_or("Cannot read the CLI output.")?;
        let mut err = child.stderr.take().ok_or("Cannot read the CLI output.")?;
        let stderr = tokio::spawn(async move {
            let (mut tail, mut buf) = (Vec::new(), [0u8; 4096]);
            while let Ok(n) = err.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > STDERR_CAP {
                    tail.drain(..tail.len() - STDERR_CAP);
                }
            }
            tail
        });
        Ok(Self {
            child,
            stdout,
            stderr,
            pgid,
            reaped: false,
        })
    }

    pub(crate) fn pgid(&self) -> Option<u32> {
        self.pgid
    }

    pub(crate) fn kill(&mut self) {
        kill_group(self.pgid);
        let _ = self.child.start_kill();
    }

    /// Waits for exit; returns the success flag, the exit description and a redacted stderr tail.
    pub(crate) async fn finish(&mut self) -> (bool, String, String) {
        let status = self.child.wait().await;
        self.reaped = true;
        // A grandchild (MCP server, hook) may hold the pipes open after the CLI exits.
        kill_group(self.pgid);
        let tail = tokio::time::timeout(Duration::from_secs(2), &mut self.stderr)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let from = tail.len().saturating_sub(STDERR_SHOWN);
        let text = super::http::redact(&String::from_utf8_lossy(&tail[from..]), None);
        match status {
            Ok(s) => (s.success(), s.to_string(), text),
            Err(_) => (false, "unknown exit status".into(), text),
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        if !self.reaped {
            self.kill();
        }
    }
}

// ---- status and models -----------------------------------------------

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CliStatus {
    pub cli: String,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub signed_in: Option<bool>,
    /// Subscription plan name, e.g. "Max 5x" or "ChatGPT Plus".
    pub plan: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct CliModel {
    pub id: String,
    pub label: String,
    /// Reasoning efforts the model accepts; empty when it has none.
    pub efforts: Vec<String>,
}

/// Efforts shared by every Claude Code and Antigravity model.
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Runs a short read-only probe; `None` on spawn failure or timeout. Returns (success, stdout+stderr).
async fn probe(bin: &Path, args: &[&str], secs: u64) -> Option<(bool, String)> {
    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(Stdio::null()).kill_on_drop(true);
    if let Some(path) = path_env(bin) {
        cmd.env("PATH", path);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(Duration::from_secs(secs), cmd.output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text.truncate(text.floor_char_boundary(STDERR_CAP));
    Some((out.status.success(), text))
}

pub(crate) fn parse_version(text: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"\d+\.\d+\.\d+\S*").expect("valid pattern"));
    re.find(text).map(|m| m.as_str().to_string())
}

/// `claude auth status` prints JSON on stdout.
pub(crate) fn parse_claude_status(text: &str) -> (Option<bool>, Option<String>) {
    let Ok(v) = serde_json::from_str::<Value>(text.trim()) else {
        return (None, None);
    };
    let plan = str_at(&v, &["planTier"])
        .or_else(|| str_at(&v, &["subscriptionType"]))
        .and_then(plan_label);
    (v["loggedIn"].as_bool(), plan)
}

/// `codex login status` exits 0 and prints "Logged in ..." when signed in.
pub(crate) fn parse_codex_status(success: bool, text: &str) -> Option<bool> {
    if success && text.contains("Logged in") {
        Some(true)
    } else if text.contains("Not logged in") {
        Some(false)
    } else {
        None
    }
}

/// `agy models` lines are `<id>\t<label>`; other lines are progress output.
pub(crate) fn parse_agy_models(text: &str) -> Vec<CliModel> {
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(id, label)| (id.trim(), label.trim()))
        .filter(|(id, _)| valid_arg_id(id, 256))
        .take(MAX_MODELS)
        .map(|(id, label)| CliModel {
            id: id.into(),
            label: if label.is_empty() { id } else { label }.into(),
            efforts: vec![],
        })
        .collect()
}

/// Reads `models[]` of Codex's `models_cache.json`, keeping user-visible entries.
pub(crate) fn parse_codex_models(json: &str) -> Vec<CliModel> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return vec![];
    };
    v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| {
            let id = m["slug"].as_str().filter(|id| valid_arg_id(id, 256))?;
            let label = m["display_name"].as_str().filter(|l| !l.is_empty());
            Some(CliModel {
                id: id.into(),
                label: label.unwrap_or(id).into(),
                efforts: m["supported_reasoning_levels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| l["effort"].as_str())
                    .filter(|e| super::settings::valid_effort(e).is_ok())
                    .map(String::from)
                    .collect(),
            })
        })
        .take(MAX_MODELS)
        .collect()
}

/// Kimi lists model aliases as `[models."<alias>"]` tables with an optional `display_name`.
pub(crate) fn parse_kimi_models(toml: &str) -> Vec<CliModel> {
    let Ok(doc) = toml.parse::<toml_edit::DocumentMut>() else {
        return vec![];
    };
    let Some(models) = doc.get("models").and_then(|m| m.as_table_like()) else {
        return vec![];
    };
    models
        .iter()
        .filter(|(id, _)| valid_arg_id(id, 256))
        .take(MAX_MODELS)
        .map(|(id, item)| CliModel {
            id: id.into(),
            label: item
                .get("display_name")
                .and_then(|l| l.as_str())
                .filter(|l| !l.is_empty())
                .unwrap_or(id)
                .into(),
            efforts: vec![],
        })
        .collect()
}

fn read_capped(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(file, 4 * 1024 * 1024), &mut text)
        .ok()?;
    Some(text)
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| crate::shell::home().join(".codex"), PathBuf::from)
}

/// The ChatGPT plan from the unverified `id_token` payload in Codex's `auth.json`.
/// Only the plan claim is read; tokens never leave this function.
fn codex_plan(auth: &str) -> Option<String> {
    use base64::Engine;
    let v: Value = serde_json::from_str(auth).ok()?;
    let payload = v["tokens"]["id_token"].as_str()?.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let plan = plan_label(claims["https://api.openai.com/auth"]["chatgpt_plan_type"].as_str()?)?;
    Some(format!("ChatGPT {plan}"))
}

/// Turns a plan id such as `max_5x` or `plus` into "Max 5x" / "Plus".
fn plan_label(id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty()
        || id.len() > 40
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    let words: Vec<String> = id
        .split(['_', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or_else(String::new, |f| {
                f.to_ascii_uppercase().to_string() + c.as_str()
            })
        })
        .collect();
    Some(words.join(" ").replace(" X", "x")).filter(|s| !s.is_empty())
}

fn kimi_home() -> PathBuf {
    crate::shell::home().join(".kimi-code")
}

/// Kimi keeps its OAuth login in `credentials/*.json`; absence proves nothing (API keys, env).
fn kimi_has_credentials() -> bool {
    std::fs::read_dir(kimi_home().join("credentials")).is_ok_and(|mut d| {
        d.any(|e| e.is_ok_and(|e| e.path().extension().is_some_and(|x| x == "json")))
    })
}

pub(crate) async fn status(cli: &str) -> CliStatus {
    let mut out = CliStatus {
        cli: cli.into(),
        binary_path: None,
        version: None,
        signed_in: None,
        plan: None,
        error: None,
    };
    let Some(bin) = crate::cli_bin::resolve(cli) else {
        return out;
    };
    out.binary_path = Some(bin.to_string_lossy().into_owned());
    match probe(&bin, &["--version"], 5).await {
        Some((_, text)) => out.version = parse_version(&text),
        None => out.error = Some("The CLI did not report its version in time.".into()),
    }
    match cli {
        "claude" => {
            if let Some((_, text)) = probe(&bin, &["auth", "status"], 10).await {
                (out.signed_in, out.plan) = parse_claude_status(&text);
            }
        }
        "codex" => {
            if let Some((ok, text)) = probe(&bin, &["login", "status"], 10).await {
                out.signed_in = parse_codex_status(ok, &text);
            }
            if out.signed_in == Some(true) {
                let path = codex_home().join("auth.json");
                out.plan = tauri::async_runtime::spawn_blocking(move || read_capped(&path))
                    .await
                    .ok()
                    .flatten()
                    .and_then(|text| codex_plan(&text));
            }
        }
        "kimi" => out.signed_in = kimi_has_credentials().then_some(true),
        // No status command: a successful model listing needs a valid session.
        _ => {
            if let Some((true, text)) = probe(&bin, &["models"], 10).await {
                out.signed_in = (!parse_agy_models(&text).is_empty()).then_some(true);
            }
        }
    }
    out
}

/// Models available to the signed-in Claude Code account, read with its OAuth login.
async fn claude_account_models() -> Vec<CliModel> {
    let Ok(Some(token)) =
        tauri::async_runtime::spawn_blocking(crate::cli_usage::claude_access_token).await
    else {
        return Vec::new();
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    else {
        return Vec::new();
    };
    let response = client
        .get("https://api.anthropic.com/v1/models?limit=1000")
        .bearer_auth(token.as_str())
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("anthropic-version", "2023-06-01")
        .send()
        .await;
    match response {
        Ok(r) if r.status().is_success() => match r.bytes().await {
            Ok(body) if body.len() <= 1 << 20 => parse_claude_models(&body),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Parses an Anthropic `/v1/models` page into `{id, label}` entries.
fn parse_claude_models(body: &[u8]) -> Vec<CliModel> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let mut out: Vec<CliModel> = Vec::new();
    for m in value["data"].as_array().into_iter().flatten() {
        let Some(id) = m["id"].as_str().filter(|id| valid_arg_id(id, 256)) else {
            continue;
        };
        if out.iter().any(|o| o.id == id) {
            continue;
        }
        out.push(CliModel {
            id: id.into(),
            label: m["display_name"].as_str().unwrap_or(id).into(),
            efforts: vec![],
        });
    }
    out.truncate(500);
    out
}

pub(crate) async fn models(cli: &str) -> Vec<CliModel> {
    let mut list = listed_models(cli).await;
    if matches!(cli, "claude" | "agy") {
        for m in &mut list {
            m.efforts = EFFORTS.map(String::from).into();
        }
    }
    list
}

async fn listed_models(cli: &str) -> Vec<CliModel> {
    match cli {
        "claude" => {
            let listed = claude_account_models().await;
            if listed.is_empty() {
                // Aliases always resolve to the latest models when the account list is unavailable.
                [
                    ("opus", "Claude Opus"),
                    ("sonnet", "Claude Sonnet"),
                    ("haiku", "Claude Haiku"),
                ]
                .map(|(id, label)| CliModel {
                    id: id.into(),
                    label: label.into(),
                    efforts: vec![],
                })
                .into()
            } else {
                listed
            }
        }
        "codex" => {
            let path = codex_home().join("models_cache.json");
            tauri::async_runtime::spawn_blocking(move || read_capped(&path))
                .await
                .ok()
                .flatten()
                .map(|t| parse_codex_models(&t))
                .unwrap_or_default()
        }
        "kimi" => {
            let path = kimi_home().join("config.toml");
            tauri::async_runtime::spawn_blocking(move || read_capped(&path))
                .await
                .ok()
                .flatten()
                .map(|t| parse_kimi_models(&t))
                .unwrap_or_default()
        }
        _ => match crate::cli_bin::resolve(cli) {
            Some(bin) => match probe(&bin, &["models"], 10).await {
                Some((true, text)) => parse_agy_models(&text),
                _ => vec![],
            },
            None => vec![],
        },
    }
}

/// Read-only Kimi agent: prompt mode would otherwise run edits and commands unasked.
pub(crate) const KIMI_AGENT: &str = "---\nname: lomi-chat\ndescription: Read-only chat assistant for Lomi\ntools:\n  - Read\n  - Grep\n  - Glob\n---\nYou are a helpful assistant. You never modify files or run commands.\n";

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLES: &str = "/private/tmp/claude-501/-Users-woro-Documents-Lomi/85242d7a-f6a3-4109-819b-55ebfbaafb03/scratchpad/cli-samples";

    fn run(cli: &str, jsonl: &str) -> Vec<Ev> {
        let mut p = Parser::new(cli);
        jsonl.lines().flat_map(|l| p.line(l)).collect()
    }

    // Trimmed real captures from the installed CLIs.
    const CLAUDE: &str = r#"{"type":"system","subtype":"hook_started","session_id":"x"}
{"type":"system","subtype":"init","cwd":"/w","session_id":"c825f374"}
{"type":"stream_event","event":{"type":"message_start","message":{}},"session_id":"c825f374","parent_tool_use_id":null}
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"session_id":"c825f374","parent_tool_use_id":null}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"O"}},"session_id":"c825f374","parent_tool_use_id":null}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"K"}},"session_id":"c825f374","parent_tool_use_id":null}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"sub"}},"session_id":"c825f374","parent_tool_use_id":"toolu_1"}
{"type":"assistant","message":{"content":[{"type":"text","text":"OK"}]}}
{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}},"session_id":"c825f374","parent_tool_use_id":null}
{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"!"}},"session_id":"c825f374","parent_tool_use_id":null}
{"type":"rate_limit_event"}
{"type":"result","subtype":"success","is_error":false,"result":"OK!","session_id":"c825f374"}"#;

    fn text(evs: &[Ev]) -> String {
        evs.iter()
            .filter_map(|e| match e {
                Ev::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn claude_stream() {
        let evs = run("claude", CLAUDE);
        assert_eq!(text(&evs), "OK\n\n!");
        assert_eq!(evs.first(), Some(&Ev::Session("c825f374".into())));
        assert_eq!(evs.last(), Some(&Ev::Final("OK!".into())));
        let err = run(
            "claude",
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Not logged in","session_id":"s"}"#,
        );
        assert_eq!(err.last(), Some(&Ev::Error("Not logged in".into())));
    }

    #[test]
    fn codex_stream() {
        let evs = run(
            "codex",
            r#"{"type":"thread.started","thread_id":"01a1"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"OK"}}
{"type":"item.completed","item":{"id":"item_1","type":"command_execution","text":"ls"}}
{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"Done"}}
{"type":"turn.completed","usage":{}}"#,
        );
        assert_eq!(
            evs,
            vec![
                Ev::Session("01a1".into()),
                Ev::Text("OK".into()),
                Ev::Text("\n\n".into()),
                Ev::Text("Done".into()),
                Ev::Final(String::new())
            ]
        );
        let failed = run(
            "codex",
            r#"{"type":"turn.failed","error":{"message":"quota"}}
{"type":"error","message":"boom"}
not json"#,
        );
        assert_eq!(
            failed,
            vec![Ev::Error("quota".into()), Ev::Error("boom".into())]
        );
    }

    #[test]
    fn kimi_stream() {
        let evs = run(
            "kimi",
            r#"{"role":"meta","type":"system.version","version":"2.7.0"}
{"role":"assistant","content":"OK"}
{"role":"tool","content":"ignored"}
{"role":"assistant","content":"More"}
{"role":"meta","type":"session.resume_hint","session_id":"session_2354","command":"kimi -r x"}"#,
        );
        assert_eq!(
            evs,
            vec![
                Ev::Text("OK".into()),
                Ev::Text("\n\n".into()),
                Ev::Text("More".into()),
                Ev::Session("session_2354".into())
            ]
        );
    }

    #[test]
    fn agy_stream() {
        let evs = run(
            "agy",
            r#"{"event":"init","conversation_id":"16b9","init":{}}
{"event":"step_update","step_update":{"step_index":0,"step_type":"user_input"}}
{"event":"step_update","step_update":{"step_index":1,"step_type":"agent_response","text_delta":"OK"}}
{"event":"step_update","step_update":{"step_index":1,"step_type":"agent_response","text_delta":"\n"}}
{"event":"result","result":{"conversation_id":"16b9","status":"SUCCESS","response":"OK\n"}}"#,
        );
        assert_eq!(text(&evs), "OK\n");
        assert_eq!(evs[0], Ev::Session("16b9".into()));
        assert_eq!(evs.last(), Some(&Ev::Final("OK\n".into())));
        let bad = run("agy", r#"{"event":"result","result":{"status":"ERROR"}}"#);
        assert!(matches!(bad.last(), Some(Ev::Error(_))));
    }

    #[test]
    fn real_samples_parse_when_present() {
        for (cli, file, want) in [
            ("claude", "claude.jsonl", "OK"),
            ("codex", "codex.jsonl", "OK"),
            ("kimi", "kimi.jsonl", "OK"),
            ("agy", "agy.jsonl", "OK\n"),
        ] {
            let Ok(data) = std::fs::read_to_string(Path::new(SAMPLES).join(file)) else {
                continue;
            };
            let evs = run(cli, &data);
            assert_eq!(text(&evs), want, "{cli}");
            assert!(evs.iter().any(|e| matches!(e, Ev::Session(_))), "{cli}");
        }
    }

    #[test]
    fn line_splitter_bounds_and_joins() {
        let mut s = LineSplitter::default();
        assert_eq!(s.push(b"ab"), Vec::<String>::new());
        assert_eq!(s.push(b"c\r\nd\ne"), vec!["abc", "d"]);
        assert_eq!(s.finish(), vec!["e"]);
        let mut s = LineSplitter::default();
        let huge = vec![b'x'; LINE_CAP + 10];
        assert!(s.push(&huge).is_empty());
        assert!(s.push(b"more").is_empty());
        assert_eq!(s.push(b"tail\nok\n"), vec!["ok"]);
        assert!(s.finish().is_empty());
    }

    fn inv<'a>(cli: &'a str, session: Option<&'a str>, system: &'a str) -> Invocation<'a> {
        Invocation {
            cli,
            model: "m1",
            cwd: "/w",
            session,
            system,
            prompt: "hi",
            agent_file: None,
            permissions: Permissions::ReadOnly,
            effort: None,
            attach_dir: None,
            images: &[],
        }
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn argv_fresh_and_resume() {
        let (a, stdin) = argv(&inv("claude", None, ""));
        assert_eq!(
            a,
            strs(&[
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--model",
                "m1"
            ])
        );
        assert_eq!(stdin.as_deref(), Some("hi"));
        let (a, _) = argv(&inv("claude", Some("s1"), "be brief"));
        assert_eq!(
            a[7..],
            strs(&["--resume", "s1", "--append-system-prompt", "be brief"])
        );

        let (a, stdin) = argv(&inv("codex", None, "ignored"));
        assert_eq!(
            a,
            strs(&[
                "exec",
                "--json",
                "--skip-git-repo-check",
                "-s",
                "read-only",
                "--cd=/w",
                "-m",
                "m1",
                "-"
            ])
        );
        assert_eq!(stdin.as_deref(), Some("hi"));
        let (a, _) = argv(&inv("codex", Some("t1"), ""));
        assert_eq!(
            a,
            strs(&[
                "exec",
                "resume",
                "t1",
                "--json",
                "--skip-git-repo-check",
                "-c",
                "sandbox_mode=\"read-only\"",
                "-m",
                "m1",
                "-"
            ])
        );

        let (a, stdin) = argv(&inv("kimi", Some("k1"), "ignored"));
        assert_eq!(
            a,
            strs(&[
                "--prompt=hi",
                "--output-format",
                "stream-json",
                "-m",
                "m1",
                "-S",
                "k1"
            ])
        );
        assert!(stdin.is_none());
        let fresh = Invocation {
            agent_file: Some("/d/kimi.md"),
            ..inv("kimi", None, "")
        };
        let (a, _) = argv(&fresh);
        assert!(!a.contains(&"-S".to_string()));
        assert_eq!(a.last().unwrap(), "--agent-file=/d/kimi.md");

        let (a, stdin) = argv(&inv("agy", None, ""));
        assert_eq!(
            a,
            strs(&[
                "-p",
                "hi",
                "--output-format",
                "stream-json",
                "--model",
                "m1",
                "--mode",
                "plan"
            ])
        );
        assert!(stdin.is_none());
        let (a, _) = argv(&inv("agy", Some("c1"), ""));
        assert_eq!(a[6..8], strs(&["--conversation", "c1"]));

        let dash = Invocation {
            prompt: "-x --plan",
            ..inv("kimi", None, "")
        };
        assert_eq!(argv(&dash).0[0], "--prompt=-x --plan");
        let dash = Invocation { cli: "agy", ..dash };
        assert_eq!(argv(&dash).0[1], " -x --plan");
    }

    #[test]
    fn argv_and_prompt_with_attachments() {
        let files = vec![
            ("/d/attachments/a1".to_string(), "pic.png".to_string()),
            ("/d/attachments/b2".to_string(), "n.txt".to_string()),
        ];
        assert_eq!(
            attach_prompt("hi", &files),
            "hi\n\nAttached files:\n- /d/attachments/a1 (pic.png)\n- /d/attachments/b2 (n.txt)"
        );
        assert_eq!(
            attach_prompt("", &files[..1]),
            "Attached files:\n- /d/attachments/a1 (pic.png)"
        );
        assert_eq!(attach_prompt("hi", &[]), "hi");
        let images = vec!["/d/attachments/a1".to_string()];
        let with = |cli, session| {
            argv(&Invocation {
                attach_dir: Some("/d/attachments"),
                images: &images,
                ..inv(cli, session, "")
            })
            .0
        };
        let has = |a: &[String], item: &str| a.iter().any(|v| v == item);
        for cli in ["claude", "kimi", "agy"] {
            assert!(has(&with(cli, None), "--add-dir=/d/attachments"), "{cli}");
            assert!(!has(&with(cli, None), "--image=/d/attachments/a1"), "{cli}");
        }
        let fresh = with("codex", None);
        assert!(
            has(&fresh, "--add-dir=/d/attachments") && has(&fresh, "--image=/d/attachments/a1")
        );
        let resumed = with("codex", Some("t1"));
        assert!(
            !has(&resumed, "--add-dir=/d/attachments")
                && has(&resumed, "--image=/d/attachments/a1")
        );
        // The stdin sentinel stays last so it cannot read as an image value.
        assert_eq!(fresh.last().unwrap(), "-");
        assert!(!has(
            &argv(&inv("claude", None, "")).0,
            "--add-dir=/d/attachments"
        ));
    }

    #[test]
    fn argv_permissions_and_effort() {
        let with = |cli, session, permissions, effort| {
            argv(&Invocation {
                permissions,
                effort,
                ..inv(cli, session, "")
            })
            .0
        };
        let has = |a: &[String], items: &[&str]| a.windows(items.len()).any(|w| w == strs(items));
        use Permissions::*;
        // claude
        let a = with("claude", None, Edit, Some("high"));
        assert!(has(&a, &["--permission-mode", "acceptEdits"]) && has(&a, &["--effort", "high"]));
        assert!(has(
            &with("claude", Some("s"), Full, None),
            &["--permission-mode", "bypassPermissions"]
        ));
        assert!(!with("claude", None, ReadOnly, None)
            .iter()
            .any(|v| v.contains("permission")));
        // codex, fresh and resume
        let a = with("codex", None, Edit, Some("max"));
        assert!(
            has(&a, &["-s", "workspace-write"])
                && has(&a, &["-c", "model_reasoning_effort=\"max\""])
        );
        let a = with("codex", Some("t"), Edit, None);
        assert!(has(&a, &["-c", "sandbox_mode=\"workspace-write\""]) && !a.contains(&"-s".into()));
        for session in [None, Some("t")] {
            let a = with("codex", session, Full, None);
            assert!(a.contains(&"--dangerously-bypass-approvals-and-sandbox".into()));
            assert!(!a.contains(&"-s".into()) && !a.iter().any(|v| v.starts_with("sandbox_mode")));
            assert_eq!(a.last().unwrap(), "-");
        }
        assert!(has(
            &with("codex", Some("t"), ReadOnly, None),
            &["-c", "sandbox_mode=\"read-only\""]
        ));
        // agy
        assert!(has(
            &with("agy", None, Edit, None),
            &["--mode", "accept-edits"]
        ));
        let a = with("agy", Some("c"), Full, Some("xhigh"));
        assert!(
            has(&a, &["--mode", "auto", "--dangerously-skip-permissions"])
                && has(&a, &["--effort", "xhigh"])
        );
        assert!(has(&with("agy", None, ReadOnly, None), &["--mode", "plan"]));
        // kimi ignores effort and uses --auto only for full access
        assert!(with("kimi", Some("k"), Full, Some("high")).contains(&"--auto".into()));
        let a = with("kimi", Some("k"), Edit, Some("high"));
        assert!(
            !a.contains(&"--auto".into()) && !a.iter().any(|v| v.contains("effort") || v == "high")
        );
    }

    #[test]
    fn argument_prompts_are_capped() {
        let long = "x".repeat(ARG_PROMPT_CAP + 1);
        assert_eq!(
            check_prompt("kimi", &long),
            Err("This message is too long for Kimi Code.".into())
        );
        assert!(check_prompt("agy", &long).is_err());
        assert!(check_prompt("claude", &long).is_ok());
        assert!(check_prompt("kimi", &long[1..]).is_ok());
    }

    #[test]
    fn spawn_errors_are_distinguished() {
        tauri::async_runtime::block_on(async {
            let mut spec = Spec {
                bin: "/nonexistent/lomi-cli".into(),
                args: vec![],
                stdin: None,
                cwd: std::env::temp_dir(),
            };
            let e = Proc::spawn(&spec).err().unwrap();
            assert!(e.contains("installed"), "{e}");
            spec.bin = std::env::temp_dir(); // a directory cannot be executed
            let e = Proc::spawn(&spec).err().unwrap();
            assert!(e.starts_with("Cannot start the CLI: "), "{e}");
        });
    }

    #[test]
    fn argument_ids_cannot_be_flags() {
        assert!(valid_model("gpt-5.2-codex").is_ok());
        assert!(valid_model("kimi-code/kimi-for-coding").is_ok());
        for bad in ["", "-m", "a b", "a\nb"] {
            assert!(valid_model(bad).is_err(), "{bad:?}");
        }
        assert!(valid_session("session_23542794-d101"));
        assert!(!valid_session("--resume"));
    }

    #[test]
    fn status_parsing() {
        assert_eq!(
            parse_version("2.1.133 (Claude Code)"),
            Some("2.1.133".into())
        );
        assert_eq!(
            parse_version("codex-cli 0.130.0-alpha.1\n"),
            Some("0.130.0-alpha.1".into())
        );
        assert_eq!(parse_version("none"), None);
        assert_eq!(
            parse_claude_status(
                r#"{"loggedIn":true,"email":"a@b.c","subscriptionType":"max","planTier":"max_5x"}"#
            ),
            (Some(true), Some("Max 5x".into()))
        );
        assert_eq!(
            parse_claude_status(r#"{"loggedIn":false}"#),
            (Some(false), None)
        );
        assert_eq!(parse_claude_status("garbage"), (None, None));
        assert_eq!(
            parse_codex_status(true, "Logged in using ChatGPT\n"),
            Some(true)
        );
        assert_eq!(parse_codex_status(false, "Not logged in\n"), Some(false));
        assert_eq!(parse_codex_status(false, "crashed"), None);
    }

    #[test]
    fn model_parsing() {
        let json = r#"{"fetched_at":"x","models":[
            {"slug":"gpt-6.2-sol","display_name":"GPT-6.2-Sol","visibility":"list",
             "supported_reasoning_levels":[{"effort":"low","description":"x"},{"effort":"ultra","description":"y"},{"effort":"-x"}]},
            {"slug":"hidden","display_name":"Hidden","visibility":"hide"},
            {"slug":"-bad","display_name":"Bad","visibility":"list"},
            {"slug":"nolabel","visibility":"list"}]}"#;
        assert_eq!(
            parse_codex_models(json),
            vec![
                CliModel {
                    id: "gpt-6.2-sol".into(),
                    label: "GPT-6.2-Sol".into(),
                    efforts: vec!["low".into(), "ultra".into()]
                },
                CliModel {
                    id: "nolabel".into(),
                    label: "nolabel".into(),
                    efforts: vec![]
                },
            ]
        );
        assert!(parse_codex_models("nope").is_empty());
        let toml = "default_model = \"a\"\n[models.\"kimi-code/k3\"]\nprovider = \"p\"\ndisplay_name = \"K3\"\n[models.plain]\nprovider = \"p\"\n";
        assert_eq!(
            parse_kimi_models(toml),
            vec![
                CliModel {
                    id: "kimi-code/k3".into(),
                    label: "K3".into(),
                    efforts: vec![]
                },
                CliModel {
                    id: "plain".into(),
                    label: "plain".into(),
                    efforts: vec![]
                },
            ]
        );
        assert_eq!(
            parse_agy_models("Fetching available models...\ngemini-x\tGemini X\n"),
            vec![CliModel {
                id: "gemini-x".into(),
                label: "Gemini X".into(),
                efforts: vec![]
            }]
        );
    }

    #[cfg(unix)]
    fn sh(script: &str, stdin: Option<&str>) -> Spec {
        Spec {
            bin: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            stdin: stdin.map(Into::into),
            cwd: std::env::temp_dir(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn process_streams_stdout_stdin_and_stderr_tail() {
        tauri::async_runtime::block_on(async {
            let mut p = Proc::spawn(&sh(
                "read line; echo \"got $line\"; echo oops >&2; exit 3",
                Some("hello\n"),
            ))
            .unwrap();
            let mut out = String::new();
            p.stdout.read_to_string(&mut out).await.unwrap();
            assert_eq!(out, "got hello\n");
            let (ok, status, stderr) = p.finish().await;
            assert!(!ok);
            assert!(status.contains('3'), "{status}");
            assert_eq!(stderr, "oops");
        });
    }

    #[cfg(unix)]
    #[test]
    fn finish_kills_grandchildren_holding_the_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!("sleep 60 & echo $! > {}; exit 0", pid_file.display());
        let started = std::time::Instant::now();
        tauri::async_runtime::block_on(async {
            let mut p = Proc::spawn(&sh(&script, None)).unwrap();
            let (ok, _, _) = p.finish().await;
            assert!(ok);
        });
        assert!(started.elapsed() < Duration::from_secs(10));
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..100 {
            // SAFETY: signal 0 only probes for existence.
            if unsafe { libc::kill(pid, 0) } != 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("grandchild {pid} survived");
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_process_kills_its_group() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!("sleep 60 & echo $! > {}; wait", pid_file.display());
        tauri::async_runtime::block_on(async {
            let p = Proc::spawn(&sh(&script, None)).unwrap();
            for _ in 0..100 {
                if std::fs::read_to_string(&pid_file).is_ok_and(|t| t.ends_with('\n')) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            drop(p);
        });
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..100 {
            // SAFETY: signal 0 only probes for existence.
            if unsafe { libc::kill(pid, 0) } != 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("grandchild {pid} survived");
    }

    #[test]
    fn claude_models_parse() {
        let body = br#"{"data":[{"id":"claude-opus-5-5","display_name":"Claude Opus 5.5"},{"id":"claude-opus-5-5"},{"id":"-bad"},{"id":"claude-haiku-5-5"}]}"#;
        let m = parse_claude_models(body);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].label, "Claude Opus 5.5");
        assert_eq!(m[1].label, "claude-haiku-5-5");
        assert!(parse_claude_models(b"nope").is_empty());
    }

    #[test]
    fn plan_names() {
        use base64::Engine;
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            br#"{"email":"a@b.co","https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}"#,
        );
        let auth = format!(r#"{{"tokens":{{"id_token":"h.{claims}.s"}}}}"#);
        assert_eq!(codex_plan(&auth).as_deref(), Some("ChatGPT Plus"));
        assert_eq!(codex_plan("{}"), None);
        assert_eq!(plan_label("max_20x").as_deref(), Some("Max 20x"));
        assert_eq!(plan_label("pro").as_deref(), Some("Pro"));
        assert_eq!(plan_label("bad plan!"), None);
    }
}
