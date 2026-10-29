use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

const SERVICE_PREFIX: &str = "dev.lomi.desktop.account";
const METADATA_FILE: &str = "credentials.json";
const METADATA_LIMIT: u64 = 64 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    version: u8,
    active_id: Option<String>,
    journal_ids: Vec<String>,
    logout_tombstone: bool,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            version: 1,
            active_id: None,
            journal_ids: Vec::new(),
            logout_tombstone: false,
        }
    }
}

impl Metadata {
    fn stage(&mut self, id: &str) {
        if !self.journal_ids.iter().any(|existing| existing == id) {
            self.journal_ids.push(id.to_string());
        }
    }

    fn reusable_id(&self) -> Option<&str> {
        self.journal_ids
            .iter()
            .map(String::as_str)
            .find(|id| Some(*id) != self.active_id.as_deref())
    }

    fn commit(&mut self, id: &str) {
        if let Some(old) = self.active_id.replace(id.to_string()) {
            if old != id {
                self.stage(&old);
            }
        }
        self.journal_ids.retain(|existing| existing != id);
        self.logout_tombstone = false;
    }

    fn tombstone(&mut self) {
        self.logout_tombstone = true;
        if let Some(active) = self.active_id.clone() {
            self.stage(&active);
        }
    }

    fn finish_removal(&mut self, id: &str) {
        self.journal_ids.retain(|existing| existing != id);
        if self.logout_tombstone && self.active_id.as_deref() == Some(id) {
            self.active_id = None;
        }
    }

    fn restorable_id(&self) -> Option<&str> {
        if self.logout_tombstone {
            None
        } else {
            self.active_id.as_deref()
        }
    }

    fn cleanup_ids(&self) -> Vec<String> {
        let mut ids = self.journal_ids.clone();
        if self.logout_tombstone {
            if let Some(active) = &self.active_id {
                if !ids.iter().any(|id| id == active) {
                    ids.push(active.clone());
                }
            }
        }
        ids
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.journal_ids.len() > 128
            || self
                .active_id
                .as_deref()
                .is_some_and(|id| !valid_credential_id(id))
            || self.journal_ids.iter().any(|id| !valid_credential_id(id))
        {
            return Err("Account credential metadata is invalid and was preserved.".into());
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CredentialSaveError {
    Metadata,
    KeyStore,
}

pub(super) struct CredentialStore {
    service: String,
    metadata_path: PathBuf,
    metadata: Metadata,
    _owner: File,
}

impl Drop for CredentialStore {
    fn drop(&mut self) {
        let _ = self._owner.unlock();
    }
}

impl CredentialStore {
    pub fn open(root: &Path, environment: &str) -> Result<Self, String> {
        crate::chat::storage::reject_link(root)?;
        fs::create_dir_all(root).map_err(|_| "Cannot create account storage.")?;
        crate::chat::storage::private(root, true)?;
        let root = fs::canonicalize(root).map_err(|_| "Cannot resolve account storage.")?;
        let owner_path = root.join("owner.lock");
        crate::chat::storage::reject_link(&owner_path)?;
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&owner_path)
            .map_err(|_| "Cannot open account storage ownership lock.")?;
        crate::chat::storage::private(&owner_path, false)?;
        owner
            .try_lock()
            .map_err(|_| "Another Lomi process owns account storage.")?;

        let metadata_path = root.join(METADATA_FILE);
        crate::chat::storage::reject_link(&metadata_path)?;
        let metadata = read_metadata(&metadata_path)?;
        let service_environment = if environment.is_empty()
            || environment.len() > 128
            || !environment
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err("The account storage environment is invalid.".into());
        } else {
            environment
        };
        Ok(Self {
            service: format!("{SERVICE_PREFIX}.{service_environment}"),
            metadata_path,
            metadata,
            _owner: owner,
        })
    }

    pub fn load_active(&mut self) -> Result<Option<String>, String> {
        self.cleanup_journal();
        let Some(id) = self.metadata.restorable_id().map(str::to_string) else {
            return Ok(None);
        };
        match self.entry(&id)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => {
                let previous = self.metadata.clone();
                self.metadata.active_id = None;
                if let Err(error) = self.persist() {
                    self.metadata = previous;
                    return Err(error);
                }
                Ok(None)
            }
            Err(_) => Err("The account session is unavailable in the system key store.".into()),
        }
    }

    pub fn save_active(&mut self, secret: &str) -> Result<bool, CredentialSaveError> {
        // Reuse an orphan to bound the journal when the key store stays unavailable.
        // Never overwrite the committed credential, including after a tombstone.
        let id = match self.metadata.reusable_id() {
            Some(id) => id.to_string(),
            None => new_credential_id().map_err(|_| CredentialSaveError::Metadata)?,
        };
        let previous = self.metadata.clone();
        self.metadata.stage(&id);
        if self.persist().is_err() {
            self.metadata = previous;
            return Err(CredentialSaveError::Metadata);
        }
        self.entry(&id)
            .map_err(|_| CredentialSaveError::KeyStore)?
            .set_password(secret)
            .map_err(|_| CredentialSaveError::KeyStore)?;

        let previous = self.metadata.clone();
        self.metadata.commit(&id);
        if self.persist().is_err() {
            self.metadata = previous;
            return Err(CredentialSaveError::Metadata);
        }
        Ok(self.cleanup_journal())
    }

    pub fn tombstone(&mut self) -> Result<(), String> {
        let previous = self.metadata.clone();
        self.metadata.tombstone();
        if let Err(error) = self.persist() {
            self.metadata = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn cleanup_journal(&mut self) -> bool {
        let ids = self.metadata.cleanup_ids();
        let mut all_removed = true;
        for id in ids {
            let removed = match self.entry(&id) {
                Ok(entry) => match entry.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => true,
                    Err(_) => false,
                },
                Err(_) => false,
            };
            if removed {
                let previous = self.metadata.clone();
                self.metadata.finish_removal(&id);
                if self.persist().is_err() {
                    self.metadata = previous;
                    all_removed = false;
                }
            } else {
                all_removed = false;
            }
        }
        all_removed
    }

    pub fn has_active_reference(&self) -> bool {
        self.metadata.active_id.is_some() && !self.metadata.logout_tombstone
    }

    fn entry(&self, id: &str) -> Result<keyring::Entry, String> {
        if !valid_credential_id(id) {
            return Err("Invalid account credential identifier.".into());
        }
        crate::credential_store::entry(&self.service, id)
            .map_err(|_| "The system key store is unavailable.".to_string())
    }

    fn persist(&mut self) -> Result<(), String> {
        self.metadata.validate()?;
        let bytes = serde_json::to_vec(&self.metadata)
            .map_err(|_| "Cannot serialize account credential metadata.")?;
        crate::chat::storage::atomic(&self.metadata_path, &bytes)
    }
}

fn read_metadata(path: &Path) -> Result<Metadata, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Metadata::default())
        }
        Err(_) => return Err("Cannot read account credential metadata.".into()),
    };
    let length = file
        .metadata()
        .map_err(|_| "Cannot inspect account credential metadata.")?
        .len();
    if length > METADATA_LIMIT {
        return Err("Account credential metadata is too large and was preserved.".into());
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(METADATA_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read account credential metadata.")?;
    let metadata: Metadata = serde_json::from_slice(&bytes)
        .map_err(|_| "Account credential metadata is invalid and was preserved.")?;
    metadata.validate()?;
    Ok(metadata)
}

fn valid_credential_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn new_credential_id() -> Result<String, String> {
    let mut bytes = [0_u8; 24];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| "Cannot create an account credential identifier.")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
