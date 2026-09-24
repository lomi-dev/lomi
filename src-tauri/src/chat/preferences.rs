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
    pub enabled: bool,
    pub credential_revision: u64,
    pub secret_mode: String,
    pub secret_id: Option<String>,
    pub models: Vec<String>,
    pub tested_model: Option<String>,
    pub test_status: Option<String>,
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
                c.secret_mode != connection.secret_mode || c.provider != connection.provider
            }) {
                return Err("Enter a new key when changing its provider or storage mode.".into());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    #[derive(Clone, Default)]
    struct Memory(Rc<RefCell<HashMap<String, String>>>);
    impl Secrets for Memory {
        fn put(&self, id: &str, key: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(id.into(), key.into());
            Ok(())
        }
        fn get(&self, id: &str) -> Result<String, String> {
            self.0.borrow().get(id).cloned().ok_or("missing".into())
        }
        fn remove(&self, id: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(id);
            Ok(())
        }
    }
    fn connection() -> Connection {
        Connection {
            id: "connection".into(),
            name: "Test".into(),
            provider: "openai".into(),
            enabled: true,
            credential_revision: 0,
            secret_mode: "system".into(),
            secret_id: None,
            models: vec![],
            tested_model: None,
            test_status: None,
        }
    }
    #[test]
    fn supported_providers_round_trip_and_refresh_preserves_custom_models() {
        for provider in [
            "openai",
            "anthropic",
            "google",
            "xai",
            "openrouter",
            "deepseek",
            "nvidia",
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("preferences.json");
            let secrets = Memory::default();
            let mut settings = Settings::open(path.clone(), root.path(), secrets.clone()).unwrap();
            let mut desired = settings.data.clone();
            let mut item = connection();
            item.provider = provider.into();
            item.models = vec!["custom/model".into()];
            desired.connections.push(item);
            settings
                .save(desired, 0, Some(("connection", "private-key")))
                .unwrap();
            settings
                .record_result(
                    "connection",
                    1,
                    "",
                    "list-models",
                    &serde_json::json!({"models": ["new/model", "new/model"]}),
                )
                .unwrap();
            let restored = Settings::open(path, root.path(), secrets).unwrap();
            assert_eq!(restored.data.connections[0].provider, provider);
            assert_eq!(
                restored.data.connections[0].models,
                ["custom/model", "new/model"]
            );
            assert_eq!(restored.key("connection").unwrap(), "private-key");
            let mut invalid = restored.data.clone();
            invalid.connections[0].provider = "unsupported".into();
            assert!(invalid.validate().is_err());
        }
    }
    #[test]
    fn rotation_uses_committed_revision_and_session_keys_expire() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.json");
        let secrets = Memory::default();
        let mut settings = Settings::open(path.clone(), root.path(), secrets.clone()).unwrap();
        let mut desired = settings.data.clone();
        desired.connections.push(connection());
        settings
            .save(desired, 0, Some(("connection", "first-secret")))
            .unwrap();
        let old = settings.data.connections[0].secret_id.clone().unwrap();
        settings
            .save(
                settings.data.clone(),
                1,
                Some(("connection", "second-secret")),
            )
            .unwrap();
        assert!(!secrets.0.borrow().contains_key(&old));
        assert_eq!(settings.key("connection").unwrap(), "second-secret");
        let mut desired = settings.data.clone();
        desired.connections[0].secret_mode = "session".into();
        settings
            .save(desired, 2, Some(("connection", "temporary-secret")))
            .unwrap();
        assert!(secrets.0.borrow().is_empty());
        assert_eq!(settings.key("connection").unwrap(), "temporary-secret");
        for entry in fs::read_dir(root.path()).unwrap() {
            assert!(!fs::read_to_string(entry.unwrap().path())
                .unwrap()
                .contains("-secret"));
        }
        drop(settings);
        assert!(Settings::open(path, root.path(), secrets)
            .unwrap()
            .key("connection")
            .is_err());
    }
    #[test]
    fn rotation_recovers_each_durable_boundary_and_ignores_stale_test() {
        for committed in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("preferences.json");
            let secrets = Memory::default();
            let mut settings = Settings::open(path.clone(), root.path(), secrets.clone()).unwrap();
            let mut desired = settings.data.clone();
            desired.connections.push(connection());
            settings
                .save(desired, 0, Some(("connection", "first")))
                .unwrap();
            let original = settings.data.clone();
            let old = original.connections[0].secret_id.clone().unwrap();
            let new = "connection-2";
            storage::atomic(
                &settings.journal,
                &serde_json::to_vec(&Journal {
                    version: 1,
                    entries: vec![old.clone(), new.into()],
                })
                .unwrap(),
            )
            .unwrap();
            // Recovery after journaling but before writing a key preserves old.
            let recovered = Settings::open(path.clone(), root.path(), secrets.clone()).unwrap();
            assert_eq!(recovered.key("connection").unwrap(), "first");
            storage::atomic(
                &settings.journal,
                &serde_json::to_vec(&Journal {
                    version: 1,
                    entries: vec![old.clone(), new.into()],
                })
                .unwrap(),
            )
            .unwrap();
            secrets.put(new, "second").unwrap();
            if committed {
                let mut desired = original.clone();
                desired.revision += 1;
                desired.connections[0].secret_id = Some(new.into());
                desired.connections[0].credential_revision = 2;
                storage::atomic(&path, &serde_json::to_vec(&desired).unwrap()).unwrap();
            }
            drop(settings);
            let mut recovered = Settings::open(path, root.path(), secrets.clone()).unwrap();
            assert_eq!(
                recovered.key("connection").unwrap(),
                if committed { "second" } else { "first" }
            );
            assert_eq!(secrets.0.borrow().len(), 1);
            if committed {
                recovered
                    .record_result(
                        "connection",
                        1,
                        "old-model",
                        "test",
                        &serde_json::json!({"status":"completed"}),
                    )
                    .unwrap();
                assert!(recovered.data.connections[0].test_status.is_none());
            }
            let revision = recovered.data.revision;
            let mut desired = recovered.data.clone();
            desired.connections.clear();
            recovered.save(desired, revision, None).unwrap();
            assert!(secrets.0.borrow().is_empty());
        }
    }
    #[test]
    fn clearing_a_key_retains_the_connection_and_default_model() {
        let root = tempfile::tempdir().unwrap();
        let secrets = Memory::default();
        let mut settings = Settings::open(
            root.path().join("preferences.json"),
            root.path(),
            secrets.clone(),
        )
        .unwrap();
        let mut data = settings.data.clone();
        data.connections.push(connection());
        data.defaults.connection_id = Some("connection".into());
        data.defaults.model = "test-model".into();
        settings
            .save(data, 0, Some(("connection", "temporary")))
            .unwrap();
        assert!(settings.clear_key("connection", 0).is_err());
        settings.clear_key("connection", 1).unwrap();
        assert_eq!(settings.data.connections.len(), 1);
        assert!(settings.data.connections[0].secret_id.is_none());
        assert_eq!(settings.data.defaults.model, "test-model");
        assert!(settings.key("connection").is_err());
        assert!(secrets.0.borrow().is_empty());
    }
    #[test]
    fn orphan_secret_recovery_uses_journal_without_enumeration() {
        let root = tempfile::tempdir().unwrap();
        let secrets = Memory::default();
        secrets.put("orphan", "not-logged").unwrap();
        storage::atomic(
            &root.path().join("secret-journal.json"),
            br#"{"version":1,"entries":["orphan"]}"#,
        )
        .unwrap();
        Settings::open(root.path().join("prefs.json"), root.path(), secrets.clone()).unwrap();
        assert!(secrets.0.borrow().is_empty());
    }
}
