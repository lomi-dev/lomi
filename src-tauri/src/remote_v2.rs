use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures_util::{Sink, SinkExt, StreamExt};
use lomi_remote_crypto_canonical::{
    verify_consumer_window_identity, verify_grant_proposal_v3, verify_registered_host_identity,
    Channel as NoiseChannel, ChannelContextV3, ChannelPurposeV3, ConsumerWindowChallenge,
    CurrentAuthorityV3, GrantV3, Handshake, Identity, Role, SignedBundle, SignedChannelAdmissionV3,
    SignedGrantV3, SignedHostKeyCommitment, TerminalTargetV3, VerifiedChannelContextV3,
};
use lomi_remote_protocol::WssAdmissionFrame;
use lomi_remote_protocol::{
    http::{
        ChannelCreateParams, ChannelCreateResult, ChannelResult, ChannelTarget,
        ChannelTerminalMembership, ChannelTicketParams, ChannelTicketResult,
        ConsumerAuthorizationContext, ConsumerWindowChallengeParams, ConsumerWindowChallengeResult,
        ConsumerWindowGetResult, ConsumerWindowRegisterParams, ConsumerWindowRegisterResult,
        CurrentAuthorityBinding, HttpRoute, InventoryHost, InventoryResult, InventoryStatus,
        PairingResult, RelayTicket, RuntimeBinding,
    },
    message_framing::{CompletedMessage, MessageAssembler, MessageEncoder, MessageKind},
    rpc::{
        BoundedList, LeaseBinding, RequestKind, RpcCall, RpcRequest, RpcResponse, RpcResult,
        TerminalClaimParams, TerminalDimension, TerminalInputParams, TerminalLeaseParams,
        TerminalResizeParams,
    },
    streams::{InputSeq, ResizeSeq},
    AccountId, AccountSessionId, ApplicationMajor, ChannelId, ConsumerAuthorizationId, DecimalU64,
    EnrollmentProof, GrantId, OperationId, PairingId, RequestId, TerminalContext, TerminalProfile,
    TimeoutMs, WindowIdentityId,
};
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{ipc::Channel as IpcChannel, Manager, State, Webview, Window};
use tokio::{
    sync::{mpsc, oneshot},
    time::{sleep_until, timeout, Instant as TokioInstant},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use zeroize::Zeroize;

const RECORDS_VERSION: u32 = 1;
const RECORDS_LIMIT: u64 = 1024 * 1024;
const IDENTITY_SECRET_SERVICE: &str = "dev.lomi.desktop.remote-v2.consumer-window.v1";

#[derive(Default)]
pub(crate) struct RemoteV2 {
    records_lock: Mutex<()>,
    registration_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    channels: Mutex<BTreeMap<String, ActiveRemoteChannel>>,
}

struct ActiveRemoteChannel {
    window_label: String,
    window_identity_id: String,
    sender: mpsc::Sender<RemoteChannelCommand>,
    terminal_control: Option<NativeTerminalControl>,
}

#[derive(Clone)]
struct NativeTerminalControl {
    sender: mpsc::Sender<NativeTerminalControlCommand>,
    focused: Arc<AtomicBool>,
    focus_generation: Arc<AtomicU64>,
    queued_bytes: Arc<AtomicUsize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteConsumerSessionTuple {
    account_id: String,
    consumer_authorization_id: String,
    account_session_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConsumerWindowRecord {
    window_label: String,
    account_id: String,
    consumer_authorization_id: String,
    account_session_id: String,
    window_identity_id: String,
    key_account: String,
    stage: RegistrationStage,
    identity_fingerprint: Option<String>,
    pending_register: Option<ConsumerWindowRegisterParams>,
    revocation_confirmed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum RegistrationStage {
    New,
    ChallengePending,
    RegisterPending,
    Registered,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConsumerWindowRecords {
    version: u32,
    records: BTreeMap<String, ConsumerWindowRecord>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredIdentitySecret {
    signed_bundle: SignedBundle,
    seed_blob: String,
}

impl Drop for StoredIdentitySecret {
    fn drop(&mut self) {
        self.seed_blob.zeroize();
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeConsumerWindowRegistration {
    window_identity_id: String,
    identity_fingerprint: String,
    session_tuple: RemoteConsumerSessionTuple,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RemoteV2NativeEvent {
    Message {
        kind: u8,
        id: String,
        bytes: Vec<u8>,
    },
    Closed {
        code: String,
    },
}

enum RemoteChannelCommand {
    Send {
        kind: u8,
        bytes: zeroize::Zeroizing<Vec<u8>>,
        completed: oneshot::Sender<Result<(), String>>,
    },
    Rpc {
        request: RpcRequest,
        focus_generation: Option<(Arc<AtomicU64>, u64)>,
        completed: oneshot::Sender<Result<RpcResponse, String>>,
    },
    Close,
}

enum NativeTerminalControlCommand {
    Claim {
        params: TerminalClaimParams,
        focus_generation: u64,
        completed: oneshot::Sender<Result<(), String>>,
    },
    Input {
        bytes: zeroize::Zeroizing<Vec<u8>>,
        focus_generation: u64,
        completed: oneshot::Sender<Result<(), String>>,
    },
    Resize {
        columns: u32,
        rows: u32,
        focus_generation: u64,
        completed: oneshot::Sender<Result<(), String>>,
    },
    Release {
        completed: oneshot::Sender<Result<(), String>>,
    },
    Close,
}

fn records_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| "Remote window storage is unavailable.")?
        .join("remote-v2-consumer-windows.json"))
}

impl RemoteV2 {
    fn registration_lock(&self, window_label: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
        let mut locks = self
            .registration_locks
            .lock()
            .map_err(|_| "Remote window registration state is unavailable.")?;
        Ok(locks
            .entry(window_label.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone())
    }
}

fn read_records(path: &Path) -> Result<ConsumerWindowRecords, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConsumerWindowRecords {
                version: RECORDS_VERSION,
                records: BTreeMap::new(),
            });
        }
        Err(_) => return Err("Remote window storage could not be opened.".into()),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(RECORDS_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Remote window storage could not be read.")?;
    if bytes.len() as u64 > RECORDS_LIMIT {
        return Err("Remote window storage exceeds its size limit.".into());
    }
    let records: ConsumerWindowRecords = serde_json::from_slice(&bytes)
        .map_err(|_| "Remote window storage is invalid and was preserved.")?;
    validate_records(&records)?;
    Ok(records)
}

fn validate_records(records: &ConsumerWindowRecords) -> Result<(), String> {
    if records.version != RECORDS_VERSION || records.records.len() > 256 {
        return Err("Remote window storage is unsupported and was preserved.".into());
    }
    for record in records.records.values() {
        validate_window_label(&record.window_label)?;
        AccountId::try_from(record.account_id.clone())
            .map_err(|_| "Remote window storage contains an invalid account ID.")?;
        ConsumerAuthorizationId::try_from(record.consumer_authorization_id.clone())
            .map_err(|_| "Remote window storage contains an invalid authorization ID.")?;
        AccountSessionId::try_from(record.account_session_id.clone())
            .map_err(|_| "Remote window storage contains an invalid session ID.")?;
        WindowIdentityId::try_from(record.window_identity_id.clone())
            .map_err(|_| "Remote window storage contains an invalid window ID.")?;
        if record.key_account.len() > 256
            || !record
                .key_account
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || record
                .identity_fingerprint
                .as_deref()
                .is_some_and(|value| !valid_hex(value, 64))
            || (record.stage == RegistrationStage::RegisterPending)
                != record.pending_register.is_some()
            || (record.stage == RegistrationStage::Registered)
                != record.identity_fingerprint.is_some()
        {
            return Err("Remote window storage contains an invalid registration.".into());
        }
    }
    Ok(())
}

fn validate_window_label(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        Err("Remote access is unavailable in this webview.".into())
    } else {
        Ok(())
    }
}

fn persist_records(path: &Path, records: &ConsumerWindowRecords) -> Result<(), String> {
    validate_records(records)?;
    crate::files::write_json(path, records, RECORDS_LIMIT as usize)
}

fn record_key(window_label: &str, tuple: &RemoteConsumerSessionTuple) -> String {
    format!(
        "{}|{}|{}|{}",
        window_label, tuple.account_id, tuple.consumer_authorization_id, tuple.account_session_id
    )
}

fn secret_account(window_label: &str, window_id: &str) -> String {
    format!("{window_label}.{window_id}")
}

fn identity_service() -> Result<String, String> {
    let origin = option_env!("LOMI_HOST_IDENTITY_ORIGIN")
        .ok_or("Remote identity is not configured for this build.")?;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(origin.as_bytes());
    Ok(format!(
        "{IDENTITY_SECRET_SERVICE}.{}",
        digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn account_hex(value: &str) -> String {
    value
        .bytes()
        .filter(|byte| *byte != b'-')
        .map(char::from)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn same_account_id(left: &str, right: &str) -> bool {
    account_hex(left) == account_hex(right)
}

fn validate_session_tuple(tuple: &RemoteConsumerSessionTuple) -> Result<(), String> {
    AccountId::try_from(tuple.account_id.clone()).map_err(|_| "Invalid account ID.")?;
    ConsumerAuthorizationId::try_from(tuple.consumer_authorization_id.clone())
        .map_err(|_| "Invalid consumer authorization ID.")?;
    AccountSessionId::try_from(tuple.account_session_id.clone())
        .map_err(|_| "Invalid account session ID.")?;
    Ok(())
}

fn session_tuple(context: &ConsumerAuthorizationContext) -> RemoteConsumerSessionTuple {
    RemoteConsumerSessionTuple {
        account_id: context.account_id.clone().into(),
        consumer_authorization_id: context.consumer_authorization_id.clone().into(),
        account_session_id: context.account_session_id.clone().into(),
    }
}

fn request_route(method: &reqwest::Method, path: &str) -> Result<HttpRoute, String> {
    let route = crate::auth::http::remote_v2_route(method, path)
        .ok_or("This Remote v2 route is unavailable.")?;
    if route.route != HttpRoute::ConsumerAuthorizationIssue
        && !matches!(
            route.surface,
            lomi_remote_protocol::HttpSurface::ConsumerIdentity
                | lomi_remote_protocol::HttpSurface::ConsumerManagement
                | lomi_remote_protocol::HttpSurface::ConsumerSession
                | lomi_remote_protocol::HttpSurface::EitherOwnPeer
        )
    {
        return Err("This Remote v2 route is not available to desktop consumers.".into());
    }
    Ok(route.route)
}

fn request_kind(route: HttpRoute) -> Option<&'static str> {
    Some(match route {
        HttpRoute::ConsumerAuthorizationIssue => "ConsumerAuthorizationIssueParams",
        HttpRoute::ConsumerAuthorizationRevoke => "ConsumerAuthorizationRevokeParams",
        HttpRoute::ConsumerWindowChallenge => "ConsumerWindowChallengeParams",
        HttpRoute::ConsumerWindowRegister => "ConsumerWindowRegisterParams",
        HttpRoute::ConsumerWindowRevoke => "ConsumerWindowRevokeParams",
        HttpRoute::ConsumerWindowGet | HttpRoute::Inventory | HttpRoute::PairingGet => return None,
        HttpRoute::PairingCreate => "PairingCreateParams",
        HttpRoute::GrantRevoke => "GrantRevokeParams",
        HttpRoute::ChannelCreate => "ChannelCreateParams",
        HttpRoute::ChannelGet => return None,
        HttpRoute::ChannelTicket => "ChannelTicketParams",
        _ => return None,
    })
}

fn response_kind(route: HttpRoute) -> Option<&'static str> {
    Some(match route {
        HttpRoute::ConsumerAuthorizationIssue => "ConsumerAuthorizationContext",
        HttpRoute::ConsumerAuthorizationRevoke => "ConsumerAuthorizationRevokeResult",
        HttpRoute::ConsumerWindowChallenge => "ConsumerWindowChallengeResult",
        HttpRoute::ConsumerWindowRegister => "ConsumerWindowRegisterResult",
        HttpRoute::ConsumerWindowGet => "ConsumerWindowGetResult",
        HttpRoute::ConsumerWindowRevoke => "ConsumerWindowRevokeResult",
        HttpRoute::Inventory => "InventoryResult",
        HttpRoute::PairingCreate | HttpRoute::PairingGet => "PairingResult",
        HttpRoute::GrantRevoke => "GrantRevokeResult",
        HttpRoute::ChannelCreate => "ChannelCreateResult",
        HttpRoute::ChannelGet => "ChannelResult",
        HttpRoute::ChannelTicket => "ChannelTicketResult",
        _ => return None,
    })
}

fn validate_request_body(route: HttpRoute, body: Option<&Value>) -> Result<(), String> {
    if matches!(
        route,
        HttpRoute::ConsumerAuthorizationRevoke
            | HttpRoute::ConsumerWindowGet
            | HttpRoute::ConsumerWindowRevoke
            | HttpRoute::GrantRevoke
    ) {
        return if body.is_none() {
            Ok(())
        } else {
            Err("This Remote v2 route does not accept a request body.".into())
        };
    }
    if matches!(
        route,
        HttpRoute::Inventory | HttpRoute::PairingGet | HttpRoute::ChannelGet
    ) {
        return if body.is_none() {
            Ok(())
        } else {
            Err("This Remote v2 route does not accept a request body.".into())
        };
    }
    let kind = request_kind(route).ok_or("This Remote v2 route is unavailable.")?;
    let body = body.ok_or("This Remote v2 route requires its canonical request body.")?;
    let json = serde_json::to_string(body).map_err(|_| "Invalid Remote v2 request body.")?;
    lomi_remote_protocol::http::validate_http_wire(kind, &json)
        .map_err(|_| "Invalid Remote v2 request body.".to_owned())
}

async fn remote_request(
    auth: &crate::auth::AuthController,
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    remote_request_structured(auth, method, path, body)
        .await
        .map_err(|error| error.message().to_owned())
}

async fn remote_request_structured(
    auth: &crate::auth::AuthController,
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, crate::auth::controller::RemoteV2HttpError> {
    let route = request_route(&method, path)?;
    validate_request_body(route, body)?;
    let (_, value) = auth.remote_v2_request(method, path, body).await?;
    let kind = response_kind(route).ok_or("This Remote v2 response is unavailable.")?;
    let json = serde_json::to_string(&value).map_err(|_| "Invalid Remote v2 response.")?;
    lomi_remote_protocol::http::validate_http_wire(kind, &json)
        .map_err(|_| "Invalid Remote v2 response.")?;
    Ok(value)
}

#[tauri::command]
pub async fn remote_v2_request(
    window: Window,
    webview: Webview,
    auth: State<'_, crate::auth::AuthController>,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<Value, crate::auth::controller::RemoteV2HttpError> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    let method = match method.as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        _ => return Err("This Remote v2 method is unavailable.".into()),
    };
    remote_request_structured(&auth, method, &path, body.as_ref()).await
}

#[tauri::command]
pub fn remote_v2_account_id(
    window: Window,
    webview: Webview,
    auth: State<'_, crate::auth::AuthController>,
) -> Result<Option<String>, String> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    Ok(auth.remote_binding().ok().map(|binding| binding.user_id))
}

#[tauri::command]
pub fn remote_v2_open_view(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    host_id: String,
    workspace_id: String,
) -> Result<(), String> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    if !valid_uuid(&host_id) || !valid_uuid(&workspace_id) {
        return Err("Invalid remote workspace reference.".into());
    }

    let mut config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == "main")
        .cloned()
        .ok_or("The desktop window configuration is unavailable.")?;
    let label = format!("remoteview{}", new_uuid()?.replace('-', ""));
    validate_window_label(&label)?;
    config.create = false;
    config.label = label;
    config.title = "Remote workspace".into();
    config.url = tauri::WebviewUrl::App(
        format!("index.html?view=remote-v2&host={host_id}&workspace={workspace_id}").into(),
    );
    config.decorations = true;
    config.transparent = false;
    config.visible = true;

    tauri::WebviewWindowBuilder::from_config(&app, &config)
        .map_err(|_| "The remote workspace window could not be prepared.")?
        .build()
        .map_err(|_| "The remote workspace window could not be opened.")?;
    Ok(())
}

#[tauri::command]
pub async fn remote_v2_authorize_consumer(
    window: Window,
    webview: Webview,
    auth: State<'_, crate::auth::AuthController>,
) -> Result<RemoteConsumerSessionTuple, crate::auth::controller::RemoteV2HttpError> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    let value = remote_request_structured(
        &auth,
        reqwest::Method::POST,
        "/v2/remote/consumer-authorizations",
        Some(&serde_json::json!({})),
    )
    .await?;
    let context: ConsumerAuthorizationContext =
        serde_json::from_value(value).map_err(|_| "Invalid consumer authorization response.")?;
    let binding = auth.remote_binding()?;
    let tuple = session_tuple(&context);
    validate_session_tuple(&tuple)?;
    if !same_account_id(&binding.user_id, &tuple.account_id)
        || binding.session_id != tuple.account_session_id
    {
        return Err("The account session changed during remote authorization.".into());
    }
    Ok(tuple)
}

async fn current_inventory(auth: &crate::auth::AuthController) -> Result<InventoryResult, String> {
    current_inventory_structured(auth)
        .await
        .map_err(|error| error.message().to_owned())
}

async fn current_inventory_structured(
    auth: &crate::auth::AuthController,
) -> Result<InventoryResult, crate::auth::controller::RemoteV2HttpError> {
    let value =
        remote_request_structured(auth, reqwest::Method::GET, "/v2/remote/inventory", None).await?;
    serde_json::from_value(value)
        .map_err(|_| crate::auth::controller::RemoteV2HttpError::local_code("INVALID_RESPONSE"))
}

fn tuple_matches_binding(
    tuple: &RemoteConsumerSessionTuple,
    binding: &crate::auth::controller::RemoteBinding,
) -> bool {
    same_account_id(&tuple.account_id, &binding.user_id)
        && tuple.account_session_id == binding.session_id
}

async fn validate_registration_tuple(
    auth: &crate::auth::AuthController,
    account_id: &str,
    expected: &RemoteConsumerSessionTuple,
) -> Result<(), crate::auth::controller::RemoteV2HttpError> {
    let inventory = current_inventory_structured(auth).await?;
    let binding = auth
        .remote_binding()
        .map_err(crate::auth::controller::RemoteV2HttpError::from)?;
    if !same_account_id(account_id, &expected.account_id)
        || !tuple_matches_binding(expected, &binding)
        || inventory_tuple(&inventory).as_ref() != Some(expected)
    {
        return Err(crate::auth::controller::RemoteV2HttpError::local_code(
            "SESSION_CHANGED",
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn remote_v2_existing_window_identity(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, RemoteV2>,
    auth: State<'_, crate::auth::AuthController>,
    tuple: RemoteConsumerSessionTuple,
) -> Result<Option<String>, String> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    validate_session_tuple(&tuple)?;
    let inventory = current_inventory(&auth).await?;
    let binding = auth.remote_binding()?;
    if !tuple_matches_binding(&tuple, &binding)
        || !inventory_tuple(&inventory).is_some_and(|current| current == tuple)
    {
        return Ok(None);
    }
    let path = records_path(&app)?;
    let _guard = state
        .records_lock
        .lock()
        .map_err(|_| "Remote window storage is unavailable.")?;
    let records = read_records(&path)?;
    let key = record_key(window.label(), &tuple);
    Ok(records
        .records
        .get(&key)
        .filter(|record| !record.revocation_confirmed)
        .map(|record| record.window_identity_id.clone()))
}

fn inventory_tuple(inventory: &InventoryResult) -> Option<RemoteConsumerSessionTuple> {
    let context = &inventory.authorization;
    Some(session_tuple(context))
}

fn create_identity_record(
    window_label: &str,
    tuple: &RemoteConsumerSessionTuple,
) -> Result<ConsumerWindowRecord, String> {
    validate_window_label(window_label)?;
    let window_id = new_uuid()?;
    Ok(ConsumerWindowRecord {
        window_label: window_label.to_owned(),
        account_id: tuple.account_id.clone(),
        consumer_authorization_id: tuple.consumer_authorization_id.clone(),
        account_session_id: tuple.account_session_id.clone(),
        window_identity_id: window_id.clone(),
        key_account: secret_account(window_label, &window_id),
        stage: RegistrationStage::New,
        identity_fingerprint: None,
        pending_register: None,
        revocation_confirmed: false,
    })
}

fn new_uuid() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Secure randomness is unavailable.")?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
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

fn valid_hex(value: &str, exact_len: usize) -> bool {
    value.len() == exact_len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn decode_hex(value: &str, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    if value.is_empty()
        || value.len() % 2 != 0
        || value.len() / 2 > maximum_bytes
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Invalid canonical cryptographic bytes.".into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair =
                std::str::from_utf8(pair).map_err(|_| "Invalid canonical cryptographic bytes.")?;
            u8::from_str_radix(pair, 16)
                .map_err(|_| "Invalid canonical cryptographic bytes.".into())
        })
        .collect()
}

fn canonical_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

enum IdentityLoadError {
    Missing,
    Unavailable,
}

fn identity_from_record(record: &ConsumerWindowRecord) -> Result<Identity, IdentityLoadError> {
    let service = identity_service().map_err(|_| IdentityLoadError::Unavailable)?;
    let mut serialized =
        crate::credential_store::get_password(&service, &record.key_account, 32 * 1024).map_err(
            |error| match error {
                keyring::Error::NoEntry => IdentityLoadError::Missing,
                _ => IdentityLoadError::Unavailable,
            },
        )?;
    let result: Result<Identity, String> = (|| -> Result<Identity, String> {
        let mut stored: StoredIdentitySecret = serde_json::from_str(&serialized)
            .map_err(|_| "The saved consumer identity is invalid.")?;
        let seed = ZeroizingSeed::decode(&stored.seed_blob)?;
        Identity::import_secret_seed_blob(&seed.0, &stored.signed_bundle)
            .map_err(|_| "The saved consumer identity could not be restored.".to_owned())
    })();
    serialized.zeroize();
    result.map_err(|_| IdentityLoadError::Unavailable)
}

struct ZeroizingSeed(zeroize::Zeroizing<Vec<u8>>);

impl ZeroizingSeed {
    fn decode(value: &str) -> Result<Self, String> {
        URL_SAFE_NO_PAD
            .decode(value)
            .map(zeroize::Zeroizing::new)
            .map(Self)
            .map_err(|_| "The saved consumer identity is invalid.".into())
    }
}

fn save_identity(record: &ConsumerWindowRecord, identity: &Identity) -> Result<(), String> {
    let service = identity_service()?;
    let entry = crate::credential_store::entry(&service, &record.key_account)
        .map_err(|_| "The system credential store is unavailable.")?;
    let seed = identity
        .export_secret_seed_blob()
        .map_err(|_| "The consumer identity could not be saved.")?;
    let stored = StoredIdentitySecret {
        signed_bundle: identity.public_bundle(),
        seed_blob: URL_SAFE_NO_PAD.encode(&*seed),
    };
    let mut serialized =
        serde_json::to_string(&stored).map_err(|_| "The consumer identity could not be saved.")?;
    let saved: Result<(), String> = entry
        .set_password(&serialized)
        .map_err(|_| "The system credential store is unavailable.".to_owned());
    serialized.zeroize();
    saved
}

fn generate_identity(
    record: &ConsumerWindowRecord,
) -> Result<(Identity, String, EnrollmentProof), String> {
    let account_id = AccountId::try_from(record.account_id.clone())
        .map_err(|_| "Invalid consumer account ID.")?;
    let window_id = WindowIdentityId::try_from(record.window_identity_id.clone())
        .map_err(|_| "Invalid consumer window ID.")?;
    let identity = Identity::generate(account_id.to_bytes(), window_id.to_bytes(), Role::Device, 1)
        .map_err(|_| "The consumer identity could not be created.")?;
    let bundle = identity.public_bundle();
    let fingerprint = canonical_hex(
        &bundle
            .bundle
            .fingerprint()
            .map_err(|_| "The consumer identity is invalid.")?,
    );
    let wire = bundle
        .to_wire()
        .map_err(|_| "The consumer identity is invalid.")?;
    Ok((identity, fingerprint, wire))
}

async fn ensure_identity(
    app: &tauri::AppHandle,
    state: &RemoteV2,
    window_label: &str,
    tuple: &RemoteConsumerSessionTuple,
) -> Result<(ConsumerWindowRecord, Identity), String> {
    let path = records_path(app)?;
    let record = {
        let _guard = state
            .records_lock
            .lock()
            .map_err(|_| "Remote window storage is unavailable.")?;
        let mut records = read_records(&path)?;
        let key = record_key(window_label, tuple);
        let record = match records.records.get(&key) {
            Some(record) if !record.revocation_confirmed => record.clone(),
            Some(_) => return Err("This consumer window is being revoked.".into()),
            None => {
                if records.records.len() >= 256 {
                    return Err("The desktop has reached the consumer window limit.".into());
                }
                let record = create_identity_record(window_label, tuple)?;
                records.records.insert(key.clone(), record.clone());
                persist_records(&path, &records)?;
                record
            }
        };
        record
    };
    match identity_from_record(&record) {
        Ok(identity) => Ok((record, identity)),
        Err(IdentityLoadError::Missing) if record.stage == RegistrationStage::New => {
            let (identity, _fingerprint, _wire) = generate_identity(&record)?;
            save_identity(&record, &identity)?;
            Ok((record, identity))
        }
        Err(IdentityLoadError::Missing | IdentityLoadError::Unavailable) => {
            Err("The consumer identity is unavailable in the system credential store.".into())
        }
    }
}

fn save_record(
    app: &tauri::AppHandle,
    state: &RemoteV2,
    record: &ConsumerWindowRecord,
) -> Result<(), String> {
    let path = records_path(app)?;
    let _guard = state
        .records_lock
        .lock()
        .map_err(|_| "Remote window storage is unavailable.")?;
    let mut records = read_records(&path)?;
    let key = record_key(
        &record.window_label,
        &RemoteConsumerSessionTuple {
            account_id: record.account_id.clone(),
            consumer_authorization_id: record.consumer_authorization_id.clone(),
            account_session_id: record.account_session_id.clone(),
        },
    );
    records.records.insert(key, record.clone());
    persist_records(&path, &records)
}

fn operation_id(value: &str) -> Result<OperationId, String> {
    OperationId::try_from(value.to_owned()).map_err(|_| "Invalid remote operation ID.".into())
}

fn now_unix_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| "The system clock is unavailable.".into())
}

async fn register_window_identity(
    app: &tauri::AppHandle,
    state: &RemoteV2,
    auth: &crate::auth::AuthController,
    window_label: &str,
    account_id: &str,
) -> Result<NativeConsumerWindowRegistration, crate::auth::controller::RemoteV2HttpError> {
    let inventory = current_inventory_structured(auth).await?;
    let tuple = inventory_tuple(&inventory).ok_or("Remote authorization is unavailable.")?;
    validate_session_tuple(&tuple)?;
    let binding = auth.remote_binding()?;
    if !tuple_matches_binding(&tuple, &binding) || !same_account_id(account_id, &tuple.account_id) {
        return Err("The account session changed before consumer registration.".into());
    }

    let (mut record, identity) = ensure_identity(app, state, window_label, &tuple).await?;
    let bundle = identity.public_bundle();
    let wire = bundle
        .to_wire()
        .map_err(|_| "The consumer identity is invalid.")?;
    let fingerprint = canonical_hex(
        &bundle
            .bundle
            .fingerprint()
            .map_err(|_| "The consumer identity is invalid.")?,
    );
    if record.stage == RegistrationStage::Registered {
        if record.identity_fingerprint.as_deref() != Some(&fingerprint) {
            return Err("The registered consumer identity does not match local storage.".into());
        }
        return Ok(NativeConsumerWindowRegistration {
            window_identity_id: record.window_identity_id,
            identity_fingerprint: fingerprint,
            session_tuple: tuple,
        });
    }

    let existing_registration = remote_request_structured(
        auth,
        reqwest::Method::GET,
        &uuid_path(
            HttpRoute::ConsumerWindowGet,
            "{windowIdentityId}",
            &record.window_identity_id,
        ),
        None,
    )
    .await;
    match existing_registration {
        Ok(value) => {
            validate_registration_tuple(auth, account_id, &tuple).await?;
            let registered: ConsumerWindowGetResult =
                serde_json::from_value(value).map_err(|_| {
                    crate::auth::controller::RemoteV2HttpError::local_code("INVALID_RESPONSE")
                })?;
            if String::from(registered.window_identity_id) != record.window_identity_id
                || registered.identity_fingerprint.expose_wire() != fingerprint
            {
                return Err(crate::auth::controller::RemoteV2HttpError::local_code(
                    "IDENTITY_MISMATCH",
                ));
            }
            record.stage = RegistrationStage::Registered;
            record.identity_fingerprint = Some(fingerprint.clone());
            record.pending_register = None;
            save_record(app, state, &record)?;
            return Ok(NativeConsumerWindowRegistration {
                window_identity_id: record.window_identity_id,
                identity_fingerprint: fingerprint,
                session_tuple: tuple,
            });
        }
        Err(error) if error.status == Some(404) => {
            validate_registration_tuple(auth, account_id, &tuple).await?;
            record.stage = RegistrationStage::ChallengePending;
            record.pending_register = None;
            save_record(app, state, &record)?;
        }
        Err(error) => return Err(error),
    }

    if record.stage == RegistrationStage::ChallengePending {
        let window_identity_id = WindowIdentityId::try_from(record.window_identity_id.clone())
            .map_err(|_| "Invalid consumer window ID.")?;
        let params = ConsumerWindowChallengeParams {
            window_identity_id,
            identity: wire,
        };
        let body = serde_json::to_value(params)
            .map_err(|_| "The consumer identity request could not be encoded.")?;
        let response = remote_request_structured(
            auth,
            reqwest::Method::POST,
            "/v2/remote/consumer-windows/challenge",
            Some(&body),
        )
        .await?;
        let challenge: ConsumerWindowChallengeResult = serde_json::from_value(response)
            .map_err(|_| "The consumer identity challenge is invalid.")?;
        let challenge_bytes = decode_hex(challenge.challenge.expose_wire(), 8192)?;
        let challenge_value = ConsumerWindowChallenge::from_canonical(&challenge_bytes)
            .map_err(|_| "The consumer identity challenge is invalid.")?;
        let account = AccountId::try_from(record.account_id.clone())
            .map_err(|_| "Invalid consumer account ID.")?;
        let authorization =
            ConsumerAuthorizationId::try_from(record.consumer_authorization_id.clone())
                .map_err(|_| "Invalid consumer authorization ID.")?;
        let window_id = WindowIdentityId::try_from(record.window_identity_id.clone())
            .map_err(|_| "Invalid consumer window ID.")?;
        let operation: OperationId = challenge.operation_id;
        if challenge_value.account_id != account.to_bytes()
            || challenge_value.consumer_authorization_id != authorization.to_bytes()
            || challenge_value.account_session_id != record.account_session_id
            || challenge_value.window_identity_id != window_id.to_bytes()
            || challenge_value.operation_id != operation.to_bytes()
            || challenge_value.window_key_fingerprint
                != bundle
                    .bundle
                    .fingerprint()
                    .map_err(|_| "The consumer identity is invalid.")?
            || challenge_value.expires_at.0 <= now_unix_seconds()?
        {
            return Err(
                "The consumer identity challenge does not match this account window.".into(),
            );
        }
        let proof = identity
            .sign_consumer_window_proof(&challenge_value)
            .map_err(|_| "The consumer identity proof could not be created.")?;
        record.pending_register = Some(ConsumerWindowRegisterParams {
            window_identity_id: window_id,
            operation_id: operation,
            proof,
        });
        record.stage = RegistrationStage::RegisterPending;
        save_record(app, state, &record)?;
    }

    let pending = record
        .pending_register
        .clone()
        .ok_or("The consumer registration recovery state is invalid.")?;
    let body = serde_json::to_value(pending)
        .map_err(|_| "The consumer registration request could not be encoded.")?;
    let response = remote_request_structured(
        auth,
        reqwest::Method::POST,
        "/v2/remote/consumer-windows/register",
        Some(&body),
    )
    .await?;
    let registered: ConsumerWindowRegisterResult = serde_json::from_value(response)
        .map_err(|_| "The consumer registration response is invalid.")?;
    if String::from(registered.window_identity_id) != record.window_identity_id
        || registered.identity_fingerprint.expose_wire() != fingerprint
    {
        return Err("The server registered a different consumer identity.".into());
    }
    record.stage = RegistrationStage::Registered;
    record.identity_fingerprint = Some(fingerprint.clone());
    record.pending_register = None;
    save_record(app, state, &record)?;
    Ok(NativeConsumerWindowRegistration {
        window_identity_id: record.window_identity_id,
        identity_fingerprint: fingerprint,
        session_tuple: tuple,
    })
}

#[tauri::command]
pub async fn remote_v2_register_consumer_window(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, RemoteV2>,
    auth: State<'_, crate::auth::AuthController>,
    account_id: String,
) -> Result<NativeConsumerWindowRegistration, crate::auth::controller::RemoteV2HttpError> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    let window_label = window.label().to_owned();
    let registration_lock = state.registration_lock(&window_label)?;
    let _registration_guard = registration_lock.lock_owned().await;
    register_window_identity(&app, &state, &auth, &window_label, &account_id).await
}

#[tauri::command]
pub async fn remote_v2_clear_revoked_consumer_window(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, RemoteV2>,
    account_id: String,
    window_identity_id: String,
) -> Result<(), String> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    let registration_lock = state.registration_lock(window.label())?;
    let _registration_guard = registration_lock.lock_owned().await;
    AccountId::try_from(account_id.clone()).map_err(|_| "Invalid account ID.")?;
    WindowIdentityId::try_from(window_identity_id.clone()).map_err(|_| "Invalid window ID.")?;
    let path = records_path(&app)?;
    let record = {
        let _guard = state
            .records_lock
            .lock()
            .map_err(|_| "Remote window storage is unavailable.")?;
        let mut records = read_records(&path)?;
        let selected = records
            .records
            .iter_mut()
            .find(|(_, record)| {
                record.window_label == window.label()
                    && record.account_id == account_id
                    && record.window_identity_id == window_identity_id
            })
            .map(|(_, record)| record)
            .ok_or("The revoked consumer identity is not saved in this window.")?;
        selected.revocation_confirmed = true;
        let record = selected.clone();
        persist_records(&path, &records)?;
        record
    };
    let service = identity_service()?;
    match crate::credential_store::entry(&service, &record.key_account)
        .and_then(|entry| entry.delete_credential())
    {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(_) => return Err(
            "The revoked consumer identity could not be removed from the system credential store."
                .into(),
        ),
    }
    let _guard = state
        .records_lock
        .lock()
        .map_err(|_| "Remote window storage is unavailable.")?;
    let mut records = read_records(&path)?;
    records.records.retain(|_, candidate| {
        candidate.window_label != record.window_label
            || candidate.window_identity_id != record.window_identity_id
            || candidate.account_id != record.account_id
    });
    persist_records(&path, &records)
}

