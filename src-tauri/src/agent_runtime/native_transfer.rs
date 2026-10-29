//! Pi 1.0.1 private, lossless session forks. The caller owns checkpoint and leases.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::CString,
    fs::{self, File},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};

const MAX_FILE: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ForkRequest {
    pub(crate) run_id: String,
    pub(crate) input_id: String,
    pub(crate) source_generation: u64,
    pub(crate) next_generation: u64,
    pub(crate) source_file: String,
    pub(crate) source_session: String,
    pub(crate) source_profile: String,
    pub(crate) source_revision: u64,
    pub(crate) destination_profile: String,
    pub(crate) destination_revision: u64,
    pub(crate) cwd: String,
    pub(crate) cwd_identity: (u64, u64),
    pub(crate) model: String,
    pub(crate) version: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileIdentity {
    digest: String,
    device: u64,
    inode: u64,
    size: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ForkReceipt {
    schema: u32,
    request: ForkRequest,
    root_identity: (u64, u64),
    source: FileIdentity,
    destination: FileIdentity,
    pub(crate) destination_file: String,
    pub(crate) session_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ForkIntent {
    schema: u32,
    request: ForkRequest,
    root_identity: (u64, u64),
    source: FileIdentity,
    destination_file: String,
    session_id: String,
    header: String,
    destination_digest: String,
    destination_size: u64,
}
fn denied() -> String {
    "Native Pi history transfer is unsupported, changed, or incomplete. Original history was preserved.".into()
}
fn session_generation(name: &str) -> Option<u64> {
    let digits = name.strip_prefix("pi-session-")?.strip_suffix(".jsonl")?;
    let generation = digits.parse::<u64>().ok()?;
    (generation > 0 && generation.to_string() == digits).then_some(generation)
}
fn directory(root: &Path) -> Result<File, String> {
    if !root.is_absolute() || fs::canonicalize(root).map_err(|_| denied())? != root {
        return Err(denied());
    }
    super::environment::check_private_directory(root)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)
        .map_err(|_| denied())?;
    let metadata = file.metadata().map_err(|_| denied())?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(denied());
    }
    Ok(file)
}
fn open(directory: &File, name: &str, create: bool) -> Result<File, String> {
    if name.contains('/') || name.contains('\0') {
        return Err(denied());
    }
    let name = CString::new(name).map_err(|_| denied())?;
    let flags = libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | if create {
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
        } else {
            libc::O_RDONLY | libc::O_NONBLOCK
        };
    let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if fd < 0 {
        return Err(denied());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn identity(metadata: &fs::Metadata, bytes: &[u8]) -> FileIdentity {
    FileIdentity {
        digest: format!("{:x}", Sha256::digest(bytes)),
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
    }
}
fn read_file(directory: &File, name: &str) -> Result<(Vec<u8>, FileIdentity), String> {
    let mut file = open(directory, name, false)?;
    let before = file.metadata().map_err(|_| denied())?;
    if !before.is_file()
        || before.nlink() != 1
        || before.uid() != unsafe { libc::geteuid() }
        || before.mode() & 0o777 != 0o600
        || before.len() == 0
        || before.len() > MAX_FILE
    {
        return Err(denied());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| denied())?;
    let after = file.metadata().map_err(|_| denied())?;
    if bytes.len() as u64 != before.len()
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(denied());
    }
    let identity = identity(&after, &bytes);
    Ok((bytes, identity))
}
fn write_new(directory: &File, name: &str, bytes: &[u8]) -> Result<FileIdentity, String> {
    if bytes.len() as u64 > MAX_FILE {
        return Err(denied());
    }
    let mut nonce = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut nonce)
        .map_err(|_| denied())?;
    let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let temporary = format!(".pi-transfer-{nonce}.tmp");
    let mut file = open(directory, &temporary, true)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| denied())?;
    let temporary = CString::new(temporary).map_err(|_| denied())?;
    let destination = CString::new(name).map_err(|_| denied())?;
    if name.contains('/') {
        return Err(denied());
    }
    #[cfg(target_os = "macos")]
    let published = unsafe {
        libc::renameatx_np(
            directory.as_raw_fd(),
            temporary.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(target_os = "linux")]
    let published = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            directory.as_raw_fd(),
            temporary.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let published = -1;
    if published != 0 {
        return Err(denied());
    }
    directory.sync_all().map_err(|_| denied())?;
    let metadata = file.metadata().map_err(|_| denied())?;
    Ok(identity(&metadata, bytes))
}
fn validate_request(root: &Path, request: &ForkRequest) -> Result<(), String> {
    if request.version != "1.0.1"
        || request.source_generation.checked_add(1) != Some(request.next_generation)
        || session_generation(&request.source_file).is_none_or(|g| g > request.source_generation)
        || request
            .model
            .split_once('/')
            .is_none_or(|(p, m)| p != "openai" || m.is_empty())
        || request.source_profile == request.destination_profile
        || [
            &request.run_id,
            &request.input_id,
            &request.source_session,
            &request.source_profile,
            &request.destination_profile,
        ]
        .iter()
        .any(|id| id.is_empty() || id.len() > 512)
        || root.file_name().and_then(|s| s.to_str()) != Some("native")
        || root
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            != Some(request.run_id.as_str())
    {
        return Err(denied());
    }
    let cwd = Path::new(&request.cwd);
    if !cwd.is_absolute() || fs::canonicalize(cwd).map_err(|_| denied())? != cwd {
        return Err(denied());
    }
    let metadata = fs::symlink_metadata(cwd).map_err(|_| denied())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || (metadata.dev(), metadata.ino()) != request.cwd_identity
    {
        return Err(denied());
    }
    Ok(())
}
fn fields(value: &Value, required: &[&str], optional: &[&str]) -> Result<(), String> {
    let map = value.as_object().ok_or_else(denied)?;
    if required.iter().any(|key| !map.contains_key(*key))
        || map
            .keys()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(denied());
    }
    Ok(())
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(denied)
}
fn usage(value: &Value) -> Result<(), String> {
    let counters = ["input", "output", "cacheRead", "cacheWrite", "totalTokens"];
    fields(
        value,
        &[
            "input",
            "output",
            "cacheRead",
            "cacheWrite",
            "totalTokens",
            "cost",
        ],
        &[],
    )?;
    if counters.iter().any(|key| {
        value[*key]
            .as_f64()
            .is_none_or(|v| !v.is_finite() || v < 0.0)
    }) {
        return Err(denied());
    }
    let cost = &value["cost"];
    fields(
        cost,
        &["input", "output", "cacheRead", "cacheWrite", "total"],
        &[],
    )?;
    if ["input", "output", "cacheRead", "cacheWrite", "total"]
        .iter()
        .any(|key| {
            cost[*key]
                .as_f64()
                .is_none_or(|v| !v.is_finite() || v < 0.0)
        })
    {
        return Err(denied());
    }
    Ok(())
}
fn unsafe_metadata(value: &Value, diagnostic_message: Option<&Value>) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                // Chat Completions IDs are retained diagnostics, never continuation handles.
                if key == "responseId"
                    && diagnostic_message.is_some_and(|message| {
                        message
                            .as_object()
                            .is_some_and(|candidate| std::ptr::eq(map, candidate))
                    })
                    && value.is_string()
                {
                    continue;
                }
                let key = key.to_ascii_lowercase();
                if [
                    "signature",
                    "encrypted",
                    "credential",
                    "authorization",
                    "account",
                    "providerref",
                    "itemid",
                    "responseid",
                ]
                .iter()
                .any(|needle| key.contains(needle))
                {
                    return Err(denied());
                }
                unsafe_metadata(value, diagnostic_message)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                unsafe_metadata(item, diagnostic_message)?;
            }
        }
        _ => (),
    }
    Ok(())
}
fn content(value: &Value, tools: bool) -> Result<Vec<String>, String> {
    if value.is_string() && !tools {
        return Ok(vec![]);
    }
    let items = value.as_array().ok_or_else(denied)?;
    let mut calls = vec![];
    for item in items {
        match string(item, "type")? {
            "text" => {
                fields(item, &["type", "text"], &[])?;
                if !item["text"].is_string() {
                    return Err(denied());
                }
            }
            "image" if !tools => {
                fields(item, &["type", "data", "mimeType"], &[])?;
                string(item, "data")?;
                string(item, "mimeType")?;
            }
            "toolCall" if tools => {
                fields(item, &["type", "id", "name", "arguments"], &[])?;
                string(item, "name")?;
                if !item["arguments"].is_object() {
                    return Err(denied());
                }
                calls.push(string(item, "id")?.into());
            }
            _ => return Err(denied()),
        }
    }
    Ok(calls)
}
fn message(value: &Value, pending: &mut BTreeSet<String>) -> Result<(), String> {
    match string(value, "role")? {
        "user" => {
            fields(value, &["role", "content", "timestamp"], &[])?;
            content(&value["content"], false)?;
            if !pending.is_empty() {
                return Err(denied());
            }
        }
        "system" => {
            fields(
                value,
                &["role", "content", "timestamp"],
                &["sections", "toolsAdded", "toolsRemoved"],
            )?;
            if !pending.is_empty() {
                return Err(denied());
            }
            if !value["content"].is_string() {
                for block in value["content"].as_array().ok_or_else(denied)? {
                    fields(block, &["type", "text"], &[])?;
                    if block["type"] != "text" || !block["text"].is_string() {
                        return Err(denied());
                    }
                }
            }
            if let Some(sections) = value.get("sections") {
                if sections.as_object().is_none_or(|sections| {
                    sections.values().any(|v| !v.is_string() && !v.is_null())
                }) {
                    return Err(denied());
                }
            }
            if let Some(tools) = value.get("toolsAdded") {
                for tool in tools.as_array().ok_or_else(denied)? {
                    fields(
                        tool,
                        &["name", "description", "parameters"],
                        &["constrainedSampling"],
                    )?;
                    string(tool, "name")?;
                    if !tool["description"].is_string() || !tool["parameters"].is_object() {
                        return Err(denied());
                    }
                    if let Some(config) = tool.get("constrainedSampling") {
                        if config != &Value::Bool(false) {
                            match string(config, "type")? {
                                "json_schema" => {
                                    fields(config, &["type", "strict"], &[])?;
                                    if !matches!(
                                        config["strict"].as_str(),
                                        Some("prefer" | "require")
                                    ) {
                                        return Err(denied());
                                    }
                                }
                                "grammar" => {
                                    fields(config, &["type", "variants"], &[])?;
                                    fields(
                                        &config["variants"],
                                        &[],
                                        &["openai_lark", "openai_regex"],
                                    )?;
                                    if config["variants"]
                                        .as_object()
                                        .ok_or_else(denied)?
                                        .values()
                                        .any(|v| !v.is_string())
                                    {
                                        return Err(denied());
                                    }
                                }
                                _ => return Err(denied()),
                            }
                        }
                    }
                }
            }
            if let Some(tools) = value.get("toolsRemoved") {
                for tool in tools.as_array().ok_or_else(denied)? {
                    fields(tool, &["name"], &[])?;
                    string(tool, "name")?;
                }
            }
        }
        "assistant" => {
            fields(
                value,
                &[
                    "role",
                    "content",
                    "api",
                    "provider",
                    "model",
                    "usage",
                    "stopReason",
                    "timestamp",
                ],
                &[
                    "errorMessage",
                    "responseId",
                    "responseModel",
                    "rawStopReason",
                ],
            )?;
            if value["provider"] != "openai"
                || value["api"] != "openai-completions"
                || !pending.is_empty()
            {
                return Err(denied());
            }
            string(value, "model")?;
            usage(&value["usage"])?;
            if value.get("errorMessage").is_some_and(|v| !v.is_string()) {
                return Err(denied());
            }
            if ["responseId", "responseModel", "rawStopReason"]
                .iter()
                .any(|key| value.get(*key).is_some_and(|v| !v.is_string()))
            {
                return Err(denied());
            }
            for call in content(&value["content"], true)? {
                if !pending.insert(call) {
                    return Err(denied());
                }
            }
            // Pi omits failed/aborted messages from request projection; the fork keeps
            // their exact bytes. The caller still requires a settled native checkpoint.
            if !matches!(
                value["stopReason"].as_str(),
                Some("stop" | "length" | "toolUse" | "error" | "aborted")
            ) {
                return Err(denied());
            }
        }
        "toolResult" => {
            fields(
                value,
                &[
                    "role",
                    "toolCallId",
                    "toolName",
                    "content",
                    "isError",
                    "timestamp",
                ],
                &["details"],
            )?;
            string(value, "toolName")?;
            content(&value["content"], false)?;
            if !value["isError"].is_boolean() || !pending.remove(string(value, "toolCallId")?) {
                return Err(denied());
            }
        }
        _ => return Err(denied()),
    }
    if value["timestamp"].as_u64().is_none() {
        return Err(denied());
    }
    Ok(())
}
/// Validate every branch, retaining raw non-header bytes rather than projecting context.
fn validate_history(bytes: &[u8], request: &ForkRequest) -> Result<usize, String> {
    if bytes.last() != Some(&b'\n') {
        return Err(denied());
    }
    let mut lines = bytes.split_inclusive(|b| *b == b'\n');
    let first = lines.next().ok_or_else(denied)?;
    let header = super::native_wire::strict_json(first)?;
    fields(
        &header,
        &["type", "version", "id", "timestamp", "cwd"],
        &["parentSession"],
    )?;
    if header["type"] != "session"
        || header["version"] != 3
        || header["id"] != request.source_session
        || header["cwd"] != request.cwd
    {
        return Err(denied());
    }
    chrono::DateTime::parse_from_rfc3339(string(&header, "timestamp")?).map_err(|_| denied())?;
    let mut states: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut messages = 0;
    let mut pending_nodes = 0usize;
    let mut tool_names = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        if index >= MAX_ENTRIES {
            return Err(denied());
        }
        let entry = super::native_wire::strict_json(line)?;
        let diagnostic_message = (entry["type"] == "message"
            && entry["message"]["role"] == "assistant"
            && entry["message"]["provider"] == "openai"
            && entry["message"]["api"] == "openai-completions")
            .then_some(&entry["message"]);
        unsafe_metadata(&entry, diagnostic_message)?;
        let id = string(&entry, "id")?.to_string();
        if id.len() > 512 || states.contains_key(&id) {
            return Err(denied());
        }
        chrono::DateTime::parse_from_rfc3339(string(&entry, "timestamp")?).map_err(|_| denied())?;
        let mut pending = if entry["parentId"].is_null() {
            BTreeSet::new()
        } else {
            states
                .get(string(&entry, "parentId")?)
                .ok_or_else(denied)?
                .clone()
        };
        let base = ["type", "id", "parentId", "timestamp"];
        let extra: &[&str] = match string(&entry, "type")? {
            "message" => {
                if entry["message"]["role"] == "assistant" {
                    for call in content(&entry["message"]["content"], true)? {
                        let name = entry["message"]["content"]
                            .as_array()
                            .ok_or_else(denied)?
                            .iter()
                            .find(|item| item["id"] == call)
                            .and_then(|item| item["name"].as_str())
                            .ok_or_else(denied)?
                            .to_string();
                        if call.len() > 512
                            || tool_names.insert(call, name).is_some()
                            || tool_names.len() > MAX_ENTRIES
                        {
                            return Err(denied());
                        }
                    }
                }
                if entry["message"]["role"] == "toolResult"
                    && tool_names
                        .get(string(&entry["message"], "toolCallId")?)
                        .map(String::as_str)
                        != Some(string(&entry["message"], "toolName")?)
                {
                    return Err(denied());
                }
                message(&entry["message"], &mut pending)?;
                messages += 1;
                &["message"]
            }
            "model_change" => {
                if entry["provider"] != "openai" {
                    return Err(denied());
                }
                string(&entry, "modelId")?;
                &["provider", "modelId"]
            }
            "thinking_level_change" => {
                if !matches!(
                    entry["thinkingLevel"].as_str(),
                    Some("off" | "minimal" | "low" | "medium" | "high" | "xhigh")
                ) {
                    return Err(denied());
                }
                &["thinkingLevel"]
            }
            "usage" => {
                if entry["provider"] != "openai" {
                    return Err(denied());
                }
                string(&entry, "kind")?;
                string(&entry, "model")?;
                usage(&entry["usage"])?;
                if entry.get("note").is_some_and(|v| !v.is_string()) {
                    return Err(denied());
                }
                &["kind", "provider", "model", "usage", "note"]
            }
            "compaction" | "branch_summary" => {
                if !pending.is_empty()
                    || !entry["summary"].is_string()
                    || entry["fromHook"].as_bool() == Some(true)
                {
                    return Err(denied());
                }
                if entry.get("fromHook").is_some_and(|v| !v.is_boolean()) {
                    return Err(denied());
                }
                if let Some(value) = entry.get("usage") {
                    usage(value)?;
                }
                if let Some(details) = entry.get("details") {
                    fields(details, &[], &["readFiles", "modifiedFiles"])?;
                    for key in ["readFiles", "modifiedFiles"] {
                        if let Some(files) = details.get(key) {
                            if files
                                .as_array()
                                .is_none_or(|items| items.iter().any(|v| !v.is_string()))
                            {
                                return Err(denied());
                            }
                        }
                    }
                }
                if let Some(system) = entry.get("systemMessage") {
                    if system["role"] != "system" {
                        return Err(denied());
                    }
                    message(system, &mut pending)?;
                }
                if entry["type"] == "compaction" {
                    if !entry["tokensBefore"].is_number()
                        || (!entry["firstKeptEntryId"].is_null()
                            && !states.contains_key(string(&entry, "firstKeptEntryId")?))
                    {
                        return Err(denied());
                    }
                    &[
                        "summary",
                        "firstKeptEntryId",
                        "tokensBefore",
                        "details",
                        "usage",
                        "fromHook",
                        "systemMessage",
                    ]
                } else {
                    if !states.contains_key(string(&entry, "fromId")?) {
                        return Err(denied());
                    }
                    &["fromId", "summary", "details", "usage", "fromHook"]
                }
            }
            "label" => {
                if !states.contains_key(string(&entry, "targetId")?)
                    || entry.get("label").is_some_and(|v| !v.is_string())
                {
                    return Err(denied());
                }
                &["targetId", "label"]
            }
            "session_info" => {
                if entry.get("name").is_some_and(|v| !v.is_string()) {
                    return Err(denied());
                }
                &["name"]
            }
            _ => return Err(denied()),
        };
        fields(&entry, &base, extra)?;
        pending_nodes = pending_nodes
            .checked_add(pending.len())
            .ok_or_else(denied)?;
        if pending.len() > 4096 || pending_nodes > 262_144 {
            return Err(denied());
        }
        states.insert(id, pending);
    }
    // A pending intermediate node is valid only when a later descendant closes it.
    // Leaves represent resumable branches and must all have completed tool results.
    let mut parents = BTreeSet::new();
    for line in bytes[first.len()..].split_inclusive(|b| *b == b'\n') {
        let entry = super::native_wire::strict_json(line)?;
        if let Some(parent) = entry["parentId"].as_str() {
            parents.insert(parent.to_string());
        }
    }
    if messages == 0
        || states
            .iter()
            .any(|(id, pending)| !parents.contains(id) && !pending.is_empty())
    {
        return Err(denied());
    }
    Ok(first.len())
}
fn receipt_name(generation: u64) -> String {
    format!("pi-transfer-{generation}.json")
}
fn intent_name(generation: u64) -> String {
    format!("pi-transfer-intent-{generation}.json")
}
fn exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(denied()),
    }
}
/// Read-only preview; the caller must compare both returned values during apply.
pub(crate) fn inspect(root: &Path, request: &ForkRequest) -> Result<(String, u64), String> {
    validate_request(root, request)?;
    let directory = directory(root)?;
    let (bytes, identity) = read_file(&directory, &request.source_file)?;
    validate_history(&bytes, request)?;
    if read_file(&directory, &request.source_file)?.1 != identity {
        return Err(denied());
    }
    Ok((identity.digest, identity.size))
}
pub(crate) fn read(root: &Path, next_generation: u64) -> Result<ForkReceipt, String> {
    let directory = directory(root)?;
    let (bytes, _) = read_file(&directory, &receipt_name(next_generation))?;
    let value = super::native_wire::strict_json(&bytes)?;
    let receipt: ForkReceipt = serde_json::from_value(value).map_err(|_| denied())?;
    if receipt.schema != 1
        || receipt.request.next_generation != next_generation
        || serde_json::to_vec(&receipt).map_err(|_| denied())? != bytes
    {
        return Err(denied());
    }
    Ok(receipt)
}

