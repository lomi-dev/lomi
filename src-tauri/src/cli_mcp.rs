mod yaml;
use crate::{cli_config, cli_titles::TitleCli};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use tauri::Manager;
use toml_edit::{DocumentMut, Item, Table};

#[derive(Clone)]
pub(crate) struct Registration {
    pub command: String,
    pub args: Vec<String>,
}

pub(crate) fn registration(
    app: &tauri::AppHandle,
    create: bool,
) -> Result<Option<Registration>, String> {
    if !crate::agent_control::supported_host() {
        return Err("Lomi MCP is not supported on this host.".into());
    }
    #[cfg(unix)]
    {
        let directory = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?;
        if create {
            std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        }
        let root = directory.join("agent-control");
        let key = if create {
            Some(
                lomi_control_core::discovery::ensure_public_key(&root)
                    .map_err(|error| error.to_string())?,
            )
        } else {
            lomi_control_core::discovery::public_key(&root).map_err(|error| error.to_string())?
        };
        let command = std::env::current_exe()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        Ok(key.map(|key| Registration {
            command,
            args: vec![
                "--mcp".into(),
                "--discovery-file".into(),
                root.join(lomi_control_core::discovery::FILE)
                    .to_string_lossy()
                    .into_owned(),
                "--discovery-key".into(),
                key,
            ],
        }))
    }
    #[cfg(not(unix))]
    {
        let _ = (app, create);
        Err("Lomi MCP is not supported on this host.".into())
    }
}

pub(crate) fn resolve(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("CLI configuration requires an absolute path.".into());
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match ancestor.symlink_metadata() {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or("Invalid CLI configuration path.")?,
                );
                ancestor = ancestor.parent().ok_or("Invalid CLI configuration path.")?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let mut result = ancestor.canonicalize().map_err(|error| error.to_string())?;
    for component in missing.iter().rev() {
        result.push(component);
    }
    Ok(result)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Json,
    Jsonc,
    Json5,
    Toml,
    TomlArray,
    Yaml,
}

fn format(cli: TitleCli) -> Option<Format> {
    use TitleCli::*;
    Some(match cli {
        Codex | Grok => Format::Toml,
        Vibe => Format::TomlArray,
        Hermes => Format::Yaml,
        Opencode | Kilo | Qwen | Gemini => Format::Jsonc,
        Openclaw => Format::Json5,
        Claude | Cursor | Agy | Copilot | Kiro | Kimi => Format::Json,
        _ => return None,
    })
}

pub(crate) fn supported(cli: TitleCli) -> bool {
    format(cli).is_some()
}

pub(crate) fn manual_reason(cli: TitleCli) -> Option<&'static str> {
    use TitleCli::*;
    match cli {
        Pi => Some("MCP requires a Pi extension; install and configure that extension in Pi."),
        _ => None,
    }
}