fn uuid_path(route: HttpRoute, placeholder: &str, id: &str) -> String {
    let path = route.contract().path.replace(placeholder, id);
    format!("{}{}", lomi_remote_protocol::http::REMOTE_HTTP_PREFIX, path)
}

fn parse_json<T: serde::de::DeserializeOwned>(
    value: Value,
    message: &'static str,
) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| message.into())
}

fn current_workspace(
    host: &InventoryHost,
    workspace_id: lomi_remote_protocol::WorkspaceId,
    workspace_epoch: lomi_remote_protocol::WorkspaceEpoch,
) -> bool {
    host.workspaces.as_slice().iter().any(|workspace| {
        workspace.workspace_id == workspace_id && workspace.workspace_epoch == workspace_epoch
    })
}

fn terminal_target(target: &ChannelTarget) -> Option<TerminalTargetV3> {
    match target {
        ChannelTarget::Workspace { .. } => None,
        ChannelTarget::Terminal {
            pane_id,
            session_id,
            session_epoch,
            ..
        } => Some(TerminalTargetV3 {
            pane_id: pane_id.to_bytes(),
            session_id: session_id.to_bytes(),
            session_epoch: session_epoch.to_bytes(),
            terminal_profile: "lomi-rust-grid-v1-experimental".into(),
        }),
    }
}

