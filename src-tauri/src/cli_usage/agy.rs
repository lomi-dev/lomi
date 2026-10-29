use super::{
    read_optional_bytes, AuthReadError, FetchFailure, UsageProcessPaths, UsageStatus, UsageWindow,
    MAX_CREDENTIAL_BYTES, MAX_RESPONSE_BYTES,
};
use crate::cli_titles::TitleCli;
use chrono::{DateTime, FixedOffset};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
const VERSION_OUTPUT_LIMIT: usize = 4096;
const STDERR_OUTPUT_LIMIT: usize = 16 * 1024;
const MAX_GROUPS: usize = 24;
const MAX_BUCKETS: usize = 32;
const MAX_WINDOWS: usize = 48;
const MAX_LABEL_CHARS: usize = 80;

#[derive(Clone)]
pub(super) struct Context {
    pub directory: PathBuf,
    working_directory: PathBuf,
    pub executable: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
    pub identity: String,
    pub unsupported_message: Option<&'static str>,
    executable_fingerprint: Vec<u8>,
}

pub(super) fn prepare(paths: &UsageProcessPaths) -> Result<Context, FetchFailure> {
    let directory = paths.directory_for(TitleCli::Agy).map_err(|_| {
        failure(
            UsageStatus::Error,
            "Cannot locate the running Antigravity CLI account directory.",
        )
    })?;
    let working_directory = paths
        .home
        .as_deref()
        .filter(|path| path.is_absolute() && path.is_dir())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            failure(
                UsageStatus::Error,
                "Cannot locate the running Antigravity CLI home directory.",
            )
        })?;
    let executable = paths
        .executable
        .as_deref()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            failure(
                UsageStatus::Error,
                "Cannot verify the running Antigravity CLI executable.",
            )
        })?
        .canonicalize()
        .map_err(|_| {
            failure(
                UsageStatus::Error,
                "Cannot verify the running Antigravity CLI executable.",
            )
        })?;
    let executable_fingerprint = executable_fingerprint(&executable)?;

    let settings_path = directory.join("settings.json");
    let settings =
        read_optional_bytes(&settings_path, MAX_CREDENTIAL_BYTES).map_err(|error| match error {
            AuthReadError::Missing => failure(
                UsageStatus::Error,
                "Antigravity CLI settings could not be read safely.",
            ),
            AuthReadError::Unsupported(message) => failure(UsageStatus::Unsupported, message),
            AuthReadError::Error(_) | AuthReadError::KeychainAccessRequired => failure(
                UsageStatus::Error,
                "Antigravity CLI settings could not be read safely.",
            ),
        })?;
    let settings_value = settings
        .as_deref()
        .map(serde_json::from_slice::<Value>)
        .transpose()
        .map_err(|_| {
            failure(
                UsageStatus::Error,
                "Antigravity CLI settings could not be read safely.",
            )
        })?;
    let model_provider = settings_value.as_ref().and_then(global_model_provider);

    let mut unsupported_message = None;
    if model_provider == Some("gemini") {
        unsupported_message =
            Some("Antigravity usage is unavailable while the Gemini provider is selected.");
    } else if paths.has_auth_argument_override {
        unsupported_message =
            Some("Antigravity usage is unavailable with an external authentication override.");
    }

    let mut identity = Sha256::new();
    identity.update(b"lomi-antigravity-usage-v1\0");
    update_path(&mut identity, &directory);
    update_path(&mut identity, &executable);
    identity.update(&executable_fingerprint);
    identity.update([u8::from(paths.has_auth_argument_override)]);
    if let Some(settings) = settings {
        identity.update((settings.len() as u64).to_be_bytes());
        identity.update(settings);
    } else {
        identity.update(u64::MAX.to_be_bytes());
    }
    let mut environment = paths.subprocess_environment.clone();
    environment.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, value) in &environment {
        update_os_str(&mut identity, name);
        update_os_str(&mut identity, value);
    }

    Ok(Context {
        directory,
        working_directory,
        executable,
        environment,
        identity: hex_digest(&identity.finalize()),
        unsupported_message,
        executable_fingerprint,
    })
}

