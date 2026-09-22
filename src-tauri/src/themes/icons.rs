use super::*;
use std::collections::{BTreeMap, HashSet};

pub(super) const ICON_JSON_LIMIT: u64 = 2 * 1024 * 1024;
pub(super) const ENTRY_LIMIT: usize = 8192;

fn document_bytes(data: &Value) -> Result<Vec<u8>, String> {
    let pretty = serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?;
    if pretty.len() as u64 <= ICON_JSON_LIMIT {
        return Ok(pretty);
    }
    let compact = serde_json::to_vec(data).map_err(|e| e.to_string())?;
    if compact.len() as u64 > ICON_JSON_LIMIT {
        return Err("Resolved icon document exceeds 2 MiB.".into());
    }
    Ok(compact)
}

pub(super) fn kind(data: &Value) -> &str {
    data["iconTheme"]["kind"].as_str().unwrap_or("color")
}

// Visit only resource fields defined by the icon theme format. Unknown metadata
// stays inert and survives export; no extension JavaScript is installed.
pub(super) fn map_resources(
    data: &mut Value,
    mut map: impl FnMut(&str, &str) -> Result<String, String>,
) -> Result<(), String> {
    let definitions = data["iconDefinitions"]
        .as_object_mut()
        .ok_or("iconDefinitions must be an object.")?;
    for definition in definitions.values_mut() {
        let definition = definition
            .as_object_mut()
            .ok_or("Icon definitions must be objects.")?;
        if let Some(path) = definition.get_mut("iconPath") {
            *path = map(path.as_str().ok_or("iconPath must be a string.")?, "image/")?.into();
        }
    }
    if let Some(fonts) = data.get_mut("fonts") {
        for font in fonts.as_array_mut().ok_or("fonts must be an array.")? {
            for source in font["src"]
                .as_array_mut()
                .ok_or("Font src must be an array.")?
            {
                let path = source["path"].as_str().ok_or("Font source needs a path.")?;
                source["path"] = map(path, "font/")?.into();
            }
        }
    }
    Ok(())
}

