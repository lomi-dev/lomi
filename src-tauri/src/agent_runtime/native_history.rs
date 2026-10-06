//! Fail-closed native continuation. Missing history never becomes a fresh task.
use crate::cli_catalog::TitleCli;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
const MAX_FILE: u64 = 128 * 1024 * 1024;
#[derive(Clone)]
struct FileFence {
    path: PathBuf,
    identity: (u64, u64, u64, i64, i64),
    digest: Vec<u8>,
}
pub(crate) struct Continuation {
    pub(crate) arguments: Vec<OsString>,
    files: Vec<FileFence>,
    absent: Vec<PathBuf>,
    root: PathBuf,
}
fn missing() -> String {
    "No qualified native history exists for this workspace. Existing files were preserved; no new conversation was started.".into()
}
fn ancestry(root: &Path, path: &Path) -> Result<(), String> {
    if !path.starts_with(root) {
        return Err(missing());
    }
    for parent in path.parent().ok_or_else(missing)?.ancestors() {
        if !parent.starts_with(root) {
            break;
        }
        let m = fs::symlink_metadata(parent).map_err(|_| missing())?;
        if !m.is_dir()
            || m.file_type().is_symlink()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o022 != 0
        {
            return Err(missing());
        }
    }
    Ok(())
}
fn read(root: &Path, path: &Path) -> Result<(Vec<u8>, FileFence), String> {
    ancestry(root, path)?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| missing())?;
    let m = file.metadata().map_err(|_| missing())?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o022 != 0
        || m.nlink() != 1
        || m.len() > MAX_FILE
    {
        return Err(missing());
    }
    let identity = (m.dev(), m.ino(), m.len(), m.mtime(), m.mtime_nsec());
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| missing())?;
    let after = file.metadata().map_err(|_| missing())?;
    if bytes.len() as u64 != m.len()
        || (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
        ) != identity
    {
        return Err(missing());
    }
    let digest = Sha256::digest(&bytes).to_vec();
    Ok((
        bytes,
        FileFence {
            path: path.into(),
            identity,
            digest,
        },
    ))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
