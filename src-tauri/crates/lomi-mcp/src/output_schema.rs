use lomi_control_protocol::control::Reply;
use serde_json::{Map, Value};
use std::{collections::HashSet, sync::Arc};

/// Preserve the generated closed wire schemas, selecting only this tool's
/// possible variants and their transitively reachable definitions.
pub fn for_tool(tool: &str) -> Arc<Map<String, Value>> {
    let kinds: &[&str] = match tool {
        "lomi_settings_read" => &["settings_snapshot"],
        "lomi_git_diff" => &["git_diff"],
        "lomi_git_history" => &["git_history"],
        "lomi_git_commit" => &["git_commit"],
        "lomi_git_remotes" => &["git_remotes"],
        "lomi_git_open" | "lomi_git_mutate" => &["operation"],
        "lomi_git_status" => &["git_status"],
        "lomi_files_mutate" => &["operation"],
        "lomi_editor_save" => &["operation"],
        "lomi_editor_open" => &["operation"],
        "lomi_editor_apply_edits" => &["operation"],
        "lomi_editor_read" => &["editor_text"],
        "lomi_files_search" => &["files_search"],
        "lomi_files_list" => &["files_list"],
        "lomi_files_read" => &["file_text"],
        "lomi_status" => &["status"],
        "lomi_diagnostics" => &["diagnostics"],
        "lomi_connect" => &["connected"],
        "lomi_workspace_list" => &["workspaces"],
        "lomi_panel_list" => &["panels"],
        "lomi_events_read" => &["events"],
        "lomi_terminal_read" => &["terminal_output", "terminal_raw", "terminal_screen"],
        "lomi_terminal_input" => &["terminal_input_ack"],
        "lomi_browser_snapshot" => &["browser_snapshot"],
        "lomi_browser_wait" => &["browser_wait"],
        "lomi_browser_logs" => &["browser_logs"],
        "lomi_android_list" => &["android_devices"],
        "lomi_android_snapshot" => &["android_snapshot"],
        "lomi_android_logcat" => &["android_logcat"],
        "lomi_browser_screenshot" | "lomi_android_screenshot" | "lomi_artifact_read" => {
            &["artifact"]
        }
        "lomi_operation_get"
        | "lomi_operation_cancel"
        | "lomi_workspace_create"
        | "lomi_workspace_update"
        | "lomi_settings_open"
        | "lomi_settings_update"
        | "lomi_project_open"
        | "lomi_project_close"
        | "lomi_panel_move"
        | "lomi_panel_focus"
        | "lomi_panel_control"
        | "lomi_panel_close"
        | "lomi_terminal_create"
        | "lomi_terminal_run"
        | "lomi_terminal_interrupt"
        | "lomi_browser_open"
        | "lomi_browser_navigate"
        | "lomi_browser_click"
        | "lomi_browser_fill"
        | "lomi_browser_key"
        | "lomi_browser_scroll"
        | "lomi_android_open"
        | "lomi_android_start"
        | "lomi_android_stop"
        | "lomi_android_input"
        | "lomi_artifact_import"
        | "lomi_android_install_apk"
        | "lomi_android_launch" => &["operation"],
        _ => panic!("Tool has no output contract: {tool}"),
    };
    let mut schema = serde_json::to_value(schemars::schema_for!(Reply)).unwrap();
    schema["$defs"]["Data"]["oneOf"]
        .as_array_mut()
        .unwrap()
        .retain(|variant| {
            variant["properties"]["kind"]["const"]
                .as_str()
                .is_some_and(|kind| kinds.contains(&kind))
        });
    if kinds.contains(&"operation") {
        let results: Option<&[&str]> = match tool {
            "lomi_operation_get" | "lomi_operation_cancel" => None,
            "lomi_settings_open" => Some(&["settings_opened"]),
            "lomi_settings_update" => Some(&["settings_updated"]),
            "lomi_project_open" => Some(&["project_opened"]),
            "lomi_project_close" => Some(&["project_closure"]),
            "lomi_workspace_create" => Some(&["workspace"]),
            "lomi_workspace_update" => Some(&["workspace", "panel", "workspace_closure"]),
            "lomi_panel_move" => Some(&["panel_moved"]),
            "lomi_panel_focus" | "lomi_panel_close" => Some(&["panel"]),
            "lomi_panel_control" => Some(&["terminal_control", "android_control"]),
            "lomi_terminal_create" => Some(&["terminal"]),
            "lomi_terminal_run" => Some(&["terminal_command"]),
            "lomi_terminal_interrupt" => Some(&["terminal_interrupt"]),
            "lomi_browser_open" => Some(&["browser"]),
            "lomi_browser_navigate" => Some(&["browser_navigation"]),
            "lomi_browser_click"
            | "lomi_browser_fill"
            | "lomi_browser_key"
            | "lomi_browser_scroll" => Some(&["browser_interaction"]),
            "lomi_android_open" => Some(&["android_panel"]),
            "lomi_android_start" | "lomi_android_stop" => Some(&["android_runtime"]),
            "lomi_android_input" => Some(&["android_input"]),
            "lomi_android_install_apk" => Some(&["android_install"]),
            "lomi_android_launch" => Some(&["android_launch"]),
            "lomi_artifact_import" => Some(&["artifact_imported"]),
            "lomi_files_mutate" => Some(&["files_mutated"]),
            "lomi_editor_save" => Some(&["editor_saved"]),
            "lomi_editor_open" => Some(&["editor_opened", "editor_previewed"]),
            "lomi_editor_apply_edits" => Some(&["editor_edited"]),
            "lomi_git_open" => Some(&["git_opened"]),
            "lomi_git_mutate" => Some(&["git_mutated"]),
            _ => panic!("Tool has no operation result contract: {tool}"),
        };
        if let Some(results) = results {
            schema["$defs"]["OperationResult"]["oneOf"]
                .as_array_mut()
                .unwrap()
                .retain(|variant| {
                    variant["properties"]["kind"]["const"]
                        .as_str()
                        .is_some_and(|kind| kind == "failure" || results.contains(&kind))
                });
        }
    }
    let mut definitions = schema.as_object_mut().unwrap().remove("$defs").unwrap();
    let mut reachable = HashSet::new();
    collect_refs(&schema, &mut reachable);
    loop {
        let before = reachable.len();
        for name in reachable.clone() {
            collect_refs(&definitions[&name], &mut reachable);
        }
        if before == reachable.len() {
            break;
        }
    }
    definitions
        .as_object_mut()
        .unwrap()
        .retain(|key, _| reachable.contains(key));
    schema["$defs"] = definitions;
    Arc::new(schema.as_object().unwrap().clone())
}
fn collect_refs(value: &Value, references: &mut HashSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(name) = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|s| s.strip_prefix("#/$defs/"))
            {
                references.insert(name.to_owned());
            }
            for value in object.values() {
                collect_refs(value, references);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_refs(value, references);
            }
        }
        _ => {}
    }
}
