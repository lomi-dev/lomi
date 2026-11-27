//! Encrypted-at-rest memory of the Host directory. The sealed blob lives in
//! `app_data/remote-core/directory.bin`; its 256-bit key lives only in the OS keychain.
//! Wiping destroys the key first, so a copied blob is unreadable afterwards (logout,
//! revoke, account change). The core's AEAD binds the blob to the Remote account and
//! device. The local desktop user is not in the AEAD's associated data (the core has no
//! extra field), so it is stored with the key in the keychain item and compared with the
//! envelope on load: a swapped file cannot be attributed to another user.

use super::engine::Ids;
use lomi_remote_core::directory::Key;
use zeroize::Zeroizing;

/// The key id the directory is sealed with. Every new directory gets a new random key, so
/// the id never needs to rise: the old key is destroyed rather than outranked.
pub const KEY_ID: u32 = 1;
const MAX_HEADER_BYTES: usize = 1024;

#[derive(Debug)]
pub struct VaultError;

pub struct Stored {
    pub ids: Ids,
    pub key: Key,
    pub blob: Vec<u8>,
}

pub trait Vault: Send {
    /// The stored record, or `None` when there is none or it is unreadable (then it is wiped).
    fn load(&mut self) -> Result<Option<Stored>, VaultError>;
    /// The key for this directory; a new random one when the record belongs to other ids.
    fn key(&mut self, ids: &Ids) -> Result<Key, VaultError>;
    fn write(&mut self, ids: &Ids, blob: &[u8]) -> Result<(), VaultError>;
    /// Destroys the key, then the blob. An error means something may remain: retry later.
    fn wipe(&mut self) -> Result<(), VaultError>;
}

/// Where the 32-byte key is kept (hex); the OS keychain in production.
pub trait SecretStore: Send {
    fn get(&self) -> Result<Option<Zeroizing<String>>, VaultError>;
    fn set(&self, value: &str) -> Result<(), VaultError>;
    fn delete(&self) -> Result<(), VaultError>;
}

pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: String) -> Self {
        Self { service }
    }

    fn entry(&self) -> Result<keyring::Entry, VaultError> {
        crate::credential_store::entry(&self.service, "directory-key").map_err(|_| VaultError)
    }
}

impl SecretStore for KeyringStore {
    fn get(&self) -> Result<Option<Zeroizing<String>>, VaultError> {
        match crate::credential_store::get_password(&self.service, "directory-key", 256) {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(VaultError),
        }
    }

    fn set(&self, value: &str) -> Result<(), VaultError> {
        self.entry()?.set_password(value).map_err(|_| VaultError)
    }

    fn delete(&self) -> Result<(), VaultError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(VaultError),
        }
    }
}

pub struct FileVault<S: SecretStore> {
    path: std::path::PathBuf,
    secrets: S,
    key: Option<Zeroizing<[u8; 32]>>,
    key_ids: Option<Ids>,
}

impl<S: SecretStore> FileVault<S> {
    pub fn new(path: std::path::PathBuf, secrets: S) -> Self {
        Self {
            path,
            secrets,
            key: None,
            key_ids: None,
        }
    }

    fn random_key() -> Result<Zeroizing<[u8; 32]>, VaultError> {
        use ring::rand::SecureRandom;
        let mut key = Zeroizing::new([0u8; 32]);
        ring::rand::SystemRandom::new()
            .fill(&mut key[..])
            .map_err(|_| VaultError)?;
        Ok(key)
    }
}