fn current_authority(
    value: &CurrentAuthorityBinding,
    terminal_membership: Option<TerminalTargetV3>,
) -> CurrentAuthorityV3 {
    CurrentAuthorityV3 {
        host_authorization_id: value.host_authorization_id.to_bytes(),
        consumer_authorization_id: value.consumer_authorization_id.to_bytes(),
        account_session_id: value.account_session_id.as_str().to_owned(),
        access_epoch: value.access_epoch.0,
        grant_revision: value.grant_revision.0,
        workspace_epoch: value.workspace_epoch.to_bytes(),
        runtime_id: value.runtime_id.to_bytes(),
        runtime_generation: value.runtime_generation.0,
        terminal_membership,
    }
}

fn expected_authority(
    value: &CurrentAuthorityBinding,
    target: &ChannelTarget,
) -> Result<CurrentAuthorityBinding, String> {
    let mut expected = value.clone();
    expected.terminal_membership = match target {
        ChannelTarget::Workspace { .. } => None,
        ChannelTarget::Terminal {
            pane_id,
            session_id,
            session_epoch,
            ..
        } => Some(ChannelTerminalMembership {
            pane_id: *pane_id,
            session_id: *session_id,
            session_epoch: *session_epoch,
            terminal_profile: TerminalProfile::ExperimentalRustGridV1,
        }),
    };
    Ok(expected)
}

