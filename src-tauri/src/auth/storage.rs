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
    #[cfg(test)]
    persist_count: usize,
    #[cfg(test)]
    fail_persist_on: Option<usize>,
    #[cfg(test)]
    fail_key_store_access: bool,
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
            #[cfg(test)]
            persist_count: 0,
            #[cfg(test)]
            fail_persist_on: None,
            #[cfg(test)]
            fail_key_store_access: false,
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

    #[cfg(test)]
    pub(super) fn deny_key_store_access(&mut self) {
        self.fail_key_store_access = true;
    }

    #[cfg(test)]
    pub(super) fn fail_next_metadata_write(&mut self) {
        self.fail_metadata_write_after(0);
    }

    #[cfg(test)]
    pub(super) fn fail_metadata_write_after(&mut self, successful_writes: usize) {
        self.fail_persist_on = Some(self.persist_count + successful_writes + 1);
    }

    fn entry(&self, id: &str) -> Result<keyring::Entry, String> {
        if !valid_credential_id(id) {
            return Err("Invalid account credential identifier.".into());
        }
        #[cfg(test)]
        if self.fail_key_store_access {
            return Err("Injected system key store access denial.".into());
        }
        crate::credential_store::entry(&self.service, id)
            .map_err(|_| "The system key store is unavailable.".to_string())
    }

    fn persist(&mut self) -> Result<(), String> {
        #[cfg(test)]
        {
            self.persist_count += 1;
            if self.fail_persist_on == Some(self.persist_count) {
                return Err("Injected account metadata write failure.".into());
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_journals_before_reference_and_commits_atomically() {
        let mut metadata = Metadata::default();
        metadata.stage("credential-1");
        assert_eq!(metadata.restorable_id(), None);
        assert_eq!(metadata.cleanup_ids(), vec!["credential-1"]);

        metadata.commit("credential-1");
        assert_eq!(metadata.restorable_id(), Some("credential-1"));
        assert!(metadata.cleanup_ids().is_empty());

        metadata.stage("credential-2");
        metadata.commit("credential-2");
        assert_eq!(metadata.restorable_id(), Some("credential-2"));
        assert_eq!(metadata.cleanup_ids(), vec!["credential-1"]);
    }

    #[test]
    fn tombstone_prevents_restore_before_key_cleanup() {
        let mut metadata = Metadata::default();
        metadata.stage("credential-1");
        metadata.commit("credential-1");
        metadata.tombstone();

        assert_eq!(metadata.restorable_id(), None);
        assert_eq!(metadata.cleanup_ids(), vec!["credential-1"]);
        metadata.finish_removal("credential-1");
        assert_eq!(metadata.active_id, None);
        assert!(metadata.logout_tombstone);
    }

    #[test]
    fn reusable_journal_ids_always_exclude_the_active_credential() {
        let mut metadata = Metadata::default();
        assert_eq!(metadata.reusable_id(), None);
        metadata.commit("active-credential");
        metadata.tombstone();
        assert_eq!(metadata.reusable_id(), None);
        metadata.stage("orphan-credential");
        assert_eq!(metadata.reusable_id(), Some("orphan-credential"));
        metadata.logout_tombstone = false;
        assert_eq!(metadata.reusable_id(), Some("orphan-credential"));
        let id = metadata.reusable_id().unwrap().to_string();
        metadata.commit(&id);
        assert_eq!(metadata.restorable_id(), Some("orphan-credential"));
        assert_eq!(metadata.cleanup_ids(), vec!["active-credential"]);
        assert!(!metadata.logout_tombstone);
    }

    #[test]
    fn full_journal_can_reuse_an_orphan_without_overwriting_the_active_reference() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = CredentialStore::open(temp.path(), "test-full-journal").unwrap();
        store.metadata.commit("active-credential");
        store.metadata.tombstone();
        for index in 0..127 {
            store.metadata.stage(&format!("orphan-{index}"));
        }
        store.persist().unwrap();
        store.deny_key_store_access();
        assert_eq!(
            store.save_active("memory-only-secret"),
            Err(CredentialSaveError::KeyStore)
        );
        store.tombstone().unwrap();
        assert_eq!(store.metadata.journal_ids.len(), 128);
        assert_eq!(
            store.metadata.active_id.as_deref(),
            Some("active-credential")
        );
        assert_eq!(store.metadata.reusable_id(), Some("orphan-0"));
        assert!(store.load_active().unwrap().is_none());
    }

    #[test]
    fn repeated_key_store_failures_keep_the_journal_bounded_across_reopens() {
        for has_active in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            if has_active {
                let mut store = CredentialStore::open(temp.path(), "test-bounded-journal").unwrap();
                store.metadata.commit("active-credential");
                store.persist().unwrap();
            }
            let mut orphan_id = None;
            for _ in 0..160 {
                let mut store = CredentialStore::open(temp.path(), "test-bounded-journal").unwrap();
                store.deny_key_store_access();
                assert_eq!(
                    store.save_active("memory-only-secret"),
                    Err(CredentialSaveError::KeyStore)
                );
                let id = store.metadata.reusable_id().unwrap().to_string();
                assert_ne!(id, "active-credential");
                if let Some(previous) = &orphan_id {
                    assert_eq!(&id, previous);
                } else {
                    orphan_id = Some(id);
                }
                store.tombstone().unwrap();
                assert_eq!(
                    store.metadata.active_id.as_deref(),
                    has_active.then_some("active-credential")
                );
                assert_eq!(
                    store.metadata.journal_ids.len(),
                    if has_active { 2 } else { 1 }
                );
                assert!(store.load_active().unwrap().is_none());
                assert!(!fs::read_to_string(&store.metadata_path)
                    .unwrap()
                    .contains("memory-only-secret"));
            }
        }
    }

    #[test]
    fn corrupt_or_unbounded_journals_are_rejected() {
        let metadata = Metadata {
            version: 2,
            ..Metadata::default()
        };
        assert!(metadata.validate().is_err());
        assert!(!valid_credential_id("../token"));
        assert!(valid_credential_id("gTz_123-abc"));
    }

    #[test]
    fn fresh_storage_does_not_access_an_unavailable_key_store() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = CredentialStore::open(temp.path(), "test-fresh-storage").unwrap();
        store.fail_key_store_access = true;

        assert_eq!(store.load_active().unwrap(), None);
        assert!(!store.has_active_reference());
        assert!(store.metadata.active_id.is_none());
        assert!(store.metadata.journal_ids.is_empty());
        assert!(!store.metadata_path.exists());
    }

    #[test]
    fn distinct_storage_namespaces_open_concurrently_with_distinct_keyring_services() {
        let temp = tempfile::tempdir().unwrap();
        let release_namespace = "auth-lomi-dev";
        let debug_namespace = "auth-lomi-dev-debug";
        let release =
            CredentialStore::open(&temp.path().join(release_namespace), release_namespace).unwrap();
        let debug =
            CredentialStore::open(&temp.path().join(debug_namespace), debug_namespace).unwrap();

        assert_ne!(release.metadata_path.parent(), debug.metadata_path.parent());
        assert_eq!(
            release.service,
            format!("{SERVICE_PREFIX}.{release_namespace}")
        );
        assert_eq!(debug.service, format!("{SERVICE_PREFIX}.{debug_namespace}"));
        assert_ne!(release.service, debug.service);
    }

    #[test]
    fn same_storage_directory_still_blocks_another_owner() {
        let temp = tempfile::tempdir().unwrap();
        let owner = CredentialStore::open(temp.path(), "test-owner").unwrap();

        assert_eq!(
            CredentialStore::open(temp.path(), "test-another-namespace").err(),
            Some("Another Lomi process owns account storage.".into())
        );
        drop(owner);
        assert!(CredentialStore::open(temp.path(), "test-owner").is_ok());
    }

    #[test]
    fn dropping_owner_releases_the_lock_with_a_cloned_handle_alive() {
        let temp = tempfile::tempdir().unwrap();
        let owner = CredentialStore::open(temp.path(), "test-inherited-owner").unwrap();
        let inherited = owner._owner.try_clone().unwrap();

        drop(owner);

        let reopened = CredentialStore::open(temp.path(), "test-reopened-owner");
        assert!(reopened.is_ok());
        drop(inherited);
    }

    #[test]
    fn opening_corrupt_metadata_preserves_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(METADATA_FILE);
        let corrupt = b"{invalid account metadata";
        fs::write(&path, corrupt).unwrap();

        assert_eq!(
            CredentialStore::open(temp.path(), "test-corrupt-metadata").err(),
            Some("Account credential metadata is invalid and was preserved.".into())
        );
        assert_eq!(fs::read(&path).unwrap(), corrupt);
    }

    #[test]
    fn denied_read_preserves_the_active_reference() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = CredentialStore::open(temp.path(), "test-denied-read").unwrap();
        store.metadata.commit("credential-1");
        store.persist().unwrap();
        let before = fs::read(&store.metadata_path).unwrap();
        store.fail_key_store_access = true;

        assert!(store.load_active().is_err());
        assert!(store.has_active_reference());
        assert_eq!(store.metadata.restorable_id(), Some("credential-1"));
        assert_eq!(fs::read(&store.metadata_path).unwrap(), before);
    }

    #[test]
    fn denied_deletion_preserves_the_tombstone_and_cleanup_journal() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = CredentialStore::open(temp.path(), "test-denied-deletion").unwrap();
        store.metadata.commit("credential-1");
        store.tombstone().unwrap();
        let before = fs::read(&store.metadata_path).unwrap();
        store.fail_key_store_access = true;

        assert!(!store.cleanup_journal());
        assert_eq!(store.load_active().unwrap(), None);
        assert!(!store.has_active_reference());
        assert_eq!(store.metadata.active_id.as_deref(), Some("credential-1"));
        assert_eq!(store.metadata.cleanup_ids(), vec!["credential-1"]);
        assert!(store.metadata.logout_tombstone);
        assert_eq!(fs::read(&store.metadata_path).unwrap(), before);
    }

    #[test]
    fn key_store_save_failure_is_distinct_from_metadata_failure() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = CredentialStore::open(temp.path(), "test-save-error").unwrap();
        store.deny_key_store_access();
        store.fail_next_metadata_write();
        assert_eq!(
            store.save_active("memory-only-secret"),
            Err(CredentialSaveError::Metadata)
        );
        assert!(store.metadata.journal_ids.is_empty());
        assert_eq!(
            store.save_active("memory-only-secret"),
            Err(CredentialSaveError::KeyStore)
        );
        assert_eq!(store.metadata.journal_ids.len(), 1);
        store.tombstone().unwrap();
        assert!(store.load_active().unwrap().is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches only a randomized, isolated Lomi test entry in macOS Keychain"]
    fn isolated_macos_keyring_round_trips_and_deletes() {
        let environment = unique_test_environment();
        let service = format!("{SERVICE_PREFIX}.{environment}");
        let id = new_credential_id().unwrap();
        let entry = crate::credential_store::entry(&service, &id).unwrap();
        let secret = format!("isolated-test-{}", new_credential_id().unwrap());

        entry.set_password(&secret).unwrap();
        assert_eq!(entry.get_password().unwrap(), secret);
        entry.delete_credential().unwrap();
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "uses a randomized, isolated Lomi test namespace in macOS Keychain"]
    fn commit_write_failure_leaves_a_journaled_orphan_for_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let environment = unique_test_environment();
        let service = format!("{SERVICE_PREFIX}.{environment}");
        let mut store = CredentialStore::open(&temp.path().join("account"), &environment).unwrap();
        store.fail_persist_on = Some(2);

        assert_eq!(
            store.save_active("isolated-test-session"),
            Err(CredentialSaveError::Metadata)
        );
        assert_eq!(store.metadata.restorable_id(), None);
        assert_eq!(store.metadata.journal_ids.len(), 1);
        let orphan_id = store.metadata.journal_ids[0].clone();
        let orphan = crate::credential_store::entry(&service, &orphan_id).unwrap();
        assert_eq!(orphan.get_password().unwrap(), "isolated-test-session");

        store.fail_persist_on = None;
        assert!(store.cleanup_journal());
        assert!(matches!(
            orphan.get_password(),
            Err(keyring::Error::NoEntry)
        ));
        assert_eq!(store.metadata.restorable_id(), None);
    }

    #[cfg(target_os = "macos")]
    fn unique_test_environment() -> String {
        new_credential_id()
            .unwrap()
            .to_ascii_lowercase()
            .replace('_', "-")
    }
}