fn global_model_provider(value: &Value) -> Option<&str> {
    let root = value.as_object()?;
    match root.get("modelProvider") {
        None => None,
        Some(Value::String(provider)) => Some(provider),
        Some(_) => None,
    }
}

pub(super) fn check_version(context: &Context) -> Result<(), FetchFailure> {
    if let Some(message) = context.unsupported_message {
        return Err(failure(UsageStatus::Unsupported, message));
    }
    ensure_executable_unchanged(context)?;
    let version = run_fixed(
        context,
        &["--version"],
        VERSION_OUTPUT_LIMIT,
        VERSION_OUTPUT_LIMIT,
    )?;
    if !version.status.success() {
        return Err(failure(
            UsageStatus::Unsupported,
            "This Antigravity CLI version does not expose the verified usage command.",
        ));
    }
    if !supported_version(&version.stdout) {
        return Err(failure(
            UsageStatus::Unsupported,
            "Antigravity usage requires a stable agy 1.1.11 or newer release.",
        ));
    }
    Ok(())
}

pub(super) fn read_usage(context: &Context) -> Result<Vec<UsageWindow>, FetchFailure> {
    if let Some(message) = context.unsupported_message {
        return Err(failure(UsageStatus::Unsupported, message));
    }
    ensure_executable_unchanged(context)?;
    let output = run_fixed(
        context,
        &["-p", "/usage", "--output-format", "json"],
        MAX_RESPONSE_BYTES,
        STDERR_OUTPUT_LIMIT,
    )?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        let unauthenticated = [
            "authentication required",
            "not authenticated",
            "unauthenticated",
            "unauthorized",
            "sign in",
            "login required",
        ]
        .iter()
        .any(|needle| text.contains(needle));
        return Err(if unauthenticated {
            failure(
                UsageStatus::Unauthenticated,
                "Sign in to Antigravity CLI to see account usage.",
            )
        } else {
            failure(
                UsageStatus::Error,
                "Antigravity CLI could not read account usage. Try again shortly.",
            )
        });
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        failure(
            UsageStatus::Error,
            "The Antigravity usage response format is not recognized.",
        )
    })?;
    parse_usage_response(&value)
}

fn parse_usage_response(value: &Value) -> Result<Vec<UsageWindow>, FetchFailure> {
    let Some(object) = value.as_object() else {
        return Err(unrecognized_response());
    };
    if object.get("status").and_then(Value::as_str) != Some("SUCCESS")
        || object.get("num_turns").and_then(Value::as_u64) != Some(0)
        || !empty_conversation_id(object.get("conversation_id"))
    {
        return Err(unrecognized_response());
    }
    let Some(command) = object.get("command").and_then(Value::as_object) else {
        return Err(unrecognized_response());
    };
    if command.get("name").and_then(Value::as_str) != Some("usage") {
        return Err(unrecognized_response());
    }
    let Some(usage) = object.get("usage").and_then(Value::as_object) else {
        return Err(unrecognized_response());
    };
    let mut token_fields = 0;
    if !all_usage_tokens_are_zero(&Value::Object(usage.clone()), &mut token_fields)
        || token_fields == 0
    {
        return Err(unrecognized_response());
    }

    let Some(groups) = command
        .get("data")
        .and_then(Value::as_object)
        .and_then(|data| data.get("groups"))
        .and_then(Value::as_array)
    else {
        return Err(unrecognized_response());
    };
    let mut windows = Vec::new();
    for group in groups.iter().take(MAX_GROUPS) {
        let Some(group) = group.as_object() else {
            continue;
        };
        let group_name =
            text_field(group, &["name", "description"]).unwrap_or_else(|| "Antigravity".to_owned());
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };
        for bucket in buckets.iter().take(MAX_BUCKETS) {
            if windows.len() >= MAX_WINDOWS {
                break;
            }
            let Some(bucket) = bucket.as_object() else {
                continue;
            };
            let Some(fraction) = bucket
                .get("remaining_fraction")
                .and_then(Value::as_f64)
                .filter(|fraction| fraction.is_finite() && (0.0..=1.0).contains(fraction))
            else {
                continue;
            };
            let Some(window) = bucket
                .get("window")
                .and_then(Value::as_str)
                .and_then(bounded_text)
            else {
                continue;
            };
            let bucket_name = text_field(bucket, &["name", "description", "id"])
                .unwrap_or_else(|| "Usage".to_owned());
            let label = bounded_text(&format!("{group_name} · {bucket_name} · {window}"))
                .unwrap_or_else(|| "Antigravity usage".to_owned());
            windows.push(UsageWindow {
                label,
                remaining_percent: Some(fraction * 100.0),
                used: None,
                limit: None,
                unit: None,
                resets_at: bucket.get("reset_time").and_then(reset_time_millis),
            });
        }
    }
    if windows.is_empty() {
        return Err(unrecognized_response());
    }
    Ok(windows)
}

