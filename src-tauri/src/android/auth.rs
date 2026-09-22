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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write, path::PathBuf};
    #[test]
    fn credentials_are_bound_to_rpc_expiry_and_instance() {
        let a = Authority::new("00000000-0000-0000-0000-000000000001".into()).unwrap();
        let b = Authority::new("00000000-0000-0000-0000-000000000002".into()).unwrap();
        assert!(a.token("requestRtcStream").is_err());
        let token = a
            .token_at("sendTouch", UNIX_EPOCH + Duration::from_secs(1000))
            .unwrap();
        let parts: Vec<_> = token.split('.').collect();
        let header: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert!(header.get("typ").is_none());
        assert_eq!(claims["exp"], 1120);
        assert_eq!(claims["aud"], serde_json::json!([rpc("sendTouch")]));
        assert!(a.public_jwk()["keys"][0].get("d").is_none());
        assert_eq!(a.allowlist()["unprotected"], serde_json::json!([]));
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        let body = format!("{}.{}", parts[0], parts[1]);
        assert!(ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_FIXED,
            a.key.public_key()
        )
        .verify(body.as_bytes(), &signature)
        .is_ok());
        assert!(ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_FIXED,
            b.key.public_key()
        )
        .verify(body.as_bytes(), &signature)
        .is_err());
    }

    #[test]
    #[ignore = "Requires the owned native probe with the lomi issuer allowlist"]
    fn native_rust_credentials_and_renewal() {
        let root = PathBuf::from(std::env::var("LOMI_ANDROID_PROBE_DIRECTORY").unwrap());
        let consent: serde_json::Value =
            super::super::storage::read(&root.join("evidence/consent.json"))
                .unwrap()
                .unwrap();
        assert_eq!(consent["accepted"], true);
        let started: serde_json::Value =
            super::super::storage::read(&root.join("evidence/started.json"))
                .unwrap()
                .unwrap();
        let pid = started["pid"]
            .as_u64()
            .filter(|pid| *pid > 0 && *pid <= i32::MAX as u64)
            .unwrap();
        let discovery = PathBuf::from(std::env::var("HOME").unwrap())
            .join("Library/Caches/TemporaryItems/avd/running");
        let registration = fs::read_to_string(discovery.join(format!("pid_{pid}.ini"))).unwrap();
        let jwks = PathBuf::from(
            registration
                .lines()
                .find_map(|line| line.strip_prefix("grpc.jwks="))
                .unwrap(),
        )
        .canonicalize()
        .unwrap();
        assert_eq!(
            jwks.parent().unwrap(),
            discovery
                .join(pid.to_string())
                .join("jwks")
                .canonicalize()
                .unwrap()
        );
        let authority = Authority::new("00000000-0000-0000-0000-000000000004".into()).unwrap();
        let path = jwks.join(format!("lomi-rust-{}.jwk", std::process::id()));
        assert!(!path.exists());
        let mut temporary = tempfile::NamedTempFile::new_in(&jwks).unwrap();
        temporary
            .write_all(authority.public_jwk().to_string().as_bytes())
            .unwrap();
        temporary.as_file().sync_all().unwrap();
        temporary.persist(&path).unwrap();
        struct Registration(PathBuf);
        impl Drop for Registration {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        let _registration = Registration(path.clone());

        async fn status(token: String) -> Result<(), tonic::Code> {
            let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:18557")
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .connect()
                .await
                .map_err(|_| tonic::Code::Unavailable)?;
            let mut client = tonic::client::Grpc::new(channel);
            client.ready().await.map_err(|_| tonic::Code::Unavailable)?;
            let mut request = tonic::Request::new(());
            request
                .metadata_mut()
                .insert("authorization", format!("Bearer {token}").parse().unwrap());
            let response: Result<tonic::Response<()>, _> = client
                .unary(
                    request,
                    tonic::codegen::http::uri::PathAndQuery::from_static(
                        "/android.emulation.control.EmulatorController/getStatus",
                    ),
                    tonic_prost::ProstCodec::default(),
                )
                .await;
            response.map(|_| ()).map_err(|error| error.code())
        }
        tauri::async_runtime::block_on(async {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                if status(authority.token("getStatus").unwrap()).await.is_ok() {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "The emulator did not accept its registered Rust authority"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert_eq!(
                status(authority.token("sendTouch").unwrap()).await,
                Err(tonic::Code::PermissionDenied)
            );
            let expired = authority
                .token_at("getStatus", SystemTime::now() - Duration::from_secs(180))
                .unwrap();
            assert_eq!(status(expired).await, Err(tonic::Code::InvalidArgument));
            assert!(status(authority.token("getStatus").unwrap()).await.is_ok());
            let other = Authority::new("00000000-0000-0000-0000-000000000005".into()).unwrap();
            assert_eq!(
                status(other.token("getStatus").unwrap()).await,
                Err(tonic::Code::InvalidArgument)
            );
        });
        fs::remove_file(path).unwrap();
        fs::write(
            root.join("evidence/native-rust-auth.json"),
            serde_json::json!({
                "passed":true,"issuer":"lomi","algorithm":"ES256","lifetimeSeconds":120,
                "wrongAudience":"PermissionDenied","expired":"InvalidArgument","renewed":"accepted",
                "unregisteredInstance":"InvalidArgument","privateKeyPersisted":false
            })
            .to_string(),
        )
        .unwrap();
    }
}