fn hex(bytes: &[u8]) -> Zeroizing<String> {
    Zeroizing::new(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn secret(key: &[u8; 32], user: &str) -> Zeroizing<String> {
    let mut text = hex(key);
    text.push('\n');
    text.push_str(user);
    text
}

fn parse_secret(text: &str) -> Option<(Zeroizing<[u8; 32]>, String)> {
    let (key, user) = text.split_once('\n')?;
    Some((unhex(key)?, user.to_owned()))
}

fn unhex(text: &str) -> Option<Zeroizing<[u8; 32]>> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = Zeroizing::new([0u8; 32]);
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// `{"v":1,"account":..,"device":..,"user":..}\n` followed by the sealed blob.
pub fn encode_envelope(ids: &Ids, blob: &[u8]) -> Vec<u8> {
    let header = serde_json::json!({
        "v": 1, "account": ids.account, "device": ids.device, "user": ids.user,
    });
    let mut out = header.to_string().into_bytes();
    out.push(b'\n');
    out.extend_from_slice(blob);
    out
}

pub fn decode_envelope(bytes: &[u8]) -> Option<(Ids, &[u8])> {
    let end = bytes
        .iter()
        .take(MAX_HEADER_BYTES)
        .position(|b| *b == b'\n')?;
    let header: serde_json::Value = serde_json::from_slice(&bytes[..end]).ok()?;
    if header.get("v")? != 1 {
        return None;
    }
    let text = |k: &str| header.get(k)?.as_str().map(str::to_owned);
    let ids = Ids {
        account: text("account")?,
        device: text("device")?,
        user: text("user")?,
    };
    Some((ids, &bytes[end + 1..]))
}

impl<S: SecretStore> Vault for FileVault<S> {
    fn load(&mut self) -> Result<Option<Stored>, VaultError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(VaultError),
        };
        let secret = self.secrets.get()?;
        let parsed =
            decode_envelope(&bytes).zip(secret.as_ref().map(|t| t.as_str()).and_then(parse_secret));
        let Some(((ids, blob), (key, _))) =
            parsed.filter(|((ids, _), (_, user))| ids.user == *user)
        else {
            // No key, an unreadable envelope or another user's key: it can never be shown.
            self.wipe()?;
            return Ok(None);
        };
        let stored = Stored {
            ids: ids.clone(),
            key: Key::new(KEY_ID, *key),
            blob: blob.to_vec(),
        };
        self.key = Some(key);
        self.key_ids = Some(ids);
        Ok(Some(stored))
    }

    fn key(&mut self, ids: &Ids) -> Result<Key, VaultError> {
        if self.key_ids.as_ref() != Some(ids) || self.key.is_none() {
            let key = Self::random_key()?;
            // The key is stored before any blob it will seal exists.
            self.secrets.set(&secret(&key, &ids.user))?;
            self.key = Some(key);
            self.key_ids = Some(ids.clone());
        }
        let key = self.key.as_ref().ok_or(VaultError)?;
        Ok(Key::new(KEY_ID, **key))
    }

    fn write(&mut self, ids: &Ids, blob: &[u8]) -> Result<(), VaultError> {
        use crate::storage_guard::{atomic, private, reject_link};
        let parent = self.path.parent().ok_or(VaultError)?;
        reject_link(parent).map_err(|_| VaultError)?;
        std::fs::create_dir_all(parent).map_err(|_| VaultError)?;
        private(parent, true).map_err(|_| VaultError)?;
        // Private file, link rejection, fsync of file and directory.
        atomic(&self.path, &encode_envelope(ids, blob)).map_err(|_| VaultError)
    }

    fn wipe(&mut self) -> Result<(), VaultError> {
        self.key = None;
        self.key_ids = None;
        let key = self.secrets.delete();
        let file = std::fs::remove_file(&self.path);
        let gone = |r: std::io::Result<()>| {
            r.is_ok() || matches!(r, Err(ref e) if e.kind() == std::io::ErrorKind::NotFound)
        };
        if key.is_ok() && gone(file) {
            Ok(())
        } else {
            Err(VaultError)
        }
    }
}

/// Opens the stored directory for `user`; anything unreadable or owned by another user is wiped.
pub fn open_for(
    vault: &mut dyn Vault,
    user: &str,
) -> Result<Option<(Ids, lomi_remote_core::directory::Directory)>, VaultError> {
    use lomi_remote_core::directory::Directory;
    let Some(Stored { ids, key, blob }) = vault.load()? else {
        return Ok(None);
    };
    let opened = (ids.user == user)
        .then(|| Directory::open(&blob, &ids.account, &ids.device, &[key], KEY_ID).ok())
        .flatten();
    match opened {
        Some(dir) => Ok(Some((ids, dir))),
        None => vault.wipe().map(|()| None),
    }
}

