//! The installed client pins this registration key; mutable endpoints carry its signature.
use crate::broker::Endpoint;
use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

const KEY: &str = "registration-key.pk8";
pub const FILE: &str = "discovery.json";
const LIMIT: u64 = 16 * 1024;
const DOMAIN: &[u8] = b"lomi-mcp-discovery-v1\0";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedEndpoint {
    version: u8,
    payload: String,
    signature: String,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Cannot authenticate the Lomi registration",
    )
}

fn private(path: &Path, directory: bool) -> io::Result<()> {
    let meta = path.symlink_metadata()?;
    if meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != if directory { 0o700 } else { 0o600 }
        || if directory {
            !meta.is_dir()
        } else {
            !meta.is_file() || meta.nlink() != 1
        }
    {
        return Err(invalid());
    }
    Ok(())
}

fn bounded_read(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    private(path, false)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid());
    }
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn unhex(value: &str, length: usize) -> io::Result<Vec<u8>> {
    if value.len() != length * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|_| invalid()))
        .collect()
}
fn key(root: &Path) -> io::Result<Ed25519KeyPair> {
    private(root, true)?;
    Ed25519KeyPair::from_pkcs8(&bounded_read(&root.join(KEY), 1024)?).map_err(|_| invalid())
}

pub fn public_key(root: &Path) -> io::Result<Option<String>> {
    match key(root) {
        Ok(key) => Ok(Some(hex(key.public_key().as_ref()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn ensure_public_key(root: &Path) -> io::Result<String> {
    match fs::DirBuilder::new().mode(0o700).create(root) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    private(root, true)?;
    if let Some(value) = public_key(root)? {
        return Ok(value);
    }
    let generated = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| invalid())?;
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    file.write_all(generated.as_ref())?;
    file.as_file().sync_all()?;
    match file.persist_noclobber(root.join(KEY)) {
        Ok(_) => {
            fs::File::open(root)?.sync_all()?;
        }
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.error),
    }
    public_key(root)?.ok_or_else(invalid)
}

pub fn publish(root: &Path, endpoint: &Endpoint) -> io::Result<()> {
    ensure_public_key(root)?;
    let payload = serde_json::to_string(endpoint).map_err(io::Error::other)?;
    let message = [DOMAIN, payload.as_bytes()].concat();
    let signed = SignedEndpoint {
        version: 1,
        payload,
        signature: hex(key(root)?.sign(&message).as_ref()),
    };
    let bytes = serde_json::to_vec(&signed).map_err(io::Error::other)?;
    if bytes.len() as u64 > LIMIT {
        return Err(invalid());
    }
    let path = root.join(FILE);
    match private(&path, false) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error),
    }
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    fs::File::open(root)?.sync_all()
}

pub fn read(path: &Path, pinned_key: &str) -> io::Result<Endpoint> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    private(path.parent().ok_or_else(invalid)?, true)?;
    let signed: SignedEndpoint =
        serde_json::from_slice(&bounded_read(path, LIMIT)?).map_err(|_| invalid())?;
    if signed.version != 1 {
        return Err(invalid());
    }
    let message = [DOMAIN, signed.payload.as_bytes()].concat();
    UnparsedPublicKey::new(&ED25519, unhex(pinned_key, 32)?)
        .verify(&message, &unhex(&signed.signature, 64)?)
        .map_err(|_| invalid())?;
    serde_json::from_str(&signed.payload).map_err(|_| invalid())
}

pub fn remove(root: &Path, instance_id: &str) -> io::Result<()> {
    let Some(public) = public_key(root)? else {
        return Ok(());
    };
    let path = root.join(FILE);
    match read(&path, &public) {
        Ok(endpoint) if endpoint.instance_id == instance_id => {
            fs::remove_file(path)?;
            fs::File::open(root)?.sync_all()
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn endpoint(id: &str) -> Endpoint {
        Endpoint {
            instance_id: id.into(),
            endpoint: "/tmp/example/control.sock".into(),
            broker_sha256: "a".repeat(64),
            ipc_version: 1,
        }
    }
    #[test]
    fn registration_survives_endpoint_changes_and_rejects_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("control");
        assert!(public_key(&root).unwrap().is_none());
        let public = ensure_public_key(&root).unwrap();
        for id in ["first", "second"] {
            publish(&root, &endpoint(id)).unwrap();
            assert_eq!(read(&root.join(FILE), &public).unwrap().instance_id, id);
        }
        assert_eq!(ensure_public_key(&root).unwrap(), public);
        remove(&root, "first").unwrap();
        assert!(root.join(FILE).exists());
        assert!(read(&root.join(FILE), &"0".repeat(64)).is_err());
        let bytes = fs::read_to_string(root.join(FILE))
            .unwrap()
            .replace("second", "forged");
        fs::write(root.join(FILE), bytes).unwrap();
        assert!(read(&root.join(FILE), &public).is_err());
        publish(&root, &endpoint("second")).unwrap();
        remove(&root, "second").unwrap();
        assert!(!root.join(FILE).exists());
    }
    #[test]
    fn invalid_keys_permissions_links_and_oversized_records_are_preserved_or_refused() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("control");
        let public = ensure_public_key(&root).unwrap();
        publish(&root, &endpoint("first")).unwrap();
        fs::set_permissions(root.join(FILE), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read(&root.join(FILE), &public).is_err());
        fs::set_permissions(root.join(FILE), fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(root.join(FILE), vec![b' '; LIMIT as usize + 1]).unwrap();
        assert!(read(&root.join(FILE), &public).is_err());
        fs::remove_file(root.join(FILE)).unwrap();
        std::os::unix::fs::symlink(root.join(KEY), root.join(FILE)).unwrap();
        assert!(read(&root.join(FILE), &public).is_err());
        assert!(publish(&root, &endpoint("next")).is_err());
        fs::write(root.join(KEY), b"invalid key").unwrap();
        assert!(ensure_public_key(&root).is_err());
        assert_eq!(fs::read(root.join(KEY)).unwrap(), b"invalid key");
    }
}