impl Continuation {
    pub(crate) fn fence(&self) -> Result<(), String> {
        for before in &self.files {
            let (_, after) = read(&self.root, &before.path)?;
            if after.identity != before.identity || after.digest != before.digest {
                return Err("Native history changed before continuation.".into());
            }
        }
        for path in &self.absent {
            match fs::symlink_metadata(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err("Native history changed before continuation.".into()),
            }
        }
        Ok(())
    }
}
/// A supervised Pi run names one exact session file rather than searching for
/// the most recent terminal session. Missing files must never create a new run.
pub(crate) fn pi_file(
    root: &Path,
    path: &Path,
    session: &str,
    cwd: &str,
) -> Result<Continuation, String> {
    super::environment::check_private_directory(root)?;
    let (bytes, fence) = read(root, path)?;
    let entries = json_lines(&bytes)?;
    let header = entries.first().ok_or_else(missing)?;
    if header["type"] != "session"
        || header["id"] != session
        || header["cwd"] != cwd
        || !entries.iter().any(|entry| entry["type"] == "message")
    {
        return Err(missing());
    }
    Ok(Continuation {
        arguments: vec!["--session".into(), path.as_os_str().into()],
        files: vec![fence],
        absent: vec![],
        root: root.into(),
    })
}
fn json_lines(bytes: &[u8]) -> Result<Vec<Value>, String> {
    std::str::from_utf8(bytes)
        .map_err(|_| missing())?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            if l.len() > 4 * 1024 * 1024 {
                return Err(missing());
            }
            serde_json::from_str(l).map_err(|_| missing())
        })
        .collect()
}
fn collect(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), String> {
    ancestry(root, &dir.join("placeholder"))?;
    if depth > 6 || out.len() > 2048 {
        return Err(missing());
    }
    for entry in fs::read_dir(dir).map_err(|_| missing())? {
        let path = entry.map_err(|_| missing())?.path();
        let m = fs::symlink_metadata(&path).map_err(|_| missing())?;
        if m.file_type().is_symlink() {
            return Err(missing());
        }
        if m.is_dir() {
            collect(root, &path, depth + 1, out)?;
        } else if path.extension().is_some_and(|v| v == "jsonl") {
            out.push(path);
        }
    }
    Ok(())
}
fn snapshot(
    root: &Path,
    path: &Path,
    result: &mut Continuation,
) -> Result<(tempfile::TempDir, rusqlite::Connection), String> {
    let scratch = tempfile::Builder::new()
        .prefix("lomi-native-history-")
        .tempdir()
        .map_err(|_| missing())?;
    for suffix in ["", "-wal"] {
        let source = PathBuf::from(format!("{}{suffix}", path.display()));
        match fs::symlink_metadata(&source) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !suffix.is_empty() => {
                result.absent.push(source);
            }
            _ => {
                let (bytes, fence) = read(root, &source)?;
                let target = scratch.path().join(format!("history.db{suffix}"));
                fs::write(&target, bytes).map_err(|_| missing())?;
                crate::chat::storage::private(&target, false)?;
                result.files.push(fence);
            }
        }
    }
    let db = rusqlite::Connection::open_with_flags(
        scratch.path().join("history.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| missing())?;
    db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON;")
        .map_err(|_| missing())?;
    let check: String = db
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|_| missing())?;
    if check != "ok" {
        return Err(missing());
    }
    Ok((scratch, db))
}
// Validate complete protobuf framing; semantic decoding remains with native agy.
fn protobuf(bytes: &[u8]) -> bool {
    fn varint(bytes: &[u8], offset: &mut usize) -> Option<u64> {
        let mut value = 0u64;
        for i in 0..10 {
            let byte = *bytes.get(*offset)?;
            *offset += 1;
            if i == 9 && byte > 1 {
                return None;
            }
            value |= u64::from(byte & 0x7f) << (i * 7);
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    if bytes.is_empty() || bytes.len() > 16 * 1024 * 1024 {
        return false;
    }
    let mut offset = 0;
    while offset < bytes.len() {
        let Some(tag) = varint(bytes, &mut offset) else {
            return false;
        };
        if tag >> 3 == 0 || tag >> 3 > (1 << 29) - 1 {
            return false;
        }
        let size = match tag & 7 {
            0 => {
                if varint(bytes, &mut offset).is_none() {
                    return false;
                }
                0
            }
            1 => 8,
            2 => {
                let Some(size) = varint(bytes, &mut offset).and_then(|v| usize::try_from(v).ok())
                else {
                    return false;
                };
                size
            }
            5 => 4,
            _ => return false,
        };
        let Some(next) = offset.checked_add(size) else {
            return false;
        };
        if next > bytes.len() {
            return false;
        }
        offset = next;
    }
    true
}
#[cfg(test)]
pub(crate) fn select(
    root: &Path,
    cli: TitleCli,
    version: &str,
    cwd: &str,
) -> Result<Continuation, String> {
    select_inner(root, cli, version, cwd, None)
}
/// Account recovery must name the saved run's exact native session. It never
/// falls back to another session in the same workspace.
pub(crate) fn select_account(
    root: &Path,
    cli: TitleCli,
    version: &str,
    cwd: &str,
    session: &str,
) -> Result<Continuation, String> {
    if !identifier(session) || root.canonicalize().map_err(|_| missing())? != root {
        return Err(missing());
    }
    let result = select_inner(root, cli, version, cwd, Some(session))?;
    result.fence()?;
    Ok(result)
}
fn select_inner(
    root: &Path,
    cli: TitleCli,
    version: &str,
    cwd: &str,
    desired: Option<&str>,
) -> Result<Continuation, String> {
    super::environment::check_private_directory(root)?;
    let mut result = Continuation {
        arguments: vec![],
        files: vec![],
        absent: vec![],
        root: root.into(),
    };
    if cli == TitleCli::Grok {
        if let Some(session) = desired {
            // Pinned Grok treats UUIDs as strict IDs, never titles. Its native
            // resolver also permits other-cwd and remote restores, so require
            // the exact existing local workspace before exposing --resume.
            if session.len() != 36
                || !session.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                })
            {
                return Err(missing());
            }
            // Native paths.rs uses URL encoding up to 255 bytes, then blake3
            // plus a cwd marker. The hashed family remains unqualified here.
            let encoded: String = cwd
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                        char::from(b).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            if encoded.len() > 255 {
                return Err(missing());
            }
            let directory = root.join("sessions").join(encoded).join(session);
            let (summary, summary_fence) = read(root, &directory.join("summary.json"))?;
            let summary: Value = serde_json::from_slice(&summary).map_err(|_| missing())?;
            if summary["info"]["id"] != session
                || summary["info"]["cwd"] != cwd
                || !summary["parent_session_id"].is_null()
                || matches!(
                    summary["session_kind"].as_str(),
                    Some("subagent" | "subagent_fork")
                )
                || summary["num_chat_messages"].as_u64().is_none_or(|n| n == 0)
                || summary["chat_format_version"] != 1
            {
                return Err(missing());
            }
            let (history, history_fence) = read(root, &directory.join("chat_history.jsonl"))?;
            if !json_lines(&history)?.iter().any(|entry| {
                entry["type"] == "user"
                    && entry["content"].as_array().is_some_and(|parts| {
                        !parts.is_empty()
                            && parts.iter().all(|part| {
                                part["type"] == "text" && part["text"].is_string()
                                    || part["type"] == "image" && part["url"].is_string()
                            })
                    })
            }) {
                return Err(missing());
            }
            let (updates, updates_fence) = read(root, &directory.join("updates.jsonl"))?;
            let updates = json_lines(&updates)?;
            if updates.is_empty()
                || updates.iter().any(|entry| {
                    !matches!(
                        entry["method"].as_str(),
                        Some("session/update" | "_x.ai/session/update")
                    ) || entry["params"]["sessionId"] != session
                })
            {
                return Err(missing());
            }
            result.arguments = vec!["--resume".into(), session.into()];
            result
                .files
                .extend([summary_fence, history_fence, updates_fence]);
            return Ok(result);
        }
    }
    if cli == TitleCli::Grok || cli == TitleCli::Kimi && version == "1.52.0" {
        if desired.is_some() {
            return Err("This native client has no qualified exact-session interactive recovery reader. Existing history was preserved.".into());
        }
        // These source-pinned flags return an error when the current workspace
        // has no saved session, instead of silently creating one.
        result.arguments.push(
            if cli == TitleCli::Grok {
                "--resume"
            } else {
                "--continue"
            }
            .into(),
        );
        return Ok(result);
    }
    if cli == TitleCli::Agy {
        let home = root.join(".gemini/antigravity-cli");
        let (mapping, fence) = read(root, &home.join("cache/last_conversations.json"))?;
        let mapping: Value = serde_json::from_slice(&mapping).map_err(|_| missing())?;
        let id = mapping
            .get(cwd)
            .and_then(Value::as_str)
            .filter(|v| identifier(v))
            .ok_or_else(missing)?;
        if desired.is_some_and(|session| session != id) {
            return Err(missing());
        }
        result.files.push(fence);
        let (_scratch, db) = snapshot(
            root,
            &home.join("conversations").join(format!("{id}.db")),
            &mut result,
        )?;
        let metadata_id: String = db
            .query_row(
                "SELECT trajectory_id FROM trajectory_meta LIMIT 1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| missing())?;
        let rows: i64 = db
            .query_row("SELECT COUNT(*) FROM trajectory_meta", [], |r| r.get(0))
            .map_err(|_| missing())?;
        let data: Vec<u8> = db
            .query_row(
                "SELECT data FROM trajectory_metadata_blob WHERE id='main'",
                [],
                |r| r.get(0),
            )
            .map_err(|_| missing())?;
        let (steps, minimum): (i64, Option<i64>) = db
            .query_row("SELECT COUNT(*),MIN(idx) FROM steps", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|_| missing())?;
        if rows != 1
            || metadata_id != id
            || steps < 1
            || minimum.is_none_or(|v| v < 0)
            || !protobuf(&data)
        {
            return Err(missing());
        }
        result.arguments = vec!["--conversation".into(), id.into()];
        result.fence()?;
        return Ok(result);
    }
    if matches!(cli, TitleCli::Kilo | TitleCli::Opencode) {
        let name = if cli == TitleCli::Kilo {
            "kilo"
        } else {
            "opencode"
        };
        let path = root.join("data").join(name).join(format!("{name}.db"));
        let (_scratch, db) = snapshot(root, &path, &mut result)?;
        let history = if desired.is_some() {
            // Native versions may contain one or both storage generations.
            let mut clauses = Vec::new();
            for table in ["message", "session_message"] {
                let exists: bool = db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                        [table],
                        |r| r.get(0),
                    )
                    .map_err(|_| missing())?;
                if exists {
                    clauses.push(format!(
                        "EXISTS(SELECT 1 FROM {table} WHERE session_id=session.id)"
                    ));
                }
            }
            if clauses.is_empty() {
                return Err(missing());
            }
            format!("({})", clauses.join(" OR "))
        } else {
            "(EXISTS(SELECT 1 FROM message WHERE session_id=session.id) OR EXISTS(SELECT 1 FROM session_message WHERE session_id=session.id))".into()
        };
        let query = format!("SELECT id FROM session WHERE directory=?1 AND parent_id IS NULL AND {history} AND (?2 IS NULL OR id=?2) ORDER BY time_updated DESC,id DESC LIMIT 1");
        let id: String = db
            .query_row(&query, rusqlite::params![cwd, desired], |r| r.get(0))
            .map_err(|_| missing())?;
        if !identifier(&id) {
            return Err(missing());
        }
        result.arguments = vec!["--session".into(), id.into()];
        result.fence()?;
        return Ok(result);
    }
    if cli == TitleCli::Kimi {
        let home: PathBuf = if desired.is_some() {
            root.into()
        } else {
            root.join("kimi")
        };
        let (index, fence) = read(root, &home.join("session_index.jsonl"))?;
        result.files.push(fence);
        let mut entries = std::collections::BTreeMap::new();
        for entry in json_lines(&index)? {
            let Some(id) = entry["sessionId"].as_str().filter(|v| identifier(v)) else {
                continue;
            };
            if entry["deleted"] == true {
                entries.remove(id);
            } else if entry["sessionDir"].is_string() && entry["workDir"].is_string() {
                entries.insert(id.to_owned(), entry);
            }
        }
        let mut latest: Option<(u64, String, PathBuf, FileFence)> = None;
        for (id, entry) in entries {
            if entry["workDir"] != cwd || desired.is_some_and(|session| session != id) {
                continue;
            }
            let session = PathBuf::from(entry["sessionDir"].as_str().ok_or_else(missing)?);
            if !session.is_absolute()
                || !session.starts_with(home.join("sessions"))
                || session.file_name().is_none_or(|name| name != id.as_str())
                || session
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(missing());
            }
            let (state, state_fence) = read(root, &session.join("state.json"))?;
            let state: Value = serde_json::from_slice(&state).map_err(|_| missing())?;
            if state["version"] != 2 || state["id"] != id || state["cwd"] != cwd {
                return Err(missing());
            }
            if state["archived"] == true {
                continue;
            }
            let updated = state["updatedAt"]
                .as_u64()
                .filter(|v| *v > 0)
                .ok_or_else(missing)?;
            if latest.as_ref().is_none_or(|(time, previous, _, _)| {
                (updated, id.as_str()) > (*time, previous.as_str())
            }) {
                latest = Some((updated, id, session, state_fence));
            }
        }
        if let Some((_, id, session, state)) = latest {
            let (wire, wire_fence) = read(root, &session.join("agents/main/wire.jsonl"))?;
            if !json_lines(&wire)?.iter().any(|e| {
                e["type"] == "context.append_message"
                    && matches!(e["message"]["role"].as_str(), Some("user" | "assistant"))
            }) {
                return Err(missing());
            }
            result.files.extend([state, wire_fence]);
            result.arguments = vec!["--session".into(), id.into()];
            return Ok(result);
        }
        return Err(missing());
    }
    let directory = match cli {
        TitleCli::Pi if desired.is_none() => root.join("pi/sessions"),
        TitleCli::Claude => root.join(if desired.is_some() {
            "projects"
        } else {
            "claude/projects"
        }),
        TitleCli::Codex => root.join(if desired.is_some() {
            "sessions"
        } else {
            "codex/sessions"
        }),
        _ => return Err(missing()),
    };
    let mut files = Vec::new();
    collect(root, &directory, 0, &mut files)?;
    files.sort_by_key(|p| {
        fs::metadata(p)
            .map(|m| (m.mtime(), m.mtime_nsec()))
            .unwrap_or_default()
    });
    for path in files.into_iter().rev() {
        if cli == TitleCli::Claude
            && (path.components().any(|c| c.as_os_str() == "subagents")
                || path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("agent-")))
        {
            continue;
        }
        let (bytes, fence) = read(root, &path)?;
        let entries = json_lines(&bytes)?;
        let Some(first) = entries.first() else {
            continue;
        };
        let id = match cli {
            TitleCli::Pi
                if first["type"] == "session"
                    && first["cwd"] == cwd
                    && entries.iter().any(|e| e["type"] == "message") =>
            {
                first["id"].as_str()
            }
            TitleCli::Codex
                if first["type"] == "session_meta"
                    && first["payload"]["cwd"] == cwd
                    && (first["payload"]["source"] == "cli"
                        || desired.is_some() && first["payload"]["source"] == "mcp")
                    && first["payload"]["parent_thread_id"].is_null()
                    && entries.iter().any(|e| e["type"] == "response_item") =>
            {
                first["payload"]["id"].as_str()
            }
            TitleCli::Claude => entries
                .iter()
                .find(|e| {
                    e["cwd"] == cwd
                        && e["isSidechain"] != true
                        && matches!(e["type"].as_str(), Some("user" | "assistant"))
                })
                .and_then(|e| e["sessionId"].as_str()),
            _ => None,
        };
        let Some(id) = id.filter(|v| identifier(v)) else {
            continue;
        };
        if desired.is_some_and(|session| session != id) {
            continue;
        }
        if desired.is_some() && cli == TitleCli::Claude {
            // A mixed transcript must not qualify via a single matching row.
            if path.file_stem().is_none_or(|name| name != id)
                || entries
                    .iter()
                    .filter(|e| matches!(e["type"].as_str(), Some("user" | "assistant")))
                    .any(|e| e["sessionId"] != id || e["cwd"] != cwd || e["isSidechain"] == true)
            {
                return Err(missing());
            }
        }
        result.arguments = match cli {
            TitleCli::Pi => vec!["--session".into(), path.clone().into_os_string()],
            TitleCli::Codex => vec!["resume".into(), id.into()],
            _ => vec!["--resume".into(), id.into()],
        };
        result.files.push(fence);
        return Ok(result);
    }
    Err(missing())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn storage() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        crate::chat::storage::private(root.path(), true).unwrap();
        root
    }
    #[test]
    fn account_codex_recovers_exact_app_server_session_and_fences_it() {
        let storage = storage();
        let root = storage.path().canonicalize().unwrap();
        let dir = root.join("sessions");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("owned.jsonl");
        fs::write(&file, "{\"type\":\"session_meta\",\"payload\":{\"id\":\"owned\",\"cwd\":\"/project\",\"source\":\"mcp\"}}\n{\"type\":\"response_item\",\"payload\":{}}\n").unwrap();
        fs::write(dir.join("other.jsonl"), "{\"type\":\"session_meta\",\"payload\":{\"id\":\"other\",\"cwd\":\"/project\",\"source\":\"cli\"}}\n{\"type\":\"response_item\",\"payload\":{}}\n").unwrap();
        let continuation =
            select_account(&root, TitleCli::Codex, "0.160.0", "/project", "owned").unwrap();
        assert_eq!(
            continuation.arguments,
            vec![OsString::from("resume"), OsString::from("owned")]
        );
        assert!(select_account(&root, TitleCli::Codex, "0.160.0", "/project", "missing").is_err());
        assert!(select_account(&root, TitleCli::Codex, "0.160.0", "/other", "owned").is_err());
        fs::write(&file, "{}").unwrap();
        assert!(continuation.fence().is_err());
    }
    #[test]
    fn account_grok_requires_exact_local_uuid_workspace_and_fences_history() {
        let storage = storage();
        let root = storage.path().canonicalize().unwrap();
        let id = "12345678-1234-1234-1234-123456789abc";
        let directory = root.join("sessions/%2Fproject").join(id);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("summary.json"),
            serde_json::json!({
                "info": {"id": id, "cwd": "/project"}, "num_chat_messages": 1,
                "chat_format_version": 1,
            })
            .to_string(),
        )
        .unwrap();
        let history = directory.join("chat_history.jsonl");
        fs::write(
            &history,
            "{\"type\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}\n",
        )
        .unwrap();
        fs::write(directory.join("updates.jsonl"), serde_json::json!({
            "method": "session/update", "params": {"sessionId": id, "update": {
                "sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "hello"}
            }}
        }).to_string()).unwrap();
        let continuation =
            select_account(&root, TitleCli::Grok, "reviewed", "/project", id).unwrap();
        assert_eq!(
            continuation.arguments,
            vec![OsString::from("--resume"), OsString::from(id)]
        );
        assert!(select_account(&root, TitleCli::Grok, "reviewed", "/other", id).is_err());
        assert!(select_account(
            &root,
            TitleCli::Grok,
            "reviewed",
            "/project",
            "session-title"
        )
        .is_err());
        assert!(select_account(
            &root,
            TitleCli::Grok,
            "reviewed",
            "/project",
            "12345678-1234-1234-1234-000000000000"
        )
        .is_err());
        fs::write(&history, "{}").unwrap();
        assert!(continuation.fence().is_err());
    }
    #[test]
    fn account_claude_rejects_mixed_identity_and_gateway_layout() {
        let storage = storage();
        let root = storage.path().canonicalize().unwrap();
        let dir = root.join("projects/project");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("owned.jsonl");
        fs::write(
            &file,
            "{\"type\":\"user\",\"sessionId\":\"owned\",\"cwd\":\"/project\"}\n",
        )
        .unwrap();
        let continuation =
            select_account(&root, TitleCli::Claude, "reviewed", "/project", "owned").unwrap();
        assert_eq!(
            continuation.arguments,
            vec![OsString::from("--resume"), OsString::from("owned")]
        );
        assert!(select(&root, TitleCli::Claude, "reviewed", "/project").is_err());
        fs::write(&file, "{\"type\":\"user\",\"sessionId\":\"owned\",\"cwd\":\"/project\"}\n{\"type\":\"assistant\",\"sessionId\":\"other\",\"cwd\":\"/project\"}\n").unwrap();
        assert!(select_account(&root, TitleCli::Claude, "reviewed", "/project", "owned").is_err());
    }
    #[test]
    fn account_sqlite_recovers_older_exact_id_with_one_storage_generation() {
        for cli in [TitleCli::Kilo, TitleCli::Opencode] {
            let storage = storage();
            let root = storage.path().canonicalize().unwrap();
            let name = if cli == TitleCli::Kilo {
                "kilo"
            } else {
                "opencode"
            };
            let dir = root.join("data").join(name);
            fs::create_dir_all(&dir).unwrap();
            let db = rusqlite::Connection::open(dir.join(format!("{name}.db"))).unwrap();
            db.execute_batch("CREATE TABLE session(id TEXT,directory TEXT,parent_id TEXT,time_updated INTEGER); CREATE TABLE session_message(session_id TEXT); INSERT INTO session VALUES('owned','/project',NULL,1),('other','/project',NULL,100),('child','/project','owned',101); INSERT INTO session_message VALUES('owned'),('other'),('child');").unwrap();
            let continuation = select_account(&root, cli, "reviewed", "/project", "owned").unwrap();
            assert_eq!(
                continuation.arguments,
                vec![OsString::from("--session"), OsString::from("owned")]
            );
            assert!(select_account(&root, cli, "reviewed", "/project", "child").is_err());
            assert!(select_account(&root, cli, "reviewed", "/project", "missing").is_err());
            assert!(select_account(&root, cli, "reviewed", "/other", "owned").is_err());
        }
    }
    #[test]
    fn account_kimi_uses_exact_live_index_entry_and_private_account_root() {
        let storage = storage();
        let root = storage.path().canonicalize().unwrap();
        let session = root.join("sessions/workspace/owned");
        fs::create_dir_all(session.join("agents/main")).unwrap();
        fs::write(
            session.join("state.json"),
            r#"{"version":2,"id":"owned","cwd":"/project","updatedAt":1}"#,
        )
        .unwrap();
        fs::write(
            session.join("agents/main/wire.jsonl"),
            "{\"type\":\"context.append_message\",\"message\":{\"role\":\"user\"}}\n",
        )
        .unwrap();
        let entry =
            serde_json::json!({"sessionId":"owned","sessionDir":session,"workDir":"/project"});
        fs::write(root.join("session_index.jsonl"), format!("{entry}\n")).unwrap();
        let continuation =
            select_account(&root, TitleCli::Kimi, "2.1.1", "/project", "owned").unwrap();
        assert_eq!(
            continuation.arguments,
            vec![OsString::from("--session"), OsString::from("owned")]
        );
        assert!(select_account(&root, TitleCli::Kimi, "2.1.1", "/project", "missing").is_err());
        fs::write(
            root.join("session_index.jsonl"),
            format!("{entry}\n{{\"sessionId\":\"owned\",\"deleted\":true}}\n"),
        )
        .unwrap();
        assert!(select_account(&root, TitleCli::Kimi, "2.1.1", "/project", "owned").is_err());
    }
    #[test]
    fn codex_resume_excludes_newer_child_rollouts() {
        let root = storage();
        let dir = root.path().join("codex/sessions");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("main.jsonl"),"{\"type\":\"session_meta\",\"payload\":{\"id\":\"main\",\"cwd\":\"/project\",\"source\":\"cli\"}}\n{\"type\":\"response_item\",\"payload\":{}}\n").unwrap();
        fs::write(dir.join("child.jsonl"),"{\"type\":\"session_meta\",\"payload\":{\"id\":\"child\",\"parent_thread_id\":\"main\",\"cwd\":\"/project\",\"source\":{\"subagent\":\"review\"}}}\n{\"type\":\"response_item\",\"payload\":{}}\n").unwrap();
        let native = select(root.path(), TitleCli::Codex, "0.160.0", "/project").unwrap();
        assert_eq!(
            native.arguments,
            vec![OsString::from("resume"), OsString::from("main")]
        );
    }
    #[test]
    fn pi_resume_requires_existing_workspace_messages_and_fences_exact_history() {
        let root = storage();
        fs::create_dir_all(root.path().join("pi/sessions")).unwrap();
        assert!(select(root.path(), TitleCli::Pi, "1.0.1", "/project").is_err());
        let file = root.path().join("pi/sessions/owned.jsonl");
        fs::write(
            &file,
            "{\"type\":\"session\",\"id\":\"owned\",\"cwd\":\"/project\"}\n",
        )
        .unwrap();
        assert!(select(root.path(), TitleCli::Pi, "1.0.1", "/project").is_err());
        fs::write(&file,"{\"type\":\"session\",\"id\":\"owned\",\"cwd\":\"/project\"}\n{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"retained\"}}\n").unwrap();
        assert!(select(root.path(), TitleCli::Pi, "1.0.1", "/other").is_err());
        let session = select(root.path(), TitleCli::Pi, "1.0.1", "/project").unwrap();
        assert_eq!(
            session.arguments,
            vec![OsString::from("--session"), file.clone().into_os_string()]
        );
        session.fence().unwrap();
        fs::write(file, "changed history").unwrap();
        assert!(session.fence().is_err());
    }
    #[test]
    fn modern_kimi_resume_uses_an_existing_workdir_bound_session_id() {
        let root = storage();
        let session = root.path().join("kimi/sessions/owned");
        fs::create_dir_all(session.join("agents/main")).unwrap();
        let index =
            serde_json::json!({"sessionId":"owned","sessionDir":session,"workDir":"/project"});
        fs::write(
            root.path().join("kimi/session_index.jsonl"),
            format!("{index}\n"),
        )
        .unwrap();
        assert!(select(root.path(), TitleCli::Kimi, "2.1.1", "/project").is_err());
        fs::write(
            session.join("state.json"),
            r#"{"id":"owned","version":2,"cwd":"/project","updatedAt":10}"#,
        )
        .unwrap();
        fs::write(session.join("agents/main/wire.jsonl"),"{\"type\":\"context.append_message\",\"message\":{\"role\":\"user\",\"content\":\"retained\"}}\n").unwrap();
        let native = select(root.path(), TitleCli::Kimi, "2.1.1", "/project").unwrap();
        assert_eq!(
            native.arguments,
            vec![OsString::from("--session"), OsString::from("owned")]
        );
        assert!(select(root.path(), TitleCli::Kimi, "2.1.1", "/other").is_err());
    }
    #[test]
    fn kimi_recency_uses_metadata_not_creation_order_and_honors_deletion() {
        let root = storage();
        let home = root.path().join("kimi");
        let mut index = String::new();
        for (id, updated, content) in [("active", 30, true), ("newer-created", 10, false)] {
            let path = home.join("sessions").join(id);
            fs::create_dir_all(path.join("agents/main")).unwrap();
            let meta =
                serde_json::json!({"id":id,"version":2,"cwd":"/project","updatedAt":updated});
            fs::write(path.join("state.json"), meta.to_string()).unwrap();
            let wire = if content {
                r#"{"type":"context.append_message","message":{"role":"user","content":"retained"}}"#
            } else {
                r#"{"type":"metadata","protocol_version":"1.5"}"#
            };
            fs::write(path.join("agents/main/wire.jsonl"), format!("{wire}\n")).unwrap();
            index.push_str(&format!(
                "{}\n",
                serde_json::json!({"sessionId":id,"sessionDir":path,"workDir":"/project"})
            ));
        }
        fs::write(home.join("session_index.jsonl"), &index).unwrap();
        let native = select(root.path(), TitleCli::Kimi, "2.1.1", "/project").unwrap();
        assert_eq!(
            native.arguments,
            vec![OsString::from("--session"), OsString::from("active")]
        );
        index.push_str("{\"sessionId\":\"newer-created\",\"deleted\":true}\n");
        fs::write(home.join("session_index.jsonl"), &index).unwrap();
        assert_eq!(
            select(root.path(), TitleCli::Kimi, "2.1.1", "/project")
                .unwrap()
                .arguments,
            native.arguments
        );
        index.push_str("{\"sessionId\":\"active\",\"deleted\":true}\n");
        fs::write(home.join("session_index.jsonl"), index).unwrap();
        assert!(select(root.path(), TitleCli::Kimi, "2.1.1", "/project").is_err());
    }
    #[test]
    fn antigravity_resume_requires_owned_metadata_protobuf_and_actual_steps() {
        let root = storage();
        let home = root.path().join(".gemini/antigravity-cli");
        fs::create_dir_all(home.join("cache")).unwrap();
        fs::create_dir(home.join("conversations")).unwrap();
        fs::write(
            home.join("cache/last_conversations.json"),
            r#"{"/project":"owned"}"#,
        )
        .unwrap();
        let path = home.join("conversations/owned.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE trajectory_meta(trajectory_id TEXT,cascade_id TEXT); CREATE TABLE trajectory_metadata_blob(id TEXT,data BLOB); CREATE TABLE steps(idx INTEGER); INSERT INTO trajectory_meta VALUES('owned','distinct-cascade'); INSERT INTO trajectory_metadata_blob VALUES('main',X'0A0178');").unwrap();
        assert!(select(root.path(), TitleCli::Agy, "1.2.16", "/project").is_err());
        db.execute("INSERT INTO steps VALUES(0)", []).unwrap();
        let native = select(root.path(), TitleCli::Agy, "1.2.16", "/project").unwrap();
        assert_eq!(
            native.arguments,
            vec![OsString::from("--conversation"), OsString::from("owned")]
        );
        assert!(select(root.path(), TitleCli::Agy, "1.2.16", "/other").is_err());
        db.execute("UPDATE trajectory_metadata_blob SET data=X'0A08'", [])
            .unwrap();
        assert!(select(root.path(), TitleCli::Agy, "1.2.16", "/project").is_err());
    }
    #[test]
    fn sqlite_session_selection_requires_existing_top_level_workspace_history() {
        for cli in [TitleCli::Kilo, TitleCli::Opencode] {
            let root = storage();
            let name = if cli == TitleCli::Kilo {
                "kilo"
            } else {
                "opencode"
            };
            let data = root.path().join("data").join(name);
            fs::create_dir_all(&data).unwrap();
            let db = rusqlite::Connection::open(data.join(format!("{name}.db"))).unwrap();
            db.execute_batch("CREATE TABLE session(id TEXT,directory TEXT,parent_id TEXT,time_updated INTEGER); CREATE TABLE message(session_id TEXT); CREATE TABLE session_message(session_id TEXT); INSERT INTO session VALUES('owned','/project',NULL,1); INSERT INTO session VALUES('subagent','/project','owned',100); INSERT INTO message VALUES('subagent');").unwrap();
            assert!(select(root.path(), cli, "reviewed", "/project").is_err());
            db.execute("INSERT INTO message VALUES('owned')", [])
                .unwrap();
            let native = select(root.path(), cli, "reviewed", "/project").unwrap();
            assert_eq!(
                native.arguments,
                vec![OsString::from("--session"), OsString::from("owned")]
            );
            assert!(select(root.path(), cli, "reviewed", "/other").is_err());
            db.execute("DELETE FROM message WHERE session_id='owned'", [])
                .unwrap();
            db.execute("INSERT INTO session_message VALUES('owned')", [])
                .unwrap();
            assert_eq!(
                select(root.path(), cli, "reviewed", "/project")
                    .unwrap()
                    .arguments,
                native.arguments
            );
        }
    }
    #[test]
    fn protobuf_metadata_never_accepts_truncation_or_invalid_tags() {
        assert!(protobuf(&[10, 1, b'x']));
        for bytes in [&[][..], &[0][..], &[10, 8][..], &[128][..], &[15][..]] {
            assert!(!protobuf(bytes));
        }
    }
}