fn host_workspace_target(
    target: &ChannelTarget,
) -> (
    lomi_remote_protocol::WorkspaceId,
    lomi_remote_protocol::WorkspaceEpoch,
) {
    match target {
        ChannelTarget::Workspace {
            workspace_id,
            workspace_epoch,
        }
        | ChannelTarget::Terminal {
            workspace_id,
            workspace_epoch,
            ..
        } => (*workspace_id, *workspace_epoch),
    }
}

fn current_account_binding(
    tuple: &RemoteConsumerSessionTuple,
    auth: &crate::auth::AuthController,
) -> Result<crate::auth::controller::RemoteBinding, String> {
    validate_session_tuple(tuple)?;
    let binding = auth.remote_binding()?;
    if !tuple_matches_binding(tuple, &binding) {
        return Err("The account session changed during remote channel setup.".into());
    }
    Ok(binding)
}

fn value_same(left: &Value, right: &Value) -> bool {
    left == right
}

async fn fresh_pairing(
    auth: &crate::auth::AuthController,
    pairing_id: &PairingId,
) -> Result<PairingResult, String> {
    let path = uuid_path(
        HttpRoute::PairingGet,
        "{pairingId}",
        &String::from(*pairing_id),
    );
    let value = remote_request(auth, reqwest::Method::GET, &path, None).await?;
    parse_json(value, "Invalid current pairing response.")
}

async fn fresh_channel_result(
    auth: &crate::auth::AuthController,
    channel_id: &ChannelId,
) -> Result<ChannelResult, String> {
    let path = uuid_path(
        HttpRoute::ChannelGet,
        "{channelId}",
        &String::from(*channel_id),
    );
    let value = remote_request(auth, reqwest::Method::GET, &path, None).await?;
    parse_json(value, "Invalid current channel response.")
}

