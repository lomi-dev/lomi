use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tauri::{Emitter, Manager, State, Window};
use tauri_plugin_opener::OpenerExt;
mod icons;
pub mod vscode;

const JSON_LIMIT: u64 = 256 * 1024;
const ASSET_LIMIT: u64 = 20 * 1024 * 1024;
const PACKAGE_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub struct Themes {
    pub(crate) lock: Mutex<Option<String>>,
    revision: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    fn native(self) -> Option<tauri::Theme> {
        match self {
            Self::System => None,
            Self::Light => Some(tauri::Theme::Light),
            Self::Dark => Some(tauri::Theme::Dark),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    version: u32,
    active: Option<String>,
    #[serde(default)]
    file_icons: Option<String>,
    #[serde(default)]
    product_icons: Option<String>,
    // Accept old settings without allowing them to disable manifest resources.
    #[serde(default, rename = "customCss", skip_serializing)]
    _legacy_custom_css: bool,
    #[serde(default)]
    appearance: Appearance,
}

impl Preferences {
    fn builtin() -> Self {
        Self {
            version: 1,
            active: None,
            file_icons: None,
            product_icons: None,
            _legacy_custom_css: false,
            appearance: Appearance::System,
        }
    }
}

#[derive(Serialize)]
pub struct Bundle {
    id: String,
    raw: String,
    #[serde(rename = "iconTheme", skip_serializing_if = "Option::is_none")]
    icon_theme: Option<Value>,
    revision: String,
    directory: String,
    migration: Vec<String>,
    #[serde(rename = "readOnly")]
    read_only: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Current {
    preferences: Preferences,
    theme: Option<Bundle>,
    file_icons: Option<Bundle>,
    product_icons: Option<Bundle>,
    safe_mode: bool,
    revision: u64,
}

#[derive(Serialize)]
pub struct Entry {
    kind: String,
    id: String,
    name: String,
    description: String,
    author: String,
    error: Option<String>,
    owner: Option<String>,
}

#[derive(Serialize)]
pub struct Catalog {
    directory: String,
    themes: Vec<Entry>,
}

fn authorize(label: &str, write: bool) -> Result<(), String> {
    if label == "settings" || (!write && label == "main") {
        Ok(())
    } else {
        Err("This theme operation is not available in this window.".into())
    }
}

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|error| error.to_string())
}

fn library(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let path = data_dir(app)?.join("themes");
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 100
        || id.starts_with('.')
        || id
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
        || id.ends_with(['.', ' '])
    {
        return Err("Invalid theme folder name.".into());
    }
    Ok(())
}

fn relative(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 1024
        || path
            .chars()
            .any(|c| c.is_control() || "\\:%?#<>\"|*".contains(c))
        || path.split('/').any(|part| {
            let base = part.split('.').next().unwrap_or("").to_ascii_lowercase();
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || ["con", "prn", "aux", "nul"].contains(&base.as_str())
                || ((base.starts_with("com") || base.starts_with("lpt"))
                    && base.len() == 4
                    && base.as_bytes()[3].is_ascii_digit())
        })
    {
        return Err(format!(
            "Use a relative path inside the theme folder: {path}"
        ));
    }
    Ok(())
}

fn inside(root: &Path, path: &str) -> Result<PathBuf, String> {
    relative(path)?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let mut target = root.clone();
    for component in path.split('/') {
        target.push(component);
        if fs::symlink_metadata(&target)
            .map_err(|error| format!("{path}: {error}"))?
            .file_type()
            .is_symlink()
        {
            return Err(format!("Theme resources cannot be symbolic links: {path}"));
        }
    }
    let target = target.canonicalize().map_err(|error| error.to_string())?;
    if !target.starts_with(root) {
        return Err("Resource escapes the theme folder.".into());
    }
    Ok(target)
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_file()
    {
        return Err(format!("Expected a regular file: {}", path.display()));
    }
    let file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err(format!("Expected a regular file: {}", path.display()));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds {} KiB.", path.display(), limit / 1024));
    }
    Ok(bytes)
}

fn mime(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "css" => Some("text/css; charset=utf-8"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "svg" => Some("image/svg+xml"),
        "avif" => Some("image/avif"),
        "ico" => Some("image/x-icon"),
        "woff" => Some("font/woff"),
        "woff2" => Some("font/woff2"),
        "ttf" => Some("font/ttf"),
        "otf" => Some("font/otf"),
        _ => None,
    }
}

