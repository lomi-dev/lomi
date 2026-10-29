use crate::cli_catalog::TitleCli;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::Path,
    sync::Mutex,
    time::SystemTime,
};
use tauri::{Emitter, Manager, State, Window};

const LIMIT: usize = 512 * 1024;
const MAX_ITEMS: usize = 200;
const MAX_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_TIMESTAMP: u64 = 8_640_000_000_000_000;

#[derive(Default)]
pub struct Notifications(pub Mutex<()>);

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationKind {
    Attention,
    Finished,
    Info,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApplicationNotificationSource {
    App,
    Android,
    Mcp,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApplicationNotificationAction {
    RecoverSession,
    RetryMcpStartup,
    OpenAgentControlSettings,
    OpenAndroidSettings,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Notification {
    id: String,
    kind: NotificationKind,
    #[serde(default)]
    agent: Option<TitleCli>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<ApplicationNotificationSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action: Option<ApplicationNotificationAction>,
    title: String,
    body: String,
    created_at: u64,
    read: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    revision: u64,
    items: Vec<Notification>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    revision: u64,
    items: Vec<Notification>,
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

fn valid_text(value: &str, max: usize) -> bool {
    value.chars().count() <= max && !value.chars().any(char::is_control)
}

impl Snapshot {
    fn validate(&self) -> Result<(), String> {
        let mut ids = HashSet::new();
        if self.revision > MAX_INTEGER
            || self.items.len() > MAX_ITEMS
            || self.items.iter().any(|item| {
                !valid_id(&item.id)
                    || item.pane_id.as_deref().is_some_and(|id| !valid_id(id))
                    || (item.source.is_some() && item.pane_id.is_some())
                    || !item
                        .id
                        .strip_prefix("notification-")
                        .and_then(|sequence| sequence.parse::<u64>().ok())
                        .is_some_and(|sequence| sequence > 0 && sequence <= self.revision)
                    || !ids.insert(&item.id)
                    || item.title.is_empty()
                    || !valid_text(&item.title, 100)
                    || !valid_text(&item.body, if item.source.is_some() { 4000 } else { 300 })
                    || item.created_at > MAX_TIMESTAMP
            })
        {
            return Err("Invalid notification inbox.".into());
        }
        Ok(())
    }
}

fn read(path: &Path) -> Result<Snapshot, String> {
    crate::chat::storage::reject_link(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => {
            return Err("The notification inbox must be a regular file.".into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Snapshot::default())
        }
        Err(error) => return Err(error.to_string()),
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Snapshot::default())
        }
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        if bytes.len() > LIMIT {
            return Err("The notification inbox exceeds 512 KiB.".into());
        }
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        let version: u32 = serde_json::from_value(
            value
                .get("version")
                .cloned()
                .ok_or("Invalid notification inbox.")?,
        )
        .map_err(|error| error.to_string())?;
        if !matches!(version, 1 | 2) {
            return Err("Unsupported notification inbox version.".into());
        }
        let mut retired = HashSet::new();
        for item in value
            .get_mut("items")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or("Invalid notification inbox.")?
        {
            if item
                .get("agent")
                .and_then(serde_json::Value::as_str)
                .is_some_and(crate::cli_catalog::is_retired_id)
            {
                retired.insert(
                    item["id"]
                        .as_str()
                        .ok_or("Invalid notification identifier.")?
                        .to_string(),
                );
                item["agent"] = serde_json::Value::Null;
            }
        }
        let data: Stored = serde_json::from_value(value).map_err(|error| error.to_string())?;
        let mut snapshot = Snapshot {
            revision: data.revision,
            items: data.items,
        };
        snapshot.validate()?;
        if !retired.is_empty() {
            snapshot.items.retain(|item| !retired.contains(&item.id));
            snapshot.revision = snapshot
                .revision
                .checked_add(1)
                .filter(|revision| *revision <= MAX_INTEGER)
                .ok_or("Notification revision limit reached.")?;
            let parent = path.parent().ok_or("Invalid notification inbox path.")?;
            let mut bytes_id = [0u8; 16];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes_id)
                .map_err(|_| "Cannot create a notification backup identifier.".to_string())?;
            let backup_id: String = bytes_id.iter().map(|byte| format!("{byte:02x}")).collect();
            let backup = parent.join(format!(
                "notifications-before-cli-removal-{}.json",
                backup_id
            ));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let mut file = options.open(&backup).map_err(|error| error.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| error.to_string())?;
            #[cfg(unix)]
            fs::File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|error| error.to_string())?;
            crate::files::write_json(
                path,
                &Stored {
                    version: 2,
                    revision: snapshot.revision,
                    items: snapshot.items.clone(),
                },
                LIMIT,
            )?;
        }
        snapshot
            .items
            .sort_by_key(|item| std::cmp::Reverse(item.created_at));
        Ok(snapshot)
    })();
    result.map_err(|error: String| {
        format!(
            "Cannot load notifications ({error}). The file has been left intact at {}.",
            path.display()
        )
    })
}

