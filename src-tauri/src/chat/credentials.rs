const SERVICE: &str = "dev.lomi.desktop.chat-ai";

pub fn put(id: &str, secret: &str) -> Result<(), String> {
    entry(id)?.set_password(secret).map_err(|_| "The system key store is locked or unavailable. Unlock it or explicitly choose session-only storage.".into())
}
pub fn get(id: &str) -> Result<String, String> {
    entry(id)?
        .get_password()
        .map_err(|_| "The API key is unavailable in the system key store.".into())
}
pub fn remove(id: &str) -> Result<(), String> {
    match entry(id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(
            "Cannot remove the old key from the system key store. Cleanup will be retried.".into(),
        ),
    }
}
fn entry(id: &str) -> Result<keyring::Entry, String> {
    if !super::process::valid_id(id) {
        return Err("Invalid credential ID.".into());
    }
    keyring::Entry::new(SERVICE, id).map_err(|_| "The system key store is unavailable.".into())
}
