use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::Manager;

fn isolated(app: &tauri::AppHandle) -> Result<(PathBuf, String, String), String> {
    let identifier = std::env::var("LOMI_LINUX_INSTANCE_IDENTIFIER").map_err(|e| e.to_string())?;
    if !identifier.starts_with("dev.lomi.linux-instance-smoke-")
        || app.config().identifier != identifier
    {
        return Err("Linux instance fixture requires its exact isolated build identity".into());
    }
    let root = std::fs::canonicalize(
        std::env::var("LOMI_LINUX_INSTANCE_DIRECTORY").map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let data = std::fs::canonicalize(app.path().app_data_dir().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let expected = std::fs::canonicalize(root.join("xdg-data").join(&identifier))
        .map_err(|e| e.to_string())?;
    let xdg = std::fs::canonicalize(std::env::var("XDG_DATA_HOME").map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let token = std::env::var("LOMI_LINUX_INSTANCE_TOKEN").map_err(|e| e.to_string())?;
    let phase = std::env::var("LOMI_LINUX_INSTANCE_PHASE").map_err(|e| e.to_string())?;
    if data != expected
        || xdg != root.join("xdg-data")
        || token.len() < 32
        || !matches!(phase.as_str(), "primary" | "restore")
    {
        return Err("Linux instance fixture profile is outside its owned XDG root".into());
    }
    Ok((root, token, phase))
}

pub(super) fn owner_contended(app: &tauri::AppHandle) -> std::io::Result<()> {
    let (root, _, phase) = isolated(app).map_err(std::io::Error::other)?;
    let data = std::fs::canonicalize(app.path().app_data_dir().map_err(std::io::Error::other)?)?;
    let unix_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos()
        .to_string();
    let path = root.join(format!("{phase}-owner-contended.json"));
    let temporary = path.with_extension("tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&json!({
            "name":"owner-contended", "phase":phase, "pid":std::process::id(),
            "unixNs":unix_ns,
            "paths":[data.join("agent-runtime/owner.lock"), data.join("chat-ai/owner.lock")]
        }))
        .map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(temporary, path)
}

pub fn result(app: &tauri::AppHandle, stage: &str, data: Value) -> Result<Value, String> {
    let (root, token, phase) = isolated(app)?;
    if stage == "instance-bootstrap" {
        return Ok(json!({"root":root,"token":token,"phase":phase}));
    }
    if data.get("token").and_then(Value::as_str) != Some(token.as_str()) {
        return Err("Invalid Linux instance fixture token".into());
    }
    let main = app.get_window("main").ok_or("Missing main window")?;
    match stage {
        "instance-gate" => {
            let gate = data
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("startup");
            if !matches!(gate, "startup" | "cycle-1" | "cycle-2") {
                return Err("Unknown Linux instance gate".into());
            }
            let open = std::fs::read_to_string(root.join(format!("{phase}-continue-{gate}")))
                .is_ok_and(|contents| contents == token);
            Ok(json!(open))
        }
        "instance-credential-probe" => {
            let service = format!("{}.credential-probe", app.config().identifier);
            let entry = keyring::Entry::new(&service, &token)
                .map_err(|_| "Fixture credential entry initialization failed")?;
            use ring::rand::SecureRandom;
            let mut random = [0u8; 32];
            ring::rand::SystemRandom::new()
                .fill(&mut random)
                .map_err(|_| "Fixture credential sample generation failed")?;
            let sample = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let set = entry.set_password(&sample).is_ok();
            let matched = set && entry.get_password().is_ok_and(|value| value == sample);
            // Cleanup is attempted even when writing or reading the test item fails.
            let deleted = entry.delete_credential().is_ok();
            let absent = matches!(entry.get_password(), Err(keyring::Error::NoEntry));
            if !set || !matched || !deleted || !absent {
                return Err(format!(
                    "Fixture credential round trip failed (set={set}, matched={matched}, deleted={deleted}, absent={absent})"
                ));
            }
            Ok(json!({"set":set,"matched":matched,"deleted":deleted,"absent":absent}))
        }
        "instance-close" => {
            main.close().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "instance-inspect" | "instance-checkpoint" => {
            let state = json!({
                "pid":std::process::id(),
                "visible":main.is_visible().map_err(|e| e.to_string())?,
                "focused":main.is_focused().map_err(|e| e.to_string())?,
                "terminals":app.state::<crate::terminal::Terminals>().smoke_sessions()
            });
            if stage == "instance-checkpoint" {
                let name = data
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("Missing checkpoint")?;
                if !matches!(
                    name,
                    "pre-ready"
                        | "startup-released"
                        | "running"
                        | "hidden-1"
                        | "reopened-1"
                        | "hidden-2"
                        | "reopened-2"
                        | "ready-to-quit"
                        | "failed"
                ) {
                    return Err("Unknown Linux instance checkpoint".into());
                }
                let path = root.join(format!("{phase}-{name}.json"));
                let temporary = path.with_extension("tmp");
                std::fs::write(
                    &temporary,
                    serde_json::to_vec_pretty(&json!({
                        "name":name,"phase":phase,"state":state,"data":data.get("details")
                    }))
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                std::fs::rename(temporary, path).map_err(|e| e.to_string())?;
            }
            Ok(state)
        }
        _ => Err("Unknown Linux instance fixture stage".into()),
    }
}
