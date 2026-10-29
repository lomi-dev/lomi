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
