use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
};

pub fn new_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|e| e.to_string())?;
    bytes[6] = (bytes[6] & 15) | 0x40;
    bytes[8] = (bytes[8] & 63) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const METHODS: &[&str] = &[
    "getStatus",
    "getScreenshot",
    "streamScreenshot",
    "sendKey",
    "sendTouch",
    "setVmState",
    "setPhysicalModel",
];

/// An instance key is generated before spawn and is never persisted or sent to a webview.
pub struct Authority {
    key: EcdsaKeyPair,
    id: String,
}

impl Authority {
    pub fn new(id: String) -> Result<Self, String> {
        if !super::storage::valid_id(&id) {
            return Err("Invalid Android authority ID".into());
        }
        let random = SystemRandom::new();
        let encoded = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &random)
            .map_err(|e| e.to_string())?;
        let key =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, encoded.as_ref(), &random)
                .map_err(|e| e.to_string())?;
        Ok(Self { key, id })
    }

    pub fn public_jwk(&self) -> serde_json::Value {
        let bytes = self.key.public_key().as_ref();
        serde_json::json!({"keys":[{"kty":"EC","crv":"P-256","alg":"ES256","use":"sig","kid":self.id,"x":URL_SAFE_NO_PAD.encode(&bytes[1..33]),"y":URL_SAFE_NO_PAD.encode(&bytes[33..65])}]})
    }

    pub fn allowlist(&self) -> serde_json::Value {
        serde_json::json!({"unprotected":[],"allowlist":[{"iss":"lomi","protected":METHODS.iter().map(|method| rpc(method)).collect::<Vec<_>>()}]})
    }

    pub fn token(&self, method: &str) -> Result<String, String> {
        if !METHODS.contains(&method) {
            return Err("Android RPC is not allowed".into());
        }
        self.token_at(method, SystemTime::now())
    }

    fn token_at(&self, method: &str, time: SystemTime) -> Result<String, String> {
        let seconds = time
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        let encode = |value: serde_json::Value| URL_SAFE_NO_PAD.encode(value.to_string());
        // Emulator 37.1.11's Tink validator rejects typ: JWT. Audience is one exact RPC.
        let header = encode(serde_json::json!({"alg":"ES256","kid":self.id}));
        let claims = encode(
            serde_json::json!({"iss":"lomi","aud":[rpc(method)],"iat":seconds.saturating_sub(30),"exp":seconds+Duration::from_secs(120).as_secs()}),
        );
        let body = format!("{header}.{claims}");
        let signature = self
            .key
            .sign(&SystemRandom::new(), body.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "{body}.{}",
            URL_SAFE_NO_PAD.encode(signature.as_ref())
        ))
    }
}

fn rpc(method: &str) -> String {
    format!("/android.emulation.control.EmulatorController/{method}")
}