pub(super) fn validate(data: &Value, kind: &str) -> Result<(), String> {
    if !matches!(kind, "file" | "product") {
        return Err("Unknown icon theme kind.".into());
    }
    let definitions = data["iconDefinitions"]
        .as_object()
        .ok_or("iconDefinitions must be an object.")?;
    if definitions.len() > ENTRY_LIMIT {
        return Err("Too many icon definitions.".into());
    }
    let mut fonts = HashSet::new();
    if let Some(value) = data.get("fonts") {
        let array = value.as_array().ok_or("fonts must be an array.")?;
        if array.len() > 32 {
            return Err("Icon themes support at most 32 fonts.".into());
        }
        for font in array {
            let id = font["id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .ok_or("Invalid icon font id.")?;
            if !fonts.insert(id) {
                return Err("Duplicate icon font id.".into());
            }
            let sources = font["src"]
                .as_array()
                .filter(|s| !s.is_empty() && s.len() <= 8)
                .ok_or("A font needs 1–8 sources.")?;
            for source in sources {
                if !matches!(
                    source["format"].as_str(),
                    Some("woff" | "woff2" | "truetype" | "opentype")
                ) {
                    return Err("Unsupported icon font format.".into());
                }
            }
            for field in ["weight", "style", "size"] {
                if let Some(value) = font.get(field) {
                    if value.as_str().is_none_or(|s| s.len() > 100) {
                        return Err(format!("Invalid font {field}."));
                    }
                }
            }
        }
    }
    if kind == "product" && fonts.is_empty() {
        return Err("Product icon themes need a font.".into());
    }
    for definition in definitions.values() {
        if !definition.is_object() {
            return Err("Icon definitions must be objects.".into());
        }
        if let Some(id) = definition.get("fontId") {
            if !id.as_str().is_some_and(|s| fonts.contains(s)) {
                return Err("Unknown icon fontId.".into());
            }
        }
        if let Some(character) = definition.get("fontCharacter") {
            if character.as_str().is_none_or(|s| {
                s.is_empty() || s.chars().count() > 16 || s.chars().any(char::is_control)
            }) {
                return Err("Invalid icon fontCharacter.".into());
            }
        }
        if kind == "product"
            && (definition.get("fontCharacter").is_none() || definition.get("iconPath").is_some())
        {
            return Err("Product icons must use fontCharacter.".into());
        }
        for field in ["fontColor", "fontSize"] {
            if let Some(value) = definition.get(field) {
                if value.as_str().is_none_or(|s| s.len() > 100) {
                    return Err(format!("Invalid icon {field}."));
                }
            }
        }
    }
    for variant in [Some(data), data.get("light"), data.get("highContrast")]
        .into_iter()
        .flatten()
    {
        if !variant.is_object() {
            return Err("Icon variants must be objects.".into());
        }
        for field in [
            "file",
            "folder",
            "folderExpanded",
            "rootFolder",
            "rootFolderExpanded",
        ] {
            if let Some(value) = variant.get(field) {
                if !value.as_str().is_some_and(|s| definitions.contains_key(s)) {
                    return Err(format!("Unknown icon in {field}."));
                }
            }
        }
        for field in [
            "fileNames",
            "fileExtensions",
            "languageIds",
            "folderNames",
            "folderNamesExpanded",
            "rootFolderNames",
            "rootFolderNamesExpanded",
        ] {
            if let Some(value) = variant.get(field) {
                let values = value
                    .as_object()
                    .ok_or("Icon associations must be objects.")?;
                for id in values.values() {
                    if !id.as_str().is_some_and(|s| definitions.contains_key(s)) {
                        return Err(format!("Unknown icon in {field}."));
                    }
                }
            }
        }
    }
    for field in [
        "hidesExplorerArrows",
        "showLanguageModeIcons",
        "usesCurrentColor",
    ] {
        if data.get(field).is_some_and(|value| !value.is_boolean()) {
            return Err(format!("{field} must be boolean."));
        }
    }
    Ok(())
}

pub(super) fn load(root: &Path, manifest: &Value) -> Result<Option<Value>, String> {
    let Some(reference) = manifest.get("iconTheme") else {
        return Ok(None);
    };
    let kind = kind(manifest);
    let path = reference["path"]
        .as_str()
        .ok_or("iconTheme needs a path.")?;
    let raw = String::from_utf8(read_limited(&inside(root, path)?, ICON_JSON_LIMIT)?)
        .map_err(|e| e.to_string())?;
    let mut data = parse_raw(raw.trim_start_matches('\u{feff}'))?;
    validate(&data, kind)?;
    let mut size = raw.len() as u64;
    let mut seen = HashSet::new();
    map_resources(&mut data, |resource_path, resource_type| {
        let path = vscode::reference(path, resource_path)?;
        resource(root, &path, resource_type)?;
        if seen.insert(path.clone()) {
            size += fs::metadata(inside(root, &path)?)
                .map_err(|e| e.to_string())?
                .len();
            if size > PACKAGE_LIMIT || seen.len() > ENTRY_LIMIT {
                return Err("Icon package exceeds resource limits.".into());
            }
        }
        Ok(path)
    })?;
    Ok(Some(data))
}

#[derive(Debug)]
pub(super) struct Document {
    pub manifest: Value,
    pub resources: BTreeMap<String, Vec<u8>>,
}

pub(super) fn builtin(kind: &str) -> Result<Document, String> {
    let (raw, assets): (&str, Vec<(&str, &[u8])>) = match kind {
        "file" => (
            include_str!("../../../themes/icons/file.json"),
            vec![
                ("file.svg", include_bytes!("../../../themes/icons/file.svg")),
                (
                    "folder.svg",
                    include_bytes!("../../../themes/icons/folder.svg"),
                ),
                (
                    "folder-open.svg",
                    include_bytes!("../../../themes/icons/folder-open.svg"),
                ),
            ],
        ),
        "product" => (
            include_str!("../../../themes/icons/product.json"),
            vec![(
                "lucide.woff2",
                include_bytes!("../../../themes/icons/lucide.woff2"),
            )],
        ),
        _ => return Err("Unknown icon theme kind.".into()),
    };
    let mut resources: BTreeMap<String, Vec<u8>> = assets
        .into_iter()
        .map(|(path, bytes)| (path.into(), bytes.to_vec()))
        .collect();
    resources.insert("icons.json".into(), raw.as_bytes().to_vec());
    resources.insert(
        "LICENSE.txt".into(),
        include_bytes!("../../../themes/icons/LICENSE.txt").to_vec(),
    );
    Ok(Document {
        manifest: serde_json::json!({"version":2,"name":if kind == "file" {"Lomi file icons"} else {"Lomi interface icons"},"iconTheme":{"kind":kind,"path":"icons.json"}}),
        resources,
    })
}

pub(super) fn export_document(root: &Path) -> Result<Document, String> {
    let manifest = manifest(root)?;
    let mut data = load(root, &manifest)?.ok_or("This is not an icon theme.")?;
    let mut resources = BTreeMap::new();
    let mut size = 0u64;
    map_resources(&mut data, |path, _| {
        if !resources.contains_key(path) {
            let bytes = read_limited(&inside(root, path)?, ASSET_LIMIT)?;
            size += bytes.len() as u64;
            if size > PACKAGE_LIMIT {
                return Err("Icon resources exceed 64 MiB.".into());
            }
            resources.insert(path.to_owned(), bytes);
        }
        Ok(path.to_owned())
    })?;
    resources.insert("icons.json".into(), document_bytes(&data)?);
    // Retain licenses when present without copying executable extension files.
    for path in ["LICENSE", "LICENSE.txt", "LICENSE.md"] {
        if root.join(path).try_exists().map_err(|e| e.to_string())? {
            resources.insert(path.into(), read_limited(&inside(root, path)?, JSON_LIMIT)?);
        }
    }
    Ok(Document {
        manifest,
        resources,
    })
}

pub(super) fn write(folder: &Path, document: &Document) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    for (path, bytes) in &document.resources {
        relative(path)?;
        let target = folder.join(path);
        fs::create_dir_all(target.parent().ok_or("Missing resource parent.")?)
            .map_err(|e| e.to_string())?;
        fs::write(target, bytes).map_err(|e| e.to_string())?;
    }
    fs::write(
        folder.join("theme.jsonc"),
        serde_json::to_vec_pretty(&document.manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    manifest(folder)?;
    Ok(())
}

pub(super) fn import(
    source: &mut vscode::Source,
    path: &str,
    kind: &str,
    name: &str,
) -> Result<Document, String> {
    let raw =
        String::from_utf8(source.read_limit(path, ICON_JSON_LIMIT)?).map_err(|e| e.to_string())?;
    let mut data = parse_raw(raw.trim_start_matches('\u{feff}'))?;
    validate(&data, kind)?;
    let mut resources = BTreeMap::new();
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut size = raw.len() as u64;
    map_resources(&mut data, |resource_path, resource_type| {
        let path = vscode::reference(path, resource_path)?;
        if !mime(&path).is_some_and(|mime| mime.starts_with(resource_type)) {
            return Err(format!("Unsupported icon resource: {path}"));
        }
        if let Some(alias) = aliases.get(&path) {
            return Ok(alias.clone());
        }
        let bytes = source.read_limit(&path, ASSET_LIMIT)?;
        size += bytes.len() as u64;
        if size > PACKAGE_LIMIT || resources.len() >= ENTRY_LIMIT - 4 {
            return Err("Icon package exceeds resource limits.".into());
        }
        let extension = Path::new(&path)
            .extension()
            .and_then(|s| s.to_str())
            .ok_or("Missing resource extension.")?;
        let alias = format!("assets/{}.{}", resources.len(), extension);
        resources.insert(alias.clone(), bytes);
        aliases.insert(path, alias.clone());
        Ok(alias)
    })?;
    for name in ["LICENSE", "LICENSE.txt", "LICENSE.md"] {
        for candidate in [vscode::reference(path, name)?, name.into()] {
            if let Ok(bytes) = source.read_limit(&candidate, JSON_LIMIT) {
                resources.insert(name.into(), bytes);
                break;
            }
        }
    }
    resources.insert("icons.json".into(), document_bytes(&data)?);
    Ok(Document {
        manifest: serde_json::json!({"version":2,"name":name,"iconTheme":{"kind":kind,"path":"icons.json"}}),
        resources,
    })
}
