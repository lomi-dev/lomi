use super::{credentials, process::valid_id, storage, store::Config};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    pub enabled: bool,
    pub credential_revision: u64,
    pub secret_mode: String,
    pub secret_id: Option<String>,
    pub models: Vec<String>,
    pub tested_model: Option<String>,
    pub test_status: Option<String>,
}
impl Connection {
    pub fn same_destination(&self, other: &Self) -> bool {
        self.provider == other.provider
            && self.base_url == other.base_url
            && self.api_format == other.api_format
    }

    pub fn configure_payload(&self, payload: &mut serde_json::Value) {
        if self.provider == "custom" {
            payload["baseUrl"] = serde_json::json!(self.base_url);
            payload["apiFormat"] = serde_json::json!(self.api_format);
        }
    }
}

fn valid_custom_base_url(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 2048
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\\' | '?' | '#'))
    {
        return false;
    }
    let Ok(url) = tauri::Url::parse(value) else {
        return false;
    };
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host == "[::1]"
            || host
                .parse::<std::net::Ipv4Addr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
        && (url.scheme() == "https" || (url.scheme() == "http" && loopback))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub revision: u64,
    pub connections: Vec<Connection>,
    pub defaults: Config,
    pub send_mode: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            connections: Vec::new(),
            defaults: Config::default(),
            send_mode: "enter".into(),
        }
    }
}
impl Preferences {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.connections.len() > 64
            || !matches!(self.send_mode.as_str(), "enter" | "modifier-enter")
            || self.revision >= (1u64 << 53) - 1
        {
            return Err("Unsupported Chat AI preferences. The original file was preserved.".into());
        }
        self.defaults.validate()?;
        let mut ids = HashSet::new();
        for connection in &self.connections {
            if !valid_id(&connection.id)
                || !ids.insert(&connection.id)
                || connection.name.trim().is_empty()
                || connection.credential_revision >= (1u64 << 53) - 1
                || connection.name.len() > 200
                || !matches!(
                    connection.provider.as_str(),
                    "openai"
                        | "anthropic"
                        | "google"
                        | "xai"
                        | "openrouter"
                        | "deepseek"
                        | "nvidia"
                        | "custom"
                )
                || !matches!(connection.secret_mode.as_str(), "system" | "session")
                || connection
                    .secret_id
                    .as_deref()
                    .is_some_and(|id| !valid_id(id))
                || connection.models.len() > 1000
                || connection
                    .models
                    .iter()
                    .any(|id| id.len() > 200 || id.is_empty())
            {
                return Err(
                    "Invalid Chat AI connection metadata. The original file was preserved.".into(),
                );
            }
            if connection.provider == "custom" {
                if !connection
                    .base_url
                    .as_deref()
                    .is_some_and(valid_custom_base_url)
                    || !matches!(
                        connection.api_format.as_deref(),
                        Some("chat-completions" | "responses" | "anthropic-messages")
                    )
                {
                    return Err("Custom API requires a valid HTTPS base URL (HTTP is allowed on localhost) and API format.".into());
                }
            } else if connection.base_url.is_some() || connection.api_format.is_some() {
                return Err("Preset providers use their fixed API endpoint.".into());
            }
        }
        if self
            .defaults
            .connection_id
            .as_ref()
            .is_some_and(|id| !ids.contains(id))
        {
            return Err("The default AI connection is unavailable.".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    version: u32,
    entries: Vec<String>,
}

pub trait Secrets {
    fn put(&self, id: &str, key: &str) -> Result<(), String>;
    fn get(&self, id: &str) -> Result<String, String>;
    fn remove(&self, id: &str) -> Result<(), String>;
}
pub struct SystemSecrets;
impl Secrets for SystemSecrets {
    fn put(&self, id: &str, key: &str) -> Result<(), String> {
        credentials::put(id, key)
    }
    fn get(&self, id: &str) -> Result<String, String> {
        credentials::get(id)
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        credentials::remove(id)
    }
}

pub struct Settings<S: Secrets = SystemSecrets> {
    path: PathBuf,
    journal: PathBuf,
    pub data: Preferences,
    session: HashMap<String, String>,
    secrets: S,
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    storage::reject_link(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err("Cannot read Chat AI preferences. The original file was preserved.".into())
        }
    };
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read Chat AI preferences.")?;
    if bytes.len() > 1024 * 1024 {
        return Err("Chat AI preferences exceed their size limit.".into());
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|_| {
        "Chat AI preferences are corrupt or unsupported. The original file was preserved.".into()
    })
}
impl<S: Secrets> Settings<S> {
    pub fn open(path: PathBuf, root: &Path, secrets: S) -> Result<Self, String> {
        let data = read::<Preferences>(&path)?.unwrap_or_default();
        data.validate()?;
        let value = Self {
            path,
            journal: root.join("secret-journal.json"),
            data,
            session: HashMap::new(),
            secrets,
        };
        value.recover()?;
        Ok(value)
    }
    fn journal(&self) -> Result<Journal, String> {
        let value = read::<Journal>(&self.journal)?.unwrap_or(Journal {
            version: 1,
            entries: Vec::new(),
        });
        if value.version != 1
            || value.entries.len() > 256
            || value.entries.iter().any(|id| !valid_id(id))
        {
            return Err("The secret cleanup journal is unsupported. It was preserved.".into());
        }
        Ok(value)
    }
    fn write_journal(&self, journal: &Journal) -> Result<(), String> {
        if journal.entries.len() > 256 {
            return Err("System key cleanup is pending. Unlock the key store before changing more connections.".into());
        }
        storage::atomic(&self.journal, &serde_json::to_vec(journal).unwrap())
    }
    fn recover(&self) -> Result<(), String> {
        let active: HashSet<&str> = self
            .data
            .connections
            .iter()
            .filter(|c| c.secret_mode == "system")
            .filter_map(|c| c.secret_id.as_deref())
            .collect();
        let mut journal = self.journal()?;

        journal.entries.retain(|id| {
            if active.contains(id.as_str()) {
                return false;
            }
            self.secrets.remove(id).is_err()
        });
        self.write_journal(&journal)?;
        // Failed removals remain journaled; a locked key store must not disable
        // local history or an explicitly selected session-only connection.
        Ok(())
    }
    pub fn key(&self, id: &str) -> Result<String, String> {
        let connection = self
            .data
            .connections
            .iter()
            .find(|c| c.id == id && c.enabled)
            .ok_or("Choose an enabled AI connection in Settings.")?;
        if connection.provider == "custom" && connection.secret_id.is_none() {
            return Ok(String::new());
        }
        let secret_id = connection
            .secret_id
            .as_deref()
            .ok_or("Configure an API key in Settings → Chat AI.")?;
        if connection.secret_mode == "session" {
            self.session
                .get(secret_id)
                .cloned()
                .ok_or("This session-only key has expired. Enter it again in Settings.".into())
        } else {
            self.secrets.get(secret_id)
        }
    }
    pub fn clear_key(&mut self, id: &str, expected: u64) -> Result<Preferences, String> {
        if self.data.revision != expected {
            return Err("conflict: Chat AI settings changed.".into());
        }
        self.recover()?;
        let mut desired = self.data.clone();
        let connection = desired
            .connections
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or("Connection unavailable.")?;
        let previous = connection.secret_id.take();
        if connection.secret_mode == "system" {
            if let Some(id) = &previous {
                let mut journal = self.journal()?;
                journal.entries.push(id.clone());
                self.write_journal(&journal)?;
            }
        }
        connection.credential_revision += 1;
        connection.tested_model = None;
        connection.test_status = None;
        desired.revision += 1;
        desired.validate()?;
        let result = storage::atomic(&self.path, &serde_json::to_vec_pretty(&desired).unwrap());
        self.data = read::<Preferences>(&self.path)?.unwrap_or_else(|| self.data.clone());
        if self.data.revision == desired.revision {
            if let Some(id) = previous {
                self.session.remove(&id);
            }
        }
        result?;
        self.recover()?;
        Ok(self.data.clone())
    }
    pub fn save(
        &mut self,
        mut desired: Preferences,
        expected: u64,
        new_key: Option<(&str, &str)>,
    ) -> Result<Preferences, String> {
        if expected != self.data.revision || desired.revision != expected {
            return Err("conflict: Chat AI settings changed in another window.".into());
        }
        desired.validate()?;
        self.recover()?;
        let mut journal = self.journal()?;
        if journal.entries.len() > 192 {
            return Err("System key cleanup is pending. Unlock the key store before changing more connections.".into());
        }
        let mut new_session: Option<(String, String)> = None;
        for connection in &mut desired.connections {
            let previous = self.data.connections.iter().find(|c| c.id == connection.id);
            connection.credential_revision = previous.map_or(0, |c| c.credential_revision);
            connection.secret_id = previous.and_then(|c| c.secret_id.clone());
            connection.tested_model = previous.and_then(|c| c.tested_model.clone());
            connection.test_status = previous.and_then(|c| c.test_status.clone());
            if let Some((id, key)) = new_key.filter(|(id, _)| *id == connection.id) {
                if key.is_empty() || key.len() > 8192 || key.contains(['\r', '\n', '\0']) {
                    return Err("Enter a valid API key.".into());
                }
                connection.credential_revision += 1;
                let secret_id = format!("{id}-{}", connection.credential_revision);
                if !valid_id(&secret_id) {
                    return Err("Connection ID is too long.".into());
                }
                // Persist identities before touching Keychain. Recovery follows
                // committed metadata even when a directory flush reported failure.
                journal.entries.push(secret_id.clone());
                if let Some(old) = previous
                    .filter(|c| c.secret_mode == "system")
                    .and_then(|c| c.secret_id.clone())
                {
                    journal.entries.push(old);
                }
                self.write_journal(&journal)?;
                if connection.secret_mode == "system" {
                    self.secrets.put(&secret_id, key)?;
                } else {
                    new_session = Some((secret_id.clone(), key.into()));
                }
                connection.secret_id = Some(secret_id);
                connection.tested_model = None;
                connection.test_status = None;
            } else if previous.is_some_and(|c| {
                c.secret_mode != connection.secret_mode || !c.same_destination(connection)
            }) {
                if connection.secret_id.is_some() {
                    return Err("Re-enter the API key when changing its destination, API format or storage mode.".into());
                }
                connection.credential_revision += 1;
                connection.tested_model = None;
                connection.test_status = None;
            }
        }
        for old in &self.data.connections {
            if old.secret_mode == "system"
                && !desired
                    .connections
                    .iter()
                    .any(|c| c.secret_id == old.secret_id)
            {
                if let Some(id) = &old.secret_id {
                    journal.entries.push(id.clone());
                }
            }
        }
        journal.entries.sort();
        journal.entries.dedup();
        self.write_journal(&journal)?;
        desired.revision += 1;
        let result = storage::atomic(&self.path, &serde_json::to_vec_pretty(&desired).unwrap());
        // A publish followed by a failed directory sync is not a rollback.
        self.data = read::<Preferences>(&self.path)?.unwrap_or_else(|| self.data.clone());
        if self.data.revision == desired.revision {
            if let Some((id, key)) = new_session {
                self.session.insert(id, key);
            }
            self.session.retain(|id, _| {
                self.data
                    .connections
                    .iter()
                    .any(|c| c.secret_mode == "session" && c.secret_id.as_ref() == Some(id))
            });
        }
        result?;
        self.recover()?;
        Ok(self.data.clone())
    }
}

impl<S: Secrets> Settings<S> {
    pub fn record_result(
        &mut self,
        id: &str,
        revision: u64,
        model: &str,
        operation: &str,
        result: &serde_json::Value,
    ) -> Result<(), String> {
        let mut desired = self.data.clone();
        let Some(connection) = desired
            .connections
            .iter_mut()
            .find(|c| c.id == id && c.credential_revision == revision)
        else {
            return Ok(());
        };
        if operation == "list-models" {
            if let Some(models) = result["models"].as_array() {
                // Keep manually added and previously available IDs when a
                // provider returns a partial or changed catalog.
                for model in models.iter().filter_map(|value| value.as_str()) {
                    if connection.models.len() >= 1000 {
                        break;
                    }
                    if !connection.models.iter().any(|existing| existing == model) {
                        connection.models.push(model.to_owned());
                    }
                }
            }
        } else {
            connection.tested_model = Some(model.into());
            connection.test_status = Some(result["status"].as_str().unwrap_or("failed").into());
        }
        desired.revision += 1;
        desired.validate()?;
        storage::atomic(&self.path, &serde_json::to_vec_pretty(&desired).unwrap())?;
        self.data = desired;
        Ok(())
    }
}
