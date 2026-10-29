//! Offline migration of explicit, drained copies. Never dispatches legacy work.
//! Publication callbacks must atomically import records and return a stable receipt;
//! repeated callbacks for a migration ID must validate/replay that receipt.
mod legacy_router;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_JSON: u64 = 32 * 1024 * 1024;
const MAX_FILES: usize = 100_000;
const MAX_ARCHIVE: u64 = 8 * 1024 * 1024 * 1024;
pub(crate) const LEGACY_KEYRING_SERVICE: &str = "dev.lomi.desktop.cli-router";
fn err(reason: impl std::fmt::Display) -> String {
    let _ = reason;
    "Migration refused; original data has been preserved".into()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) struct MigrationOptions {
    pub migration_id: String,
    pub source_root: PathBuf,
    pub session: PathBuf,
    pub archive_root: PathBuf,
    /// Both owners must be drained; only explicitly prepared copies are admitted.
    pub offline_copy: bool,
    /// Include the replacement store owner and any external side-store locks.
    pub additional_owner_locks: Vec<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ImportBundle {
    pub migration_id: String,
    pub source_schema: i64,
    pub snapshot: Value,
    pub requests: Vec<Value>,
    pub credential_journal: Vec<Value>,
    pub run_id_map: BTreeMap<String, String>,
    pub missing_run_ids: BTreeSet<String>,
    pub physical_auth_roots: BTreeMap<String, PathBuf>,
    pub keyring_service: String,
    pub archive: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ManifestEntry {
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: String,
    pub mode: u32,
    pub source_identity: String,
    pub sqlite_schema: Option<i64>,
    pub relationship: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Staged,
    PublishIntent,
    StorePublished,
    Committed,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    migration_id: String,
    phase: Phase,
    source_root: PathBuf,
    source_root_identity: String,
    session_identity: String,
    session: PathBuf,
    manifest_digest: String,
    session_digest: String,
    bundle_digest: String,
    publication_receipt: Option<String>,
}
pub(crate) struct PreparedMigration {
    bundle: ImportBundle,
    journal: Journal,
    _locks: Vec<fs::File>,
}

#[cfg(unix)]
fn checked(path: &Path) -> Result<fs::Metadata, String> {
    use std::os::unix::fs::MetadataExt;
    // Check every existing path component: canonicalization alone would follow links.
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(err("parent"));
        }
        current.push(component);
        let m = fs::symlink_metadata(&current).map_err(err)?;
        if m.file_type().is_symlink() {
            return Err(err("link"));
        }
    }
    let m = fs::symlink_metadata(path).map_err(err)?;
    if (!m.is_dir() && !m.is_file())
        || m.uid() != unsafe { libc::geteuid() }
        || (if m.is_dir() {
            m.mode() & 0o022 != 0
        } else {
            m.mode() & 0o077 != 0
        })
        || (m.is_file() && m.nlink() != 1)
    {
        return Err(err("permissions"));
    }
    Ok(m)
}
#[cfg(not(unix))]
fn checked(_: &Path) -> Result<fs::Metadata, String> {
    Err("Offline migration private storage is not qualified on this platform".into())
}
#[cfg(unix)]
fn identity(m: &fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        "{}:{}:{}:{}:{}:{}",
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.mode()
    )
}
#[cfg(not(unix))]
fn identity(_: &fs::Metadata) -> String {
    String::new()
}
#[cfg(unix)]
fn mode(m: &fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    m.mode() & 0o777
}
#[cfg(not(unix))]
fn mode(_: &fs::Metadata) -> u32 {
    0
}
fn private_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(err)?;
    }
    #[cfg(not(unix))]
    {
        return Err(err("platform"));
    }
    checked(path)?;
    Ok(())
}
fn staging_dir(parent: &Path, prefix: &str) -> Result<tempfile::TempDir, String> {
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(parent)
        .map_err(err)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).map_err(err)?;
    }
    checked(dir.path())?;
    Ok(dir)
}
fn sync_dir(path: &Path) -> Result<(), String> {
    fs::File::open(path).and_then(|f| f.sync_all()).map_err(err)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut f = o.open(path).map_err(err)?;
    f.write_all(bytes).map_err(err)?;
    f.sync_all().map_err(err)?;
    sync_dir(path.parent().ok_or_else(|| err("parent"))?)
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        checked(path)?;
    }
    let parent = path.parent().ok_or_else(|| err("parent"))?;
    checked(parent)?;
    let temp = parent.join(format!(".migration-{}", digest(bytes)));
    if temp.exists() {
        checked(&temp)?;
        if fs::read(&temp).map_err(err)? != bytes {
            return Err(err("conflict"));
        }
    } else {
        write_new(&temp, bytes)?;
    }
    fs::rename(&temp, path).map_err(err)?;
    sync_dir(parent)
}
fn lock(path: &Path) -> Result<fs::File, String> {
    checked(path.parent().ok_or_else(|| err("parent"))?)?;
    if path.exists() {
        checked(path)?;
    }
    let mut o = fs::OpenOptions::new();
    o.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
        o.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        let f = o.open(path).map_err(err)?;
        if identity(&f.metadata().map_err(err)?) != identity(&checked(path)?) {
            return Err(err("lock inode replaced"));
        }
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Migration refused: an owner is still active".into());
        }
        Ok(f)
    }
    #[cfg(not(unix))]
    {
        Err(err("platform"))
    }
}
fn inventory(root: &Path) -> Result<Vec<PathBuf>, String> {
    fn walk(p: &Path, out: &mut Vec<PathBuf>, bytes: &mut u64) -> Result<(), String> {
        if mode(&checked(p)?) & 0o077 != 0 {
            return Err(err("private directory"));
        }
        for e in fs::read_dir(p).map_err(err)? {
            let p = e.map_err(err)?.path();
            let m = checked(&p)?;
            if m.is_dir() {
                walk(&p, out, bytes)?;
            } else {
                *bytes = bytes.checked_add(m.len()).ok_or_else(|| err("size"))?;
                if *bytes > MAX_ARCHIVE || out.len() >= MAX_FILES {
                    return Err(err("size"));
                }
                out.push(p);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, &mut out, &mut 0)?;
    out.sort();
    Ok(out)
}
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let before = checked(path)?;
    if before.len() > max {
        return Err(err("size"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(err)?;
    if identity(&file.metadata().map_err(err)?) != identity(&before) {
        return Err(err("replaced file"));
    }
    let mut b = Vec::new();
    file.take(max + 1).read_to_end(&mut b).map_err(err)?;
    if b.len() as u64 > max {
        return Err(err("size"));
    }
    if identity(&before) != identity(&checked(path)?) {
        return Err(err("changed"));
    }
    Ok(b)
}
fn stream_copy(source: &Path, target: &Path) -> Result<(), String> {
    let before = checked(source)?;
    let mut input = fs::OpenOptions::new();
    input.read(true);
    let mut output = fs::OpenOptions::new();
    output.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        input.custom_flags(libc::O_NOFOLLOW);
        output.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut reader = input.open(source).map_err(err)?;
    let mut writer = output.open(target).map_err(err)?;
    if identity(&reader.metadata().map_err(err)?) != identity(&before) {
        return Err(err("identity"));
    }
    let copied = std::io::copy(
        &mut Read::by_ref(&mut reader).take(MAX_ARCHIVE + 1),
        &mut writer,
    )
    .map_err(err)?;
    if copied != before.len()
        || copied > MAX_ARCHIVE
        || identity(&checked(source)?) != identity(&before)
    {
        return Err(err("changed"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        writer
            .set_permissions(fs::Permissions::from_mode(mode(&before)))
            .map_err(err)?;
    }
    writer.sync_all().map_err(err)?;
    sync_dir(target.parent().ok_or_else(|| err("parent"))?)
}
fn file_digest(path: &Path) -> Result<String, String> {
    let before = checked(path)?;
    if before.len() > MAX_ARCHIVE {
        return Err(err("size"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(err)?;
    if identity(&file.metadata().map_err(err)?) != identity(&before) {
        return Err(err("replaced file"));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(err)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if identity(&before) != identity(&checked(path)?) {
        return Err(err("changed"));
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn db(path: &Path) -> Result<Connection, String> {
    checked(path)?;
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
    c.busy_timeout(std::time::Duration::from_secs(1))
        .map_err(err)?;
    let result: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(err)?;
    if result != "ok" {
        return Err(err("integrity"));
    }
    Ok(c)
}
fn table(c: &Connection, name: &str) -> Result<Vec<Value>, String> {
    let mut s = c
        .prepare(&format!("SELECT * FROM {name} ORDER BY rowid"))
        .map_err(err)?;
    let cols: Vec<String> = s.column_names().iter().map(|x| x.to_string()).collect();
    let mut rows = s.query([]).map_err(err)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(err)? {
        if out.len() >= 100_000 {
            return Err(err("records"));
        }
        let mut v = serde_json::Map::new();
        for (i, k) in cols.iter().enumerate() {
            let x = r.get_ref(i).map_err(err)?;
            let val = match x {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(n) => json!(n),
                rusqlite::types::ValueRef::Text(t) => {
                    if t.len() > MAX_JSON as usize {
                        return Err(err("size"));
                    }
                    json!(std::str::from_utf8(t).map_err(err)?)
                }
                _ => return Err(err("record type")),
            };
            v.insert(k.clone(), val);
        }
        out.push(Value::Object(v));
    }
    Ok(out)
}
fn validate_file_references(root: &Path) -> Result<(), String> {
    for path in inventory(root)? {
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let value: Value = serde_json::from_slice(&read_bounded(&path, MAX_JSON)?).map_err(err)?;
        if let Some(name) = value.get("sessionFile").and_then(Value::as_str) {
            let rel = Path::new(name);
            if rel.components().count() != 1
                || !matches!(
                    rel.components().next(),
                    Some(std::path::Component::Normal(_))
                )
            {
                return Err(err("session file reference"));
            }
            checked(&path.parent().ok_or_else(|| err("parent"))?.join(rel))?;
        }
        if path
            .file_name()
            .is_some_and(|n| n == "native-checkpoint.json")
        {
            if value["schema"].as_u64() != Some(1) {
                return Err(err("checkpoint schema"));
            }
            let generation = value["generation"]
                .as_u64()
                .ok_or_else(|| err("generation"))?;
            let record = path
                .parent()
                .ok_or_else(|| err("parent"))?
                .join(format!("attempt-{generation}.json"));
            if value["historyDigest"].as_str() != Some(file_digest(&record)?.as_str()) {
                return Err(err("checkpoint digest"));
            }
        }
    }
    Ok(())
}
fn session_v5(
    value: &mut Value,
    map: &mut BTreeMap<String, String>,
    known: &BTreeSet<String>,
    missing: &mut BTreeSet<String>,
) -> Result<(), String> {
    match value {
        Value::Object(o) => {
            if o.get("type").and_then(Value::as_str) == Some("cli-agent") {
                let id = o
                    .get("runId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| err("run ref"))?
                    .to_string();
                if id.is_empty() || id.len() > 256 {
                    return Err(err("id"));
                }
                let task = map
                    .entry(id.clone())
                    .or_insert_with(|| format!("legacy:{id}"))
                    .clone();
                if !known.contains(&id) {
                    missing.insert(id);
                }
                o.remove("runId");
                o.insert("type".into(), json!("agent-task"));
                o.insert("taskId".into(), json!(task));
            } else if o.get("type").and_then(Value::as_str) == Some("agent-task") {
                // The frontend may have already saved its v1–v4 projection as
                // v5 before the user reviews the data import. Keep that exact
                // task identity and still account for legacy placeholders.
                let task = o
                    .get("taskId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| err("task ref"))?;
                if task.is_empty() || task.len() > 263 {
                    return Err(err("task id"));
                }
                if let Some(id) = task.strip_prefix("legacy:") {
                    if id.is_empty() || id.len() > 256 {
                        return Err(err("legacy task id"));
                    }
                    map.insert(id.to_owned(), task.to_owned());
                    if !known.contains(id) {
                        missing.insert(id.to_owned());
                    }
                }
            }
            for v in o.values_mut() {
                session_v5(v, map, known, missing)?;
            }
        }
        Value::Array(a) => {
            for v in a {
                session_v5(v, map, known, missing)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn legacy_session_backup(
    session: &Path,
    current: &[u8],
    version: u64,
) -> Result<(Vec<u8>, PathBuf, u64), String> {
    if version <= 4 {
        return Ok((current.to_vec(), session.to_path_buf(), version));
    }
    // save_session preserves the exact pre-upgrade file under this name. A v5
    // projection cannot be handed to the old executable during rollback.
    for legacy_version in (1..=4).rev() {
        let path = session.with_file_name(format!("session.v{legacy_version}.json"));
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                let bytes = read_bounded(&path, MAX_JSON)?;
                let value: Value = serde_json::from_slice(&bytes).map_err(err)?;
                if value["version"].as_u64() != Some(legacy_version)
                    || !value["projects"].is_array()
                    || contains_new_task_descriptor(&value)
                {
                    return Err(err("invalid pre-upgrade session backup"));
                }
                return Ok((bytes, path, legacy_version));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(err(error)),
        }
    }
    Err("Migration refused: the exact session v1–v4 backup required for rollback is missing. Original data has been preserved.".into())
}

fn contains_new_task_descriptor(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.get("type").and_then(Value::as_str) == Some("agent-task")
                || object.values().any(contains_new_task_descriptor)
        }
        Value::Array(values) => values.iter().any(contains_new_task_descriptor),
        _ => false,
    }
}

/// Native callers hold the new runtime actor mutex and its already-acquired
/// owner lock throughout preview/apply. Paths are derived from app data only.
/// Legacy locks and immutable snapshot proof are established internally.
pub(crate) fn prepare_owned(
    app_data_root: &Path,
    migration_id: &str,
) -> Result<PreparedMigration, String> {
    checked(app_data_root)?;
    let root = app_data_root.canonicalize().map_err(err)?;
    let runtime = root.join("agent-runtime");
    checked(&runtime)?;
    let archives = runtime.join("migration-archives");
    if !archives.exists() {
        private_dir(&archives)?;
    }
    prepare_inner(
        &MigrationOptions {
            migration_id: migration_id.into(),
            source_root: root.join("cli-router"),
            session: root.join("session.json"),
            archive_root: archives,
            offline_copy: false,
            additional_owner_locks: Vec::new(),
        },
        true,
    )
}

fn prepare_inner(options: &MigrationOptions, app_owned: bool) -> Result<PreparedMigration, String> {
    if !options.offline_copy && !app_owned {
        return Err(
            "Migration is currently qualified only for explicit drained copies/fixtures".into(),
        );
    }
    if options.migration_id.is_empty()
        || options.migration_id.len() > 128
        || !options
            .migration_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(err("id"));
    }
    checked(&options.source_root)?;
    checked(&options.session)?;
    checked(&options.archive_root)?;
    let source = options.source_root.canonicalize().map_err(err)?;

    let session = options.session.canonicalize().map_err(err)?;
    let archive_root = options.archive_root.canonicalize().map_err(err)?;
    if mode(&checked(&archive_root)?) & 0o077 != 0 {
        return Err(err("archive privacy"));
    }
    if archive_root.starts_with(&source)
        || source.starts_with(&archive_root)
        || session.starts_with(&source)
    {
        return Err(err("overlap"));
    }
    let mut locks = vec![lock(&archive_root.join("migration.global.lock"))?];
    locks.push(lock(&session.with_extension("migration-session.lock"))?);
    let mut paths = inventory(&source)?;
    let mut owners = options.additional_owner_locks.clone();
    owners.push(source.join("router.owner.lock"));
    for p in &paths {
        if p.file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".lock"))
        {
            owners.push(p.clone());
        }
    }
    owners = owners
        .into_iter()
        .map(|path| {
            let parent = path.parent().ok_or_else(|| err("lock parent"))?;
            checked(parent)?;
            Ok(parent
                .canonicalize()
                .map_err(err)?
                .join(path.file_name().ok_or_else(|| err("lock name"))?))
        })
        .collect::<Result<Vec<_>, String>>()?;
    owners.sort();
    owners.dedup();
    for p in owners {
        locks.push(lock(&p)?);
    }
    // Repeat inventory after admission, because acquiring locks can add lock files.
    paths = inventory(&source)?;
    let source_root_identity = identity(&checked(&source)?);
    let destination = archive_root.join(&options.migration_id);
    if destination.exists() {
        checked(&destination)?;
        let journal: Journal =
            serde_json::from_slice(&read_bounded(&destination.join("journal.json"), MAX_JSON)?)
                .map_err(err)?;
        if journal.migration_id != options.migration_id
            || journal.source_root != source
            || journal.session != session
        {
            return Err(err("journal identity"));
        }
        let bundle_bytes = read_bounded(&destination.join("bundle.json"), MAX_JSON)?;
        if digest(&bundle_bytes) != journal.bundle_digest {
            return Err(err("bundle digest"));
        }
        let bundle: ImportBundle = serde_json::from_slice(&bundle_bytes).map_err(err)?;
        let prepared = PreparedMigration {
            bundle,
            journal,
            _locks: locks,
        };
        prepared.validate()?;
        return Ok(prepared);
    }
    // Physical snapshots are streamed before ANY SQLite connection opens.
    // SQLite only opens the disposable working copy, never source files.
    let physical = staging_dir(&archive_root, ".migration-physical-")?;
    copy_tree(&source, physical.path())?;
    let identities: BTreeMap<PathBuf, String> = paths
        .iter()
        .map(|p| Ok((p.clone(), identity(&checked(p)?))))
        .collect::<Result<_, String>>()?;
    let working = staging_dir(&archive_root, ".migration-sqlite-")?;
    copy_tree(physical.path(), working.path())?;
    validate_file_references(physical.path())?;
    let c = db(&working.path().join("router.sqlite"))?;
    let schema: i64 = c
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(err)?;
    if !(1..=5).contains(&schema) {
        return Err("Migration refused: unsupported legacy schema".into());
    }
    let count: i64 = c
        .query_row("SELECT count(*) FROM snapshot", [], |r| r.get(0))
        .map_err(err)?;
    if count != 1 {
        return Err(err("snapshot count"));
    }
    let size: i64 = c
        .query_row(
            "SELECT length(CAST(data AS BLOB)) FROM snapshot WHERE id=1",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if size < 0 || size as u64 > MAX_JSON {
        return Err(err("size"));
    }
    let text: String = c
        .query_row("SELECT data FROM snapshot WHERE id=1", [], |r| r.get(0))
        .map_err(err)?;
    let snapshot: Value = serde_json::from_str(&text).map_err(err)?;
    legacy_router::decode(&snapshot)?;
    let requests = table(&c, "requests")?;
    let revision = snapshot["revision"]
        .as_u64()
        .ok_or_else(|| err("revision"))?;
    for r in &requests {
        let id = r["id"].as_str().ok_or_else(|| err("request"))?;
        let hash = r["digest"].as_str().ok_or_else(|| err("request"))?;
        if id.is_empty()
            || id.len() > 512
            || hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(err("request"));
        }
        let rev = if schema == 1 {
            let v: Value = serde_json::from_str(r["result"].as_str().ok_or_else(|| err("result"))?)
                .map_err(err)?;
            legacy_router::decode(&v)?;
            v["revision"].as_u64().ok_or_else(|| err("revision"))?
        } else {
            let t = r["committed_revision"]
                .as_str()
                .ok_or_else(|| err("revision"))?;
            let n = t.parse::<u64>().map_err(err)?;
            if n.to_string() != t {
                return Err(err("revision"));
            }
            n
        };
        if rev > revision {
            return Err(err("revision"));
        }
    }
    let has_journal:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='credential_journal')",[],|r|r.get(0)).map_err(err)?;
    if schema >= 2 && !has_journal {
        return Err(err("missing journal"));
    }
    let journal_records = if has_journal {
        table(&c, "credential_journal")?
    } else {
        Vec::new()
    };
    for r in &journal_records {
        if !matches!(r["status"].as_str(), Some("staged" | "cleanup" | "remove"))
            || r["id"]
                .as_str()
                .is_none_or(|s| s.is_empty() || s.len() > 512)
            || r["profile_id"]
                .as_str()
                .is_none_or(|s| s.is_empty() || s.len() > 256)
        {
            return Err(err("credential journal"));
        }
    }
    let mut roots = BTreeMap::new();
    for p in snapshot["profiles"]
        .as_array()
        .ok_or_else(|| err("profiles"))?
    {
        let id = p["id"].as_str().ok_or_else(|| err("profile"))?;
        if !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(err("profile path"));
        }
        // Retired clients are archival metadata, never credential bindings.
        // Their previously removed homes need not exist to preserve history.
        let retired = p["cli"]
            .as_str()
            .is_some_and(crate::cli_catalog::is_retired_id);
        if matches!(p["storageMode"].as_str(), Some("cli_managed")) && !retired {
            let root = source.join("profiles").join(id);
            checked(&root)?;
            roots.insert(id.to_string(), root);
        }
    }
    let known: BTreeSet<String> = snapshot["runs"]
        .as_array()
        .ok_or_else(|| err("runs"))?
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| err("run"))
        })
        .collect::<Result<_, _>>()?;
    let mut map: BTreeMap<String, String> = known
        .iter()
        .map(|id| (id.clone(), format!("legacy:{id}")))
        .collect();
    let mut missing = BTreeSet::new();
    let session_identity = identity(&checked(&session)?);
    let original_session = read_bounded(&session, MAX_JSON)?;
    let mut next_session: Value = serde_json::from_slice(&original_session).map_err(err)?;
    let version = next_session["version"]
        .as_u64()
        .ok_or_else(|| err("session version"))?;
    if !(1..=5).contains(&version) {
        return Err(err("session schema"));
    }
    let (rollback_session, rollback_source, rollback_version) =
        legacy_session_backup(&session, &original_session, version)?;
    session_v5(&mut next_session, &mut map, &known, &mut missing)?;
    next_session["version"] = json!(5);
    let next_bytes = serde_json::to_vec(&next_session).map_err(err)?;
    let final_destination = destination.clone();
    let staged = staging_dir(&archive_root, ".migration-staged-")?;
    let destination = staged.path().to_path_buf();
    private_dir(&destination.join("originals"))?;
    private_dir(&destination.join("sqlite"))?;
    let mut manifest = Vec::new();
    for p in paths {
        let rel = p.strip_prefix(&source).map_err(err)?;
        let relative = rel.to_str().ok_or_else(|| err("filename"))?.to_string();
        let target = destination.join("originals").join(rel);
        let mut parent = destination.join("originals");
        for component in rel.parent().unwrap_or(Path::new("")).components() {
            parent.push(component);
            if !parent.exists() {
                private_dir(&parent)?;
            }
        }
        let frozen = physical.path().join(rel);
        let m = checked(&frozen)?;
        if identity(&checked(&p)?) != identities[&p] || file_digest(&p)? != file_digest(&frozen)? {
            return Err(err("changed source"));
        }
        stream_copy(&frozen, &target)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(mode(&m))).map_err(err)?;
            fs::File::open(&target)
                .and_then(|f| f.sync_all())
                .map_err(err)?;
        }
        let parts: Vec<_> = relative.split('/').collect();
        let mut relationship = json!({"runId":if parts.first()==Some(&"runs"){parts.get(1).copied()}else{None},"accountId":if parts.first()==Some(&"profiles"){parts.get(1).copied()}else{None},"source":"legacy_router","adoption":"archive_only"});
        if let Some(run_id) = relationship["runId"].as_str() {
            if let Some(run) = snapshot["runs"]
                .as_array()
                .and_then(|runs| runs.iter().find(|r| r["id"].as_str() == Some(run_id)))
            {
                for field in ["model", "cwd", "generation", "allowedProfileIds"] {
                    relationship[field] = run[field].clone();
                }
            }
        }
        let is_db = matches!(
            p.extension().and_then(|s| s.to_str()),
            Some("sqlite" | "sqlite3" | "db")
        );
        let sqlite_schema = if is_db {
            let connection = db(&working.path().join(rel))?;
            let s: i64 = connection
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .map_err(err)?;
            let logical = destination
                .join("sqlite")
                .join(format!("{}.sqlite", digest(relative.as_bytes())));
            write_new(&logical, &[])?;
            connection
                .backup(rusqlite::MAIN_DB, &logical, None)
                .map_err(err)?;
            fs::File::open(&logical)
                .and_then(|f| f.sync_all())
                .map_err(err)?;
            db(&logical)?;
            manifest.push(ManifestEntry {
                relative_path: format!("sqlite/{}", logical.file_name().unwrap().to_string_lossy()),
                bytes: fs::metadata(&logical).map_err(err)?.len(),
                sha256: file_digest(&logical)?,
                mode: 0o600,
                source_identity: identities[&p].clone(),
                sqlite_schema: Some(s),
                relationship: json!({"logicalBackupOf":relative}),
            });
            Some(s)
        } else {
            None
        };
        manifest.push(ManifestEntry {
            relative_path: format!("originals/{relative}"),
            bytes: m.len(),
            sha256: file_digest(&frozen)?,
            mode: mode(&m),
            source_identity: identities[&p].clone(),
            sqlite_schema,
            relationship,
        });
    }
    write_new(&destination.join("session.original"), &original_session)?;
    write_new(
        &destination.join("session.rollback.json"),
        &rollback_session,
    )?;
    write_new(&destination.join("session.v5.json"), &next_bytes)?;
    manifest.push(ManifestEntry {
        relative_path: "session.original".into(),
        bytes: original_session.len() as u64,
        sha256: digest(&original_session),
        mode: 0o600,
        source_identity: identity(&checked(&session)?),
        sqlite_schema: None,
        relationship: json!({"pairedSessionVersion":version}),
    });
    manifest.push(ManifestEntry {
        relative_path: "session.rollback.json".into(),
        bytes: rollback_session.len() as u64,
        sha256: digest(&rollback_session),
        mode: 0o600,
        source_identity: identity(&checked(&rollback_source)?),
        sqlite_schema: None,
        relationship: json!({"pairedLegacySessionVersion":rollback_version}),
    });
    let bundle = ImportBundle {
        migration_id: options.migration_id.clone(),
        source_schema: schema,
        snapshot,
        requests,
        credential_journal: journal_records,
        run_id_map: map,
        missing_run_ids: missing,
        physical_auth_roots: roots,
        keyring_service: LEGACY_KEYRING_SERVICE.into(),
        archive: final_destination.clone(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(err)?;
    let bundle_bytes = serde_json::to_vec(&bundle).map_err(err)?;
    write_new(&destination.join("manifest.json"), &manifest_bytes)?;
    write_new(&destination.join("bundle.json"), &bundle_bytes)?;
    let journal = Journal {
        migration_id: options.migration_id.clone(),
        phase: Phase::Staged,
        source_root: source,
        source_root_identity,
        session_identity,
        session,
        manifest_digest: digest(&manifest_bytes),
        session_digest: digest(&next_bytes),
        bundle_digest: digest(&bundle_bytes),
        publication_receipt: None,
    };
    let prepared = PreparedMigration {
        bundle,
        journal,
        _locks: locks,
    };
    atomic(
        &destination.join("journal.json"),
        &serde_json::to_vec(&prepared.journal).map_err(err)?,
    )?;
    sync_dir(&destination)?;
    fs::rename(&destination, &final_destination).map_err(err)?;
    sync_dir(&archive_root)?;
    prepared.validate()?;
    Ok(prepared)
}

impl PreparedMigration {
    pub(crate) fn bundle(&self) -> &ImportBundle {
        &self.bundle
    }
    pub(crate) fn reviewed_digest(&self) -> &str {
        &self.journal.bundle_digest
    }
    pub(crate) fn phase(&self) -> &Phase {
        &self.journal.phase
    }
    fn save_journal(&self) -> Result<(), String> {
        atomic(
            &self.bundle.archive.join("journal.json"),
            &serde_json::to_vec(&self.journal).map_err(err)?,
        )
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if matches!(self.journal.phase, Phase::StorePublished | Phase::Committed)
            && self
                .journal
                .publication_receipt
                .as_ref()
                .is_none_or(|r| r.is_empty())
        {
            return Err(err("publication state"));
        }
        let root = &self.bundle.archive;
        let bytes = read_bounded(&root.join("manifest.json"), MAX_JSON)?;
        if digest(&bytes) != self.journal.manifest_digest {
            return Err(err("manifest"));
        }
        let entries: Vec<ManifestEntry> = serde_json::from_slice(&bytes).map_err(err)?;
        for e in entries {
            let rel = Path::new(&e.relative_path);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err(err("manifest path"));
            }
            let p = root.join(rel);
            let b = read_bounded(&p, MAX_ARCHIVE)?;
            if b.len() as u64 != e.bytes || digest(&b) != e.sha256 || mode(&checked(&p)?) != e.mode
            {
                return Err(err("archive"));
            }
            if e.relative_path.starts_with("sqlite/") {
                db(&p)?;
            }
        }
        if digest(&read_bounded(&root.join("session.v5.json"), MAX_JSON)?)
            != self.journal.session_digest
            || digest(&read_bounded(&root.join("bundle.json"), MAX_JSON)?)
                != self.journal.bundle_digest
        {
            return Err(err("staging"));
        }
        Ok(())
    }
    fn validate_source(&self) -> Result<(), String> {
        if identity(&checked(&self.journal.source_root)?) != self.journal.source_root_identity {
            return Err(err("source root replaced"));
        }
        let entries: Vec<ManifestEntry> = serde_json::from_slice(&read_bounded(
            &self.bundle.archive.join("manifest.json"),
            MAX_JSON,
        )?)
        .map_err(err)?;
        for entry in entries {
            if let Some(rel) = entry.relative_path.strip_prefix("originals/") {
                let source = self.journal.source_root.join(rel);
                if identity(&checked(&source)?) != entry.source_identity
                    || file_digest(&source)? != entry.sha256
                {
                    return Err(err("source changed since review"));
                }
            }
        }
        let actual = file_digest(&self.journal.session)?;
        if actual != self.journal.session_digest
            && identity(&checked(&self.journal.session)?) != self.journal.session_identity
        {
            return Err(err("session replaced"));
        }
        let original = file_digest(&self.bundle.archive.join("session.original"))?;
        if actual != original
            && !(self.journal.phase == Phase::StorePublished
                && actual == self.journal.session_digest)
        {
            return Err(err("session changed since review"));
        }
        Ok(())
    }
    /// Dry-run is prepare + inspect bundle + drop; it does not invoke this method.
    pub(crate) fn publish(
        &mut self,
        mut import: impl FnMut(&ImportBundle) -> Result<String, String>,
    ) -> Result<(), String> {
        self.validate()?;
        if self.journal.phase == Phase::Committed {
            if digest(&read_bounded(&self.journal.session, MAX_JSON)?)
                != self.journal.session_digest
            {
                return Err(err("committed session"));
            }
            return Ok(());
        }
        self.validate_source()?;
        if self.journal.phase == Phase::Staged {
            self.journal.phase = Phase::PublishIntent;
            self.save_journal()?;
        }
        if self.journal.phase == Phase::PublishIntent {
            let receipt = import(&self.bundle)?;
            if receipt.is_empty() {
                return Err(err("publication receipt"));
            }
            self.journal.publication_receipt = Some(receipt);
            self.journal.phase = Phase::StorePublished;
            self.save_journal()?;
        }
        if self.journal.publication_receipt.is_none() {
            return Err(err("publication state"));
        }
        // Only a published store can introduce v5 task references into the session.
        let bytes = read_bounded(&self.bundle.archive.join("session.v5.json"), MAX_JSON)?;
        atomic(&self.journal.session, &bytes)?;
        self.journal.phase = Phase::Committed;
        self.save_journal()
    }
    /// Restore an isolated paired installation, retaining newer history privately.
    /// Caller must drain the new owner, whose lock is included in prepare options.
    /// This never mutates the original auth homes or executes an archived request.
    pub(crate) fn rollback_to(&self, target: &Path, newer_history: &Path) -> Result<(), String> {
        self.validate()?;
        checked(target)?;
        checked(newer_history)?;
        if target.starts_with(&self.journal.source_root)
            || self.journal.source_root.starts_with(target)
            || target.starts_with(&self.bundle.archive)
            || self.bundle.archive.starts_with(target)
            || target.starts_with(newer_history)
            || newer_history.starts_with(target)
        {
            return Err(err("overlap"));
        }
        if fs::read_dir(target).map_err(err)?.next().is_some() {
            return Err(err("rollback target must be empty"));
        }
        let newer = target.join("newer-history");
        private_dir(&newer)?;
        copy_tree(newer_history, &newer)?;
        let store = target.join("legacy-store");
        private_dir(&store)?;
        copy_tree(&self.bundle.archive.join("originals"), &store)?;
        // Physical WAL/SHM and exact old-app-compatible session bytes form a
        // matched pair. The autosaved v5 source remains separately archived.
        write_new(
            &target.join("session.json"),
            &read_bounded(&self.bundle.archive.join("session.rollback.json"), MAX_JSON)?,
        )?;
        write_new(&target.join("rollback.json"),&serde_json::to_vec(&json!({"migrationId":self.bundle.migration_id,"schema":self.bundle.source_schema,"replay":false,"newerHistory":"newer-history","credentialService":LEGACY_KEYRING_SERVICE,"physicalAuthRoots":self.bundle.physical_auth_roots})).map_err(err)?)?;
        db(&store.join("router.sqlite"))?;
        sync_dir(target)
    }
}
fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    for p in inventory(source)? {
        let rel = p.strip_prefix(source).map_err(err)?;
        let mut parent = target.to_path_buf();
        for c in rel.parent().unwrap_or(Path::new("")).components() {
            parent.push(c);
            if !parent.exists() {
                private_dir(&parent)?;
            }
        }
        let dest = target.join(rel);
        stream_copy(&p, &dest)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, fs::Permissions::from_mode(mode(&checked(&p)?)))
                .map_err(err)?;
        }
    }
    sync_dir(target)
}
