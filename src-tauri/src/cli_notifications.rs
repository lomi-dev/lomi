use crate::{
    agent_notifications,
    cli_config::{self, revision},
    cli_titles::TitleCli,
};
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

const CODEX_EVENTS: [&str; 2] = ["agent-turn-complete", "approval-requested"];

pub(crate) fn supported(cli: TitleCli) -> bool {
    matches!(cli, TitleCli::Codex | TitleCli::Claude)
}

pub(crate) fn inspect(cli: TitleCli, path: &Path) -> Result<bool, String> {
    match cli {
        TitleCli::Claude => Ok(agent_notifications::inspect(path)?.configured),
        TitleCli::Codex => {
            let source = cli_config::read(path)?;
            codex_configured(source.as_deref())
        }
        _ => Err("Notifications are not supported for this CLI.".into()),
    }
}

pub(crate) fn enable(cli: TitleCli, path: &Path, expected: Option<&str>) -> Result<(), String> {
    match cli {
        TitleCli::Claude => agent_notifications::enable(path, expected),
        TitleCli::Codex => enable_codex(path, expected),
        _ => Err("Notifications are not supported for this CLI.".into()),
    }
}

fn codex_document(source: Option<&str>) -> Result<DocumentMut, String> {
    let document = source
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|_| "Codex configuration is not valid TOML. The file was left intact.")?;
    let Some(tui) = document.get("tui") else {
        return Ok(document);
    };
    let table = tui
        .as_table_like()
        .ok_or("Codex tui settings must be a TOML table.")?;
    if let Some(notifications) = table.get("notifications") {
        let valid = notifications.as_bool().is_some()
            || notifications
                .as_array()
                .is_some_and(|items| items.iter().all(|item| item.as_str().is_some()));
        if !valid {
            return Err("Codex tui.notifications must be a boolean or an array of strings. The file was left intact.".into());
        }
    }
    for (key, allowed) in [
        ("notification_method", &["auto", "osc9", "bel"][..]),
        ("notification_condition", &["unfocused", "always"][..]),
    ] {
        if let Some(value) = table.get(key) {
            if !value.as_str().is_some_and(|value| allowed.contains(&value)) {
                return Err(format!(
                    "Codex tui.{key} has an invalid value. The file was left intact."
                ));
            }
        }
    }
    Ok(document)
}

fn codex_configured(source: Option<&str>) -> Result<bool, String> {
    let document = codex_document(source)?;
    let Some(tui) = document.get("tui") else {
        return Ok(false);
    };
    let table = tui
        .as_table_like()
        .ok_or("Codex tui settings must be a TOML table.")?;
    let notifications_enabled = match table.get("notifications") {
        None => true,
        Some(value) if value.as_bool() == Some(true) => true,
        Some(value) => value.as_array().is_some_and(|events| {
            CODEX_EVENTS
                .iter()
                .all(|event| events.iter().any(|item| item.as_str() == Some(event)))
        }),
    };
    Ok(notifications_enabled
        && table.get("notification_method").and_then(Item::as_str) == Some("osc9")
        && table.get("notification_condition").and_then(Item::as_str) == Some("always"))
}

fn set_codex_notifications(tui: &mut dyn toml_edit::TableLike) -> Result<(), String> {
    if tui.get("notifications").is_none() {
        return Ok(());
    }
    let notifications = tui
        .entry("notifications")
        .or_insert(Item::None)
        .as_value_mut()
        .ok_or("Codex tui.notifications must be a boolean or an array of strings.")?;
    let decor = notifications.decor().clone();
    match notifications {
        Value::Boolean(value) if *value.value() => {}
        Value::Boolean(_) => {
            let mut replacement = Value::Array(CODEX_EVENTS.into_iter().collect::<Array>());
            *replacement.decor_mut() = decor;
            *notifications = replacement;
        }
        Value::Array(events) => {
            for event in CODEX_EVENTS {
                if !events.iter().any(|item| item.as_str() == Some(event)) {
                    events.push(event);
                }
            }
        }
        _ => {
            return Err("Codex tui.notifications must be a boolean or an array of strings.".into())
        }
    }
    Ok(())
}

fn set_codex_string(tui: &mut dyn toml_edit::TableLike, key: &str, value: &str) {
    let item = tui.entry(key).or_insert(Item::None);
    let mut replacement = Value::from(value);
    if let Some(previous) = item.as_value() {
        *replacement.decor_mut() = previous.decor().clone();
    }
    *item = Item::Value(replacement);
}