async fn create_admitted_channel(
    auth: &crate::auth::AuthController,
    inventory: &InventoryResult,
    host: InventoryHost,
    pairing: PairingResult,
    runtime: RuntimeBinding,
    target: ChannelTarget,
    identity: &Identity,
    window_identity_id: &str,
) -> Result<(ChannelId, VerifiedChannelContextV3, SignedBundle, [u8; 32]), String> {
    if host.status != InventoryStatus::Online || host.runtime != runtime {
        return Err("The selected host runtime is no longer online.".into());
    }
    let (workspace_id, workspace_epoch) = host_workspace_target(&target);
    if !current_workspace(&host, workspace_id, workspace_epoch) {
        return Err("The selected workspace is no longer published by this host.".into());
    }
    let PairingResult::Approved {
        pairing_id,
        proposal,
        grant: signed_grant_wire,
        identities,
        authority,
    } = pairing.clone()
    else {
        return Err("Workspace approval is no longer active.".into());
    };
    if proposal.authority.runtime_id != runtime.runtime_id
        || proposal.authority.runtime_generation != runtime.runtime_generation
        || proposal.authority.access_epoch != runtime.host_access_epoch
        || authority.account_session_id != inventory.authorization.account_session_id
        || authority.consumer_authorization_id != inventory.authorization.consumer_authorization_id
        || proposal.identities != identities
        || proposal.authority != authority
    {
        return Err("The approval context no longer matches the selected host.".into());
    }
    let window_id = WindowIdentityId::try_from(window_identity_id.to_owned())
        .map_err(|_| "Invalid consumer window ID.")?;
    let consumer_wire = identity
        .public_bundle()
        .to_wire()
        .map_err(|_| "The consumer identity is invalid.")?;
    let account_id = inventory.authorization.account_id;
    let consumer = verify_consumer_window_identity(
        &consumer_wire,
        account_id.to_bytes(),
        window_id.to_bytes(),
    )
    .map_err(|_| "The saved consumer identity does not match this account window.")?;
    if consumer_wire != identities.consumer
        || canonical_hex(
            &consumer
                .bundle
                .fingerprint()
                .map_err(|_| "The consumer identity is invalid.")?,
        ) != identities.consumer_fingerprint.expose_wire()
    {
        return Err("The approval belongs to a different consumer identity.".into());
    }

    let commitment_pin: [u8; 32] = decode_hex(host.initial_commitment.expose_wire(), 32)?
        .try_into()
        .map_err(|_| "The registered host identity is invalid.")?;
    let signed_commitment = SignedHostKeyCommitment::from_json(
        &serde_json::to_string(&host.signed_commitment)
            .map_err(|_| "The registered host identity is invalid.")?,
    )
    .map_err(|_| "The registered host identity is invalid.")?;
    let host_bundle = verify_registered_host_identity(
        &signed_commitment,
        &commitment_pin,
        account_id.to_bytes(),
        &identities.host,
    )
    .map_err(|_| "The registered host identity could not be verified.")?;
    let inventory_bundle = SignedBundle::from_wire(&host.identity)
        .map_err(|_| "The inventory host identity is invalid.")?;
    if host_bundle != inventory_bundle
        || host_bundle.bundle.subject_id != host.host_id.to_bytes()
        || canonical_hex(
            &host_bundle
                .bundle
                .fingerprint()
                .map_err(|_| "The registered host identity is invalid.")?,
        ) != host.identity_fingerprint.expose_wire()
        || identities.host_fingerprint != host.identity_fingerprint
    {
        return Err("The selected host identity changed after approval.".into());
    }

    let maximum_bytes = decode_hex(proposal.maximum_grant.expose_wire(), 8192)?;
    let maximum_grant =
        GrantV3::from_canonical(&maximum_bytes).map_err(|_| "The approval proposal is invalid.")?;
    let signed_grant = SignedGrantV3::from_wire(&signed_grant_wire)
        .map_err(|_| "The signed approval is invalid.")?;
    let proposal_authority = current_authority(&proposal.authority, None);
    let host_identity_pin = host_bundle
        .bundle
        .fingerprint()
        .map_err(|_| "The registered host identity is invalid.")?;
    let verified_grant = verify_grant_proposal_v3(
        &maximum_grant,
        &signed_grant,
        &host_bundle,
        &host_identity_pin,
        &consumer,
        &decode_hex(identities.consumer_fingerprint.expose_wire(), 32)?
            .try_into()
            .map_err(|_| "The consumer identity fingerprint is invalid.")?,
        &proposal_authority,
        now_unix_seconds()?,
    )
    .map_err(|_| "The signed approval could not be verified against its proposal.")?;
    if verified_grant.account_id != account_id.to_bytes()
        || verified_grant.host_id != host.host_id.to_bytes()
        || verified_grant.workspace_id != workspace_id.to_bytes()
        || verified_grant.workspace_epoch != workspace_epoch.to_bytes()
        || verified_grant.runtime_id != runtime.runtime_id.to_bytes()
        || verified_grant.runtime_generation != runtime.runtime_generation.0
        || verified_grant.access_epoch != runtime.host_access_epoch.0
        || verified_grant.grant_revision != authority.grant_revision.0
    {
        return Err("The approval does not authorize the selected workspace runtime.".into());
    }

    let operation_id =
        OperationId::try_from(new_uuid()?).map_err(|_| "The channel operation ID is invalid.")?;
    let grant_id = GrantId::from_bytes(verified_grant.grant_id)
        .map_err(|_| "The approved grant ID is invalid.")?;
    let create_params = ChannelCreateParams {
        operation_id,
        grant_id,
        grant_revision: authority.grant_revision,
        runtime: runtime.clone(),
        target: target.clone(),
        window_identity_id: None,
    };
    let create_body = serde_json::to_value(create_params)
        .map_err(|_| "The channel request could not be encoded.")?;
    let path = uuid_path(HttpRoute::ChannelCreate, "", "");
    let created_value =
        remote_request(auth, reqwest::Method::POST, &path, Some(&create_body)).await?;
    let created: ChannelCreateResult =
        parse_json(created_value, "The channel request was invalid.")?;
    let channel_id = match created.channel {
        ChannelResult::Pending { channel_id, .. } => channel_id,
        _ => return Err("The secure workspace channel could not be created.".into()),
    };

    let deadline = Instant::now() + Duration::from_secs(30);
    let (inputs, admission_wire) = loop {
        if Instant::now() >= deadline {
            return Err("The host did not admit the secure workspace channel in time.".into());
        }
        match fresh_channel_result(auth, &channel_id).await? {
            ChannelResult::Admitted {
                channel_id: response_id,
                inputs,
                admission,
            } if response_id == channel_id => break (inputs, admission),
            ChannelResult::Pending {
                channel_id: response_id,
                ..
            } if response_id == channel_id => {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            _ => return Err("The secure workspace channel is no longer available.".into()),
        }
    };
    if inputs.grant != signed_grant_wire
        || inputs.identities != identities
        || inputs.authority != expected_authority(&authority, &target)?
    {
        return Err("The channel admission changed the approved authority.".into());
    }
    let context_bytes = decode_hex(inputs.expected_context.expose_wire(), 8192)?;
    let expected_context = ChannelContextV3::from_canonical(&context_bytes)
        .map_err(|_| "The channel context is invalid.")?;
    let expected_json =
        serde_json::to_value(&expected_context).map_err(|_| "The channel context is invalid.")?;
    if !value_same(
        &expected_json,
        &Value::Object(inputs.expected_context_json.clone()),
    ) || expected_context.grant != verified_grant
        || expected_context.channel_id != channel_id.to_bytes()
        || expected_context.grant.workspace_id != workspace_id.to_bytes()
        || expected_context.grant.workspace_epoch != workspace_epoch.to_bytes()
        || expected_context.grant.runtime_id != runtime.runtime_id.to_bytes()
        || expected_context.grant.runtime_generation != runtime.runtime_generation.0
        || expected_context.grant.grant_revision != authority.grant_revision.0
    {
        return Err("The channel context does not match the approved workspace.".into());
    }
    match (
        &expected_context.purpose,
        &target,
        &expected_context.terminal,
    ) {
        (ChannelPurposeV3::Workspace, ChannelTarget::Workspace { .. }, None) => {}
        (ChannelPurposeV3::Terminal, ChannelTarget::Terminal { .. }, Some(terminal))
            if Some(terminal) == terminal_target(&target).as_ref() => {}
        _ => return Err("The channel purpose does not match the selected target.".into()),
    }
    let signed_admission = SignedChannelAdmissionV3::from_wire(&admission_wire)
        .map_err(|_| "The host channel admission is invalid.")?;
    let authority_for_channel = current_authority(&inputs.authority, terminal_target(&target));
    let verified_admission = signed_admission
        .verify(
            &signed_grant,
            &expected_context,
            &host_bundle,
            &host_identity_pin,
            &consumer,
            &decode_hex(identities.consumer_fingerprint.expose_wire(), 32)?
                .try_into()
                .map_err(|_| "The consumer identity fingerprint is invalid.")?,
            &authority_for_channel,
            now_unix_seconds()?,
        )
        .map_err(|_| "The host channel admission could not be verified.")?;
    let _ = pairing_id;
    Ok((
        channel_id,
        verified_admission,
        host_bundle,
        host_identity_pin,
    ))
}

async fn establish_native_relay(
    ticket: RelayTicket,
    channel_id: ChannelId,
    identity: &Identity,
    verified_context: VerifiedChannelContextV3,
    host_bundle: SignedBundle,
    host_identity_pin: [u8; 32],
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        NoiseChannel,
    ),
    String,
> {
    let endpoint = option_env!("LOMI_HOST_RELAY_ENDPOINT")
        .ok_or("Remote relay is not configured for this build.")?;
    let (mut socket, _) = timeout(Duration::from_secs(10), connect_async(endpoint))
        .await
        .map_err(|_| "The secure relay connection timed out.")?
        .map_err(|_| "The secure relay connection could not be opened.")?;
    let admission = WssAdmissionFrame { channel_id, ticket };
    let authentication = serde_json::to_string(&admission)
        .map_err(|_| "The one-use relay admission could not be encoded.")?;
    socket
        .send(Message::Text(authentication.into()))
        .await
        .map_err(|_| "The relay admission could not be sent.")?;
    let mut authenticated = false;
    let mut handshake_ready = false;
    let mut handshake = Some(
        Handshake::new_v3(
            true,
            identity,
            verified_context,
            &host_bundle,
            &host_identity_pin,
            now_unix_seconds()?,
        )
        .map_err(|_| "The approved secure channel could not be initialized.")?,
    );
    let handshake_result: Result<NoiseChannel, String> = timeout(Duration::from_secs(10), async {
        while let Some(message) = socket.next().await {
            match message.map_err(|_| "The relay connection ended during authentication.")? {
                Message::Text(text) if text.len() <= 2048 => {
                    let control: Value = serde_json::from_str(text.as_ref())
                        .map_err(|_| "The relay returned invalid authentication state.")?;
                    let kind = control
                        .get("type")
                        .and_then(Value::as_str)
                        .ok_or("The relay returned invalid authentication state.")?;
                    if kind == "authenticated" && !authenticated && !handshake_ready {
                        authenticated = true;
                    } else if kind == "ready" && authenticated && !handshake_ready {
                        let first = handshake
                            .as_mut()
                            .ok_or("The secure channel handshake is closed.")?
                            .write()
                            .map_err(|_| "The secure channel handshake could not start.")?;
                        socket
                            .send(Message::Binary(first.into()))
                            .await
                            .map_err(|_| "The secure channel handshake could not be sent.")?;
                        handshake_ready = true;
                    } else {
                        return Err("The relay returned unexpected authentication state.".into());
                    }
                }
                Message::Binary(frame)
                    if frame.len() <= 512 && authenticated && handshake_ready =>
                {
                    let mut active = handshake
                        .take()
                        .ok_or("The secure channel handshake is closed.")?;
                    active
                        .read(&frame)
                        .map_err(|_| "The host secure channel identity could not be verified.")?;
                    let response = active
                        .write()
                        .map_err(|_| "The secure channel handshake could not finish.")?;
                    socket
                        .send(Message::Binary(response.into()))
                        .await
                        .map_err(|_| "The secure channel handshake could not be sent.")?;
                    let noise = active
                        .finish()
                        .map_err(|_| "The secure channel handshake did not complete.")?;
                    return Ok(noise);
                }
                Message::Binary(_) => {
                    return Err("The relay sent an unexpected handshake frame.".into())
                }
                _ => return Err("The relay sent an unexpected authentication frame.".into()),
            }
        }
        Err("The relay closed before channel admission completed.".into())
    })
    .await
    .map_err(|_| "The secure relay admission timed out.".to_owned())?;
    let result = handshake_result?;
    if let Some(mut pending) = handshake {
        pending.dispose();
    }
    Ok((socket, result))
}

#[tauri::command]
pub async fn remote_v2_channel_open(
    window: Window,
    webview: Webview,
    app: tauri::AppHandle,
    state: State<'_, RemoteV2>,
    auth: State<'_, crate::auth::AuthController>,
    window_identity_id: String,
    host: InventoryHost,
    pairing: PairingResult,
    runtime: RuntimeBinding,
    target: ChannelTarget,
    events: IpcChannel<RemoteV2NativeEvent>,
) -> Result<String, String> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    let requested_window = WindowIdentityId::try_from(window_identity_id.clone())
        .map_err(|_| "Invalid consumer window ID.")?;
    let inventory = current_inventory(&auth).await?;
    let tuple = session_tuple(&inventory.authorization);
    current_account_binding(&tuple, &auth)?;
    let hosts: Vec<InventoryHost> = inventory.hosts.clone().into();
    let selected_host = hosts
        .iter()
        .find(|candidate| candidate.host_id == host.host_id)
        .ok_or("The selected host is no longer available.")?;
    if !value_same(
        &serde_json::to_value(selected_host).map_err(|_| "Invalid current host state.")?,
        &serde_json::to_value(&host).map_err(|_| "Invalid selected host state.")?,
    ) || selected_host.status != InventoryStatus::Online
        || selected_host.runtime != runtime
    {
        return Err("The selected host changed. Refresh the workspace list and try again.".into());
    }
    let pairing_id = match &pairing {
        PairingResult::Approved { pairing_id, .. } => pairing_id,
        _ => return Err("The selected workspace approval is no longer active.".into()),
    };
    let current_pairing = fresh_pairing(&auth, pairing_id).await?;
    if !value_same(
        &serde_json::to_value(&current_pairing).map_err(|_| "Invalid current approval state.")?,
        &serde_json::to_value(&pairing).map_err(|_| "Invalid selected approval state.")?,
    ) {
        return Err("The selected workspace approval changed. Refresh and try again.".into());
    }
    let pairing_authority = match &current_pairing {
        PairingResult::Approved { authority, .. } => authority.clone(),
        _ => return Err("The selected workspace approval is no longer active.".into()),
    };
    let record = {
        let path = records_path(&app)?;
        let _guard = state
            .records_lock
            .lock()
            .map_err(|_| "Remote window storage is unavailable.")?;
        let records = read_records(&path)?;
        let key = record_key(window.label(), &tuple);
        records
            .records
            .get(&key)
            .filter(|record| {
                record.window_identity_id == window_identity_id
                    && record.stage == RegistrationStage::Registered
                    && !record.revocation_confirmed
            })
            .cloned()
            .ok_or("This account window has not been registered for remote access.")?
    };
    if WindowIdentityId::try_from(record.window_identity_id.clone())
        .map_err(|_| "Invalid saved consumer window ID.")?
        != requested_window
    {
        return Err("The saved consumer identity does not match this window.".into());
    }
    let identity = identity_from_record(&record)
        .map_err(|_| "The consumer identity is unavailable in the system credential store.")?;
    let identity_fingerprint = canonical_hex(
        &identity
            .public_bundle()
            .bundle
            .fingerprint()
            .map_err(|_| "The saved consumer identity is invalid.")?,
    );
    if record.identity_fingerprint.as_deref() != Some(identity_fingerprint.as_str()) {
        return Err(
            "The saved consumer identity changed. Register this account window again.".into(),
        );
    }
    let (channel_id, verified_context, host_bundle, host_identity_pin) = create_admitted_channel(
        &auth,
        &inventory,
        selected_host.clone(),
        current_pairing,
        runtime.clone(),
        target.clone(),
        &identity,
        &window_identity_id,
    )
    .await?;
    current_account_binding(&tuple, &auth)?;
    let ticket_params = ChannelTicketParams {
        runtime: runtime.clone(),
        window_identity_id: None,
    };
    let ticket_body = serde_json::to_value(ticket_params)
        .map_err(|_| "The relay ticket request could not be encoded.")?;
    let path = uuid_path(
        HttpRoute::ChannelTicket,
        "{channelId}",
        &String::from(channel_id),
    );
    let ticket_value =
        remote_request(&auth, reqwest::Method::POST, &path, Some(&ticket_body)).await?;
    let ticket: ChannelTicketResult =
        parse_json(ticket_value, "The relay ticket response is invalid.")?;
    if ticket.channel_id != channel_id || ticket.expires_in_ms.0 == 0 {
        return Err("The relay ticket does not match the approved channel.".into());
    }
    current_account_binding(&tuple, &auth)?;
    let (socket, noise) = establish_native_relay(
        ticket.ticket,
        channel_id,
        &identity,
        verified_context,
        host_bundle,
        host_identity_pin,
    )
    .await?;
    current_account_binding(&tuple, &auth)?;

    let handle_id = new_uuid()?;
    let route = channel_id.to_bytes();
    let (sender, receiver) = mpsc::channel(64);
    let terminal_context = match target {
        ChannelTarget::Terminal {
            pane_id,
            session_id,
            session_epoch,
            workspace_id,
            workspace_epoch,
        } => Some(TerminalContext {
            workspace: lomi_remote_protocol::WorkspaceContext {
                grant_revision: pairing_authority.grant_revision,
                host_access_epoch: runtime.host_access_epoch,
                runtime_generation: runtime.runtime_generation,
                runtime_id: runtime.runtime_id,
                workspace_epoch,
                workspace_id,
            },
            pane_id,
            session_id,
            session_epoch,
            profile: TerminalProfile::ExperimentalRustGridV1,
        }),
        ChannelTarget::Workspace { .. } => None,
    };
    let terminal_control = if let Some(expected_terminal) = terminal_context {
        let (control_sender, control_receiver) = mpsc::channel(128);
        let control = NativeTerminalControl {
            sender: control_sender.clone(),
            focused: Arc::new(AtomicBool::new(false)),
            focus_generation: Arc::new(AtomicU64::new(0)),
            queued_bytes: Arc::new(AtomicUsize::new(0)),
        };
        let remote_sender = sender.clone();
        let focused = control.focused.clone();
        let generation = control.focus_generation.clone();
        let queued_bytes = control.queued_bytes.clone();
        tokio::spawn(native_terminal_control_loop(
            expected_terminal,
            remote_sender,
            focused,
            generation,
            queued_bytes,
            control_receiver,
        ));
        Some((control, control_sender))
    } else {
        None
    };
    let (control, control_sender) = terminal_control.map_or((None, None), |(control, sender)| {
        (Some(control), Some(sender))
    });
    {
        let mut channels = state
            .channels
            .lock()
            .map_err(|_| "Remote channel state is unavailable.")?;
        if channels.len() >= 32 {
            return Err("Too many remote channels are active in this window.".into());
        }
        if channels
            .insert(
                handle_id.clone(),
                ActiveRemoteChannel {
                    window_label: window.label().to_owned(),
                    window_identity_id,
                    sender: sender.clone(),
                    terminal_control: control,
                },
            )
            .is_some()
        {
            return Err("Remote channel handle collision.".into());
        }
    }
    tokio::spawn(remote_channel_task(
        socket,
        noise,
        route,
        events,
        receiver,
        control_sender,
    ));
    Ok(handle_id)
}

struct PendingNativeRpc {
    request: RpcRequest,
    completed: oneshot::Sender<Result<RpcResponse, String>>,
}

fn logical_message_kind(kind: u8) -> Result<MessageKind, String> {
    match kind {
        1 => Ok(MessageKind::RpcRequest),
        2 => Ok(MessageKind::RpcResponse),
        3 => Ok(MessageKind::WorkspaceFrame),
        4 => Ok(MessageKind::TerminalFrame),
        5 => Ok(MessageKind::FileWatchFrame),
        _ => Err("Invalid Remote v2 message kind.".into()),
    }
}

