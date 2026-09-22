use crate::cli_config::{read, revision};
use crate::{files::main_window, terminal::Terminals};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(any(target_os = "linux", test))]
use std::fs;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{State, Window};
use toml_edit::{Array, DocumentMut, Item, Table};

const LIMIT: u64 = 1024 * 1024;

#[derive(Default)]
pub struct CliTitleConfig(pub(crate) Mutex<()>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleSetup {
    cli: TitleCli,
    path: String,
    revision: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TitleCli {
    Codex,
    Agy,
    Cursor,
    Claude,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TitleProcess {
    pub cli: TitleCli,
    pub pid: u32,
}

#[cfg(target_os = "linux")]
fn identify(executable: &Path, argv: &[u8]) -> Option<TitleCli> {
    use std::os::unix::ffi::OsStrExt;
    match executable.file_name()?.to_str()? {
        "codex" => Some(TitleCli::Codex),
        "agy" => Some(TitleCli::Agy),
        "claude" | "claude.exe" => Some(TitleCli::Claude),
        "cursor-agent" | "cursor-agent-sea" => Some(TitleCli::Cursor),
        "node" | "nodejs" | "bun" => {
            // Match the launcher path, never prompt text or shell command contents.
            let mut args = argv.split(|byte| *byte == 0);
            let invoked = Path::new(std::ffi::OsStr::from_bytes(args.next()?));
            if invoked.file_name().is_some_and(|name| name == "claude") {
                return Some(TitleCli::Claude);
            }
            let script = args.find(|arg| !arg.starts_with(b"--"))?;
            let script = Path::new(std::ffi::OsStr::from_bytes(script));
            if script.ends_with("@anthropic-ai/claude-code/cli.js") {
                return Some(TitleCli::Claude);
            }
            if script.file_name().is_some_and(|name| name == "index.js")
                && script.parent() == executable.parent()
                && executable.with_file_name("cursor-agent").is_file()
            {
                return Some(TitleCli::Cursor);
            }
            None
        }
        // Native Claude installations use the version number as the binary filename.
        _ if executable.parent()?.ends_with("claude/versions") => Some(TitleCli::Claude),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
pub fn process_in_group(group: u32) -> Option<TitleProcess> {
    let mut pending = vec![group];
    // Bound /proc traversal to 64 wrapper descendants per terminal group.
    for _ in 0..64 {
        let pid = pending.pop()?;
        let root = PathBuf::from(format!("/proc/{pid}"));
        let Ok(stat) = fs::read_to_string(root.join("stat")) else {
            continue;
        };
        let process_group = stat
            .rsplit_once(") ")
            .and_then(|(_, fields)| fields.split_whitespace().nth(2))
            .and_then(|value| value.parse::<u32>().ok());
        if process_group != Some(group) {
            continue;
        }
        if let Ok(executable) = fs::read_link(root.join("exe")) {
            let mut argv = Vec::new();
            if executable
                .file_name()
                .is_some_and(|name| name == "node" || name == "nodejs" || name == "bun")
            {
                if let Ok(file) = fs::File::open(root.join("cmdline")) {
                    let _ = file.take(4096).read_to_end(&mut argv);
                }
            }
            if let Some(cli) = identify(&executable, &argv) {
                return Some(TitleProcess { cli, pid });
            }
        }
        if let Ok(children) = fs::read_to_string(root.join(format!("task/{pid}/children"))) {
            pending.extend(
                children
                    .split_whitespace()
                    .filter_map(|pid| pid.parse::<u32>().ok())
                    .take(64 - pending.len().min(64)),
            );
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn configuration(process: TitleProcess) -> Result<(PathBuf, bool), String> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = Vec::new();
    fs::File::open(format!("/proc/{}/environ", process.pid))
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|_| "Cannot read the running CLI configuration location.")?;
    if bytes.len() as u64 > LIMIT {
        return Err("The CLI process environment exceeds 1 MiB.".into());
    }
    // Only configuration locations and the title toggle leave this function, never credentials.
    let variable = |name: &[u8]| {
        bytes
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(name).filter(|value| !value.is_empty()))
    };
    let path_variable =
        |name: &[u8]| variable(name).map(|value| PathBuf::from(std::ffi::OsStr::from_bytes(value)));
    let home = path_variable(b"HOME=");
    let (directory, filename) = match process.cli {
        TitleCli::Codex => (
            path_variable(b"CODEX_HOME=").or_else(|| home.map(|home| home.join(".codex"))),
            "config.toml",
        ),
        TitleCli::Agy => (
            home.map(|home| home.join(".gemini/antigravity-cli")),
            "settings.json",
        ),
        TitleCli::Cursor => (
            path_variable(b"CURSOR_CONFIG_DIR=")
                .or_else(|| path_variable(b"XDG_CONFIG_HOME=").map(|home| home.join("cursor")))
                .or_else(|| home.map(|home| home.join(".cursor"))),
            "cli-config.json",
        ),
        TitleCli::Claude => (
            path_variable(b"CLAUDE_CONFIG_DIR=").or_else(|| home.map(|home| home.join(".claude"))),
            "settings.json",
        ),
    };
    let directory = directory.ok_or("Cannot locate the running CLI configuration directory.")?;
    if !directory.is_absolute() {
        return Err("CLI title setup requires an absolute configuration directory.".into());
    }
    // Resolve existing ancestors without creating directories before consent.
    let mut ancestor = directory.as_path();
    let mut missing = Vec::new();
    while !ancestor.try_exists().map_err(|error| error.to_string())? {
        missing.push(
            ancestor
                .file_name()
                .ok_or("Invalid CLI configuration directory.")?,
        );
        ancestor = ancestor
            .parent()
            .ok_or("Invalid CLI configuration directory.")?;
    }
    let mut path = ancestor.canonicalize().map_err(|error| error.to_string())?;
    for component in missing.iter().rev() {
        path.push(component);
    }
    path.push(filename);
    let path = match path.symlink_metadata() {
        Ok(_) => path.canonicalize().map_err(|error| error.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path,
        Err(error) => return Err(error.to_string()),
    };
    let disabled = variable(b"CLAUDE_CODE_DISABLE_TERMINAL_TITLE=")
        .is_some_and(|value| truthy(&String::from_utf8_lossy(value)));
    Ok((path, disabled))
}

#[cfg(not(target_os = "linux"))]
fn configuration(_process: TitleProcess) -> Result<(PathBuf, bool), String> {
    Err("Automatic CLI title setup is currently available on Linux.".into())
}

fn truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn document(source: Option<&str>) -> Result<DocumentMut, String> {
    let doc = source
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|_| "CLI configuration is not valid TOML. The file was left intact.")?;
    if let Some(tui) = doc.get("tui") {
        let table = tui
            .as_table_like()
            .ok_or("Codex tui settings must be a TOML table.")?;
        if let Some(titles) = table.get("terminal_title") {
            if !titles
                .as_array()
                .is_some_and(|items| items.iter().all(|item| item.is_str()))
            {
                return Err(
                    "Codex terminal_title must be an array of strings. The file was left intact."
                        .into(),
                );
            }
        }
    }
    Ok(doc)
}

fn agy_command() -> Result<String, String> {
    let executable = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe().map_err(|error| error.to_string())?)
        .canonicalize()
        .map_err(|_| "Cannot locate Lomi's title formatter. Restart Lomi and try again.")?;
    let path = executable
        .to_str()
        .ok_or("The Lomi executable path is not UTF-8.")?;
    Ok(format!(
        "{} --agy-terminal-title",
        crate::shell::quote(path, "bash")?
    ))
}

/// Resolves the active conversation's title without opening a window or reading transcripts.
pub fn print_agy_title() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin()
        .take(LIMIT + 1)
        .read_to_string(&mut input)
        .map_err(|_| "Cannot read agy title data.")?;
    if input.len() as u64 > LIMIT {
        return Err("agy title data exceeds 1 MiB.".into());
    }
    let data: Value = serde_json::from_str(&input).map_err(|_| "Invalid agy title data.")?;
    if !data.is_object() {
        return Err("agy title data must be a JSON object.".into());
    }
    let annotations = crate::shell::home().join(".gemini/antigravity-cli/annotations");
    println!("{}", agy_title(&data, &annotations));
    Ok(())
}

fn agy_annotation_title(source: &str) -> Option<String> {
    // agy writes title as the first protobuf text field; never match text inside tags.
    let field = regex::Regex::new(r#"^\s*title\s*:\s*("(?:\\.|[^"\\])*")"#).ok()?;
    let quoted = field.captures(source)?.get(1)?.as_str();
    let escapes = regex::Regex::new(r#"\\\\|\\x([0-9a-fA-F]{2})|\\U([0-9a-fA-F]{8})"#).ok()?;
    let quoted = escapes.replace_all(quoted, |captures: &regex::Captures<'_>| {
        if let Some(hex) = captures.get(1) {
            format!("\\u00{}", hex.as_str())
        } else if let Some(hex) = captures.get(2) {
            u32::from_str_radix(hex.as_str(), 16)
                .ok()
                .and_then(char::from_u32)
                .and_then(|character| serde_json::to_string(&character.to_string()).ok())
                .map(|quoted| quoted[1..quoted.len() - 1].to_owned())
                .unwrap_or_else(|| captures[0].to_owned())
        } else {
            captures[0].to_owned()
        }
    });
    serde_json::from_str::<String>(&quoted)
        .ok()
        .filter(|title| !title.trim().is_empty())
}

fn agy_title(data: &Value, annotations: &Path) -> String {
    let id = data["conversation_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .or_else(|| data["session_id"].as_str())
        .filter(|id| !id.is_empty());
    let title = id
        .and_then(|id| {
            if id.len() > 128
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return None;
            }
            let path = annotations.join(format!("{id}.pbtxt"));
            if !path.symlink_metadata().ok()?.is_file() {
                return None;
            }
            // The summaries database may lag behind active conversations and /resume renames.
            agy_annotation_title(&read(&path).ok()??)
        })
        .or_else(|| {
            data["conversation_title"]
                .as_str()
                .filter(|title| !title.trim().is_empty())
                .map(str::to_owned)
        });
    title
        .as_deref()
        .unwrap_or("agy")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

fn json_document(cli: TitleCli, source: Option<&str>) -> Result<Value, String> {
    let doc: Value = serde_json::from_str(source.unwrap_or("{}"))
        .map_err(|_| "CLI configuration is not valid JSON. The file was left intact.")?;
    if !doc.is_object() {
        return Err("CLI configuration must be a JSON object.".into());
    }
    let (section, key) = match cli {
        TitleCli::Agy => ("title", "enabled"),
        TitleCli::Cursor => ("display", "showStatusIndicators"),
        TitleCli::Claude => ("env", "CLAUDE_CODE_DISABLE_TERMINAL_TITLE"),
        TitleCli::Codex => unreachable!(),
    };
    if let Some(settings) = doc.get(section) {
        if !settings.is_object() {
            return Err(format!("CLI {section} settings must be a JSON object."));
        }
        if let Some(value) = settings.get(key) {
            if !(if cli == TitleCli::Claude {
                value.is_string()
            } else {
                value.is_boolean()
            }) {
                return Err(format!(
                    "CLI {section}.{key} has an invalid type. The file was left intact."
                ));
            }
        }
    }
    if cli == TitleCli::Agy {
        for key in ["type", "command"] {
            if doc
                .get("title")
                .and_then(|title| title.get(key))
                .is_some_and(|value| !value.is_string())
            {
                return Err(format!("agy title.{key} must be a string."));
            }
        }
    }
    if cli == TitleCli::Claude
        && doc
            .get("terminalTitleFromRename")
            .is_some_and(|value| !value.is_boolean())
    {
        return Err("Claude Code terminalTitleFromRename must be a boolean.".into());
    }
    Ok(doc)
}

fn configured(cli: TitleCli, source: Option<&str>, disabled: bool) -> Result<bool, String> {
    if cli == TitleCli::Codex {
        let doc = document(source)?;
        return Ok(doc
            .get("tui")
            .and_then(|tui| tui.get("terminal_title"))
            .and_then(Item::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some("thread-title"))
            }));
    }
    let doc = json_document(cli, source)?;
    Ok(match cli {
        TitleCli::Agy => {
            doc["title"]["command"]
                .as_str()
                .is_some_and(|command| !command.trim().is_empty())
                && doc["title"]["enabled"].as_bool() != Some(false)
        }
        TitleCli::Cursor => doc["display"]["showStatusIndicators"] == true,
        TitleCli::Claude => {
            !doc["env"]["CLAUDE_CODE_DISABLE_TERMINAL_TITLE"]
                .as_str()
                .map(truthy)
                .unwrap_or(disabled)
                && doc["terminalTitleFromRename"].as_bool() != Some(false)
        }
        TitleCli::Codex => unreachable!(),
    })
}

fn inspect(cli: TitleCli, path: &Path, disabled: bool) -> Result<Option<TitleSetup>, String> {
    let source = read(path)?;
    Ok(
        (!configured(cli, source.as_deref(), disabled)?).then(|| TitleSetup {
            cli,
            path: path.to_string_lossy().into_owned(),
            revision: revision(source.as_deref()),
        }),
    )
}

fn enable(cli: TitleCli, path: &Path, expected: Option<&str>) -> Result<(), String> {
    let source = read(path)?;
    let conflict =
        "CLI configuration changed. Check the settings again before allowing the update.";
    if revision(source.as_deref()).as_deref() != expected {
        return Err(conflict.into());
    }
    let output = if cli == TitleCli::Codex {
        let mut doc = document(source.as_deref())?;
        let tui = doc
            .entry("tui")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or("Codex tui settings must be a TOML table.")?;
        let titles = tui.entry("terminal_title").or_insert(Item::None);
        let mut value =
            toml_edit::Value::Array(["activity", "thread-title"].into_iter().collect::<Array>());
        if let Some(previous) = titles.as_value() {
            *value.decor_mut() = previous.decor().clone();
        }
        *titles = Item::Value(value);
        doc.to_string()
    } else {
        let mut doc = json_document(cli, source.as_deref())?;
        match cli {
            TitleCli::Agy => {
                if doc["title"]["command"]
                    .as_str()
                    .is_none_or(|command| command.trim().is_empty())
                {
                    doc["title"]["type"] = json!("command");
                    doc["title"]["command"] = json!(agy_command()?);
                }
                doc["title"]["enabled"] = json!(true);
            }
            TitleCli::Cursor => {
                doc["display"]["showStatusIndicators"] = json!(true);
            }
            TitleCli::Claude => {
                doc["env"]["CLAUDE_CODE_DISABLE_TERMINAL_TITLE"] = json!("0");
                if doc["terminalTitleFromRename"] == false {
                    doc["terminalTitleFromRename"] = json!(true);
                }
            }
            TitleCli::Codex => unreachable!(),
        }
        format!(
            "{}\n",
            serde_json::to_string_pretty(&doc).map_err(|error| error.to_string())?
        )
    };
    crate::cli_config::write(path, source.as_deref(), output)
}

#[tauri::command]
pub async fn inspect_cli_titles(
    window: Window,
    terminals: State<'_, Terminals>,
    state: State<'_, CliTitleConfig>,
    id: String,
    process: TitleProcess,
) -> Result<Option<TitleSetup>, String> {
    main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    terminals.check_title_process(&id, process)?;
    let (path, disabled) = configuration(process)?;
    inspect(process.cli, &path, disabled)
}

#[tauri::command]
pub async fn enable_cli_titles(
    window: Window,
    terminals: State<'_, Terminals>,
    state: State<'_, CliTitleConfig>,
    id: String,
    process: TitleProcess,
    path: String,
    revision: Option<String>,
) -> Result<(), String> {
    main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    terminals.check_title_process(&id, process)?;
    let (current, _) = configuration(process)?;
    if current != Path::new(&path) {
        return Err(
            "The running CLI configuration location changed. Check the settings again.".into(),
        );
    }
    enable(process.cli, &current, revision.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_json_title_settings_without_replacing_customizations() {
        for (cli, source) in [
            (
                TitleCli::Agy,
                r#"{"title":{"type":"command","command":"my-title --custom","enabled":false},"notifications":true}"#,
            ),
            (
                TitleCli::Cursor,
                r#"{"display":{"showStatusIndicators":false,"showLineNumbers":true},"permissions":{"allow":["Shell(ls)"],"deny":["Shell(rm)"]}}"#,
            ),
            (
                TitleCli::Claude,
                r#"{"env":{"CLAUDE_CODE_DISABLE_TERMINAL_TITLE":"1","CUSTOM":"keep"},"terminalTitleFromRename":false,"hooks":{"Stop":[]}}"#,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("settings.json");
            fs::write(&path, source).unwrap();
            let setup = inspect(cli, &path, false).unwrap().unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            enable(cli, &path, setup.revision.as_deref()).unwrap();
            assert!(inspect(cli, &path, false).unwrap().is_none());
            let mut before: Value = serde_json::from_str(source).unwrap();
            let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            match cli {
                TitleCli::Agy => before["title"]["enabled"] = json!(true),
                TitleCli::Cursor => before["display"]["showStatusIndicators"] = json!(true),
                TitleCli::Claude => {
                    before["env"]["CLAUDE_CODE_DISABLE_TERMINAL_TITLE"] = json!("0");
                    before["terminalTitleFromRename"] = json!(true);
                }
                _ => unreachable!(),
            }
            assert_eq!(after, before);
            let backup = fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .contains(".lomi-backup-")
                })
                .unwrap();
            assert_eq!(fs::read_to_string(backup.path()).unwrap(), source);
            fs::write(&path, "{\"external\":true}").unwrap();
            assert!(enable(cli, &path, setup.revision.as_deref()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), "{\"external\":true}");
        }
        for cli in [TitleCli::Agy, TitleCli::Cursor, TitleCli::Claude] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("new/settings.json");
            assert_eq!(
                inspect(cli, &path, false).unwrap().is_none(),
                cli == TitleCli::Claude
            );
            assert!(!path.parent().unwrap().exists());
            enable(cli, &path, None).unwrap();
            assert!(inspect(cli, &path, true).unwrap().is_none());
            if cli == TitleCli::Agy {
                let doc = json_document(cli, read(&path).unwrap().as_deref()).unwrap();
                assert_eq!(doc["title"]["command"], agy_command().unwrap());
            }
        }
        assert!(!configured(TitleCli::Claude, None, true).unwrap());
        assert!(configured(
            TitleCli::Claude,
            Some(r#"{"env":{"CLAUDE_CODE_DISABLE_TERMINAL_TITLE":"0"}}"#),
            true
        )
        .unwrap());
        assert!(configured(
            TitleCli::Agy,
            Some(r#"{"title":{"type":"command","command":"custom"}}"#),
            false
        )
        .unwrap());
    }

    #[test]
    fn rejects_invalid_json_without_losing_the_original_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for cli in [TitleCli::Agy, TitleCli::Cursor, TitleCli::Claude] {
            for source in [
                "secret invalid",
                "[]",
                "null",
                r#"{"title":[],"display":[],"env":[]}"#,
                r#"{"title":{"enabled":"no"},"display":{"showStatusIndicators":0},"env":{"CLAUDE_CODE_DISABLE_TERMINAL_TITLE":true}}"#,
            ] {
                fs::write(&path, source).unwrap();
                assert!(inspect(cli, &path, false).is_err());
                assert!(enable(cli, &path, revision(Some(source)).as_deref()).is_err());
                assert_eq!(fs::read_to_string(&path).unwrap(), source);
            }
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn formats_agy_conversation_changes_and_bounds_untrusted_titles() {
        let dir = tempfile::tempdir().unwrap();
        let annotations = dir.path().join("annotations");
        let mut data =
            json!({"conversation_title":"Ulepsz system zakładek", "agent_state":"working"});
        assert_eq!(agy_title(&data, &annotations), "Ulepsz system zakładek");
        data["conversation_title"] = json!("Inna rozmowa");
        data["tool_confirmation_pending"] = json!(true);
        assert_eq!(agy_title(&data, &annotations), "Inna rozmowa");
        assert_eq!(
            agy_title(
                &json!({"cwd":"/tmp/project", "conversation_id":"unknown"}),
                &annotations
            ),
            "agy"
        );
        assert!(!annotations.exists());
        data["conversation_title"] = json!("\u{1b}\u{7}\n".to_owned() + &"ą".repeat(500));
        let title = agy_title(&data, &annotations);
        assert_eq!(title.chars().count(), 256);
        assert!(!title.chars().any(char::is_control));
    }

    #[test]
    fn resolves_live_agy_names_and_resume_renames_without_a_summaries_database() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first-conversation.pbtxt");
        let second = dir.path().join("second-conversation.pbtxt");
        fs::write(&first, r#"title:"Data Wydania Dipsick V4""#).unwrap();
        fs::write(&second, r#"title: "Druga rozmowa""#).unwrap();
        let mut data = json!({
            "cwd": "/tmp/lomi",
            "conversation_id": "first-conversation",
            "agent_state": "idle",
            "transcript_path": "/unreadable/transcript.jsonl",
        });
        assert_eq!(agy_title(&data, dir.path()), "Data Wydania Dipsick V4");
        assert_eq!(
            fs::read_to_string(&first).unwrap(),
            r#"title:"Data Wydania Dipsick V4""#
        );
        data["conversation_id"] = json!("second-conversation");
        assert_eq!(agy_title(&data, dir.path()), "Druga rozmowa");

        fs::write(
            &second,
            r#"title:"Zażółć \"gęślą\" \\x41 \u015b \U0001f980""#,
        )
        .unwrap();
        data["conversation_title"] = json!("Stale CLI title");
        assert_eq!(agy_title(&data, dir.path()), "Zażółć \"gęślą\" \\x41 ś 🦀");
        data["conversation_id"] = json!("");
        data["session_id"] = json!("second-conversation");
        assert_eq!(agy_title(&data, dir.path()), "Zażółć \"gęślą\" \\x41 ś 🦀");
        fs::write(&second, r#"title:"Safe\x1b\x07\nname""#).unwrap();
        assert_eq!(agy_title(&data, dir.path()), "Safe name");
        assert!(agy_annotation_title(r#"tags: "a title: " tags: "Not a title""#).is_none());
        fs::write(&second, "invalid annotation").unwrap();
        assert_eq!(agy_title(&data, dir.path()), "Stale CLI title");
        assert_eq!(fs::read_to_string(&second).unwrap(), "invalid annotation");
        data.as_object_mut().unwrap().remove("conversation_title");
        fs::write(&second, "x".repeat(LIMIT as usize + 1)).unwrap();
        assert_eq!(agy_title(&data, dir.path()), "agy");
    }

    #[test]
    fn agy_title_ids_cannot_escape_the_annotations_directory() {
        let dir = tempfile::tempdir().unwrap();
        let annotations = dir.path().join("annotations");
        fs::create_dir(&annotations).unwrap();
        let outside = dir.path().join("outside.pbtxt");
        fs::write(&outside, r#"title:"Must not read""#).unwrap();
        for id in [
            "../outside".to_owned(),
            outside.with_extension("").to_string_lossy().into_owned(),
        ] {
            assert_eq!(
                agy_title(&json!({"conversation_id":id}), &annotations),
                "agy"
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, annotations.join("link.pbtxt")).unwrap();
            assert_eq!(
                agy_title(&json!({"conversation_id":"link"}), &annotations),
                "agy"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn identifies_native_clis_and_node_launchers_without_matching_prompt_arguments() {
        for (path, cli) in [
            ("/bin/agy", TitleCli::Agy),
            ("/bin/codex", TitleCli::Codex),
            ("/bin/claude", TitleCli::Claude),
            (
                "/home/user/.local/share/claude/versions/2.1.263",
                TitleCli::Claude,
            ),
            ("/cursor/cursor-agent-sea", TitleCli::Cursor),
        ] {
            assert_eq!(identify(Path::new(path), b""), Some(cli));
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("cursor-agent"), "launcher").unwrap();
        let argv = format!(
            "/home/user/.local/bin/agent\0--use-system-ca\0{}/index.js\0",
            dir.path().display()
        );
        assert_eq!(
            identify(&dir.path().join("node"), argv.as_bytes()),
            Some(TitleCli::Cursor)
        );
        assert_eq!(
            identify(
                Path::new("/usr/bin/node"),
                b"node\0/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js\0"
            ),
            Some(TitleCli::Claude)
        );
        for path in [
            "/usr/bin/node",
            "/usr/bin/bash",
            "/bin/agent",
            "/usr/bin/cursor",
            "/bin/ssh",
        ] {
            assert_eq!(
                identify(
                    Path::new(path),
                    b"node\0other.js\0codex\0agy\0claude\0cursor-agent\0"
                ),
                None
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolves_each_cli_environment_without_creating_settings_before_consent() {
        use std::process::{Command, Stdio};
        let dir = tempfile::tempdir().unwrap();
        for (cli, variables, relative) in [
            (
                TitleCli::Agy,
                vec![],
                ".gemini/antigravity-cli/settings.json",
            ),
            (TitleCli::Cursor, vec![], ".cursor/cli-config.json"),
            (
                TitleCli::Cursor,
                vec![("XDG_CONFIG_HOME", "xdg")],
                "xdg/cursor/cli-config.json",
            ),
            (
                TitleCli::Cursor,
                vec![
                    ("XDG_CONFIG_HOME", "xdg"),
                    ("CURSOR_CONFIG_DIR", "cursor-custom"),
                ],
                "cursor-custom/cli-config.json",
            ),
            (TitleCli::Claude, vec![], ".claude/settings.json"),
            (
                TitleCli::Claude,
                vec![("CLAUDE_CONFIG_DIR", "claude-custom")],
                "claude-custom/settings.json",
            ),
        ] {
            let mut command = Command::new("sh");
            command
                .env_clear()
                .env("HOME", dir.path())
                .env("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1");
            for (key, value) in variables {
                command.env(key, dir.path().join(value));
            }
            let mut child = command
                .args(["-c", "printf ready; read -r _"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdout
                .as_mut()
                .unwrap()
                .read_exact(&mut [0; 5])
                .unwrap();
            let result = configuration(TitleProcess {
                cli,
                pid: child.id(),
            });
            child.kill().unwrap();
            child.wait().unwrap();
            assert_eq!(result.unwrap(), (dir.path().join(relative), true));
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn updates_only_titles_and_keeps_a_backup_for_toml_layouts() {
        for (source, preserved) in [
            (
                "# Preferences\nmodel = 'custom-model'\n",
                "# Preferences\nmodel = 'custom-model'\n",
            ),
            (
                "[tui]\nterminal_title = ['project'] # custom title\nnotifications = false\n",
                "# custom title\nnotifications = false",
            ),
            (
                "tui = { terminal_title = [], notifications = false }\n",
                "notifications = false",
            ),
            (
                "tui.terminal_title = ['project']\ntui.notifications = false\n",
                "tui.notifications = false",
            ),
            (
                "[tui.model_availability_nux]\ncustom = 4\n",
                "[tui.model_availability_nux]\ncustom = 4",
            ),
            (
                "# Title settings\r\n[tui]\r\nterminal_title = []\r\n",
                "# Title settings",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            fs::write(&path, source).unwrap();
            let setup = inspect(TitleCli::Codex, &path, false).unwrap().unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            enable(TitleCli::Codex, &path, setup.revision.as_deref()).unwrap();
            assert!(inspect(TitleCli::Codex, &path, false).unwrap().is_none());
            let updated = fs::read_to_string(&path).unwrap();
            assert!(updated.contains(preserved), "{updated}");
            if source.contains("\r\n") {
                assert!(!updated.replace("\r\n", "").contains('\n'));
            }
            let backups = fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("config.toml.lomi-backup-")
                })
                .collect::<Vec<_>>();
            assert_eq!(backups.len(), 1);
            assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), source);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert!(inspect(TitleCli::Codex, &path, false)
            .unwrap()
            .unwrap()
            .revision
            .is_none());
        assert!(!path.exists());
        enable(TitleCli::Codex, &path, None).unwrap();
        assert!(inspect(TitleCli::Codex, &path, false).unwrap().is_none());
        fs::write(
            &path,
            "[tui]\nterminal_title = ['thread-title', 'project']\n",
        )
        .unwrap();
        assert!(inspect(TitleCli::Codex, &path, false).unwrap().is_none());
    }

    #[test]
    fn leaves_invalid_and_concurrently_changed_files_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for source in [
            "broken = [secret".to_string(),
            "tui = 1".into(),
            "[tui]\nterminal_title = 'project'".into(),
            "[tui]\nterminal_title = [1]".into(),
            " ".repeat(LIMIT as usize + 1),
        ] {
            fs::write(&path, &source).unwrap();
            assert!(inspect(TitleCli::Codex, &path, false).is_err());
            assert!(enable(TitleCli::Codex, &path, revision(Some(&source)).as_deref()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
        fs::write(&path, "model = 'before'\n").unwrap();
        let setup = inspect(TitleCli::Codex, &path, false).unwrap().unwrap();
        let external = "model = 'external edit'\n";
        fs::write(&path, external).unwrap();
        assert!(enable(TitleCli::Codex, &path, setup.revision.as_deref())
            .unwrap_err()
            .contains("changed"));
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
        fs::write(&path, "").unwrap();
        assert!(enable(TitleCli::Codex, &path, None).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        let full = format!("#{}\n", " ".repeat(LIMIT as usize - 2));
        fs::write(&path, &full).unwrap();
        let setup = inspect(TitleCli::Codex, &path, false).unwrap().unwrap();
        assert!(enable(TitleCli::Codex, &path, setup.revision.as_deref()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), full);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolves_process_home_and_preserves_symlinks_and_permissions() {
        use std::{
            os::unix::fs::{symlink, PermissionsExt},
            process::{Command, Stdio},
        };
        let dir = tempfile::tempdir().unwrap();
        let running_path = |command: &mut Command| {
            let mut child = command
                .args(["-c", "printf ready; read -r _"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdout
                .as_mut()
                .unwrap()
                .read_exact(&mut [0; 5])
                .unwrap();
            let path = configuration(TitleProcess {
                cli: TitleCli::Codex,
                pid: child.id(),
            })
            .map(|(path, _)| path);
            child.kill().unwrap();
            child.wait().unwrap();
            path.unwrap()
        };
        let target = dir.path().join("dotfiles.toml");
        fs::write(&target, "model = 'test'\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, dir.path().join("config.toml")).unwrap();
        let path = running_path(Command::new("sh").env("CODEX_HOME", dir.path()));
        assert_eq!(path, target);
        let setup = inspect(TitleCli::Codex, &path, false).unwrap().unwrap();
        enable(TitleCli::Codex, &path, setup.revision.as_deref()).unwrap();
        assert!(dir.path().join("config.toml").is_symlink());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::set_permissions(&target, fs::Permissions::from_mode(0o440)).unwrap();
        let before = fs::read_to_string(&target).unwrap();
        assert!(enable(TitleCli::Codex, &target, revision(Some(&before)).as_deref()).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), before);

        let default = dir.path().join(".codex");
        fs::create_dir(&default).unwrap();
        let path = running_path(
            Command::new("sh")
                .env_remove("CODEX_HOME")
                .env("HOME", dir.path()),
        );
        assert_eq!(path, default.join("config.toml"));
    }
}