fn empty_conversation_id(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.is_empty(),
        _ => false,
    }
}

fn all_usage_tokens_are_zero(value: &Value, count: &mut usize) -> bool {
    match value {
        Value::Object(object) => object.iter().all(|(key, value)| {
            if key.to_ascii_lowercase().contains("token") {
                *count += 1;
                value.as_i64() == Some(0) || value.as_u64() == Some(0)
            } else {
                all_usage_tokens_are_zero(value, count)
            }
        }),
        Value::Array(values) => values
            .iter()
            .all(|value| all_usage_tokens_are_zero(value, count)),
        _ => true,
    }
}

fn text_field(object: &serde_json::Map<String, Value>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| object.get(*name).and_then(Value::as_str))
        .and_then(bounded_text)
}

fn bounded_text(value: &str) -> Option<String> {
    let text = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_LABEL_CHARS)
        .collect::<String>();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn reset_time_millis(value: &Value) -> Option<i64> {
    let date = DateTime::<FixedOffset>::parse_from_rfc3339(value.as_str()?).ok()?;
    let millis = date.timestamp_millis();
    (DateTime::<chrono::Utc>::from_timestamp_millis(millis).is_some() && millis >= 0)
        .then_some(millis)
}

fn supported_version(output: &[u8]) -> bool {
    let Ok(output) = std::str::from_utf8(output) else {
        return false;
    };
    let output = output.trim();
    let words = output.split_whitespace().collect::<Vec<_>>();
    let version = match words.as_slice() {
        [version] => *version,
        ["agy", version] => *version,
        ["Antigravity", "CLI", version] => *version,
        _ => return false,
    };
    let version = version.strip_prefix('v').unwrap_or(version);
    let (core, build) = version
        .split_once('+')
        .map_or((version, None), |(core, build)| (core, Some(build)));
    if build.is_some_and(|build| {
        build.is_empty()
            || !build
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    }) {
        return false;
    }
    let mut components = core.split('.');
    let Some(major) = components.next().and_then(parse_version_component) else {
        return false;
    };
    let Some(minor) = components.next().and_then(parse_version_component) else {
        return false;
    };
    let Some(patch) = components.next().and_then(parse_version_component) else {
        return false;
    };
    if components.next().is_some()
        || major != 1
        || core.contains('-')
        || (minor < 1 || (minor == 1 && patch < 11))
    {
        return false;
    }
    true
}

fn parse_version_component(component: &str) -> Option<u64> {
    if component.is_empty()
        || (component.len() > 1 && component.starts_with('0'))
        || !component.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    component.parse().ok()
}

