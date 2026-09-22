use quick_xml::{events::Event, Reader, XmlVersion};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};

const XML_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub enum Host {
    MacArm64,
    MacX64,
    LinuxX64,
    WindowsX64,
}
impl Host {
    pub fn native() -> Result<Self, String> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Ok(Self::MacArm64),
            ("macos", "x86_64") => Ok(Self::MacX64),
            ("linux", "x86_64") => Ok(Self::LinuxX64),
            ("windows", "x86_64") => Ok(Self::WindowsX64),
            _ => Err(
                "No verified Android tool distribution is configured for this host architecture."
                    .into(),
            ),
        }
    }

    pub fn abi(self) -> &'static str {
        if matches!(self, Self::MacArm64) {
            "arm64-v8a"
        } else {
            "x86_64"
        }
    }

    fn matches(self, archive: &Node) -> bool {
        let os = match self {
            Self::MacArm64 | Self::MacX64 => "macosx",
            Self::LinuxX64 => "linux",
            Self::WindowsX64 => "windows",
        };
        let arch = archive.value(&["host-arch"]);
        archive.value(&["host-os"]).is_none_or(|value| value == os)
            && archive
                .value(&["host-bits"])
                .is_none_or(|value| value == "64")
            && arch.is_none_or(|value| {
                if matches!(self, Self::MacArm64) {
                    matches!(value, "aarch64" | "arm64")
                } else {
                    matches!(value, "x64" | "x86_64" | "x86")
                }
            })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct License {
    pub id: String,
    pub text: String,
    pub digest: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub id: String,
    pub revision: String,
    pub name: String,
    pub license: String,
    pub url: String,
    pub size: u64,
    pub sha1: String,
    pub image: Option<SystemImage>,
    pub dependencies: Vec<Dependency>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub id: String,
    pub minimum_revision: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemImage {
    pub api: u32,
    pub minor_api: u32,
    pub tag: String,
    pub abi: String,
}

#[derive(Debug, Serialize)]
pub struct Catalog {
    pub licenses: Vec<License>,
    pub packages: Vec<Package>,
}

/// Exit code zero is insufficient: some CLI package identifiers are silently ignored.
/// The installed metadata must describe the selected revision and accepted license.
pub fn verify_local_package(
    xml: &str,
    expected: &Package,
    accepted: &License,
) -> Result<(), String> {
    let root = parse(xml)?;
    let local: Vec<_> = root
        .children
        .iter()
        .filter(|node| node.name == "localPackage")
        .collect();
    let [local] = local.as_slice() else {
        return Err("SDK installation did not produce exactly one package descriptor.".into());
    };
    for field in ["revision", "uses-license"] {
        if local
            .children
            .iter()
            .filter(|node| node.name == field)
            .count()
            != 1
        {
            return Err("Installed SDK metadata contains ambiguous package fields.".into());
        }
    }
    let normalized = |text: &str| -> Result<Vec<u32>, String> {
        let mut values = text
            .split('.')
            .map(|part| {
                part.parse::<u32>()
                    .map_err(|_| "Invalid SDK revision".into())
            })
            .collect::<Result<Vec<_>, String>>()?;
        if values.is_empty() || values.len() > 3 {
            return Err("Invalid SDK revision".into());
        }
        values.resize(3, 0);
        Ok(values)
    };
    let revision = local
        .child("revision")
        .ok_or("Missing installed revision")?;
    for field in ["major", "minor", "micro", "preview"] {
        if revision
            .children
            .iter()
            .filter(|node| node.name == field)
            .count()
            > 1
        {
            return Err("Installed SDK metadata contains ambiguous revision fields.".into());
        }
    }
    let actual_revision = ["major", "minor", "micro"]
        .map(|part| revision.value(&[part]).unwrap_or("0"))
        .join(".");
    if local.attribute("path")? != expected.id
        || revision.value(&["major"]).is_none()
        || revision
            .value(&["preview"])
            .is_some_and(|value| value != "0")
        || normalized(&actual_revision)? != normalized(&expected.revision)?
    {
        return Err("Installed Android package differs from the selected stable revision.".into());
    }
    let licenses: Vec<_> = root
        .children
        .iter()
        .filter(|node| node.name == "license")
        .collect();
    let [license] = licenses.as_slice() else {
        return Err("Installed Android package has ambiguous license metadata.".into());
    };
    if accepted.id != expected.license
        || license.attribute("id")? != accepted.id
        || local
            .child("uses-license")
            .ok_or("Missing installed license reference")?
            .attribute("ref")?
            != accepted.id
        || accepted.digest != format!("{:x}", Sha256::digest(accepted.text.as_bytes()))
        || license.text != accepted.text
    {
        return Err("Installed Android license differs from the accepted catalog text.".into());
    }
    Ok(())
}

/// Reads only provider metadata. Package installation is a separate, consented operation.
pub fn packages(xml: &str, base: &str, host: Host) -> Result<Catalog, String> {
    if !matches!(
        base,
        "https://dl.google.com/android/repository/"
            | "https://dl.google.com/android/repository/sys-img/android/"
            | "https://dl.google.com/android/repository/sys-img/google_apis/"
            | "https://dl.google.com/android/repository/sys-img/google_apis_playstore/"
    ) {
        return Err("Untrusted Android catalog location".into());
    }
    let root = parse(xml)?;
    let licenses: Vec<_> = root
        .children
        .iter()
        .filter(|node| node.name == "license")
        .map(|node| {
            let id = node.attribute("id")?.to_string();
            let text = node.text.clone();
            if !component(&id) || text.trim().is_empty() || text.len() > 256 * 1024 {
                return Err("Invalid Android license in catalog".into());
            }
            Ok(License {
                id,
                digest: format!("{:x}", Sha256::digest(text.as_bytes())),
                text,
            })
        })
        .collect::<Result<_, String>>()?;
    if licenses
        .iter()
        .map(|license| &license.id)
        .collect::<BTreeSet<_>>()
        .len()
        != licenses.len()
    {
        return Err("Duplicate Android license ID".into());
    }
    let mut packages = Vec::new();
    for node in root
        .children
        .iter()
        .filter(|node| node.name == "remotePackage")
    {
        let id = node.attribute("path")?;
        if !(matches!(id, "emulator" | "platform-tools")
            || id.starts_with("cmdline-tools;")
            || id.starts_with("system-images;"))
        {
            continue;
        }
        if node.child("obsolete").is_some()
            || node
                .child("channelRef")
                .and_then(|n| n.attrs.get("ref"))
                .is_some_and(|channel| channel != "channel-0")
            || node
                .value(&["revision", "preview"])
                .is_some_and(|value| value != "0")
        {
            continue;
        }
        if !id.split(';').all(component) {
            return Err("Invalid SDK package identifier".into());
        }
        let revision = node
            .child("revision")
            .ok_or("Missing SDK package revision")?;
        let mut parts = Vec::new();
        for part in ["major", "minor", "micro"] {
            if let Some(value) = revision.value(&[part]) {
                parts.push(
                    value
                        .parse::<u32>()
                        .map_err(|_| "Invalid SDK revision")?
                        .to_string(),
                );
            }
        }
        if parts.is_empty() {
            return Err("Empty SDK revision".into());
        }
        let license = node
            .child("uses-license")
            .ok_or("SDK package has no license")?
            .attribute("ref")?;
        if !licenses.iter().any(|candidate| candidate.id == license) {
            return Err("SDK package references an unavailable license".into());
        }
        let image = if id.starts_with("system-images;") {
            let details = node.child("type-details").ok_or("Missing image details")?;
            let abi = details.required(&["abi"])?;
            let base_tag = details.required(&["tag", "id"])?;
            // New catalogs describe page size as a second tag, while the
            // package path still uses the combined legacy identifier.
            let page_size_16k = details
                .children
                .iter()
                .any(|node| node.name == "tag" && node.value(&["id"]) == Some("page_size_16kb"));
            let tag = if page_size_16k && !base_tag.ends_with("_ps16k") {
                format!("{base_tag}_ps16k")
            } else {
                base_tag.to_string()
            };
            if abi != host.abi()
                || !matches!(
                    tag.as_str(),
                    "default"
                        | "google_apis"
                        | "google_apis_playstore"
                        | "google_apis_ps16k"
                        | "google_apis_playstore_ps16k"
                )
            {
                continue;
            }
            // Current catalogs also contain preview codenames, "36x" extension
            // tracks and decimal stable API levels. Unsupported tracks must not
            // prevent browsing the ordinary compatible images.
            let Some((api, inline_minor)) = api_level(details.required(&["api-level"])?) else {
                continue;
            };
            if api < 26 || details.value(&["codename"]).is_some() {
                continue;
            }
            let minor_api = details
                .value(&["minor-api-level"])
                .map(|value| {
                    value
                        .parse::<u32>()
                        .map_err(|_| "Invalid minor Android API level")
                })
                .transpose()?
                .unwrap_or(inline_minor);
            if inline_minor != 0 && minor_api != inline_minor {
                return Err("Conflicting Android minor API levels".into());
            }
            Some(SystemImage {
                api,
                minor_api,
                abi: abi.into(),
                tag,
            })
        } else {
            None
        };
        let Some(archive) = node.child("archives").and_then(|archives| {
            archives
                .children
                .iter()
                .find(|a| a.name == "archive" && host.matches(a))
        }) else {
            continue;
        };
        let complete = archive
            .child("complete")
            .ok_or("SDK archive has no complete download")?;
        let file = complete.required(&["url"])?;
        if file.is_empty()
            || file.len() > 240
            || !file
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || file.contains("..")
            || !file.ends_with(".zip")
        {
            return Err("SDK archive URL is outside the provider directory".into());
        }
        let checksum = complete
            .child("checksum")
            .ok_or("SDK archive has no checksum")?;
        let sha1 = checksum.text.trim();
        if checksum.attribute("type")? != "sha1"
            || sha1.len() != 40
            || !sha1.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Unsupported SDK archive checksum".into());
        }
        let size = complete
            .required(&["size"])?
            .parse::<u64>()
            .map_err(|_| "Invalid SDK archive size")?;
        if size == 0 || size > 12 * 1024 * 1024 * 1024 {
            return Err("SDK archive exceeds the download limit".into());
        }
        packages.push(Package {
            id: id.into(),
            revision: parts.join("."),
            name: node.required(&["display-name"])?.into(),
            license: license.into(),
            url: format!("{base}{file}"),
            size,
            sha1: sha1.to_ascii_lowercase(),
            image,
            dependencies: node
                .child("dependencies")
                .map(|dependencies| {
                    dependencies
                        .children
                        .iter()
                        .filter(|n| n.name == "dependency")
                        .map(|dependency| {
                            let id = dependency.attribute("path")?;
                            if !id.split(';').all(component) {
                                return Err("Invalid SDK dependency ID".into());
                            }
                            let minimum_revision = dependency
                                .child("min-revision")
                                .map(|revision| {
                                    ["major", "minor", "micro"]
                                        .iter()
                                        .filter_map(|part| revision.value(&[part]))
                                        .map(|v| {
                                            v.parse::<u32>().map(|n| n.to_string()).map_err(|_| {
                                                "Invalid minimum SDK revision".to_string()
                                            })
                                        })
                                        .collect::<Result<Vec<_>, String>>()
                                        .map(|parts| parts.join("."))
                                })
                                .transpose()?;
                            if minimum_revision.as_ref().is_some_and(String::is_empty) {
                                return Err("Empty minimum SDK revision".into());
                            }
                            Ok(Dependency {
                                id: id.into(),
                                minimum_revision,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?
                .unwrap_or_default(),
        });
    }
    Ok(Catalog { licenses, packages })
}

pub(super) fn api_level(value: &str) -> Option<(u32, u32)> {
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map(str::parse).transpose().ok()?.unwrap_or(0);
    if parts.next().is_some() {
        None
    } else {
        Some((major, minor))
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
    pub min_api: u32,
    pub min_minor_api: u32,
}

/// Both files belong to the installed sdklib provider, not a fixed phone list.
/// Read bounded XML entries directly; do not extract the executable JAR.
pub fn profiles_from_tools(tools: &Path, pixel_limit: u32) -> Result<Vec<Profile>, String> {
    let file = std::fs::File::open(tools.join("lib/sdklib/tools.sdklib.jar"))
        .map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut result = BTreeMap::new();
    for name in [
        "com/android/sdklib/devices/devices.xml",
        "com/android/sdklib/devices/nexus.xml",
    ] {
        let entry = archive.by_name(name).map_err(|e| e.to_string())?;
        if entry.size() > XML_LIMIT as u64 {
            return Err("Android profile catalog exceeds its size limit".into());
        }
        let mut xml = String::new();
        entry
            .take(XML_LIMIT as u64 + 1)
            .read_to_string(&mut xml)
            .map_err(|e| e.to_string())?;
        for profile in profiles(&xml, pixel_limit)? {
            if result
                .get(&profile.id)
                .is_some_and(|existing| existing != &profile)
            {
                return Err("Conflicting Android hardware profile IDs".into());
            }
            result.insert(profile.id.clone(), profile);
        }
    }
    if result.is_empty() {
        return Err("The installed tools contain no compatible phone profiles.".into());
    }
    Ok(result.into_values().collect())
}

pub fn profiles(xml: &str, pixel_limit: u32) -> Result<Vec<Profile>, String> {
    let root = parse(xml)?;
    let mut result = Vec::new();
    for device in root.children.iter().filter(|node| node.name == "device") {
        if device
            .attrs
            .get("deprecated")
            .is_some_and(|value| value == "true")
        {
            continue;
        }
        let Some(hardware) = device.child("hardware") else {
            continue;
        };
        let Some(screen) = hardware.child("screen") else {
            continue;
        };
        if hardware.child("hinge").is_some() || screen.child("foldable-region").is_some() {
            continue;
        }
        let diagonal = screen
            .required(&["diagonal-length"])?
            .parse::<f64>()
            .map_err(|_| "Invalid profile diagonal")?;
        if !(3.0..=7.0).contains(&diagonal) {
            continue;
        }
        let number = |path: &[&str]| -> Result<u32, String> {
            screen
                .required(path)?
                .parse()
                .map_err(|_| "Invalid profile dimensions".into())
        };
        let width = number(&["dimensions", "x-dimension"])?;
        let height = number(&["dimensions", "y-dimension"])?;
        if width < 320
            || height < width
            || width > super::rpc::MAX_DISPLAY_EDGE
            || height > super::rpc::MAX_DISPLAY_EDGE
            || width
                .checked_mul(height)
                .is_none_or(|pixels| pixels > pixel_limit)
        {
            continue;
        }
        let density = screen.required(&["pixel-density"])?;
        let dpi = match density {
            "ldpi" => 120,
            "mdpi" => 160,
            "tvdpi" => 213,
            "hdpi" => 240,
            "xhdpi" => 320,
            "xxhdpi" => 480,
            "xxxhdpi" => 640,
            _ => density
                .strip_suffix("dpi")
                .unwrap_or(density)
                .parse()
                .map_err(|_| "Unsupported profile density")?,
        };
        let id = device.required(&["id"])?;
        if id.len() > 160 || id.starts_with('-') || id.chars().any(char::is_control) {
            return Err("Invalid profile ID".into());
        }
        let minimum = device
            .value(&["software", "api-level"])
            .unwrap_or("0")
            .split('-')
            .next()
            .unwrap();
        let (major, minor) = minimum.split_once('.').unwrap_or((minimum, "0"));
        let min_api = major.parse().map_err(|_| "Invalid profile API range")?;
        let min_minor_api = minor
            .parse()
            .map_err(|_| "Invalid profile minor API range")?;
        result.push(Profile {
            id: id.into(),
            name: device.required(&["name"])?.into(),
            width,
            height,
            dpi,
            min_api,
            min_minor_api,
        });
    }
    Ok(result)
}

fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && !value.contains("..")
}

#[derive(Default)]
struct Node {
    name: String,
    attrs: BTreeMap<String, String>,
    text: String,
    children: Vec<Node>,
}
impl Node {
    fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|node| node.name == name)
    }
    fn value(&self, path: &[&str]) -> Option<&str> {
        let mut node = self;
        for name in path {
            node = node.child(name)?;
        }
        Some(node.text.trim())
    }
    fn required(&self, path: &[&str]) -> Result<&str, String> {
        self.value(path)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Missing Android catalog field {}", path.join("/")))
    }
    fn attribute(&self, key: &str) -> Result<&str, String> {
        self.attrs
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("Missing Android catalog attribute {key}"))
    }
}

fn parse(xml: &str) -> Result<Node, String> {
    if xml.len() > XML_LIMIT {
        return Err("Android catalog exceeds 8 MiB".into());
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = vec![Node::default()];
    let mut count = 0;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(element) => {
                count += 1;
                if count > 50000 || stack.len() > 24 {
                    return Err("Android XML structure exceeds limits".into());
                }
                let mut node = Node {
                    name: std::str::from_utf8(element.local_name().as_ref())
                        .map_err(|e| e.to_string())?
                        .into(),
                    ..Node::default()
                };
                for attr in element.attributes() {
                    let attr = attr.map_err(|e| e.to_string())?;
                    if node.attrs.len() >= 32 || attr.value.len() > 8192 {
                        return Err("Android XML attribute exceeds limits".into());
                    }
                    node.attrs.insert(
                        std::str::from_utf8(attr.key.as_ref())
                            .map_err(|e| e.to_string())?
                            .into(),
                        attr.normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|e| e.to_string())?
                            .into_owned(),
                    );
                }
                stack.push(node);
            }
            Event::End(_) => {
                if stack.len() < 2 {
                    return Err("Unbalanced Android XML".into());
                }
                let node = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(node);
            }
            Event::Text(text) => stack
                .last_mut()
                .unwrap()
                .text
                .push_str(&text.decode().map_err(|e| e.to_string())?),
            Event::CData(text) => stack
                .last_mut()
                .unwrap()
                .text
                .push_str(&text.decode().map_err(|e| e.to_string())?),
            Event::GeneralRef(reference) => {
                let value = if let Some(character) =
                    reference.resolve_char_ref().map_err(|e| e.to_string())?
                {
                    character.to_string()
                } else {
                    match reference.decode().map_err(|e| e.to_string())?.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => return Err("External XML entities are forbidden".into()),
                    }
                    .to_string()
                };
                stack.last_mut().unwrap().text.push_str(&value);
            }
            Event::DocType(_) => return Err("Android catalogs must not contain a DTD".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 || stack[0].children.len() != 1 || !stack[0].text.trim().is_empty() {
        return Err("Android catalog requires one complete XML document".into());
    }
    Ok(stack.pop().unwrap().children.pop().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_modern_images_preserve_minor_api_and_page_size_while_previews_are_excluded() {
        let package = |id: &str, tag: &str, api: &str, codename: &str, channel: u8| {
            format!(
            "<remotePackage path='system-images;android-{id};{tag};arm64-v8a'><type-details><api-level>{api}</api-level>{codename}<abi>arm64-v8a</abi><tag><id>{tag}</id></tag></type-details><revision><major>5</major></revision><display-name>System image</display-name><uses-license ref='terms'/><channelRef ref='channel-{channel}'/><archives><archive><complete><size>1000</size><checksum type='sha1'>{}</checksum><url>image.zip</url></complete></archive></archives></remotePackage>", "a".repeat(40)
        )
        };
        let xml = format!(
            "<repository><license id='terms'>Terms</license>{}{}{}{}</repository>",
            package("37.2", "google_apis_playstore_ps16k", "37.2", "", 0),
            package(
                "37.2-beta3",
                "google_apis_playstore_ps16k",
                "37.1",
                "<codename>DEV</codename>",
                0
            ),
            package("38", "google_apis", "38", "", 1),
            package("36-ext19", "google_apis", "36x", "", 0)
        );
        let xml = xml.replace(
            "<tag><id>google_apis_playstore_ps16k</id></tag>",
            "<tag><id>google_apis_playstore</id></tag><tag><id>page_size_16kb</id></tag>",
        );
        let result = packages(
            &xml,
            "https://dl.google.com/android/repository/sys-img/google_apis_playstore/",
            Host::MacArm64,
        )
        .unwrap();
        assert_eq!(result.packages.len(), 1);
        let image = result.packages[0].image.as_ref().unwrap();
        assert_eq!((image.api, image.minor_api), (37, 2));
        assert_eq!(image.tag, "google_apis_playstore_ps16k");
        assert!(packages(
            &xml,
            "https://dl.google.com/android/repository/sys-img/google_apis_playstore/",
            Host::WindowsX64
        )
        .unwrap()
        .packages
        .is_empty());
    }

    #[test]
    fn modern_phone_profiles_use_hardware_bounds_and_preserve_minor_api_requirements() {
        let xml = "<devices><device><id>pixel_10_pro_xl</id><name>Pixel 10 Pro XL</name><hardware><screen><diagonal-length>6.8</diagonal-length><pixel-density>480dpi</pixel-density><dimensions><x-dimension>1344</x-dimension><y-dimension>2992</y-dimension></dimensions></screen></hardware><software><api-level>36.1-</api-level></software></device></devices>";
        assert!(profiles(xml, super::super::rpc::MAX_PIXELS)
            .unwrap()
            .is_empty());
        let result = profiles(xml, super::super::rpc::MAX_DISPLAY_PIXELS).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!((result[0].min_api, result[0].min_minor_api), (36, 1));
        assert_eq!(
            (result[0].width, result[0].height, result[0].dpi),
            (1344, 2992, 480)
        );
        assert!(profiles(
            &xml.replace("<screen>", "<hinge/><screen>"),
            super::super::rpc::MAX_DISPLAY_PIXELS
        )
        .unwrap()
        .is_empty());
        assert!(profiles(
            &xml.replace("2992", "5000"),
            super::super::rpc::MAX_DISPLAY_PIXELS
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn installed_revision_and_license_must_match_the_selected_catalog() {
        let license = License {
            id: "test-license".into(),
            text: "Accepted terms".into(),
            digest: format!("{:x}", Sha256::digest(b"Accepted terms")),
        };
        let package = Package {
            id: "platform-tools".into(),
            revision: "37.0.1".into(),
            name: "Tools".into(),
            license: license.id.clone(),
            url: String::new(),
            size: 1,
            sha1: "a".repeat(40),
            image: None,
            dependencies: vec![],
        };
        let xml = "<repository><license id='test-license'>Accepted terms</license><localPackage path='platform-tools'><revision><major>37</major><minor>0</minor><micro>1</micro></revision><uses-license ref='test-license'/></localPackage></repository>";
        verify_local_package(xml, &package, &license).unwrap();
        for invalid in [
            xml.replace("<micro>1</micro>", "<micro>2</micro>"),
            xml.replace("<micro>1</micro>", "<micro>1</micro><micro>2</micro>"),
            xml.replace("<micro>1</micro>", "<micro>1</micro><preview>1</preview>"),
            xml.replace("Accepted terms", "Changed terms"),
            xml.replace("platform-tools", "emulator"),
            "<repository/>".into(),
            xml.replace("</repository>", "<localPackage path='other'/></repository>"),
        ] {
            assert!(verify_local_package(&invalid, &package, &license).is_err());
        }
    }

    #[test]
    fn rejects_external_entities_and_preserves_license_characters() {
        assert!(
            parse("<!DOCTYPE sdk [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><sdk>&x;</sdk>")
                .is_err()
        );
        assert!(parse("<sdk><license>&unknown;</license></sdk>").is_err());
        assert!(parse("<sdk/><extra/>").is_err());
        assert_eq!(
            parse("<sdk><license>A &amp; B &#x17C; <![CDATA[<text>]]></license></sdk>")
                .unwrap()
                .children[0]
                .text,
            "A & B ż <text>"
        );
    }

    #[test]
    #[ignore = "Requires the downloaded official catalogs in the isolated native fixture"]
    fn actual_provider_catalogs_select_verified_host_archives() {
        let root =
            std::path::PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap());
        let tools = std::fs::read_to_string(root.join("evidence/repository2-3.xml")).unwrap();
        let catalog = packages(
            &tools,
            "https://dl.google.com/android/repository/",
            Host::MacArm64,
        )
        .unwrap();
        assert!(catalog.packages.iter().any(|p| p.id == "emulator"
            && p.revision == "37.1.11"
            && p.sha1 == "f22f44948a2b7f0a0103645b9a639290eef92426"));
        for host in [Host::MacX64, Host::LinuxX64, Host::WindowsX64] {
            assert!(
                packages(&tools, "https://dl.google.com/android/repository/", host)
                    .unwrap()
                    .packages
                    .iter()
                    .any(|p| p.id == "emulator")
            );
        }
        let images = std::fs::read_to_string(root.join("evidence/aosp-images.xml")).unwrap();
        assert!(packages(
            &images,
            "https://dl.google.com/android/repository/sys-img/android/",
            Host::MacArm64
        )
        .unwrap()
        .packages
        .iter()
        .any(|p| p.id == "system-images;android-36;default;arm64-v8a" && p.revision == "2"));
        assert!(matches!(Host::native().unwrap(), Host::MacArm64));
        let profiles_xml =
            std::fs::read_to_string(root.join("evidence/profiles-devices.xml")).unwrap();
        assert!(!profiles(&profiles_xml, 2073600).unwrap().is_empty());
        let profiles = profiles_from_tools(&root.join("sdk/cmdline-tools/23.0"), 8294400).unwrap();
        let nexus_xml = std::fs::read_to_string(root.join("evidence/profiles-nexus.xml")).unwrap();
        let provider = super::profiles(&nexus_xml, 8294400).unwrap();
        assert!(!provider.is_empty());
        assert!(provider.iter().all(|profile| profiles.contains(profile)));
        let images = packages(
            &images,
            "https://dl.google.com/android/repository/sys-img/android/",
            Host::MacArm64,
        )
        .unwrap();
        let image = images
            .packages
            .iter()
            .find(|p| p.id == "system-images;android-36;default;arm64-v8a")
            .unwrap();
        assert!(image
            .dependencies
            .iter()
            .any(|d| d.id == "emulator" && d.minimum_revision.as_deref() == Some("35.4.9")));
    }
}