pub(crate) fn configuration_path(cli: TitleCli, entries: &[&[u8]]) -> Result<PathBuf, String> {
    let env = |name: &str| -> Option<PathBuf> {
        entries.iter().find_map(|entry| {
            let value = entry
                .strip_prefix(name.as_bytes())
                .filter(|value| !value.is_empty())?;
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                Some(PathBuf::from(std::ffi::OsString::from_vec(value.to_vec())))
            }
            #[cfg(not(unix))]
            {
                Some(PathBuf::from(String::from_utf8_lossy(value).into_owned()))
            }
        })
    };
    let home = env("HOME=").ok_or("Cannot locate the CLI home directory.")?;
    let xdg = env("XDG_CONFIG_HOME=").unwrap_or_else(|| home.join(".config"));
    let prefer_jsonc = |directory: PathBuf, stem: &str| -> Result<PathBuf, String> {
        let json = directory.join(format!("{stem}.json"));
        let jsonc = directory.join(format!("{stem}.jsonc"));
        // Both files can contribute settings. Refuse an ambiguous write target.
        if json.try_exists().map_err(|e| e.to_string())?
            && jsonc.try_exists().map_err(|e| e.to_string())?
        {
            return Err("Both JSON and JSONC configuration files exist. Use the CLI's MCP setup to choose the active configuration.".into());
        }
        Ok(if jsonc.try_exists().map_err(|e| e.to_string())? {
            jsonc
        } else {
            json
        })
    };
    use TitleCli::*;
    let path = match cli {
        Gemini => env("GEMINI_CLI_HOME=")
            .unwrap_or_else(|| home.clone())
            .join(".gemini/settings.json"),
        Copilot => env("COPILOT_HOME=")
            .unwrap_or_else(|| home.join(".copilot"))
            .join("mcp-config.json"),
        Opencode => {
            if env("OPENCODE_CONFIG_CONTENT=").is_some() || env("OPENCODE_CONFIG_DIR=").is_some() {
                return Err(
                    "OpenCode uses an additional configuration source. Register Lomi in that configuration."
                        .into(),
                );
            }
            match env("OPENCODE_CONFIG=") {
                Some(path) => path,
                None => prefer_jsonc(xdg.join("opencode"), "opencode")?,
            }
        }
        Kilo => {
            if env("KILO_CONFIG_CONTENT=").is_some() || env("KILO_CONFIG_DIR=").is_some() {
                return Err("Kilo uses an additional configuration source. Register Lomi in that configuration.".into());
            }
            env("KILO_CONFIG=").unwrap_or_else(|| xdg.join("kilo/kilo.jsonc"))
        }
        Openclaw => {
            if env("OPENCLAW_PROFILE=").is_some() || env("OPENCLAW_HOME=").is_some() {
                return Err("OpenClaw uses a custom profile or home. Register Lomi in that profile's configuration.".into());
            }
            env("OPENCLAW_CONFIG_PATH=").unwrap_or_else(|| {
                env("OPENCLAW_STATE_DIR=")
                    .unwrap_or_else(|| home.join(".openclaw"))
                    .join("openclaw.json")
            })
        }
        Hermes => env("HERMES_HOME=")
            .unwrap_or_else(|| home.join(".hermes"))
            .join("config.yaml"),
        Qwen => {
            let directory = env("QWEN_HOME=").unwrap_or_else(|| home.join(".qwen"));
            let directory = directory
                .strip_prefix("~")
                .map(|suffix| home.join(suffix))
                .unwrap_or_else(|_| directory.clone());
            directory.join("settings.json")
        }
        Kiro => env("KIRO_HOME=")
            .unwrap_or_else(|| home.join(".kiro"))
            .join("settings/mcp.json"),
        Vibe => {
            if entries.iter().any(|entry| *entry == b"VIBE_CLI=rust") {
                return Err("Use Vibe's Python CLI for stdio MCP configuration.".into());
            }
            env("VIBE_HOME=")
                .unwrap_or_else(|| home.join(".vibe"))
                .join("config.toml")
        }
        Kimi => env("KIMI_CODE_HOME=")
            .unwrap_or_else(|| home.join(".kimi-code"))
            .join("mcp.json"),
        Grok => env("GROK_HOME=")
            .unwrap_or_else(|| home.join(".grok"))
            .join("config.toml"),
        _ => return Err("Automatic MCP configuration is not available for this CLI.".into()),
    };
    resolve(&path)
}

fn document(source: Option<&str>) -> Result<DocumentMut, String> {
    source
        .unwrap_or_default()
        .parse()
        .map_err(|_| "CLI configuration is not valid TOML. The file was left intact.".into())
}

fn json_options(cli: TitleCli) -> jsonc_parser::ParseOptions {
    let comments = matches!(format(cli), Some(Format::Jsonc | Format::Json5));
    let json5 = format(cli) == Some(Format::Json5);
    jsonc_parser::ParseOptions {
        allow_comments: comments,
        allow_trailing_commas: comments && !matches!(cli, TitleCli::Gemini | TitleCli::Qwen),
        allow_loose_object_property_names: json5,
        allow_missing_commas: false,
        allow_single_quoted_strings: json5,
        allow_hexadecimal_numbers: json5,
        allow_unary_plus_numbers: json5,
    }
}