fn resource(root: &Path, path: &str, kind: &str) -> Result<(), String> {
    let content_type = mime(path).ok_or_else(|| format!("Unsupported theme resource: {path}"))?;
    if !content_type.starts_with(kind) {
        return Err(format!("Expected {kind} resource: {path}"));
    }
    let file = inside(root, path)?;
    let metadata = fs::metadata(&file).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > ASSET_LIMIT {
        return Err(format!("Invalid or oversized resource: {path}"));
    }
    if kind == "text/css" {
        String::from_utf8(read_limited(&file, JSON_LIMIT)?).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
fn parse_raw(raw: &str) -> Result<Value, String> {
    use jsonc_parser::ast::Value as Ast;
    use jsonc_parser::{parse_to_ast, parse_to_serde_value, ParseOptions};
    let options = ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    };
    let parsed = parse_to_ast(raw, &Default::default(), &options)
        .map_err(|e| format!("theme.jsonc: {e}"))?;
    fn visit(node: &Ast<'_>, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err("Theme nesting exceeds 32 levels.".into());
        }
        match node {
            Ast::Object(object) => {
                let mut seen = std::collections::HashSet::new();
                for property in &object.properties {
                    let name = property.name.as_str();
                    if !seen.insert(name) {
                        return Err(format!("Duplicate theme property: {name}"));
                    }
                    visit(&property.value, depth + 1)?;
                }
            }
            Ast::Array(array) => {
                for value in &array.elements {
                    visit(value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(parsed.value.as_ref().ok_or("Expected a theme object.")?, 0)?;
    parse_to_serde_value(raw, &options).map_err(|e| e.to_string())
}
fn document_raw(root: &Path) -> Result<(String, Vec<String>), String> {
    let modern = root
        .join("theme.jsonc")
        .try_exists()
        .map_err(|e| e.to_string())?;
    let path = inside(root, if modern { "theme.jsonc" } else { "theme.json" })?;
    let bytes = read_limited(&path, JSON_LIMIT)?;
    let raw = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    if modern {
        return Ok((raw, Vec::new()));
    }
    let legacy: Value = serde_json::from_str(&raw).map_err(|e| format!("theme.json: {e}"))?;
    validate_manifest(root, &legacy)?;
    let mut common = legacy
        .as_object()
        .ok_or("Expected a legacy theme object.")?
        .clone();
    let mut modern = serde_json::Map::new();
    modern.insert("version".into(), 2.into());
    common.remove("version");
    common.remove("$schema");
    for key in ["name", "author", "description", "appearance"] {
        if let Some(value) = common.remove(key) {
            modern.insert(key.into(), value);
        }
    }
    let mut resources = serde_json::Map::new();
    for key in ["assets", "stylesheets"] {
        if let Some(value) = common.remove(key) {
            resources.insert(key.into(), value);
        }
    }
    if let Some(value) = common.remove("stylesheet") {
        resources.insert("stylesheets".into(), Value::Array(vec![value]));
    }
    modern.insert("common".into(), common.into());
    modern.insert("resources".into(), resources.into());
    Ok((serde_json::to_string_pretty(&modern).map_err(|e|e.to_string())?,vec!["Converted version 1. The original theme.json is preserved; saving creates theme.jsonc.".into()]))
}
fn manifest(root: &Path) -> Result<Value, String> {
    let (raw, _) = document_raw(root)?;
    let data = parse_raw(&raw)?;
    validate_modern(root, &data)?;
    Ok(data)
}
fn validate_modern(root: &Path, data: &Value) -> Result<(), String> {
    let object = data.as_object().ok_or("theme.jsonc must be an object.")?;
    if data["version"] != 2 {
        return Err("Unsupported theme version. Expected version 2.".into());
    }
    for key in object.keys() {
        if ![
            "$schema",
            "version",
            "name",
            "author",
            "description",
            "appearance",
            "common",
            "light",
            "dark",
            "resources",
            "vscode",
            "iconTheme",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("Unknown theme field: {key}"));
        }
    }
    if data["name"]
        .as_str()
        .is_none_or(|name| name.trim().is_empty() || name.chars().count() > 160)
    {
        return Err("The theme needs a name of 1–160 characters.".into());
    }
    if let Some(value) = data.get("appearance") {
        if !matches!(value.as_str(), Some("adaptive" | "dark" | "light")) {
            return Err("Invalid theme appearance.".into());
        }
    }
    if data.get("iconTheme").is_some() {
        if ["common", "light", "dark", "resources", "vscode"]
            .iter()
            .any(|field| data.get(field).is_some())
        {
            return Err("Icon theme packages cannot also define color surfaces.".into());
        }
        icons::load(root, data)?;
    }
    if let Some(value) = data.get("vscode") {
        vscode::validate(value)?;
    }
    for field in ["common", "light", "dark"] {
        if let Some(values) = data.get(field) {
            let values = values
                .as_object()
                .ok_or("Theme variants must be objects.")?;
            for key in values.keys() {
                if ![
                    "tokens",
                    "styles",
                    "backgrounds",
                    "layout",
                    "terminal",
                    "editor",
                    "plugins",
                ]
                .contains(&key.as_str())
                {
                    return Err(format!("Unknown {field} field: {key}"));
                }
            }
            let mut legacy = values.clone();
            legacy.remove("editor");
            legacy.remove("plugins");
            legacy.insert("version".into(), 1.into());
            legacy.insert("name".into(), "Surface".into());
            validate_manifest(root, &legacy.into())?;
        }
    }
    if let Some(resources) = data.get("resources") {
        let resources = resources
            .as_object()
            .ok_or("resources must be an object.")?;
        for key in resources.keys() {
            if !["assets", "stylesheets"].contains(&key.as_str()) {
                return Err(format!("Unknown resources field: {key}"));
            }
        }
        let mut legacy = resources.clone();
        legacy.insert("version".into(), 1.into());
        legacy.insert("name".into(), "Resources".into());
        validate_manifest(root, &legacy.into())?;
    }
    Ok(())
}

fn validate_manifest(root: &Path, data: &Value) -> Result<(), String> {
    let object = data.as_object().ok_or("theme.json must be an object.")?;
    if data["version"] != 1 {
        return Err("Unsupported theme version. Expected version 1.".into());
    }
    let name = data["name"].as_str().ok_or("The theme needs a name.")?;
    if name.trim().is_empty() || name.len() > 160 {
        return Err("Theme name must contain 1–160 characters.".into());
    }
    for key in object.keys() {
        if ![
            "$schema",
            "version",
            "name",
            "author",
            "description",
            "appearance",
            "layout",
            "tokens",
            "styles",
            "assets",
            "backgrounds",
            "terminal",
            "stylesheet",
            "stylesheets",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("Unknown theme field: {key}"));
        }
    }
    for key in ["author", "description", "$schema"] {
        if let Some(value) = object.get(key) {
            if value.as_str().is_none_or(|value| value.len() > 2000) {
                return Err(format!("Invalid {key}."));
            }
        }
    }
    if let Some(value) = object.get("appearance") {
        if !matches!(value.as_str(), Some("dark" | "light")) {
            return Err("appearance must be dark or light.".into());
        }
    }
    if let Some(layout) = object.get("layout") {
        let layout = layout.as_object().ok_or("layout must be an object.")?;
        for (key, value) in layout {
            let allowed: &[&str] = match key.as_str() {
                "tabs" => &["inline", "above", "below"],
                "statusbar" => &["top", "bottom"],
                "settingsNavigation" => &["left", "right", "top", "bottom"],
                _ => return Err(format!("Unknown layout field: {key}")),
            };
            if value.as_str().is_none_or(|value| !allowed.contains(&value)) {
                return Err(format!("Invalid layout.{key}."));
            }
        }
    }
    for key in ["tokens", "styles", "assets", "backgrounds", "terminal"] {
        if object.get(key).is_some_and(|value| !value.is_object()) {
            return Err(format!("{key} must be an object."));
        }
    }
    if object.contains_key("stylesheet") && object.contains_key("stylesheets") {
        return Err("Use stylesheets or the legacy stylesheet field, not both.".into());
    }
    let stylesheets = if let Some(value) = object.get("stylesheets") {
        value
            .as_array()
            .ok_or("stylesheets must be an array of relative CSS paths.")?
            .iter()
            .collect::<Vec<_>>()
    } else {
        object.get("stylesheet").into_iter().collect()
    };
    let mut seen = std::collections::HashSet::new();
    for stylesheet in stylesheets {
        let path = stylesheet
            .as_str()
            .ok_or("Stylesheet paths must be strings.")?;
        relative(path)?;
        if !path.ends_with(".css") {
            return Err("Stylesheet paths must name CSS files.".into());
        }
        if !seen.insert(path) {
            return Err("stylesheets must not contain duplicate paths.".into());
        }
        resource(root, path, "text/css")?;
    }

    if let Some(assets) = data["assets"].as_object() {
        for path in assets.values() {
            resource(
                root,
                path.as_str().ok_or("Asset paths must be strings.")?,
                "",
            )?;
        }
    }
    if let Some(backgrounds) = data["backgrounds"].as_object() {
        for (area, background) in backgrounds {
            if ![
                "app",
                "terminal",
                "sidebar",
                "titlebar",
                "statusbar",
                "settings",
                "modal",
            ]
            .contains(&area.as_str())
                || !background.is_object()
            {
                return Err(format!("Invalid background area: {area}"));
            }
            if let Some(path) = background.get("image") {
                resource(
                    root,
                    path.as_str()
                        .ok_or("Background image must be a relative path.")?,
                    "image/",
                )?;
            }
        }
    }
    Ok(())
}

fn bundle(root: &Path, id: &str) -> Result<Bundle, String> {
    valid_id(id)?;
    let folder = inside(root, id)?;
    let (raw, migration) = document_raw(&folder)?;
    Ok(Bundle {
        id: id.into(),
        revision: hash(raw.as_bytes()),
        icon_theme: icons::load(&folder, &parse_raw(&raw)?)?,
        raw,
        directory: folder.to_string_lossy().into_owned(),
        migration,
        read_only: false,
    })
}

fn theme_root(app: &tauri::AppHandle, id: &str) -> Result<PathBuf, String> {
    valid_id(id)?;
    if builtin_bundle(id).is_some() {
        return Err("Built-in themes have no editable folder. Duplicate the theme first.".into());
    }
    if id.starts_with("@plugin-") {
        return crate::plugins::theme_directories(app)?
            .into_iter()
            .find(|theme| theme.id == id)
            .map(|theme| theme.directory)
            .ok_or("The plugin theme is no longer installed.".into());
    }
    inside(&library(app)?, id)
}
fn app_bundle(app: &tauri::AppHandle, id: &str) -> Result<Bundle, String> {
    if let Some(bundle) = builtin_bundle(id) {
        return Ok(bundle);
    }
    if !id.starts_with("@plugin-") {
        return bundle(&library(app)?, id);
    }
    let folder = theme_root(app, id)?;
    let (raw, migration) = document_raw(&folder)?;
    Ok(Bundle {
        id: id.into(),
        revision: hash(raw.as_bytes()),
        icon_theme: icons::load(&folder, &parse_raw(&raw)?)?,
        raw,
        directory: folder.to_string_lossy().into_owned(),
        migration,
        read_only: true,
    })
}
fn builtin_bundle(id: &str) -> Option<Bundle> {
    let raw = match id {
        "@builtin-deepmono" => include_str!("../../themes/deepmono.json"),
        _ => return None,
    };
    Some(Bundle {
        id: id.into(),
        raw: raw.into(),
        revision: hash(raw.as_bytes()),
        icon_theme: None,
        directory: String::new(),
        migration: Vec::new(),
        read_only: true,
    })
}
pub(crate) fn selected_themes(app: &tauri::AppHandle) -> Result<Vec<String>, String> {
    let preferences = read_preferences(&data_dir(app)?.join("theme-settings.json"))?;
    Ok([
        preferences.active,
        preferences.file_icons,
        preferences.product_icons,
    ]
    .into_iter()
    .flatten()
    .collect())
}

fn read_preferences(path: &Path) -> Result<Preferences, String> {
    if !path.try_exists().map_err(|error| error.to_string())? {
        return Ok(Preferences::builtin());
    }
    let result = read_limited(path, JSON_LIMIT).and_then(|bytes| parse_preferences(&bytes));
    result.map_err(|error: String| {
        format!(
            "{error} Theme settings have been left intact at {}. Select Lomi to recover.",
            path.display()
        )
    })
}

fn parse_preferences(bytes: &[u8]) -> Result<Preferences, String> {
    let value: Preferences = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if value.version != 1 {
        return Err("Unsupported theme settings version.".into());
    }
    for id in [&value.active, &value.file_icons, &value.product_icons]
        .into_iter()
        .flatten()
    {
        valid_id(id)?;
    }
    Ok(value)
}

#[cfg(unix)]
pub(crate) fn agent_prepare(
    app: &tauri::AppHandle,
    patch: &lomi_control_protocol::settings::SettingsPatch,
    check: &dyn Fn() -> Result<(), lomi_control_protocol::ErrorCode>,
) -> Result<lomi_control_core::broker::SettingsPlan, lomi_control_protocol::ErrorCode> {
    use crate::settings_control::PreferenceSource;
    use lomi_control_core::{atomic_file, broker::SettingsPlan};
    use lomi_control_protocol::{
        settings::{SettingsPatch, SettingsThemeValues, SettingsUpdateValues},
        ErrorCode,
    };
    use std::sync::Arc;
    use tauri::Manager;
    check()?;
    if crate::plugins::safe_mode() || std::env::var_os("LOMI_SAFE_THEME").is_some_and(|v| v == "1")
    {
        return Err(ErrorCode::UnsupportedCapability);
    }
    let state = app.state::<Themes>();
    let _guard = state
        .lock
        .lock()
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let source = PreferenceSource::open(app, "theme-settings.json", JSON_LIMIT, check)?;
    let before = source
        .bytes
        .as_deref()
        .map(parse_preferences)
        .transpose()
        .map_err(|_| ErrorCode::UnsupportedCapability)?
        .unwrap_or_else(Preferences::builtin);
    let mut values = serde_json::to_value(&before).map_err(|_| ErrorCode::ResourceExhausted)?;
    values.as_object_mut().unwrap().remove("version");
    let before = SettingsUpdateValues::Themes(
        serde_json::from_value::<SettingsThemeValues>(values)
            .map_err(|_| ErrorCode::UnsupportedCapability)?,
    );
    let mut after = before.clone();
    patch.apply_values(&mut after)?;
    let mut stored = source
        .bytes
        .as_deref()
        .map(serde_json::from_slice::<Value>)
        .transpose()
        .map_err(|_| ErrorCode::UnsupportedCapability)?
        .unwrap_or_else(|| serde_json::json!({"version":1,"active":null}));
    match patch {
        SettingsPatch::ThemeBuiltin { value } => stored["active"] = serde_json::json!(value.id()),
        SettingsPatch::ThemeAppearance { value } => stored["appearance"] = serde_json::json!(value),
        _ => return Err(ErrorCode::UnsupportedCapability),
    }
    let bytes = serde_json::to_vec_pretty(&stored).map_err(|_| ErrorCode::ResourceExhausted)?;
    if bytes.len() as u64 > JSON_LIMIT {
        return Err(ErrorCode::ResourceExhausted);
    }
    let validated = parse_preferences(&bytes).map_err(|_| ErrorCode::UnsupportedCapability)?;
    validate_selections(app, &validated).map_err(|_| ErrorCode::UnsupportedCapability)?;
    let apply_app = app.clone();
    Ok(SettingsPlan {
        before,
        after,
        source_revision: source.revision.clone(),
        apply: Arc::new(move |check| {
            let state = apply_app.state::<Themes>();
            let _guard = state
                .lock
                .lock()
                .map_err(|_| ErrorCode::StorageUnavailable)?;
            validate_selections(&apply_app, &validated).map_err(|_| ErrorCode::RevisionConflict)?;
            let revision = source.commit(&bytes, check)?;
            let event_revision = state.revision.fetch_add(1, Ordering::SeqCst) + 1;
            apply_app
                .emit("theme-changed", event_revision)
                .map_err(|_| atomic_file::ReplaceError::Uncertain)?;
            Ok(revision)
        }),
    })
}

#[tauri::command]
pub fn load_theme_preferences(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Themes>,
) -> Result<Current, String> {
    authorize(window.label(), false)?;
    let mut observed = state.lock.lock().map_err(|error| error.to_string())?;
    let safe_mode = crate::plugins::safe_mode()
        || std::env::var_os("LOMI_SAFE_THEME").is_some_and(|value| value == "1");
    let preferences = if safe_mode {
        Preferences::builtin()
    } else {
        read_preferences(&data_dir(&app)?.join("theme-settings.json"))?
    };
    let theme = preferences
        .active
        .as_ref()
        .map(|id| app_bundle(&app, id))
        .transpose()?;
    let file_icons = preferences
        .file_icons
        .as_ref()
        .map(|id| app_bundle(&app, id))
        .transpose()?;
    let product_icons = preferences
        .product_icons
        .as_ref()
        .map(|id| app_bundle(&app, id))
        .transpose()?;
    for (bundle, expected) in [
        (&theme, "color"),
        (&file_icons, "file"),
        (&product_icons, "product"),
    ] {
        if let Some(bundle) = bundle {
            if icons::kind(&parse_raw(&bundle.raw)?) != expected {
                return Err(format!("Expected a {expected} theme."));
            }
        }
    }
    let mut identity = serde_json::to_string(&preferences).map_err(|e| e.to_string())?;
    for bundle in [&theme, &file_icons, &product_icons].into_iter().flatten() {
        identity.push_str(&bundle.revision);
        if !bundle.directory.is_empty() {
            resource_identity(Path::new(&bundle.directory), &mut identity, &mut 0, 0)?;
        }
    }
    let identity = hash(identity.as_bytes());
    if observed.as_ref() != Some(&identity) {
        *observed = Some(identity);
        state.revision.fetch_add(1, Ordering::SeqCst);
    }
    Ok(Current {
        preferences,
        theme,
        file_icons,
        product_icons,
        safe_mode,
        revision: state.revision.load(Ordering::SeqCst),
    })
}

#[tauri::command]
pub fn load_theme(window: Window, app: tauri::AppHandle, id: String) -> Result<Bundle, String> {
    authorize(window.label(), false)?;
    app_bundle(&app, &id)
}

#[tauri::command]
pub fn list_themes(window: Window, app: tauri::AppHandle) -> Result<Catalog, String> {
    authorize(window.label(), true)?;
    let directory = library(&app)?;
    let mut themes = Vec::new();
    for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if id.starts_with('.')
            || builtin_bundle(&id).is_some()
            || !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
        {
            continue;
        }
        let result = inside(&directory, &id).and_then(|folder| manifest(&folder));
        let (name, description, author, error, kind) = match result {
            Ok(bundle) => (
                bundle["name"].as_str().unwrap_or(&id).into(),
                bundle["description"].as_str().unwrap_or("").into(),
                bundle["author"].as_str().unwrap_or("").into(),
                None,
                icons::kind(&bundle).to_string(),
            ),
            Err(error) => (
                id.clone(),
                String::new(),
                String::new(),
                Some(error),
                "color".into(),
            ),
        };
        themes.push(Entry {
            kind,
            id,
            name,
            description,
            author,
            error,
            owner: None,
        });
    }
    if let Ok(contributions) = crate::plugins::theme_directories(&app) {
        for theme in contributions {
            let (name, description, author, error, kind) = match manifest(&theme.directory) {
                Ok(data) => (
                    data["name"].as_str().unwrap_or(&theme.id).into(),
                    data["description"].as_str().unwrap_or("").into(),
                    data["author"].as_str().unwrap_or("").into(),
                    None,
                    icons::kind(&data).to_string(),
                ),
                Err(error) => (
                    theme.id.clone(),
                    String::new(),
                    String::new(),
                    Some(error),
                    "color".into(),
                ),
            };
            themes.push(Entry {
                kind,
                id: theme.id,
                name,
                description,
                author,
                error,
                owner: Some(theme.owner),
            });
        }
    }
    themes.sort_by_key(|entry| entry.name.to_lowercase());
    Ok(Catalog {
        directory: directory.to_string_lossy().into_owned(),
        themes,
    })
}

#[tauri::command]
pub fn save_theme_preferences(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Themes>,
    data: Preferences,
) -> Result<(), String> {
    authorize(window.label(), true)?;
    let _guard = state.lock.lock().map_err(|error| error.to_string())?;
    validate_selections(&app, &data)?;
    let directory = data_dir(&app)?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    crate::files::write_json(
        &directory.join("theme-settings.json"),
        &data,
        JSON_LIMIT as usize,
    )?;
    let revision = state.revision.fetch_add(1, Ordering::SeqCst) + 1;
    app.emit("theme-changed", revision)
        .map_err(|error| error.to_string())
}

fn validate_selections(app: &tauri::AppHandle, data: &Preferences) -> Result<(), String> {
    if data.version != 1 {
        return Err("Unsupported theme settings version.".into());
    }
    for (id, expected) in [
        (&data.active, "color"),
        (&data.file_icons, "file"),
        (&data.product_icons, "product"),
    ] {
        if let Some(id) = id {
            if icons::kind(&parse_raw(&app_bundle(app, id)?.raw)?) != expected {
                return Err(format!("Expected a {expected} theme."));
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn refresh_themes(window: Window, app: tauri::AppHandle) -> Result<(), String> {
    authorize(window.label(), true)?;
    app.emit("theme-changed", ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_theme_manifest(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Themes>,
    id: String,
    expected: String,
    raw: String,
) -> Result<Bundle, String> {
    authorize(window.label(), true)?;
    let _guard = state.lock.lock().map_err(|e| e.to_string())?;
    if id.starts_with("@plugin-") {
        return Err("Duplicate a plugin theme before editing it.".into());
    }
    if builtin_bundle(&id).is_some() {
        return Err("Duplicate a built-in theme before editing it.".into());
    }
    let root = library(&app)?;
    save_manifest(&root, &id, &expected, &raw)?;
    let revision = state.revision.fetch_add(1, Ordering::SeqCst) + 1;
    app.emit("theme-changed", revision)
        .map_err(|e| e.to_string())?;
    bundle(&root, &id)
}
fn save_manifest(root: &Path, id: &str, expected: &str, raw: &str) -> Result<(), String> {
    use std::io::Write;
    if raw.len() as u64 > JSON_LIMIT {
        return Err("theme.jsonc exceeds 256 KiB.".into());
    }
    valid_id(id)?;
    let folder = inside(root, id)?;
    validate_modern(&folder, &parse_raw(raw)?)?;
    let path = folder.join("theme.jsonc");
    let mut temporary = tempfile::NamedTempFile::new_in(&folder).map_err(|e| e.to_string())?;
    if path.try_exists().map_err(|e| e.to_string())? {
        let checked = inside(&folder, "theme.jsonc")?;
        let permissions = fs::metadata(checked)
            .map_err(|e| e.to_string())?
            .permissions();
        if permissions.readonly() {
            return Err("This theme file is read-only.".into());
        }
        temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|e| e.to_string())?;
    }
    temporary
        .write_all(raw.as_bytes())
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    if hash(document_raw(&folder)?.0.as_bytes()) != expected {
        return Err("This theme changed on disk. Your draft is retained; reopen the editor or copy your draft before reloading.".into());
    }
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn open_themes_folder(
    window: Window,
    app: tauri::AppHandle,
    id: Option<String>,
) -> Result<(), String> {
    authorize(window.label(), true)?;
    let mut path = library(&app)?;
    if let Some(id) = id {
        valid_id(&id)?;
        path = theme_root(&app, &id)?;
    }
    app.opener()
        .open_path(path.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| error.to_string())
}

fn copy_package(
    source: &Path,
    target: &Path,
    total: &mut u64,
    count: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("Theme folders may be nested at most 16 levels.".into());
    }
    fs::create_dir(target).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        *count += 1;
        if *count > icons::ENTRY_LIMIT {
            return Err("Theme packages may contain at most 8192 entries.".into());
        }
        let name = entry.file_name();
        relative(
            name.to_str()
                .ok_or("Theme file names must be valid Unicode.")?,
        )?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        let destination = target.join(&name);
        if kind.is_dir() {
            copy_package(&entry.path(), &destination, total, count, depth + 1)?;
        } else if kind.is_file() {
            let bytes = read_limited(&entry.path(), ASSET_LIMIT)?;
            *total += bytes.len() as u64;
            if *total > PACKAGE_LIMIT {
                return Err("Theme packages may contain at most 64 MiB.".into());
            }
            fs::write(destination, bytes).map_err(|error| error.to_string())?;
        } else {
            return Err("Theme packages cannot contain symbolic links or special files.".into());
        }
    }
    Ok(())
}

fn import(root: &Path, source: &Path) -> Result<String, String> {
    if fs::symlink_metadata(source)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Choose a theme folder, not a symbolic link.".into());
    }
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    let vscode_folder = !source.join("theme.jsonc").exists()
        && source.join("package.json").exists()
        && (!source.join("theme.json").exists()
            || String::from_utf8(read_limited(&inside(&source, "theme.json")?, JSON_LIMIT)?)
                .map_err(|e| e.to_string())
                .and_then(|raw| parse_raw(&raw))?
                .get("version")
                .is_none());
    if source.is_file()
        || vscode_folder
        || (!source.join("theme.jsonc").exists() && !source.join("theme.json").exists())
    {
        return vscode::install(root, &source)?
            .into_iter()
            .next()
            .ok_or("No color themes were imported.".into());
    }
    manifest(&source)?;
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid theme folder.")?;
    valid_id(name)?;
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    if root.starts_with(&source) {
        return Err(
            "Choose an individual theme folder outside the themes directory's ancestors.".into(),
        );
    }
    if source.parent() == Some(root.as_path()) {
        return Ok(name.into());
    }
    let mut id = name.to_string();
    let mut suffix = 2;
    while root.join(&id).exists() {
        id = format!("{name}-{suffix}");
        suffix += 1;
    }
    valid_id(&id)?;
    let temporary = root.join(format!(".import-{id}"));
    if temporary.exists() {
        return Err(
            "An unfinished import folder already exists. Remove it before retrying.".into(),
        );
    }
    let result = (|| {
        copy_package(&source, &temporary, &mut 0, &mut 0, 0)?;
        manifest(&temporary)?;
        if !temporary.join("theme.jsonc").exists() {
            let (raw, _) = document_raw(&temporary)?;
            fs::write(temporary.join("theme.jsonc"), raw).map_err(|e| e.to_string())?;
        }
        fs::rename(&temporary, root.join(&id)).map_err(|error| error.to_string())?;
        Ok(id)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(temporary);
    }
    result
}

#[tauri::command]
pub async fn import_theme(
    window: Window,
    app: tauri::AppHandle,
    path: String,
) -> Result<String, String> {
    authorize(window.label(), true)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Themes>();
        let _guard = state.lock.lock().map_err(|error| error.to_string())?;
        import(&library(&app)?, Path::new(&path))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn create_theme(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Themes>,
) -> Result<String, String> {
    authorize(window.label(), true)?;
    let _guard = state.lock.lock().map_err(|error| error.to_string())?;
    create_starter(&library(&app)?)
}

fn create_starter(root: &Path) -> Result<String, String> {
    let mut id = "my-theme".to_string();
    let mut suffix = 2;
    while root.join(&id).exists() {
        id = format!("my-theme-{suffix}");
        suffix += 1;
    }
    let temporary = root.join(format!(".create-{id}"));
    fs::create_dir(&temporary).map_err(|error| error.to_string())?;
    let result = (|| {
        let raw = serde_json::to_vec_pretty(&serde_json::json!({
            "$schema": "./theme.schema.json",
            "version": 2,
            "name": "My theme",
            "appearance": "adaptive",
            "common": {}
        }))
        .map_err(|error| error.to_string())?;
        fs::write(temporary.join("theme.jsonc"), raw).map_err(|error| error.to_string())?;
        fs::write(
            temporary.join("theme.schema.json"),
            include_str!("../../themes/theme.schema.json"),
        )
        .map_err(|error| error.to_string())?;
        manifest(&temporary)?;
        fs::rename(&temporary, root.join(&id)).map_err(|error| error.to_string())?;
        Ok(id)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(temporary);
    }
    result
}

#[tauri::command]
pub fn sync_theme_window(window: Window, appearance: Appearance) -> Result<(), String> {
    authorize(window.label(), false)?;
    window
        .set_theme(appearance.native())
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "linux")]
    if appearance == Appearance::System {
        // GTK clears prefer-dark when resetting the override. Restore the portal's
        // current value; Tao continues forwarding subsequent portal changes to GTK.
        let theme = window.theme().map_err(|error| error.to_string())?;
        window
            .set_theme(Some(theme))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn asset(root: &Path, uri_path: &str) -> Result<(&'static str, Vec<u8>), String> {
    let decoded = percent_decode_str(uri_path.trim_start_matches('/'))
        .decode_utf8()
        .map_err(|error| error.to_string())?;
    let mut parts = decoded.splitn(3, '/');
    let id = parts.next().ok_or("Missing theme id.")?;
    valid_id(id)?;
    let revision = parts.next().ok_or("Missing asset revision.")?;
    if revision.is_empty()
        || !revision
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("Invalid asset revision.".into());
    }
    let path = parts.next().ok_or("Missing resource path.")?;
    let mime = mime(path).ok_or("Unsupported theme resource.")?;
    let folder = inside(root, id)?;
    let path = inside(&folder, path)?;
    let bytes = read_limited(
        &path,
        if mime.starts_with("text/css") {
            JSON_LIMIT
        } else {
            ASSET_LIMIT
        },
    )?;
    Ok((mime, bytes))
}

pub fn protocol(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let app = context.app_handle().clone();
    let label = context.webview_label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let result = authorize(&label, false).and_then(|()| {
            if request.method() != "GET" {
                return Err("Only GET is supported.".into());
            }
            let path = request.uri().path();
            let encoded = path
                .trim_start_matches('/')
                .split('/')
                .next()
                .ok_or("Missing theme id.")?;
            let id = percent_decode_str(encoded)
                .decode_utf8()
                .map_err(|e| e.to_string())?;
            if id.starts_with("@plugin-") {
                let decoded = percent_decode_str(path.trim_start_matches('/'))
                    .decode_utf8()
                    .map_err(|e| e.to_string())?;
                let mut parts = decoded.splitn(3, '/');
                parts.next();
                let revision = parts.next().ok_or("Missing revision.")?;
                if revision.is_empty()
                    || !revision
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                {
                    return Err("Invalid revision.".into());
                }
                let path = parts.next().ok_or("Missing resource path.")?;
                let mime = mime(path).ok_or("Unsupported theme resource.")?;
                let bytes = read_limited(
                    &inside(&theme_root(&app, &id)?, path)?,
                    if mime.starts_with("text/css") {
                        JSON_LIMIT
                    } else {
                        ASSET_LIMIT
                    },
                )?;
                Ok((mime, bytes))
            } else {
                asset(&library(&app)?, path)
            }
        });
        let (status, mime, bytes) = match result {
            Ok((mime, bytes)) => (200, mime, bytes),
            Err(_) => (404, "text/plain", b"Theme resource unavailable".to_vec()),
        };
        responder.respond(
            tauri::http::Response::builder()
                .status(status)
                .header("Content-Type", mime)
                .header("Cache-Control", "no-store")
                .header("Access-Control-Allow-Origin", "*")
                .header("X-Content-Type-Options", "nosniff")
                .header(
                    "Content-Security-Policy",
                    "default-src 'none'; style-src 'unsafe-inline'; sandbox",
                )
                .body(bytes)
                .expect("valid theme response"),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_embedded_read_only_and_duplicate_into_editable_packages() {
        let root = tempfile::tempdir().unwrap();
        let deepmono = builtin_bundle("@builtin-deepmono").unwrap();
        assert!(deepmono.read_only);
        assert!(deepmono.directory.is_empty());
        assert_eq!(deepmono.revision, hash(deepmono.raw.as_bytes()));
        let data = parse_raw(&deepmono.raw).unwrap();
        validate_modern(root.path(), &data).unwrap();
        assert_eq!(data["name"], "DeepMono");
        assert_eq!(icons::kind(&data), "color");
        assert!(builtin_bundle("missing-theme").is_none());

        let default_id = duplicate(root.path(), None).unwrap();
        assert_eq!(
            manifest(&root.path().join(default_id)).unwrap()["name"],
            "Lomi"
        );
        let copied = duplicate_builtin(root.path(), &deepmono.raw).unwrap();
        let copy = bundle(root.path(), &copied).unwrap();
        assert!(!copy.read_only);
        assert!(!copy.directory.is_empty());
        assert_eq!(copy.raw, deepmono.raw);

        let path = root.path().join("theme-settings.json");
        assert!(read_preferences(&path).unwrap().active.is_none());
        let preferences = Preferences {
            active: Some(deepmono.id),
            ..Preferences::builtin()
        };
        fs::write(&path, serde_json::to_vec(&preferences).unwrap()).unwrap();
        assert_eq!(read_preferences(&path).unwrap().active, preferences.active);
    }

    #[test]
    fn shared_grammar_and_duplicate_preserve_comments_resources_and_originals() {
        let root = tempfile::tempdir().unwrap();
        let fixtures: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/themes/grammar.json")).unwrap();
        for fixture in fixtures.as_array().unwrap() {
            let result = parse_raw(fixture["raw"].as_str().unwrap())
                .and_then(|data| validate_modern(root.path(), &data));
            assert_eq!(
                result.is_ok(),
                fixture["valid"].as_bool().unwrap(),
                "{}: {result:?}",
                fixture["name"]
            );
        }
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        let raw = "// Keep this comment\n{\"version\":2,\"name\":\"Copy\"}";
        fs::write(source.join("theme.jsonc"), raw).unwrap();
        let copied = duplicate(root.path(), Some(&source)).unwrap();
        assert_eq!(
            fs::read_to_string(root.path().join(copied).join("theme.jsonc")).unwrap(),
            raw
        );
        assert_eq!(fs::read_to_string(source.join("theme.jsonc")).unwrap(), raw);
        let builtin = duplicate(root.path(), None).unwrap();
        assert_eq!(manifest(&root.path().join(builtin)).unwrap()["version"], 2);
    }
    #[test]
    fn raw_jsonc_saves_preserve_comments_revisions_permissions_and_legacy_sources() {
        let root = tempfile::tempdir().unwrap();
        let folder = fixture(root.path());
        let legacy = fs::read(folder.join("theme.json")).unwrap();
        let original = bundle(root.path(), "sample").unwrap();
        let raw = r#"{ // preserve this comment
 "version":2,"name":"Raw","common":{"layout":{"tabs":"below"}},}"#;
        save_manifest(root.path(), "sample", &original.revision, raw).unwrap();
        assert_eq!(fs::read_to_string(folder.join("theme.jsonc")).unwrap(), raw);
        assert_eq!(fs::read(folder.join("theme.json")).unwrap(), legacy);
        assert!(
            save_manifest(root.path(), "sample", &original.revision, raw)
                .unwrap_err()
                .contains("changed on disk")
        );
        let next = bundle(root.path(), "sample").unwrap();
        for invalid in [
            "{",
            r#"{"version":2,"version":2,"name":"Duplicate"}"#,
            r#"{"version":2,"name":"Missing","resources":{"stylesheets":["missing.css"]}}"#,
            r#"{"version":2,"name":"Invalid","common":{"layout":{"tabs":"floating"}}}"#,
        ] {
            assert!(save_manifest(root.path(), "sample", &next.revision, invalid).is_err());
            assert_eq!(fs::read_to_string(folder.join("theme.jsonc")).unwrap(), raw);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = folder.join("theme.jsonc");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
            save_manifest(root.path(), "sample", &next.revision, raw).unwrap();
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o640
            );
            fs::set_permissions(&path, fs::Permissions::from_mode(0o440)).unwrap();
            assert!(save_manifest(root.path(), "sample", &next.revision, raw)
                .unwrap_err()
                .contains("read-only"));
        }
    }

    fn fixture(root: &Path) -> PathBuf {
        let theme = root.join("sample");
        fs::create_dir(&theme).unwrap();
        fs::write(theme.join("theme.json"), r##"{"version":1,"name":"Sample","backgrounds":{"terminal":{"image":"wall paper.svg"}},"stylesheet":"theme.css"}"##).unwrap();
        fs::write(theme.join("wall paper.svg"), "<svg/>").unwrap();
        fs::write(theme.join("theme.css"), ".tab { border-radius: 0; }").unwrap();
        theme
    }

    #[test]
    fn creates_self_contained_starters_without_overwriting_existing_files() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(create_starter(root.path()).unwrap(), "my-theme");
        fs::write(root.path().join("my-theme/theme.css"), "user changes").unwrap();
        assert_eq!(create_starter(root.path()).unwrap(), "my-theme-2");
        assert_eq!(
            fs::read_to_string(root.path().join("my-theme/theme.css")).unwrap(),
            "user changes"
        );
        let theme = bundle(root.path(), "my-theme-2").unwrap();
        let data = parse_raw(&theme.raw).unwrap();
        assert_eq!(data["name"], "My theme");
        assert_eq!(data["appearance"], "adaptive");
        assert_eq!(data["common"], serde_json::json!({}));
        assert!(root.path().join("my-theme-2/theme.schema.json").is_file());
        assert_eq!(
            fs::read_dir(root.path().join("my-theme-2"))
                .unwrap()
                .count(),
            2
        );
    }

    #[test]
    fn missing_legacy_stylesheets_reject_incomplete_themes() {
        let root = tempfile::tempdir().unwrap();
        let theme = fixture(root.path());
        fs::remove_file(theme.join("theme.css")).unwrap();
        assert!(bundle(root.path(), "sample").is_err());
    }

    #[test]
    fn validates_and_imports_all_declared_stylesheets_in_order() {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let theme = fixture(source.path());
        fs::create_dir(theme.join("styles")).unwrap();
        fs::write(
            theme.join("styles/żółty motyw.css"),
            ".tab { border-radius: 12px; }",
        )
        .unwrap();
        let mut data = serde_json::json!({"version":1,"name":"CSS theme","stylesheets":["theme.css", "styles/żółty motyw.css"]});
        fs::write(theme.join("theme.json"), data.to_string()).unwrap();
        let id = import(root.path(), &theme).unwrap();
        assert_eq!(
            parse_raw(&bundle(root.path(), &id).unwrap().raw).unwrap()["resources"]["stylesheets"],
            data["stylesheets"]
        );
        assert_eq!(
            asset(
                root.path(),
                "/sample/1/styles/%C5%BC%C3%B3%C5%82ty%20motyw.css"
            )
            .unwrap()
            .1,
            b".tab { border-radius: 12px; }"
        );
        fs::remove_file(theme.join("styles/żółty motyw.css")).unwrap();
        assert!(manifest(&theme).is_err());
        for invalid in [
            serde_json::json!("theme.css"),
            serde_json::json!(null),
            serde_json::json!([false]),
            serde_json::json!(["theme.css", "theme.css"]),
            serde_json::json!(["../theme.css"]),
            serde_json::json!(["https://example.com/theme.css"]),
            serde_json::json!(["wall paper.svg"]),
        ] {
            data["stylesheets"] = invalid;
            fs::write(theme.join("theme.json"), data.to_string()).unwrap();
            assert!(manifest(&theme).is_err(), "{data}");
        }
        data["stylesheets"] = serde_json::json!([]);
        fs::write(theme.join("theme.json"), data.to_string()).unwrap();
        assert!(manifest(&theme).is_ok());
        data["stylesheet"] = serde_json::json!("theme.css");
        fs::write(theme.join("theme.json"), data.to_string()).unwrap();
        assert!(manifest(&theme).is_err());
    }

    #[test]
    fn imports_complete_packages_without_overwriting_and_serves_local_assets() {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let theme = fixture(source.path());
        assert_eq!(import(root.path(), &theme).unwrap(), "sample");
        assert_eq!(import(root.path(), &theme).unwrap(), "sample-2");
        assert_eq!(
            asset(root.path(), "/sample/1/wall%20paper.svg").unwrap().1,
            b"<svg/>"
        );
        assert_eq!(
            asset(root.path(), "/sample/2/theme.css").unwrap().0,
            "text/css; charset=utf-8"
        );
        assert!(asset(root.path(), "/sample/1/theme.json").is_err());
        assert_eq!(
            import(root.path(), &root.path().join("sample")).unwrap(),
            "sample"
        );
    }

    #[test]
    fn rejects_escaping_paths_missing_assets_and_unsupported_versions() {
        let root = tempfile::tempdir().unwrap();
        let theme = fixture(root.path());
        for path in [
            "/sample/1/../wall.svg",
            "/sample/1/%2e%2e/wall.svg",
            "/sample/1/C:%5cwall.svg",
            "/sample/1//wall.svg",
            "/sample/1/https://example.com/wall.svg",
        ] {
            assert!(asset(root.path(), path).is_err(), "{path}");
        }
        fs::remove_file(theme.join("wall paper.svg")).unwrap();
        assert!(manifest(&theme).is_err());
        fs::write(theme.join("theme.json"), r#"{"version":2,"name":"Future"}"#).unwrap();
        assert!(manifest(&theme).is_err());
    }

    #[test]
    fn preserves_invalid_preferences_and_restricts_windows() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("theme-settings.json");
        assert!(read_preferences(&path).unwrap().active.is_none());
        for value in ["broken", r#"{"version":2,"active":null,"customCss":true}"#] {
            fs::write(&path, value).unwrap();
            assert!(read_preferences(&path).unwrap_err().contains("left intact"));
            assert_eq!(fs::read_to_string(&path).unwrap(), value);
        }
        assert!(authorize("main", true).is_err());
        assert!(authorize("main", false).is_ok());
        assert!(authorize("settings", true).is_ok());
        assert!(authorize("other", false).is_err());
    }

    #[test]
    fn appearance_defaults_to_system_and_preserves_invalid_preferences() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("theme-settings.json");
        assert_eq!(
            read_preferences(&path).unwrap().appearance,
            Appearance::System
        );
        fs::write(
            &path,
            r#"{"version":1,"active":"sample","customCss":false}"#,
        )
        .unwrap();
        let legacy = read_preferences(&path).unwrap();
        assert_eq!(legacy.appearance, Appearance::System);
        assert_eq!(legacy.active.as_deref(), Some("sample"));
        assert!(serde_json::to_value(&legacy)
            .unwrap()
            .get("customCss")
            .is_none());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            r#"{"version":1,"active":"sample","customCss":false}"#
        );
        for appearance in [Appearance::System, Appearance::Light, Appearance::Dark] {
            let preferences = Preferences {
                appearance,
                ..Preferences::builtin()
            };
            fs::write(&path, serde_json::to_vec(&preferences).unwrap()).unwrap();
            assert_eq!(read_preferences(&path).unwrap().appearance, appearance);
        }
        let invalid = r#"{"version":1,"active":null,"customCss":true,"appearance":"auto"}"#;
        fs::write(&path, invalid).unwrap();
        assert!(read_preferences(&path).unwrap_err().contains("left intact"));
        assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
        assert_eq!(Appearance::System.native(), None);
        assert_eq!(Appearance::Light.native(), Some(tauri::Theme::Light));
        assert_eq!(Appearance::Dark.native(), Some(tauri::Theme::Dark));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_removes_failed_imports() {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let theme = fixture(source.path());
        std::os::unix::fs::symlink("/etc/passwd", theme.join("outside.svg")).unwrap();
        assert!(import(root.path(), &theme).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        assert!(asset(source.path(), "/sample/1/outside.svg").is_err());
    }
}

fn resource_identity(
    root: &Path,
    output: &mut String,
    count: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("Theme resources exceed 16 levels.".into());
    }
    let mut entries = fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        *count += 1;
        if *count > icons::ENTRY_LIMIT {
            return Err("Too many theme resources.".into());
        }
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        output.push_str(&format!(
            "{:?}:{:?}:{}",
            entry.file_name(),
            metadata.modified().map_err(|e| e.to_string())?,
            metadata.len()
        ));
        if entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
            return Err("Theme resources cannot use symbolic links.".into());
        }
        if metadata.is_dir() {
            resource_identity(&entry.path(), output, count, depth + 1)?;
        }
    }
    Ok(())
}
#[tauri::command]
pub fn duplicate_theme(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Themes>,
    id: Option<String>,
    kind: Option<String>,
) -> Result<String, String> {
    authorize(window.label(), true)?;
    let _guard = state.lock.lock().map_err(|e| e.to_string())?;
    if let Some(bundle) = id.as_deref().and_then(builtin_bundle) {
        return duplicate_builtin(&library(&app)?, &bundle.raw);
    }
    if id.is_none() && kind.as_deref().is_some_and(|kind| kind != "color") {
        let root = library(&app)?;
        let stage = tempfile::tempdir_in(&root).map_err(|e| e.to_string())?;
        icons::write(
            stage.path(),
            &icons::builtin(kind.as_deref().unwrap_or("file"))?,
        )?;
        return duplicate(&root, Some(stage.path()));
    }
    let source = id.as_ref().map(|id| theme_root(&app, id)).transpose()?;
    duplicate(&library(&app)?, source.as_deref())
}
fn duplicate_builtin(root: &Path, raw: &str) -> Result<String, String> {
    let stage = tempfile::tempdir_in(root).map_err(|e| e.to_string())?;
    fs::write(stage.path().join("theme.jsonc"), raw).map_err(|e| e.to_string())?;
    duplicate(root, Some(stage.path()))
}
fn duplicate(root: &Path, source: Option<&Path>) -> Result<String, String> {
    let stage = tempfile::Builder::new()
        .prefix(".duplicate-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let folder = stage.path().join("package");
    let raw = if let Some(source) = source {
        copy_package(source, &folder, &mut 0, &mut 0, 0)?;
        document_raw(&folder)?.0
    } else {
        fs::create_dir(&folder).map_err(|e| e.to_string())?;
        include_str!("../../themes/lomi.json").to_string()
    };
    // Preserve the source text, comments and resources; authors can rename their copy in the editor.
    fs::write(folder.join("theme.jsonc"), raw).map_err(|e| e.to_string())?;
    manifest(&folder)?;
    let mut id = "theme-copy".to_string();
    let mut suffix = 2;
    while root.join(&id).exists() {
        id = format!("theme-copy-{suffix}");
        suffix += 1;
    }
    fs::rename(folder, root.join(&id)).map_err(|e| e.to_string())?;
    Ok(id)
}