fn enable_codex(path: &Path, expected: Option<&str>) -> Result<(), String> {
    let source = cli_config::read(path)?;
    if revision(source.as_deref()).as_deref() != expected {
        return Err(
            "Codex configuration changed. Review it again before enabling notifications.".into(),
        );
    }
    if codex_configured(source.as_deref())? {
        return Ok(());
    }
    let mut document = codex_document(source.as_deref())?;
    let tui = document
        .entry("tui")
        .or_insert(Item::Table(Table::new()))
        .as_table_like_mut()
        .ok_or("Codex tui settings must be a TOML table.")?;
    set_codex_notifications(tui)?;
    set_codex_string(tui, "notification_method", "osc9");
    set_codex_string(tui, "notification_condition", "always");
    let output = document.to_string();
    if codex_configured(Some(&output))? {
        cli_config::write(path, source.as_deref(), output)
    } else {
        Err("Codex notifications could not be enabled. The file was left intact.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn codex_setup_enables_supported_events_and_osc9_without_dropping_other_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let source = "# Preserve this note\nmodel = 'custom'\n[tui]\nnotifications = ['custom-event'] # preserve events\nnotification_method = 'bel'\nnotification_condition = 'unfocused'\ntheme = 'dark'\n";
        fs::write(&path, source).unwrap();
        let before = cli_config::revision(Some(source));
        assert!(!inspect(TitleCli::Codex, &path).unwrap());
        enable(TitleCli::Codex, &path, before.as_deref()).unwrap();
        let updated = fs::read_to_string(&path).unwrap();
        let document = updated.parse::<DocumentMut>().unwrap();
        let tui = document["tui"].as_table_like().unwrap();
        let events = tui.get("notifications").unwrap().as_array().unwrap();
        assert_eq!(
            events.iter().filter_map(Value::as_str).collect::<Vec<_>>(),
            ["custom-event", "agent-turn-complete", "approval-requested",]
        );
        assert_eq!(
            tui.get("notification_method").and_then(Item::as_str),
            Some("osc9")
        );
        assert_eq!(
            tui.get("notification_condition").and_then(Item::as_str),
            Some("always")
        );
        assert_eq!(document["model"].as_str(), Some("custom"));
        assert_eq!(tui.get("theme").and_then(Item::as_str), Some("dark"));
        assert!(updated.contains("# Preserve this note"));
        assert!(updated.contains("# preserve events"));
        assert!(inspect(TitleCli::Codex, &path).unwrap());

        let after_setup = updated.clone();
        fs::write(&path, "model = 'external edit'\n").unwrap();
        assert!(enable(TitleCli::Codex, &path, before.as_deref()).is_err());
        assert_ne!(fs::read_to_string(&path).unwrap(), after_setup);
    }

    #[test]
    fn codex_setup_rejects_invalid_values_without_rewriting_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        for source in [
            "tui = false\n",
            "[tui]\nnotifications = 1\n",
            "[tui]\nnotifications = ['event', 1]\n",
            "[tui]\nnotification_method = 'native'\n",
            "[tui]\nnotification_condition = 'sometimes'\n",
        ] {
            fs::write(&path, source).unwrap();
            let expected = cli_config::revision(Some(source));
            assert!(inspect(TitleCli::Codex, &path).is_err());
            assert!(enable(TitleCli::Codex, &path, expected.as_deref()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
    }

    #[test]
    fn codex_setup_respects_the_default_enabled_notifications_flag() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        for source in ["", "[tui]\ntheme = 'dark'\n"] {
            fs::write(&path, source).unwrap();
            let expected = cli_config::revision(Some(source));
            enable(TitleCli::Codex, &path, expected.as_deref()).unwrap();
            assert!(inspect(TitleCli::Codex, &path).unwrap());
            let updated = fs::read_to_string(&path).unwrap();
            assert!(!updated.contains("notifications ="));
        }
    }

    #[test]
    fn codex_setup_does_not_rewrite_an_already_configured_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let source = "model = 'custom'\n[tui]\nnotifications = true\nnotification_method = 'osc9'\nnotification_condition = 'always'\n";
        fs::write(&path, source).unwrap();
        let expected = cli_config::revision(Some(source));
        enable(TitleCli::Codex, &path, expected.as_deref()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        assert!(enable(TitleCli::Codex, &path, None).is_err());
    }

    #[test]
    fn claude_adapter_reuses_the_existing_hook_installer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(&path, json!({"env":{"CUSTOM":"keep"}}).to_string()).unwrap();
        assert!(supported(TitleCli::Claude));
        assert!(!inspect(TitleCli::Claude, &path).unwrap());
        let revision = cli_config::revision(Some(&fs::read_to_string(&path).unwrap()));
        enable(TitleCli::Claude, &path, revision.as_deref()).unwrap();
        assert!(inspect(TitleCli::Claude, &path).unwrap());
        assert!(!supported(TitleCli::Cursor));
        assert!(inspect(TitleCli::Cursor, &path).is_err());
    }
}
