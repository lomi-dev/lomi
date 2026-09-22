use super::*;
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

// References may contain ../, but must remain inside the selected extension root.
pub(super) fn reference(base: &str, path: &str) -> Result<String, String> {
    if path.starts_with('/') || path.contains('\\') {
        return Err("VS Code theme references must stay inside the selected folder.".into());
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    parts.pop();
    for part in path.split('/') {
        match part {
            "." => {}
            ".." => {
                parts.pop().ok_or("Theme reference escapes its package. Choose the extension folder instead of an individual file.")?;
            }
            _ => {
                relative(part)?;
                parts.push(part);
            }
        }
    }
    let result = parts.join("/");
    relative(&result)?;
    Ok(result)
}

pub(super) enum Source {
    Folder(PathBuf),
    Vsix(Box<ZipArchive<Cursor<Vec<u8>>>>),
}
impl Source {
    fn read(&mut self, path: &str) -> Result<Vec<u8>, String> {
        self.read_limit(path, JSON_LIMIT)
    }
    pub(super) fn read_limit(&mut self, path: &str, limit: u64) -> Result<Vec<u8>, String> {
        relative(path)?;
        match self {
            Self::Folder(root) => read_limited(&inside(root, path)?, limit),
            Self::Vsix(zip) => {
                let file = zip
                    .by_name(&format!("extension/{path}"))
                    .map_err(|e| format!("{path}: {e}"))?;
                if !file.is_file() || file.is_symlink() || file.size() > limit {
                    return Err(format!("Invalid or oversized theme file: {path}"));
                }
                let mut result = Vec::new();
                file.take(limit + 1)
                    .read_to_end(&mut result)
                    .map_err(|e| e.to_string())?;
                if result.len() as u64 > limit {
                    return Err("Theme file exceeds its size limit.".into());
                }
                Ok(result)
            }
        }
    }
    fn json(&mut self, path: &str) -> Result<Value, String> {
        let raw = String::from_utf8(self.read(path)?).map_err(|e| e.to_string())?;
        parse_raw(raw.trim_start_matches('\u{feff}')).map_err(|e| format!("{path}: {e}"))
    }
}

fn textmate(source: &mut Source, path: &str) -> Result<Value, String> {
    let bytes = source.read(path)?;
    let plist =
        plist::Value::from_reader_xml(Cursor::new(bytes)).map_err(|e| format!("{path}: {e}"))?;
    let mut pending = vec![(&plist, 0)];
    let mut too_deep = false;
    while let Some((value, depth)) = pending.pop() {
        if depth > 32 {
            too_deep = true;
            break;
        }
        match value {
            plist::Value::Array(values) => {
                pending.extend(values.iter().map(|value| (value, depth + 1)))
            }
            plist::Value::Dictionary(values) => {
                pending.extend(values.values().map(|value| (value, depth + 1)))
            }
            _ => {}
        }
    }
    if too_deep {
        // Dispose untrusted nested values iteratively before serde can recurse into them.
        let mut pending = vec![plist];
        while let Some(value) = pending.pop() {
            match value {
                plist::Value::Array(values) => pending.extend(values),
                plist::Value::Dictionary(values) => {
                    pending.extend(values.into_iter().map(|(_, value)| value))
                }
                _ => {}
            }
        }
        return Err("TextMate theme nesting exceeds 32 levels.".into());
    }
    let data = serde_json::to_value(plist).map_err(|e| e.to_string())?;
    if !data["settings"].is_array() {
        return Err("TextMate themes need a settings array.".into());
    }
    let mut theme = serde_json::json!({ "tokenColors": data["settings"], "name": data["name"] });
    textmate_colors(&mut theme);
    Ok(theme)
}

fn textmate_colors(theme: &mut Value) {
    let mut colors = theme["colors"].as_object().cloned().unwrap_or_default();
    for rule in theme["tokenColors"].as_array().into_iter().flatten() {
        if rule["scope"]
            .as_str()
            .is_some_and(|scope| !scope.is_empty())
            || rule["scope"].is_array()
        {
            continue;
        }
        for (field, id) in [
            ("background", "editor.background"),
            ("foreground", "editor.foreground"),
            ("caret", "editorCursor.foreground"),
            ("selection", "editor.selectionBackground"),
            ("lineHighlight", "editor.lineHighlightBackground"),
            ("invisibles", "editorWhitespace.foreground"),
        ] {
            if let Some(value) = rule["settings"]
                .get(field)
                .filter(|value| value.is_string())
            {
                colors.insert(id.into(), value.clone());
            }
        }
    }
    if !colors.is_empty() {
        theme["colors"] = colors.into();
    }
}

fn load(
    source: &mut Source,
    path: &str,
    stack: &mut Vec<String>,
    total: &mut usize,
) -> Result<Value, String> {
    *total += 1;
    if stack.len() >= 16 || *total > 64 || stack.iter().any(|p| p == path) {
        return Err("Cyclic or excessive VS Code theme includes.".into());
    }
    stack.push(path.to_owned());
    let mut data = if path.to_ascii_lowercase().ends_with(".tmtheme") {
        textmate(source, path)?
    } else {
        source.json(path)?
    };
    if !data.is_object() {
        return Err("Expected a VS Code theme object.".into());
    }
    if ![
        "include",
        "colors",
        "tokenColors",
        "settings",
        "semanticTokenColors",
    ]
    .iter()
    .any(|key| data.get(key).is_some())
    {
        return Err("This file does not define a VS Code color theme.".into());
    }
    let mut base = if let Some(include) = data.get("include") {
        let include = include
            .as_str()
            .ok_or("include must be a relative file path.")?;
        load(source, &reference(path, include)?, stack, total)?
    } else {
        serde_json::json!({})
    };
    data.as_object_mut().unwrap().remove("include");
    if data["settings"].is_array() {
        data["tokenColors"] = data["settings"].clone();
        textmate_colors(&mut data);
        data.as_object_mut().unwrap().remove("settings");
    }
    if let Some(file) = data["tokenColors"].as_str() {
        let tokens = textmate(source, &reference(path, file)?)?;
        data["tokenColors"] = tokens["tokenColors"].clone();
        if let Some(colors) = tokens["colors"].as_object() {
            let mut merged = data["colors"].as_object().cloned().unwrap_or_default();
            merged.extend(colors.clone());
            data["colors"] = merged.into();
        }
    }
    for field in ["colors", "semanticTokenColors"] {
        if let Some(values) = data.get(field) {
            let values = values
                .as_object()
                .ok_or_else(|| format!("{field} must be an object."))?;
            let mut combined = base[field].as_object().cloned().unwrap_or_default();
            for (key, value) in values {
                if field == "colors" && value == "default" {
                    combined.remove(key);
                } else if !value.is_null() {
                    combined.insert(key.clone(), value.clone());
                }
            }
            data[field] = combined.into();
        }
    }
    if let Some(tokens) = data.get("tokenColors") {
        let mut combined = base["tokenColors"].as_array().cloned().unwrap_or_default();
        combined.extend(
            tokens
                .as_array()
                .ok_or("tokenColors must be an array or TextMate path.")?
                .clone(),
        );
        data["tokenColors"] = combined.into();
    }
    // VS Code combines semanticHighlighting with logical OR across include files.
    if base["semanticHighlighting"] == true {
        data["semanticHighlighting"] = true.into();
    }
    base.as_object_mut()
        .unwrap()
        .extend(data.as_object().unwrap().clone());
    stack.pop();
    if serde_json::to_vec(&base).map_err(|e| e.to_string())?.len() as u64 > JSON_LIMIT {
        return Err("Resolved VS Code theme exceeds 256 KiB.".into());
    }
    Ok(base)
}

pub(super) fn validate(data: &Value) -> Result<(), String> {
    let data = data.as_object().ok_or("vscode must be an object.")?;
    if data.contains_key("include") || data.get("tokenColors").is_some_and(Value::is_string) {
        return Err("Import the VS Code theme to resolve external references first.".into());
    }
    for field in ["colors", "semanticTokenColors"] {
        if data.get(field).is_some_and(|v| !v.is_object()) {
            return Err(format!("vscode.{field} must be an object."));
        }
    }
    if data.get("tokenColors").is_some_and(|v| !v.is_array()) {
        return Err("vscode.tokenColors must be an array.".into());
    }
    if data
        .get("semanticHighlighting")
        .is_some_and(|v| !v.is_boolean())
    {
        return Err("vscode.semanticHighlighting must be a boolean.".into());
    }
    Ok(())
}

fn documents(path: &Path) -> Result<Vec<icons::Document>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("Choose a theme file or extension folder, not a symbolic link.".into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let is_vsix = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vsix"));
    let (mut source, entry) = if is_vsix {
        let bytes = read_limited(&path, PACKAGE_LIMIT)?;
        let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        if zip.len() > icons::ENTRY_LIMIT {
            return Err("VSIX packages may contain at most 8192 entries.".into());
        }
        let mut size = 0u64;
        let mut names = std::collections::HashSet::new();
        for i in 0..zip.len() {
            let file = zip.by_index(i).map_err(|e| e.to_string())?;
            if file.is_symlink()
                || file.enclosed_name().is_none()
                || !names.insert(file.name().to_owned())
            {
                return Err("VSIX contains unsafe or duplicate paths.".into());
            }
            size = size.checked_add(file.size()).ok_or("VSIX size overflow.")?;
            if size > PACKAGE_LIMIT {
                return Err("Uncompressed VSIX exceeds 64 MiB.".into());
            }
        }
        (Source::Vsix(Box::new(zip)), "package.json".to_string())
    } else if path.is_dir() {
        (Source::Folder(path), "package.json".to_string())
    } else {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Invalid theme file name.")?
            .to_string();
        (
            Source::Folder(path.parent().ok_or("Missing theme folder.")?.to_path_buf()),
            name,
        )
    };
    let (contributions, author) = if entry == "package.json" {
        let package = source.json(&entry)?;
        let mut themes = Vec::new();
        for (field, kind) in [
            ("themes", "color"),
            ("iconThemes", "file"),
            ("productIconThemes", "product"),
        ] {
            if let Some(contributions) = package["contributes"].get(field) {
                for contribution in contributions
                    .as_array()
                    .ok_or("Theme contributions must be arrays.")?
                {
                    let mut contribution = Value::Object(
                        contribution
                            .as_object()
                            .ok_or("Theme contributions must be objects.")?
                            .clone(),
                    );
                    contribution["lomiKind"] = kind.into();
                    themes.push(contribution);
                }
            }
        }
        if themes.is_empty() || themes.len() > 64 {
            return Err("An extension must contain 1–64 color or icon themes.".into());
        }
        (
            themes,
            package["publisher"].as_str().unwrap_or("").to_string(),
        )
    } else {
        (vec![serde_json::json!({"path": entry})], String::new())
    };
    let localized = source.json("package.nls.json").unwrap_or(Value::Null);
    let mut output = Vec::new();
    let mut total_size = 0u64;
    for contribution in contributions {
        let path = reference(
            "package.json",
            contribution["path"]
                .as_str()
                .ok_or("Theme contribution needs a path.")?,
        )?;
        let contribution_kind = contribution["lomiKind"].as_str().unwrap_or("auto");
        let detected =
            if contribution_kind == "auto" && !path.to_ascii_lowercase().ends_with(".tmtheme") {
                let raw = String::from_utf8(source.read_limit(&path, icons::ICON_JSON_LIMIT)?)
                    .map_err(|e| e.to_string())?;
                let data = parse_raw(raw.trim_start_matches('\u{feff}'))?;
                if data.get("iconDefinitions").is_some() {
                    if [
                        "file",
                        "folder",
                        "fileNames",
                        "fileExtensions",
                        "languageIds",
                        "folderNames",
                    ]
                    .iter()
                    .any(|key| data.get(key).is_some())
                    {
                        "file"
                    } else {
                        "product"
                    }
                } else {
                    "color"
                }
            } else {
                contribution_kind
            };
        if matches!(detected, "file" | "product") {
            let mut name = contribution["label"]
                .as_str()
                .or(contribution["id"].as_str())
                .unwrap_or(&path)
                .to_string();
            if let Some(key) = name.strip_prefix('%').and_then(|n| n.strip_suffix('%')) {
                name = localized[key]
                    .as_str()
                    .or(contribution["id"].as_str())
                    .unwrap_or(&name)
                    .to_string();
            }
            let mut document = icons::import(&mut source, &path, detected, &name)?;
            if !author.is_empty() {
                document.manifest["author"] = author.clone().into();
            }
            total_size += document
                .resources
                .values()
                .map(|bytes| bytes.len() as u64)
                .sum::<u64>();
            if total_size > PACKAGE_LIMIT {
                return Err("Imported themes exceed 64 MiB.".into());
            }
            output.push(document);
            continue;
        }
        let mut theme = load(&mut source, &path, &mut Vec::new(), &mut 0)?;
        let kind = match contribution["uiTheme"].as_str() {
            Some("vs") => "light",
            Some("vs-dark") => "dark",
            Some("hc-black") => "hcDark",
            Some("hc-light") => "hcLight",
            Some(_) => return Err("Unknown VS Code uiTheme.".into()),
            None => theme["type"].as_str().unwrap_or("dark"),
        }
        .to_string();
        let mut name = contribution["label"]
            .as_str()
            .or(theme["name"].as_str())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(&path)
            .to_string();
        if let Some(key) = name
            .strip_prefix('%')
            .and_then(|name| name.strip_suffix('%'))
        {
            name = localized[key]
                .as_str()
                .or(contribution["id"].as_str())
                .unwrap_or(&name)
                .to_string();
        }
        theme["type"] = kind.clone().into();
        validate(&theme)?;
        let mut manifest = serde_json::json!({
            "version": 2, "name": name,
            "appearance": if kind == "light" || kind == "hcLight" { "light" } else { "dark" },
            "vscode": theme,
        });
        if !author.is_empty() {
            manifest["author"] = author.clone().into();
        }
        total_size += serde_json::to_vec(&manifest)
            .map_err(|e| e.to_string())?
            .len() as u64;
        if total_size > PACKAGE_LIMIT {
            return Err("Imported themes exceed 64 MiB.".into());
        }
        output.push(icons::Document {
            manifest,
            resources: Default::default(),
        });
    }
    Ok(output)
}

pub(super) fn install(root: &Path, path: &Path) -> Result<Vec<String>, String> {
    let documents = documents(path)?;
    let staging = tempfile::tempdir_in(root).map_err(|e| e.to_string())?;
    let mut ids = Vec::new();
    for (index, document) in documents.iter().enumerate() {
        let mut suffix = index + 1;
        let mut id = format!("vscode-theme-{suffix}");
        while root.join(&id).exists() || ids.contains(&id) {
            suffix += 1;
            id = format!("vscode-theme-{suffix}");
        }
        let folder = staging.path().join(&id);
        fs::create_dir(&folder).map_err(|e| e.to_string())?;
        for (path, bytes) in &document.resources {
            let target = folder.join(path);
            fs::create_dir_all(target.parent().ok_or("Missing resource parent.")?)
                .map_err(|e| e.to_string())?;
            fs::write(target, bytes).map_err(|e| e.to_string())?;
        }
        let raw = serde_json::to_string_pretty(&document.manifest).map_err(|e| e.to_string())?;
        validate_modern(&folder, &parse_raw(&raw)?)?;
        fs::write(folder.join("theme.jsonc"), raw).map_err(|e| e.to_string())?;
        ids.push(id);
    }
    let mut committed = Vec::new();
    for id in &ids {
        if let Err(error) = fs::rename(staging.path().join(id), root.join(id)) {
            for path in committed {
                let _ = fs::remove_dir_all(path);
            }
            return Err(error.to_string());
        }
        committed.push(root.join(id));
    }
    Ok(ids)
}

#[tauri::command]
pub async fn import_vscode_themes(
    window: Window,
    app: tauri::AppHandle,
    path: String,
) -> Result<Vec<String>, String> {
    authorize(window.label(), true)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Themes>();
        let _guard = state.lock.lock().map_err(|e| e.to_string())?;
        install(&library(&app)?, Path::new(&path))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportTheme {
    pub name: String,
    pub theme: Value,
}
fn export(directory: &Path, name: &str, themes: &[ExportTheme]) -> Result<String, String> {
    if themes.is_empty() || themes.len() > 2 || name.is_empty() || name.chars().count() > 160 {
        return Err("Invalid color theme export.".into());
    }
    export_package(directory, name, themes, None)
}
fn export_package(
    directory: &Path,
    name: &str,
    themes: &[ExportTheme],
    icons: Option<&icons::Document>,
) -> Result<String, String> {
    if name.is_empty() || name.chars().count() > 160 {
        return Err("Invalid theme export name.".into());
    }
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if !directory.is_dir() {
        return Err("Choose an export folder.".into());
    }
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = format!("lomi-{}", slug.trim_matches('-'));
    let slug = slug.trim_end_matches('-');
    let mut target = directory.join(format!("{slug}.vsix"));
    let mut suffix = 2;
    while target.exists() {
        target = directory.join(format!("{slug}-{suffix}.vsix"));
        suffix += 1;
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&directory).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(temporary.as_file_mut());
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut contributions = Vec::new();
    let mut uncompressed_size = 0u64;
    let mut account = |size: usize| -> Result<(), String> {
        uncompressed_size += size as u64;
        if uncompressed_size > PACKAGE_LIMIT {
            return Err("Exported VSIX exceeds 64 MiB uncompressed.".into());
        }
        Ok(())
    };
    for (index, item) in themes.iter().enumerate() {
        if item.name.trim().is_empty() || item.name.chars().count() > 180 {
            return Err("Exported theme labels must contain 1–180 characters.".into());
        }
        validate(&item.theme)?;
        let raw = serde_json::to_string_pretty(&item.theme).map_err(|e| e.to_string())?;
        if raw.len() as u64 > JSON_LIMIT {
            return Err("Exported theme exceeds 256 KiB.".into());
        }
        account(raw.len())?;
        let kind = match item.theme["type"].as_str() {
            Some("dark") => "vs-dark",
            Some("light") => "vs",
            Some("hcDark") => "hc-black",
            Some("hcLight") => "hc-light",
            _ => return Err("Invalid exported theme type.".into()),
        };
        let file = format!("themes/theme-{index}-color-theme.json");
        zip.start_file(format!("extension/{file}"), options)
            .map_err(|e| e.to_string())?;
        zip.write_all(raw.as_bytes()).map_err(|e| e.to_string())?;
        contributions.push(
            serde_json::json!({ "label": item.name, "uiTheme": kind, "path": format!("./{file}") }),
        );
    }
    let mut package = serde_json::json!({ "name": slug, "displayName": name, "version": "1.0.0", "publisher": "lomi-local", "engines": { "vscode": "^1.85.0" }, "categories": ["Themes"], "contributes": { "themes": contributions } });
    if let Some(document) = icons {
        let field = match icons::kind(&document.manifest) {
            "file" => "iconThemes",
            "product" => "productIconThemes",
            _ => return Err("Invalid icon theme kind.".into()),
        };
        package["contributes"]
            .as_object_mut()
            .ok_or("Invalid contributions.")?
            .remove("themes");
        package["contributes"][field] =
            serde_json::json!([{"id":slug,"label":name,"path":"./icons/icons.json"}]);
        for (path, bytes) in &document.resources {
            account(bytes.len())?;
            relative(path)?;
            zip.start_file(format!("extension/icons/{path}"), options)
                .map_err(|e| e.to_string())?;
            zip.write_all(bytes).map_err(|e| e.to_string())?;
        }
    }
    let files = [
        ("extension/package.json", serde_json::to_string_pretty(&package).map_err(|e| e.to_string())?),
        ("extension/README.md", "# Local theme\n\nExported from Lomi for local use. Install this VSIX using Extensions: Install from VSIX in VS Code.\n\nColors, token rules and icon definitions are portable. Icon extensions include their images and fonts. Lomi layouts, CSS, backgrounds and plugin UI are not VS Code theme features. Imported semantic and contextual rules remain available in VS Code. Review the original theme's license before redistribution.\n".into()),
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="utf-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="json" ContentType="application/json"/><Default Extension="md" ContentType="text/markdown"/><Default Extension="txt" ContentType="text/plain"/><Default Extension="svg" ContentType="image/svg+xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpg" ContentType="image/jpeg"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="gif" ContentType="image/gif"/><Default Extension="webp" ContentType="image/webp"/><Default Extension="avif" ContentType="image/avif"/><Default Extension="ico" ContentType="image/x-icon"/><Default Extension="woff" ContentType="font/woff"/><Default Extension="woff2" ContentType="font/woff2"/><Default Extension="ttf" ContentType="font/ttf"/><Default Extension="otf" ContentType="font/otf"/><Default Extension="vsixmanifest" ContentType="text/xml"/></Types>"#.into()),
        ("extension.vsixmanifest", format!(r#"<?xml version="1.0" encoding="utf-8"?><PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011"><Metadata><Identity Language="en-US" Id="{slug}" Version="1.0.0" Publisher="lomi-local"/><DisplayName>{slug}</DisplayName><Description xml:space="preserve">Local Lomi theme</Description><Tags>theme</Tags><Categories>Themes</Categories><Properties><Property Id="Microsoft.VisualStudio.Code.Engine" Value="^1.85.0"/></Properties></Metadata><Installation><InstallationTarget Id="Microsoft.VisualStudio.Code"/></Installation><Dependencies/><Assets><Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true"/></Assets></PackageManifest>"#)),
    ];
    for (path, body) in files {
        account(body.len())?;
        zip.start_file(path, options).map_err(|e| e.to_string())?;
        zip.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist_noclobber(&target)
        .map_err(|e| e.to_string())?;
    Ok(target.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn export_vscode_theme(
    window: Window,
    directory: String,
    name: String,
    themes: Vec<ExportTheme>,
) -> Result<String, String> {
    authorize(window.label(), true)?;
    tauri::async_runtime::spawn_blocking(move || export(Path::new(&directory), &name, &themes))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn export_vscode_icon_theme(
    window: Window,
    app: tauri::AppHandle,
    directory: String,
    id: Option<String>,
    kind: String,
) -> Result<String, String> {
    authorize(window.label(), true)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Themes>();
        let _guard = state.lock.lock().map_err(|e| e.to_string())?;
        let document = if let Some(id) = id {
            icons::export_document(&theme_root(&app, &id)?)?
        } else {
            icons::builtin(&kind)?
        };
        if icons::kind(&document.manifest) != kind {
            return Err("Icon theme kind does not match.".into());
        }
        let name = document.manifest["name"]
            .as_str()
            .ok_or("Missing icon theme name.")?;
        export_package(Path::new(&directory), name, &[], Some(&document))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_vsix_roundtrips_fonts_images_and_all_associations() {
        for kind in ["file", "product"] {
            let source = icons::builtin(kind).unwrap();
            let dest = tempfile::tempdir().unwrap();
            let path = export_package(dest.path(), "Portable icons", &[], Some(&source)).unwrap();
            let library = tempfile::tempdir().unwrap();
            let ids = install(library.path(), Path::new(&path)).unwrap();
            assert_eq!(ids.len(), 1);
            let folder = library.path().join(&ids[0]);
            let installed = manifest(&folder).unwrap();
            assert_eq!(icons::kind(&installed), kind);
            let data = icons::load(&folder, &installed).unwrap().unwrap();
            let original =
                parse_raw(std::str::from_utf8(&source.resources["icons.json"]).unwrap()).unwrap();
            assert_eq!(
                data["iconDefinitions"].as_object().unwrap().len(),
                original["iconDefinitions"].as_object().unwrap().len()
            );
            let mut original = original.clone();
            let mut expected = Vec::new();
            icons::map_resources(&mut original, |path, _| {
                expected.push(source.resources[path].clone());
                Ok(path.into())
            })
            .unwrap();
            let mut actual = Vec::new();
            icons::map_resources(&mut data.clone(), |path, _| {
                actual.push(fs::read(inside(&folder, path)?).map_err(|e| e.to_string())?);
                Ok(path.into())
            })
            .unwrap();
            assert_eq!(actual, expected);
            let exported = icons::export_document(&folder).unwrap();
            let second = export_package(dest.path(), "Again", &[], Some(&exported)).unwrap();
            assert_eq!(documents(Path::new(&second)).unwrap().len(), 1);
        }
    }

    #[test]
    fn mixed_theme_extensions_install_atomically_and_ignore_executable_files() {
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        let icons = icons::builtin("file").unwrap();
        icons::write(&source.path().join("icons"), &icons).unwrap();
        let mut package: Value =
            serde_json::from_slice(&fs::read(source.path().join("package.json")).unwrap()).unwrap();
        package["contributes"]["iconThemes"] =
            serde_json::json!([{"id":"files","label":"Files","path":"icons/icons.json"}]);
        fs::write(
            source.path().join("package.json"),
            serde_json::to_vec(&package).unwrap(),
        )
        .unwrap();
        fs::write(
            source.path().join("do-not-run.js"),
            "throw Error('extension code ran')",
        )
        .unwrap();
        let library = tempfile::tempdir().unwrap();
        let ids = install(library.path(), source.path()).unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(
            icons::kind(&manifest(&library.path().join(&ids[2])).unwrap()),
            "file"
        );
        assert!(!library.path().join(&ids[2]).join("do-not-run.js").exists());
        fs::remove_file(source.path().join("icons/folder.svg")).unwrap();
        let empty = tempfile::tempdir().unwrap();
        assert!(install(empty.path(), source.path()).is_err());
        assert_eq!(fs::read_dir(empty.path()).unwrap().count(), 0);
    }

    #[test]
    fn icon_resources_cannot_escape_use_network_or_execute_scripts() {
        let source = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        for path in [
            "../outside.svg",
            "https://example.com/icon.svg",
            "file:///icon.svg",
            "icon.js",
            "a%2fb.svg",
            "a\\b.svg",
        ] {
            fs::write(source.path().join("icons.json"),serde_json::to_vec(&serde_json::json!({"iconDefinitions":{"file":{"iconPath":path}},"file":"file"})).unwrap()).unwrap();
            assert!(
                install(library.path(), &source.path().join("icons.json")).is_err(),
                "{path}"
            );
        }
        assert_eq!(fs::read_dir(library.path()).unwrap().count(), 0);
    }

    #[test]
    fn icon_import_preserves_association_order_and_rejects_invalid_contribution_objects() {
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("icons.json"), r##"{"iconDefinitions":{"z":{"fontCharacter":"z"},"a":{"fontCharacter":"a"}},"fileNames":{"z.ts":"z","a.ts":"a"}}"##).unwrap();
        let root = tempfile::tempdir().unwrap();
        let ids = install(root.path(), &source.path().join("icons.json")).unwrap();
        let installed = bundle(root.path(), &ids[0]).unwrap().icon_theme.unwrap();
        assert_eq!(
            installed["fileNames"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["z.ts", "a.ts"]
        );
        fs::write(
            source.path().join("package.json"),
            r#"{"contributes":{"iconThemes":["icons.json"]}}"#,
        )
        .unwrap();
        assert!(documents(source.path()).unwrap_err().contains("objects"));
    }

    #[cfg(unix)]
    #[test]
    fn icon_fonts_reject_symlinks_and_oversized_resources() {
        let source = tempfile::tempdir().unwrap();
        icons::write(source.path(), &icons::builtin("product").unwrap()).unwrap();
        let font = source.path().join("lucide.woff2");
        fs::remove_file(&font).unwrap();
        std::os::unix::fs::symlink("LICENSE.txt", &font).unwrap();
        assert!(manifest(source.path()).is_err());
        fs::remove_file(&font).unwrap();
        fs::File::create(&font)
            .unwrap()
            .set_len(ASSET_LIMIT + 1)
            .unwrap();
        assert!(manifest(source.path()).is_err());
    }
    fn fixture(root: &Path) {
        fs::create_dir(root.join("themes")).unwrap();
        fs::write(root.join("package.json"), r#"{"publisher":"author","main":"./do-not-run.js","contributes":{"themes":[{"label":"Portable","path":"./themes/dark.json","uiTheme":"hc-black"},{"label":"Light","path":"./themes/light.json","uiTheme":"vs"}]}}"#).unwrap();
        fs::write(root.join("base.json"), r##"{"colors":{"editor.background":"#123456","removed":"#fff"},"tokenColors":[{"scope":"comment","settings":{"foreground":"#aaa"}}],"semanticHighlighting":true,"semanticTokenColors":{"variable":"#def"}}"##).unwrap();
        fs::write(root.join("themes/dark.json"), r##"{// comment
            "include":"../base.json", "colors":{"editor.background":"#001122","removed":"default"},"tokenColors":"../syntax.tmTheme","semanticHighlighting":false,}"##).unwrap();
        fs::write(
            root.join("themes/light.json"),
            r##"{"colors":{"editor.background":"#ffffff"}}"##,
        )
        .unwrap();
        fs::write(root.join("syntax.tmTheme"), r##"<?xml version="1.0"?><plist version="1.0"><dict><key>settings</key><array><dict><key>scope</key><string>string</string><key>settings</key><dict><key>foreground</key><string>#abcdef</string></dict></dict></array></dict></plist>"##).unwrap();
    }
    #[test]
    fn imports_extension_includes_textmate_and_all_variants_without_code() {
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        let root = tempfile::tempdir().unwrap();
        let ids = install(root.path(), source.path()).unwrap();
        assert_eq!(ids.len(), 2);
        let first = manifest(&root.path().join(&ids[0])).unwrap();
        assert_eq!(first["vscode"]["colors"]["editor.background"], "#001122");
        assert!(first["vscode"]["colors"].get("removed").is_none());
        assert_eq!(first["vscode"]["tokenColors"].as_array().unwrap().len(), 2);
        assert_eq!(first["vscode"]["semanticHighlighting"], true);
        assert_eq!(first["vscode"]["semanticTokenColors"]["variable"], "#def");
        assert_eq!(first["vscode"]["type"], "hcDark");
        assert_eq!(first["appearance"], "dark");
        assert!(!root.path().join(&ids[0]).join("do-not-run.js").exists());
        assert_eq!(
            manifest(&root.path().join(&ids[1])).unwrap()["appearance"],
            "light"
        );
        assert_ne!(install(root.path(), source.path()).unwrap(), ids);
        // A VS Code extension may use a root theme.json, like a legacy Lomi package.
        fs::write(
            source.path().join("theme.json"),
            r##"{"colors":{"foreground":"#abcdef"}}"##,
        )
        .unwrap();
        assert!(super::super::import(root.path(), source.path()).is_ok());
    }
    #[test]
    fn export_vsix_reimports_without_losing_rules_or_overwriting() {
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        let docs = documents(source.path()).unwrap();
        let themes: Vec<_> = docs
            .iter()
            .map(|d| ExportTheme {
                name: d.manifest["name"].as_str().unwrap().into(),
                theme: d.manifest["vscode"].clone(),
            })
            .collect();
        let dest = tempfile::tempdir().unwrap();
        let path = export(dest.path(), "Portable & friends", &themes).unwrap();
        let reimport = documents(Path::new(&path)).unwrap();
        for (before, after) in docs.iter().zip(reimport.iter()) {
            assert_eq!(before.manifest["vscode"], after.manifest["vscode"]);
        }
        let bytes = fs::read(&path).unwrap();
        assert_ne!(
            export(dest.path(), "Portable & friends", &themes).unwrap(),
            path
        );
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn rejects_cycles_escaping_paths_and_failed_batch_without_partial_installs() {
        for path in [
            "../../secret",
            "/etc/passwd",
            "http://bad/theme",
            "C:\\theme",
            "a%2fsecret",
            "../secret",
        ] {
            assert!(reference("theme.json", path).is_err(), "{path}");
        }
        assert_eq!(
            reference("themes/dark.json", "../base.json").unwrap(),
            "base.json"
        );
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        let root = tempfile::tempdir().unwrap();
        fs::write(
            source.path().join("themes/light.json"),
            r#"{"include":"light.json"}"#,
        )
        .unwrap();
        assert!(install(root.path(), source.path())
            .unwrap_err()
            .contains("Cyclic"));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        fs::write(
            source.path().join("themes/light.json"),
            r#"{"include":"../../secret"}"#,
        )
        .unwrap();
        assert!(install(root.path(), source.path()).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn rejects_unsafe_zip_entries_and_malformed_json() {
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("bad.vsix");
        let mut zip = ZipWriter::new(fs::File::create(&path).unwrap());
        zip.start_file("../secret", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"secret").unwrap();
        zip.finish().unwrap();
        assert!(documents(&path).is_err());
        let file = source.path().join("bad.json");
        for text in [
            r#"{"colors":{},"colors":{}}"#,
            r#"{"name":"not a theme"}"#,
            r#"{"colors":[]}"#,
        ] {
            fs::write(&file, text).unwrap();
            assert!(documents(&file).is_err());
        }
    }
    #[test]
    fn bounds_textmate_nesting_before_serialization() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("deep.tmTheme");
        let xml = format!(
            "<plist version=\"1.0\"><dict><key>settings</key>{}<string>x</string>{}</dict></plist>",
            "<array>".repeat(300),
            "</array>".repeat(300)
        );
        fs::write(&path, xml).unwrap();
        assert!(documents(&path).unwrap_err().contains("32 levels"));
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_includes() {
        let source = tempfile::tempdir().unwrap();
        fixture(source.path());
        fs::remove_file(source.path().join("base.json")).unwrap();
        std::os::unix::fs::symlink("themes/light.json", source.path().join("base.json")).unwrap();
        assert!(documents(source.path()).is_err());
    }
}