fn now_millis(started: Instant) -> Result<u64, String> {
    u64::try_from(started.elapsed().as_millis())
        .map_err(|_| "Remote channel clock overflowed.".into())
}

fn response_request_id(response: &RpcResponse) -> &RequestId {
    match response {
        RpcResponse::Result { request_id, .. } | RpcResponse::Error { request_id, .. } => {
            request_id
        }
    }
}

fn zeroize_rpc_request(request: &mut RpcRequest) {
    if let RpcCall::TerminalInput(params) = &mut request.call {
        params.bytes.0.zeroize();
    }
}

async fn send_logical_message(
    sink: &mut (impl Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    noise: &mut NoiseChannel,
    encoder: &mut MessageEncoder,
    route: [u8; 16],
    kind: MessageKind,
    mut bytes: Vec<u8>,
    focus_generation: Option<(&AtomicU64, u64)>,
) -> Result<(), String> {
    if focus_generation
        .is_some_and(|(current, expected)| current.load(Ordering::Acquire) != expected)
    {
        bytes.zeroize();
        return Err("TERMINAL_FOCUS_CHANGED".into());
    }
    let mut sent_any = false;
    let send_result = async {
        encoder
            .begin(kind, std::mem::take(&mut bytes))
            .map_err(|_| "Remote message framing failed.")?;
        loop {
            if focus_generation
                .is_some_and(|(current, expected)| current.load(Ordering::Acquire) != expected)
            {
                if sent_any {
                    return Err("FOCUS_CHANGED_DURING_FRAGMENT".into());
                }
                encoder.abort();
                return Err("TERMINAL_ENCODER_ABORTED".into());
            }
            let Some(frame) = encoder
                .next_frame()
                .map_err(|_| "Remote message framing failed.")?
            else {
                break;
            };
            let mut frame = frame;
            let encrypted = noise.seal_binary(route, &frame);
            frame.zeroize();
            let ciphertext = encrypted.map_err(|_| "Remote channel encryption failed.")?;
            sink.send(Message::Binary(ciphertext.into()))
                .await
                .map_err(|_| "Remote channel send failed.")?;
            sent_any = true;
        }
        Ok(())
    }
    .await;
    bytes.zeroize();
    send_result
}

async fn native_rpc(
    sender: &mpsc::Sender<RemoteChannelCommand>,
    call: RpcCall,
    timeout_ms: u32,
    focus_generation: Option<(Arc<AtomicU64>, u64)>,
) -> Result<RpcResponse, String> {
    let request_id =
        RequestId::try_from(new_uuid()?).map_err(|_| "Remote request ID is invalid.")?;
    let request_timeout =
        TimeoutMs::try_from(timeout_ms as u64).map_err(|_| "Remote request timeout is invalid.")?;
    let request = RpcRequest {
        v: ApplicationMajor::try_from(2).map_err(|_| "Remote protocol version is invalid.")?,
        r#type: RequestKind::Request,
        request_id,
        timeout_ms: request_timeout,
        call,
    };
    let (completed, receiver) = oneshot::channel();
    sender
        .send(RemoteChannelCommand::Rpc {
            request,
            focus_generation,
            completed,
        })
        .await
        .map_err(|_| "Remote channel is closed.")?;
    match timeout(Duration::from_millis(timeout_ms as u64 + 1000), receiver).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("Remote channel closed before the response arrived.".into()),
        Err(_) => {
            let _ = sender.send(RemoteChannelCommand::Close).await;
            Err("REMOTE_OPERATION_OUTCOME_UNKNOWN".into())
        }
    }
}

async fn remote_channel_task(
    mut socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut noise: NoiseChannel,
    route: [u8; 16],
    mut events: IpcChannel<RemoteV2NativeEvent>,
    mut commands: mpsc::Receiver<RemoteChannelCommand>,
    control_sender: Option<mpsc::Sender<NativeTerminalControlCommand>>,
) {
    let started = Instant::now();
    let mut encoder = MessageEncoder::new();
    let mut assembler = MessageAssembler::new();
    let mut js_requests: HashMap<String, RpcRequest> = HashMap::new();
    let mut native_requests: HashMap<String, PendingNativeRpc> = HashMap::new();
    let (mut sink, mut stream) = socket.split();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let reason = loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(RemoteChannelCommand::Close) | None => break "CHANNEL_CLOSED",
                    Some(RemoteChannelCommand::Send { kind, mut bytes, completed }) => {
                        let result = async {
                            let message_kind = logical_message_kind(kind)?;
                            if message_kind == MessageKind::RpcRequest {
                                let mut request = lomi_remote_protocol::decode_request(&bytes).map_err(|_| "Invalid Remote v2 request.")?;
                                if matches!(&request.call, RpcCall::TerminalClaim(_) | RpcCall::TerminalRenew(_) | RpcCall::TerminalRelease(_) | RpcCall::TerminalInput(_) | RpcCall::TerminalResize(_)) {
                                    zeroize_rpc_request(&mut request);
                                    return Err("Terminal control is available only through the native control session.".into());
                                }
                                let id = String::from(request.request_id.clone());
                                if js_requests.len() >= 32 || js_requests.contains_key(&id) || native_requests.contains_key(&id) {
                                    return Err("Remote request limit or ID conflict.".into());
                                }
                                js_requests.insert(id, request);
                            } else if message_kind == MessageKind::FileWatchFrame {
                                lomi_remote_protocol::decode_watch_frame(&bytes).map_err(|_| "Invalid file-watch frame.")?;
                            } else if message_kind == MessageKind::WorkspaceFrame {
                                if !matches!(lomi_remote_protocol::decode_workspace_frame(&bytes), Ok(lomi_remote_protocol::streams::WorkspaceStreamFrame::Ack(_))) {
                                    return Err("Only workspace stream acknowledgements are accepted from the consumer.".into());
                                }
                            } else {
                                return Err("This desktop channel does not accept raw non-RPC messages.".into());
                            }
                            send_logical_message(&mut sink, &mut noise, &mut encoder, route, message_kind, std::mem::take(&mut *bytes), None).await
                        }.await;
                        let close_after = result.is_err();
                        let _ = completed.send(result);
                        if close_after { break "REMOTE_CHANNEL_PROTOCOL_ERROR"; }
                    }
                    Some(RemoteChannelCommand::Rpc { mut request, focus_generation, completed }) => {
                        let id = String::from(request.request_id.clone());
                        let mut payload = match serde_json::to_vec(&request) {
                            Ok(payload) => payload,
                            Err(_) => { let _ = completed.send(Err("Remote request could not be encoded.".into())); zeroize_rpc_request(&mut request); break "REMOTE_CHANNEL_PROTOCOL_ERROR"; }
                        };
                        let valid = lomi_remote_protocol::decode_request(&payload).is_ok();
                        if !valid || native_requests.len() >= 32 || native_requests.contains_key(&id) || js_requests.contains_key(&id) {
                            payload.zeroize();
                            zeroize_rpc_request(&mut request);
                            let _ = completed.send(Err("Remote request limit or validation failed.".into()));
                            break "REMOTE_CHANNEL_PROTOCOL_ERROR";
                        }
                        native_requests.insert(id.clone(), PendingNativeRpc { request, completed });
                        let send = send_logical_message(&mut sink, &mut noise, &mut encoder, route, MessageKind::RpcRequest, payload, focus_generation.as_ref().map(|(generation, value)| (&**generation, *value))).await;
                        if send.is_err() {
                            encoder.abort();
                            let reason = send.unwrap_err();
                            let close_after = reason != "TERMINAL_FOCUS_CHANGED";
                            if let Some(mut pending) = native_requests.remove(&id) {
                                zeroize_rpc_request(&mut pending.request);
                                let _ = pending.completed.send(Err(reason));
                            }
                            if close_after { break "REMOTE_CHANNEL_SEND_FAILED"; }
                        }
                    }
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Binary(frame))) if frame.len() <= lomi_remote_protocol::message_framing::MAX_TRANSPORT_FRAME_LEN => {
                        let plaintext = match noise.open_binary(route, &frame) {
                            Ok(value) => value,
                            Err(_) => break "REMOTE_CHANNEL_AUTHENTICATION_FAILED",
                        };
                        let completed = match assembler.push(&plaintext, now_millis(started).unwrap_or(u64::MAX)) {
                            Ok(value) => value,
                            Err(_) => break "REMOTE_CHANNEL_FRAMING_FAILED",
                        };
                        let Some(CompletedMessage { kind, id, bytes }) = completed else { continue; };
                        let result = match kind {
                            MessageKind::RpcResponse => {
                                let response = match lomi_remote_protocol::decode_response(&bytes) {
                                    Ok(value) => value,
                                    Err(_) => break "REMOTE_CHANNEL_RPC_INVALID",
                                };
                                let request_id = String::from(response_request_id(&response).clone());
                                if let Some(mut pending) = native_requests.remove(&request_id) {
                                    match lomi_remote_protocol::decode_response_for(&bytes, &pending.request) {
                                        Ok(checked) => { zeroize_rpc_request(&mut pending.request); let _ = pending.completed.send(Ok(checked)); }
                                        Err(_) => { zeroize_rpc_request(&mut pending.request); let _ = pending.completed.send(Err("Remote response validation failed.".into())); break "REMOTE_CHANNEL_RPC_INVALID"; }
                                    }
                                } else if let Some(request) = js_requests.remove(&request_id) {
                                    if lomi_remote_protocol::decode_response_for(&bytes, &request).is_err() { break "REMOTE_CHANNEL_RPC_INVALID"; }
                                    let _ = events.send(RemoteV2NativeEvent::Message { kind: 2, id: id.to_string(), bytes });
                                } else {
                                    break "REMOTE_CHANNEL_RPC_UNMATCHED";
                                }
                                Ok(())
                            }
                            MessageKind::WorkspaceFrame => {
                                if lomi_remote_protocol::decode_workspace_frame(&bytes).is_err() { break "REMOTE_CHANNEL_STREAM_INVALID"; }
                                events.send(RemoteV2NativeEvent::Message { kind: 3, id: id.to_string(), bytes }).map_err(|_| "REMOTE_VIEW_CLOSED".to_owned())
                            }
                            MessageKind::TerminalFrame => {
                                if lomi_remote_protocol::decode_terminal_frame(&bytes).is_err() { break "REMOTE_CHANNEL_STREAM_INVALID"; }
                                events.send(RemoteV2NativeEvent::Message { kind: 4, id: id.to_string(), bytes }).map_err(|_| "REMOTE_VIEW_CLOSED".to_owned())
                            }
                            MessageKind::FileWatchFrame => {
                                if lomi_remote_protocol::decode_watch_frame(&bytes).is_err() { break "REMOTE_CHANNEL_STREAM_INVALID"; }
                                events.send(RemoteV2NativeEvent::Message { kind: 5, id: id.to_string(), bytes }).map_err(|_| "REMOTE_VIEW_CLOSED".to_owned())
                            }
                            MessageKind::RpcRequest => break "REMOTE_CHANNEL_UNEXPECTED_REQUEST",
                        };
                        if result.is_err() { break "REMOTE_VIEW_CLOSED"; }
                    }
                    Some(Ok(Message::Text(text))) if text.len() <= 2048 => break "REMOTE_CHANNEL_UNEXPECTED_CONTROL",
                    Some(Ok(Message::Ping(payload))) => {
                        if sink.send(Message::Pong(payload)).await.is_err() { break "REMOTE_CHANNEL_SEND_FAILED"; }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => break "REMOTE_CHANNEL_CLOSED",
                    Some(Ok(_)) => break "REMOTE_CHANNEL_FRAME_INVALID",
                    Some(Err(_)) => break "REMOTE_CHANNEL_TRANSPORT_FAILED",
                }
            }
            _ = tick.tick() => {
                if assembler.expire(now_millis(started).unwrap_or(u64::MAX)).is_err() { break "REMOTE_CHANNEL_ASSEMBLY_TIMEOUT"; }
            }
        }
    };
    for mut pending in native_requests.into_values() {
        zeroize_rpc_request(&mut pending.request);
        let _ = pending.completed.send(Err(
            "Remote channel closed before the response arrived.".into()
        ));
    }
    commands.close();
    while let Ok(command) = commands.try_recv() {
        match command {
            RemoteChannelCommand::Send {
                mut bytes,
                completed,
                ..
            } => {
                bytes.zeroize();
                let _ = completed.send(Err(
                    "Remote channel closed before the send completed.".into()
                ));
            }
            RemoteChannelCommand::Rpc {
                mut request,
                completed,
                ..
            } => {
                zeroize_rpc_request(&mut request);
                let _ = completed.send(Err(
                    "Remote channel closed before the request completed.".into()
                ));
            }
            RemoteChannelCommand::Close => {}
        }
    }
    let _ = sink.send(Message::Close(None)).await;
    noise.dispose();
    if let Some(control) = control_sender {
        let _ = control.send(NativeTerminalControlCommand::Close).await;
    }
    let _ = events.send(RemoteV2NativeEvent::Closed {
        code: reason.into(),
    });
}