#[derive(Debug)]
struct ProcessOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_fixed(
    context: &Context,
    arguments: &[&str],
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<ProcessOutput, FetchFailure> {
    run_fixed_with_timeout(
        context,
        arguments,
        stdout_limit,
        stderr_limit,
        COMMAND_TIMEOUT,
    )
}

fn run_fixed_with_timeout(
    context: &Context,
    arguments: &[&str],
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
) -> Result<ProcessOutput, FetchFailure> {
    ensure_executable_unchanged(context)?;
    let mut command = Command::new(&context.executable);
    command
        .args(arguments)
        .env_clear()
        .envs(
            context
                .environment
                .iter()
                .map(|(name, value)| (name, value)),
        )
        .current_dir(&context.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A new session removes the application's controlling terminal from the child.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().map_err(|_| {
        failure(
            UsageStatus::Error,
            "Antigravity CLI could not be started safely.",
        )
    })?;
    let pid = child.id();
    let Some(mut stdout) = child.stdout.take() else {
        stop_child(&mut child, pid);
        return Err(failure(
            UsageStatus::Error,
            "Antigravity CLI output could not be captured safely.",
        ));
    };
    let Some(mut stderr) = child.stderr.take() else {
        stop_child(&mut child, pid);
        return Err(failure(
            UsageStatus::Error,
            "Antigravity CLI output could not be captured safely.",
        ));
    };
    if set_nonblocking(&stdout).is_err() || set_nonblocking(&stderr).is_err() {
        stop_child(&mut child, pid);
        return Err(failure(
            UsageStatus::Error,
            "Antigravity CLI output could not be captured safely.",
        ));
    }

    let deadline = Instant::now() + timeout;
    let mut out = Vec::with_capacity(stdout_limit.min(16 * 1024));
    let mut err = Vec::with_capacity(stderr_limit.min(4096));
    loop {
        match drain(&mut stdout, &mut out, stdout_limit) {
            Ok(_) => {}
            Err(DrainError::TooLarge) => {
                stop_child(&mut child, pid);
                return Err(failure(
                    UsageStatus::Error,
                    "Antigravity CLI output exceeded the safe size limit.",
                ));
            }
            Err(DrainError::Io) => {
                stop_child(&mut child, pid);
                return Err(failure(
                    UsageStatus::Error,
                    "Antigravity CLI output could not be read safely.",
                ));
            }
        }
        match drain(&mut stderr, &mut err, stderr_limit) {
            Ok(_) => {}
            Err(DrainError::TooLarge) => {
                stop_child(&mut child, pid);
                return Err(failure(
                    UsageStatus::Error,
                    "Antigravity CLI output exceeded the safe size limit.",
                ));
            }
            Err(DrainError::Io) => {
                stop_child(&mut child, pid);
                return Err(failure(
                    UsageStatus::Error,
                    "Antigravity CLI output could not be read safely.",
                ));
            }
        }
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(_) => {
                stop_child(&mut child, pid);
                return Err(failure(
                    UsageStatus::Error,
                    "Antigravity CLI could not be stopped safely.",
                ));
            }
        };
        if let Some(status) = status {
            // The direct child may have left same-session helpers behind. Stop
            // its process group before draining pipes, even when every helper
            // closed stdio and the output streams already look complete.
            kill_process_group(pid);
            let drain_deadline = Instant::now() + Duration::from_millis(100);
            loop {
                let stdout_closed =
                    drain(&mut stdout, &mut out, stdout_limit).map_err(|error| {
                        if matches!(error, DrainError::TooLarge) {
                            failure(
                                UsageStatus::Error,
                                "Antigravity CLI output exceeded the safe size limit.",
                            )
                        } else {
                            failure(
                                UsageStatus::Error,
                                "Antigravity CLI output could not be read safely.",
                            )
                        }
                    })?;
                let stderr_closed =
                    drain(&mut stderr, &mut err, stderr_limit).map_err(|error| {
                        if matches!(error, DrainError::TooLarge) {
                            failure(
                                UsageStatus::Error,
                                "Antigravity CLI output exceeded the safe size limit.",
                            )
                        } else {
                            failure(
                                UsageStatus::Error,
                                "Antigravity CLI output could not be read safely.",
                            )
                        }
                    })?;
                if stdout_closed && stderr_closed {
                    break;
                }
                if Instant::now() >= drain_deadline {
                    kill_process_group(pid);
                    return Err(failure(
                        UsageStatus::Error,
                        "Antigravity CLI output could not be captured safely.",
                    ));
                }
                thread::sleep(Duration::from_millis(2));
            }
            return Ok(ProcessOutput {
                status,
                stdout: out,
                stderr: err,
            });
        }
        if Instant::now() >= deadline {
            stop_child(&mut child, pid);
            return Err(failure(
                UsageStatus::Error,
                "Antigravity CLI usage check timed out.",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[derive(Clone, Copy)]
enum DrainError {
    TooLarge,
    Io,
}

fn drain(reader: &mut impl Read, output: &mut Vec<u8>, limit: usize) -> Result<bool, DrainError> {
    let mut buffer = [0; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(read) => {
                if output.len().saturating_add(read) > limit {
                    return Err(DrainError::TooLarge);
                }
                output.extend_from_slice(&buffer[..read]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(_) => return Err(DrainError::Io),
        }
    }
}

#[cfg(unix)]
fn set_nonblocking(file: &impl std::os::fd::AsRawFd) -> io::Result<()> {
    let descriptor = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_nonblocking(_file: &impl Read) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Antigravity usage requires a Unix process runner",
    ))
}

fn stop_child(child: &mut std::process::Child, pid: u32) {
    kill_process_group(pid);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn kill_process_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_process_group(_pid: u32) {}

fn ensure_executable_unchanged(context: &Context) -> Result<(), FetchFailure> {
    let fingerprint = executable_fingerprint(&context.executable)?;
    if fingerprint != context.executable_fingerprint {
        return Err(failure(
            UsageStatus::Error,
            "The running Antigravity CLI executable changed during the usage check.",
        ));
    }
    Ok(())
}

fn executable_fingerprint(path: &Path) -> Result<Vec<u8>, FetchFailure> {
    let metadata = fs::metadata(path).map_err(|_| {
        failure(
            UsageStatus::Error,
            "Cannot verify the running Antigravity CLI executable.",
        )
    })?;
    if !metadata.is_file() {
        return Err(failure(
            UsageStatus::Error,
            "Cannot verify the running Antigravity CLI executable.",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(metadata.len().to_be_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
            hasher.update(duration.as_secs().to_be_bytes());
            hasher.update(duration.subsec_nanos().to_be_bytes());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        hasher.update(metadata.dev().to_be_bytes());
        hasher.update(metadata.ino().to_be_bytes());
        hasher.update(metadata.mode().to_be_bytes());
        hasher.update(metadata.ctime().to_be_bytes());
        hasher.update(metadata.ctime_nsec().to_be_bytes());
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(failure(
                UsageStatus::Error,
                "The running Antigravity CLI executable is not executable.",
            ));
        }
    }
    Ok(hasher.finalize().to_vec())
}

fn update_path(hasher: &mut Sha256, path: &Path) {
    update_os_str(hasher, path.as_os_str());
}

fn update_os_str(hasher: &mut Sha256, value: &OsStr) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = value.as_bytes();
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    #[cfg(not(unix))]
    {
        let value = value.to_string_lossy();
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn failure(status: UsageStatus, message: &'static str) -> FetchFailure {
    FetchFailure {
        status,
        message,
        retry_after: None,
    }
}

fn unrecognized_response() -> FetchFailure {
    failure(
        UsageStatus::Error,
        "The Antigravity usage response format is not recognized.",
    )
}
