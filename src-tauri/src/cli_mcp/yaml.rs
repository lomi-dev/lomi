use super::{owned, Registration, TitleCli};
use serde_json::{json, Value};

fn document(source: Option<&str>) -> Result<Value, String> {
    let options = serde_saphyr::options! {
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::Error,
        merge_keys: serde_saphyr::MergeKeyPolicy::Error,
        reject_unsupported_tags: true,
        strict_booleans: true,
        with_snippet: false,
        budget: serde_saphyr::budget! {
            max_depth: 64,
            max_nodes: 50_000,
            max_events: 100_000,
            max_total_scalar_bytes: 1024 * 1024,
            max_aliases: 0,
            max_anchors: 0,
        },
    };
    let value: Value = serde_saphyr::from_str_with_options(source.unwrap_or("{}"), options)
        .map_err(|_| "CLI YAML is invalid or uses unsupported aliases, tags, or merge keys. The file was left intact.")?;
    if !value.is_object() {
        return Err("CLI YAML configuration must be a mapping.".into());
    }
    Ok(value)
}

fn key(cli: TitleCli) -> &'static str {
    match cli {
        TitleCli::Goose => "extensions",
        TitleCli::Continue => "mcpServers",
        _ => "mcp_servers",
    }
}
fn command_key(cli: TitleCli) -> &'static str {
    if cli == TitleCli::Goose {
        "cmd"
    } else {
        "command"
    }
}

fn entry(cli: TitleCli, doc: &Value) -> Result<Option<&Value>, String> {
    let Some(servers) = doc.get(key(cli)) else {
        return Ok(None);
    };
    if cli == TitleCli::Continue {
        let entries = servers
            .as_array()
            .ok_or("Continue mcpServers must be a list.")?
            .iter()
            .filter(|entry| entry.get("name").and_then(Value::as_str) == Some("lomi"))
            .collect::<Vec<_>>();
        if entries.len() > 1 {
            return Err("Multiple MCP servers are named lomi. The file was left intact.".into());
        }
        Ok(entries.first().copied())
    } else {
        Ok(servers
            .as_object()
            .ok_or("CLI MCP settings must be a mapping.")?
            .get("lomi"))
    }
}

pub(super) fn configured(
    cli: TitleCli,
    source: Option<&str>,
    expected: Option<&Registration>,
) -> Result<bool, String> {
    let doc = document(source)?;
    let Some(entry) = entry(cli, &doc)? else {
        return Ok(false);
    };
    if !entry.is_object() {
        return Err("Lomi MCP settings must be a mapping.".into());
    }
    let Some(expected) = expected else {
        return Ok(false);
    };
    Ok(entry[command_key(cli)] == expected.command
        && entry["args"] == json!(expected.args)
        && entry.get("url").is_none()
        && entry.get("uri").is_none()
        && entry["disabled"] != true
        && entry["enabled"] != false
        && (cli != TitleCli::Goose || entry["type"] == "stdio" && entry["enabled"] == true))
}