struct NativeTerminalLease {
    binding: LeaseBinding,
    expires_at: Instant,
    next_input_id: u64,
    next_input_seq: u64,
    next_resize_seq: u64,
}

async fn native_terminal_control_loop(
    expected_terminal: TerminalContext,
    sender: mpsc::Sender<RemoteChannelCommand>,
    focused: Arc<AtomicBool>,
    focus_generation: Arc<AtomicU64>,
    queued_bytes: Arc<AtomicUsize>,
    mut commands: mpsc::Receiver<NativeTerminalControlCommand>,
) {
    const MAX_TTL_MS: u32 = 5_000;
    const RENEW_EARLY_MS: u32 = 2_500;
    let mut lease: Option<NativeTerminalLease> = None;
    loop {
        let renewal_at = lease.as_ref().and_then(|active| {
            focused.load(Ordering::Acquire).then(|| {
                active
                    .expires_at
                    .checked_sub(Duration::from_millis(RENEW_EARLY_MS as u64))
                    .unwrap_or_else(Instant::now)
            })
        });
        let command = if let Some(due) = renewal_at {
            tokio::select! {
                biased;
                command = commands.recv() => command,
                _ = sleep_until(TokioInstant::from_std(due)) => {
                    if let Some(active) = lease.as_mut() {
                        let generation = focus_generation.load(Ordering::Acquire);
                        let request = RpcCall::TerminalRenew(TerminalLeaseParams {
                            terminal: expected_terminal.clone(), lease: active.binding.clone(),
                        });
                        let renewed = match native_rpc(&sender, request, 1_500, Some((focus_generation.clone(), generation))).await {
                            Ok(RpcResponse::Result { result: RpcResult::TerminalRenew(renewed), .. })
                                if renewed.terminal == expected_terminal
                                    && renewed.lease.version == active.binding.version
                                    && renewed.expires_in_ms.get() <= MAX_TTL_MS as u64
                                    && renewed.expires_in_ms.get() > 0
                                    && renewed.next_input_id.0 == active.next_input_id
                                    && renewed.next_input_seq.0.0 == active.next_input_seq
                                    && renewed.next_resize_seq.0.0 == active.next_resize_seq
                                    && focused.load(Ordering::Acquire)
                                    && focus_generation.load(Ordering::Acquire) == generation => {
                                active.expires_at = Instant::now() + Duration::from_millis(renewed.expires_in_ms.get());
                                true
                            }
                            _ => false,
                        };
                        if !renewed {
                            // Never keep retrying a lost or rejected lease. A later
                            // claim must be an explicit user action.
                            lease = None;
                        }
                    }
                    continue;
                }
            }
        } else {
            commands.recv().await
        };
        let Some(command) = command else {
            break;
        };
        match command {
            NativeTerminalControlCommand::Close => {
                let _ = advance_generation(&focus_generation);
                focused.store(false, Ordering::Release);
                break;
            }
            NativeTerminalControlCommand::Claim {
                params,
                focus_generation: generation,
                completed,
            } => {
                let result = async {
                    if params.terminal != expected_terminal {
                        return Err("TERMINAL_CONTEXT_MISMATCH".into());
                    }
                    if !focused.load(Ordering::Acquire)
                        || focus_generation.load(Ordering::Acquire) != generation
                    {
                        return Err("TERMINAL_FOCUS_CHANGED".into());
                    }
                    if lease.is_some() {
                        return Err("TERMINAL_CONTROL_ALREADY_CLAIMED".into());
                    }
                    let response = native_rpc(
                        &sender,
                        RpcCall::TerminalClaim(params),
                        10_000,
                        Some((focus_generation.clone(), generation)),
                    )
                    .await?;
                    let response = match response {
                        RpcResponse::Result {
                            result: RpcResult::TerminalClaim(value),
                            ..
                        } => value,
                        RpcResponse::Error { .. } => return Err("TERMINAL_CLAIM_DENIED".into()),
                        _ => return Err("INVALID_TERMINAL_CLAIM_RESPONSE".into()),
                    };
                    if response.terminal != expected_terminal
                        || response.expires_in_ms.get() == 0
                        || response.expires_in_ms.get() > MAX_TTL_MS as u64
                    {
                        return Err("INVALID_TERMINAL_LEASE".into());
                    }
                    let active = NativeTerminalLease {
                        binding: response.lease.clone(),
                        expires_at: Instant::now()
                            + Duration::from_millis(response.expires_in_ms.get()),
                        next_input_id: response.next_input_id.0,
                        next_input_seq: response.next_input_seq.0 .0,
                        next_resize_seq: response.next_resize_seq.0 .0,
                    };
                    if !focused.load(Ordering::Acquire)
                        || focus_generation.load(Ordering::Acquire) != generation
                    {
                        let _ = native_rpc(
                            &sender,
                            RpcCall::TerminalRelease(TerminalLeaseParams {
                                terminal: expected_terminal.clone(),
                                lease: active.binding,
                            }),
                            1_500,
                            None,
                        )
                        .await;
                        return Err("TERMINAL_FOCUS_CHANGED".into());
                    }
                    lease = Some(active);
                    Ok(())
                }
                .await;
                let _ = completed.send(result);
            }
            NativeTerminalControlCommand::Input {
                mut bytes,
                focus_generation: generation,
                completed,
            } => {
                queued_bytes.fetch_sub(bytes.len(), Ordering::AcqRel);
                let result = async {
                    if bytes.is_empty() || bytes.len() > 4_096 {
                        return Err("INVALID_TERMINAL_INPUT".into());
                    }
                    if !focused.load(Ordering::Acquire)
                        || focus_generation.load(Ordering::Acquire) != generation
                    {
                        return Err("TERMINAL_FOCUS_CHANGED".into());
                    }
                    let active = lease.as_mut().ok_or("TERMINAL_LEASE_REQUIRED")?;
                    if Instant::now() >= active.expires_at {
                        lease = None;
                        return Err("TERMINAL_LEASE_EXPIRED".into());
                    }
                    if active.next_input_id == u64::MAX || active.next_input_seq == u64::MAX {
                        return Err("TERMINAL_SEQUENCE_EXHAUSTED".into());
                    }
                    let input_id = DecimalU64(active.next_input_id);
                    let input_seq = InputSeq(DecimalU64(active.next_input_seq));
                    let submitted = bytes.len();
                    let call = RpcCall::TerminalInput(TerminalInputParams {
                        terminal: expected_terminal.clone(),
                        lease: active.binding.clone(),
                        input_id,
                        input_seq,
                        bytes: BoundedList(std::mem::take(&mut *bytes)),
                    });
                    // The operation is one-shot. A response timeout closes the channel and no sequence is replayed.
                    let response = native_rpc(
                        &sender,
                        call,
                        10_000,
                        Some((focus_generation.clone(), generation)),
                    )
                    .await?;
                    let receipt = match response {
                        RpcResponse::Result {
                            result: RpcResult::TerminalInput(value),
                            ..
                        } => value,
                        RpcResponse::Error { .. } => {
                            return Err("TERMINAL_INPUT_REJECTED".into());
                        }
                        _ => {
                            return Err("INVALID_TERMINAL_INPUT_RECEIPT".into());
                        }
                    };
                    if receipt.accepted_bytes().get() != submitted as u64 {
                        let _ = sender.send(RemoteChannelCommand::Close).await;
                        return Err("TERMINAL_INPUT_OUTCOME_UNKNOWN".into());
                    }
                    active.next_input_id += 1;
                    active.next_input_seq += 1;
                    if !focused.load(Ordering::Acquire)
                        || focus_generation.load(Ordering::Acquire) != generation
                    {
                        return Err("TERMINAL_FOCUS_CHANGED".into());
                    }
                    Ok(())
                }
                .await;
                bytes.zeroize();
                let _ = completed.send(result);
            }
            NativeTerminalControlCommand::Resize {
                columns,
                rows,
                focus_generation: generation,
                completed,
            } => {
                let result = async {
                    if !focused.load(Ordering::Acquire)
                        || focus_generation.load(Ordering::Acquire) != generation
                    {
                        return Err("TERMINAL_FOCUS_CHANGED".into());
                    }
                    let active = lease.as_mut().ok_or("TERMINAL_LEASE_REQUIRED")?;
                    if Instant::now() >= active.expires_at {
                        lease = None;
                        return Err("TERMINAL_LEASE_EXPIRED".into());
                    }
                    if active.next_resize_seq == u64::MAX {
                        return Err("TERMINAL_SEQUENCE_EXHAUSTED".into());
                    }
                    let call = RpcCall::TerminalResize(TerminalResizeParams {
                        terminal: expected_terminal.clone(),
                        lease: active.binding.clone(),
                        resize_seq: ResizeSeq(DecimalU64(active.next_resize_seq)),
                        columns: TerminalDimension::try_from(columns)
                            .map_err(|_| "INVALID_TERMINAL_SIZE")?,
                        rows: TerminalDimension::try_from(rows)
                            .map_err(|_| "INVALID_TERMINAL_SIZE")?,
                    });
                    let response = native_rpc(
                        &sender,
                        call,
                        10_000,
                        Some((focus_generation.clone(), generation)),
                    )
                    .await?;
                    match response {
                        RpcResponse::Result {
                            result: RpcResult::TerminalResize(value),
                            ..
                        } if value.terminal == expected_terminal
                            && value.resize_seq.0 .0 == active.next_resize_seq
                            && value.columns.0 == columns
                            && value.rows.0 == rows =>
                        {
                            active.next_resize_seq += 1;
                            if !focused.load(Ordering::Acquire)
                                || focus_generation.load(Ordering::Acquire) != generation
                            {
                                Err("TERMINAL_FOCUS_CHANGED".into())
                            } else {
                                Ok(())
                            }
                        }
                        RpcResponse::Error { .. } => Err("TERMINAL_RESIZE_REJECTED".into()),
                        _ => Err("INVALID_TERMINAL_RESIZE_RECEIPT".into()),
                    }
                }
                .await;
                let _ = completed.send(result);
            }
            NativeTerminalControlCommand::Release { completed } => {
                let result = if let Some(active) = lease.take() {
                    match native_rpc(
                        &sender,
                        RpcCall::TerminalRelease(TerminalLeaseParams {
                            terminal: expected_terminal.clone(),
                            lease: active.binding,
                        }),
                        1_500,
                        None,
                    )
                    .await
                    {
                        Ok(RpcResponse::Result {
                            result: RpcResult::TerminalRelease(_),
                            ..
                        }) => Ok(()),
                        Ok(RpcResponse::Error { .. }) => Err("TERMINAL_RELEASE_REJECTED".into()),
                        Ok(_) => Err("INVALID_TERMINAL_RELEASE_RESPONSE".into()),
                        Err(error) => Err(error),
                    }
                } else {
                    Ok(())
                };
                let _ = completed.send(result);
            }
        }
    }
    if let Some(active) = lease.take() {
        let _ = native_rpc(
            &sender,
            RpcCall::TerminalRelease(TerminalLeaseParams {
                terminal: expected_terminal,
                lease: active.binding,
            }),
            1_500,
            None,
        )
        .await;
    }
}