/// Seals `dir` with the key of `ids` (created on first use) and stores it.
pub fn persist(
    vault: &mut dyn Vault,
    ids: &Ids,
    dir: &lomi_remote_core::directory::Directory,
) -> Result<(), VaultError> {
    use ring::rand::SecureRandom;
    let key = vault.key(ids)?;
    let mut nonce = [0u8; lomi_remote_core::directory::NONCE_LEN];
    ring::rand::SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| VaultError)?;
    let blob = dir.seal(&key, nonce).map_err(|_| VaultError)?;
    vault.write(ids, &blob)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub struct MemSecrets {
        pub value: Arc<Mutex<Option<String>>>,
        pub fail_delete: Arc<Mutex<bool>>,
    }

    impl SecretStore for MemSecrets {
        fn get(&self) -> Result<Option<Zeroizing<String>>, VaultError> {
            Ok(self.value.lock().unwrap().clone().map(Zeroizing::new))
        }
        fn set(&self, value: &str) -> Result<(), VaultError> {
            *self.value.lock().unwrap() = Some(value.to_owned());
            Ok(())
        }
        fn delete(&self) -> Result<(), VaultError> {
            if *self.fail_delete.lock().unwrap() {
                return Err(VaultError);
            }
            *self.value.lock().unwrap() = None;
            Ok(())
        }
    }

    fn ids(user: &str) -> Ids {
        Ids {
            account: "acc".into(),
            device: "dev".into(),
            user: user.into(),
        }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lomi-remote-core-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("directory.bin")
    }

    #[test]
    fn envelope_round_trips_and_rejects_garbage() {
        let bytes = encode_envelope(&ids("u"), b"\x00blob\n\xff");
        let (got, blob) = decode_envelope(&bytes).unwrap();
        assert_eq!((got, blob), (ids("u"), &b"\x00blob\n\xff"[..]));
        assert!(decode_envelope(b"no newline").is_none());
        assert!(decode_envelope(b"{\"v\":2}\nx").is_none());
        assert!(decode_envelope(b"{\"v\":1,\"account\":\"a\"}\nx").is_none());
    }

    #[test]
    fn saved_directory_loads_with_its_key_and_wipe_destroys_both() {
        let secrets = MemSecrets::default();
        let path = temp("roundtrip");
        let mut vault = FileVault::new(path.clone(), secrets.clone());
        assert!(vault.load().unwrap().is_none());
        let key = vault.key(&ids("u")).unwrap();
        vault.write(&ids("u"), b"sealed").unwrap();

        let mut again = FileVault::new(path.clone(), secrets.clone());
        let stored = again.load().unwrap().unwrap();
        assert_eq!((stored.ids, stored.blob), (ids("u"), b"sealed".to_vec()));
        assert_eq!(stored.key.id, key.id);

        again.wipe().unwrap();
        assert!(secrets.value.lock().unwrap().is_none());
        assert!(!path.exists());
        assert!(again.load().unwrap().is_none());
    }

    #[test]
    fn another_directory_gets_a_new_key() {
        let secrets = MemSecrets::default();
        let mut vault = FileVault::new(temp("rekey"), secrets.clone());
        vault.key(&ids("u1")).unwrap();
        let first = secrets.value.lock().unwrap().clone();
        vault.key(&ids("u1")).unwrap();
        assert_eq!(
            *secrets.value.lock().unwrap(),
            first,
            "same ids keep the key"
        );
        vault.key(&ids("u2")).unwrap();
        assert_ne!(*secrets.value.lock().unwrap(), first);
    }

    #[test]
    fn blob_without_its_key_is_discarded_and_failed_key_deletion_is_reported() {
        let secrets = MemSecrets::default();
        let path = temp("nokey");
        let mut vault = FileVault::new(path.clone(), secrets.clone());
        vault.key(&ids("u")).unwrap();
        vault.write(&ids("u"), b"x").unwrap();
        *secrets.value.lock().unwrap() = None;
        assert!(FileVault::new(path.clone(), secrets.clone())
            .load()
            .unwrap()
            .is_none());
        assert!(!path.exists());

        vault.key(&ids("u")).unwrap();
        vault.write(&ids("u"), b"x").unwrap();
        *secrets.fail_delete.lock().unwrap() = true;
        assert!(vault.wipe().is_err(), "the caller must retry");
        assert!(
            !path.exists(),
            "the blob is removed even when the keychain refuses"
        );
    }

    #[test]
    fn persisted_directory_opens_only_for_its_user_and_is_wiped_for_others() {
        use lomi_remote_core::directory::Directory;
        let secrets = MemSecrets::default();
        let path = temp("open-for");
        let mut vault = FileVault::new(path.clone(), secrets.clone());
        persist(&mut vault, &ids("u1"), &Directory::new("acc", "dev")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let mut fresh = FileVault::new(path.clone(), secrets.clone());
        let (got, _) = open_for(&mut fresh, "u1").unwrap().unwrap();
        assert_eq!(got, ids("u1"));

        let mut other = FileVault::new(path.clone(), secrets.clone());
        assert!(open_for(&mut other, "u2").unwrap().is_none());
        assert!(!path.exists() && secrets.value.lock().unwrap().is_none());
    }

    #[test]
    fn a_swapped_envelope_user_does_not_match_the_keychain_user() {
        let secrets = MemSecrets::default();
        let path = temp("swap");
        let mut vault = FileVault::new(path.clone(), secrets.clone());
        vault.key(&ids("u1")).unwrap();
        vault.write(&ids("u1"), b"x").unwrap();
        // Same file claiming another user, key still bound to u1 in the keychain.
        std::fs::write(&path, encode_envelope(&ids("u2"), b"x")).unwrap();
        assert!(FileVault::new(path.clone(), secrets)
            .load()
            .unwrap()
            .is_none());
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_is_refused() {
        let secrets = MemSecrets::default();
        let path = temp("link");
        let real = path.parent().unwrap().with_extension("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, path.parent().unwrap()).unwrap();
        let mut vault = FileVault::new(path, secrets);
        vault.key(&ids("u")).unwrap();
        assert!(vault.write(&ids("u"), b"x").is_err());
    }
}