fn json_source(cli: TitleCli, source: &str) -> &str {
    if cli == TitleCli::Agy
        && source
            .bytes()
            .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
    {
        "{}"
    } else {
        source
    }
}

pub(crate) fn json_document(cli: TitleCli, source: Option<&str>) -> Result<Value, String> {
    let source = json_source(cli, source.unwrap_or("{}"));
    let parsed = jsonc_parser::parse_to_ast(source, &Default::default(), &json_options(cli))
        .map_err(|_| "CLI configuration is not valid JSON/JSONC. The file was left intact.")?;
    fn validate(value: &jsonc_parser::ast::Value<'_>, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err("CLI configuration nesting exceeds 64 levels.".into());
        }
        match value {
            jsonc_parser::ast::Value::Object(object) => {
                let mut names = std::collections::HashSet::new();
                for prop in &object.properties {
                    if !names.insert(prop.name.as_str()) {
                        return Err(
                            "CLI configuration contains duplicate keys. The file was left intact."
                                .into(),
                        );
                    }
                    validate(&prop.value, depth + 1)?;
                }
            }
            jsonc_parser::ast::Value::Array(array) => {
                for value in &array.elements {
                    validate(value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let ast = parsed.value.ok_or("CLI configuration must be an object.")?;
    validate(&ast, 0)?;
    if ast.as_object().is_none() {
        return Err("CLI configuration must be an object.".into());
    }
    jsonc_parser::parse_to_serde_value(source, &json_options(cli))
        .map_err(|_| "CLI configuration is not valid JSON/JSONC. The file was left intact.".into())
}

fn json_keys(cli: TitleCli) -> &'static [&'static str] {
    match cli {
        TitleCli::Opencode | TitleCli::Openclaw => &["mcp", "servers"],
        TitleCli::Kilo => &["mcp"],
        _ => &["mcpServers"],
    }
}
fn command_array(cli: TitleCli) -> bool {
    matches!(cli, TitleCli::Opencode | TitleCli::Kilo)
}

fn json_servers(
    cli: TitleCli,
    doc: &Value,
) -> Result<Option<&serde_json::Map<String, Value>>, String> {
    if cli == TitleCli::Opencode
        && doc
            .get("mcp")
            .and_then(Value::as_object)
            .is_some_and(|mcp| {
                mcp.values()
                    .any(|entry| entry.get("type").is_some() || entry.get("command").is_some())
            })
    {
        return Err("OpenCode uses the legacy MCP format. Migrate it to v2 or register Lomi with that CLI version.".into());
    }
    let mut current = doc;
    for key in json_keys(cli) {
        let Some(next) = current.get(key) else {
            return Ok(None);
        };
        if !next.is_object() {
            return Err(format!(
                "CLI {key} must be an object. The file was left intact."
            ));
        }
        current = next;
    }
    Ok(current.as_object())
}

// Replace only the selected value; comments and unrelated settings keep their bytes.
pub(crate) fn set_json(
    cli: TitleCli,
    source: &str,
    keys: &[&str],
    value: &Value,
) -> Result<String, String> {
    use jsonc_parser::common::Ranged;
    let source = json_source(cli, source);
    let parsed = jsonc_parser::parse_to_ast(source, &Default::default(), &json_options(cli))
        .map_err(|e| e.to_string())?;
    let root = parsed.value.ok_or("CLI configuration must be an object.")?;
    let mut node = &root;
    for (index, key) in keys.iter().enumerate() {
        let object = node
            .as_object()
            .ok_or("CLI configuration section must be an object.")?;
        if let Some(property) = object.get(key) {
            if index + 1 == keys.len() {
                let mut output = source.to_owned();
                output.replace_range(
                    property.value.start()..property.value.end(),
                    &serde_json::to_string_pretty(value).map_err(|e| e.to_string())?,
                );
                return Ok(output);
            }
            node = &property.value;
        } else {
            let mut nested = value.clone();
            for tail in keys[index + 1..].iter().rev() {
                nested = json!({*tail: nested});
            }
            let insertion = format!(
                "\n{}: {}{}\n",
                serde_json::to_string(key).unwrap(),
                serde_json::to_string_pretty(&nested).map_err(|e| e.to_string())?,
                if object.properties.is_empty() {
                    ""
                } else {
                    ","
                }
            );
            let mut output = source.to_owned();
            output.insert_str(object.start() + 1, &insertion);
            return Ok(output);
        }
    }
    Err("Missing CLI configuration key.".into())
}

fn owned(command: Option<&str>, args: Option<Vec<&str>>, expected: &Registration) -> bool {
    let (Some(command), Some(args)) = (command, args) else {
        return false;
    };
    let helper = Path::new(&expected.command).with_file_name("lomi-mcp");
    if Path::new(command) == helper {
        return args.len() == 6
            && args[0] == "--endpoint"
            && args[2] == "--instance"
            && args[4] == "--broker-sha256"
            && args[5].len() == 64
            && args[5].bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    let current_arguments = args.len() == 5
        && args[0] == "--mcp"
        && args[1] == "--discovery-file"
        && args.get(2) == expected.args.get(2).map(String::as_str).as_ref()
        && args[3] == "--discovery-key"
        && args[4].len() == 64
        && args[4].bytes().all(|byte| byte.is_ascii_hexdigit());
    current_arguments
        && (command == expected.command
            || matches!(
                Path::new(command)
                    .file_name()
                    .and_then(|name| name.to_str()),
                Some("lomi" | "lomi.exe")
            ))
}

fn json_owned(cli: TitleCli, entry: &Value, expected: &Registration) -> bool {
    if command_array(cli) {
        let Some(command) = entry.get("command").and_then(Value::as_array) else {
            return false;
        };
        owned(
            command.first().and_then(Value::as_str),
            command
                .get(1..)
                .and_then(|args| args.iter().map(Value::as_str).collect()),
            expected,
        )
    } else {
        owned(
            entry.get("command").and_then(Value::as_str),
            entry
                .get("args")
                .and_then(Value::as_array)
                .and_then(|args| args.iter().map(Value::as_str).collect()),
            expected,
        )
    }
}
fn toml_matches(entry: &dyn toml_edit::TableLike, expected: &Registration) -> bool {
    entry.get("command").and_then(Item::as_str) == Some(expected.command.as_str())
        && entry
            .get("args")
            .and_then(Item::as_array)
            .is_some_and(|args| {
                args.iter()
                    .map(|arg| arg.as_str())
                    .eq(expected.args.iter().map(|arg| Some(arg.as_str())))
            })
        && entry
            .get("enabled")
            .is_none_or(|value| value.as_bool() == Some(true))
        && entry.get("url").is_none()
}

pub(crate) fn configured(
    cli: TitleCli,
    source: Option<&str>,
    expected: Option<&Registration>,
) -> Result<bool, String> {
    let format = format(cli).ok_or("Automatic MCP configuration is not available for this CLI.")?;
    if format == Format::Yaml {
        return yaml::configured(source, expected);
    }
    if matches!(format, Format::Toml | Format::TomlArray) {
        let doc = document(source)?;
        let Some(servers) = doc.get("mcp_servers") else {
            return Ok(false);
        };
        let entry = if format == Format::TomlArray {
            let servers = servers
                .as_array_of_tables()
                .ok_or("mcp_servers must be an array of tables.")?;
            let entries = servers
                .iter()
                .filter(|entry| entry.get("name").and_then(Item::as_str) == Some("lomi"))
                .collect::<Vec<_>>();
            if entries.len() > 1 {
                return Err(
                    "Multiple MCP servers are named lomi. The file was left intact.".into(),
                );
            }
            entries
                .first()
                .map(|entry| *entry as &dyn toml_edit::TableLike)
        } else {
            let servers = servers
                .as_table_like()
                .ok_or("mcp_servers must be a table.")?;
            servers
                .get("lomi")
                .map(|entry| {
                    entry
                        .as_table_like()
                        .ok_or("Lomi MCP settings must be a table.")
                })
                .transpose()?
        };
        return Ok(entry.zip(expected).is_some_and(|(entry, expected)| {
            toml_matches(entry, expected)
                && (format != Format::TomlArray
                    || entry.get("transport").and_then(Item::as_str) == Some("stdio"))
        }));
    }
    let doc = json_document(cli, source)?;
    let Some(entry) = json_servers(cli, &doc)?.and_then(|servers| servers.get("lomi")) else {
        return Ok(false);
    };
    if !entry.is_object() {
        return Err("Lomi MCP settings must be an object.".into());
    }
    let Some(expected) = expected else {
        return Ok(false);
    };
    let command_matches = if command_array(cli) {
        entry["command"]
            == json!(std::iter::once(&expected.command)
                .chain(expected.args.iter())
                .collect::<Vec<_>>())
    } else {
        entry["command"] == expected.command && entry["args"] == json!(expected.args)
    };
    let required_type = match cli {
        TitleCli::Copilot | TitleCli::Opencode | TitleCli::Kilo => Some("local"),
        TitleCli::Claude | TitleCli::Cursor => Some("stdio"),
        _ => None,
    };
    Ok(command_matches
        && match required_type {
            Some("local") => entry["type"] == "local",
            Some(kind) => entry.get("type").is_none_or(|value| value == kind),
            None => entry.get("type").is_none_or(|value| value == "stdio"),
        }
        && entry.get("transport").is_none_or(|value| value == "stdio")
        && entry["disabled"] != true
        && entry["enabled"] != false
        && (cli != TitleCli::Agy || entry.get("serverUrl").is_none())
        && entry.get("url").is_none()
        && entry.get("httpUrl").is_none()
        && !doc
            .get("disabledMcpServers")
            .and_then(Value::as_array)
            .is_some_and(|names| names.iter().any(|name| name == "lomi")))
}

pub(crate) fn enable(
    cli: TitleCli,
    path: &Path,
    revision: Option<&str>,
    expected: &Registration,
) -> Result<(), String> {
    let source = cli_config::read(path)?;
    if cli_config::revision(source.as_deref()).as_deref() != revision {
        return Err("CLI configuration changed. Check it again before installing Lomi MCP.".into());
    }
    if expected.command.is_empty() {
        return Err("Cannot locate the Lomi executable.".into());
    }
    if configured(cli, source.as_deref(), Some(expected))? {
        return Ok(());
    }
    let conflict = "A different MCP server is already named lomi. Rename that entry in the CLI configuration before installing Lomi MCP.";
    let format = format(cli).ok_or("Automatic MCP configuration is not available for this CLI.")?;
    let output = if format == Format::Yaml {
        yaml::updated(source.as_deref(), expected)?
    } else if matches!(format, Format::Toml | Format::TomlArray) {
        let mut doc = document(source.as_deref())?;
        let existing: Option<&dyn toml_edit::TableLike> = if format == Format::TomlArray {
            doc.get("mcp_servers")
                .and_then(Item::as_array_of_tables)
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.get("name").and_then(Item::as_str) == Some("lomi"))
                })
                .map(|item| item as &dyn toml_edit::TableLike)
        } else {
            doc.get("mcp_servers")
                .and_then(|servers| servers.get("lomi"))
                .and_then(Item::as_table_like)
        };
        if existing.is_some_and(|entry| {
            !owned(
                entry.get("command").and_then(Item::as_str),
                entry
                    .get("args")
                    .and_then(Item::as_array)
                    .and_then(|args| args.iter().map(|arg| arg.as_str()).collect()),
                expected,
            )
        }) {
            return Err(conflict.into());
        }
        let entry: &mut dyn toml_edit::TableLike = if format == Format::TomlArray {
            let servers = doc
                .entry("mcp_servers")
                .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
                .as_array_of_tables_mut()
                .ok_or("mcp_servers must be an array of tables.")?;
            let index = servers
                .iter()
                .position(|entry| entry.get("name").and_then(Item::as_str) == Some("lomi"));
            let index = match index {
                Some(index) => index,
                None => {
                    let mut entry = Table::new();
                    entry.insert("name", toml_edit::value("lomi"));
                    servers.push(entry);
                    servers.len() - 1
                }
            };
            servers.get_mut(index).unwrap()
        } else {
            let servers = doc
                .entry("mcp_servers")
                .or_insert(Item::Table(Table::new()))
                .as_table_like_mut()
                .ok_or("mcp_servers must be a table.")?;
            servers
                .entry("lomi")
                .or_insert(Item::Table(Table::new()))
                .as_table_like_mut()
                .ok_or("Lomi MCP settings must be a table.")?
        };
        if (entry.contains_key("command")
            || entry.contains_key("url")
            || entry.contains_key("args"))
            && !owned(
                entry.get("command").and_then(Item::as_str),
                entry
                    .get("args")
                    .and_then(Item::as_array)
                    .and_then(|args| args.iter().map(|arg| arg.as_str()).collect()),
                expected,
            )
        {
            return Err(conflict.into());
        }
        entry.insert("command", toml_edit::value(&expected.command));
        entry.insert(
            "args",
            toml_edit::value(
                expected
                    .args
                    .iter()
                    .map(String::as_str)
                    .collect::<toml_edit::Array>(),
            ),
        );
        entry.remove("url");
        if format == Format::TomlArray {
            entry.insert("transport", toml_edit::value("stdio"));
        } else {
            entry.remove("transport");
        }
        if format != Format::TomlArray || entry.contains_key("enabled") {
            entry.insert("enabled", toml_edit::value(true));
        }
        doc.to_string()
    } else {
        let doc = json_document(cli, source.as_deref())?;
        let existing = json_servers(cli, &doc)?.and_then(|servers| servers.get("lomi"));
        if existing.is_some_and(|entry| !json_owned(cli, entry, expected)) {
            return Err(conflict.into());
        }
        let mut entry = existing.cloned().unwrap_or_else(|| json!({}));
        let object = entry
            .as_object_mut()
            .ok_or("Lomi MCP settings must be an object.")?;
        if command_array(cli) {
            object.insert(
                "command".into(),
                json!(std::iter::once(&expected.command)
                    .chain(expected.args.iter())
                    .collect::<Vec<_>>()),
            );
            object.remove("args");
        } else {
            object.insert("command".into(), json!(expected.command));
            object.insert("args".into(), json!(expected.args));
        }
        let kind = match cli {
            TitleCli::Opencode | TitleCli::Kilo | TitleCli::Copilot => Some("local"),
            TitleCli::Claude | TitleCli::Cursor => Some("stdio"),
            _ => None,
        };
        if let Some(kind) = kind {
            object.insert("type".into(), json!(kind));
        } else {
            object.remove("type");
        }
        object.remove("transport");
        if cli == TitleCli::Copilot {
            object.entry("tools").or_insert_with(|| json!(["*"]));
        }
        if cli == TitleCli::Kilo || object.contains_key("enabled") {
            object.insert("enabled".into(), json!(true));
        }
        if object.contains_key("disabled") {
            object.insert("disabled".into(), json!(false));
        }
        object.remove("url");
        object.remove("httpUrl");
        if cli == TitleCli::Agy {
            object.remove("serverUrl");
        }
        let mut keys = json_keys(cli).to_vec();
        keys.push("lomi");
        let mut output = set_json(cli, source.as_deref().unwrap_or("{}\n"), &keys, &entry)?;
        if let Some(names) = doc.get("disabledMcpServers") {
            let mut names = names
                .as_array()
                .ok_or("disabledMcpServers must be an array.")?
                .clone();
            names.retain(|name| name != "lomi");
            output = set_json(cli, &output, &["disabledMcpServers"], &json!(names))?;
        }
        output
    };
    cli_config::write(path, source.as_deref(), output)
}