#[derive(Clone)]
struct RemoteChannelAccess {
    sender: mpsc::Sender<RemoteChannelCommand>,
    terminal_control: Option<NativeTerminalControl>,
}

fn remote_channel_access(
    state: &RemoteV2,
    window_label: &str,
    channel_handle_id: &str,
) -> Result<RemoteChannelAccess, String> {
    let channels = state
        .channels
        .lock()
        .map_err(|_| "Remote channel state is unavailable.")?;
    let channel = channels
        .get(channel_handle_id)
        .filter(|channel| channel.window_label == window_label)
        .ok_or("This remote channel is not available in this window.")?;
    Ok(RemoteChannelAccess {
        sender: channel.sender.clone(),
        terminal_control: channel.terminal_control.clone(),
    })
}

fn trusted_remote_channel(
    window: &Window,
    webview: &Webview,
    state: &RemoteV2,
    channel_handle_id: &str,
) -> Result<RemoteChannelAccess, String> {
    crate::shared_host::trusted_full_window(window, webview)?;
    if !valid_uuid(channel_handle_id) {
        return Err("Invalid remote channel handle.".into());
    }
    remote_channel_access(state, window.label(), channel_handle_id)
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn advance_focus_generation(control: &NativeTerminalControl) -> Result<u64, String> {
    advance_generation(&control.focus_generation)
}

fn advance_generation(generation: &AtomicU64) -> Result<u64, String> {
    loop {
        let current = generation.load(Ordering::Acquire);
        let next = current
            .checked_add(1)
            .ok_or("Terminal focus generation is exhausted.")?;
        if generation
            .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(next);
        }
    }
}

async fn await_control_result(
    receiver: oneshot::Receiver<Result<(), String>>,
) -> Result<(), String> {
    match timeout(Duration::from_secs(12), receiver).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("Remote terminal control stopped before the operation completed.".into()),
        Err(_) => Err("Remote terminal control operation outcome is unknown.".into()),
    }
}

#[tauri::command]
pub async fn remote_v2_channel_send(
    window: Window,
    webview: Webview,
    state: State<'_, RemoteV2>,
    channel_handle_id: String,
    kind: u8,
    mut bytes: Vec<u8>,
) -> Result<(), String> {
    let access = match trusted_remote_channel(&window, &webview, &state, &channel_handle_id) {
        Ok(access) => access,
        Err(error) => {
            bytes.zeroize();
            return Err(error);
        }
    };
    if bytes.is_empty() || bytes.len() > lomi_remote_protocol::message_framing::MAX_MESSAGE_LEN {
        bytes.zeroize();
        return Err("Remote channel message size is invalid.".into());
    }
    if !matches!(kind, 1 | 3 | 5) {
        bytes.zeroize();
        return Err("This remote message kind is unavailable to the webview.".into());
    }
    let (completed, receiver) = oneshot::channel();
    match access.sender.try_send(RemoteChannelCommand::Send {
        kind,
        bytes: zeroize::Zeroizing::new(bytes),
        completed,
    }) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(mut command)) => {
            if let RemoteChannelCommand::Send { bytes, .. } = &mut command {
                bytes.zeroize();
            }
            return Err("Remote channel send queue is full.".into());
        }
        Err(mpsc::error::TrySendError::Closed(mut command)) => {
            if let RemoteChannelCommand::Send { bytes, .. } = &mut command {
                bytes.zeroize();
            }
            return Err("Remote channel is closed.".into());
        }
    }
    match timeout(Duration::from_secs(12), receiver).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("Remote channel closed before the send completed.".into()),
        Err(_) => Err("Remote channel send outcome is unknown.".into()),
    }
}

#[tauri::command]
pub async fn remote_v2_channel_close(
    window: Window,
    webview: Webview,
    state: State<'_, RemoteV2>,
    channel_handle_id: String,
) -> Result<(), String> {
    crate::shared_host::trusted_full_window(&window, &webview)?;
    if !valid_uuid(&channel_handle_id) {
        return Err("Invalid remote channel handle.".into());
    }
    let channel = {
        let mut channels = state
            .channels
            .lock()
            .map_err(|_| "Remote channel state is unavailable.")?;
        match channels.get(&channel_handle_id) {
            Some(channel) if channel.window_label == window.label() => {
                channels.remove(&channel_handle_id)
            }
            Some(_) => return Err("This remote channel is not available in this window.".into()),
            None => return Ok(()),
        }
    };
    if let Some(channel) = channel {
        if let Some(control) = channel.terminal_control {
            if advance_focus_generation(&control).is_err() {
                control.focused.store(false, Ordering::Release);
                let _ = channel.sender.send(RemoteChannelCommand::Close).await;
                let _ = control
                    .sender
                    .send(NativeTerminalControlCommand::Close)
                    .await;
                return Err(
                    "Terminal focus generation is exhausted; the channel was closed.".into(),
                );
            }
            control.focused.store(false, Ordering::Release);
            let (completed, receiver) = oneshot::channel();
            let _ = control
                .sender
                .send(NativeTerminalControlCommand::Release { completed })
                .await;
            let _ = timeout(Duration::from_secs(2), receiver).await;
            let _ = control
                .sender
                .send(NativeTerminalControlCommand::Close)
                .await;
        }
        let _ = channel.sender.send(RemoteChannelCommand::Close).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn remote_v2_terminal_claim(
    window: Window,
    webview: Webview,
    state: State<'_, RemoteV2>,
    channel_handle_id: String,
    params: TerminalClaimParams,
) -> Result<(), String> {
    let access = trusted_remote_channel(&window, &webview, &state, &channel_handle_id)?;
    let control = access
        .terminal_control
        .ok_or("Terminal control is unavailable on this channel.")?;
    if !control.focused.load(Ordering::Acquire) {
        return Err("Focus the terminal before enabling input control.".into());
    }
    let generation = control.focus_generation.load(Ordering::Acquire);
    let (completed, receiver) = oneshot::channel();
    control
        .sender
        .try_send(NativeTerminalControlCommand::Claim {
            params,
            focus_generation: generation,
            completed,
        })
        .map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => {
                "Remote terminal control queue is full.".to_owned()
            }
            mpsc::error::TrySendError::Closed(_) => "Remote terminal control is closed.".to_owned(),
        })?;
    await_control_result(receiver).await
}

fn reserve_control_bytes(control: &NativeTerminalControl, length: usize) -> Result<(), String> {
    loop {
        let current = control.queued_bytes.load(Ordering::Acquire);
        let next = current
            .checked_add(length)
            .ok_or("Remote terminal input queue is full.")?;
        if next > 16 * 1024 {
            return Err("Remote terminal input queue is full.".into());
        }
        if control
            .queued_bytes
            .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Ok(());
        }
    }
}

#[tauri::command]
pub async fn remote_v2_terminal_input(
    window: Window,
    webview: Webview,
    state: State<'_, RemoteV2>,
    channel_handle_id: String,
    mut bytes: Vec<u8>,
) -> Result<(), String> {
    let access = match trusted_remote_channel(&window, &webview, &state, &channel_handle_id) {
        Ok(access) => access,
        Err(error) => {
            bytes.zeroize();
            return Err(error);
        }
    };
    let Some(control) = access.terminal_control else {
        bytes.zeroize();
        return Err("Terminal control is unavailable on this channel.".into());
    };
    if bytes.is_empty() || bytes.len() > 4_096 {
        bytes.zeroize();
        return Err("Terminal input size is invalid.".into());
    }
    if !control.focused.load(Ordering::Acquire) {
        bytes.zeroize();
        return Err("Terminal input is disabled while the terminal is unfocused.".into());
    }
    if let Err(error) = reserve_control_bytes(&control, bytes.len()) {
        bytes.zeroize();
        return Err(error);
    }
    let generation = control.focus_generation.load(Ordering::Acquire);
    let length = bytes.len();
    let (completed, receiver) = oneshot::channel();
    match control
        .sender
        .try_send(NativeTerminalControlCommand::Input {
            bytes: zeroize::Zeroizing::new(bytes),
            focus_generation: generation,
            completed,
        }) {
        Ok(()) => await_control_result(receiver).await,
        Err(mpsc::error::TrySendError::Full(mut command)) => {
            if let NativeTerminalControlCommand::Input { bytes, .. } = &mut command {
                bytes.zeroize();
            }
            control.queued_bytes.fetch_sub(length, Ordering::AcqRel);
            Err("Remote terminal input queue is full.".into())
        }
        Err(mpsc::error::TrySendError::Closed(mut command)) => {
            if let NativeTerminalControlCommand::Input { bytes, .. } = &mut command {
                bytes.zeroize();
            }
            control.queued_bytes.fetch_sub(length, Ordering::AcqRel);
            Err("Remote terminal control is closed.".into())
        }
    }
}

#[tauri::command]
pub async fn remote_v2_terminal_resize(
    window: Window,
    webview: Webview,
    state: State<'_, RemoteV2>,
    channel_handle_id: String,
    columns: u32,
    rows: u32,
) -> Result<(), String> {
    let access = trusted_remote_channel(&window, &webview, &state, &channel_handle_id)?;
    let control = access
        .terminal_control
        .ok_or("Terminal control is unavailable on this channel.")?;
    if !control.focused.load(Ordering::Acquire) {
        return Err("Terminal resize is disabled while the terminal is unfocused.".into());
    }
    let generation = control.focus_generation.load(Ordering::Acquire);
    let (completed, receiver) = oneshot::channel();
    control
        .sender
        .try_send(NativeTerminalControlCommand::Resize {
            columns,
            rows,
            focus_generation: generation,
            completed,
        })
        .map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => "Remote terminal input queue is full.".to_owned(),
            mpsc::error::TrySendError::Closed(_) => "Remote terminal control is closed.".to_owned(),
        })?;
    await_control_result(receiver).await
}

#[tauri::command]
pub async fn remote_v2_terminal_release(
    window: Window,
    webview: Webview,
    state: State<'_, RemoteV2>,
    channel_handle_id: String,
) -> Result<(), String> {
    let access = trusted_remote_channel(&window, &webview, &state, &channel_handle_id)?;
    let control = access
        .terminal_control
        .ok_or("Terminal control is unavailable on this channel.")?;
    if advance_focus_generation(&control).is_err() {
        control.focused.store(false, Ordering::Release);
        let _ = access.sender.send(RemoteChannelCommand::Close).await;
        let _ = control
            .sender
            .send(NativeTerminalControlCommand::Close)
            .await;
        return Err("Terminal focus generation is exhausted; the channel was closed.".into());
    }
    control.focused.store(false, Ordering::Release);
    let (completed, receiver) = oneshot::channel();
    control
        .sender
        .send(NativeTerminalControlCommand::Release { completed })
        .await
        .map_err(|_| "Remote terminal control is closed.")?;
    await_control_result(receiver).await
}

#[tauri::command]
pub async fn remote_v2_terminal_focus(
    window: Window,
    webview: Webview,
    state: State<'_, RemoteV2>,
    channel_handle_id: String,
    focused: bool,
) -> Result<(), String> {
    let access = trusted_remote_channel(&window, &webview, &state, &channel_handle_id)?;
    let control = access
        .terminal_control
        .ok_or("Terminal control is unavailable on this channel.")?;
    if let Err(error) = advance_focus_generation(&control) {
        if !focused {
            control.focused.store(false, Ordering::Release);
            let _ = access.sender.send(RemoteChannelCommand::Close).await;
            let _ = control
                .sender
                .send(NativeTerminalControlCommand::Close)
                .await;
        }
        return Err(error);
    }
    control.focused.store(focused, Ordering::Release);
    if focused {
        return Ok(());
    }
    let (completed, receiver) = oneshot::channel();
    control
        .sender
        .send(NativeTerminalControlCommand::Release { completed })
        .await
        .map_err(|_| "Remote terminal control is closed.")?;
    await_control_result(receiver).await
}
