use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Emitter, Manager, State, Window};

const PACKAGE_LIMIT: usize = 64 * 1024 * 1024;
const FILE_LIMIT: usize = 20 * 1024 * 1024;
const JSON_LIMIT: usize = 256 * 1024;
#[derive(Default)]
pub struct Plugins {
    // Package operations serialize access to shared catalog and runtime state.
    inner: Mutex<Runtime>,
}
#[derive(Default)]
struct Runtime {
    evaluated: BTreeMap<String, (String, BTreeMap<String, String>)>,
    pending: BTreeMap<String, (String, String, String)>,
    statuses: BTreeMap<String, Status>,
    cleaned: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    schema_version: u32,
    id: String,
    name: String,
    version: String,
    description: String,
    host_api: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    activation: Option<String>,
    #[serde(default)]
    stylesheets: Vec<String>,
    #[serde(default)]
    contributes: Contributions,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contributions {
    #[serde(default)]
    views: Vec<View>,
    #[serde(default)]
    commands: Vec<Command>,
    #[serde(default)]
    keybindings: Vec<Binding>,
    #[serde(default)]
    fills: Vec<Fill>,
    #[serde(default)]
    themes: Vec<Theme>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct View {
    id: String,
    title: String,
    placement: String,
    multiple: bool,
    state_version: u32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    id: String,
    label: String,
    description: String,
    #[serde(default)]
    context: CommandContext,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    view_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_input: Option<bool>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    command: String,
    shortcut: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fill {
    id: String,
    slot: String,
    label: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Theme {
    id: String,
    path: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Installed {
    revision: String,
    source: String,
    enabled: bool,
    trusted_revision: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    version: u32,
    packages: BTreeMap<String, Installed>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            packages: BTreeMap::new(),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    id: String,
    #[serde(flatten)]
    installed: Installed,
    manifest: Option<Manifest>,
    error: Option<String>,
    restart_required: bool,
    theme_ids: Vec<String>,
    status: Option<Status>,
    evaluated: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    directory: String,
    entries: Vec<Entry>,
    safe_mode: bool,
}

fn authorize(label: &str, write: bool) -> Result<(), String> {
    if label == "settings" || (!write && label == "main") {
        Ok(())
    } else {
        Err("This plugin operation is not available in this window.".into())
    }
}
pub(crate) fn safe_mode() -> bool {
    std::env::args().any(|a| a == "--safe-mode" || a == "--disable-plugins")
        || std::env::var_os("LOMI_SAFE_MODE").is_some()
}
fn root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("plugins");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(root)
}
fn id(value: &str) -> Result<(), String> {
    if value.len() > 160
        || !value.contains('.')
        || value.split('.').any(|part| {
            !part.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        })
    {
        return Err("Plugin and contribution IDs must be namespaced lowercase identifiers.".into());
    }
    Ok(())
}
fn relative(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 512
        || value
            .chars()
            .any(|c| c.is_control() || "\\:%?#<>\"|*".contains(c))
        || value.split('/').any(|s| {
            let base = s.split('.').next().unwrap_or("").to_ascii_lowercase();
            s.is_empty()
                || s == "."
                || s == ".."
                || s.ends_with(['.', ' '])
                || ["con", "prn", "aux", "nul"].contains(&base.as_str())
                || ((base.starts_with("com") || base.starts_with("lpt"))
                    && base.len() == 4
                    && base.as_bytes()[3].is_ascii_digit())
        })
    {
        return Err("Invalid relative plugin path.".into());
    }
    Ok(())
}
fn text(value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > max || value.chars().any(char::is_control)
    {
        Err("Invalid plugin metadata text.".into())
    } else {
        Ok(())
    }
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("Expected a regular plugin file; symbolic links are not allowed.".into());
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Expected regular file.".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Plugin file exceeds its size limit.".into());
    }
    Ok(bytes)
}
fn inside(root: &Path, path: &str) -> Result<PathBuf, String> {
    relative(path)?;
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    let mut target = canonical.clone();
    for part in path.split('/') {
        target.push(part);
        if fs::symlink_metadata(&target)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Plugin paths cannot contain symbolic links.".into());
        }
    }
    if !target
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(canonical)
    {
        return Err("Plugin path escapes its package.".into());
    }
    Ok(target)
}
fn manifest(bytes: &[u8]) -> Result<Manifest, String> {
    if bytes.len() > JSON_LIMIT {
        return Err("plugin.json exceeds 256 KiB.".into());
    }
    let data: Manifest = serde_json::from_slice(bytes).map_err(|e| format!("plugin.json: {e}"))?;
    if data.schema_version != 1 || data.host_api != 1 {
        return Err("Unsupported plugin schema or host API version.".into());
    }
    id(&data.id)?;
    if data.id.len() > 100 {
        return Err("Plugin ID exceeds 100 bytes.".into());
    }
    text(&data.name, 160)?;
    text(&data.description, 2000)?;
    if data.version.len() > 80
        || !regex::Regex::new(r"^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$")
            .unwrap()
            .is_match(&data.version)
    {
        return Err("Invalid plugin version.".into());
    }
    if let Some(entry) = &data.entry {
        relative(entry)?;
        if !entry.ends_with(".js") && !entry.ends_with(".mjs") {
            return Err("Expected a prebuilt JavaScript entry.".into());
        }
    }
    if data
        .activation
        .as_deref()
        .is_some_and(|a| !["startup", "lazy"].contains(&a))
    {
        return Err("Unsupported activation trigger.".into());
    }
    if data.stylesheets.len() > 16 {
        return Err("Too many plugin stylesheets.".into());
    }
    let mut seen = BTreeSet::new();
    for path in &data.stylesheets {
        relative(path)?;
        if !path.ends_with(".css") || !seen.insert(path.clone()) {
            return Err("Invalid or duplicate stylesheet.".into());
        }
    }
    seen.clear();
    let c = &data.contributes;
    if [
        c.views.len(),
        c.commands.len(),
        c.keybindings.len(),
        c.fills.len(),
        c.themes.len(),
    ]
    .iter()
    .any(|n| *n > 128)
    {
        return Err("Too many plugin contributions.".into());
    }
    for key in c
        .views
        .iter()
        .map(|v| &v.id)
        .chain(c.commands.iter().map(|v| &v.id))
        .chain(c.fills.iter().map(|v| &v.id))
        .chain(c.themes.iter().map(|v| &v.id))
    {
        id(key)?;
        if !key.starts_with(&format!("{}.", data.id)) || !seen.insert(key.clone()) {
            return Err("Duplicate or foreign contribution ID.".into());
        }
    }
    for view in &c.views {
        text(&view.title, 160)?;
        if (view.placement == "sidebar" && view.multiple)
            || !["central", "sidebar"].contains(&view.placement.as_str())
            || !(1..=1000).contains(&view.state_version)
        {
            return Err("Invalid view contract.".into());
        }
    }
    for command in &c.commands {
        text(&command.label, 160)?;
        text(&command.description, 2000)?;
        if let Some(types) = &command.context.view_types {
            if types.len() > 128 || types.iter().any(|s| s.len() > 160) {
                return Err("Invalid command context.".into());
            }
        }
    }
    seen.clear();
    for binding in &c.keybindings {
        if !c.commands.iter().any(|v| v.id == binding.command)
            || !seen.insert(binding.command.clone())
        {
            return Err("Invalid or duplicate keybinding reference.".into());
        }
        if let Some(shortcut) = &binding.shortcut {
            if !crate::keybindings::valid_shortcut(shortcut) {
                return Err("Invalid plugin shortcut.".into());
            }
        }
    }
    for fill in &c.fills {
        text(&fill.label, 160)?;
        if !["statusbar", "sidebar-actions", "view-actions"].contains(&fill.slot.as_str()) {
            return Err("Unsupported plugin slot.".into());
        }
    }
    for theme in &c.themes {
        relative(&theme.path)?;
    }
    if data.entry.is_none()
        && !(c.views.is_empty()
            && c.commands.is_empty()
            && c.keybindings.is_empty()
            && c.fills.is_empty())
    {
        return Err("Executable contributions need an entry.".into());
    }
    Ok(data)
}
fn preferences(root: &Path) -> Result<Preferences, String> {
    let path = root.join("installed.json");
    if !path.exists() {
        return Ok(Preferences::default());
    }
    parse_preferences(&read(&path, JSON_LIMIT)?)
}
fn parse_preferences(bytes: &[u8]) -> Result<Preferences, String> {
    let data: Preferences = serde_json::from_slice(bytes)
        .map_err(|e| format!("Plugin preferences are invalid and were preserved: {e}"))?;
    if data.version != 1 || data.packages.len() > 128 {
        return Err("Unsupported plugin preferences; file preserved.".into());
    }
    for (key, value) in &data.packages {
        id(key)?;
        revision(&value.revision)?;
        if let Some(trust) = &value.trusted_revision {
            revision(trust)?;
        }
    }
    Ok(data)
}

#[cfg(unix)]
#[derive(Serialize)]
pub(crate) struct ShortcutContribution {
    id: String,
    label: String,
    shortcut: Option<String>,
}

/// Read immutable shortcut metadata while holding the same lock as package
/// lifecycle changes. The callback may publish a keybinding CAS under that lock.
#[cfg(unix)]
pub(crate) fn with_shortcut_definitions<T, E>(
    app: &tauri::AppHandle,
    check: &dyn Fn() -> Result<(), lomi_control_protocol::ErrorCode>,
    apply: impl FnOnce(String, Vec<ShortcutContribution>) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<lomi_control_protocol::ErrorCode>,
{
    use crate::settings_control::PreferenceSource;
    use lomi_control_core::project_files::ProjectDirectory;
    use lomi_control_protocol::ErrorCode;
    let state = app.state::<Plugins>();
    let _guard = state
        .inner
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    check()?;
    let source = PreferenceSource::open(app, "plugins/installed.json", JSON_LIMIT as u64, check)?;
    let prefs = source
        .bytes
        .as_deref()
        .map(parse_preferences)
        .transpose()
        .map_err(|_| ErrorCode::UnsupportedCapability)?
        .unwrap_or_default();
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| ErrorCode::StorageUnavailable)?
        .canonicalize()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let root = ProjectDirectory::open(&directory)?;
    let mut hash = Sha256::new();
    hash.update(source.revision.as_deref().unwrap_or("missing").as_bytes());
    let mut contributions = Vec::new();
    for (id, installed) in prefs.packages {
        check()?;
        let relative = format!("plugins/{id}/{}/plugin.json", installed.revision);
        hash.update(id.as_bytes());
        hash.update(installed.revision.as_bytes());
        match root
            .open_file(&relative, JSON_LIMIT as u64)
            .and_then(|file| file.read_bytes(JSON_LIMIT as u64, check))
        {
            Ok(bytes) => {
                hash.update(Sha256::digest(&bytes));
                if let Ok(manifest) = manifest(&bytes) {
                    for command in manifest.contributes.commands {
                        contributions.push(ShortcutContribution {
                            shortcut: manifest
                                .contributes
                                .keybindings
                                .iter()
                                .find(|b| b.command == command.id)
                                .and_then(|b| b.shortcut.clone()),
                            id: command.id,
                            label: command.label,
                        });
                        if contributions.len() > 4096 {
                            return Err(ErrorCode::ResourceExhausted.into());
                        }
                    }
                }
            }
            // A missing or invalid package has no commands in the existing provider.
            Err(
                ErrorCode::TargetNotFound
                | ErrorCode::ScopeDenied
                | ErrorCode::UnsupportedCapability,
            ) => hash.update(b"unavailable"),
            Err(error) => return Err(error.into()),
        }
    }
    if serde_json::to_vec(&contributions)
        .map_err(|_| ErrorCode::ResourceExhausted)?
        .len()
        > 256 * 1024
    {
        return Err(ErrorCode::ResourceExhausted.into());
    }
    check()?;
    root.check()?;
    apply(format!("{:x}", hash.finalize()), contributions)
}
fn revision(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        Err("Invalid package revision.".into())
    } else {
        Ok(())
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn snapshot(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    fn walk(
        root: &Path,
        path: &Path,
        result: &mut BTreeMap<String, Vec<u8>>,
        total: &mut usize,
        entries: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 16 {
            return Err("Plugin folder nesting exceeds 16 levels.".into());
        }
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            *entries += 1;
            if *entries > 2048 {
                return Err("Plugin package exceeds 2048 entries.".into());
            }
            let entry_path = entry.path();
            let relative_path = entry_path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("Package path is not UTF-8.")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            relative(&relative_path)?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err("Plugin packages cannot contain symbolic links.".into());
            }
            if kind.is_dir() {
                walk(root, &entry_path, result, total, entries, depth + 1)?;
            } else if kind.is_file() {
                let bytes = read(&inside(root, &relative_path)?, FILE_LIMIT)?;
                *total += bytes.len();
                if *total > PACKAGE_LIMIT {
                    return Err("Plugin package exceeds 64 MiB.".into());
                }
                result.insert(relative_path, bytes);
            } else {
                return Err("Plugin packages can contain only regular files and folders.".into());
            }
        }
        Ok(())
    }
    if fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Choose a real package folder, not a symbolic link.".into());
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files, &mut 0, &mut 0, 0)?;
    Ok(files)
}
fn snapshot_revision(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hash = Sha256::new();
    for (path, bytes) in files {
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}
fn validate_snapshot(files: &BTreeMap<String, Vec<u8>>) -> Result<Manifest, String> {
    let data = manifest(
        files
            .get("plugin.json")
            .ok_or("The folder needs plugin.json.")?,
    )?;
    for path in data.entry.iter().chain(data.stylesheets.iter()) {
        if !files.contains_key(path) {
            return Err(format!("Missing package file: {path}"));
        }
    }
    for theme in &data.contributes.themes {
        if !files.contains_key(&format!("{}/theme.jsonc", theme.path))
            && !files.contains_key(&format!("{}/theme.json", theme.path))
        {
            return Err(format!("Missing theme in {}", theme.path));
        }
    }
    Ok(data)
}
fn install(root: &Path, source: &Path) -> Result<String, String> {
    let mut prefs = preferences(root)?;
    let files = snapshot(source)?;
    let manifest = validate_snapshot(&files)?;
    if prefs.packages.get(&manifest.id).is_some_and(|v| v.enabled) {
        return Err(
            "Disable this plugin before replacing it. Unsaved panels must be resolved first."
                .into(),
        );
    }
    if prefs.packages.len() >= 128 && !prefs.packages.contains_key(&manifest.id) {
        return Err("The plugin library is limited to 128 packages.".into());
    }
    let revision = snapshot_revision(&files);
    let parent = root.join(&manifest.id);
    fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    inside(root, &manifest.id)?;
    let target = parent.join(&revision);
    if !target.exists() {
        let stage = tempfile::Builder::new()
            .prefix(".install-")
            .tempdir_in(root)
            .map_err(|e| e.to_string())?;
        for (path, bytes) in &files {
            let destination = stage.path().join(path);
            fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
            let mut file = fs::File::create(destination).map_err(|e| e.to_string())?;
            file.write_all(bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
        }
        fs::rename(stage.path(), &target).map_err(|e| e.to_string())?;
    } else if snapshot_revision(&snapshot(&target)?) != revision {
        return Err("The installed snapshot changed. It cannot inherit trust.".into());
    }
    prefs.packages.insert(
        manifest.id.clone(),
        Installed {
            revision,
            source: source.to_string_lossy().into_owned(),
            enabled: false,
            trusted_revision: None,
        },
    );
    crate::files::write_json(&root.join("installed.json"), &prefs, JSON_LIMIT)?;
    Ok(manifest.id)
}
#[tauri::command]
pub fn list_plugins(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Plugins>,
) -> Result<Catalog, String> {
    authorize(window.label(), false)?;
    let mut runtime = state.inner.lock().map_err(|e| e.to_string())?;
    let root = root(&app)?;
    let prefs = preferences(&root)?;
    if !runtime.cleaned {
        cleanup_revisions(&root, &prefs)?;
        runtime.cleaned = true;
    }
    let mut entries = Vec::new();
    for (id, installed) in prefs.packages {
        let result = inside(&root, &format!("{}/{}/plugin.json", id, installed.revision))
            .and_then(|path| read(&path, JSON_LIMIT))
            .and_then(|bytes| manifest(&bytes));
        let (manifest, error) = match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        let evaluated = runtime.evaluated.get(&id);
        entries.push(Entry {
            status: runtime.statuses.get(&id).cloned(),
            theme_ids: manifest
                .as_ref()
                .map(|manifest| {
                    manifest
                        .contributes
                        .themes
                        .iter()
                        .map(|theme| theme_id(&theme.id))
                        .collect()
                })
                .unwrap_or_default(),
            restart_required: evaluated.is_some_and(|(r, _)| r != &installed.revision),
            evaluated: evaluated.is_some(),
            id,
            installed,
            manifest,
            error,
        });
    }
    Ok(Catalog {
        directory: root.to_string_lossy().into_owned(),
        entries,
        safe_mode: safe_mode(),
    })
}
#[tauri::command]
pub async fn import_plugin(
    window: Window,
    app: tauri::AppHandle,
    path: String,
) -> Result<String, String> {
    authorize(window.label(), true)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Plugins>();
        let _guard = state.inner.lock().map_err(|e| e.to_string())?;
        let selected=crate::themes::selected_themes(&app)?;
        for selected in selected{
            let source=Path::new(&path);let next=manifest(&read(&inside(source,"plugin.json")?,JSON_LIMIT)?)?;
            if crate::plugins::theme_directories(&app)?.iter().any(|theme|theme.id==selected&&theme.owner==next.id) && !next.contributes.themes.iter().any(|theme|theme_id(&theme.id)==selected){return Err("Choose another theme before replacing the package that supplies the selected theme.".into());}
        }
        let result = install(&root(&app)?, Path::new(&path))?;
        app.emit("plugins-changed", ()).map_err(|e| e.to_string())?;
        app.emit("theme-changed", ()).map_err(|e|e.to_string())?;
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn enable_plugin(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Plugins>,
    id: String,
    expected: String,
) -> Result<(), String> {
    authorize(window.label(), true)?;
    let _guard = state.inner.lock().map_err(|e| e.to_string())?;
    let root = root(&app)?;
    let mut prefs = preferences(&root)?;
    let installed = prefs
        .packages
        .get_mut(&id)
        .ok_or("Plugin is not installed.")?;
    if installed.revision != expected {
        return Err("Plugin changed. Review its new revision before enabling.".into());
    }
    let files = snapshot(&inside(&root, &format!("{id}/{expected}"))?)?;
    validate_snapshot(&files)?;
    if snapshot_revision(&files) != expected {
        return Err("Plugin bytes changed. Reimport and review the package.".into());
    }
    installed.trusted_revision = Some(expected);
    installed.enabled = true;
    crate::files::write_json(&root.join("installed.json"), &prefs, JSON_LIMIT)?;
    app.emit("plugins-changed", ()).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn prepare_plugin(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Plugins>,
    id: String,
    expected: String,
) -> Result<Manifest, String> {
    crate::files::main_window(&window)?;
    if safe_mode() {
        return Err("Safe startup skips executable plugins.".into());
    }
    let mut runtime = state.inner.lock().map_err(|e| e.to_string())?;
    let root = root(&app)?;
    let prefs = preferences(&root)?;
    let installed = prefs.packages.get(&id).ok_or("Plugin is not installed.")?;
    if !installed.enabled
        || installed.revision != expected
        || installed.trusted_revision.as_deref() != Some(&expected)
    {
        return Err("This package revision has not been enabled and trusted.".into());
    }
    if runtime
        .evaluated
        .get(&id)
        .is_some_and(|(r, _)| r != &expected)
    {
        return Err("Restart Lomi to load the replacement code.".into());
    }
    let files = snapshot(&inside(&root, &format!("{id}/{expected}"))?)?;
    if snapshot_revision(&files) != expected {
        return Err("Plugin files changed after trust was granted.".into());
    }
    let manifest = validate_snapshot(&files)?;
    runtime.evaluated.insert(
        id,
        (
            expected,
            files
                .iter()
                .map(|(path, bytes)| (path.clone(), digest(bytes)))
                .collect(),
        ),
    );
    Ok(manifest)
}
#[tauri::command]
pub fn request_plugin_removal(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Plugins>,
    id: String,
    expected: String,
    uninstall: bool,
) -> Result<String, String> {
    authorize(window.label(), true)?;
    let mut runtime = state.inner.lock().map_err(|e| e.to_string())?;
    let prefs = preferences(&root(&app)?)?;
    let installed = prefs.packages.get(&id).ok_or("Plugin is not installed.")?;
    if installed.revision != expected || runtime.pending.values().any(|(owner, _, _)| owner == &id)
    {
        return Err("Plugin changed or already has a pending operation.".into());
    }
    let token = format!(
        "{}-{}",
        id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    runtime.pending.insert(
        token.clone(),
        (
            id.clone(),
            expected,
            if uninstall { "uninstall" } else { "disable" }.into(),
        ),
    );
    if let Err(error) = focus_main(&app).and_then(|()| {
        app.emit_to(
            "main",
            "plugin-close-request",
            serde_json::json!({"token":token,"id":id}),
        )
        .map_err(|e| e.to_string())
    }) {
        runtime.pending.remove(&token);
        return Err(error);
    }
    Ok(token)
}
#[tauri::command]
pub fn finish_plugin_removal(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Plugins>,
    token: String,
    approved: bool,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    let mut runtime = state.inner.lock().map_err(|e| e.to_string())?;
    let (id, expected, operation) = runtime
        .pending
        .remove(&token)
        .ok_or("Unknown plugin operation.")?;
    let result = (|| -> Result<(), String> {
        if !approved {
            return Ok(());
        }
        let themes = app.state::<crate::themes::Themes>();
        let _theme_guard = themes.lock.lock().map_err(|e| e.to_string())?;
        if operation == "uninstall" {
            for selected in crate::themes::selected_themes(&app)? {
                if theme_directories(&app)?
                    .iter()
                    .any(|theme| theme.owner == id && theme.id == selected)
                {
                    return Err(
                        "Select a fallback theme in Settings before removing this plugin.".into(),
                    );
                }
            }
        }
        let root = root(&app)?;
        let mut prefs = preferences(&root)?;
        let installed = prefs
            .packages
            .get_mut(&id)
            .ok_or("Plugin is no longer installed.")?;
        if installed.revision != expected {
            return Err("Plugin changed during closure.".into());
        }
        if operation == "uninstall" {
            prefs.packages.remove(&id);
        } else {
            installed.enabled = false;
        }
        crate::files::write_json(&root.join("installed.json"), &prefs, JSON_LIMIT)?;
        // Evaluated revisions may still back cached modules until the next process.
        if operation == "uninstall" && !runtime.evaluated.contains_key(&id) {
            if let Ok(path) = inside(&root, &id) {
                let _ = fs::remove_dir_all(path);
            }
        }
        app.emit("plugins-changed", ()).map_err(|e| e.to_string())?;
        Ok(())
    })();
    app.emit(
        "plugin-operation-finished",
        serde_json::json!({
            "token":token,"approved":approved && result.is_ok(),"error":result.as_ref().err()
        }),
    )
    .map_err(|e| e.to_string())?;
    result
}
fn mime(path: &str) -> Option<&'static str> {
    Some(match Path::new(path).extension()?.to_str()? {
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        _ => return None,
    })
}
pub fn protocol(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let app = context.app_handle().clone();
    let label = context.webview_label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let result = (|| -> Result<(&'static str, Vec<u8>), String> {
            if label != "main" || request.method() != "GET" || safe_mode() {
                return Err("Unavailable plugin resource.".into());
            }
            let decoded = percent_decode_str(request.uri().path().trim_start_matches('/'))
                .decode_utf8()
                .map_err(|e| e.to_string())?;
            let mut parts = decoded.splitn(3, '/');
            let id = parts.next().ok_or("Missing ID.")?;
            let revision = parts.next().ok_or("Missing revision.")?;
            let path = parts.next().ok_or("Missing path.")?;
            relative(path)?;
            let state = app.state::<Plugins>();
            let runtime = state.inner.lock().map_err(|e| e.to_string())?;
            let (evaluated, hashes) = runtime
                .evaluated
                .get(id)
                .ok_or("Plugin is not activated.")?;
            if evaluated != revision {
                return Err("Unknown revision.".into());
            }
            let expected = hashes.get(path).ok_or("Unknown package resource.")?;
            let mime = mime(path).ok_or("Unsupported resource type.")?;
            let bytes = read(
                &inside(&root(&app)?, &format!("{id}/{revision}/{path}"))?,
                FILE_LIMIT,
            )?;
            if &digest(&bytes) != expected {
                return Err("Package resource changed after approval.".into());
            }
            Ok((mime, bytes))
        })();
        let (status, mime, bytes) = match result {
            Ok((mime, bytes)) => (200, mime, bytes),
            Err(_) => (404, "text/plain", b"Plugin resource unavailable".to_vec()),
        };
        responder.respond(
            tauri::http::Response::builder()
                .status(status)
                .header("Content-Type", mime)
                .header("Access-Control-Allow-Origin", "*")
                .header("X-Content-Type-Options", "nosniff")
                .header("Cache-Control", "no-store")
                .body(bytes)
                .expect("valid plugin response"),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_plugin_contract_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/plugin-contract.json"))
                .unwrap();
        for case in cases.as_array().unwrap() {
            let valid = match case["kind"].as_str().unwrap() {
                "manifest" => manifest(&serde_json::to_vec(&case["input"]).unwrap()).is_ok(),
                "path" => relative(case["input"].as_str().unwrap()).is_ok(),
                "shortcut" => crate::keybindings::valid_shortcut(case["input"].as_str().unwrap()),
                kind => panic!("Unknown shared case: {kind}"),
            };
            assert_eq!(valid, case["valid"].as_bool().unwrap(), "{}", case["name"]);
        }
        let oversized = vec![b' '; JSON_LIMIT + 1];
        assert!(manifest(&oversized).is_err());
    }

    fn fixture(source: &Path) {
        fs::create_dir_all(source.join("dist")).unwrap();
        fs::write(source.join("plugin.json"),r#"{"schemaVersion":1,"id":"test.context","name":"Test","version":"1.0.0","description":"Test package","hostApi":1,"entry":"dist/index.js","activation":"lazy"}"#).unwrap();
        fs::write(
            source.join("dist/index.js"),
            "export function activate() {}",
        )
        .unwrap();
    }
    #[test]
    fn startup_cleanup_removes_only_obsolete_revision_directories() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        let owner = install(root.path(), source.path()).unwrap();
        let prefs = preferences(root.path()).unwrap();
        let current = root
            .path()
            .join(&owner)
            .join(&prefs.packages[&owner].revision);
        let obsolete = root.path().join(&owner).join("0".repeat(64));
        fs::create_dir(&obsolete).unwrap();
        fs::write(obsolete.join("index.js"), "old").unwrap();
        let note = root.path().join(&owner).join("notes");
        fs::create_dir(&note).unwrap();
        cleanup_revisions(root.path(), &prefs).unwrap();
        assert!(current.exists());
        assert!(!obsolete.exists());
        assert!(note.exists());
        fs::write(root.path().join("installed.json"), "broken").unwrap();
        assert!(preferences(root.path()).is_err());
        assert!(current.exists());
        fs::rename(
            root.path().join("installed.json"),
            root.path().join("installed.backup.json"),
        )
        .unwrap();
        cleanup_revisions(root.path(), &preferences(root.path()).unwrap()).unwrap();
        assert!(current.exists());
    }
    #[test]
    fn imports_immutable_revisions_without_trust_and_preserves_previous_install_on_failure() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        install(root.path(), source.path()).unwrap();
        let before = read(&root.path().join("installed.json"), JSON_LIMIT).unwrap();
        let prefs = preferences(root.path()).unwrap();
        let installed = &prefs.packages["test.context"];
        assert!(!installed.enabled);
        assert!(installed.trusted_revision.is_none());
        assert!(root
            .path()
            .join("test.context")
            .join(&installed.revision)
            .join("dist/index.js")
            .is_file());
        fs::write(source.path().join("plugin.json"), "invalid").unwrap();
        assert!(install(root.path(), source.path()).is_err());
        assert_eq!(
            read(&root.path().join("installed.json"), JSON_LIMIT).unwrap(),
            before
        );
        fixture(source.path());
        fs::write(
            source.path().join("dist/index.js"),
            "export function activate() { return () => {}; }",
        )
        .unwrap();
        install(root.path(), source.path()).unwrap();
        let after = preferences(root.path()).unwrap();
        assert_ne!(after.packages["test.context"].revision, installed.revision);
        assert!(after.packages["test.context"].trusted_revision.is_none());
        assert!(root
            .path()
            .join("test.context")
            .join(&installed.revision)
            .exists());
    }
    #[test]
    fn rejects_caller_traversal_symlinks_missing_entries_and_oversized_packages() {
        assert!(authorize("browser-1", false).is_err());
        assert!(authorize("main", true).is_err());
        assert!(authorize("settings", true).is_ok());
        for path in [
            "../a", "/a", "a//b", "C:/a", "a\\b", "a%2fb", "a?b", "aux.txt", "a/COM1", "a/.",
            "a/b ",
        ] {
            assert!(relative(path).is_err(), "{path}");
        }
        assert!(relative("dist/space żółć.js").is_ok());
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        fs::remove_file(source.path().join("dist/index.js")).unwrap();
        assert!(validate_snapshot(&snapshot(source.path()).unwrap()).is_err());
        fixture(source.path());
        fs::File::create(source.path().join("huge.bin"))
            .unwrap()
            .set_len(FILE_LIMIT as u64 + 1)
            .unwrap();
        assert!(snapshot(source.path()).is_err());
        fs::remove_file(source.path().join("huge.bin")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", source.path().join("linked")).unwrap();
            assert!(snapshot(source.path()).is_err());
            assert!(inside(source.path(), "linked").is_err());
        }
        for value in [
            serde_json::json!({"schemaVersion":2}),
            serde_json::json!({"dependencies":["other.plugin"]}),
        ] {
            assert!(manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }
    #[test]
    fn invalid_preferences_are_preserved_and_snapshot_identity_covers_paths_and_bytes() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("installed.json"), "broken").unwrap();
        assert!(preferences(root.path()).is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("installed.json")).unwrap(),
            "broken"
        );
        let first = BTreeMap::from([("ab".into(), b"c".to_vec())]);
        let second = BTreeMap::from([("a".into(), b"bc".to_vec())]);
        assert_ne!(snapshot_revision(&first), snapshot_revision(&second));
        assert_eq!(mime("chunk.js"), Some("text/javascript; charset=utf-8"));
        assert_eq!(mime("code.ts"), None);
    }
}

pub(crate) fn theme_id(contribution: &str) -> String {
    format!("@plugin-{}", digest(contribution.as_bytes()))
}
pub(crate) struct PluginTheme {
    pub id: String,
    pub owner: String,
    pub directory: PathBuf,
}
pub(crate) fn theme_directories(app: &tauri::AppHandle) -> Result<Vec<PluginTheme>, String> {
    let root = root(app)?;
    let prefs = preferences(&root)?;
    let mut themes = Vec::new();
    for (owner, installed) in prefs.packages {
        let folder = match inside(&root, &format!("{owner}/{}", installed.revision)) {
            Ok(folder) => folder,
            Err(_) => continue,
        };
        let manifest = match inside(&folder, "plugin.json")
            .and_then(|path| read(&path, JSON_LIMIT))
            .and_then(|bytes| manifest(&bytes))
        {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        for theme in manifest.contributes.themes {
            if let Ok(directory) = inside(&folder, &theme.path) {
                themes.push(PluginTheme {
                    id: theme_id(&theme.id),
                    owner: owner.clone(),
                    directory,
                });
            }
        }
    }
    Ok(themes)
}
#[tauri::command]
pub fn request_plugin_restart(window: Window, app: tauri::AppHandle) -> Result<(), String> {
    authorize(window.label(), true)?;
    focus_main(&app)?;
    app.emit_to("main", "plugin-restart-request", ())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn restart_plugins(window: Window, app: tauri::AppHandle) -> Result<(), String> {
    crate::files::main_window(&window)?;
    app.state::<crate::android::manager::Android>()
        .require_exit_ready()?;
    app.request_restart();
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    phase: String,
    error: String,
    evaluated: bool,
}
#[tauri::command]
pub fn report_plugin_status(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Plugins>,
    statuses: BTreeMap<String, Status>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    if statuses.len() > 128 {
        return Err("Too many plugin diagnostics.".into());
    }
    for (owner, status) in &statuses {
        id(owner)?;
        if status.error.len() > 8192
            || ![
                "discovered",
                "incompatible",
                "disabled",
                "activating",
                "active",
                "failed",
                "deactivating",
            ]
            .contains(&status.phase.as_str())
        {
            return Err("Invalid plugin diagnostic.".into());
        }
    }
    state.inner.lock().map_err(|e| e.to_string())?.statuses = statuses;
    app.emit_to("settings", "plugins-status-changed", ())
        .map_err(|e| e.to_string())
}

fn focus_main(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_window("main")
        .ok_or("The workspace window is unavailable.")?;
    window
        .show()
        .and_then(|()| window.set_focus())
        .map_err(|e| e.to_string())
}
fn cleanup_revisions(root: &Path, prefs: &Preferences) -> Result<(), String> {
    // A missing catalog may be an explicit recovery; retain its package backups.
    if !root.join("installed.json").exists() {
        return Ok(());
    }
    for owner in fs::read_dir(root).map_err(|e| e.to_string())? {
        let owner = owner.map_err(|e| e.to_string())?;
        let name = owner.file_name().to_string_lossy().into_owned();
        if id(&name).is_err() || !owner.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        for entry in fs::read_dir(owner.path()).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let value = entry.file_name().to_string_lossy().into_owned();
            if revision(&value).is_ok()
                && entry.file_type().map_err(|e| e.to_string())?.is_dir()
                && prefs
                    .packages
                    .get(&name)
                    .is_none_or(|installed| installed.revision != value)
            {
                fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}