// The caller holds Notifications' mutex through both persistence and publication.
fn update(path: &Path, change: impl FnOnce(&mut Snapshot) -> bool) -> Result<Snapshot, String> {
    try_update(path, |snapshot| Ok(change(snapshot)))
}

fn try_update(
    path: &Path,
    change: impl FnOnce(&mut Snapshot) -> Result<bool, String>,
) -> Result<Snapshot, String> {
    let mut snapshot = read(path)?;
    if change(&mut snapshot)? {
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_INTEGER)
            .ok_or("Notification revision limit reached.")?;
        snapshot.validate()?;
        let parent = path.parent().ok_or("Invalid notification inbox path.")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        crate::files::write_json(
            path,
            &Stored {
                version: 2,
                revision: snapshot.revision,
                items: snapshot.items.clone(),
            },
            LIMIT,
        )?;
    }
    Ok(snapshot)
}

fn path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("notifications.json"))
}

fn publish(app: &tauri::AppHandle, snapshot: Snapshot) -> Result<Snapshot, String> {
    app.emit_to("main", "notifications-changed", &snapshot)
        .map_err(|error| error.to_string())?;
    Ok(snapshot)
}

#[cfg(test)]
fn append(
    path: &Path,
    kind: NotificationKind,
    agent: Option<TitleCli>,
    title: &str,
    body: &str,
    now: u64,
) -> Result<Snapshot, String> {
    append_targeted(path, kind, agent, None, title, body, now)
}

fn append_targeted(
    path: &Path,
    kind: NotificationKind,
    agent: Option<TitleCli>,
    pane_id: Option<&str>,
    title: &str,
    body: &str,
    now: u64,
) -> Result<Snapshot, String> {
    append_item(
        path,
        Notification {
            id: String::new(),
            kind,
            agent,
            pane_id: pane_id.map(str::to_owned),
            source: None,
            action: None,
            title: title.into(),
            body: body.into(),
            created_at: now,
            read: false,
        },
    )
}

fn append_item(path: &Path, mut item: Notification) -> Result<Snapshot, String> {
    if item.pane_id.as_deref().is_some_and(|id| !valid_id(id))
        || (item.source.is_some() && item.pane_id.is_some())
    {
        return Err("Invalid notification target.".into());
    }
    if item.title.is_empty()
        || !valid_text(&item.title, 100)
        || !valid_text(&item.body, if item.source.is_some() { 4000 } else { 300 })
        || item.created_at > MAX_TIMESTAMP
    {
        return Err("Invalid notification text or timestamp.".into());
    }
    try_update(path, |snapshot| {
        let revision = snapshot
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_INTEGER)
            .ok_or("Notification revision limit reached.")?;
        item.id = format!("notification-{revision}");
        item.created_at = item
            .created_at
            .max(snapshot.items.first().map_or(0, |item| item.created_at));
        snapshot.items.insert(0, item);
        snapshot.items.truncate(MAX_ITEMS);
        // Match write_json's pretty serialization, including the next revision.
        while serde_json::to_vec_pretty(&Stored {
            version: 2,
            revision,
            items: snapshot.items.clone(),
        })
        .map_err(|error| error.to_string())?
        .len()
            > LIMIT
        {
            if snapshot.items.len() <= 1 {
                return Err("The notification exceeds the inbox size limit.".into());
            }
            snapshot.items.pop();
        }
        Ok(true)
    })
}