pub(super) fn updated(
    cli: TitleCli,
    source: Option<&str>,
    expected: &Registration,
) -> Result<String, String> {
    let mut doc = document(source)?;
    let existing = entry(cli, &doc)?;
    if existing.is_some_and(|entry| {
        !owned(
            entry.get(command_key(cli)).and_then(Value::as_str),
            entry
                .get("args")
                .and_then(Value::as_array)
                .and_then(|args| args.iter().map(Value::as_str).collect()),
            expected,
        )
    }) {
        return Err("A different MCP server is already named lomi. Rename that entry before installing Lomi MCP.".into());
    }
    let mut value = existing.cloned().unwrap_or_else(|| json!({}));
    let server = value
        .as_object_mut()
        .ok_or("Lomi MCP settings must be a mapping.")?;
    server.insert(command_key(cli).into(), json!(expected.command));
    server.insert("args".into(), json!(expected.args));
    server.remove("url");
    server.remove("uri");
    if server.contains_key("disabled") {
        server.insert("disabled".into(), json!(false));
    }
    if server.contains_key("enabled") {
        server.insert("enabled".into(), json!(true));
    }
    if cli == TitleCli::Goose {
        server.insert("type".into(), json!("stdio"));
        server.insert("name".into(), json!("Lomi"));
        server.insert("enabled".into(), json!(true));
        server.entry("envs").or_insert_with(|| json!({}));
        server.entry("env_keys").or_insert_with(|| json!([]));
    }
    let object = doc.as_object_mut().unwrap();
    if cli == TitleCli::Continue {
        server.insert("name".into(), json!("lomi"));
        // Continue validates metadata even when only adding a server to a new local config.
        object
            .entry("name")
            .or_insert_with(|| json!("Local configuration"));
        object.entry("version").or_insert_with(|| json!("1.0.0"));
        object.entry("schema").or_insert_with(|| json!("v1"));
        let servers = object
            .entry(key(cli))
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("Continue mcpServers must be a list.")?;
        if let Some(index) = servers.iter().position(|entry| entry["name"] == "lomi") {
            servers[index] = value;
        } else {
            servers.push(value);
        }
    } else {
        object
            .entry(key(cli))
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("CLI MCP settings must be a mapping.")?
            .insert("lomi".into(), value);
    }
    serde_saphyr::to_string(&doc).map_err(|_| "Cannot serialize CLI YAML configuration.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_config;

    #[test]
    fn yaml_clients_preserve_settings_and_servers_and_do_not_reinstall() {
        let registration = Registration {
            command: "/Applications/Lomi.app/Contents/MacOS/lomi".into(),
            args: vec![
                "--mcp".into(),
                "--discovery-file".into(),
                "/private/lomi/discovery.json".into(),
            ],
        };
        for (cli, source) in [
            (TitleCli::Hermes, "# retained in backup\nmodel: test\nmcp_servers:\n  other:\n    command: keep\n"),
            (TitleCli::Goose, "# retained in backup\nGOOSE_MODEL: test\nextensions:\n  other:\n    cmd: keep\n    enabled: false\n"),
            (TitleCli::Continue, "# retained in backup\nname: test\nversion: 1.0.0\nschema: v1\nmodels: []\nmcpServers:\n  - name: other\n    command: keep\n"),
        ] {
            let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("config.yaml");
            std::fs::write(&path, source).unwrap();
            super::super::enable(cli, &path, cli_config::revision(Some(source)).as_deref(), &registration).unwrap();
            let output = std::fs::read_to_string(&path).unwrap();
            assert!(configured(cli, Some(&output), Some(&registration)).unwrap());
            let parsed = document(Some(&output)).unwrap();
            let previous = document(Some(source)).unwrap();
            for (name, value) in previous.as_object().unwrap() { if name != key(cli) { assert_eq!(parsed[name], *value); } }
            if cli == TitleCli::Continue { assert_eq!(parsed["mcpServers"][0]["command"], "keep"); }
            else { assert_eq!(parsed[key(cli)]["other"], previous[key(cli)]["other"]); }
            super::super::enable(cli, &path, cli_config::revision(Some(&output)).as_deref(), &registration).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), output);
            let backup = std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).find(|entry| entry.file_name().to_string_lossy().contains("lomi-backup")).unwrap();
            assert_eq!(std::fs::read_to_string(backup.path()).unwrap(), source);
        }
    }

    #[test]
    fn yaml_preserves_unsupported_or_conflicting_documents() {
        let registration = Registration {
            command: "/bin/lomi".into(),
            args: vec!["--mcp".into()],
        };
        for (cli, source) in [
            (TitleCli::Hermes, "mcp_servers:\n  lomi: {}"),
            (TitleCli::Goose, "extensions: []"),
            (
                TitleCli::Continue,
                "mcpServers:\n - name: lomi\n - name: lomi",
            ),
            (TitleCli::Hermes, "model: a\nmodel: b"),
            (TitleCli::Hermes, "model: &a [one]\nother: *a"),
            (TitleCli::Hermes, "model: !include private.yaml"),
            (TitleCli::Hermes, "---\nmodel: a\n---\nmodel: b"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.yaml");
            std::fs::write(&path, source).unwrap();
            assert!(
                super::super::enable(
                    cli,
                    &path,
                    cli_config::revision(Some(source)).as_deref(),
                    &registration
                )
                .is_err(),
                "{source}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        }
    }
}