/// Must be called under the run writer lease after validating the latest checkpoint.
/// Reviewed source bytes are checked before writing or adopting any fork state.
pub(crate) fn prepare_reviewed(
    root: &Path,
    request: &ForkRequest,
    expected_digest: &str,
    expected_size: u64,
) -> Result<ForkReceipt, String> {
    validate_request(root, request)?;
    if expected_digest.len() != 64
        || !expected_digest.bytes().all(|b| b.is_ascii_hexdigit())
        || expected_size == 0
        || expected_size > MAX_FILE
    {
        return Err(denied());
    }
    let directory = directory(root)?;
    let (source, source_identity) = read_file(&directory, &request.source_file)?;
    if source_identity.digest != expected_digest || source_identity.size != expected_size {
        return Err(denied());
    }
    let suffix = validate_history(&source, request)?;
    match fs::symlink_metadata(root.join(receipt_name(request.next_generation))) {
        Ok(_) => {
            let receipt = read(root, request.next_generation)?;
            fence(root, request, &receipt)?;
            // A prior exclusive rename may have succeeded before directory sync
            // failed. Retrying must make the validated receipt durable before
            // the caller commits a destination account pin.
            directory.sync_all().map_err(|_| denied())?;
            return Ok(receipt);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => return Err(denied()),
    }
    let destination_file = format!("pi-session-{}.jsonl", request.next_generation);
    let metadata = directory.metadata().map_err(|_| denied())?;
    let root_identity = (metadata.dev(), metadata.ino());
    let intent_file = intent_name(request.next_generation);
    let intent = if exists(&root.join(&intent_file))? {
        let (bytes, _) = read_file(&directory, &intent_file)?;
        let value = super::native_wire::strict_json(&bytes)?;
        let intent: ForkIntent = serde_json::from_value(value).map_err(|_| denied())?;
        if serde_json::to_vec(&intent).map_err(|_| denied())? != bytes {
            return Err(denied());
        }
        intent
    } else {
        // Without durable prior intent, an existing destination is foreign state.
        if exists(&root.join(&destination_file))? {
            return Err(denied());
        }
        let mut uuid = [0u8; 16];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut uuid)
            .map_err(|_| denied())?;
        uuid[6] = (uuid[6] & 0x0f) | 0x40;
        uuid[8] = (uuid[8] & 0x3f) | 0x80;
        let hex: String = uuid.iter().map(|byte| format!("{byte:02x}")).collect();
        let session_id = format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| denied())?;
        let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp(
            i64::try_from(now.as_secs()).map_err(|_| denied())?,
            now.subsec_nanos(),
        )
        .ok_or_else(denied)?
        .to_rfc3339();
        let header = serde_json::json!({"type":"session", "version":3, "id":session_id, "timestamp":timestamp, "cwd":request.cwd, "parentSession":root.join(&request.source_file)});
        let header = serde_json::to_string(&header).map_err(|_| denied())?;
        let mut destination = header.as_bytes().to_vec();
        destination.push(b'\n');
        destination.extend_from_slice(&source[suffix..]);
        if destination.len() as u64 > MAX_FILE
            || read_file(&directory, &request.source_file)?.1 != source_identity
        {
            return Err(denied());
        }
        let intent = ForkIntent {
            schema: 1,
            request: request.clone(),
            root_identity,
            source: source_identity.clone(),
            destination_file: destination_file.clone(),
            session_id,
            header,
            destination_digest: format!("{:x}", Sha256::digest(&destination)),
            destination_size: destination.len() as u64,
        };
        write_new(
            &directory,
            &intent_file,
            &serde_json::to_vec(&intent).map_err(|_| denied())?,
        )?;
        intent
    };
    if intent.schema != 1
        || intent.request != *request
        || intent.root_identity != root_identity
        || intent.source != source_identity
        || intent.destination_file != destination_file
    {
        return Err(denied());
    }
    let header = super::native_wire::strict_json(intent.header.as_bytes())?;
    fields(
        &header,
        &["type", "version", "id", "timestamp", "cwd", "parentSession"],
        &[],
    )?;
    let id = intent.session_id.as_bytes();
    if id.len() != 36
        || id.iter().enumerate().any(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                *byte != b'-'
            } else {
                !byte.is_ascii_hexdigit()
            }
        })
        || id[14] != b'4'
        || !matches!(id[19], b'8' | b'9' | b'a' | b'b')
        || intent.session_id == request.source_session
        || header["id"] != intent.session_id
        || header["parentSession"] != root.join(&request.source_file).to_string_lossy().as_ref()
    {
        return Err(denied());
    }
    let mut destination = intent.header.as_bytes().to_vec();
    destination.push(b'\n');
    destination.extend_from_slice(&source[suffix..]);
    let mut destination_request = request.clone();
    destination_request
        .source_session
        .clone_from(&intent.session_id);
    validate_history(&destination, &destination_request)?;
    if destination.len() as u64 != intent.destination_size
        || format!("{:x}", Sha256::digest(&destination)) != intent.destination_digest
        || read_file(&directory, &request.source_file)?.1 != source_identity
    {
        return Err(denied());
    }
    let destination_identity = if exists(&root.join(&destination_file))? {
        let (existing, identity) = read_file(&directory, &destination_file)?;
        if existing != destination {
            return Err(denied());
        }
        open(&directory, &destination_file, false)?
            .sync_all()
            .map_err(|_| denied())?;
        identity
    } else {
        write_new(&directory, &destination_file, &destination)?
    };
    let receipt = ForkReceipt {
        schema: 1,
        request: request.clone(),
        root_identity,
        source: source_identity,
        destination: destination_identity,
        destination_file,
        session_id: intent.session_id,
    };
    let bytes = serde_json::to_vec(&receipt).map_err(|_| denied())?;
    write_new(&directory, &receipt_name(request.next_generation), &bytes)?;
    fence(root, request, &receipt)?;
    Ok(receipt)
}
/// Fence ONLY the first destination launch; Pi appends to this file after dispatch.
pub(crate) fn fence(
    root: &Path,
    request: &ForkRequest,
    receipt: &ForkReceipt,
) -> Result<(), String> {
    validate_request(root, request)?;
    let directory = directory(root)?;
    let metadata = directory.metadata().map_err(|_| denied())?;
    let (destination, destination_identity) = read_file(&directory, &receipt.destination_file)?;
    if receipt.schema != 1
        || &receipt.request != request
        || receipt.root_identity != (metadata.dev(), metadata.ino())
        || receipt.destination_file != format!("pi-session-{}.jsonl", request.next_generation)
        || session_generation(&receipt.destination_file).is_none()
        || read(root, request.next_generation)? != *receipt
        || read_file(&directory, &request.source_file)?.1 != receipt.source
        || destination_identity != receipt.destination
    {
        return Err(denied());
    }
    let mut destination_request = request.clone();
    destination_request
        .source_session
        .clone_from(&receipt.session_id);
    validate_history(&destination, &destination_request)?;
    Ok(())
}