fn timestamp() -> Result<u64, String> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    u64::try_from(now).map_err(|_| "Invalid notification timestamp.".into())
}

#[tauri::command]
pub fn record_app_notification(
    window: Window,
    app: tauri::AppHandle,
    kind: NotificationKind,
    source: ApplicationNotificationSource,
    title: String,
    body: String,
    action: Option<ApplicationNotificationAction>,
) -> Result<Snapshot, String> {
    crate::files::main_window(&window)?;
    let state = app.state::<Notifications>();
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    let snapshot = append_item(
        &path(&app)?,
        Notification {
            id: String::new(),
            kind,
            agent: None,
            pane_id: None,
            source: Some(source),
            action,
            title,
            body,
            created_at: timestamp()?,
            read: false,
        },
    )?;
    // A failed event must not turn a committed append into a retriable error.
    let _ = app.emit_to("main", "notifications-changed", &snapshot);
    Ok(snapshot)
}

pub(crate) fn record(
    app: &tauri::AppHandle,
    kind: NotificationKind,
    agent: Option<TitleCli>,
    pane_id: Option<&str>,
    title: &str,
    body: &str,
) -> Result<(), String> {
    let state = app.state::<Notifications>();
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    publish(
        app,
        append_targeted(&path(app)?, kind, agent, pane_id, title, body, timestamp()?)?,
    )?;
    Ok(())
}

#[tauri::command]
pub fn load_notifications(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Notifications>,
) -> Result<Snapshot, String> {
    crate::files::main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    read(&path(&app)?)
}

fn mark_read(snapshot: &mut Snapshot, ids: &[String]) -> bool {
    let mut changed = false;
    for item in &mut snapshot.items {
        if !item.read && ids.contains(&item.id) {
            item.read = true;
            changed = true;
        }
    }
    changed
}

#[tauri::command]
pub fn mark_notifications_read(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Notifications>,
    ids: Vec<String>,
) -> Result<Snapshot, String> {
    crate::files::main_window(&window)?;
    if ids.len() > MAX_ITEMS || ids.iter().any(|id| !valid_id(id)) {
        return Err("Invalid notification IDs.".into());
    }
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    publish(
        &app,
        update(&path(&app)?, |snapshot| mark_read(snapshot, &ids))?,
    )
}

#[tauri::command]
pub fn dismiss_notification(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Notifications>,
    id: String,
) -> Result<Snapshot, String> {
    crate::files::main_window(&window)?;
    if !valid_id(&id) {
        return Err("Invalid notification ID.".into());
    }
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    publish(
        &app,
        update(&path(&app)?, |snapshot| {
            let before = snapshot.items.len();
            snapshot.items.retain(|item| item.id != id);
            snapshot.items.len() != before
        })?,
    )
}

#[tauri::command]
pub fn clear_read_notifications(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Notifications>,
) -> Result<Snapshot, String> {
    crate::files::main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    publish(
        &app,
        update(&path(&app)?, |snapshot| {
            let before = snapshot.items.len();
            snapshot.items.retain(|item| !item.read);
            snapshot.items.len() != before
        })?,
    )
}

fn clear_all(snapshot: &mut Snapshot) -> bool {
    let changed = !snapshot.items.is_empty();
    snapshot.items.clear();
    changed
}

#[tauri::command]
pub fn clear_notifications(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Notifications>,
) -> Result<Snapshot, String> {
    crate::files::main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    publish(&app, update(&path(&app)?, clear_all)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_item(body: &str) -> Notification {
        Notification {
            id: String::new(),
            kind: NotificationKind::Info,
            agent: None,
            pane_id: None,
            source: Some(ApplicationNotificationSource::App),
            action: None,
            title: "Application event".into(),
            body: body.into(),
            created_at: 1,
            read: false,
        }
    }

    #[test]
    fn roundtrips_application_metadata_and_info_in_version_two() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        for (source, action, source_json, action_json) in [
            (
                ApplicationNotificationSource::App,
                ApplicationNotificationAction::RecoverSession,
                "app",
                "recover-session",
            ),
            (
                ApplicationNotificationSource::Android,
                ApplicationNotificationAction::OpenAndroidSettings,
                "android",
                "open-android-settings",
            ),
            (
                ApplicationNotificationSource::Mcp,
                ApplicationNotificationAction::RetryMcpStartup,
                "mcp",
                "retry-mcp-startup",
            ),
            (
                ApplicationNotificationSource::App,
                ApplicationNotificationAction::OpenAgentControlSettings,
                "app",
                "open-agent-control-settings",
            ),
        ] {
            let mut item = app_item(&"🦀".repeat(4000));
            item.title = "🦀".repeat(100);
            item.source = Some(source);
            item.action = Some(action);
            append_item(&path, item).unwrap();
            let loaded = read(&path).unwrap();
            assert_eq!(loaded.items[0].source, Some(source));
            assert_eq!(loaded.items[0].action, Some(action));
            assert_eq!(loaded.items[0].body.chars().count(), 4000);
            let data: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(data["version"], 2);
            assert_eq!(data["items"][0]["kind"], "info");
            assert_eq!(data["items"][0]["source"], source_json);
            assert_eq!(data["items"][0]["action"], action_json);
        }
        append_item(&path, app_item("")).unwrap();
        append(&path, NotificationKind::Finished, None, "Agent", "", 2).unwrap();
        let data: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(data["items"][0].get("source").is_none());
        assert!(data["items"][0].get("action").is_none());
        assert!(data["items"][1].get("action").is_none());
    }

    #[test]
    fn existing_mixed_inbox_preserves_application_metadata_across_agent_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let mut application = serde_json::json!({
            "id": "notification-2", "kind": "info", "agent": null,
            "source": "mcp", "action": "retry-mcp-startup",
            "title": "MCP startup failed", "body": "Detail ".repeat(100),
            "createdAt": 2, "read": false
        });
        let fixture = serde_json::json!({
            "version": 2, "revision": 2, "items": [
                {
                    "id": "notification-1", "kind": "finished", "agent": "codex",
                    "title": "Agent finished", "body": "Project", "createdAt": 1,
                    "read": false
                },
                application.clone()
            ]
        });
        let bytes = serde_json::to_vec_pretty(&fixture).unwrap();
        fs::write(&path, &bytes).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(loaded.revision, 2);
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(serde_json::to_value(&loaded.items[0]).unwrap(), application);
        assert_eq!(loaded.items[1].agent, Some(TitleCli::Codex));
        assert_eq!(fs::read(&path).unwrap(), bytes);

        let appended = append(
            &path,
            NotificationKind::Attention,
            Some(TitleCli::Claude),
            "Input needed",
            "Project",
            3,
        )
        .unwrap();
        assert_eq!(appended.revision, 3);
        assert_eq!(appended.items.len(), 3);
        assert_eq!(
            serde_json::to_value(&read(&path).unwrap().items[1]).unwrap(),
            application
        );

        let marked = update(&path, |snapshot| {
            mark_read(snapshot, &["notification-2".into()])
        })
        .unwrap();
        application["read"] = serde_json::json!(true);
        assert_eq!(marked.revision, 4);
        assert_eq!(
            serde_json::to_value(&read(&path).unwrap().items[1]).unwrap(),
            application
        );

        let dismissed = update(&path, |snapshot| {
            let before = snapshot.items.len();
            snapshot.items.retain(|item| item.id != "notification-1");
            snapshot.items.len() != before
        })
        .unwrap();
        assert_eq!(dismissed.revision, 5);
        assert_eq!(dismissed.items.len(), 2);
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], 2);
        assert_eq!(saved["items"][1], application);
        assert_eq!(
            serde_json::to_value(&read(&path).unwrap().items[1]).unwrap(),
            application
        );
    }

    #[test]
    fn future_metadata_reports_unsupported_version_without_changing_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let fixture = serde_json::json!({
            "version": 3, "revision": 1, "futureEnvelope": { "format": "next" },
            "items": [{
                "id": "notification-1", "kind": "future-kind", "agent": "goose",
                "source": "future-source", "action": "future-action",
                "futureMetadata": { "retry": true }, "title": "Future notification",
                "body": "Details", "createdAt": 1, "read": false
            }]
        });
        let bytes = serde_json::to_vec_pretty(&fixture).unwrap();
        fs::write(&path, &bytes).unwrap();
        let error = read(&path).err().unwrap();
        assert!(
            error.contains("Unsupported notification inbox version."),
            "{error}"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let error = append(&path, NotificationKind::Attention, None, "Input", "", 2)
            .err()
            .unwrap();
        assert!(
            error.contains("Unsupported notification inbox version."),
            "{error}"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_invalid_application_text_without_changing_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        append_item(&path, app_item("Original")).unwrap();
        let before = fs::read(&path).unwrap();
        for (title, body) in [
            (String::new(), "Body".into()),
            ("a".repeat(101), "Body".into()),
            ("Title".into(), "a".repeat(4001)),
            ("Bad\nTitle".into(), "Body".into()),
            ("Title".into(), "Bad\tBody".into()),
            ("Title".into(), "Bad\u{0085}Body".into()),
        ] {
            let mut item = app_item(&body);
            item.title = title;
            assert!(append_item(&path, item).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        assert!(append(
            &path,
            NotificationKind::Attention,
            None,
            "Agent",
            &"a".repeat(301),
            2
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn application_append_prunes_oldest_by_serialized_bytes_without_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let body = "🦀".repeat(4000);
        for time in 1..=40 {
            let mut item = app_item(&body);
            item.created_at = time;
            append_item(&path, item).unwrap();
            assert!(fs::metadata(&path).unwrap().len() <= LIMIT as u64);
        }
        let snapshot = read(&path).unwrap();
        assert_eq!(snapshot.revision, 40);
        assert!(snapshot.items.len() < 40);
        assert!(snapshot.items.iter().all(|item| item.body == body));
        assert_eq!(snapshot.items[0].id, "notification-40");
        assert_eq!(
            snapshot.items.last().unwrap().created_at,
            41 - snapshot.items.len() as u64
        );
    }

    #[test]
    fn application_append_preserves_corrupt_files_and_invalid_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        for data in [
            "broken".into(),
            " ".repeat(LIMIT + 1),
            r#"{"version":2,"revision":1,"items":[{"id":"notification-1","kind":"info","source":"unknown","title":"Event","body":"","createdAt":1,"read":false}]}"#.into(),
            r#"{"version":2,"revision":1,"items":[{"id":"notification-1","kind":"info","source":"app","action":"run-shell","title":"Event","body":"","createdAt":1,"read":false}]}"#.into(),
        ] {
            fs::write(&path, &data).unwrap();
            assert!(append_item(&path, app_item("New event")).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), data);
        }
    }

    #[test]
    fn retired_notifications_migrate_with_exact_backup_and_keep_surviving_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let source = r#"{"version":2,"revision":2,"items":[{"id":"notification-1","kind":"finished","agent":"goose","title":"Retired task","body":"","createdAt":1,"read":false},{"id":"notification-2","kind":"attention","agent":"codex","title":"Keep task","body":"","createdAt":2,"read":false}]}"#;
        fs::write(&path, source).unwrap();
        let snapshot = read(&path).unwrap();
        assert_eq!(snapshot.revision, 3);
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].agent, Some(TitleCli::Codex));
        let backups: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("notifications-before-cli-removal-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        let filename = backups[0].file_name().unwrap().to_str().unwrap();
        let backup_id = filename
            .strip_prefix("notifications-before-cli-removal-")
            .unwrap()
            .strip_suffix(".json")
            .unwrap();
        assert_eq!(backup_id.len(), 32);
        assert!(backup_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), source);
        assert_eq!(read(&path).unwrap().revision, 3);
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["items"].as_array().unwrap().len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&backups[0]).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn unknown_or_invalid_retired_notifications_preserve_original_inbox() {
        for (agent, title) in [("unknown-cli", "Task"), ("goose", "")] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("notifications.json");
            let source = serde_json::json!({"version":2,"revision":1,"items":[{"id":"notification-1","kind":"finished","agent":agent,"title":title,"body":"","createdAt":1,"read":false}]}).to_string();
            fs::write(&path, &source).unwrap();
            assert!(read(&path).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn roundtrips_agent_metadata_in_version_two() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        for agent in [Some(TitleCli::Claude), Some(TitleCli::Codex), None] {
            append(&path, NotificationKind::Finished, agent, "Finished", "", 1).unwrap();
            assert_eq!(read(&path).unwrap().items[0].agent, agent);
        }
        let data: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(data["version"], 2);
        assert_eq!(data["items"][1]["agent"], "codex");
        assert_eq!(data["items"][2]["agent"], "claude");
    }

    #[test]
    fn legacy_items_load_without_pane_ids_and_new_targets_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let legacy = r#"{"version":2,"revision":1,"items":[{"id":"notification-1","kind":"finished","agent":"codex","title":"Finished","body":"","createdAt":1,"read":false}]}"#;
        fs::write(&path, legacy).unwrap();

        let loaded = read(&path).unwrap();
        assert_eq!(loaded.items[0].pane_id, None);
        assert_eq!(fs::read_to_string(&path).unwrap(), legacy);

        let target = "12345678-1234-4234-8234-123456789abc";
        let appended = append_targeted(
            &path,
            NotificationKind::Attention,
            Some(TitleCli::Claude),
            Some(target),
            "Input needed",
            "Project",
            2,
        )
        .unwrap();
        assert_eq!(appended.revision, 2);
        assert_eq!(appended.items[0].pane_id.as_deref(), Some(target));
        assert_eq!(appended.items[1].pane_id, None);

        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], 2);
        assert_eq!(saved["items"][0]["paneId"], target);
        assert!(saved["items"][1].get("paneId").is_none());
        assert_eq!(
            read(&path).unwrap().items[0].pane_id.as_deref(),
            Some(target)
        );
    }

    #[test]
    fn rejects_invalid_pane_targets_and_application_attribution_without_changing_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        append(&path, NotificationKind::Finished, None, "Finished", "", 1).unwrap();
        let before = fs::read(&path).unwrap();

        let too_long = "x".repeat(129);
        for target in ["", "not/a-pane", too_long.as_str()] {
            assert!(append_targeted(
                &path,
                NotificationKind::Attention,
                None,
                Some(target),
                "Input needed",
                "Project",
                2,
            )
            .is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }

        let mut application = app_item("Operational event");
        application.pane_id = Some("12345678-1234-4234-8234-123456789abc".into());
        assert!(append_item(&path, application).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn loads_legacy_agent_without_rewriting_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let legacy = r#"{"version":1,"revision":1,"items":[{"id":"notification-1","kind":"finished","title":"Finished","body":"","createdAt":1,"read":false}]}"#;
        fs::write(&path, legacy).unwrap();
        let snapshot = read(&path).unwrap();
        assert_eq!(snapshot.items[0].agent, None);
        assert_eq!(snapshot.items[0].source, None);
        assert_eq!(snapshot.items[0].action, None);
        assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
        append(
            &path,
            NotificationKind::Attention,
            Some(TitleCli::Claude),
            "Input",
            "",
            2,
        )
        .unwrap();
        let snapshot = read(&path).unwrap();
        assert_eq!(snapshot.items[1].agent, None);
        let data: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(data["version"], 2);
    }

    #[test]
    fn rejects_invalid_agent_without_replacing_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        append(
            &path,
            NotificationKind::Finished,
            Some(TitleCli::Codex),
            "Finished",
            "",
            1,
        )
        .unwrap();
        let mut data: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        data["items"][0]["agent"] = serde_json::json!("unknown-agent");
        let before = serde_json::to_vec(&data).unwrap();
        fs::write(&path, &before).unwrap();
        assert!(read(&path).is_err());
        assert!(append(&path, NotificationKind::Attention, None, "Input", "", 2).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn persists_restart_and_explicit_reads_preserve_concurrent_arrivals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        assert_eq!(read(&path).unwrap().revision, 0);
        let first = append(
            &path,
            NotificationKind::Attention,
            None,
            "Needs input",
            "Project",
            10,
        )
        .unwrap();
        let ids = vec![first.items[0].id.clone()];
        append(
            &path,
            NotificationKind::Finished,
            None,
            "Finished",
            "Project",
            9,
        )
        .unwrap();
        let result = update(&path, |snapshot| mark_read(snapshot, &ids)).unwrap();
        assert_eq!(result.revision, 3);
        assert!(!result.items[0].read);
        assert!(result.items[1].read);
        assert_eq!(read(&path).unwrap().revision, 3);
        assert_eq!(
            update(&path, |snapshot| mark_read(snapshot, &ids))
                .unwrap()
                .revision,
            3
        );
        assert_eq!(
            update(&path, |snapshot| mark_read(snapshot, &["missing".into()]))
                .unwrap()
                .revision,
            3
        );
    }

    #[test]
    fn serializes_concurrent_arrivals_without_lost_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let state = std::sync::Arc::new(Notifications::default());
        let workers: Vec<_> = (0..8)
            .map(|time| {
                let state = state.clone();
                let path = path.clone();
                std::thread::spawn(move || {
                    let _guard = state.0.lock().unwrap();
                    append(
                        &path,
                        NotificationKind::Finished,
                        None,
                        "Finished",
                        "",
                        time,
                    )
                    .unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let snapshot = read(&path).unwrap();
        assert_eq!(snapshot.revision, 8);
        assert_eq!(snapshot.items.len(), 8);
    }

    #[test]
    fn clears_only_read_items_and_retains_revision_after_emptying() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let first = append(&path, NotificationKind::Attention, None, "Input", "", 1).unwrap();
        append(&path, NotificationKind::Finished, None, "Finished", "", 2).unwrap();
        update(&path, |snapshot| {
            mark_read(snapshot, &[first.items[0].id.clone()])
        })
        .unwrap();
        let result = update(&path, |snapshot| {
            let before = snapshot.items.len();
            snapshot.items.retain(|item| !item.read);
            snapshot.items.len() != before
        })
        .unwrap();
        assert_eq!(result.items.len(), 1);
        assert!(!result.items[0].read);
        let last_id = result.items[0].id.clone();
        update(&path, |snapshot| {
            snapshot.items.clear();
            true
        })
        .unwrap();
        let restarted = append(&path, NotificationKind::Finished, None, "Finished", "", 3).unwrap();
        assert_eq!(restarted.revision, 6);
        assert_ne!(restarted.items[0].id, last_id);
    }

    #[test]
    fn clears_read_and_unread_items_and_preserves_subsequent_arrivals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let state = Notifications::default();
        let first = append(&path, NotificationKind::Attention, None, "Input", "", 1).unwrap();
        let second = append(&path, NotificationKind::Finished, None, "Finished", "", 2).unwrap();
        update(&path, |snapshot| {
            mark_read(snapshot, &[first.items[0].id.clone()])
        })
        .unwrap();
        let before = read(&path).unwrap();
        assert!(before.items.iter().any(|item| item.read));
        assert!(before.items.iter().any(|item| !item.read));
        let cleared = {
            let _guard = state.0.lock().unwrap();
            update(&path, clear_all).unwrap()
        };
        assert_eq!(cleared.revision, 4);
        assert!(cleared.items.is_empty());
        let restarted = read(&path).unwrap();
        assert_eq!(restarted.revision, 4);
        assert!(restarted.items.is_empty());
        let saved = fs::read(&path).unwrap();
        {
            let _guard = state.0.lock().unwrap();
            assert_eq!(update(&path, clear_all).unwrap().revision, 4);
        }
        assert_eq!(fs::read(&path).unwrap(), saved);
        let arrival = {
            let _guard = state.0.lock().unwrap();
            append(&path, NotificationKind::Finished, None, "Finished", "", 3).unwrap()
        };
        assert_eq!(arrival.revision, 5);
        assert_eq!(arrival.items.len(), 1);
        assert_eq!(arrival.items[0].id, "notification-5");
        assert_ne!(arrival.items[0].id, first.items[0].id);
        assert_ne!(arrival.items[0].id, second.items[0].id);
        assert!(!read(&path).unwrap().items[0].read);
    }

    #[test]
    fn clear_all_preserves_corrupt_inboxes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let corrupt = "broken";
        fs::write(&path, corrupt).unwrap();
        assert!(update(&path, clear_all).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), corrupt);
    }

    #[test]
    fn rejects_invalid_items_without_replacing_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let snapshot = append(&path, NotificationKind::Attention, None, "Input", "", 1).unwrap();
        for case in 0..5 {
            let mut data = Stored {
                version: 1,
                revision: snapshot.revision,
                items: snapshot.items.clone(),
            };
            match case {
                0 => data.items = vec![data.items[0].clone(); MAX_ITEMS + 1],
                1 => data.items.push(data.items[0].clone()),
                2 => data.items[0].body = "control\n".into(),
                3 => data.items[0].created_at = MAX_TIMESTAMP + 1,
                _ => data.items[0].id = "notification-99".into(),
            }
            let before = serde_json::to_vec(&data).unwrap();
            fs::write(&path, &before).unwrap();
            assert!(update(&path, |_| true).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_and_preserves_their_targets() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.json");
        let path = dir.path().join("notifications.json");
        append(&target, NotificationKind::Attention, None, "Input", "", 1).unwrap();
        let before = fs::read(&target).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(append(&path, NotificationKind::Finished, None, "Finished", "", 2).is_err());
        assert!(path.is_symlink());
        assert_eq!(fs::read(&target).unwrap(), before);
    }

    #[test]
    fn bounds_history_and_keeps_ids_unique() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        for time in 0..205 {
            append(
                &path,
                NotificationKind::Finished,
                None,
                "Finished",
                "",
                time,
            )
            .unwrap();
        }
        let snapshot = read(&path).unwrap();
        assert_eq!(snapshot.revision, 205);
        assert_eq!(snapshot.items.len(), MAX_ITEMS);
        assert_eq!(snapshot.items[0].created_at, 204);
        assert_eq!(snapshot.items.last().unwrap().created_at, 5);
        assert_eq!(
            snapshot
                .items
                .iter()
                .map(|item| &item.id)
                .collect::<HashSet<_>>()
                .len(),
            MAX_ITEMS
        );
    }

    #[test]
    fn preserves_invalid_future_oversized_and_failed_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        for data in [
            "broken".into(),
            r#"{"version":3,"revision":0,"items":[]}"#.into(),
            " ".repeat(LIMIT + 1),
            r#"{"version":1,"revision":9007199254740992,"items":[]}"#.into(),
            r#"{"version":1,"revision":9007199254740991,"items":[]}"#.into(),
        ] {
            fs::write(&path, &data).unwrap();
            assert!(append(&path, NotificationKind::Attention, None, "Input", "", 0).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), data);
        }
        fs::remove_file(&path).unwrap();
        append(&path, NotificationKind::Attention, None, "Input", "", 0).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(append(
            &path,
            NotificationKind::Attention,
            None,
            "Input",
            &"a".repeat(301),
            0
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
